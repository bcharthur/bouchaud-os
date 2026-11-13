//! OTA : ce que le NOYAU garde quand l'agent tourne en anneau 3
//! (BOUCHAUD_OTA_ANNEAU3_V1, P4).
//!
//! # Partage des roles
//!
//! L'AGENT (`/bin/bo-ota`, `userland/services/ota/`, service non critique de
//! `serviced`) fait tout le travail expose au reseau : lecture de
//! `/etc/bouchaud/ota.conf`, HTTPS avec la pile TLS du depot
//! (`system/tls-core`, la meme que celle du noyau) en mode strict, decision
//! signee (`bouchaud_update_core::ota::decide`), telechargement par parties
//! avec reprise, preparation dans `/update/`, `pret generation=N` pour
//! `updated`, etat et historique dans `/persist/.bouchaud/`.
//!
//! Le NOYAU garde l'autorite, et rien de ce que l'agent ecrit ne la remplace :
//!   - `updated` (anneau 3, seul detenteur de la capacite d'installation)
//!     installe ; le noyau REVERIFIE tout au `begin` et au `finish`
//!     (signature, materiel, anti-retour, empreinte, slot inactif seul) ;
//!   - l'etat d'autorite est PUBLIE en lecture seule, regenere a chaque
//!     ouverture : `/proc/bouchaud/ota` ([`instantane`]). L'agent n'en tient
//!     pas de copie qu'il pourrait alterer : il le relit ;
//!   - le REDEMARRAGE reste un acte du noyau ([`lance`]) : jamais sans un
//!     candidat dans META, jamais sans la politique de la configuration ou le
//!     consentement de l'utilisateur (`bo-update consentir`).
//!
//! Un agent compromis peut donc gaspiller de la bande passante ou se taire ;
//! il ne peut ni installer une image non signee, ni redescendre de version,
//! ni redemarrer la machine sans candidat verifie.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use bouchaud_update_core as uc;

/// The agent's configuration (in the system image, read-only).
pub const CONFIG: &str = "/etc/bouchaud/ota.conf";
/// Etat et historique, ecrits par l'agent dans STATE.
const ETAT: &str = "/persist/.bouchaud/ota-etat";
const HISTORIQUE: &str = ".bouchaud/ota-historique";
const HISTORIQUE_MAX: usize = 50;
/// La demande de redemarrage de l'agent, une fois le candidat enregistre :
/// `sequence=N version=V`.
const DEMANDE_REDEMARRAGE: &str = "/update/redemarrage";

/// Consentement donne pendant CE demarrage (`bo-update consentir`).
static CONSENTEMENT: AtomicBool = AtomicBool::new(false);
/// Verifications demandees (`bo-update verifier`) : un COMPTEUR, que l'agent
/// compare a la derniere valeur vue (aucune demande n'est perdue ni rejouee).
static VERIFICATIONS: AtomicU64 = AtomicU64::new(0);

fn horodatage() -> String {
    let d = crate::arch::x86_64::rtc::now_utc();
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", d.year, d.month, d.day, d.hour, d.minute, d.second)
}

fn lit(chemin: &str) -> Option<Vec<u8>> {
    let fs = crate::fs::ramfs::fs();
    fs.resolve(chemin, 0).map(|i| fs.nodes[i].content.clone())
}

fn config() -> String {
    lit(CONFIG).map(|c| String::from_utf8_lossy(&c).into_owned()).unwrap_or_default()
}

fn a_des_sources() -> bool {
    config().lines().any(|l| {
        let s = l.trim().strip_prefix("source=").unwrap_or("").trim();
        s.starts_with("https://") || s.starts_with("http://")
    })
}

/// `bo-update consentir` : redemarrer vers la release preparee.
pub fn consentir() { CONSENTEMENT.store(true, Ordering::Release); }
/// `bo-update verifier` : verifier les sources sans attendre la periode.
pub fn verifier_maintenant() { VERIFICATIONS.fetch_add(1, Ordering::AcqRel); }

