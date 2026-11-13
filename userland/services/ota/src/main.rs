//! BOUCHAUD_OTA_ANNEAU3_V1 -- l'agent de mise a jour, en anneau 3 (P4).
//!
//! Lance par `serviced` (service `ota`, NON critique : un agent absent ne rend
//! pas le systeme malade, il ne le met plus a jour). Il fait ce que faisait le
//! fil noyau `ota` (src/kernel/update/ota.rs jusqu'a P4), avec les memes
//! lignes de journal :
//!
//!   1. au demarrage (bail reseau obtenu), au retour du reseau, toutes les 6 h
//!      et sur `bo-update verifier`, pour chaque `source=` de
//!      `/etc/bouchaud/ota.conf` : `manifest` et `manifest.sig` en HTTPS
//!      STRICT, puis `bouchaud_update_core::ota::decide` avec l'etat
//!      d'autorite du noyau (`/proc/bouchaud/ota` : cles, plancher,
//!      releases des slots) ;
//!   2. `telechargement=consenti` : la premiere release disponible, par
//!      parties (index, taille, SHA-256), le flux ENTIER contre le manifeste
//!      SIGNE ; preparee dans `/update/`, `pret generation=N` en dernier ;
//!   3. attend que `updated` l'installe (le noyau la revérifie), repropose
//!      apres un refus (3 essais), puis demande le redemarrage au noyau
//!      (`/update/redemarrage`), qui seul decide selon la politique et le
//!      consentement.
//!
//! Il ne detient AUCUNE capacite d'installation et n'ecrit sur aucun disque :
//! RAM (`/update/`) et STATE (`/persist/.bouchaud/`), comme avant.
mod natif;
mod web;

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use bouchaud_update_core as uc;
use uc::ota::{Decision, Refus};

const CONFIG: &str = "/etc/bouchaud/ota.conf";
const AUTORITE: &str = "/proc/bouchaud/ota";
const PERIODE_S: u64 = 6 * 3600;
const RELANCE_MIN_S: u64 = 60;
const ETAT: &str = "/persist/.bouchaud/ota-etat";
const HISTORIQUE: &str = "/persist/.bouchaud/ota-historique";
const HISTORIQUE_MAX: usize = 50;
const CHAMPS_MAX: usize = 32;
const CLE_MAX: usize = 40;
const VALEUR_MAX: usize = 240;
const MANIFESTE_MAX: usize = 4096;
/// Taille des parties publiee par fabrique_release.py (--parties-mio 4).
const PARTIE_MAX: usize = 4 * 1024 * 1024;
const INDEX_MAX: usize = 64 * 1024;
const TENTATIVES: u32 = 4;
const ESSAIS_INSTALLATION: u64 = 3;

fn horodatage() -> String {
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (a, mo, j, h, mi, se) = web::date_utc(s);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", a, mo, j, h, mi, se)
}

fn dort_s(s: u64) { std::thread::sleep(Duration::from_secs(s)); }

// ---- l'etat d'autorite du noyau -------------------------------------------

struct Autorite { champs: Vec<(String, String)>, cles: Vec<uc::manifeste::CleDeConfiance> }

