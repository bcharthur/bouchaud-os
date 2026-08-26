//! Lecture d'un en-tete PE32+ (AMD64), sans rien charger.
//!
//! # Ce que ce module fait, et surtout ne pretend pas faire
//!
//! Il lit, valide et CLASSE. Il ne projette aucune section, n'applique aucune
//! relocation et n'execute rien. Sa raison d'etre immediate est de rendre un
//! verdict exact et explicable sur un `.exe` :
//!
//!   * ce PE vise-t-il bien AMD64 ?
//!   * est-ce un executable, ou une DLL ?
//!   * quel sous-systeme demande-t-il ?
//!   * de quelles bibliotheques depend-il ?
//!
//! La derniere question decide de tout. Un `.exe` compile pour le runtime
//! Bouchaud n'importe que des bibliotheques Bouchaud. Un binaire Windows
//! ordinaire importe `kernel32.dll`, `user32.dll` ou `ntdll.dll` : le charger
//! reviendrait a sauter dans du code qui appellerait aussitot une API Win32
//! inexistante, et le processus mourrait sur une adresse invalide sans que rien
//! n'explique pourquoi. Il vaut mieux le refuser en le NOMMANT.
//!
//! Comme `format.rs`, tout est ici fonction du seul contenu du fichier, donc
//! exercable sur l'hote (`tools/exec/test_format.rs`).

#![allow(dead_code)]

use super::format::{offset_pe, PE_MAGIC};

/// `IMAGE_FILE_MACHINE_AMD64`.
pub const MACHINE_AMD64: u16 = 0x8664;
/// Signature de l'en-tete optionnel PE32+ (64 bits).
pub const OPTIONAL_PE32PLUS: u16 = 0x20b;
/// Signature de l'en-tete optionnel PE32 (32 bits) : reconnue pour pouvoir
/// dire « 32 bits » plutot que « illisible ».
pub const OPTIONAL_PE32: u16 = 0x10b;

/// `IMAGE_FILE_DLL`.
pub const FILE_DLL: u16 = 0x2000;
/// `IMAGE_FILE_EXECUTABLE_IMAGE`.
pub const FILE_EXECUTABLE: u16 = 0x0002;

/// Sous-systemes qui nous concernent.
pub const SUBSYSTEM_NATIVE: u16 = 1;
pub const SUBSYSTEM_WINDOWS_GUI: u16 = 2;
pub const SUBSYSTEM_WINDOWS_CUI: u16 = 3;

/// Ce qu'un en-tete PE32+ dit de son image.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EnTetePe {
    pub machine: u16,
    pub nombre_sections: u16,
    pub caracteristiques: u16,
    pub magic_optionnel: u16,
    pub point_entree_rva: u32,
    pub base_image: u64,
    pub alignement_section: u32,
    pub alignement_fichier: u32,
    pub sous_systeme: u16,
    pub taille_image: u32,
    pub taille_entetes: u32,
    /// Table d'import : adresse virtuelle et taille, 0 si absente.
    pub import_rva: u32,
    pub import_taille: u32,
    /// Offset du premier en-tete de section dans le fichier.
    pub offset_sections: usize,
}

/// Pourquoi une image ne peut pas etre chargee.
///
/// Chaque cas porte de quoi ecrire un message qui nomme le probleme. Un refus
/// qui dit seulement « non » oblige a deviner.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RefusPe {
    Tronque(&'static str),
    PasUnPe,
    Machine(u16),
    PeTrenteDeuxBits,
    MagicOptionnel(u16),
    EstUneDll,
    /// Le binaire importe une bibliotheque Windows : il lui faut une couche
    /// Win32, qui n'existe pas.
    SousSystemeWindows { bibliotheque: [u8; 32], longueur: usize },
    /// Une section annonce des octets hors du fichier.
    SectionHorsFichier { rva: u32 },
    /// La table de relocations deborde, ou un bloc est incoherent.
    RelocationsHorsFichier,
    /// La table d'import pointe hors du fichier.
    ImportsHorsFichier,
    /// Une section demande a la fois l'ecriture et l'execution.
    EcritureEtExecution { rva: u32 },
    /// L'image annonce plus de sections que le chargeur ne peut en tenir.
    TropDeSections { annoncees: u16, capacite: usize },
    /// Deux sections se recouvrent en memoire ou dans le fichier.
    SectionsQuiSeRecouvrent { premiere: u32, seconde: u32 },
    /// Le point d'entree ne tombe dans aucune section executable.
    PointEntreeInvalide { rva: u32 },
    /// `SizeOfImage`, `SizeOfHeaders` ou un alignement est incoherent.
    GeometrieIncoherente(&'static str),
    /// `ImageBase + SizeOfImage` deborde l'espace d'adressage.
    ImageTropGrande,
}

impl RefusPe {
    pub fn nom_bibliotheque(&self) -> Option<&[u8]> {
        match self {
            RefusPe::SousSystemeWindows { bibliotheque, longueur } => {
                Some(&bibliotheque[..*longueur])
            }
            _ => None,
        }
    }
}

#[inline]
fn lit_u16(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*data.get(offset)?, *data.get(offset + 1)?]))
}

