//! TLS — socle honnete (HTTPS).
//!
//! IMPORTANT : le handshake TLS n'est PAS implemente. Un client TLS 1.2/1.3
//! fonctionnel exige une pile cryptographique complete, ecrite au bit pres :
//!
//!   - echange de cles : X25519 ou ECDHE P-256 (arithmetique sur courbe) ;
//!   - chiffrement authentifie : AES-128/256-GCM ou ChaCha20-Poly1305 ;
//!   - hachage : SHA-256/384, HMAC, HKDF (derivation de cles) ;
//!   - signatures : RSA / ECDSA ;
//!   - X.509 / ASN.1 DER : parsing et **validation de la chaine de certificats**
//!     contre un magasin de CA racines ;
//!   - une source d'entropie (CSPRNG) pour les aleas du handshake.
//!
//! C'est un projet a part entiere (plusieurs milliers de lignes auditees). Il
//! est volontairement laisse en chantier plutot que simule. Ce module fournit
//! seulement le cadre (couche record + types) pour une implementation future.

/// Versions TLS.
pub const TLS_1_2: u16 = 0x0303;
pub const TLS_1_3: u16 = 0x0304;

/// Types de record TLS.
#[derive(Clone, Copy)]
#[repr(u8)]
pub enum ContentType {
    ChangeCipherSpec = 20,
    Alert = 21,
    Handshake = 22,
    ApplicationData = 23,
}

/// Construit l'en-tete d'un record TLS (5 octets) devant une charge utile.
/// (Brique de bas niveau ; le contenu chiffre n'est pas gere ici.)
pub fn record_header(buf: &mut [u8], ct: ContentType, version: u16, len: u16) -> bool {
    if buf.len() < 5 { return false; }
    buf[0] = ct as u8;
    buf[1] = (version >> 8) as u8;
    buf[2] = version as u8;
    buf[3] = (len >> 8) as u8;
    buf[4] = len as u8;
    true
}

/// Etat d'implementation, pour les messages utilisateur.
pub fn status() -> &'static str {
    "non implemente (necessite X25519/AES-GCM/SHA-256/HKDF/X.509)"
}
