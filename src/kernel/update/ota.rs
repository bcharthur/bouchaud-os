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
//! # Download and install (slice 2) -- only with CONSENT
//!
//! `telechargement=consenti` in the configuration: for the first source whose
//! decision is `disponible`, the agent downloads the image IN PARTS
//! (`system-slot.parts` index, then `system-slot.img.partNNN`, <= 4 MiB each:
//! a RAMFS file is capped at 64 MiB and memory stays bounded). Each part is
//! checked against the index (size, SHA-256) and the WHOLE stream against
//! the SIGNED manifest (size, SHA-256): a hostile index or mirror can waste
//! bandwidth, never stage a byte the signature does not cover. Staged in
//! `/update/` (RAM), `pret` written last; `updated` (ring 3, the only holder
//! of the install capability) installs into the INACTIVE slot.
//!
//! `redemarrage=consenti`: once the candidate is recorded, the agent reboots
//! into it (health commit or automatic rollback follow). Without it, it
//! waits and says so (`consentement_requis`). There is no implicit consent.
//!
//! A production build without a committed production key refuses every
//! source (`aucune_cle_de_confiance`): fail closed.
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

fn config() -> String {
    let fs = crate::fs::ramfs::fs();
    match fs.resolve(CONFIG, 0) {
        Some(i) => String::from(String::from_utf8_lossy(&fs.nodes[i].content)),
        None => String::new(),
    }
}

fn consenti(cle: &str) -> bool {
    config().lines().any(|l| l.trim() == format!("{}=consenti", cle))
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
    let (lecture, courante, echouee) = super::etat_anti_retour();
    match uc::ota::decide(&texte, &signature, super::cles_de_confiance(), lecture, courante, echouee, super::MATERIEL) {
        Ok(Decision::Disponible(m)) => {
            // Every source is still CHECKED (and logged); the first trusted,
            // newer one is the one downloaded.
            let mut g = RETENUE.lock();
            if g.is_none() {
                *g = Some(Retenue { source: String::from(source), texte, signature, manifeste: m });
            }
            drop(g);
            Resultat::Disponible { version: String::from(m.version()), sequence: m.sequence, octets: m.octets }
        }
        Ok(Decision::AJour { sequence }) => Resultat::AJour { sequence },
        Err(r) => Resultat::Refuse(Refus::texte(r)),
    }
}

/// The first trusted, newer release seen in this cycle.
struct Retenue {
    source: String,
    texte: Vec<u8>,
    signature: [u8; 64],
    manifeste: uc::manifeste::Manifeste,
}
static RETENUE: crate::kernel::sync::SpinLock<Option<Retenue>> = crate::kernel::sync::SpinLock::new(None);

/// Part size published by the release tool (fabrique_release.py
/// --parties-mio 4). Bounded by the kernel HTTPS client, which caps ONE
/// response at 8 000 000 bytes (tls::https_fetch, recv_http): a larger part
/// would arrive truncated -- and be refused (`partie_taille_differente`).
const PARTIE_MAX: usize = 4 * 1024 * 1024;
const INDEX_MAX: usize = 64 * 1024;

fn ecrit_ramfs(chemin: &str, donnees: &[u8]) -> Result<(), &'static str> {
    let mut fs = crate::fs::ramfs::fs();
    let dossier = match fs.resolve("/update", 0) {
        Some(d) => d,
        None => fs.mkdir_at(0, "update")?,
    };
    let nom = chemin.rsplit('/').next().ok_or("chemin")?;
    let f = fs.touch_at(dossier, nom)?;
    if !fs.write_node_bytes(f, donnees) { return Err("fichier_trop_grand"); }
    Ok(())
}

fn hex32(h: &[u8; 32]) -> String {
    let mut s = String::new();
    for b in h { s.push_str(&format!("{:02x}", b)); }
    s
}

