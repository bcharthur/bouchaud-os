//! Extinction ACPI reelle : le bloc PM1 et l'objet `\_S5_` du DSDT.
//!
//! # Pourquoi ce module existe
//!
//! `power::shutdown` n'ecrivait que sur des ports d'EMULATEUR : `0x604` pour
//! QEMU, `0xB004` pour Bochs, `0x4004` pour VirtualBox. Aucun de ces ports
//! n'existe sur une machine reelle. Sur le materiel de reference, choisir
//! « Eteindre » dans le menu ecrivait donc dans le vide puis s'arretait sur un
//! `hlt` : l'ecran se figeait, les ventilateurs continuaient de tourner, et il
//! fallait tenir le bouton d'alimentation -- ce qui emporte au passage le
//! releve de vol qu'on cherchait justement a relire.
//!
//! # Ce qu'il faut trouver pour eteindre pour de vrai
//!
//! La sequence ACPI tient en une ecriture, mais ses operandes sont eparpilles :
//!
//! 1. Le RSDP donne la racine des tables (XSDT en revision 2+, RSDT sinon).
//! 2. La FADT (signature `FACP`) donne l'adresse d'entree/sortie du bloc de
//!    controle PM1a -- et parfois PM1b, sur les machines a deux blocs --, ainsi
//!    que l'adresse du DSDT.
//! 3. Le DSDT contient l'objet AML `\_S5_`, un paquet dont les deux premiers
//!    elements sont les valeurs `SLP_TYP` a ecrire dans PM1a et PM1b.
//! 4. L'extinction est alors `PM1x_CNT = (SLP_TYP << 10) | SLP_EN`.
//!
//! La valeur `SLP_TYP` n'est PAS une constante : elle vaut 0 sur beaucoup de
//! chipsets, 5 sur d'autres, 7 sur certains portables. L'ecrire en dur eteint
//! une machine sur trois et met les autres en veille profonde.
//!
//! # Ce que ce module ne fait pas
//!
//! Il n'interprete pas l'AML. Il CHERCHE la signature `_S5_` dans le DSDT et
//! lit le paquet qui suit, ce qui est la forme que prend cet objet dans toutes
//! les tables reelles. Un interpreteur AML complet serait plusieurs milliers de
//! lignes pour un seul entier ; en echange, cette lecture refuse tout ce
//! qu'elle ne reconnait pas plutot que de deviner, et `eteint()` rend alors
//! `false` sans rien ecrire.

use core::sync::atomic::{AtomicBool, AtomicU16, AtomicU8, Ordering};

use crate::arch::x86_64::ports::{inw, outw};
use crate::boot::BootInfo;

mod aml;
use aml::extrait_s5;

/// Bit `SLP_EN` du registre de controle PM1 : c'est lui qui declenche.
const SLP_EN: u16 = 1 << 13;
/// Decalage du champ `SLP_TYP` dans le meme registre.
const SLP_TYP_SHIFT: u16 = 10;
/// Bit `SCI_EN` : ACPI est-il deja en main du systeme, ou encore du firmware ?
const SCI_EN: u16 = 1;
/// Longueur de l'en-tete commun a toutes les tables ACPI.
const SDT_HEADER_LEN: usize = 36;
/// Une table plus grande que cela est une adresse fausse, pas une table.
const MAX_TABLE_LEN: usize = 1 << 20;
/// Au-dela, la racine ne pointe plus sur des tables mais sur du bruit.
const MAX_ROOT_ENTRIES: usize = 64;

static PRET: AtomicBool = AtomicBool::new(false);
static PM1A_CNT: AtomicU16 = AtomicU16::new(0);
static PM1B_CNT: AtomicU16 = AtomicU16::new(0);
static SLP_TYP_A: AtomicU8 = AtomicU8::new(0);
static SLP_TYP_B: AtomicU8 = AtomicU8::new(0);
static SMI_CMD: AtomicU16 = AtomicU16::new(0);
static ACPI_ENABLE: AtomicU8 = AtomicU8::new(0);

unsafe fn lit8(phys: u64) -> u8 {
    core::ptr::read_volatile(crate::kernel::memory::phys_to_virt(phys))
}

unsafe fn lit16(phys: u64) -> u16 {
    u16::from_le_bytes([lit8(phys), lit8(phys + 1)])
}

unsafe fn lit32(phys: u64) -> u32 {
    u32::from_le_bytes([lit8(phys), lit8(phys + 1), lit8(phys + 2), lit8(phys + 3)])
}