#[inline]
fn lit_u32(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes([
        *data.get(offset)?,
        *data.get(offset + 1)?,
        *data.get(offset + 2)?,
        *data.get(offset + 3)?,
    ]))
}

#[inline]
fn lit_u64(data: &[u8], offset: usize) -> Option<u64> {
    let bas = lit_u32(data, offset)? as u64;
    let haut = lit_u32(data, offset + 4)? as u64;
    Some(bas | (haut << 32))
}

/// Lit l'en-tete d'une image PE32+ AMD64.
pub fn lit_entete(data: &[u8]) -> Result<EnTetePe, RefusPe> {
    let pe = offset_pe(data).ok_or(RefusPe::PasUnPe)?;
    if data.len() < pe + 4 || data[pe..pe + 4] != PE_MAGIC {
        return Err(RefusPe::PasUnPe);
    }

    // En-tete COFF : 20 octets apres la signature.
    let coff = pe + 4;
    let machine = lit_u16(data, coff).ok_or(RefusPe::Tronque("en-tete COFF"))?;
    if machine != MACHINE_AMD64 {
        return Err(RefusPe::Machine(machine));
    }
    let nombre_sections = lit_u16(data, coff + 2).ok_or(RefusPe::Tronque("nombre de sections"))?;
    let taille_optionnel = lit_u16(data, coff + 16).ok_or(RefusPe::Tronque("taille de l'en-tete optionnel"))?;
    let caracteristiques = lit_u16(data, coff + 18).ok_or(RefusPe::Tronque("caracteristiques"))?;

    let optionnel = coff + 20;
    let magic = lit_u16(data, optionnel).ok_or(RefusPe::Tronque("en-tete optionnel"))?;
    if magic == OPTIONAL_PE32 {
        return Err(RefusPe::PeTrenteDeuxBits);
    }
    if magic != OPTIONAL_PE32PLUS {
        return Err(RefusPe::MagicOptionnel(magic));
    }
    if caracteristiques & FILE_DLL != 0 {
        return Err(RefusPe::EstUneDll);
    }

    // Champs de l'en-tete optionnel PE32+, aux offsets du format.
    let point_entree_rva = lit_u32(data, optionnel + 16).ok_or(RefusPe::Tronque("point d'entree"))?;
    let base_image = lit_u64(data, optionnel + 24).ok_or(RefusPe::Tronque("base de l'image"))?;
    let alignement_section = lit_u32(data, optionnel + 32).ok_or(RefusPe::Tronque("alignement de section"))?;
    let alignement_fichier = lit_u32(data, optionnel + 36).ok_or(RefusPe::Tronque("alignement de fichier"))?;
    let sous_systeme = lit_u16(data, optionnel + 68).ok_or(RefusPe::Tronque("sous-systeme"))?;
    let taille_image = lit_u32(data, optionnel + 56).ok_or(RefusPe::Tronque("taille de l'image"))?;
    let taille_entetes = lit_u32(data, optionnel + 60).ok_or(RefusPe::Tronque("taille des en-tetes"))?;

    // Repertoires de donnees : le second est la table d'import.
    let nombre_repertoires = lit_u32(data, optionnel + 108).ok_or(RefusPe::Tronque("nombre de repertoires"))?;
    let (import_rva, import_taille) = if nombre_repertoires >= 2 {
        (
            lit_u32(data, optionnel + 112 + 8).unwrap_or(0),
            lit_u32(data, optionnel + 112 + 12).unwrap_or(0),
        )
    } else {
        (0, 0)
    };

    Ok(EnTetePe {
        machine,
        nombre_sections,
        caracteristiques,
        magic_optionnel: magic,
        point_entree_rva,
        base_image,
        alignement_section,
        alignement_fichier,
        sous_systeme,
        taille_image,
        taille_entetes,
        import_rva,
        import_taille,
        offset_sections: optionnel + taille_optionnel as usize,
    })
}

