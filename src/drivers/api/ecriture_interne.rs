//! BOUCHAUD_CLE_USB_TEST_V1 -- une image de test sur cle USB n'ecrit JAMAIS
//! sur les disques internes de la machine.
//!
//! # Pourquoi au niveau du pilote
//!
//! Une machine qu'on essaie depuis une cle porte deja un systeme -- le sien,
//! ou Windows. Lui ecrire dessus pendant un essai serait irreversible. La regle
//! ne peut donc pas vivre chez les appelants (montage differe, persistance,
//! caches, installation, commandes `nvmetest`/`disktest`, installation de mise
//! a jour...) : il suffirait d'en oublier un. Elle vit dans les DEUX seuls
//! points ou un octet part vers un disque interne -- l'ecriture du pilote NVMe
//! (`PiloteNvme::soumet_ecriture`) et celle du pilote ATA (`ata::write`) --,
//! et la cle USB (stockage de masse, boite noire) n'est pas concernee.
//!
//! # Pourquoi a la compilation
//!
//! Option `cle-usb-test` : aucun reglage a l'execution ne peut la desarmer, et
//! une image construite sans elle ne la porte pas par accident. L'image le dit
//! au demarrage (`BOUCHAUD_DISQUES_INTERNES lecture_seule=1`), et chaque refus
//! est compte et journalise (`BOUCHAUD_ECRITURE_INTERNE_REFUSEE`).
//!
//! Les lectures restent permises : le diagnostic des peripheriques lit le
//! disque interne (table de partitions, identification) sans rien y changer.

use core::sync::atomic::{AtomicU64, Ordering};

/// Les disques internes sont-ils en lecture seule dans cette image ?
pub const LECTURE_SEULE: bool = cfg!(feature = "cle-usb-test");

static REFUSEES: AtomicU64 = AtomicU64::new(0);
static BLOCS_REFUSES: AtomicU64 = AtomicU64::new(0);

/// Une ecriture de `blocs` blocs a `lba` sur le disque interne `peripherique`
/// est-elle permise ? Dans une image `cle-usb-test`, jamais : le refus est
/// compte, et journalise (les huit premiers, puis aux puissances de deux --
/// un appelant qui reessaie en boucle ne noie pas la console).
pub fn permise(peripherique: &'static str, lba: u64, blocs: usize) -> bool {
    if !LECTURE_SEULE {
        return true;
    }
    let n = REFUSEES.fetch_add(1, Ordering::Relaxed) + 1;
    BLOCS_REFUSES.fetch_add(blocs as u64, Ordering::Relaxed);
    if n <= 8 || n.is_power_of_two() {
        crate::serial_println!(
            "BOUCHAUD_ECRITURE_INTERNE_REFUSEE peripherique={} lba={} blocs={} n={}",
            peripherique, lba, blocs, n
        );
    }
    false
}

/// (ecritures refusees, blocs refuses).
pub fn refusees() -> (u64, u64) {
    (REFUSEES.load(Ordering::Relaxed), BLOCS_REFUSES.load(Ordering::Relaxed))
}

/// Une ligne, au demarrage, avant le premier pilote de stockage.
pub fn annonce() {
    crate::serial_println!(
        "BOUCHAUD_DISQUES_INTERNES lecture_seule={} image={}",
        LECTURE_SEULE as u8,
        if LECTURE_SEULE { "cle-usb-test" } else { "standard" }
    );
}

/// Le bilan, a la demande (commande `disques-internes`) ou en fin de banc.
pub fn bilan() {
    let (n, blocs) = refusees();
    crate::serial_println!(
        "BOUCHAUD_DISQUES_INTERNES_BILAN lecture_seule={} ecritures_refusees={} blocs_refuses={}",
        LECTURE_SEULE as u8, n, blocs
    );
}
