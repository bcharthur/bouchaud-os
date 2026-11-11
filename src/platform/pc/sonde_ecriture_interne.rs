//! BOUCHAUD_CLE_USB_TEST_V1 -- la sonde du banc : EMETTRE de vraies commandes
//! d'ecriture vers chaque support, et compter ce que la regle a refuse.
//!
//! Compilee seulement avec `banc-ecriture-interne`, jamais dans une image
//! livree, et REFUSE de s'executer hors d'une machine virtuelle QEMU (TCG ou
//! KVM, d'apres la signature CPUID de l'hyperviseur) : dans une image SANS
//! `cle-usb-test` -- le temoin du banc --, ces commandes ecrivent, vident,
//! effacent et FORMATENT vraiment.
//!
//! Pour chaque commande : `BOUCHAUD_SONDE_ECRITURE support=.. commande=..
//! resultat=refusee|partie|..`. `refusee` se lit au compteur de la regle (il
//! a avance pendant la tentative) -- pas au code d'erreur, qu'un disque qui
//! rejette la commande rendrait aussi. Ce qui est PARTI se verifie hors de la
//! machine : empreinte des images et trace des commandes vues par QEMU
//! (`tools/ci/run_cle_usb_test.sh`).

use crate::drivers::bloc::{self, Achevement, Volume};
use crate::drivers::ecriture_interne;

const MARQUE: &[u8; 32] = b"BOUCHAUD-SONDE-ECRITURE-INTERNE!";

/// La machine est-elle une VM QEMU (TCG ou KVM) ?
fn sous_qemu() -> bool {
    use core::arch::x86_64::__cpuid;
    if __cpuid(1).ecx & (1 << 31) == 0 {
        return false; // materiel nu
    }
    let sig = __cpuid(0x4000_0000);
    let mut nom = [0u8; 12];
    nom[0..4].copy_from_slice(&sig.ebx.to_le_bytes());
    nom[4..8].copy_from_slice(&sig.ecx.to_le_bytes());
    nom[8..12].copy_from_slice(&sig.edx.to_le_bytes());
    &nom == b"TCGTCGTCGTCG" || &nom == b"KVMKVMKVM\0\0\0"
}

struct Bilan {
    parties: u32,
    refusees: u32,
}

impl Bilan {
    /// Execute `essai` et classe le resultat d'apres le compteur de la regle.
    fn essaie(&mut self, support: &str, commande: &str, essai: impl FnOnce() -> &'static str) {
        let avant = ecriture_interne::refusees().0;
        let detail = essai();
        let refusee = ecriture_interne::refusees().0 > avant;
        if refusee {
            self.refusees += 1;
        } else {
            self.parties += 1;
        }
        crate::serial_println!(
            "BOUCHAUD_SONDE_ECRITURE support={} commande={} resultat={} detail={}",
            support, commande, if refusee { "refusee" } else { "partie" }, detail
        );
    }
}

fn achevement(a: Achevement) -> &'static str {
    match a {
        Achevement::Fait(_) => "fait",
        Achevement::EnCours => "en-cours",
        Achevement::Erreur => "erreur",
        Achevement::Absent => "absent",
    }
}