/// Offset de l'en-tete optionnel dans le fichier.
pub fn offset_optionnel(data: &[u8]) -> Option<usize> {
    let pe = offset_pe(data)?;
    if data.len() < pe + 4 || data[pe..pe + 4] != PE_MAGIC {
        return None;
    }
    Some(pe + 4 + 20)
}

/// Bibliotheques dont la presence prouve qu'un binaire attend Windows.
///
/// La liste est volontairement courte et sure : ce sont les DLL que tout
/// programme Windows finit par importer, directement ou par sa runtime C. En
/// reconnaitre une suffit ; en manquer une n'est pas grave, le chargement
/// echouera plus loin et ce module aura au moins dit ce qu'il a vu.
const BIBLIOTHEQUES_WINDOWS: [&[u8]; 8] = [
    b"kernel32", b"user32", b"ntdll", b"gdi32",
    b"advapi32", b"shell32", b"ole32", b"msvcrt",
];

/// Le nom designe-t-il une bibliotheque Windows ?
pub fn est_bibliotheque_windows(nom: &[u8]) -> bool {
    // Comparaison insensible a la casse et sans l'extension : les tables
    // d'import ecrivent aussi bien `KERNEL32.dll` que `kernel32.DLL`.
    let base = match nom.iter().position(|&b| b == b'.') {
        Some(point) => &nom[..point],
        None => nom,
    };
    BIBLIOTHEQUES_WINDOWS.iter().any(|attendue| {
        attendue.len() == base.len()
            && attendue
                .iter()
                .zip(base.iter())
                .all(|(a, b)| a.eq_ignore_ascii_case(b))
    })
}

// ---------------------------------------------------------------------------
// Sections
// ---------------------------------------------------------------------------

/// `IMAGE_SCN_MEM_*` : les seuls drapeaux qui decident d'une protection.
pub const SCN_MEM_EXECUTE: u32 = 0x2000_0000;
pub const SCN_MEM_READ: u32 = 0x4000_0000;
pub const SCN_MEM_WRITE: u32 = 0x8000_0000;
/// `IMAGE_SCN_CNT_UNINITIALIZED_DATA` : `.bss`, sans octets dans le fichier.
pub const SCN_CNT_BSS: u32 = 0x0000_0080;

/// Taille d'un `IMAGE_SECTION_HEADER`.
pub const TAILLE_SECTION: usize = 40;

/// Une section, telle qu'elle devra etre projetee.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Section {
    pub nom: [u8; 8],
    /// Taille en memoire. Peut depasser `taille_brute` : le reste est zero.
    pub taille_virtuelle: u32,
    /// Adresse relative a `base_image`.
    pub rva: u32,
    /// Octets presents dans le fichier.
    pub taille_brute: u32,
    pub offset_brut: u32,
    pub caracteristiques: u32,
}

impl Section {
    pub fn executable(&self) -> bool { self.caracteristiques & SCN_MEM_EXECUTE != 0 }
    pub fn lisible(&self) -> bool { self.caracteristiques & SCN_MEM_READ != 0 }
    pub fn inscriptible(&self) -> bool { self.caracteristiques & SCN_MEM_WRITE != 0 }

    /// Une section a la fois inscriptible et executable viole W^X.
    ///
    /// Le format l'autorise ; ce noyau ne le fera pas. Une image qui l'exige
    /// est refusee plutot que projetee dans un etat qu'on ne veut pas offrir.
    pub fn viole_w_xor_x(&self) -> bool {
        self.executable() && self.inscriptible()
    }
}