impl Autorite {
    fn lit() -> Option<Autorite> {
        let t = fs::read_to_string(AUTORITE).ok()?;
        let mut champs = Vec::new();
        let mut cles = Vec::new();
        for l in t.lines() {
            let Some((k, v)) = l.split_once('=') else { continue };
            if k == "cle_de_confiance" {
                let p: Vec<&str> = v.split_whitespace().collect();
                if let [nom, hex, dev] = p[..] {
                    if let Some(publique) = hex32(hex) {
                        cles.push(uc::manifeste::CleDeConfiance {
                            nom: Box::leak(nom.to_string().into_boxed_str()),
                            publique, developpement: dev == "1",
                        });
                    }
                }
            } else {
                champs.push((k.to_string(), v.to_string()));
            }
        }
        Some(Autorite { champs, cles })
    }
    fn champ(&self, k: &str) -> &str {
        self.champs.iter().find(|(x, _)| x == k).map(|(_, v)| v.as_str()).unwrap_or("")
    }
    fn nombre(&self, k: &str) -> u64 { self.champ(k).parse().unwrap_or(0) }
    fn drapeau(&self, k: &str) -> bool { self.champ(k) == "1" }
    fn lecture(&self) -> uc::anti_retour::Lecture {
        let v = self.champ("anti_retour");
        if v == "neuf" { return uc::anti_retour::Lecture::Neuf; }
        let Some(r) = v.strip_prefix("valide ") else { return uc::anti_retour::Lecture::Illisible };
        let n = |c: &str| r.split_whitespace().find_map(|x| x.strip_prefix(c)).and_then(|x| x.parse::<u64>().ok());
        match (n("sequence="), n("plancher="), n("slot_a="), n("slot_b=")) {
            (Some(sequence), Some(plancher), Some(a), Some(b)) =>
                uc::anti_retour::Lecture::Valide(uc::anti_retour::Registre { sequence, plancher, slots: [a, b] }),
            _ => uc::anti_retour::Lecture::Illisible,
        }
    }
}

fn hex32(h: &str) -> Option<[u8; 32]> {
    if h.len() != 64 { return None; }
    let mut o = [0u8; 32];
    for (i, b) in o.iter_mut().enumerate() { *b = u8::from_str_radix(&h[2 * i..2 * i + 2], 16).ok()?; }
    Some(o)
}

fn hex(h: &[u8]) -> String { h.iter().map(|b| format!("{:02x}", b)).collect() }

/// Bloque jusqu'a ce que le noyau publie son etat (il l'a toujours fait des
/// qu'il a lance serviced ; une absence est une regression, dite une fois).
fn autorite() -> Autorite {
    let mut dit = false;
    loop {
        if let Some(a) = Autorite::lit() { return a; }
        if !dit { println!("BOUCHAUD_OTA_AUTORITE_ABSENTE chemin={} consequence=attente", AUTORITE); dit = true; }
        dort_s(5);
    }
}

// ---- etat et historique (STATE) -------------------------------------------

struct Etat { champs: Vec<(String, String)> }

impl Etat {
    /// L'etat du demarrage PRECEDENT, relu une fois, borne ; une ligne mal
    /// formee est ignoree.
    fn charge() -> Etat {
        let mut champs: Vec<(String, String)> = Vec::new();
        let ancien = fs::read(ETAT).map(|c| String::from_utf8_lossy(&c).into_owned()).unwrap_or_default();
        for l in ancien.lines() {
            if champs.len() >= CHAMPS_MAX { break; }
            let Some((k, v)) = l.split_once('=') else { continue };
            if k.is_empty() || k.len() > CLE_MAX || v.len() > VALEUR_MAX { continue; }
            if !k.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') { continue; }
            if champs.iter().any(|(x, _)| x == k) { continue; }
            champs.push((k.to_string(), v.to_string()));
        }
        Etat { champs }
    }
    fn ecrit(&self) {
        if !Path::new("/persist").is_dir() { return; }
        let _ = fs::create_dir_all("/persist/.bouchaud");
        let mut t = String::new();
        for (k, v) in &self.champs { t.push_str(&format!("{}={}\n", k, v)); }
        let _ = fs::write(ETAT, t);
    }
    fn mets(&mut self, k: &str, v: String) {
        match self.champs.iter_mut().find(|(x, _)| x == k) {
            Some(e) => e.1 = v,
            None => self.champs.push((k.to_string(), v)),
        }
        self.ecrit();
    }
    fn retire(&mut self, k: &str) { self.champs.retain(|(x, _)| x != k); self.ecrit(); }
    fn lu(&self, k: &str) -> Option<String> { self.champs.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone()) }
}

fn historique(ligne: &str) {
    if Path::new("/persist").is_dir() {
        let _ = fs::create_dir_all("/persist/.bouchaud");
        let ancien = fs::read(HISTORIQUE).map(|c| String::from_utf8_lossy(&c).into_owned()).unwrap_or_default();
        let mut lignes: Vec<String> = ancien.lines().map(String::from).collect();
        lignes.push(format!("{} {}", horodatage(), ligne));
        let debut = lignes.len().saturating_sub(HISTORIQUE_MAX);
        let mut t = String::new();
        for l in &lignes[debut..] { t.push_str(l); t.push('\n'); }
        let _ = fs::write(HISTORIQUE, t);
    }
    println!("BOUCHAUD_OTA_HISTORIQUE {}", ligne);
}

