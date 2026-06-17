//! Magasin de CA racines de confiance (embarque via `include_bytes!`).
//!
//! Ces certificats racines (Mozilla CA bundle) servent d'ancres de confiance
//! pour la validation de chaine. Pour en ajouter, deposer le DER dans `ca/` et
//! referencer le fichier ici.

use super::x509::{self, Certificate, PubKey};
use alloc::vec::Vec;

/// Certificats racines bruts (DER).
pub static ROOTS_DER: &[&[u8]] = &[
    include_bytes!("ca/ISRG_Root_X1.der"),
    include_bytes!("ca/ISRG_Root_X2.der"),
    include_bytes!("ca/DigiCert_Global_Root_CA.der"),
    include_bytes!("ca/GTS_Root_R1.der"),
    include_bytes!("ca/USERTrust_RSA_Certification_Authority.der"),
    include_bytes!("ca/Baltimore_CyberTrust_Root.der"),
];

/// Nombre de racines embarquees.
pub fn count() -> usize { ROOTS_DER.len() }

/// Cherche une racine dont le `subject` correspond a l'`issuer` donne et qui
/// signe valablement `child`. Renvoie sa cle publique si trouvee.
pub fn find_issuer_for(child: &Certificate) -> Option<Certificate> {
    for der in ROOTS_DER {
        if let Some(root) = x509::parse(der) {
            if root.subject == child.issuer {
                return Some(root);
            }
        }
    }
    None
}

/// Toutes les racines parsees (pour diagnostics).
pub fn parsed() -> Vec<Certificate> {
    let mut v = Vec::new();
    for der in ROOTS_DER {
        if let Some(c) = x509::parse(der) { v.push(c); }
    }
    v
}

/// Renvoie la cle publique d'une racine par index (diagnostic).
pub fn root_pubkey(i: usize) -> Option<PubKey> {
    x509::parse(ROOTS_DER.get(i)?).map(|c| c.pubkey)
}