unsafe fn lit64(phys: u64) -> u64 {
    u64::from_le_bytes([
        lit8(phys), lit8(phys + 1), lit8(phys + 2), lit8(phys + 3),
        lit8(phys + 4), lit8(phys + 5), lit8(phys + 6), lit8(phys + 7),
    ])
}

unsafe fn signature(phys: u64) -> [u8; 4] {
    [lit8(phys), lit8(phys + 1), lit8(phys + 2), lit8(phys + 3)]
}

unsafe fn longueur(phys: u64) -> Option<usize> {
    let len = lit32(phys + 4) as usize;
    (len >= SDT_HEADER_LEN && len <= MAX_TABLE_LEN).then_some(len)
}

/// Trouve la FADT depuis le RSDP.
unsafe fn trouve_fadt(rsdp: u64) -> Option<u64> {
    let mut entete = [0u8; 8];
    for (index, octet) in entete.iter_mut().enumerate() {
        *octet = lit8(rsdp + index as u64);
    }
    if &entete != b"RSD PTR " {
        return None;
    }
    let revision = lit8(rsdp + 15);
    let xsdt = if revision >= 2 { lit64(rsdp + 24) } else { 0 };
    let rsdt = lit32(rsdp + 16) as u64;
    let (racine, taille_entree) = if xsdt != 0 {
        (xsdt, 8usize)
    } else if rsdt != 0 {
        (rsdt, 4usize)
    } else {
        return None;
    };
    let racine_len = longueur(racine)?;
    let count = ((racine_len - SDT_HEADER_LEN) / taille_entree).min(MAX_ROOT_ENTRIES);
    for index in 0..count {
        let pointeur = racine + SDT_HEADER_LEN as u64 + (index * taille_entree) as u64;
        let table = if taille_entree == 8 {
            lit64(pointeur)
        } else {
            lit32(pointeur) as u64
        };
        if table != 0 && &signature(table) == b"FACP" {
            return Some(table);
        }
    }
    None
}

/// Lit dans la FADT le port PM1a/PM1b et l'adresse du DSDT.
///
/// Les champs 64 bits (`X_PM1a_CNT_BLK`, `X_DSDT`) priment quand ils sont
/// renseignes : sur une machine UEFI recente, les champs 32 bits historiques
/// peuvent etre a zero. L'inverse est vrai sur les machines plus anciennes,
/// d'ou les deux lectures.
unsafe fn lit_fadt(fadt: u64) -> Option<(u16, u16, u64, u16, u8)> {
    let len = longueur(fadt)?;
    if len < 132 {
        return None;
    }
    let smi_cmd = lit32(fadt + 48) as u16;
    let acpi_enable = lit8(fadt + 52);
    let mut pm1a = lit32(fadt + 64) as u16;
    let mut pm1b = lit32(fadt + 68) as u16;
    let mut dsdt = lit32(fadt + 40) as u64;
    if len >= 148 {
        let x_dsdt = lit64(fadt + 140);
        if x_dsdt != 0 {
            dsdt = x_dsdt;
        }
    }
    // Structures d'adresse generique : octet 0 = espace (1 = port d'E/S),
    // octets 4..12 = adresse.
    if len >= 184 {
        if lit8(fadt + 172) == 1 {
            let adresse = lit64(fadt + 176);
            if adresse != 0 {
                pm1a = adresse as u16;
            }
        }
    }
    if len >= 196 {
        if lit8(fadt + 184) == 1 {
            let adresse = lit64(fadt + 188);
            if adresse != 0 {
                pm1b = adresse as u16;
            }
        }
    }
    if pm1a == 0 {
        return None;
    }
    Some((pm1a, pm1b, dsdt, smi_cmd, acpi_enable))
}

