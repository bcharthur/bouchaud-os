//! Qui passe devant sur le pilote USB, et quand.
//!
//! # Le defaut, releve le 16 septembre 2026
//!
//! Trois consommateurs se partagent `RUNTIME_BUSY`, l'unique verrou du pilote
//! xHCI :
//!
//!   * la scrutation HID -- clavier et souris ;
//!   * l'enregistreur de vol, qui ecrit sur la cle ;
//!   * le systeme de fichiers, qui LIT sur la meme cle.
//!
//! Les deux premiers tentent une prise instantanee et RENONCENT si le verrou
//! est pris. Le troisieme, lui, attend -- jusqu'a cinquante millions de tours.
//!
//! Quand le navigateur demarre, le systeme de fichiers lit quatre cents
//! mebioctets de binaires sur la cle d'amorcage. Il tient donc le verrou
//! presque en continu, et les deux autres renoncent, encore et encore. Cela
//! donne exactement les deux symptomes rapportes le meme jour :
//!
//!   * « le clavier fonctionne mais quand je commence a taper dans Ladybird,
//!     il est deconnecte, je peux plus ecrire » -- la scrutation HID ne passe
//!     plus ;
//!   * l'archive s'arrete a 7,30 s, a l'instant ou les services demarrent --
//!     l'enregistreur ne passe plus.
//!
//! Le clavier n'etait pas deconnecte, et l'enregistreur n'avait pas echoue.
//! Les deux etaient AFFAMES, par la meme cause, au meme instant.
//!
//! # La regle
//!
//! Renoncer une fois est normal : une commande BOT dure quelques
//! millisecondes, et la suivante laisse un creneau. Renoncer LONGTEMPS ne
//! l'est pas. Passe un seuil, le consommateur affame leve un drapeau, et le
//! systeme de fichiers -- qui, lui, peut attendre -- lui cede le passage.
//!
//! La cession est BORNEE dans les deux sens : le systeme de fichiers ne cede
//! jamais indefiniment, et le drapeau retombe des que l'affame a pu ecrire.
//! Un verrou d'entree-sortie qui se donne la priorite sans borne remplace une
//! famine par un blocage.
//!
//! # Pourquoi ce module est PUR
//!
//! Aucun `use crate::`, aucun `unsafe`, le temps en parametre. La famine ne
//! se reproduit sur la machine qu'en lancant un navigateur de quatre cents
//! mebioctets depuis la cle d'amorcage ; ici elle se fabrique en trois appels.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Renoncements consecutifs avant de reclamer le passage.
///
/// Un seul saut est normal. Huit d'affilee, a vingt millisecondes de cadence,
/// veut dire que le verrou est tenu depuis plus de cent cinquante
/// millisecondes sans interruption : ce n'est plus une commande qui passe,
/// c'est un flux qui occupe.
pub const SAUTS_AVANT_PRIORITE: u64 = 8;

/// Duree de famine au-dela de laquelle on reclame, meme sans avoir compte
/// assez de sauts -- un consommateur lent peut etre affame sans scruter
/// souvent.
pub const FAMINE_AVANT_PRIORITE_NS: u64 = 200_000_000; // 200 ms

/// Ce que le systeme de fichiers accepte d'attendre, au maximum, pour laisser
/// passer un affame. Au-dela il reprend la main : mieux vaut une trace
/// trouee qu'un systeme de fichiers bloque.
pub const CESSION_MAXIMALE_NS: u64 = 50_000_000; // 50 ms

/// Etat de famine d'un consommateur prioritaire du pilote USB.
/// Sentinelle de « pas encore date ».
///
/// ZERO NE PEUT PAS SERVIR. C'est un horodatage parfaitement legitime -- la
/// premiere famine d'un amorcage peut tomber a `monotonic_ns() == 0` --, et
/// l'utiliser comme marqueur d'absence rendait cette famine-la INDATABLE :
/// `compare_exchange(0, 0)` reussit en ne changeant rien, le seuil de duree
/// ne tombait jamais, et le passage n'etait jamais reclame. C'est le test
/// `une_famine_longue_reclame_meme_sans_beaucoup_de_sauts` qui l'a montre.
const JAMAIS: u64 = u64::MAX;

pub struct Equite {
    sauts: AtomicU64,
    premier_saut_ns: AtomicU64,
    reclame: AtomicBool,
    cessions: AtomicU64,
}

impl Equite {
    pub const fn neuve() -> Self {
        Self {
            sauts: AtomicU64::new(0),
            premier_saut_ns: AtomicU64::new(JAMAIS),
            reclame: AtomicBool::new(false),
            cessions: AtomicU64::new(0),
        }
    }

    /// Le verrou etait pris : on a renonce. Rend `true` si, a partir de
    /// maintenant, le passage doit etre reclame.
    pub fn saut(&self, maintenant_ns: u64, seuil_sauts: u64, seuil_famine_ns: u64) -> bool {
        let sauts = self.sauts.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        // Le PREMIER saut de la serie date la famine. Poser l'horodatage a
        // chaque saut la ferait paraitre perpetuellement naissante, et le
        // seuil de duree ne serait jamais atteint.
        let _ = self.premier_saut_ns.compare_exchange(
            JAMAIS, maintenant_ns, Ordering::AcqRel, Ordering::Relaxed,
        );
        let depuis = self.premier_saut_ns.load(Ordering::Acquire);
        let assez_longtemps = depuis != JAMAIS
            && maintenant_ns.saturating_sub(depuis) >= seuil_famine_ns;
        if sauts >= seuil_sauts || assez_longtemps {
            self.reclame.store(true, Ordering::Release);
            return true;
        }
        false
    }

    /// Le verrou a ete obtenu : la famine est finie.
    pub fn succes(&self) {
        self.sauts.store(0, Ordering::Relaxed);
        self.premier_saut_ns.store(JAMAIS, Ordering::Relaxed);
        self.reclame.store(false, Ordering::Release);
    }

    /// Quelqu'un reclame-t-il le passage ?
    pub fn reclame(&self) -> bool {
        self.reclame.load(Ordering::Acquire)
    }

    pub fn note_cession(&self) {
        self.cessions.fetch_add(1, Ordering::Relaxed);
    }

    pub fn cessions(&self) -> u64 {
        self.cessions.load(Ordering::Relaxed)
    }

    pub fn sauts(&self) -> u64 {
        self.sauts.load(Ordering::Relaxed)
    }
}

/// Le systeme de fichiers doit-il encore ceder le passage ?
///
/// `false` des que plus personne ne reclame, ET des que la cession a trop
/// dure : un consommateur qui reclame sans jamais aboutir -- pilote en
/// panne, peripherique parti -- ne doit pas bloquer le systeme de fichiers.
pub fn cede_encore(
    reclame: bool,
    attendu_ns: u64,
    cession_maximale_ns: u64,
) -> bool {
    reclame && attendu_ns < cession_maximale_ns
}
