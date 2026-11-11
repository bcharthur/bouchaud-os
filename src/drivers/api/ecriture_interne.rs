//! BOUCHAUD_CLE_USB_TEST_V1 -- une image de test sur cle USB n'ecrit JAMAIS
//! sur les disques internes de la machine.
//!
//! # Pourquoi au niveau du pilote
//!
//! Une machine qu'on essaie depuis une cle porte deja un systeme -- le sien,
//! ou Windows. Lui ecrire dessus pendant un essai serait irreversible. La regle
//! ne peut donc pas vivre chez les appelants (montage, persistance, caches,
//! installation, mise a jour, `disktest`, `ata-integrite`...) : il suffirait
//! d'en oublier un. Elle vit aux points ou une COMMANDE part vers un support,
//! et nulle part ailleurs :
//!   - ATA : `ata::emet_commande`, seule ecriture du registre de commande ;
//!   - NVMe : `File::pose`, seule ecriture d'une entree de file de soumission
//!     (files admin ET d'entree-sortie) ;
//!   - USB : `bot_commande_une_fois` et `blackbox_bot_en_place`, seuls
//!     codages d'un CBW.
//! `tools/verifie-ecriture-interne.py` le verifie a chaque CI, mutations
//! comprises.
//!
//! # Pourquoi a la compilation
//!
//! Option `cle-usb-test` : aucun reglage a l'execution ne peut la desarmer, et
//! une image construite sans elle ne la porte pas par accident. L'image le dit
//! au demarrage (`BOUCHAUD_DISQUES_INTERNES lecture_seule=1`), et chaque refus
//! est compte et journalise (`BOUCHAUD_ECRITURE_INTERNE_REFUSEE`).
//!
//! La regle elle-meme -- listes blanches, identite du support autorise -- est
//! PURE et testee sur l'hote : `stockage_regle.rs`.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[path = "stockage_regle.rs"]
pub mod regle;

pub use regle::{Identite, Support};

/// Les disques internes sont-ils en lecture seule dans cette image ?
pub const LECTURE_SEULE: bool = cfg!(feature = "cle-usb-test");

/// Le GUID de disque du support de demarrage autorise, grave a la
/// construction de l'image (`BOUCHAUD_MEDIA_AUTORISE`, voir
/// `tools/release/build_trigkey_image.sh`). Absent : aucun support n'est
/// autorise, l'image est entierement en lecture seule.
pub const GUID_AUTORISE: Option<[u8; 16]> = regle::guid_autorise(option_env!("BOUCHAUD_MEDIA_AUTORISE"));

static REFUSEES: AtomicU64 = AtomicU64::new(0);
static REFUSEES_PAR_SUPPORT: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
/// Les emplacements xHCI (`slot_id`) qui ont presente le GUID autorise
/// depuis le demarrage. Par emplacement, et non par sonde : la meme cle est
/// lue par la boite noire PUIS par le pilote de volume, et ce n'est qu'UN
/// support.
static SLOTS_AUTORISES: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static AMBIGU: AtomicBool = AtomicBool::new(false);

fn indice(support: Support) -> usize {
    match support {
        Support::Ata => 0,
        Support::Nvme => 1,
        Support::Usb => 2,
    }
}

fn juge(support: Support, identite: Identite, classe: regle::Classe, code: u8, detail: u64) -> bool {
    let v = regle::verdict(LECTURE_SEULE, support, identite, AMBIGU.load(Ordering::Acquire), classe);
    let regle::Verdict::Refuse(raison) = v else { return true };
    let n = REFUSEES.fetch_add(1, Ordering::Relaxed) + 1;
    REFUSEES_PAR_SUPPORT[indice(support)].fetch_add(1, Ordering::Relaxed);
    // Les huit premiers, puis aux puissances de deux : un appelant qui
    // reessaie en boucle ne noie pas la console.
    if n <= 8 || n.is_power_of_two() {
        crate::serial_println!(
            "BOUCHAUD_ECRITURE_INTERNE_REFUSEE support={} commande={:#04x} detail={:#x} identite={} raison={} n={}",
            support.nom(), code, detail, identite.nom(), raison.nom(), n
        );
    }
    false
}

/// Une commande ATA peut-elle partir ? (registre de commande)
pub fn ata(commande: u8, lba: u64) -> bool {
    juge(Support::Ata, Identite::Inconnue, regle::classe_ata(commande), commande, lba)
}

