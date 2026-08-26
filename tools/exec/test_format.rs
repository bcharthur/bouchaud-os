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

#[path = "../../src/kernel/process/loader/image.rs"]
mod image;

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

// ---------------------------------------------------------------------------
// Sections, relocations, imports
// ---------------------------------------------------------------------------

/// La fixture `hello.exe`, generee par `tools/exec/fabrique-hello-exe.py`.
///
/// Le generateur et le parseur sont ecrits separement, l'un en Python, l'autre
/// en Rust, a partir de la specification et non l'un de l'autre. Qu'ils
/// s'accordent est donc une VERIFICATION, pas une tautologie.
fn hello_exe() -> Vec<u8> {
    // Le chemin est fourni par `test-format.sh`, qui a lance le generateur.
    // Passer par une variable plutot que par un chemin en dur laisse le test
    // fonctionner quel que soit le repertoire courant.
    let chemin = match std::env::var("BO_HELLO_EXE") {
        Ok(valeur) => valeur,
        Err(_) => {
            panic!(
                "BO_HELLO_EXE absent : lance ce test par tools/exec/test-format.sh, \
                 qui fabrique la fixture d'abord"
            )
        }
    };
    std::fs::read(&chemin).unwrap_or_else(|e| panic!("{} illisible : {}", chemin, e))
}

fn sections_de(image: &[u8]) -> (pe::EnTetePe, Vec<pe::Section>) {
    let entete = pe::lit_entete(image).expect("en-tete valide");
    let mut tampon = [pe::Section {
        nom: [0; 8], taille_virtuelle: 0, rva: 0,
        taille_brute: 0, offset_brut: 0, caracteristiques: 0,
    }; 16];
    let nombre = pe::lit_sections(image, &entete, &mut tampon).expect("sections valides");
    (entete, tampon[..nombre].to_vec())
}

/// F : le parseur de sections lit ce que le generateur a ecrit.
#[test]
fn les_sections_sont_lues_avec_leurs_protections() {
    let image = hello_exe();
    let (entete, sections) = sections_de(&image);
    assert_eq!(sections.len(), 2, "text et rdata");
    assert_eq!(entete.base_image, 0x1_4000_0000);
    assert_eq!(entete.point_entree_rva, 0x1000);

    let texte = &sections[0];
    assert_eq!(&texte.nom[..5], b".text");
    assert!(texte.executable() && texte.lisible());
    assert!(!texte.inscriptible(), "le code n'est pas inscriptible");
    assert!(!texte.viole_w_xor_x(), "W^X respecte");

    let donnees = &sections[1];
    assert_eq!(&donnees.nom[..6], b".rdata");
    assert!(donnees.lisible() && !donnees.executable());
}

/// Une section dont les octets debordent du fichier est refusee, pas bornee.
#[test]
fn une_section_hors_fichier_est_refusee() {
    let mut image = hello_exe();
    let entete = pe::lit_entete(&image).unwrap();
    // Gonfler la taille brute de `.text` au-dela du fichier.
    let base = entete.offset_sections + 16;
    image[base..base + 4].copy_from_slice(&0x00ff_ffffu32.to_le_bytes());
    let mut tampon = [pe::Section {
        nom: [0; 8], taille_virtuelle: 0, rva: 0,
        taille_brute: 0, offset_brut: 0, caracteristiques: 0,
    }; 16];
    let resultat = pe::lit_sections(&image, &entete, &mut tampon);
    assert!(
        matches!(resultat, Err(pe::RefusPe::SectionHorsFichier { .. })),
        "obtenu {:?}", resultat
    );
}

/// G : la relocation DIR64 de la fixture est trouvee, et une seule.
#[test]
fn la_relocation_dir64_est_lue() {
    let image = hello_exe();
    let (entete, sections) = sections_de(&image);

    // Le repertoire 5 porte la table de relocations. Le generateur y met un
    // unique DIR64, sur l'immediat 64 bits du `movabs`.
    let optionnel = pe::offset_optionnel(&image).expect("offset optionnel");
    let rva = u32::from_le_bytes(image[optionnel + 112 + 40..optionnel + 112 + 44].try_into().unwrap());
    let taille = u32::from_le_bytes(image[optionnel + 112 + 44..optionnel + 112 + 48].try_into().unwrap());
    assert!(rva != 0 && taille != 0, "table de relocations presente");

    let mut vues = Vec::new();
    let comptees = pe::parcourt_relocations(
        &image, rva, taille,
        |r| pe::offset_de_rva(&sections, r),
        |relocation| vues.push(relocation),
    ).expect("table lisible");

    assert_eq!(comptees, 1, "une seule relocation utile");
    assert_eq!(vues[0].genre, pe::REL_DIR64);
    // Elle vise bien un octet dans `.text`.
    assert!(vues[0].rva >= 0x1000 && vues[0].rva < 0x2000, "rva={:#x}", vues[0].rva);
    let _ = entete;
}

