//! Magasin de CA racines de confiance (embarque via `include_bytes!`).
//!
//! Ces certificats racines (Mozilla CA bundle) servent d'ancres de confiance
//! pour la validation de chaine. Pour en ajouter, deposer le DER dans `ca/` et
//! referencer le fichier ici.

use super::x509::{self, Certificate, PubKey};
use alloc::vec::Vec;

/// Certificats racines bruts (DER). Selection couvrant la majorite du web public
/// (Let's Encrypt, Google, DigiCert, GlobalSign, Amazon, Sectigo/USERTrust,
/// Microsoft, GoDaddy, Certum...), en variantes RSA et ECDSA.
pub static ROOTS_DER: &[&[u8]] = &[
    // Let's Encrypt
    include_bytes!("ca/ISRG_Root_X1.der"),
    include_bytes!("ca/ISRG_Root_X2.der"),
    // DigiCert
    include_bytes!("ca/DigiCert_Global_Root_CA.der"),
    include_bytes!("ca/DigiCert_Global_Root_G2.der"),
    include_bytes!("ca/Baltimore_CyberTrust_Root.der"),
    // Google Trust Services
    include_bytes!("ca/GTS_Root_R1.der"),
    include_bytes!("ca/GTS_Root_R2.der"),
    include_bytes!("ca/GTS_Root_R4.der"),
    // GlobalSign
    include_bytes!("ca/GlobalSign_Root_CA.der"),
    include_bytes!("ca/GlobalSign_Root_CA__R3.der"),
    include_bytes!("ca/GlobalSign_Root_CA__R6.der"),
    // Sectigo / USERTrust (Comodo)
    include_bytes!("ca/USERTrust_RSA_Certification_Authority.der"),
    include_bytes!("ca/USERTrust_ECC_Certification_Authority.der"),
    include_bytes!("ca/COMODO_RSA_Certification_Authority.der"),
    include_bytes!("ca/Sectigo_Public_Server_Authentication_Root_R46.der"),
    // Amazon
    include_bytes!("ca/Amazon_Root_CA_1.der"),
    // Microsoft
    include_bytes!("ca/Microsoft_RSA_Root_Certificate_Authority_2017.der"),
    // GoDaddy
    include_bytes!("ca/Go_Daddy_Root_Certificate_Authority__G2.der"),
    // Certum
    include_bytes!("ca/Certum_Trusted_Network_CA.der"),
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
