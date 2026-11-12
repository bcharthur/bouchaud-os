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
// ---- BOUCHAUD_OTA_V1, etat visible et persistant ---------------------------
//
// /persist/.bouchaud/ota-etat (cle=valeur, reecrit a chaque evenement) et
// /persist/.bouchaud/ota-historique (une ligne par echec, rollback,
// installation ; les HISTORIQUE_MAX dernieres). Ils vivent dans STATE : ils
// traversent les redemarrages et les changements de slot. L'ecriture differee
// les synchronise ; aucune E/S ici.
const ETAT: &str = ".bouchaud/ota-etat";
const HISTORIQUE: &str = ".bouchaud/ota-historique";
const HISTORIQUE_MAX: usize = 50;

static CHAMPS: crate::kernel::sync::SpinLock<Vec<(String, String)>> =
    crate::kernel::sync::SpinLock::new(Vec::new());
/// Bornes de l'etat relu sur STATE (un fichier altere ne fait pas grossir le noyau).
const CHAMPS_MAX: usize = 32;
const CLE_MAX: usize = 40;
const VALEUR_MAX: usize = 240;
/// Premiere relance apres un echec recuperable de telechargement ; doublee a
/// chaque echec consecutif, jamais au-dela de PERIODE_S.
pub const RELANCE_MIN_S: u64 = 60;

/// Consentement donne pendant CE demarrage (`bo-update consentir`).
static CONSENTEMENT: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
/// Verification demandee maintenant (`bo-update verifier`).
static VERIFIER_MAINTENANT: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

fn horodatage() -> String {
    let d = crate::arch::x86_64::rtc::now_utc();
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", d.year, d.month, d.day, d.hour, d.minute, d.second)
}

/// Ecrit `/persist/<rel>` sous le verrou du RAMFS, SANS E/S (l'ecriture
/// differee synchronise). Silencieux si /persist n'est pas monte.
fn ecrit_persist(rel: &str, contenu: &[u8]) {
    let mut fs = crate::fs::ramfs::fs();
    let Some(racine) = fs.resolve("/persist", 0) else { return };
    let (dossier, nom) = rel.split_once('/').unwrap_or(("", rel));
    let d = if dossier.is_empty() { racine } else {
        match fs.find_child(racine, dossier) {
            Some(d) => d,
            None => match fs.mkdir_at(racine, dossier) { Ok(d) => d, Err(_) => return },
        }
    };
    if let Ok(f) = fs.touch_at(d, nom) {
        let _ = fs.write_node_bytes(f, contenu);
    }
}

fn lit_persist(rel: &str) -> Vec<u8> {
    let fs = crate::fs::ramfs::fs();
    match fs.resolve(&format!("/persist/{}", rel), 0) {
        Some(i) => fs.nodes[i].content.clone(),
        None => Vec::new(),
    }
}

/// Met a jour un champ de l'etat et le reecrit.
fn etat(cle: &str, valeur: String) {
    let texte = {
        let mut c = CHAMPS.lock();
        match c.iter_mut().find(|(k, _)| k == cle) {
            Some(e) => e.1 = valeur,
            None => c.push((String::from(cle), valeur)),
        }
        texte_champs(&c)
    };
    ecrit_persist(ETAT, texte.as_bytes());
}

/// Retire un champ de l'etat et le reecrit.
fn etat_retire(cle: &str) {
    let texte = {
        let mut c = CHAMPS.lock();
        c.retain(|(k, _)| k != cle);
        texte_champs(&c)
    };
    ecrit_persist(ETAT, texte.as_bytes());
}

fn texte_champs(c: &[(String, String)]) -> String {
    let mut t = String::new();
    for (k, v) in c.iter() { t.push_str(&format!("{}={}\n", k, v)); }
    t
}

