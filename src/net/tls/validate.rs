//! Validation d'une chaine de certificats X.509 contre le magasin de racines.

use super::x509::{self, Certificate};
use super::roots;
use alloc::vec::Vec;

/// Resultat detaille de la validation.
pub struct ChainResult {
    pub trusted: bool,
    pub hostname_ok: bool,
    pub expired: bool,
    pub anchor: Option<&'static str>,
    pub detail: &'static str,
}

/// Convertit la date RTC courante en entier AAAAMMJJhhmmss.
pub fn now_stamp() -> u64 {
    let dt = crate::arch::x86_64::rtc::now();
    (dt.year as u64) * 10_000_000_000
        + (dt.month as u64) * 100_000_000
        + (dt.day as u64) * 1_000_000
        + (dt.hour as u64) * 10_000
        + (dt.minute as u64) * 100
        + (dt.second as u64)
}

/// Valide la chaine `certs` (leaf en premier) pour `hostname` a l'instant `now`.
pub fn validate(certs: &[Certificate], hostname: &str, now: u64) -> ChainResult {
    let mut res = ChainResult {
        trusted: false,
        hostname_ok: false,
        expired: false,
        anchor: None,
        detail: "",
    };
    if certs.is_empty() {
        res.detail = "aucun certificat";
        return res;
    }

    // 1. Le nom d'hote doit figurer dans les SAN du certificat feuille.
    res.hostname_ok = x509::matches_hostname(&certs[0], hostname);

    // 2. Validite temporelle de chaque certificat de la chaine.
    for c in certs {
        if now != 0 && (now < c.not_before || now > c.not_after) {
            res.expired = true;
        }
    }

    // 3. Chaque certificat doit etre signe par le suivant.
    for i in 0..certs.len() - 1 {
        if !x509::verify_signed_by(&certs[i], &certs[i + 1].pubkey) {
            res.detail = "signature de chaine invalide";
            return res;
        }
    }

    // 4. Le dernier certificat doit etre signe par une racine de confiance.
    let last = &certs[certs.len() - 1];
    match roots::find_issuer_for(last) {
        Some(root) => {
            if x509::verify_signed_by(last, &root.pubkey) {
                res.trusted = true;
                res.detail = if res.expired {
                    "chaine cryptographiquement valide mais expiree"
                } else if !res.hostname_ok {
                    "chaine valide mais nom d'hote non couvert par le certificat"
                } else {
                    "chaine de confiance complete"
                };
            } else {
                res.detail = "signature de la racine invalide";
            }
        }
        None => {
            // Cas ou le serveur envoie lui-meme la racine en bout de chaine.
            if last.subject == last.issuer && x509::verify_signed_by(last, &last.pubkey) {
                res.detail = "racine auto-signee non presente dans le magasin";
            } else {
                res.detail = "ancre de confiance inconnue (racine absente du magasin)";
            }
        }
    }
    res
}

/// Parse une liste de certificats DER en structures.
pub fn parse_chain(ders: &[Vec<u8>]) -> Vec<Certificate> {
    let mut out = Vec::new();
    for d in ders {
        if let Some(c) = x509::parse(d) {
            out.push(c);
        }
    }
    out
}