/// L'etat courant, tel que l'agent l'a ecrit (`bo-update status`).
pub fn etat_texte() -> String {
    match lit(ETAT) {
        Some(c) if !c.is_empty() => String::from_utf8_lossy(&c).into_owned(),
        _ => String::from("agent=pas_encore_d_etat\n"),
    }
}

pub fn historique_texte() -> String {
    lit(&format!("/persist/{}", HISTORIQUE)).map(|c| String::from_utf8_lossy(&c).into_owned()).unwrap_or_default()
}

/// BOUCHAUD_OTA_ANNEAU3_V1 -- `/proc/bouchaud/ota`, regenere a CHAQUE
/// ouverture. Tout ce que l'agent a besoin de savoir du noyau, et rien qu'il
/// puisse ecrire. Une ligne `cle=valeur` par fait ; `cle_de_confiance=` une
/// fois par cle (nom, cle publique hexadecimale, developpement 0/1).
pub fn instantane() -> Vec<u8> {
    let mut t = String::from("schema=bouchaud-ota-noyau/1\n");
    t.push_str(&format!("materiel={}\n", super::MATERIEL));
    for c in super::cles_de_confiance() {
        let mut hex = String::new();
        for b in c.publique.iter() { hex.push_str(&format!("{:02x}", b)); }
        t.push_str(&format!("cle_de_confiance={} {} {}\n", c.nom, hex, c.developpement as u8));
    }
    let (lecture, courante, echouee) = super::etat_anti_retour();
    match lecture {
        uc::anti_retour::Lecture::Valide(r) => t.push_str(&format!(
            "anti_retour=valide sequence={} plancher={} slot_a={} slot_b={}\n",
            r.sequence, r.plancher, r.slots[0], r.slots[1])),
        uc::anti_retour::Lecture::Neuf => t.push_str("anti_retour=neuf\n"),
        uc::anti_retour::Lecture::Illisible => t.push_str("anti_retour=illisible\n"),
    }
    t.push_str(&format!("release_courante={}\nrelease_en_echec={}\n", courante, echouee));
    match super::passage() {
        Some(p) => t.push_str(&format!("slot_actif={}\ndernier_demarrage={}\n",
            if p.slot == uc::Slot::A { "A" } else { "B" }, p.raison.texte())),
        None => t.push_str("slot_actif=-\n"),
    }
    t.push_str(&format!("candidat_en_attente={}\n", super::candidat_en_attente() as u8));
    t.push_str(&format!("refus_installation={}\n", super::refus_installation()));
    t.push_str(&format!("bail_reseau={}\n", crate::net::bail_obtenu() as u8));
    t.push_str(&format!("persist_en_attente={}\n",
        crate::platform::pc::installation::montage_differe_en_attente() as u8));
    t.push_str(&format!("verifications_demandees={}\n", VERIFICATIONS.load(Ordering::Acquire)));
    t.push_str(&format!("consentement={}\n", CONSENTEMENT.load(Ordering::Acquire) as u8));
    t.push_str(&format!("horodatage={}\n", horodatage()));
    t.into_bytes()
}

