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
    let dt = crate::arch::x86_64::rtc::now_utc();
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

    // BOUCHAUD_P13_3_PATH_PREFIX_V2
    // 2. Construit le chemin a partir de la feuille, mais s'arrete DES qu'une
    // trust anchor locale peut signer le certificat courant. Les certificats
    // supplementaires envoyes par le serveur ne font pas partie du chemin
    // choisi et ne doivent donc pas pouvoir le faire echouer.
    for i in 0..certs.len() {
        let courant = &certs[i];

        // Une variante cross-signee d'une trust anchor peut etre remplacee par
        // l'ancre locale equivalente (meme Subject + meme cle publique). Une
        // trust anchor n'est pas validee comme un certificat ordinaire du chemin.
        if roots::find_equivalent_anchor(courant).is_some() {
            res.trusted = true;
            res.anchor = Some("magasin:subject+spki");
            res.detail = if !res.hostname_ok {
                "chaine valide mais nom d'hote non couvert par le certificat"
            } else {
                "chaine de confiance complete (ancre equivalente cross-signee)"
            };
            return res;
        }

        // Le certificat courant est un element normal du chemin : sa periode
        // de validite compte. La trust anchor locale, elle, n'est pas dans certs.
        if now != 0 && (now < courant.not_before || now > courant.not_after) {
            res.expired = true;
        }

        // Cas prefere : une racine locale signe deja le certificat courant.
        // Pour example.com, le chemin peut donc s'arreter sur le certificat
        // "SSL.com TLS Transit ECC CA R2" et ignorer la racine cross-signee
        // supplementaire envoyee par le serveur.
        if roots::find_issuer_for(courant).is_some() {
            res.trusted = true;
            res.anchor = Some("magasin:issuer");
            res.detail = if res.expired {
                "chaine cryptographiquement valide mais expiree"
            } else if !res.hostname_ok {
                "chaine valide mais nom d'hote non couvert par le certificat"
            } else {
                "chaine de confiance complete"
            };
            return res;
        }

        // Pas encore d'ancre : il faut un certificat serveur suivant coherent.
        let Some(suivant) = certs.get(i + 1) else {
            res.detail = "ancre de confiance inconnue (racine absente du magasin)";
            return res;
        };

        if courant.issuer != suivant.subject {
            res.detail = "issuer/subject incoherents dans la chaine";
            return res;
        }
        // BOUCHAUD_TLS_SIGNATAIRE_CA_V1
        //
        // Une signature valide ne suffit pas : son auteur doit avoir le DROIT
        // de signer. Sans ce controle, n'importe quelle feuille legitimement
        // emise -- `attaquant.test`, CA:FALSE -- signait une fausse feuille
        // pour n'importe quel nom, et la chaine
        //     [fausse feuille, feuille de l'attaquant, intermediaire]
        // recevait « [TLS OK] » : chaque signature verifie, le nom d'hote
        // correspond. Voir `tools/securite/test_chaine_x509.rs`.
        //
        // RFC 5280 4.2.1.9 : sans basicConstraints cA=TRUE -- extension
        // absente comprise -- la cle d'un certificat ne verifie aucune
        // signature de certificat. Les racines du magasin ne passent pas par
        // ici : elles sont ancres par configuration, pas par ce qu'elles
        // disent d'elles-memes.
        if !suivant.is_ca {
            res.detail = "signataire sans basicConstraints CA dans la chaine";
            return res;
        }
        if !x509::verify_signed_by(courant, &suivant.pubkey) {
            res.detail = "signature de chaine invalide";
            return res;
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