// ---- configuration ---------------------------------------------------------

fn config() -> String { fs::read_to_string(CONFIG).unwrap_or_default() }

fn consenti(cle: &str) -> bool { config().lines().any(|l| l.trim() == format!("{}=consenti", cle)) }

fn sources() -> Vec<String> {
    config().lines()
        .filter_map(|l| l.trim().strip_prefix("source="))
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| s.starts_with("https://") || s.starts_with("http://"))
        .collect()
}

// ---- verification -----------------------------------------------------------

fn telecharge(url: &str, max: usize) -> Result<Vec<u8>, &'static str> {
    match web::get(url, max) {
        Err(web::Echec::Confiance(l)) => {
            println!("BOUCHAUD_OTA_TLS_REFUS url={} {}", url, l);
            Err("certificat_non_fiable")
        }
        Err(web::Echec::Transport(e)) | Err(web::Echec::Protocole(e)) => {
            println!("BOUCHAUD_OTA_TRANSPORT url={} {}", url, e);
            Err("injoignable")
        }
        Ok(r) if r.statut != 200 => {
            println!("BOUCHAUD_OTA_HTTP_STATUT url={} statut={}", url, r.statut);
            Err("http_erreur")
        }
        Ok(r) if r.corps.is_empty() => Err("injoignable"),
        Ok(r) if r.corps.len() > max => Err("trop_grand"),
        Ok(r) => Ok(r.corps),
    }
}

