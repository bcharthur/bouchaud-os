//! OTA agent, slice 1: periodic SIGNED check of a release source
//! (BOUCHAUD_OTA_V1).
//!
//! # What it does
//!
//! At boot (once the network has a lease) then every `PERIODE_S`, for each
//! `source=` line of `/etc/bouchaud/ota.conf` (in order, mirrors allowed):
//!   1. downloads `<source>/manifest` and `<source>/manifest.sig` (HTTPS with
//!      the kernel's TLS and root store, or HTTP for a local mirror -- the
//!      DECISION never trusts the transport, only the signature);
//!   2. decides with `bouchaud_update_core::ota::decide`: trusted key,
//!      canonical v2, hardware family, anti-rollback floor, newer than the
//!      running slot;
//!   3. logs `BOUCHAUD_OTA_VERIFIE source=.. resultat=disponible|a_jour|refuse ..`.
//!
//! For the stable channel the source is
//! `https://github.com/<owner>/<repo>/releases/latest/download`: GitHub's
//! "latest" is the newest release that is NOT a prerelease, which is
//! exactly a release promoted testing -> stable (release-trigkey.yml).
//!
//! # What it does NOT do (yet), on purpose
//!
//! It downloads no image and installs nothing: the decision is published,
//! the user consents, `updated` installs (next slice). Full-auto mode is not
//! implemented. A production build without a committed production key
//! refuses every source (`aucune_cle_de_confiance`): fail closed.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bouchaud_update_core as uc;
use uc::ota::{Decision, Refus};

/// The agent's configuration (in the system image, read-only).
pub const CONFIG: &str = "/etc/bouchaud/ota.conf";
/// Six hours between two checks.
pub const PERIODE_S: u64 = 6 * 3600;
/// Longest wait for a network lease before a check is skipped.
const ATTENTE_RESEAU_S: u64 = 180;
/// The manifest is a few hundred bytes; anything larger is not ours.
const MANIFESTE_MAX: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resultat {
    Disponible { version: String, sequence: u64, octets: u64 },
    AJour { sequence: u64 },
    Refuse(&'static str),
}

fn sources() -> Vec<String> {
    let texte = {
        let fs = crate::fs::ramfs::fs();
        match fs.resolve(CONFIG, 0) {
            Some(i) => fs.nodes[i].content.clone(),
            None => return Vec::new(),
        }
    };
    let texte = String::from_utf8_lossy(&texte);
    texte.lines()
        .filter_map(|l| l.trim().strip_prefix("source="))
        .map(|s| String::from(s.trim().trim_end_matches('/')))
        .filter(|s| s.starts_with("https://") || s.starts_with("http://"))
        .collect()
}

fn telecharge(url: &str, max: usize) -> Result<Vec<u8>, &'static str> {
    let d = crate::net::fetch_document(url);
    // `ok` means "an HTTP answer was parsed", NOT "200": a 404 page is a
    // body too (found by the ota-verifie bench, source `absent`). Only the
    // final status line counts.
    let statut_200 = d.banner.iter().rev()
        .find(|l| l.starts_with("HTTP/"))
        .map_or(false, |l| l.split_whitespace().nth(1) == Some("200"));
    if !d.ok || !statut_200 || d.body.is_empty() {
        return Err("injoignable");
    }
    if d.body.len() > max {
        return Err("trop_grand");
    }
    Ok(d.body)
}

/// One check of one source. No disk write, no install.
pub fn verifie(source: &str) -> Resultat {
    let texte = match telecharge(&format!("{}/manifest", source), MANIFESTE_MAX) {
        Ok(t) => t,
        Err(e) => return Resultat::Refuse(if e == "injoignable" { "manifeste_injoignable" } else { "manifeste_trop_grand" }),
    };
    let sig = match telecharge(&format!("{}/manifest.sig", source), 64) {
        Ok(s) if s.len() == 64 => s,
        Ok(_) => return Resultat::Refuse("signature_mal_formee"),
        Err(_) => return Resultat::Refuse("signature_injoignable"),
    };
    let mut signature = [0u8; 64];
    signature.copy_from_slice(&sig);
    let (lecture, courante) = super::etat_anti_retour();
    match uc::ota::decide(&texte, &signature, super::cles_de_confiance(), lecture, courante, super::MATERIEL) {
        Ok(Decision::Disponible(m)) => Resultat::Disponible {
            version: String::from(m.version()), sequence: m.sequence, octets: m.octets,
        },
        Ok(Decision::AJour { sequence }) => Resultat::AJour { sequence },
        Err(r) => Resultat::Refuse(Refus::texte(r)),
    }
}

/// Checks every configured source, logs one line per source.
pub fn verifie_tout() -> usize {
    let s = sources();
    for source in &s {
        match verifie(source) {
            Resultat::Disponible { version, sequence, octets } => crate::serial_println!(
                "BOUCHAUD_OTA_VERIFIE source={} resultat=disponible version={} sequence={} octets={} installation=consentement_requis",
                source, version, sequence, octets),
            Resultat::AJour { sequence } => crate::serial_println!(
                "BOUCHAUD_OTA_VERIFIE source={} resultat=a_jour sequence={}", source, sequence),
            Resultat::Refuse(r) => crate::serial_println!(
                "BOUCHAUD_OTA_VERIFIE source={} resultat=refuse raison={}", source, r),
        }
    }
    s.len()
}

fn dort_s(s: u64) {
    crate::kernel::task::sleep_ticks(crate::kernel::timer::ms_to_ticks(s.saturating_mul(1000)));
}

fn fil_ota() -> ! {
    loop {
        let mut attente = 0;
        while !crate::net::bail_obtenu() && attente < ATTENTE_RESEAU_S {
            dort_s(1);
            attente += 1;
        }
        if crate::net::bail_obtenu() {
            let n = verifie_tout();
            crate::serial_println!("BOUCHAUD_OTA_CYCLE sources={} prochain_s={}", n, PERIODE_S);
        } else {
            crate::serial_println!("BOUCHAUD_OTA_CYCLE sources=0 raison=pas_de_reseau prochain_s={}", PERIODE_S);
        }
        dort_s(PERIODE_S);
    }
}

/// Started at boot. Inert without a configuration.
pub fn lance() {
    if sources().is_empty() {
        crate::serial_println!("BOUCHAUD_OTA_INACTIF raison=sans_source config={}", CONFIG);
        return;
    }
    if !crate::kernel::task::spawn_noyau(fil_ota, "ota") {
        crate::serial_println!("BOUCHAUD_OTA_INACTIF raison=tache_non_creee");
    }
}