/// L'etat du demarrage PRECEDENT (STATE), relu une fois : le dernier
/// telechargement, la derniere verification, etc. traversent les
/// redemarrages. Borne ; une ligne mal formee est ignoree.
fn charge_etat() {
    let ancien = String::from_utf8_lossy(&lit_persist(ETAT)).into_owned();
    let mut c = CHAMPS.lock();
    for l in ancien.lines() {
        if c.len() >= CHAMPS_MAX { break; }
        let Some((k, v)) = l.split_once('=') else { continue };
        if k.is_empty() || k.len() > CLE_MAX || v.len() > VALEUR_MAX { continue; }
        if !k.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') { continue; }
        if c.iter().any(|(x, _)| x == k) { continue; }
        c.push((String::from(k), String::from(v)));
    }
}

/// Lecture d'un champ de l'etat.
fn etat_lu(cle: &str) -> Option<String> {
    CHAMPS.lock().iter().find(|(k, _)| k == cle).map(|(_, v)| v.clone())
}

/// Ajoute une ligne a l'historique (les HISTORIQUE_MAX dernieres).
fn historique(ligne: &str) {
    let ancien = String::from_utf8_lossy(&lit_persist(HISTORIQUE)).into_owned();
    let mut lignes: Vec<&str> = ancien.lines().collect();
    let nouvelle = format!("{} {}", horodatage(), ligne);
    lignes.push(&nouvelle);
    let debut = lignes.len().saturating_sub(HISTORIQUE_MAX);
    let mut t = String::new();
    for l in &lignes[debut..] { t.push_str(l); t.push('\n'); }
    ecrit_persist(HISTORIQUE, t.as_bytes());
    crate::serial_println!("BOUCHAUD_OTA_HISTORIQUE {}", ligne);
}

/// `bo-update consentir` : redemarrer vers la release preparee.
pub fn consentir() { CONSENTEMENT.store(true, core::sync::atomic::Ordering::Release); }
/// `bo-update verifier` : verifier les sources sans attendre la periode.
pub fn verifier_maintenant() { VERIFIER_MAINTENANT.store(true, core::sync::atomic::Ordering::Release); }

/// L'etat courant, tel que `bo-update status` l'affiche.
pub fn etat_texte() -> String {
    let mut t = String::new();
    for (k, v) in CHAMPS.lock().iter() { t.push_str(&format!("{}={}\n", k, v)); }
    t
}

pub fn historique_texte() -> String { String::from_utf8_lossy(&lit_persist(HISTORIQUE)).into_owned() }

/// Identite du systeme qui tourne et des slots, au demarrage.
fn note_identite() {
    charge_etat();
    // Un telechargement en cours au demarrage precedent n'a pas fini : coupure
    // ou redemarrage pendant le telechargement. Les parties vivaient en RAM ;
    // rien n'avait ete offert a `updated` ni ecrit dans un slot. On le dit, et
    // le cycle suivant recommence depuis la premiere partie, sous signature.
    if let Some(v) = etat_lu("telechargement_en_cours") {
        historique(&format!("telechargement_interrompu {}", v));
        crate::serial_println!("BOUCHAUD_OTA_TELECHARGEMENT_INTERROMPU {} consequence=reprise_depuis_la_premiere_partie", v);
        etat("dernier_telechargement", format!("{} interrompu {}", horodatage(), v));
        etat_retire("telechargement_en_cours");
    }
    let version = {
        let fs = crate::fs::ramfs::fs();
        fs.resolve("/etc/bouchaud/version", 0).map(|i| String::from_utf8_lossy(&fs.nodes[i].content).into_owned())
    }.unwrap_or_default();
    let champ = |k: &str| version.lines().find_map(|l| l.strip_prefix(&format!("{}=", k)).map(String::from)).unwrap_or_else(|| String::from("?"));
    etat("version", champ("version"));
    etat("commit", champ("commit"));
    etat("canal", config().lines().find_map(|l| l.trim().strip_prefix("canal=").map(String::from)).unwrap_or_else(|| String::from("stable")));
    let (lecture, courante, echouee) = super::etat_anti_retour();
    let slot = super::passage().map(|p| if p.slot == uc::Slot::A { "A" } else { "B" }).unwrap_or("-");
    etat("slot_actif", String::from(slot));
    etat("release_slot_actif", format!("{}", courante));
    if let uc::anti_retour::Lecture::Valide(r) = lecture {
        let autre = if slot == "A" { 1 } else { 0 };
        etat("release_slot_inactif", format!("{}", r.slots[autre]));
        etat("plancher", format!("{}", r.plancher));
    }
    if echouee != 0 { etat("release_en_echec", format!("{}", echouee)); }
    // Un demarrage qui suit un rollback le dit, une fois, dans l'historique.
    if let Some(p) = super::passage() {
        let raison = p.raison.texte();
        if raison.contains("rollback") || raison.contains("recovery") {
            historique(&format!("rollback raison={} slot={}", raison, slot));
        }
        etat("dernier_demarrage", String::from(raison));
    }
    etat("prochaine_installation", String::from("aucune"));
}