enum Resultat { Disponible { version: String, sequence: u64, octets: u64 }, AJour { sequence: u64 }, Refuse(&'static str) }

struct Retenue { source: String, texte: Vec<u8>, signature: [u8; 64], manifeste: uc::manifeste::Manifeste }

fn verifie(source: &str, retenue: &mut Option<Retenue>) -> Resultat {
    let texte = match telecharge(&format!("{}/manifest", source), MANIFESTE_MAX) {
        Ok(t) => t,
        Err(e) => return Resultat::Refuse(match e {
            "certificat_non_fiable" => "certificat_non_fiable",
            "http_erreur" => "manifeste_http_erreur",
            "injoignable" => "manifeste_injoignable",
            _ => "manifeste_trop_grand",
        }),
    };
    let sig = match telecharge(&format!("{}/manifest.sig", source), 64) {
        Ok(s) if s.len() == 64 => s,
        Ok(_) => return Resultat::Refuse("signature_mal_formee"),
        Err(_) => return Resultat::Refuse("signature_injoignable"),
    };
    let mut signature = [0u8; 64];
    signature.copy_from_slice(&sig);
    // L'etat d'autorite RELU a chaque decision : jamais une copie perimee.
    let a = autorite();
    let materiel = a.champ("materiel").to_string();
    match uc::ota::decide(&texte, &signature, &a.cles, a.lecture(), a.nombre("release_courante"), a.nombre("release_en_echec"), &materiel) {
        Ok(Decision::Disponible(m)) => {
            if retenue.is_none() {
                *retenue = Some(Retenue { source: source.to_string(), texte, signature, manifeste: m });
            }
            Resultat::Disponible { version: m.version().to_string(), sequence: m.sequence, octets: m.octets }
        }
        Ok(Decision::AJour { sequence }) => Resultat::AJour { sequence },
        Err(r) => Resultat::Refuse(Refus::texte(r)),
    }
}

fn verifie_tout(etat: &mut Etat, retenue: &mut Option<Retenue>) -> usize {
    *retenue = None;
    let s = sources();
    for source in &s {
        let r = verifie(source, retenue);
        etat.mets("derniere_verification", format!("{} source={} resultat={}", horodatage(), source, match &r {
            Resultat::Disponible { sequence, .. } => format!("disponible sequence={}", sequence),
            Resultat::AJour { sequence } => format!("a_jour sequence={}", sequence),
            Resultat::Refuse(x) => format!("refuse raison={}", x),
        }));
        match r {
            Resultat::Disponible { version, sequence, octets } => println!(
                "BOUCHAUD_OTA_VERIFIE source={} resultat=disponible version={} sequence={} octets={} installation=consentement_requis",
                source, version, sequence, octets),
            Resultat::AJour { sequence } => println!("BOUCHAUD_OTA_VERIFIE source={} resultat=a_jour sequence={}", source, sequence),
            Resultat::Refuse(r) => println!("BOUCHAUD_OTA_VERIFIE source={} resultat=refuse raison={}", source, r),
        }
    }
    s.len()
}

// ---- telechargement -----------------------------------------------------------

/// Un echec de transport est repris ; un refus de confiance ou une taille
/// hors bornes ne l'est pas.
fn telecharge_avec_reprise(url: &str, max: usize) -> Result<Vec<u8>, &'static str> {
    let mut derniere = "injoignable";
    for essai in 1..=TENTATIVES {
        match telecharge(url, max) {
            Ok(c) => {
                if essai > 1 { println!("BOUCHAUD_OTA_REPRISE url={} essai={} resultat=ok", url, essai); }
                return Ok(c);
            }
            Err(e) => {
                derniere = e;
                println!("BOUCHAUD_OTA_REPRISE url={} essai={} raison={}", url, essai, e);
                if e == "trop_grand" || e == "certificat_non_fiable" { break; }
                dort_s(2u64 << essai);
            }
        }
    }
    Err(derniere)
}

/// Une partie : une panne de transport OU un corps COURT (coupe en route) est
/// reprise, cette partie seulement. Un corps de la bonne taille dont
/// l'empreinte differe n'est PAS repris : c'est le contenu, pas le transport.
fn telecharge_partie(url: &str, taille: usize) -> Result<Vec<u8>, &'static str> {
    let mut derniere = "partie_injoignable";
    for essai in 1..=TENTATIVES {
        let raison = match telecharge(url, PARTIE_MAX) {
            Ok(c) if c.len() == taille => {
                if essai > 1 { println!("BOUCHAUD_OTA_REPRISE url={} essai={} resultat=ok", url, essai); }
                return Ok(c);
            }
            Ok(c) if c.len() < taille => { derniere = "partie_tronquee"; format!("partie_tronquee octets={}/{}", c.len(), taille) }
            Ok(_) => return Err("partie_taille_differente"),
            Err("trop_grand") => return Err("partie_taille_differente"),
            Err("certificat_non_fiable") => return Err("certificat_non_fiable"),
            Err(e) => { derniere = "partie_injoignable"; e.to_string() }
        };
        println!("BOUCHAUD_OTA_REPRISE url={} essai={} raison={}", url, essai, raison);
        if essai < TENTATIVES { dort_s(2u64 << essai); }
    }
    Err(derniere)
}

fn ecrit_update(nom: &str, donnees: &[u8]) -> Result<(), &'static str> {
    fs::create_dir_all("/update").map_err(|_| "update_absent")?;
    fs::write(format!("/update/{}", nom), donnees).map_err(|_| "fichier_trop_grand")
}

