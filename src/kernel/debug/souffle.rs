//! Etat de survie de l'enregistreur de vol : ce qu'il sait de sa propre mort.
//!
//! # Le piege que ce module casse
//!
//! L'archive du 16 septembre couvre 1,07 s a 7,30 s ; la machine a tourne
//! vingt minutes. Au dernier echantillon ecrit, elle affiche `bb_failures=0`,
//! `bb_busy_skips=0`, `bb_filets=0`. Ces zeros ne disent pas que tout allait
//! bien : ils disent qu'a 7,047 s tout allait ENCORE bien. La suite devait
//! etre racontee par des compteurs qui ne voyagent que dans un enregistrement
//! -- qu'il fallait justement pouvoir ecrire.
//!
//! L'instrumentation ne pouvait pas rapporter sa propre mort, et tant que
//! c'est vrai aucune correction de l'enregistreur n'est verifiable : un echec
//! produit exactement le meme silence qu'avant.
//!
//! Cet etat-ci vit en RAM, ne touche aucun peripherique, et se lit encore
//! quand la cle USB a disparu.
//!
//! # Pourquoi ce module est PUR
//!
//! Aucun `use crate::`, aucun `unsafe`, et l'horloge entre par parametre. Il
//! se compile donc seul avec `rustc --test`, et les regles ci-dessous sont
//! verifiees sur machine hote plutot que supposees sur la machine cible --
//! ou, precisement, on ne peut pas les observer.

use core::sync::atomic::{AtomicU64, Ordering};

/// Sentinelle de « pas encore date ».
///
/// Meme raison qu'a `drivers::equite_pilote` : zero est un horodatage
/// legitime, et s'en servir comme marqueur d'absence rend indatable un echec
/// survenu a `monotonic_ns() == 0`. Le defaut a ete trouve la-bas par un test,
/// puis corrige ICI AUSSI -- il y etait, latent, exactement sous la meme forme.
const JAMAIS: u64 = u64::MAX;

/// Compteurs de survie. Un seul exemplaire par systeme, mais la structure
/// reste instanciable pour que les tests n'aient pas a se partager un etat.
pub struct Souffle {
    /// Enregistrements REELLEMENT poses sur le support.
    poses: AtomicU64,
    /// Echecs d'ecriture depuis l'amorcage.
    echecs: AtomicU64,
    /// Longueur de la serie d'echecs en cours.
    serie: AtomicU64,
    /// La plus longue serie observee. C'est ELLE qui dit « il est mort »
    /// plutot que « il a bute une fois » : une machine saine sous charge
    /// saute des fenetres isolees, une machine qui a perdu sa cle en saute
    /// des milliers d'affilee.
    pire_serie: AtomicU64,
    /// Horodatage du dernier succes.
    dernier_ok_ns: AtomicU64,
    /// Horodatage du premier echec de la serie en cours; zero si aucune.
    premier_echec_ns: AtomicU64,
    /// Genre et sequence du dernier enregistrement perdu.
    dernier_genre: AtomicU64,
    derniere_seq: AtomicU64,
}

/// Lecture instantanee, sans atomiques, pour l'affichage et le journal.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Etat {
    pub poses: u64,
    pub echecs: u64,
    pub serie: u64,
    pub pire_serie: u64,
    pub dernier_ok_ns: u64,
    pub premier_echec_ns: u64,
    pub dernier_genre: u64,
    pub derniere_seq: u64,
    pub silence_ms: u64,
}

impl Souffle {
    pub const fn neuf() -> Self {
        Self {
            poses: AtomicU64::new(0),
            echecs: AtomicU64::new(0),
            serie: AtomicU64::new(0),
            pire_serie: AtomicU64::new(0),
            dernier_ok_ns: AtomicU64::new(0),
            premier_echec_ns: AtomicU64::new(JAMAIS),
            dernier_genre: AtomicU64::new(0),
            derniere_seq: AtomicU64::new(0),
        }
    }

    /// Un enregistrement est passe.
    pub fn succes(&self, ts_ns: u64) {
        self.poses.fetch_add(1, Ordering::Relaxed);
        self.dernier_ok_ns.store(ts_ns, Ordering::Release);
        self.serie.store(0, Ordering::Relaxed);
        self.premier_echec_ns.store(JAMAIS, Ordering::Relaxed);
    }

    /// Un enregistrement est perdu.
    pub fn echec(&self, ts_ns: u64, genre: u64, seq: u64) {
        self.echecs.fetch_add(1, Ordering::Relaxed);
        let serie = self.serie.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        // `fetch_max` et non une lecture suivie d'une ecriture : deux fils
        // peuvent echouer en meme temps, et la pire serie ne doit jamais
        // reculer -- c'est le seul chiffre qui survit a une accalmie.
        self.pire_serie.fetch_max(serie, Ordering::Relaxed);
        self.dernier_genre.store(genre, Ordering::Relaxed);
        self.derniere_seq.store(seq, Ordering::Relaxed);
        // Le PREMIER echec de la serie, pas le dernier : c'est lui qui date
        // le debut du silence, et donc l'evenement qui l'a cause.
        let _ = self.premier_echec_ns.compare_exchange(
            JAMAIS, ts_ns, Ordering::AcqRel, Ordering::Relaxed,
        );
    }

    pub fn etat(&self, maintenant_ns: u64) -> Etat {
        let dernier_ok_ns = self.dernier_ok_ns.load(Ordering::Acquire);
        Etat {
            poses: self.poses.load(Ordering::Relaxed),
            echecs: self.echecs.load(Ordering::Relaxed),
            serie: self.serie.load(Ordering::Relaxed),
            pire_serie: self.pire_serie.load(Ordering::Relaxed),
            dernier_ok_ns,
            // Hors du module, « aucun echec en cours » se lit toujours zero :
            // la sentinelle est un detail interne. `serie` leve l'ambiguite
            // d'un echec survenu a l'instant zero.
            premier_echec_ns: match self.premier_echec_ns.load(Ordering::Relaxed) {
                JAMAIS => 0,
                date => date,
            },
            dernier_genre: self.dernier_genre.load(Ordering::Relaxed),
            derniere_seq: self.derniere_seq.load(Ordering::Relaxed),
            // Tant que rien n'a jamais ete pose, il n'y a pas de silence a
            // expliquer : l'enregistreur n'a pas encore commence.
            silence_ms: if dernier_ok_ns == 0 {
                0
            } else {
                maintenant_ns.saturating_sub(dernier_ok_ns) / 1_000_000
            },
        }
    }
}

/// L'enregistreur a-t-il cesse d'ecrire ?
///
/// Un saut isole est normal : le systeme de fichiers tient la cle pendant que
/// le navigateur y lit ses quatre cents mebioctets. Ce qui ne l'est pas, c'est
/// un silence QUI DURE alors que des enregistrements continuent d'etre
/// produits. Les deux conditions sont exigees ensemble, sinon une machine au
/// repos -- qui n'ecrit rien parce qu'il ne se passe rien -- serait declaree
/// morte.
pub fn silencieux(etat: &Etat, seuil_ms: u64) -> bool {
    etat.dernier_ok_ns != 0 && etat.silence_ms >= seuil_ms && etat.serie > 0
}