/// Un bloc de relocation incoherent ne fait ni boucler ni lire hors fichier.
#[test]
fn un_bloc_de_relocation_absurde_est_refuse() {
    let sections = [pe::Section {
        nom: *b".fake   ", taille_virtuelle: 0x1000, rva: 0x1000,
        taille_brute: 0x40, offset_brut: 0, caracteristiques: 0,
    }];
    // Bloc annoncant une taille de 4 : plus petit que son propre en-tete.
    let mut data = vec![0u8; 0x40];
    data[0..4].copy_from_slice(&0x1000u32.to_le_bytes());
    data[4..8].copy_from_slice(&4u32.to_le_bytes());
    let resultat = pe::parcourt_relocations(
        &data, 0x1000, 16,
        |r| pe::offset_de_rva(&sections, r),
        |_| panic!("aucune entree ne doit sortir"),
    );
    assert_eq!(resultat, Err(pe::RefusPe::RelocationsHorsFichier));
}

/// H + I : la fixture n'importe rien, donc elle est reconnue comme native.
#[test]
fn hello_exe_est_une_image_bouchaud_sans_importation() {
    let image = hello_exe();
    let (entete, sections) = sections_de(&image);
    let verdict = pe::classe_dependances(&image, &entete, |r| pe::offset_de_rva(&sections, r))
        .expect("table d'import lisible");
    assert_eq!(verdict, pe::Dependances::Aucune);
}

/// H : une image qui importe kernel32 est classee Windows, en nommant la
/// bibliotheque fautive.
#[test]
fn une_image_qui_importe_kernel32_est_classee_windows() {
    // Une section unique porte a la fois le descripteur et le nom.
    let sections = [pe::Section {
        nom: *b".idata  ", taille_virtuelle: 0x200, rva: 0x1000,
        taille_brute: 0x200, offset_brut: 0, caracteristiques: 0,
    }];
    let mut data = vec![0u8; 0x200];
    // Descripteur : le RVA du nom est en offset 12.
    data[12..16].copy_from_slice(&(0x1000u32 + 0x40).to_le_bytes());
    data[0x40..0x40 + 12].copy_from_slice(b"KERNEL32.dll");
    // Terminateur nul apres le premier descripteur.

    let entete = pe::EnTetePe {
        machine: pe::MACHINE_AMD64, nombre_sections: 1, caracteristiques: 0,
        magic_optionnel: pe::OPTIONAL_PE32PLUS, point_entree_rva: 0x1000,
        base_image: 0x1_4000_0000, alignement_section: 0x1000,
        alignement_fichier: 0x200, sous_systeme: pe::SUBSYSTEM_WINDOWS_CUI,
        taille_image: 0x2000, taille_entetes: 0x200,
        import_rva: 0x1000, import_taille: 20, offset_sections: 0,
    };
    let verdict = pe::classe_dependances(&data, &entete, |r| pe::offset_de_rva(&sections, r))
        .expect("lisible");
    match verdict {
        pe::Dependances::Windows { offset_nom } => {
            assert_eq!(&data[offset_nom..offset_nom + 8], b"KERNEL32");
        }
        autre => panic!("attendu Windows, obtenu {:?}", autre),
    }
}

/// Une RVA que ne couvre aucune section ne se devine pas.
#[test]
fn une_rva_sans_section_ne_rend_aucun_offset() {
    let sections = [pe::Section {
        nom: *b".text   ", taille_virtuelle: 0x100, rva: 0x1000,
        taille_brute: 0x100, offset_brut: 0x200, caracteristiques: 0,
    }];
    assert_eq!(pe::offset_de_rva(&sections, 0x1000), Some(0x200));
    assert_eq!(pe::offset_de_rva(&sections, 0x10ff), Some(0x2ff));
    assert_eq!(pe::offset_de_rva(&sections, 0x0fff), None);
    assert_eq!(pe::offset_de_rva(&sections, 0x1100), None);
}