/// Downloads the retained release in parts, checks it against the SIGNED
/// manifest, stages it for `updated`. Err: nothing usable is staged.
fn telecharge_et_prepare(r: &Retenue) -> Result<u64, &'static str> {
    let index = telecharge(&format!("{}/system-slot.parts", r.source), INDEX_MAX).map_err(|_| "index_injoignable")?;
    let index = String::from(String::from_utf8_lossy(&index));
    let mut total = crate::net::security::tls::sha256::Sha256::new();
    let mut octets = 0u64;
    let mut k = 0usize;
    for ligne in index.lines().filter(|l| !l.trim().is_empty()) {
        let champs: Vec<&str> = ligne.split_whitespace().collect();
        let attendu_nom = format!("system-slot.img.part{:03}", k);
        if champs.len() != 3 || champs[0] != attendu_nom { return Err("index_mal_forme"); }
        let taille: usize = champs[1].parse().map_err(|_| "index_mal_forme")?;
        if taille == 0 || taille > PARTIE_MAX { return Err("partie_hors_bornes"); }
        let corps = telecharge(&format!("{}/{}", r.source, attendu_nom), PARTIE_MAX).map_err(|_| "partie_injoignable")?;
        if corps.len() != taille { return Err("partie_taille_differente"); }
        let h = crate::net::security::tls::sha256::sha256(&corps);
        if hex32(&h) != champs[2] { return Err("partie_empreinte_differente"); }
        total.update(&corps);
        octets += corps.len() as u64;
        if octets > r.manifeste.octets { return Err("image_plus_longue_que_le_manifeste"); }
        ecrit_ramfs(&format!("/update/system.img.part{:03}", k), &corps)?;
        crate::serial_println!("BOUCHAUD_OTA_PARTIE n={} octets={} total={}", k, corps.len(), octets);
        k += 1;
    }
    // The ONLY check that matters: the whole stream is what was signed.
    if octets != r.manifeste.octets { return Err("image_taille_differente_du_manifeste"); }
    if total.finalize() != r.manifeste.empreinte { return Err("image_empreinte_differente_du_manifeste"); }
    ecrit_ramfs("/update/manifest", &r.texte)?;
    ecrit_ramfs("/update/manifest.sig", &r.signature)?;
    ecrit_ramfs("/update/pret", b"1\n")?;
    Ok(octets)
}

/// After a `disponible`: download (consent), wait for the candidate,
/// reboot (consent).
fn installe_si_consenti() {
    let Some(r) = RETENUE.lock().take() else { return };
    if !consenti("telechargement") {
        crate::serial_println!("BOUCHAUD_OTA_TELECHARGEMENT_EN_ATTENTE consentement_requis sequence={}", r.manifeste.sequence);
        return;
    }
    match telecharge_et_prepare(&r) {
        Ok(octets) => crate::serial_println!(
            "BOUCHAUD_OTA_TELECHARGE source={} sequence={} octets={} empreinte=signee_verifiee prepare=/update",
            r.source, r.manifeste.sequence, octets),
        Err(e) => {
            crate::serial_println!("BOUCHAUD_OTA_TELECHARGEMENT_REFUSE source={} raison={}", r.source, e);
            return;
        }
    }
    // `updated` installs into the inactive slot; wait for the candidate.
    let mut s = 0;
    while !super::candidat_en_attente() && s < 1800 {
        dort_s(1);
        s += 1;
    }
    if !super::candidat_en_attente() {
        crate::serial_println!("BOUCHAUD_OTA_INSTALLATION_ABSENTE attente_s={}", s);
        return;
    }
    crate::serial_println!("BOUCHAUD_OTA_CANDIDAT_PRET sequence={} attente_s={}", r.manifeste.sequence, s);
    if consenti("redemarrage") {
        crate::serial_println!("BOUCHAUD_OTA_REDEMARRAGE consentement=configuration sequence={}", r.manifeste.sequence);
        crate::kernel::power::reboot();
    }
    crate::serial_println!("BOUCHAUD_OTA_REDEMARRAGE_EN_ATTENTE consentement_requis sequence={}", r.manifeste.sequence);
}

/// Checks every configured source, logs one line per source.
pub fn verifie_tout() -> usize {
    *RETENUE.lock() = None;
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
            installe_si_consenti();
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