/// Ajoute une ligne a l'historique de l'agent, sous le verrou du RAMFS (le
/// noyau n'y ecrit qu'une ligne : son redemarrage).
fn historique(ligne: &str) {
    {
        let mut fs = crate::fs::ramfs::fs();
        let Some(racine) = fs.resolve("/persist", 0) else { return };
        let d = match fs.find_child(racine, ".bouchaud") {
            Some(d) => d,
            None => match fs.mkdir_at(racine, ".bouchaud") { Ok(d) => d, Err(_) => return },
        };
        let ancien = fs.find_child(d, "ota-historique")
            .map(|f| String::from_utf8_lossy(&fs.nodes[f].content).into_owned())
            .unwrap_or_default();
        let mut lignes: Vec<&str> = ancien.lines().collect();
        let nouvelle = format!("{} {}", horodatage(), ligne);
        lignes.push(&nouvelle);
        let debut = lignes.len().saturating_sub(HISTORIQUE_MAX);
        let mut t = String::new();
        for l in &lignes[debut..] { t.push_str(l); t.push('\n'); }
        if let Ok(f) = fs.touch_at(d, "ota-historique") {
            let _ = fs.write_node_bytes(f, t.as_bytes());
        }
    }
    crate::serial_println!("BOUCHAUD_OTA_HISTORIQUE {}", ligne);
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

fn dort_s(s: u64) {
    crate::kernel::task::sleep_ticks(crate::kernel::timer::ms_to_ticks(s.saturating_mul(1000)));
}

fn redemarre(sequence: &str, pourquoi: &str) -> ! {
    historique(&format!("redemarrage sequence={} consentement={}", sequence, pourquoi));
    // L'etat et l'historique sont dans /persist : les ecrire AVANT de couper.
    let rc = crate::fs::persistance::synchronise();
    crate::serial_println!("BOUCHAUD_OTA_SYNC_AVANT_REDEMARRAGE rc={}", rc);
    crate::kernel::power::reboot();
}

/// Le gardien du redemarrage : attend la demande de l'agent, ne la suit que
/// si META porte un candidat, puis la politique ou le consentement.
fn fil_redemarrage() -> ! {
    let mut annonce = false;
    loop {
        dort_s(1);
        let Some(demande) = lit(DEMANDE_REDEMARRAGE) else { continue };
        let demande = String::from_utf8_lossy(&demande).into_owned();
        let sequence = demande.split_whitespace()
            .find_map(|c| c.strip_prefix("sequence="))
            .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .map(String::from);
        let Some(sequence) = sequence else { continue };
        if !super::candidat_en_attente() {
            // Une demande sans candidat n'est pas suivie (agent fautif, ou
            // ecrite par un autre) : le noyau le dit une fois.
            if !annonce {
                crate::serial_println!("BOUCHAUD_OTA_REDEMARRAGE_REFUSE raison=aucun_candidat_dans_meta sequence={}", sequence);
                annonce = true;
            }
            continue;
        }
        if CONSENTEMENT.load(Ordering::Acquire) {
            crate::serial_println!("BOUCHAUD_OTA_REDEMARRAGE consentement=utilisateur sequence={}", sequence);
            redemarre(&sequence, "utilisateur");
        }
        match politique() {
            Politique::Consenti => {
                crate::serial_println!("BOUCHAUD_OTA_REDEMARRAGE consentement=configuration sequence={}", sequence);
                redemarre(&sequence, "configuration");
            }
            Politique::Fenetre { debut, fin, inactif_min } => {
                let inactif = crate::gui::window_manager::inactif_depuis_ms() / 60_000;
                if dans_la_fenetre(debut, fin) && inactif >= inactif_min {
                    crate::serial_println!("BOUCHAUD_OTA_REDEMARRAGE consentement=fenetre inactivite_min={} sequence={}", inactif, sequence);
                    redemarre(&sequence, "fenetre");
                }
                if !annonce {
                    crate::serial_println!(
                        "BOUCHAUD_OTA_REDEMARRAGE_EN_ATTENTE fenetre={:02}-{:02}h inactivite_min={} ou=\"bo-update consentir\" sequence={}",
                        debut, fin, inactif_min, sequence);
                    annonce = true;
                }
            }
            Politique::Demande => {
                if !annonce {
                    crate::serial_println!(
                        "BOUCHAUD_OTA_REDEMARRAGE_EN_ATTENTE consentement_requis sequence={} commande=\"bo-update consentir\"", sequence);
                    annonce = true;
                }
            }
        }
    }
}

/// Started at boot. Inert without a configuration. L'agent lui-meme est
/// lance par `serviced` (service `ota`, non critique).
pub fn lance() {
    if !a_des_sources() {
        crate::serial_println!("BOUCHAUD_OTA_INACTIF raison=sans_source config={}", CONFIG);
        return;
    }
    crate::serial_println!("BOUCHAUD_OTA_NOYAU agent=anneau3 chemin=/bin/bo-ota autorite=/proc/bouchaud/ota redemarrage=noyau");
    if !crate::kernel::task::spawn_noyau(fil_redemarrage, "ota-redemarrage") {
        crate::serial_println!("BOUCHAUD_OTA_INACTIF raison=tache_non_creee");
    }
}