// ---------------------------------------------------------------------------
// Durcissement : l'entree est traitee comme hostile
// ---------------------------------------------------------------------------

fn entete_type() -> pe::EnTetePe {
    pe::EnTetePe {
        machine: pe::MACHINE_AMD64,
        nombre_sections: 2,
        caracteristiques: pe::FILE_EXECUTABLE,
        magic_optionnel: pe::OPTIONAL_PE32PLUS,
        point_entree_rva: 0x1000,
        base_image: 0x1_4000_0000,
        alignement_section: 0x1000,
        alignement_fichier: 0x200,
        sous_systeme: pe::SUBSYSTEM_WINDOWS_CUI,
        taille_image: 0x4000,
        taille_entetes: 0x400,
        import_rva: 0,
        import_taille: 0,
        offset_sections: 0,
    }
}

fn section(rva: u32, taille: u32, caracteristiques: u32) -> pe::Section {
    pe::Section {
        nom: *b"........",
        taille_virtuelle: taille,
        rva,
        taille_brute: taille,
        offset_brut: 0x400,
        caracteristiques,
    }
}

const CODE: u32 = pe::SCN_MEM_EXECUTE | pe::SCN_MEM_READ;
const DONNEES: u32 = pe::SCN_MEM_READ | pe::SCN_MEM_WRITE;

/// Une image qui annonce plus de sections que le chargeur ne peut en tenir est
/// REFUSEE, pas tronquee.
#[test]
fn test_pe_too_many_sections() {
    let image = hello_exe();
    let mut entete = pe::lit_entete(&image).expect("valide");
    entete.nombre_sections = 50;

    let mut tampon = [section(0, 0, 0); 32];
    let resultat = pe::lit_sections(&image, &entete, &mut tampon);
    match resultat {
        Err(pe::RefusPe::TropDeSections { annoncees, capacite }) => {
            assert_eq!(annoncees, 50);
            assert_eq!(capacite, 32);
        }
        autre => panic!(
            "une image de 50 sections doit etre refusee, pas chargee a moitie : {:?}",
            autre
        ),
    }
}

#[test]
fn test_pe_section_overlap() {
    // Deux sections qui se recouvrent donnent une projection dependant de
    // l'ORDRE de mapping. On refuse plutot que de figer ce hasard.
    let sections = [section(0x1000, 0x2000, CODE), section(0x2000, 0x1000, DONNEES)];
    let resultat = pe::valide_avant_projection(&entete_type(), &sections);
    assert!(
        matches!(resultat, Err(pe::RefusPe::SectionsQuiSeRecouvrent { .. })),
        "obtenu {:?}", resultat
    );

    // Adjacentes sans recouvrement : acceptees.
    let sections = [section(0x1000, 0x1000, CODE), section(0x2000, 0x1000, DONNEES)];
    assert_eq!(pe::valide_avant_projection(&entete_type(), &sections), Ok(()));
}

#[test]
fn test_pe_bad_entry() {
    // Point d'entree dans une section de DONNEES : refuse.
    let sections = [section(0x1000, 0x1000, DONNEES), section(0x2000, 0x1000, CODE)];
    let resultat = pe::valide_avant_projection(&entete_type(), &sections);
    assert!(
        matches!(resultat, Err(pe::RefusPe::PointEntreeInvalide { rva: 0x1000 })),
        "obtenu {:?}", resultat
    );

    // Point d'entree hors de toute section : refuse aussi.
    let mut entete = entete_type();
    entete.point_entree_rva = 0x3500;
    let sections = [section(0x1000, 0x1000, CODE)];
    assert!(matches!(
        pe::valide_avant_projection(&entete, &sections),
        Err(pe::RefusPe::PointEntreeInvalide { .. })
    ));
}

