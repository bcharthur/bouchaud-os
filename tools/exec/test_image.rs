//! Harnais de test hote pour la PREPARATION d'une image executable.
//!
//! # Ce qui est teste
//!
//! `pe::prepare` transforme des octets en [`ImagePreparee`] : une base, un
//! point d'entree absolu, et des segments a projeter. C'est une fonction des
//! seuls octets d'entree -- aucun espace d'adressage, aucune tache, aucun
//! verrou --, donc entierement exercable sans QEMU.
//!
//! # Ce qui n'est PAS teste ici, et ne peut pas l'etre
//!
//! La projection. Aucun test hote ne peut prouver qu'une image PE s'execute :
//! cela demande un espace d'adressage, un `iretq` et une machine. Ces tests
//! prouvent que la DESCRIPTION est correcte, pas qu'un `.exe` fonctionne.
//!
//! Lance par `tools/exec/test-image.sh`.

#[path = "../../src/kernel/process/loader/format.rs"]
mod format;

#[path = "../../src/kernel/process/loader/image.rs"]
mod image;

#[path = "../../src/kernel/process/loader/pe.rs"]
mod pe;

use image::RefusImage;
use pe::{RefusPe, RefusPreparation};

const BASE: u64 = 0x0000_0001_4000_0000;
const ALIGN_SECTION: u32 = 0x1000;
const ALIGN_FICHIER: u32 = 0x200;

/// Un constructeur de PE32+ minimal mais REEL : en-tetes coherents, sections
/// alignees, point d'entree dans du code.
struct Fabrique {
    sections: Vec<(([u8; 8]), u32, u32, u32, u32, u32)>, // nom, rva, vsize, raw_off, raw_size, carac
    entree_rva: u32,
    taille_image: u32,
    reloc_rva: u32,
    reloc_taille: u32,
    import_rva: u32,
    import_taille: u32,
    corps: Vec<(usize, Vec<u8>)>,
}

const OFFSET_PE: usize = 0x80;
const TAILLE_OPTIONNEL: u16 = 240;

fn nom(texte: &str) -> [u8; 8] {
    let mut n = [0u8; 8];
    let octets = texte.as_bytes();
    n[..octets.len().min(8)].copy_from_slice(&octets[..octets.len().min(8)]);
    n
}

impl Fabrique {
    fn neuve() -> Self {
        Self {
            sections: Vec::new(),
            entree_rva: 0x1000,
            taille_image: 0x3000,
            reloc_rva: 0,
            reloc_taille: 0,
            import_rva: 0,
            import_taille: 0,
            corps: Vec::new(),
        }
    }

    fn section(mut self, n: &str, rva: u32, vsize: u32, raw_off: u32, raw_size: u32, carac: u32) -> Self {
        self.sections.push((nom(n), rva, vsize, raw_off, raw_size, carac));
        self
    }

    fn octets(mut self, offset: usize, data: Vec<u8>) -> Self {
        self.corps.push((offset, data));
        self
    }