fn resultat<T>(r: Result<T, &'static str>) -> &'static str {
    match r {
        Ok(_) => "ok",
        Err(e) => e,
    }
}

pub fn commande() {
    if !sous_qemu() {
        crate::println!("sonde-ecriture-interne : refusee hors d'une machine virtuelle QEMU");
        crate::serial_println!("BOUCHAUD_SONDE_ECRITURE_REFUSEE raison=pas-une-vm-qemu");
        return;
    }
    crate::serial_println!(
        "BOUCHAUD_SONDE_ECRITURE_DEBUT lecture_seule={}",
        ecriture_interne::LECTURE_SEULE as u8
    );
    let mut b = Bilan { parties: 0, refusees: 0 };
    let mut bloc_marque = [0u8; 512];
    bloc_marque[..MARQUE.len()].copy_from_slice(MARQUE);

    // --- ATA : le disque esclave (le disque de donnees du banc) ------------
    use crate::drivers::ata::{self, Drive};
    let (_, secteurs) = ata::capacities();
    if ata::present(Drive::Slave) && secteurs > 1 {
        let lba = secteurs - 1;
        b.essaie("ata", "write-sectors", || if ata::write(Drive::Slave, lba, 1, &bloc_marque) == 1 { "ecrit" } else { "non-ecrit" });
        b.essaie("ata", "flush-cache", || if ata::vide_cache(Drive::Slave) { "ok" } else { "echec" });
        b.essaie("ata", "data-set-management", || if ata::sonde_brute(Drive::Slave, 0x06) { "emise" } else { "non-emise" });
        b.essaie("ata", "security-erase-unit", || if ata::sonde_brute(Drive::Slave, 0xF4) { "emise" } else { "non-emise" });
    } else {
        crate::serial_println!("BOUCHAUD_SONDE_ECRITURE support=ata resultat=absent");
    }

    // --- NVMe : le disque interne ------------------------------------------
    use crate::drivers::nvme;
    use crate::drivers::nvme::{sqe_entete, Sqe};
    if let Some(dernier) = nvme::sonde_dernier_bloc().filter(|d| *d > 4) {
        let v = nvme::VOLUME_INTERNE;
        b.essaie("nvme", "write", || achevement(bloc::ecrit(v, dernier, 1, &bloc_marque)));
        b.essaie("nvme", "flush", || if bloc::vidange(v) { "ok" } else { "echec" });
        // BOUCHAUD_NVME_CID_JAMAIS_POSE_V1 : plus d'ecritures refusees que le
        // pot ne compte d'identifiants, puis une LECTURE doit encore partir.
        let mut refus = 0u32;
        for _ in 0..300 {
            let avant = ecriture_interne::refusees().0;
            let _ = bloc::ecrit(v, dernier, 1, &bloc_marque);
            refus += (ecriture_interne::refusees().0 > avant) as u32;
        }
        let mut relu = [0u8; 512];
        let lecture = bloc::lit(v, 0, 1, &mut relu);
        crate::serial_println!(
            "BOUCHAUD_SONDE_ECRITURE_POT support=nvme ecritures=300 refusees={} lecture_apres={}",
            refus, achevement(lecture)
        );
        // Write Zeroes (0x08) : SLBA, NLB = 0 (un bloc).
        let mut wz: Sqe = sqe_entete(0x08, 0, 0);
        wz[10] = (dernier - 1) as u32;
        wz[11] = ((dernier - 1) >> 32) as u32;
        b.essaie("nvme", "write-zeroes", || resultat(nvme::sonde_brute(false, wz, &[])));
        // Dataset Management (0x09), une plage, Deallocate (AD, bit 2).
        let mut dsm: Sqe = sqe_entete(0x09, 0, 0);
        dsm[10] = 0;
        dsm[11] = 1 << 2;
        let mut plage = [0u8; 16];
        plage[4..8].copy_from_slice(&1u32.to_le_bytes());
        plage[8..16].copy_from_slice(&(dernier - 2).to_le_bytes());
        b.essaie("nvme", "dataset-management", || resultat(nvme::sonde_brute(false, dsm, &plage)));
        // Set Features (0x09) : cache d'ecriture volatil (FID 0x06).
        let mut sf: Sqe = sqe_entete(0x09, 0, 0);
        sf[10] = 0x06;
        b.essaie("nvme", "set-features-vwc", || resultat(nvme::sonde_brute(true, sf, &[])));
        // Format NVM (0x80), namespace 1. EN DERNIER : dans le temoin, il
        // efface le namespace pour de bon.
        let fmt: Sqe = sqe_entete(0x80, 0, 1);
        b.essaie("nvme", "format-nvm", || resultat(nvme::sonde_brute(true, fmt, &[])));
    } else {
        crate::serial_println!("BOUCHAUD_SONDE_ECRITURE support=nvme resultat=absent");
    }

    // --- USB : le support lie au volume -----------------------------------
    use crate::drivers::xhci_active;
    let v = xhci_active::VOLUME_USB;
    let d = bloc::descripteur(v);
    if bloc::present(v) && d.taille_bloc == 512 && d.blocs > 4 {
        let dernier = d.blocs - 1;
        b.essaie("usb", "write-10-volume", || achevement(bloc::ecrit(v, dernier, 1, &bloc_marque)));
        let cdb = |op: u8, lba: u32, n: u16| {
            let l = lba.to_be_bytes();
            let c = n.to_be_bytes();
            [op, 0, l[0], l[1], l[2], l[3], 0, c[0], c[1], 0]
        };
        let ecrit = cdb(0x2A, (dernier - 1) as u32, 1);
        b.essaie("usb", "write-10", || resultat(xhci_active::sonde_scsi(&ecrit, 512).map(|(_, r)| r).unwrap_or(Err("absent"))));
        let sync = cdb(0x35, 0, 0);
        b.essaie("usb", "synchronize-cache", || resultat(xhci_active::sonde_scsi(&sync, 0).map(|(_, r)| r).unwrap_or(Err("absent"))));
        // Interdites sur TOUT support dans une image `cle-usb-test`, meme
        // l'autorise : sans phase de donnees, elles ne font rien de plus que
        // se presenter -- c'est ce qu'on veut voir (ou ne pas voir) cote QEMU.
        let format = [0x04u8, 0, 0, 0, 0, 0];
        b.essaie("usb", "format-unit", || resultat(xhci_active::sonde_scsi(&format, 0).map(|(_, r)| r).unwrap_or(Err("absent"))));
        let unmap = [0x42u8, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        b.essaie("usb", "unmap", || resultat(xhci_active::sonde_scsi(&unmap, 0).map(|(_, r)| r).unwrap_or(Err("absent"))));
        let mut ata16 = [0u8; 16];
        ata16[0] = 0x85; // ATA PASS-THROUGH (16), sans donnees
        ata16[14] = 0xE7; // FLUSH CACHE ATA, deguise
        b.essaie("usb", "ata-pass-through-16", || resultat(xhci_active::sonde_scsi(&ata16, 0).map(|(_, r)| r).unwrap_or(Err("absent"))));
        let mut ws = [0u8; 16];
        ws[0] = 0x93; // WRITE SAME (16), zero bloc
        b.essaie("usb", "write-same-16", || resultat(xhci_active::sonde_scsi(&ws, 0).map(|(_, r)| r).unwrap_or(Err("absent"))));
    } else {
        crate::serial_println!("BOUCHAUD_SONDE_ECRITURE support=usb resultat=absent");
    }

    crate::serial_println!(
        "BOUCHAUD_SONDE_ECRITURE_FIN lecture_seule={} parties={} refusees={}",
        ecriture_interne::LECTURE_SEULE as u8, b.parties, b.refusees
    );
    ecriture_interne::bilan();
}