/// La politique de redemarrage de la configuration :
///   redemarrage=demande             (defaut) : jamais sans `bo-update consentir` ;
///   redemarrage=consenti            : des que le candidat est pret ;
///   redemarrage=fenetre:HH-HH:MIN   : aussi, automatiquement, entre HH et HH
///                                     (UTC) apres MIN minutes SANS entree.
enum Politique { Demande, Consenti, Fenetre { debut: u8, fin: u8, inactif_min: u64 } }

fn politique() -> Politique {
    let v = config().lines().find_map(|l| l.trim().strip_prefix("redemarrage=").map(String::from)).unwrap_or_default();
    if v == "consenti" { return Politique::Consenti; }
    if let Some(f) = v.strip_prefix("fenetre:") {
        let p: Vec<&str> = f.split([':', '-']).collect();
        if let [a, b, m] = p[..] {
            if let (Ok(debut), Ok(fin), Ok(inactif_min)) = (a.parse(), b.parse(), m.parse()) {
                if debut < 24 && fin < 24 && inactif_min >= 5 {
                    return Politique::Fenetre { debut, fin, inactif_min };
                }
            }
        }
        crate::serial_println!("BOUCHAUD_OTA_POLITIQUE_INVALIDE valeur={} repli=demande", v);
    }
    Politique::Demande
}

fn dans_la_fenetre(debut: u8, fin: u8) -> bool {
    let h = crate::arch::x86_64::rtc::now_utc().hour as u8;
    if debut <= fin { h >= debut && h < fin } else { h >= debut || h < fin }
}
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

/// RESUME: a transient network failure (a stalled TCP stream answered by an
/// empty response -- seen at part 16 of 24 under QEMU) retries THIS part
/// only; the parts already verified stay staged. A part that arrives but
/// does not match the index is NOT retried: that is not the network.
const TENTATIVES: u32 = 4;

fn telecharge_avec_reprise(url: &str, max: usize) -> Result<Vec<u8>, &'static str> {
    let mut derniere = "injoignable";
    for essai in 1..=TENTATIVES {
        match telecharge(url, max) {
            Ok(c) => {
                if essai > 1 {
                    crate::serial_println!("BOUCHAUD_OTA_REPRISE url={} essai={} resultat=ok", url, essai);
                }
                return Ok(c);
            }
            Err(e) => {
                derniere = e;
                crate::serial_println!("BOUCHAUD_OTA_REPRISE url={} essai={} raison={}", url, essai, e);
                if e == "trop_grand" { break; }
                dort_s(2u64 << essai);
            }
        }
    }
    Err(derniere)
}