    fn construit(&self) -> Vec<u8> {
        let taille_fichier = 0x4000usize;
        let mut image = vec![0u8; taille_fichier];
        image[0] = b'M';
        image[1] = b'Z';
        image[0x3c..0x40].copy_from_slice(&(OFFSET_PE as u32).to_le_bytes());
        image[OFFSET_PE..OFFSET_PE + 4].copy_from_slice(b"PE\0\0");

        let coff = OFFSET_PE + 4;
        image[coff..coff + 2].copy_from_slice(&pe::MACHINE_AMD64.to_le_bytes());
        image[coff + 2..coff + 4]
            .copy_from_slice(&(self.sections.len() as u16).to_le_bytes());
        image[coff + 16..coff + 18].copy_from_slice(&TAILLE_OPTIONNEL.to_le_bytes());
        image[coff + 18..coff + 20]
            .copy_from_slice(&pe::FILE_EXECUTABLE.to_le_bytes());

        let opt = coff + 20;
        image[opt..opt + 2].copy_from_slice(&pe::OPTIONAL_PE32PLUS.to_le_bytes());
        image[opt + 16..opt + 20].copy_from_slice(&self.entree_rva.to_le_bytes());
        image[opt + 24..opt + 32].copy_from_slice(&BASE.to_le_bytes());
        image[opt + 32..opt + 36].copy_from_slice(&ALIGN_SECTION.to_le_bytes());
        image[opt + 36..opt + 40].copy_from_slice(&ALIGN_FICHIER.to_le_bytes());
        image[opt + 56..opt + 60].copy_from_slice(&self.taille_image.to_le_bytes());
        image[opt + 60..opt + 64].copy_from_slice(&0x400u32.to_le_bytes()); // SizeOfHeaders
        image[opt + 68..opt + 70]
            .copy_from_slice(&pe::SUBSYSTEM_WINDOWS_CUI.to_le_bytes());
        image[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes()); // repertoires

        // Repertoire 1 : imports. Repertoire 5 : relocations.
        let rep = opt + 112;
        image[rep + 8..rep + 12].copy_from_slice(&self.import_rva.to_le_bytes());
        image[rep + 12..rep + 16].copy_from_slice(&self.import_taille.to_le_bytes());
        image[rep + 40..rep + 44].copy_from_slice(&self.reloc_rva.to_le_bytes());
        image[rep + 44..rep + 48].copy_from_slice(&self.reloc_taille.to_le_bytes());

        let mut curseur = opt + TAILLE_OPTIONNEL as usize;
        for (n, rva, vsize, raw_off, raw_size, carac) in &self.sections {
            image[curseur..curseur + 8].copy_from_slice(n);
            image[curseur + 8..curseur + 12].copy_from_slice(&vsize.to_le_bytes());
            image[curseur + 12..curseur + 16].copy_from_slice(&rva.to_le_bytes());
            image[curseur + 16..curseur + 20].copy_from_slice(&raw_size.to_le_bytes());
            image[curseur + 20..curseur + 24].copy_from_slice(&raw_off.to_le_bytes());
            image[curseur + 36..curseur + 40].copy_from_slice(&carac.to_le_bytes());
            curseur += pe::TAILLE_SECTION;
        }

        for (offset, data) in &self.corps {
            image[*offset..*offset + data.len()].copy_from_slice(data);
        }
        image
    }
}

/// Le cas nominal : du code, des donnees, du BSS.
fn hello_minimal() -> Fabrique {
    Fabrique::neuve()
        .section(".text", 0x1000, 0x100, 0x400, 0x200,
                 pe::SCN_MEM_READ | pe::SCN_MEM_EXECUTE)
        .section(".data", 0x2000, 0x080, 0x600, 0x200,
                 pe::SCN_MEM_READ | pe::SCN_MEM_WRITE)
}

// ------------------------------------------------------------------ nominal

#[test]
fn une_image_saine_produit_ses_segments() {
    let data = hello_minimal().construit();
    let image = pe::prepare(&data, None).expect("image saine");

    assert_eq!(image.base, BASE);
    assert_eq!(image.point_entree, BASE + 0x1000, "point d'entree ABSOLU");
    assert_eq!(image.decalage, 0, "chargee a sa base : rien a relocaliser");

    // En-tetes + deux sections.
    let segments = image.segments();
    assert_eq!(segments.len(), 3, "en-tetes plus une section chacune");

    let entetes = &segments[0];
    assert_eq!(entetes.adresse, BASE);
    assert!(entetes.droits.lecture && !entetes.droits.ecriture && !entetes.droits.execution,
            "les en-tetes sont projetes en lecture seule");

    let texte = &segments[1];
    assert_eq!(texte.adresse, BASE + 0x1000);
    assert!(texte.droits.execution && !texte.droits.ecriture);
    assert_eq!(texte.offset_source, 0x400);

    let donnees = &segments[2];
    assert_eq!(donnees.adresse, BASE + 0x2000);
    assert!(donnees.droits.ecriture && !donnees.droits.execution);
}

#[test]
fn une_section_bss_ne_lit_rien_du_fichier() {
    let data = Fabrique::neuve()
        .section(".text", 0x1000, 0x100, 0x400, 0x200,
                 pe::SCN_MEM_READ | pe::SCN_MEM_EXECUTE)
        .section(".bss", 0x2000, 0x1000, 0, 0,
                 pe::SCN_MEM_READ | pe::SCN_MEM_WRITE | pe::SCN_CNT_BSS)
        .construit();
    let image = pe::prepare(&data, None).expect("image avec BSS");

    let bss = image.segments().iter().find(|s| s.adresse == BASE + 0x2000).unwrap();
    assert_eq!(bss.taille_source, 0, "un BSS n'a aucune source");
    assert_eq!(bss.taille_zero(), bss.taille, "il est entierement fabrique");
    assert_eq!(bss.taille, 0x1000);
}

