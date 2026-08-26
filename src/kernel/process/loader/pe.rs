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