fn telecharge_et_prepare(r: &Retenue, t0: Instant) -> Result<u64, &'static str> {
    let index = telecharge_avec_reprise(&format!("{}/system-slot.parts", r.source), INDEX_MAX).map_err(|_| "index_injoignable")?;
    let index = String::from_utf8_lossy(&index).into_owned();
    let mut total = bouchaud_tls::sha256::Sha256::new();
    let mut octets = 0u64;
    let mut k = 0usize;
    for ligne in index.lines().filter(|l| !l.trim().is_empty()) {
        let champs: Vec<&str> = ligne.split_whitespace().collect();
        let attendu_nom = format!("system-slot.img.part{:03}", k);
        if champs.len() != 3 || champs[0] != attendu_nom { return Err("index_mal_forme"); }
        let taille: usize = champs[1].parse().map_err(|_| "index_mal_forme")?;
        if taille == 0 || taille > PARTIE_MAX { return Err("partie_hors_bornes"); }
        let corps = telecharge_partie(&format!("{}/{}", r.source, attendu_nom), taille)?;
        if hex(&bouchaud_tls::sha256::sha256(&corps)) != champs[2] { return Err("partie_empreinte_differente"); }
        total.update(&corps);
        octets += corps.len() as u64;
        if octets > r.manifeste.octets { return Err("image_plus_longue_que_le_manifeste"); }
        ecrit_update(&format!("system.img.part{:03}", k), &corps)?;
        // t_ms : l'instant de la partie (les lignes de l'anneau 3 n'ont pas
        // l'horodatage serie du noyau ; ota-reseau mesure les ecarts avec lui).
        println!("BOUCHAUD_OTA_PARTIE n={} octets={} total={} t_ms={}", k, corps.len(), octets, t0.elapsed().as_millis());
        k += 1;
    }
    // La SEULE verification qui compte : le flux entier est celui qui a ete signe.
    if octets != r.manifeste.octets { return Err("image_taille_differente_du_manifeste"); }
    if total.finalize() != r.manifeste.empreinte { return Err("image_empreinte_differente_du_manifeste"); }
    ecrit_update("manifest", &r.texte)?;
    ecrit_update("manifest.sig", &r.signature)?;
    Ok(octets)
}