/// Decouvre le bloc PM1 et `\_S5_`, et retient de quoi eteindre plus tard.
///
/// Appele une fois au demarrage, PAS a l'extinction : chercher les tables au
/// moment de couper obligerait a parcourir la memoire physique alors que le
/// systeme est deja en train de s'arreter. Le releve de vol porte ainsi la
/// preuve que la machine SAIT s'eteindre, bien avant qu'on le lui demande.
pub fn prepare(boot: &BootInfo) {
    let Some(rsdp) = boot.rsdp_address else {
        crate::serial_println!("BOUCHAUD_ACPI_S5 etat=absent raison=pas-de-rsdp");
        return;
    };
    unsafe {
        let Some(fadt) = trouve_fadt(rsdp) else {
            crate::serial_println!("BOUCHAUD_ACPI_S5 etat=absent raison=pas-de-fadt");
            return;
        };
        let Some((pm1a, pm1b, dsdt, smi_cmd, acpi_enable)) = lit_fadt(fadt) else {
            crate::serial_println!("BOUCHAUD_ACPI_S5 etat=absent raison=fadt-illisible");
            return;
        };
        let Some(dsdt_len) = longueur(dsdt) else {
            crate::serial_println!("BOUCHAUD_ACPI_S5 etat=absent raison=dsdt-illisible");
            return;
        };
        // Le DSDT est lu par le fenetrage physique : il n'est ni copie ni
        // modifie, et la recherche est bornee par sa propre longueur.
        let corps = core::slice::from_raw_parts(
            crate::kernel::memory::phys_to_virt(dsdt + SDT_HEADER_LEN as u64),
            dsdt_len - SDT_HEADER_LEN,
        );
        let Some((typ_a, typ_b)) = extrait_s5(corps) else {
            crate::serial_println!(
                "BOUCHAUD_ACPI_S5 etat=absent raison=pas-de-s5 dsdt={:#x} octets={}",
                dsdt, dsdt_len,
            );
            return;
        };
        PM1A_CNT.store(pm1a, Ordering::Release);
        PM1B_CNT.store(pm1b, Ordering::Release);
        SLP_TYP_A.store(typ_a, Ordering::Release);
        SLP_TYP_B.store(typ_b, Ordering::Release);
        SMI_CMD.store(smi_cmd, Ordering::Release);
        ACPI_ENABLE.store(acpi_enable, Ordering::Release);
        PRET.store(true, Ordering::Release);
        crate::serial_println!(
            "BOUCHAUD_ACPI_S5 etat=pret pm1a={:#06x} pm1b={:#06x} slp_typ_a={} slp_typ_b={} dsdt={:#x}",
            pm1a, pm1b, typ_a, typ_b, dsdt,
        );
    }
}

/// La machine sait-elle s'eteindre par ACPI ?
pub fn disponible() -> bool {
    PRET.load(Ordering::Acquire)
}

/// Ecrit la sequence d'extinction S5. Ne rend la main que si elle a echoue.
///
/// # Passer ACPI en main du systeme d'abord
///
/// Sur une machine demarree en mode ACPI-par-le-firmware, `SCI_EN` est a zero
/// et le bloc PM1 n'obeit pas encore. Ecrire `ACPI_ENABLE` dans `SMI_CMD`
/// demande au firmware de rendre la main ; l'attente qui suit est BORNEE, et
/// les interruptions ne sont pas masquees pendant celle-ci.
pub fn eteint() -> bool {
    if !PRET.load(Ordering::Acquire) {
        return false;
    }
    let pm1a = PM1A_CNT.load(Ordering::Acquire);
    let pm1b = PM1B_CNT.load(Ordering::Acquire);
    let typ_a = SLP_TYP_A.load(Ordering::Acquire) as u16;
    let typ_b = SLP_TYP_B.load(Ordering::Acquire) as u16;
    let smi_cmd = SMI_CMD.load(Ordering::Acquire);
    let acpi_enable = ACPI_ENABLE.load(Ordering::Acquire);

    unsafe {
        if smi_cmd != 0 && acpi_enable != 0 && inw(pm1a) & SCI_EN == 0 {
            crate::arch::x86_64::ports::outb(smi_cmd, acpi_enable);
            // Trois cents millisecondes au plus, en relachant le processeur :
            // le firmware repond en general en moins de dix.
            let limite = crate::kernel::timer::monotonic_ns() + 300_000_000;
            while inw(pm1a) & SCI_EN == 0 {
                if crate::kernel::timer::monotonic_ns() >= limite {
                    crate::serial_println!(
                        "BOUCHAUD_ACPI_S5 sci_en=timeout : on tente l'ecriture quand meme"
                    );
                    break;
                }
                core::hint::spin_loop();
            }
        }
        crate::serial_println!(
            "BOUCHAUD_ACPI_S5 extinction pm1a={:#06x} valeur={:#06x}",
            pm1a,
            (typ_a << SLP_TYP_SHIFT) | SLP_EN,
        );
        outw(pm1a, (typ_a << SLP_TYP_SHIFT) | SLP_EN);
        if pm1b != 0 {
            outw(pm1b, (typ_b << SLP_TYP_SHIFT) | SLP_EN);
        }
        // Le courant tombe pendant cette boucle sur une machine qui a obei.
        for _ in 0..1_000_000 {
            core::hint::spin_loop();
        }
    }
    false
}