/// Lit les en-tetes de section. Le nombre vient de l'en-tete COFF.
pub fn lit_sections(
    data: &[u8],
    entete: &EnTetePe,
    sortie: &mut [Section],
) -> Result<usize, RefusPe> {
    // BOUCHAUD_PE_HARDENING_V1
    //
    // `min(annoncees, capacite)` TRONQUAIT en silence. Une image annoncant
    // cinquante sections aurait ete chargee avec les trente-deux premieres :
    // un programme a moitie projete, dont les sections manquantes sont
    // simplement absentes de l'espace d'adressage. Il aurait faute plus loin,
    // a un endroit sans rapport.
    //
    // Un chargeur de binaire traite son entree comme HOSTILE. Ce qu'il ne peut
    // pas charger entierement, il le refuse.
    let annoncees = entete.nombre_sections;
    if annoncees as usize > sortie.len() {
        return Err(RefusPe::TropDeSections {
            annoncees,
            capacite: sortie.len(),
        });
    }
    let nombre = annoncees as usize;
    for index in 0..nombre {
        let base = entete.offset_sections + index * TAILLE_SECTION;
        if data.len() < base + TAILLE_SECTION {
            return Err(RefusPe::Tronque("en-tete de section"));
        }
        let mut nom = [0u8; 8];
        nom.copy_from_slice(&data[base..base + 8]);
        let section = Section {
            nom,
            taille_virtuelle: lit_u32(data, base + 8).ok_or(RefusPe::Tronque("taille virtuelle"))?,
            rva: lit_u32(data, base + 12).ok_or(RefusPe::Tronque("rva de section"))?,
            taille_brute: lit_u32(data, base + 16).ok_or(RefusPe::Tronque("taille brute"))?,
            offset_brut: lit_u32(data, base + 20).ok_or(RefusPe::Tronque("offset brut"))?,
            caracteristiques: lit_u32(data, base + 36)
                .ok_or(RefusPe::Tronque("caracteristiques de section"))?,
        };
        // Une section dont les octets debordent du fichier ferait lire
        // n'importe quoi. C'est une image invalide, pas une lecture a borner
        // silencieusement : la borner produirait un programme a moitie charge.
        if section.taille_brute > 0 {
            let fin = section.offset_brut as usize + section.taille_brute as usize;
            if fin > data.len() {
                return Err(RefusPe::SectionHorsFichier { rva: section.rva });
            }
        }
        sortie[index] = section;
    }
    Ok(nombre)
}

// ---------------------------------------------------------------------------
// Relocations (base relocation table)
// ---------------------------------------------------------------------------

/// `IMAGE_REL_BASED_ABSOLUTE` : bourrage, a ignorer.
pub const REL_ABSOLUTE: u16 = 0;
/// `IMAGE_REL_BASED_DIR64` : le seul type qu'un binaire AMD64 produit.
pub const REL_DIR64: u16 = 10;

/// Une relocation a appliquer : ajouter le decalage a l'adresse `rva`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Relocation {
    pub rva: u32,
    pub genre: u16,
}

/// Parcourt la table de relocations et remet chaque entree a `visite`.
///
/// Le format est une suite de blocs : huit octets d'en-tete (RVA de page,
/// taille du bloc), puis des entrees de 16 bits dont les 4 bits hauts donnent
/// le type et les 12 bas le decalage dans la page.
///
/// `visite` plutot qu'un `Vec` : ce code doit pouvoir tourner sans allocateur,
/// et une image peut porter des milliers de relocations.
pub fn parcourt_relocations<F: FnMut(Relocation)>(
    data: &[u8],
    rva_table: u32,
    taille_table: u32,
    offset_de_rva: impl Fn(u32) -> Option<usize>,
    mut visite: F,
) -> Result<usize, RefusPe> {
    if rva_table == 0 || taille_table == 0 {
        return Ok(0);
    }
    let debut = offset_de_rva(rva_table).ok_or(RefusPe::RelocationsHorsFichier)?;
    let fin = debut
        .checked_add(taille_table as usize)
        .ok_or(RefusPe::RelocationsHorsFichier)?;
    if fin > data.len() {
        return Err(RefusPe::RelocationsHorsFichier);
    }

    let mut curseur = debut;
    let mut comptees = 0usize;
    while curseur + 8 <= fin {
        let page = lit_u32(data, curseur).ok_or(RefusPe::Tronque("bloc de relocation"))?;
        let taille_bloc =
            lit_u32(data, curseur + 8 - 4).ok_or(RefusPe::Tronque("taille de bloc"))? as usize;
        // Un bloc plus petit que son en-tete, ou qui deborde, ferait boucler
        // sans fin ou lire hors du fichier. Les deux se refusent ici.
        if taille_bloc < 8 || curseur + taille_bloc > fin {
            return Err(RefusPe::RelocationsHorsFichier);
        }
        let mut entree = curseur + 8;
        while entree + 2 <= curseur + taille_bloc {
            let brut = lit_u16(data, entree).ok_or(RefusPe::Tronque("entree de relocation"))?;
            let genre = brut >> 12;
            let decalage = (brut & 0x0fff) as u32;
            if genre != REL_ABSOLUTE {
                visite(Relocation { rva: page + decalage, genre });
                comptees += 1;
            }
            entree += 2;
        }
        curseur += taille_bloc;
    }
    Ok(comptees)
}