/// One part: a network failure OR a SHORT body (a response cut in transit)
/// is retried, this part only. A body of the announced size whose digest
/// differs is NOT retried here: that is the content, not the transport (the
/// caller refuses the download; the next cycle starts over).
fn telecharge_partie(url: &str, taille: usize) -> Result<Vec<u8>, &'static str> {
    let mut derniere = "partie_injoignable";
    for essai in 1..=TENTATIVES {
        let raison = match telecharge(url, PARTIE_MAX) {
            Ok(c) if c.len() == taille => {
                if essai > 1 {
                    crate::serial_println!("BOUCHAUD_OTA_REPRISE url={} essai={} resultat=ok", url, essai);
                }
                return Ok(c);
            }
            Ok(c) if c.len() < taille => { derniere = "partie_tronquee"; format!("partie_tronquee octets={}/{}", c.len(), taille) }
            Ok(_) => return Err("partie_taille_differente"),
            Err("trop_grand") => return Err("partie_taille_differente"),
            Err(e) => { derniere = "partie_injoignable"; String::from(e) }
        };
        crate::serial_println!("BOUCHAUD_OTA_REPRISE url={} essai={} raison={}", url, essai, raison);
        if essai < TENTATIVES { dort_s(2u64 << essai); }
    }
    Err(derniere)
}

fn hex32(h: &[u8; 32]) -> String {
    let mut s = String::new();
    for b in h { s.push_str(&format!("{:02x}", b)); }
    s
}

/// Downloads the retained release in parts, checks it against the SIGNED
/// manifest, stages it for `updated`. Err: nothing usable is staged.
fn telecharge_et_prepare(r: &Retenue) -> Result<u64, &'static str> {
    let index = telecharge_avec_reprise(&format!("{}/system-slot.parts", r.source), INDEX_MAX).map_err(|_| "index_injoignable")?;
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
        let corps = telecharge_partie(&format!("{}/{}", r.source, attendu_nom), taille)?;
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
    Ok(octets)
}

/// Generation of `/update/pret` offered to `updated` (one per install attempt).
static GENERATION: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// Install attempts of one staged release (a kernel refusal ends an attempt).
const ESSAIS_INSTALLATION: u64 = 3;