#[test]
fn test_pe_wx() {
    // W^X : une section a la fois inscriptible et executable est refusee. Le
    // format l'autorise ; ce noyau ne l'offrira pas.
    let sections = [section(0x1000, 0x1000, CODE | pe::SCN_MEM_WRITE)];
    let resultat = pe::valide_avant_projection(&entete_type(), &sections);
    assert!(
        matches!(resultat, Err(pe::RefusPe::EcritureEtExecution { rva: 0x1000 })),
        "obtenu {:?}", resultat
    );
}

#[test]
fn test_pe_geometrie_incoherente() {
    // Alignements non puissances de deux, SizeOfHeaders qui depasse
    // SizeOfImage, SizeOfImage nul : chacun a son message.
    let sections = [section(0x1000, 0x1000, CODE)];

    let mut entete = entete_type();
    entete.alignement_section = 0x1234;
    assert!(matches!(
        pe::valide_avant_projection(&entete, &sections),
        Err(pe::RefusPe::GeometrieIncoherente(_))
    ));

    let mut entete = entete_type();
    entete.alignement_fichier = 0x4000; // superieur a SectionAlignment
    assert!(matches!(
        pe::valide_avant_projection(&entete, &sections),
        Err(pe::RefusPe::GeometrieIncoherente(_))
    ));

    let mut entete = entete_type();
    entete.taille_entetes = 0x9000; // depasse SizeOfImage
    assert!(matches!(
        pe::valide_avant_projection(&entete, &sections),
        Err(pe::RefusPe::GeometrieIncoherente(_))
    ));

    let mut entete = entete_type();
    entete.taille_image = 0;
    assert!(matches!(
        pe::valide_avant_projection(&entete, &sections),
        Err(pe::RefusPe::GeometrieIncoherente(_))
    ));
}

#[test]
fn test_pe_section_hors_image() {
    // Une section qui deborde SizeOfImage : sans ce test, le calcul d'adresse
    // designerait une page hors de l'image reservee.
    let sections = [section(0x3800, 0x2000, CODE)];
    let mut entete = entete_type();
    entete.point_entree_rva = 0x3800;
    assert!(matches!(
        pe::valide_avant_projection(&entete, &sections),
        Err(pe::RefusPe::SectionHorsFichier { .. })
    ));
}

#[test]
fn test_pe_image_trop_grande() {
    // `ImageBase + SizeOfImage` qui deborde l'espace d'adressage : le calcul
    // d'adresse bouclerait et designerait une page sans rapport.
    let sections = [section(0x1000, 0x1000, CODE)];
    let mut entete = entete_type();
    entete.base_image = u64::MAX - 0x100;
    assert_eq!(
        pe::valide_avant_projection(&entete, &sections),
        Err(pe::RefusPe::ImageTropGrande)
    );
}

#[test]
fn test_pe_bad_relocation() {
    // Une relocation qui vise hors de l'image ne doit pas etre appliquee : elle
    // ecrirait huit octets a une adresse arbitraire du processus.
    let image = hello_exe();
    let (entete, sections) = sections_de(&image);
    let optionnel = pe::offset_optionnel(&image).expect("offset");
    let rva = u32::from_le_bytes(
        image[optionnel + 152..optionnel + 156].try_into().unwrap(),
    );
    let taille = u32::from_le_bytes(
        image[optionnel + 156..optionnel + 160].try_into().unwrap(),
    );

    let mut hors_image = 0usize;
    pe::parcourt_relocations(&image, rva, taille, |r| pe::offset_de_rva(&sections, r), |reloc| {
        if reloc.rva >= entete.taille_image {
            hors_image += 1;
        }
    })
    .expect("table lisible");
    assert_eq!(hors_image, 0, "la fixture ne vise que l'interieur de l'image");

    // Une table qui pointe hors du fichier est refusee.
    assert_eq!(
        pe::parcourt_relocations(&image, 0x7fff_0000, 16,
            |r| pe::offset_de_rva(&sections, r), |_| {}),
        Err(pe::RefusPe::RelocationsHorsFichier)
    );
}

/// La fixture reelle passe toute la validation : sans ce test, les precedents
/// prouveraient seulement qu'on sait dire non.
#[test]
fn test_pe_hello_exe_passe_la_validation() {
    let image = hello_exe();
    let (entete, sections) = sections_de(&image);
    assert_eq!(
        pe::valide_avant_projection(&entete, &sections), Ok(()),
        "hello.exe doit etre accepte",
    );
}