// ---------------------------------------------------------------------------
// Imports
// ---------------------------------------------------------------------------

/// Taille d'un `IMAGE_IMPORT_DESCRIPTOR`.
pub const TAILLE_DESCRIPTEUR_IMPORT: usize = 20;

/// Verdict sur les dependances d'une image.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dependances {
    /// Aucune importation : une image autonome, ce qu'est un `hello.exe`
    /// Bouchaud de premiere generation.
    Aucune,
    /// Des importations, toutes hors de l'univers Windows.
    Bouchaud,
    /// Au moins une bibliotheque Windows. Position dans le fichier du nom
    /// fautif, pour pouvoir le citer.
    Windows { offset_nom: usize },
}

/// Classe les dependances d'une image sans rien charger.
pub fn classe_dependances(
    data: &[u8],
    entete: &EnTetePe,
    offset_de_rva: impl Fn(u32) -> Option<usize>,
) -> Result<Dependances, RefusPe> {
    if entete.import_rva == 0 || entete.import_taille == 0 {
        return Ok(Dependances::Aucune);
    }
    let mut curseur = offset_de_rva(entete.import_rva).ok_or(RefusPe::ImportsHorsFichier)?;
    let mut vues = 0usize;

    // Le tableau se termine par un descripteur entierement nul.
    while curseur + TAILLE_DESCRIPTEUR_IMPORT <= data.len() {
        let bloc = &data[curseur..curseur + TAILLE_DESCRIPTEUR_IMPORT];
        if bloc.iter().all(|&octet| octet == 0) {
            break;
        }
        let rva_nom = lit_u32(data, curseur + 12).ok_or(RefusPe::Tronque("nom d'import"))?;
        let offset_nom = offset_de_rva(rva_nom).ok_or(RefusPe::ImportsHorsFichier)?;
        let nom = chaine_c(data, offset_nom).ok_or(RefusPe::ImportsHorsFichier)?;
        if est_bibliotheque_windows(nom) {
            return Ok(Dependances::Windows { offset_nom });
        }
        vues += 1;
        curseur += TAILLE_DESCRIPTEUR_IMPORT;
    }

    if vues == 0 {
        Ok(Dependances::Aucune)
    } else {
        Ok(Dependances::Bouchaud)
    }
}

/// Chaine terminee par zero a partir d'un offset, sans deborder.
pub fn chaine_c(data: &[u8], offset: usize) -> Option<&[u8]> {
    let reste = data.get(offset..)?;
    // Une chaine sans terminateur jusqu'a la fin du fichier est une image
    // invalide, pas une chaine tres longue.
    let fin = reste.iter().position(|&octet| octet == 0)?;
    Some(&reste[..fin])
}