/// Une entree de file de soumission NVMe peut-elle etre posee ?
pub fn nvme(admin: bool, sqe: &[u32; 16]) -> bool {
    let detail = ((admin as u64) << 63) | sqe[10] as u64 | ((sqe[11] as u64) << 32);
    juge(Support::Nvme, Identite::Inconnue, regle::classe_nvme(admin, sqe), (sqe[0] & 0xFF) as u8, detail)
}

/// Un bloc de commande SCSI peut-il partir vers ce stockage USB ?
pub fn scsi(identite: Identite, cdb: &[u8]) -> bool {
    let lba = if cdb.len() >= 6 { u32::from_be_bytes([cdb[2], cdb[3], cdb[4], cdb[5]]) as u64 } else { 0 };
    juge(Support::Usb, identite, regle::classe_scsi(cdb), cdb.first().copied().unwrap_or(0), lba)
}

/// L'identite d'un stockage USB d'apres son bloc LBA 1, ETABLIE ET JOURNALISEE.
///
/// Un second emplacement qui presente le GUID autorise rend la situation
/// AMBIGUE pour le reste de la session : plus aucune ecriture USB n'est
/// permise, sur aucun des supports. Une cle autorisee retiree puis rebranchee
/// sur un autre emplacement compte donc pour deux : le refus est le repli.
///
/// LIMITE (mesuree par `run_cle_usb_test.sh`, scenario `ambigu`) :
/// l'enumeration est sequentielle. Tant que la seconde cle n'est pas
/// arrivee, la premiere EST la seule autorisee, et sa sonde de montage lui
/// envoie `SYNCHRONIZE CACHE` (sans donnees, apres aucune ecriture : rien n'est
/// modifie). Aucun peripherique ne peut savoir qu'un autre arrivera.
pub fn etablit_identite(slot: u8, bloc_lba1: Option<&[u8]>) -> Identite {
    let id = regle::identite(bloc_lba1, GUID_AUTORISE);
    if id == Identite::Autorisee {
        SLOTS_AUTORISES[slot as usize / 64].fetch_or(1 << (slot % 64), Ordering::AcqRel);
        let n: u32 = SLOTS_AUTORISES.iter().map(|m| m.load(Ordering::Acquire).count_ones()).sum();
        if n >= 2 {
            AMBIGU.store(true, Ordering::Release);
        }
    }
    if LECTURE_SEULE {
        crate::serial_println!(
            "BOUCHAUD_MEDIA_IDENTITE support=usb slot={} identite={} ambigu={}",
            slot, id.nom(), AMBIGU.load(Ordering::Acquire) as u8
        );
    }
    id
}

/// (refus, refus ATA, refus NVMe, refus USB).
pub fn refusees() -> (u64, u64, u64, u64) {
    (
        REFUSEES.load(Ordering::Relaxed),
        REFUSEES_PAR_SUPPORT[0].load(Ordering::Relaxed),
        REFUSEES_PAR_SUPPORT[1].load(Ordering::Relaxed),
        REFUSEES_PAR_SUPPORT[2].load(Ordering::Relaxed),
    )
}

/// Une ligne, au demarrage, avant le premier pilote de stockage.
pub fn annonce() {
    crate::serial_println!(
        "BOUCHAUD_DISQUES_INTERNES lecture_seule={} image={} media_autorise={}",
        LECTURE_SEULE as u8,
        if LECTURE_SEULE { "cle-usb-test" } else { "standard" },
        if GUID_AUTORISE.is_some() { "guid-gpt" } else { "aucun" },
    );
}

/// Le bilan, a la demande (commande `disques-internes`) ou en fin de banc.
pub fn bilan() {
    let (n, ata, nvme, usb) = refusees();
    crate::println!(
        "disques internes : lecture_seule={} refus={} (ata={} nvme={} usb={}) ambigu={}",
        LECTURE_SEULE as u8, n, ata, nvme, usb, AMBIGU.load(Ordering::Acquire) as u8
    );
    crate::serial_println!(
        "BOUCHAUD_DISQUES_INTERNES_BILAN lecture_seule={} refus={} ata={} nvme={} usb={} ambigu={}",
        LECTURE_SEULE as u8, n, ata, nvme, usb, AMBIGU.load(Ordering::Acquire) as u8
    );
}
