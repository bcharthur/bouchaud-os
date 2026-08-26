//! Identification d'un format d'executable, et rien d'autre.
//!
//! # Pourquoi ce fichier ne charge rien
//!
//! Tout ce qui est ici est une fonction des premiers octets d'un fichier :
//! aucun espace d'adressage, aucun verrou, aucun descripteur. C'est ce qui
//! permet de l'exercer sur la machine de developpement -- voir
//! `tools/exec/test_format.rs`.
//!
//! Reconnaitre un format est aussi la seule etape qui doit rendre un message
//! JUSTE avant toute autre chose. Un `.exe` refuse par « signature ELF
//! absente » n'apprend rien a personne ; il faut dire que c'est un PE, et
//! pourquoi celui-ci ne peut pas tourner.

#![allow(dead_code)]

/// Ce que les premiers octets d'un fichier disent de lui.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    /// ELF64 petit-boutiste x86-64, le format natif actuel.
    Elf64,
    /// PE32+ (`.exe`) pour AMD64.
    Pe32Plus,
    /// Script `#!`.
    Script,
    /// Rien de reconnu.
    Inconnu,
}

/// Signature ELF.
pub const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
/// En-tete MZ d'un binaire DOS, que tout PE conserve en prefixe.
pub const MZ_MAGIC: [u8; 2] = [b'M', b'Z'];
/// Signature PE, a l'offset indique par `e_lfanew`.
pub const PE_MAGIC: [u8; 4] = [b'P', b'E', 0, 0];

/// Offset du champ `e_lfanew` dans l'en-tete MZ.
const E_LFANEW: usize = 0x3c;

/// Reconnait le format sans rien charger.
///
/// N'a besoin que du debut du fichier : un en-tete MZ complet fait 64 octets,
/// et l'offset `e_lfanew` pointe rarement au-dela du premier bloc.
pub fn identifie(data: &[u8]) -> Format {
    if data.len() >= 4 && data[0..4] == ELF_MAGIC {
        return Format::Elf64;
    }
    if data.len() >= 2 && data[0..2] == b"#!"[..] {
        return Format::Script;
    }
    if data.len() >= 2 && data[0..2] == MZ_MAGIC {
        // Un MZ seul est un binaire DOS ; c'est la signature PE, a l'offset
        // qu'il designe, qui en fait un executable moderne. Ne pas verifier
        // ferait passer un vieux .com pour un PE32+.
        if let Some(offset) = offset_pe(data) {
            if data.len() >= offset + 4 && data[offset..offset + 4] == PE_MAGIC {
                return Format::Pe32Plus;
            }
        }
    }
    Format::Inconnu
}

/// Offset de l'en-tete PE annonce par le prefixe MZ, s'il est plausible.
pub fn offset_pe(data: &[u8]) -> Option<usize> {
    if data.len() < E_LFANEW + 4 {
        return None;
    }
    let offset = u32::from_le_bytes([
        data[E_LFANEW],
        data[E_LFANEW + 1],
        data[E_LFANEW + 2],
        data[E_LFANEW + 3],
    ]) as usize;
    // Un offset qui deborde n'est pas une erreur de lecture a signaler plus
    // tard : c'est deja la preuve que ce fichier n'est pas un PE valide.
    if offset < E_LFANEW + 4 || offset > data.len() {
        return None;
    }
    Some(offset)
}

impl Format {
    /// Nom court, pour un message d'erreur qui nomme ce qu'il a vu.
    pub fn nom(self) -> &'static str {
        match self {
            Format::Elf64 => "ELF64",
            Format::Pe32Plus => "PE32+",
            Format::Script => "script #!",
            Format::Inconnu => "format inconnu",
        }
    }
}