/// Traduit une adresse virtuelle relative en offset dans le fichier.
///
/// Rend `None` quand aucune section ne la couvre : une RVA qui ne correspond a
/// rien est une image invalide, et deviner l'offset produirait un programme
/// charge depuis les mauvais octets.
pub fn offset_de_rva(sections: &[Section], rva: u32) -> Option<usize> {
    for section in sections {
        let debut = section.rva;
        let fin = debut.checked_add(section.taille_virtuelle.max(section.taille_brute))?;
        if rva >= debut && rva < fin {
            let dans_section = rva - debut;
            if dans_section >= section.taille_brute {
                return None; // dans le `.bss` : aucun octet dans le fichier
            }
            return Some(section.offset_brut as usize + dans_section as usize);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Validation avant projection
// ---------------------------------------------------------------------------

/// Verifie tout ce qui doit l'etre AVANT de projeter une image en memoire.
///
/// # Pourquoi cette fonction existe separement
///
/// Une image invalide qu'on projette quand meme ne faute pas au moment ou elle
/// est fausse : elle faute plus tard, ailleurs, dans du code sans rapport. Les
/// verifications sont donc groupees ici, avant que le moindre octet ne soit
/// mappe, et chacune rend un refus qui NOMME ce qu'elle a trouve.
///
/// Un chargeur de binaire traite son entree comme hostile. Ce fichier peut
/// venir d'un disque, d'un telechargement, d'un autre programme.
pub fn valide_avant_projection(
    entete: &EnTetePe,
    sections: &[Section],
) -> Result<(), RefusPe> {
    // --- geometrie generale ------------------------------------------------
    if entete.alignement_section == 0 || !entete.alignement_section.is_power_of_two() {
        return Err(RefusPe::GeometrieIncoherente("SectionAlignment"));
    }
    if entete.alignement_fichier == 0 || !entete.alignement_fichier.is_power_of_two() {
        return Err(RefusPe::GeometrieIncoherente("FileAlignment"));
    }
    if entete.alignement_fichier > entete.alignement_section {
        return Err(RefusPe::GeometrieIncoherente(
            "FileAlignment superieur a SectionAlignment",
        ));
    }
    if entete.taille_image == 0 {
        return Err(RefusPe::GeometrieIncoherente("SizeOfImage nul"));
    }
    if entete.taille_entetes as usize > entete.taille_image as usize {
        return Err(RefusPe::GeometrieIncoherente(
            "SizeOfHeaders depasse SizeOfImage",
        ));
    }
    // `ImageBase + SizeOfImage` doit tenir dans l'espace d'adressage : sans ce
    // test, le calcul d'adresse d'une section pourrait boucler et designer une
    // page qui n'a rien a voir.
    if entete
        .base_image
        .checked_add(entete.taille_image as u64)
        .is_none()
    {
        return Err(RefusPe::ImageTropGrande);
    }

    // --- chaque section tient dans l'image ---------------------------------
    for section in sections {
        let etendue = section.taille_virtuelle.max(section.taille_brute);
        let fin = section
            .rva
            .checked_add(etendue)
            .ok_or(RefusPe::SectionHorsFichier { rva: section.rva })?;
        if fin > entete.taille_image {
            return Err(RefusPe::SectionHorsFichier { rva: section.rva });
        }
        if section.viole_w_xor_x() {
            return Err(RefusPe::EcritureEtExecution { rva: section.rva });
        }
    }

    // --- aucune section ne recouvre une autre ------------------------------
    //
    // Deux sections qui se recouvrent en memoire donnent une projection dont le
    // resultat depend de l'ORDRE de mapping -- donc du chargeur, pas du
    // binaire. C'est le genre de dependance qu'on refuse plutot que de figer.
    for (index, section) in sections.iter().enumerate() {
        let etendue = section.taille_virtuelle.max(section.taille_brute);
        if etendue == 0 {
            continue; // une section vide ne recouvre rien
        }
        let fin = section.rva + etendue;
        for autre in &sections[index + 1..] {
            let autre_etendue = autre.taille_virtuelle.max(autre.taille_brute);
            if autre_etendue == 0 {
                continue;
            }
            let autre_fin = autre.rva + autre_etendue;
            if section.rva < autre_fin && autre.rva < fin {
                return Err(RefusPe::SectionsQuiSeRecouvrent {
                    premiere: section.rva,
                    seconde: autre.rva,
                });
            }
        }
    }

    // --- le point d'entree tombe dans du code executable -------------------
    //
    // Un point d'entree hors d'une section executable est soit une image
    // corrompue, soit une tentative de faire sauter le chargeur ailleurs.
    let entree_valide = sections.iter().any(|section| {
        let etendue = section.taille_virtuelle.max(section.taille_brute);
        section.executable()
            && entete.point_entree_rva >= section.rva
            && entete.point_entree_rva < section.rva.saturating_add(etendue)
    });
    if !entree_valide {
        return Err(RefusPe::PointEntreeInvalide {
            rva: entete.point_entree_rva,
        });
    }

    Ok(())
}