enum Issue { Rien, EchecRecuperable(&'static str), Candidat }

/// La generation de `pret` : jamais reutilisee, meme apres un redemarrage de
/// l'agent (updated traite chaque generation une fois).
fn generation_suivante() -> u64 {
    let deja = fs::read_to_string("/update/pret").ok()
        .and_then(|t| t.split_whitespace().find_map(|c| c.strip_prefix("generation=").and_then(|g| g.parse::<u64>().ok())))
        .unwrap_or(0);
    deja + 1
}

fn installe_si_consenti(etat: &mut Etat, retenue: &mut Option<Retenue>, t0: Instant) -> Issue {
    let Some(r) = retenue.take() else { return Issue::Rien };
    if !consenti("telechargement") {
        println!("BOUCHAUD_OTA_TELECHARGEMENT_EN_ATTENTE consentement_requis sequence={}", r.manifeste.sequence);
        return Issue::Rien;
    }
    let _ = ecrit_update("pret", b"en_cours\n");
    etat.mets("telechargement_en_cours", format!("sequence={} debut={}", r.manifeste.sequence, horodatage()));
    let issue = telecharge_et_prepare(&r, t0);
    etat.retire("telechargement_en_cours");
    match issue {
        Ok(octets) => {
            println!("BOUCHAUD_OTA_TELECHARGE source={} sequence={} octets={} empreinte=signee_verifiee prepare=/update",
                r.source, r.manifeste.sequence, octets);
            etat.mets("dernier_telechargement", format!("{} ok sequence={} octets={}", horodatage(), r.manifeste.sequence, octets));
        }
        Err(e) => {
            etat.mets("dernier_telechargement", format!("{} echec raison={}", horodatage(), e));
            historique(&format!("telechargement_refuse sequence={} raison={}", r.manifeste.sequence, e));
            println!("BOUCHAUD_OTA_TELECHARGEMENT_REFUSE source={} raison={}", r.source, e);
            return Issue::EchecRecuperable(e);
        }
    }
    // Chaque offre a `updated` est une NOUVELLE generation de `pret`. Un refus
    // compte par le noyau termine l'essai ; les parties, deja verifiees, sont
    // reproposees (au plus ESSAIS_INSTALLATION).
    let mut essai = 0u64;
    let mut s;
    loop {
        essai += 1;
        let g = generation_suivante();
        let refus_avant = autorite().nombre("refus_installation");
        if ecrit_update("pret", format!("pret generation={}\n", g).as_bytes()).is_err() {
            println!("BOUCHAUD_OTA_INSTALLATION_ABSENTE raison=pret_non_ecrit");
            return Issue::Rien;
        }
        s = 0;
        let mut a = autorite();
        while !a.drapeau("candidat_en_attente") && a.nombre("refus_installation") == refus_avant && s < 1800 {
            dort_s(1);
            s += 1;
            a = autorite();
        }
        if a.drapeau("candidat_en_attente") { break; }
        let refuse = a.nombre("refus_installation") != refus_avant;
        if refuse { historique(&format!("installation_refusee sequence={} essai={}", r.manifeste.sequence, essai)); }
        if refuse && essai < ESSAIS_INSTALLATION {
            println!("BOUCHAUD_OTA_INSTALLATION_ECHOUEE essai={} consequence=nouvel_essai", essai);
            continue;
        }
        println!("BOUCHAUD_OTA_INSTALLATION_ABSENTE essai={} attente_s={}", essai, s);
        return Issue::Rien;
    }
    println!("BOUCHAUD_OTA_CANDIDAT_PRET sequence={} essai={} attente_s={}", r.manifeste.sequence, essai, s);
    historique(&format!("candidat_pret sequence={} version={}", r.manifeste.sequence, r.manifeste.version()));
    etat.mets("prochaine_installation", format!("prete sequence={} version={} redemarrage=en_attente", r.manifeste.sequence, r.manifeste.version()));
    // Le redemarrage est un acte du NOYAU : il ne suit cette demande que si
    // META porte un candidat, selon la politique ou le consentement.
    let _ = ecrit_update("redemarrage", format!("sequence={} version={}\n", r.manifeste.sequence, r.manifeste.version()).as_bytes());
    Issue::Candidat
}

fn relance_s(n: u32) -> u64 {
    let mut s = RELANCE_MIN_S;
    for _ in 1..n.min(20) { s = s.saturating_mul(2); }
    s.min(PERIODE_S)
}

/// Identite du systeme qui tourne et des slots, au demarrage.
fn note_identite(etat: &mut Etat) {
    if let Some(v) = etat.lu("telechargement_en_cours") {
        historique(&format!("telechargement_interrompu {}", v));
        println!("BOUCHAUD_OTA_TELECHARGEMENT_INTERROMPU {} consequence=reprise_depuis_la_premiere_partie", v);
        etat.mets("dernier_telechargement", format!("{} interrompu {}", horodatage(), v));
        etat.retire("telechargement_en_cours");
    }
    let version = fs::read_to_string("/etc/bouchaud/version").unwrap_or_default();
    let champ = |k: &str| version.lines().find_map(|l| l.strip_prefix(&format!("{}=", k)).map(String::from)).unwrap_or_else(|| "?".into());
    etat.mets("version", champ("version"));
    etat.mets("commit", champ("commit"));
    etat.mets("canal", config().lines().find_map(|l| l.trim().strip_prefix("canal=").map(String::from)).unwrap_or_else(|| "stable".into()));
    let a = autorite();
    let slot = a.champ("slot_actif").to_string();
    etat.mets("slot_actif", slot.clone());
    etat.mets("release_slot_actif", a.champ("release_courante").to_string());
    if let uc::anti_retour::Lecture::Valide(r) = a.lecture() {
        let autre = if slot == "A" { 1 } else { 0 };
        etat.mets("release_slot_inactif", format!("{}", r.slots[autre]));
        etat.mets("plancher", format!("{}", r.plancher));
    }
    if a.nombre("release_en_echec") != 0 { etat.mets("release_en_echec", a.champ("release_en_echec").to_string()); }
    let raison = a.champ("dernier_demarrage").to_string();
    if raison.contains("rollback") || raison.contains("recovery") {
        historique(&format!("rollback raison={} slot={}", raison, slot));
    }
    if !raison.is_empty() { etat.mets("dernier_demarrage", raison); }
    etat.mets("prochaine_installation", "aucune".into());
    etat.mets("agent", "anneau3".into());
}

/// `bo-ota --sonde URL` : une requete, en mode strict, et son verdict sur une
/// ligne (diagnostic sur la machine, et preuve du TLS de l'anneau 3 sur
/// l'hote : tools/ci/test_bo_ota_tls.sh). Code 0 : 200 recu.
fn sonde(url: &str) -> i32 {
    match web::get(url, PARTIE_MAX) {
        Ok(r) => {
            println!("BO_OTA_SONDE url={} statut={} octets={} sha256={}", url, r.statut, r.corps.len(),
                hex(&bouchaud_tls::sha256::sha256(&r.corps)));
            if r.statut == 200 { 0 } else { 1 }
        }
        Err(web::Echec::Confiance(l)) => { println!("BO_OTA_SONDE url={} refus=confiance {}", url, l); 2 }
        Err(web::Echec::Transport(e)) => { println!("BO_OTA_SONDE url={} refus=transport {}", url, e); 3 }
        Err(web::Echec::Protocole(e)) => { println!("BO_OTA_SONDE url={} refus=protocole {}", url, e); 4 }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 3 && args[1] == "--sonde" {
        std::process::exit(sonde(&args[2]));
    }
    if let Some(h) = natif::canal_de_controle() {
        std::thread::spawn(move || natif::controle(h));
    }
    let t0 = Instant::now();
    if sources().is_empty() {
        // Inerte, mais VIVANT : serviced le redemarrerait sinon en boucle.
        println!("BOUCHAUD_OTA_INACTIF raison=sans_source config={}", CONFIG);
        loop { dort_s(3600); }
    }
    println!("BOUCHAUD_OTA_AGENT anneau=3 pid={} tls=bouchaud-tls(strict) autorite={}", std::process::id(), AUTORITE);
    // /persist peut ne pas etre encore monte (montage differe apres le
    // bureau) : on attend qu'il le soit, borne.
    let mut s = 0;
    while autorite().drapeau("persist_en_attente") && s < 120 { dort_s(1); s += 1; }
    let mut etat = Etat::charge();
    note_identite(&mut etat);
    let mut retenue: Option<Retenue> = None;
    let mut echecs_consecutifs = 0u32;
    let mut verifications_vues = autorite().nombre("verifications_demandees");
    loop {
        // Verifier DES que le reseau est la (un cable branche plus tard
        // declenche la verification).
        if !autorite().drapeau("bail_reseau") {
            etat.mets("reseau", "attente_du_bail".into());
            while !autorite().drapeau("bail_reseau") { dort_s(5); }
            println!("BOUCHAUD_OTA_RESEAU_DISPONIBLE");
        }
        etat.mets("reseau", "bail_obtenu".into());
        verifications_vues = autorite().nombre("verifications_demandees").max(verifications_vues);
        let n = verifie_tout(&mut etat, &mut retenue);
        println!("BOUCHAUD_OTA_CYCLE sources={} prochain_s={}", n, PERIODE_S);
        etat.mets("prochaine_verification_s", format!("{}", PERIODE_S));
        let attente = match installe_si_consenti(&mut etat, &mut retenue, t0) {
            Issue::EchecRecuperable(raison) => {
                echecs_consecutifs = echecs_consecutifs.saturating_add(1);
                let s = relance_s(echecs_consecutifs);
                println!("BOUCHAUD_OTA_RELANCE dans_s={} echecs_consecutifs={} raison={}", s, echecs_consecutifs, raison);
                etat.mets("prochaine_verification_s", format!("{}", s));
                s
            }
            Issue::Rien => { echecs_consecutifs = 0; PERIODE_S }
            // Le noyau redemarre (ou attend le consentement) : plus rien a
            // telecharger d'ici la.
            Issue::Candidat => loop { dort_s(3600); },
        };
        // Periode, interrompue par `bo-update verifier` ou par la perte puis
        // le retour du reseau.
        let mut ecoule = 0;
        let mut perdu = false;
        while ecoule < attente {
            let a = autorite();
            if a.nombre("verifications_demandees") != verifications_vues {
                verifications_vues = a.nombre("verifications_demandees");
                break;
            }
            let bail = a.drapeau("bail_reseau");
            if !bail { perdu = true; }
            if perdu && bail { break; }
            dort_s(5);
            ecoule += 5;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relance_bornee() {
        assert_eq!(relance_s(1), 60);
        assert_eq!(relance_s(2), 120);
        assert_eq!(relance_s(30), PERIODE_S);
    }

    #[test]
    fn hex_aller_retour() {
        let h = "00ff".repeat(16);
        assert_eq!(hex(&hex32(&h).unwrap()), h);
        assert!(hex32("zz").is_none());
    }
}