#[test]
fn la_taille_memoire_prime_sur_la_taille_fichier() {
    // `.data` porte 0x200 octets bruts pour 0x800 en memoire : les 0x600
    // restants sont du zero, et personne ne doit les lire dans le fichier.
    let data = Fabrique::neuve()
        .section(".text", 0x1000, 0x100, 0x400, 0x200,
                 pe::SCN_MEM_READ | pe::SCN_MEM_EXECUTE)
        .section(".data", 0x2000, 0x800, 0x600, 0x200,
                 pe::SCN_MEM_READ | pe::SCN_MEM_WRITE)
        .construit();
    let image = pe::prepare(&data, None).unwrap();
    let donnees = image.segments().iter().find(|s| s.adresse == BASE + 0x2000).unwrap();
    assert_eq!(donnees.taille_source, 0x200);
    assert_eq!(donnees.taille, 0x1000, "arrondi a SectionAlignment");
    assert_eq!(donnees.taille_zero(), 0x1000 - 0x200);
}

// -------------------------------------------------------------- relocations

/// Fabrique un bloc de relocations DIR64 pour une page.
fn bloc_reloc(page_rva: u32, decalages: &[u16]) -> Vec<u8> {
    let taille = 8 + decalages.len() * 2;
    let mut bloc = Vec::new();
    bloc.extend_from_slice(&page_rva.to_le_bytes());
    bloc.extend_from_slice(&(taille as u32).to_le_bytes());
    for offset in decalages {
        bloc.extend_from_slice(&(((pe::REL_DIR64 as u16) << 12) | offset).to_le_bytes());
    }
    bloc
}

fn image_relogeable() -> Vec<u8> {
    let bloc = bloc_reloc(0x2000, &[0x10]);
    Fabrique::neuve()
        .section(".text", 0x1000, 0x100, 0x400, 0x200,
                 pe::SCN_MEM_READ | pe::SCN_MEM_EXECUTE)
        .section(".data", 0x2000, 0x100, 0x600, 0x200,
                 pe::SCN_MEM_READ | pe::SCN_MEM_WRITE)
        .section(".reloc", 0x3000, bloc.len() as u32, 0x800, 0x200,
                 pe::SCN_MEM_READ)
        .octets(0x800, bloc)
        .reloc(0x3000, 12)
        .taille(0x4000)
        .construit()
}

impl Fabrique {
    fn reloc(mut self, rva: u32, taille: u32) -> Self {
        self.reloc_rva = rva;
        self.reloc_taille = taille;
        self
    }
    fn taille(mut self, taille_image: u32) -> Self {
        self.taille_image = taille_image;
        self
    }
    fn import(mut self, rva: u32, taille: u32) -> Self {
        self.import_rva = rva;
        self.import_taille = taille;
        self
    }
}

#[test]
fn une_base_differente_exige_des_relocations() {
    let sans = hello_minimal().construit();
    assert_eq!(
        pe::prepare(&sans, Some(BASE + 0x1000_0000)),
        Err(RefusPreparation::RelocationsAbsentes),
        "deplacer une image sans table de relocations rendrait toutes ses \
         adresses absolues fausses"
    );
}

#[test]
fn une_image_relogeable_accepte_une_autre_base() {
    let data = image_relogeable();
    let nouvelle = BASE + 0x1000_0000;
    let image = pe::prepare(&data, Some(nouvelle)).expect("image relogeable");
    assert_eq!(image.base, nouvelle);
    assert_eq!(image.decalage, 0x1000_0000);
    assert_eq!(image.point_entree, nouvelle + 0x1000);
}

