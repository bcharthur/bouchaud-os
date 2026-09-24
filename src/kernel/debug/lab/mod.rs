//! LAB MODE : l'observabilite qui ne depend de rien.
//!
//! # Le constat qui fonde ce module
//!
//! Releve TRIGKEY, campagne `1179cdd` :
//!
//! ```text
//! com1=bus-flottant  serial_bytes=0
//! rx_packets=64  rx_cur=0  desc_nic=64  desc_cpu=0  (pendant 151 s)
//! blackbox : session 395 s, persistance 193 s, checkpoints=0, fin=false
//! ```
//!
//! Trois faits, une seule lecon. La trace serie n'existe pas sur cette
//! machine ; la reception meurt apres soixante-quatre trames, donc le reseau
//! non plus ; la boite noire s'arrete deux cents secondes avant la fin de la
//! session, donc le disque non plus. Chacun des trois canaux d'enquete a
//! disparu exactement au moment ou il servait.
//!
//! Le LAB inverse la dependance : les sondes ecrivent dans un anneau en RAM,
//! et les canaux viennent y lire. Aucun canal n'est sur le chemin d'une sonde.
//!
//! ```text
//!   SONDE ──▶ ANNEAU (2048, RAM) ──┬──▶ boite noire
//!                                  ├──▶ shell
//!                                  ├──▶ BRDP (TCP 2222)
//!                                  └──▶ telemetrie (UDP, best effort)
//! ```
//!
//! # Ce que ce module ne fait PAS
//!
//! Il ne repare rien et ne pretend rien corriger. La panne RTL8168 reste
//! entiere : la reception s'arrete au premier bouclage `63 -> 0` et le second
//! tour n'a jamais lieu. Ce module sert a la VOIR, en capturant les
//! descripteurs et les registres a l'instant du bouclage plutot qu'apres coup.

pub mod anneau;
pub mod auditd;
pub mod auditeur;
pub mod catalogue;

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub use anneau::{Evenement, Lecture};
pub use catalogue::{id, Categorie};

/// Capacite de l'anneau, en evenements.
///
/// Deux mille quarante-huit emplacements de soixante-quatre octets : cent
/// vingt-huit kibioctets de BSS, et de quoi tenir plusieurs minutes a la
/// cadence d'un auditeur a dix hertz qui capture un bouclage d'anneau complet.
/// Une capture RTL8168 complete coute soixante-dix evenements environ ; la
/// fenetre couvre donc une trentaine de captures meme sans lecteur.
pub const CAPACITE: usize = 2048;

static ANNEAU: anneau::Anneau<CAPACITE> = anneau::Anneau::nouveau();
static ARME: AtomicBool = AtomicBool::new(false);
/// Evenements emis avant que l'horloge monotone ne soit utilisable.
static AVANT_HORLOGE: AtomicU64 = AtomicU64::new(0);

/// Ouvre le LAB. Idempotent.
///
/// Emettre avant cet appel n'est pas une faute : l'anneau est utilisable des
/// le premier instant, puisqu'il est statique et sans allocation. L'appel ne
/// fait que dater l'ouverture et publier la capacite.
pub fn demarre() {
    if ARME.swap(true, Ordering::AcqRel) {
        return;
    }
    emets(Categorie::Lab, id::LAB_DEMARRE, [CAPACITE as u64, 0, 0, 0]);
}

pub fn arme() -> bool {
    ARME.load(Ordering::Relaxed)
}

/// L'horloge, avec sa propre panne prevue.
///
/// Le LAB doit fonctionner AVANT que le timer soit calibre -- le bouclage
/// d'anneau RTL8168 peut survenir tot. Un horodatage nul est alors juste, et
/// se compte : c'est plus honnete qu'un evenement escamote faute d'heure.
fn maintenant_ns() -> u64 {
    let t = crate::kernel::timer::monotonic_ns();
    if t == 0 {
        AVANT_HORLOGE.fetch_add(1, Ordering::Relaxed);
    }
    t
}

fn cpu() -> u16 {
    crate::arch::x86_64::usermode::cpu_index() as u16
}

/// LE POINT D'ENTREE DES SONDES.
///
/// Ne bloque pas, n'alloue pas, ne prend aucun verrou, ne peut pas echouer.
/// Appelable sous `VERROU_RECEPTION`, dans un traitant d'interruption, ou
/// pendant une reprise de carte.
pub fn emets(categorie: Categorie, event_id: u32, args: [u64; 4]) -> u64 {
    ANNEAU.emets(maintenant_ns(), cpu(), categorie as u16, event_id, args)
}

/// Emet en fournissant l'horodatage. Pour une sonde qui a deja lu l'horloge.
pub fn emets_a(t_ns: u64, categorie: Categorie, event_id: u32, args: [u64; 4]) -> u64 {
    ANNEAU.emets(t_ns, cpu(), categorie as u16, event_id, args)
}

/// Lit un evenement, en reessayant les dechirures.
pub fn lis(seq: u64) -> Lecture {
    ANNEAU.lis_stable(seq, 4)
}

/// Total emis, plus ancien encore lisible, prochain numero, capacite.
pub fn compteurs() -> (u64, u64, u64, u64) {
    (
        ANNEAU.emises(),
        ANNEAU.plus_ancienne(),
        ANNEAU.prochaine(),
        CAPACITE as u64,
    )
}

/// Recale un curseur en retard et CHIFFRE la perte.
///
/// Un lecteur qui se recale sans compter presenterait une chronologie trouee
/// comme continue. Celui-ci emet `LAB_PERTE` : le trou fait partie du releve.
pub fn recale(curseur: u64) -> (u64, u64) {
    let (neuf, perdues) = ANNEAU.recale(curseur);
    if perdues > 0 {
        emets(
            Categorie::Lab,
            id::LAB_PERTE,
            [perdues, curseur, neuf, 0],
        );
    }
    (neuf, perdues)
}

/// Evenements emis alors que l'horloge monotone rendait encore zero.
pub fn avant_horloge() -> u64 {
    AVANT_HORLOGE.load(Ordering::Relaxed)
}

/// Parcourt les evenements a partir de `curseur`, au plus `limite`.
///
/// Rend le curseur d'apres. Le rappel recoit les evenements DANS L'ORDRE ; une
/// dechirure persistante interrompt le parcours plutot que de sauter un
/// numero, parce qu'un trou silencieux est pire qu'un parcours court.
pub fn parcours(
    curseur: u64,
    limite: usize,
    mut rappel: impl FnMut(&Evenement),
) -> u64 {
    let (mut seq, _) = recale(curseur);
    let mut rendus = 0usize;
    while rendus < limite {
        match lis(seq) {
            Lecture::Evenement(ev) => {
                rappel(&ev);
                seq += 1;
                rendus += 1;
            }
            Lecture::PasEncore => break,
            Lecture::Ecrasee => {
                let (neuf, _) = recale(seq);
                if neuf == seq {
                    break;
                }
                seq = neuf;
            }
            Lecture::Dechiree => break,
        }
    }
    seq
}

/// Curseur pour « les `combien` derniers evenements ».
pub fn queue(combien: usize) -> u64 {
    let prochaine = ANNEAU.prochaine();
    prochaine
        .saturating_sub(combien as u64)
        .max(ANNEAU.plus_ancienne())
}
