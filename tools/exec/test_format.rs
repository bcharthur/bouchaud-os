//! Harnais de test hote pour l'identification des formats d'executable.
//!
//! Meme principe que `tools/gui/test_protocole.rs` : reconnaitre un format est
//! une fonction des seuls octets du fichier, donc exercable sans QEMU. Les
//! modules du noyau sont inclus tels quels -- le code teste est exactement
//! celui qui tourne sur la machine.
//!
//! Lance par `tools/exec/test-format.sh`.

#[path = "../../src/kernel/process/loader/format.rs"]
mod format;

#[path = "../../src/kernel/process/loader/pe.rs"]
mod pe;

// `pe.rs` fait `use super::format::...`. Les deux modules etant declares au
// meme niveau ici, `super` designe la racine du harnais et la resolution est
// exactement celle du noyau.
use format::{identifie, Format};

/// Fabrique un en-tete PE32+ minimal mais valide.
fn pe_minimal(machine: u16, magic_optionnel: u16, caracteristiques: u16) -> Vec<u8> {
    let offset_pe = 0x80usize;
    let mut image = vec![0u8; 0x200];
    image[0] = b'M';
    image[1] = b'Z';
    image[0x3c..0x40].copy_from_slice(&(offset_pe as u32).to_le_bytes());
    image[offset_pe..offset_pe + 4].copy_from_slice(b"PE\0\0");

    let coff = offset_pe + 4;
    image[coff..coff + 2].copy_from_slice(&machine.to_le_bytes());
    image[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes()); // 1 section
    image[coff + 16..coff + 18].copy_from_slice(&240u16.to_le_bytes()); // taille optionnel
    image[coff + 18..coff + 20].copy_from_slice(&caracteristiques.to_le_bytes());

    let opt = coff + 20;
    image[opt..opt + 2].copy_from_slice(&magic_optionnel.to_le_bytes());
    image[opt + 16..opt + 20].copy_from_slice(&0x1000u32.to_le_bytes()); // entree
    image[opt + 24..opt + 32].copy_from_slice(&0x0000_0001_4000_0000u64.to_le_bytes());
    image[opt + 32..opt + 36].copy_from_slice(&0x1000u32.to_le_bytes());
    image[opt + 36..opt + 40].copy_from_slice(&0x200u32.to_le_bytes());
    image[opt + 68..opt + 70].copy_from_slice(&3u16.to_le_bytes()); // CUI
    image[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes()); // repertoires
    image
}

#[test]
fn un_elf_est_reconnu_comme_elf() {
    let mut image = vec![0u8; 64];
    image[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
    assert_eq!(identifie(&image), Format::Elf64);
}

#[test]
fn un_pe_est_reconnu_comme_pe() {
    let image = pe_minimal(pe::MACHINE_AMD64, pe::OPTIONAL_PE32PLUS, pe::FILE_EXECUTABLE);
    assert_eq!(identifie(&image), Format::Pe32Plus);
}

#[test]
fn un_mz_sans_signature_pe_n_est_pas_un_pe() {
    // Un vieux binaire DOS commence aussi par MZ. Le prendre pour un PE32+
    // ferait lire des champs qui n'existent pas.
    let mut image = vec![0u8; 0x200];
    image[0] = b'M';
    image[1] = b'Z';
    image[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    // ... mais rien a l'offset 0x80.
    assert_eq!(identifie(&image), Format::Inconnu);
}

#[test]
fn un_offset_pe_qui_deborde_ne_fait_pas_lire_hors_du_fichier() {
    let mut image = vec![0u8; 0x100];
    image[0] = b'M';
    image[1] = b'Z';
    image[0x3c..0x40].copy_from_slice(&0xffff_fff0u32.to_le_bytes());
    assert_eq!(identifie(&image), Format::Inconnu);
    assert_eq!(format::offset_pe(&image), None);
}

#[test]
fn un_script_est_reconnu() {
    assert_eq!(identifie(b"#!/bin/sh\necho"), Format::Script);
}

#[test]
fn un_fichier_vide_ou_court_ne_panique_pas() {
    for taille in 0..8 {
        assert_eq!(identifie(&vec![0u8; taille]), Format::Inconnu);
    }
}

#[test]
fn l_entete_pe_est_lu_correctement() {
    let image = pe_minimal(pe::MACHINE_AMD64, pe::OPTIONAL_PE32PLUS, pe::FILE_EXECUTABLE);
    let entete = pe::lit_entete(&image).expect("en-tete valide");
    assert_eq!(entete.machine, pe::MACHINE_AMD64);
    assert_eq!(entete.magic_optionnel, pe::OPTIONAL_PE32PLUS);
    assert_eq!(entete.point_entree_rva, 0x1000);
    assert_eq!(entete.base_image, 0x1_4000_0000);
    assert_eq!(entete.alignement_section, 0x1000);
    assert_eq!(entete.alignement_fichier, 0x200);
    assert_eq!(entete.sous_systeme, pe::SUBSYSTEM_WINDOWS_CUI);
    assert_eq!(entete.nombre_sections, 1);
}

#[test]
fn un_pe_32_bits_est_refuse_en_le_disant() {
    // Le message doit distinguer « 32 bits » de « illisible » : ce sont deux
    // problemes differents pour celui qui a compile le binaire.
    let image = pe_minimal(pe::MACHINE_AMD64, pe::OPTIONAL_PE32, pe::FILE_EXECUTABLE);
    assert_eq!(pe::lit_entete(&image), Err(pe::RefusPe::PeTrenteDeuxBits));
}

#[test]
fn une_autre_architecture_est_refusee_en_la_nommant() {
    const MACHINE_ARM64: u16 = 0xaa64;
    let image = pe_minimal(MACHINE_ARM64, pe::OPTIONAL_PE32PLUS, pe::FILE_EXECUTABLE);
    assert_eq!(pe::lit_entete(&image), Err(pe::RefusPe::Machine(MACHINE_ARM64)));
}

#[test]
fn une_dll_n_est_pas_un_executable() {
    let image = pe_minimal(
        pe::MACHINE_AMD64,
        pe::OPTIONAL_PE32PLUS,
        pe::FILE_EXECUTABLE | pe::FILE_DLL,
    );
    assert_eq!(pe::lit_entete(&image), Err(pe::RefusPe::EstUneDll));
}

#[test]
fn les_bibliotheques_windows_sont_reconnues_quelle_que_soit_la_casse() {
    for nom in [
        &b"kernel32.dll"[..], b"KERNEL32.DLL", b"KeRnEl32.Dll",
        b"user32.dll", b"ntdll.dll", b"msvcrt.dll", b"ntdll",
    ] {
        assert!(pe::est_bibliotheque_windows(nom), "{:?}", core::str::from_utf8(nom));
    }
}

#[test]
fn une_bibliotheque_bouchaud_n_est_pas_prise_pour_windows() {
    // C'est le sens meme du test : un .exe compile pour le runtime Bouchaud
    // doit passer. Le confondre avec un binaire Windows le refuserait a tort.
    for nom in [
        &b"bouchaud.dll"[..], b"bo-runtime.dll", b"kernel32x.dll", b"user.dll", b"",
    ] {
        assert!(!pe::est_bibliotheque_windows(nom), "{:?}", core::str::from_utf8(nom));
    }
}
