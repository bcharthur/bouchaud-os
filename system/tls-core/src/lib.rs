//! BOUCHAUD_TLS_PARTAGE_V1 -- la pile TLS 1.3 du noyau, compilee hors du noyau.
//!
//! Aucune copie : chaque module est le FICHIER du noyau
//! (`src/net/security/tls/*.rs`), inclus par `#[path]`. Une correction de la
//! pile s'applique donc au noyau et a l'anneau 3 dans le meme commit, et les
//! racines de confiance sont les memes octets (`roots.rs`, `ca/`).
//!
//! Ce que le noyau fournit de son cote (TcpConn, RDRAND, RTC), l'hote le
//! fournit ici par `handshake::Transport` et `handshake::Plateforme`. Le module
//! `rng` du noyau n'est PAS inclus : il lit RDRAND sous un verrou noyau ;
//! l'anneau 3 passe `getrandom`.
#![no_std]
extern crate alloc;

macro_rules! module_du_noyau {
    ($($nom:ident => $fichier:literal),* $(,)?) => {
        $( #[path = $fichier] pub mod $nom; )*
    };
}

module_du_noyau! {
    sha256 => "../../../src/net/security/tls/sha256.rs",
    sha512 => "../../../src/net/security/tls/sha512.rs",
    hash => "../../../src/net/security/tls/hash.rs",
    aes => "../../../src/net/security/tls/aes.rs",
    gcm => "../../../src/net/security/tls/gcm.rs",
    chacha => "../../../src/net/security/tls/chacha.rs",
    x25519 => "../../../src/net/security/tls/x25519.rs",
    ec => "../../../src/net/security/tls/ec.rs",
    p256 => "../../../src/net/security/tls/p256.rs",
    p384 => "../../../src/net/security/tls/p384.rs",
    bignum => "../../../src/net/security/tls/bignum.rs",
    rsa => "../../../src/net/security/tls/rsa.rs",
    asn1 => "../../../src/net/security/tls/asn1.rs",
    x509 => "../../../src/net/security/tls/x509.rs",
    roots => "../../../src/net/security/tls/roots.rs",
    validate => "../../../src/net/security/tls/validate.rs",
    record => "../../../src/net/security/tls/record.rs",
    alert => "../../../src/net/security/tls/alert.rs",
    handshake => "../../../src/net/security/tls/handshake.rs",
}