#[test]
fn les_relocations_dir64_ajoutent_le_decalage() {
    let data = image_relogeable();
    let decalage = 0x1000_0000i64;
    let entete = pe::lit_entete(&data).unwrap();
    let mut sections = [pe::Section {
        nom: [0; 8], taille_virtuelle: 0, rva: 0,
        taille_brute: 0, offset_brut: 0, caracteristiques: 0,
    }; 32];
    let nombre = pe::lit_sections(&data, &entete, &mut sections).unwrap();

    // Image projetee : la valeur a reloger vaut BASE + 0x1234.
    let mut projetee = vec![0u8; entete.taille_image as usize];
    let avant = BASE + 0x1234;
    projetee[0x2010..0x2018].copy_from_slice(&avant.to_le_bytes());

    let appliquees = pe::applique_relocations(
        &data, &entete, &sections[..nombre], decalage, &mut projetee,
    ).expect("relocations applicables");

    assert_eq!(appliquees, 1);
    let mut apres = [0u8; 8];
    apres.copy_from_slice(&projetee[0x2010..0x2018]);
    assert_eq!(
        u64::from_le_bytes(apres),
        avant + decalage as u64,
        "DIR64 ajoute le decalage a la valeur en place"
    );
}

#[test]
fn un_decalage_nul_n_applique_aucune_relocation() {
    let data = image_relogeable();
    let entete = pe::lit_entete(&data).unwrap();
    let mut sections = [pe::Section {
        nom: [0; 8], taille_virtuelle: 0, rva: 0,
        taille_brute: 0, offset_brut: 0, caracteristiques: 0,
    }; 32];
    let nombre = pe::lit_sections(&data, &entete, &mut sections).unwrap();
    let mut projetee = vec![0u8; entete.taille_image as usize];
    let temoin = projetee.clone();

    assert_eq!(
        pe::applique_relocations(&data, &entete, &sections[..nombre], 0, &mut projetee),
        Ok(0),
    );
    assert_eq!(projetee, temoin, "aucun octet touche");
}

#[test]
fn un_type_de_relocation_inconnu_est_refuse() {
    // Type 3 (`HIGHLOW`) : produit par du x86 32 bits, jamais par AMD64.
    let mut bloc = Vec::new();
    bloc.extend_from_slice(&0x2000u32.to_le_bytes());
    bloc.extend_from_slice(&12u32.to_le_bytes());
    bloc.extend_from_slice(&((3u16 << 12) | 0x10).to_le_bytes());
    bloc.extend_from_slice(&0u16.to_le_bytes());

    let data = Fabrique::neuve()
        .section(".text", 0x1000, 0x100, 0x400, 0x200,
                 pe::SCN_MEM_READ | pe::SCN_MEM_EXECUTE)
        .section(".data", 0x2000, 0x100, 0x600, 0x200,
                 pe::SCN_MEM_READ | pe::SCN_MEM_WRITE)
        .section(".reloc", 0x3000, bloc.len() as u32, 0x800, 0x200, pe::SCN_MEM_READ)
        .octets(0x800, bloc)
        .reloc(0x3000, 12)
        .taille(0x4000)
        .construit();

    assert_eq!(
        pe::prepare(&data, Some(BASE + 0x1000_0000)),
        Err(RefusPreparation::RelocationInconnue { genre: 3, rva: 0x2010 }),
        "deviner le sens d'un type inconnu serait pire que refuser"
    );
}

// -------------------------------------------------------------------- refus

#[test]
fn une_image_qui_importe_windows_est_refusee_avec_le_nom() {
    // Descripteur d'import unique pointant vers "KERNEL32.dll".
    let mut descripteur = vec![0u8; pe::TAILLE_DESCRIPTEUR_IMPORT * 2];
    descripteur[12..16].copy_from_slice(&0x2100u32.to_le_bytes()); // RVA du nom

    let data = Fabrique::neuve()
        .section(".text", 0x1000, 0x100, 0x400, 0x200,
                 pe::SCN_MEM_READ | pe::SCN_MEM_EXECUTE)
        .section(".idata", 0x2000, 0x200, 0x600, 0x200, pe::SCN_MEM_READ)
        .octets(0x600, descripteur)
        .octets(0x700, b"KERNEL32.dll\0".to_vec())
        .import(0x2000, 40)
        .construit();

    match pe::prepare(&data, None) {
        Err(RefusPreparation::Format(RefusPe::SousSystemeWindows { bibliotheque, longueur })) => {
            assert_eq!(&bibliotheque[..longueur], b"KERNEL32.dll");
        }
        autre => panic!("attendu un refus nommant la bibliotheque, recu {autre:?}"),
    }
}