/// After a `disponible`: download (consent), wait for the candidate,
/// reboot (consent).
/// What a cycle's install step ended with.
enum Issue {
    /// Nothing to retry soon (nothing available, no consent, installed, ...).
    Rien,
    /// The download failed in a way the next cycle can fix (network,
    /// truncated or altered part): retried sooner than PERIODE_S.
    EchecRecuperable(&'static str),
}

fn installe_si_consenti() -> Issue {
    let Some(r) = RETENUE.lock().take() else { return Issue::Rien };
    if !consenti("telechargement") {
        crate::serial_println!("BOUCHAUD_OTA_TELECHARGEMENT_EN_ATTENTE consentement_requis sequence={}", r.manifeste.sequence);
        return Issue::Rien;
    }
    // Nothing is offered to `updated` while parts are being written.
    let _ = ecrit_ramfs("/update/pret", b"en_cours\n");
    // Persistant (STATE) : un redemarrage pendant le telechargement se voit au
    // demarrage suivant (BOUCHAUD_OTA_TELECHARGEMENT_INTERROMPU).
    etat("telechargement_en_cours", format!("sequence={} debut={}", r.manifeste.sequence, horodatage()));
    let issue = telecharge_et_prepare(&r);
    etat_retire("telechargement_en_cours");
    match issue {
        Ok(octets) => {
            crate::serial_println!(
                "BOUCHAUD_OTA_TELECHARGE source={} sequence={} octets={} empreinte=signee_verifiee prepare=/update",
                r.source, r.manifeste.sequence, octets);
            etat("dernier_telechargement", format!("{} ok sequence={} octets={}", horodatage(), r.manifeste.sequence, octets));
        }
        Err(e) => {
            etat("dernier_telechargement", format!("{} echec raison={}", horodatage(), e));
            historique(&format!("telechargement_refuse sequence={} raison={}", r.manifeste.sequence, e));
            crate::serial_println!("BOUCHAUD_OTA_TELECHARGEMENT_REFUSE source={} raison={}", r.source, e);
            return Issue::EchecRecuperable(e);
        }
    }
    // Each offer to `updated` is a NEW generation of `pret` (revue #308) :
    // updated keeps watching after a failed install, and handles each
    // generation once. A refusal counted by the kernel ends the attempt; the
    // parts, already verified, are offered again (up to ESSAIS_INSTALLATION).
    let mut essai = 0u64;
    let mut s = 0;
    loop {
        essai += 1;
        GENERATION.fetch_add(1, core::sync::atomic::Ordering::AcqRel);
        let g = GENERATION.load(core::sync::atomic::Ordering::Acquire);
        let refus_avant = super::refus_installation();
        if ecrit_ramfs("/update/pret", format!("pret generation={}\n", g).as_bytes()).is_err() {
            crate::serial_println!("BOUCHAUD_OTA_INSTALLATION_ABSENTE raison=pret_non_ecrit");
            return Issue::Rien;
        }
        s = 0;
        while !super::candidat_en_attente() && super::refus_installation() == refus_avant && s < 1800 {
            dort_s(1);
            s += 1;
        }
        if super::candidat_en_attente() { break; }
        if super::refus_installation() != refus_avant {
            historique(&format!("installation_refusee sequence={} essai={}", r.manifeste.sequence, essai));
        }
        if super::refus_installation() != refus_avant && essai < ESSAIS_INSTALLATION {
            crate::serial_println!("BOUCHAUD_OTA_INSTALLATION_ECHOUEE essai={} consequence=nouvel_essai", essai);
            continue;
        }
        crate::serial_println!("BOUCHAUD_OTA_INSTALLATION_ABSENTE essai={} attente_s={}", essai, s);
        return Issue::Rien;
    }
    crate::serial_println!("BOUCHAUD_OTA_CANDIDAT_PRET sequence={} essai={} attente_s={}", r.manifeste.sequence, essai, s);
    historique(&format!("candidat_pret sequence={} version={}", r.manifeste.sequence, r.manifeste.version()));
    etat("prochaine_installation", format!("prete sequence={} version={} redemarrage=en_attente", r.manifeste.sequence, r.manifeste.version()));
    // Le redemarrage : jamais pendant une session active, jamais sans accord.
    match politique() {
        Politique::Consenti => {
            crate::serial_println!("BOUCHAUD_OTA_REDEMARRAGE consentement=configuration sequence={}", r.manifeste.sequence);
            redemarre(r.manifeste.sequence, "configuration");
        }
        Politique::Demande => crate::serial_println!(
            "BOUCHAUD_OTA_REDEMARRAGE_EN_ATTENTE consentement_requis sequence={} commande=\"bo-update consentir\"", r.manifeste.sequence),
        Politique::Fenetre { debut, fin, inactif_min } => crate::serial_println!(
            "BOUCHAUD_OTA_REDEMARRAGE_EN_ATTENTE fenetre={:02}-{:02}h inactivite_min={} ou=\"bo-update consentir\" sequence={}",
            debut, fin, inactif_min, r.manifeste.sequence),
    }
    loop {
        if CONSENTEMENT.load(core::sync::atomic::Ordering::Acquire) {
            crate::serial_println!("BOUCHAUD_OTA_REDEMARRAGE consentement=utilisateur sequence={}", r.manifeste.sequence);
            redemarre(r.manifeste.sequence, "utilisateur");
        }
        if let Politique::Fenetre { debut, fin, inactif_min } = politique() {
            let inactif = crate::gui::window_manager::inactif_depuis_ms() / 60_000;
            if dans_la_fenetre(debut, fin) && inactif >= inactif_min {
                crate::serial_println!("BOUCHAUD_OTA_REDEMARRAGE consentement=fenetre inactivite_min={} sequence={}", inactif, r.manifeste.sequence);
                redemarre(r.manifeste.sequence, "fenetre");
            }
        }
        dort_s(1);
    }
}

fn redemarre(sequence: u64, pourquoi: &str) -> ! {
    historique(&format!("redemarrage sequence={} consentement={}", sequence, pourquoi));
    // L'etat et l'historique sont dans /persist : les ecrire AVANT de couper.
    let rc = crate::fs::persistance::synchronise();
    crate::serial_println!("BOUCHAUD_OTA_SYNC_AVANT_REDEMARRAGE rc={}", rc);
    crate::kernel::power::reboot();
}

/// Checks every configured source, logs one line per source.
pub fn verifie_tout() -> usize {
    *RETENUE.lock() = None;
    let s = sources();
    for source in &s {
        let r = verifie(source);
        etat("derniere_verification", format!("{} source={} resultat={}", horodatage(), source, match &r {
            Resultat::Disponible { sequence, .. } => format!("disponible sequence={}", sequence),
            Resultat::AJour { sequence } => format!("a_jour sequence={}", sequence),
            Resultat::Refuse(x) => format!("refuse raison={}", x),
        }));
        match r {
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
    // L'identite d'abord : /persist peut ne pas etre encore monte (montage
    // differe apres le bureau) -- on attend qu'il le soit, borne.
    let mut s = 0;
    while crate::platform::pc::installation::montage_differe_en_attente() && s < 120 {
        dort_s(1);
        s += 1;
    }
    note_identite();
    let mut echecs_consecutifs = 0u32;
    loop {
        // Verifier DES que le reseau est la (pas d'abandon apres un delai :
        // un cable branche plus tard declenche la verification).
        if !crate::net::bail_obtenu() {
            etat("reseau", String::from("attente_du_bail"));
            while !crate::net::bail_obtenu() { dort_s(5); }
            crate::serial_println!("BOUCHAUD_OTA_RESEAU_DISPONIBLE");
        }
        etat("reseau", String::from("bail_obtenu"));
        VERIFIER_MAINTENANT.store(false, core::sync::atomic::Ordering::Release);
        let n = verifie_tout();
        crate::serial_println!("BOUCHAUD_OTA_CYCLE sources={} prochain_s={}", n, PERIODE_S);
        etat("prochaine_verification_s", format!("{}", PERIODE_S));
        let attente = match installe_si_consenti() {
            Issue::EchecRecuperable(raison) => {
                // Un echec de RESEAU ou d'integrite d'une partie n'attend pas
                // six heures : relance bornee (60 s, 120 s, ... <= PERIODE_S).
                // La decision reste celle du manifeste SIGNE, revue a chaque
                // cycle : rien n'est installe sans lui.
                echecs_consecutifs = echecs_consecutifs.saturating_add(1);
                let s = relance_s(echecs_consecutifs);
                crate::serial_println!("BOUCHAUD_OTA_RELANCE dans_s={} echecs_consecutifs={} raison={}", s, echecs_consecutifs, raison);
                etat("prochaine_verification_s", format!("{}", s));
                s
            }
            Issue::Rien => { echecs_consecutifs = 0; PERIODE_S }
        };
        // Periode, interrompue par une demande (`bo-update verifier`) ou par
        // la perte puis le retour du reseau.
        let mut ecoule = 0;
        let mut perdu = false;
        while ecoule < attente {
            if VERIFIER_MAINTENANT.load(core::sync::atomic::Ordering::Acquire) { break; }
            let bail = crate::net::bail_obtenu();
            if !bail { perdu = true; }
            if perdu && bail { break; }
            dort_s(5);
            ecoule += 5;
        }
    }
}

/// Attente avant le cycle suivant apres `n` echecs consecutifs (n >= 1).
pub fn relance_s(n: u32) -> u64 {
    let mut s = RELANCE_MIN_S;
    for _ in 1..n.min(20) { s = s.saturating_mul(2); }
    s.min(PERIODE_S)
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