#[test]
fn une_section_inscriptible_et_executable_est_refusee() {
    let data = Fabrique::neuve()
        .section(".text", 0x1000, 0x100, 0x400, 0x200,
                 pe::SCN_MEM_READ | pe::SCN_MEM_EXECUTE | pe::SCN_MEM_WRITE)
        .construit();
    assert!(
        matches!(
            pe::prepare(&data, None),
            Err(RefusPreparation::Format(RefusPe::EcritureEtExecution { .. }))
        ),
        "W^X est refuse a la preparation, pas seulement a la projection"
    );
}

#[test]
fn un_point_d_entree_hors_du_code_est_refuse() {
    let data = Fabrique::neuve()
        .section(".data", 0x1000, 0x100, 0x400, 0x200,
                 pe::SCN_MEM_READ | pe::SCN_MEM_WRITE)
        .construit();
    assert!(matches!(
        pe::prepare(&data, None),
        Err(RefusPreparation::Format(RefusPe::PointEntreeInvalide { .. }))
    ));
}

// ------------------------------------------------- validation de la description

#[test]
fn la_validation_refuse_des_segments_qui_se_recouvrent() {
    let mut img = image::ImagePreparee::neuve(BASE, BASE, 0x4000, 0);
    img.ajoute(image::Segment {
        adresse: BASE, taille: 0x2000, offset_source: 0, taille_source: 0,
        droits: image::Droits { lecture: true, ecriture: false, execution: true },
    }).unwrap();
    img.ajoute(image::Segment {
        adresse: BASE + 0x1000, taille: 0x1000, offset_source: 0, taille_source: 0,
        droits: image::Droits::lecture_seule(),
    }).unwrap();

    assert!(matches!(
        img.valide(0x4000),
        Err(RefusImage::SegmentsQuiSeRecouvrent { .. })
    ), "l'ordre de projection deciderait du resultat");
}

#[test]
fn la_validation_refuse_une_source_hors_fichier() {
    let mut img = image::ImagePreparee::neuve(BASE, BASE, 0x4000, 0);
    img.ajoute(image::Segment {
        adresse: BASE, taille: 0x1000, offset_source: 0x3F00, taille_source: 0x400,
        droits: image::Droits { lecture: true, ecriture: false, execution: true },
    }).unwrap();
    assert!(matches!(
        img.valide(0x4000),
        Err(RefusImage::SourceHorsFichier { .. })
    ));
}

#[test]
fn la_validation_refuse_une_image_vide() {
    let img = image::ImagePreparee::neuve(BASE, BASE, 0x1000, 0);
    assert_eq!(img.valide(0x1000), Err(RefusImage::Vide));
}

#[test]
fn la_validation_reverifie_w_xor_x() {
    let mut img = image::ImagePreparee::neuve(BASE, BASE, 0x2000, 0);
    img.ajoute(image::Segment {
        adresse: BASE, taille: 0x1000, offset_source: 0, taille_source: 0,
        droits: image::Droits { lecture: true, ecriture: true, execution: true },
    }).unwrap();
    assert!(matches!(
        img.valide(0x1000),
        Err(RefusImage::EcritureEtExecution { .. })
    ), "derniere barriere avant que des pages W+X n'existent reellement");
}

#[test]
fn la_capacite_de_segments_est_bornee_sans_troncature_silencieuse() {
    let mut img = image::ImagePreparee::neuve(BASE, BASE, 0x100000, 0);
    for index in 0..image::MAX_SEGMENTS {
        img.ajoute(image::Segment {
            adresse: BASE + (index as u64) * 0x1000, taille: 0x1000,
            offset_source: 0, taille_source: 0,
            droits: image::Droits::lecture_seule(),
        }).unwrap();
    }
    assert!(matches!(
        img.ajoute(image::Segment {
            adresse: BASE + 0x40000, taille: 0x1000,
            offset_source: 0, taille_source: 0,
            droits: image::Droits::lecture_seule(),
        }),
        Err(RefusImage::TropDeSegments { .. })
    ), "refuser, jamais tronquer en silence");
}
