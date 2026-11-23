//! BOUCHAUD_COMPOSITORD_PRESENTATION_V1 -- tranche 1 du compositeur ring 3.
//!
//! `bo_display_present(affichage, region, demande*)` : un processus ring 3
//! qui detient la CAPACITE D'AFFICHAGE (objet `Display`, droit
//! `DISPLAY_PRESENT`) fait poser un rectangle d'une de ses regions partagees
//! a l'ecran. C'est l'interface dont un `compositord` ring 3 a besoin pour
//! remplacer la boucle de composition du noyau : il ne manipule que des
//! identifiants (capacite, region) et des rectangles, jamais une adresse
//! physique ni une commande du pilote.
//!
//! Securite :
//!   - aucun appel ne cree d'objet `Display` ; le noyau en donne UN, au seul
//!     programme `/bin/composited` qu'il lance lui-meme (`lance_compositeur`) ;
//!   - la region doit etre lue avec le droit READ du processus appelant ;
//!   - les bornes (surface dans la region, rectangle dans la surface,
//!     destination dans l'ecran) sont jugees par `contrat::valide`, pure et
//!     testee sur l'hote, AVANT toute lecture ;
//!   - tant que le bureau NOYAU tient l'ecran, l'appel est refuse
//!     (`WouldBlock`) : deux compositeurs n'ecrivent jamais le meme tampon.
//!
//! Retour : l'empreinte des pixels RELUS a l'ecran apres presentation ; le
//! client la compare a celle qu'il a calculee (preuve de bout en bout).

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use super::abi::types::{Error, HandleId, ObjectKind, Result, Rights};
use super::handle;
use super::object::Object;

#[path = "affichage_contrat.rs"]
pub mod contrat;

static BUREAU_NOYAU: AtomicBool = AtomicBool::new(false);
/// BOUCHAUD_BUREAU_RING3_2A : le bureau de cette machine est le compositeur
/// ring 3 ; le bureau noyau ne doit pas demarrer.
static BUREAU_RING3: AtomicBool = AtomicBool::new(false);

/// L'ecran appartient-il au compositeur ring 3 (bureau configure, ou ecran
/// deja pris) ? Le bureau noyau refuse alors de demarrer.
pub fn ecran_reserve_au_compositeur() -> bool {
    BUREAU_RING3.load(Ordering::Acquire) || ECRAN_AU_COMPOSITEUR.load(Ordering::Acquire)
}
static ECRAN_AU_COMPOSITEUR: AtomicBool = AtomicBool::new(false);

/// Le bureau NOYAU tient-il l'ecran (et donc les entrees) ?
pub fn bureau_noyau_actif() -> bool {
    BUREAU_NOYAU.load(Ordering::Acquire)
}

/// Le compositeur ring 3 a pris l'ecran (premiere presentation acceptee).
pub fn ecran_au_compositeur() -> bool {
    ECRAN_AU_COMPOSITEUR.load(Ordering::Acquire)
}
static PRESENTATIONS: AtomicU64 = AtomicU64::new(0);
static REFUS: AtomicU64 = AtomicU64::new(0);

/// Le fil du bureau noyau demande l'ecran. Rend `false` -- et le bureau noyau
/// ne demarre pas -- si le compositeur ring 3 l'a deja, ou s'il est le bureau
/// configure de cette machine (BOUCHAUD_BUREAU_RING3_2A).
///
/// Exclusion sans verrou, ordre sequentiel : chaque cote PUBLIE sa prise
/// puis LIT celle de l'autre (`present` fait de meme). Au moins l'un des deux
/// voit l'autre ; deux compositeurs n'ecrivent jamais le meme ecran.
pub fn bureau_noyau_demarre() -> bool {
    // A configured ring-3 desktop already owns the reservation. Publishing
    // a speculative claim here can make its live DISPLAY_INFO fail.
    if ecran_reserve_au_compositeur() {
        REFUS.fetch_add(1, Ordering::Relaxed);
        crate::serial_println!("BOUCHAUD_BUREAU_NOYAU_REFUSE raison=ecran-au-compositeur");
        return false;
    }
    BUREAU_NOYAU.store(true, Ordering::SeqCst);
    if BUREAU_RING3.load(Ordering::SeqCst) || ECRAN_AU_COMPOSITEUR.load(Ordering::SeqCst) {
        BUREAU_NOYAU.store(false, Ordering::SeqCst);
        REFUS.fetch_add(1, Ordering::Relaxed);
        crate::serial_println!("BOUCHAUD_BUREAU_NOYAU_REFUSE raison=ecran-au-compositeur");
        return false;
    }
    true
}

/// Seul chemin qui fait naitre une capacite d'affichage.
pub const COMPOSITEUR: &str = "/bin/composited";

/// Droits de la capacite donnee au compositeur : presenter, inspecter, et
/// l'ATTENDRE dans un waitset (BOUCHAUD_ENTREES_RING3_V1 : elle est lisible
/// quand une entree attend). Ni DUP ni TRANSFER : elle ne se multiplie pas et
/// ne se passe pas.
pub const DROITS_COMPOSITEUR: Rights = Rights(Rights::DISPLAY_PRESENT.0 | Rights::INSPECT.0 | Rights::WAIT.0);

/// BOUCHAUD_BUREAU_RING3_2A : configuration du bureau ring 3, posee dans
/// l'image. Premiere ligne : `CHEMIN [VAR=VALEUR ...]` -- le client confie au
/// compositeur et les variables qu'il recoit en plus de celles du shell.
pub const CONFIG_BUREAU: &str = "/bureau-ring3";

/// Zone d'une fenetre d'application par defaut (allocation = zone).
pub const ZONE_FENETRE: (usize, usize) = (640, 400);

/// Au plus quatre applications confiees au compositeur ring 3.
pub const CLIENTS_MAX: usize = 4;

/// Une ligne de `CONFIG_BUREAU` : `CHEMIN [LxH] [differe] [BO_VAR=VALEUR ...]`
/// (variables `BO_*` et `BOUCHAUD_*` seulement).
///
/// BOUCHAUD_BUREAU_UTILISABLE_V1 : `differe` -- l'application n'est PAS lancee
/// au demarrage ; elle figure au dock du compositeur, qui la lance a la
/// demande (`bo_display_relance_client`). Ligne `@dock` : le compositeur
/// affiche un dock (lanceur) et les titres des fenetres.
pub struct ClientBureau {
    pub chemin: alloc::string::String,
    pub zone: (usize, usize),
    pub vars: alloc::vec::Vec<alloc::string::String>,
    pub differe: bool,
}

/// Nom affiche d'une application : le dernier element du chemin, reduit aux
/// caracteres sans danger pour une variable d'environnement et une police
/// minimale (lettres, chiffres, '-', '_'), 20 au plus.
fn nom_application(chemin: &str) -> alloc::string::String {
    let base = chemin.rsplit('/').next().unwrap_or("application");
    // `calculatrice-gui` s'affiche « calculatrice » : le suffixe ne dit rien
    // a qui regarde le dock.
    base.strip_suffix("-gui").unwrap_or(base)
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(20)
        .collect()
}

/// Lit la configuration du bureau ring 3 : les clients, dans l'ordre (le
/// dernier est au premier plan), le scenario eventuel (`@scenario NOM`) et le
/// dock (`@dock`).
fn config_bureau() -> Option<(alloc::vec::Vec<ClientBureau>, Option<alloc::string::String>, bool, bool)> {
    use alloc::string::ToString;
    let texte = {
        let fs = crate::fs::ramfs::fs();
        let idx = fs.resolve(CONFIG_BUREAU, 0)?;
        if fs.nodes[idx].kind != crate::fs::ramfs::NodeKind::File {
            return None;
        }
        fs.nodes[idx].content_str()
    };
    let mut clients = alloc::vec::Vec::new();
    let mut scenario = None;
    let mut dock = false;
    let mut faute = false;
    for ligne in texte.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let mut mots = ligne.split_whitespace();
        let Some(premier) = mots.next() else { continue };
        if premier == "@scenario" {
            // Un mot simple : il devient une variable du compositeur.
            scenario = mots.next()
                .filter(|m| m.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
                .map(|m| m.to_string());
            continue;
        }
        if premier == "@dock" {
            dock = true;
            continue;
        }
        if premier == "@faute-compositeur" {
            // Banc de reprise : le PREMIER compositeur plante sur F12.
            faute = true;
            continue;
        }
        if clients.len() == CLIENTS_MAX {
            crate::serial_println!("BOUCHAUD_BUREAU_RING3_ECHEC raison=trop-de-clients max={}", CLIENTS_MAX);
            break;
        }
        let mut zone = ZONE_FENETRE;
        let mut vars = alloc::vec::Vec::new();
        let mut differe = false;
        for m in mots {
            if m == "differe" {
                differe = true;
                continue;
            }
            if let Some((l, h)) = m.split_once('x') {
                if let (Ok(l), Ok(h)) = (l.parse::<usize>(), h.parse::<usize>()) {
                    if (64..=1920).contains(&l) && (64..=1200).contains(&h) {
                        zone = (l, h);
                    }
                    continue;
                }
            }
            // Seules des affectations de variables `BO_*` / `BOUCHAUD_*`
            // passent : la configuration ne sert pas a injecter un
            // environnement arbitraire (LD_PRELOAD, PATH...).
            if (m.starts_with("BO_") || m.starts_with("BOUCHAUD_")) && m.contains('=') {
                vars.push(m.to_string());
            }
        }
        clients.push(ClientBureau { chemin: premier.to_string(), zone, vars, differe });
    }
    if clients.is_empty() { None } else { Some((clients, scenario, dock, faute)) }
}

/// Lance `/bin/composited` avec la capacite d'affichage, s'il est present.
/// Absent (cas de toutes les images livrees aujourd'hui) : rien ne se passe.
///
/// BOUCHAUD_BUREAU_RING3_2A/2B : si l'image porte `CONFIG_BUREAU`, le noyau
/// lance d'abord les clients qui y sont nommes -- des clients BOGU ordinaires,
/// avec surface et canal, comme sous le bureau noyau, chacun dans son
/// processus, avec son `Configure` initial -- et donne au compositeur, en plus
/// de l'affichage, une capacite `Fenetre` par client (`BO_FENETRES=h1,h2,...`,
/// du fond vers le premier plan ; mode `bureau`). Le bureau noyau, lui, ne
/// demarre plus : l'ecran a un seul compositeur.
/// Allocation d'une surface de client : de quoi l'agrandir jusqu'a l'ecran
/// courant des bancs sans la reallouer (BOUCHAUD_BUREAU_RING3_2B), bornee.
fn allocation_pour(zone: (usize, usize)) -> (usize, usize) {
    (zone.0.max(1280).min(1920), zone.1.max(720).min(1200))
}

/// Les clients de la configuration, gardes pour `relance` : le compositeur ne
/// peut relancer QUE ce que l'image a configure.
static CLIENTS_CONFIGURES: crate::kernel::sync::SpinLock<alloc::vec::Vec<ClientBureau>> =
    crate::kernel::sync::SpinLock::new(alloc::vec::Vec::new());
/// Relances par client, bornees : une application qui meurt en boucle ne
/// fait pas lancer des processus sans fin.
pub const RELANCES_MAX: u32 = 8;
static RELANCES: [core::sync::atomic::AtomicU32; CLIENTS_MAX] =
    [const { core::sync::atomic::AtomicU32::new(0) }; CLIENTS_MAX];

/// Lance UN client configure ; le foyer lui est donne si `focus`.
fn lance_client(c: &ClientBureau, rang: usize, focus: bool) -> Option<super::fenetre::Fenetre> {
    let titre = c.chemin.rsplit('/').next().unwrap_or("application");
    match crate::gui::client::Client::lance_avec(&c.chemin, titre, 0, c.zone, allocation_pour(c.zone), &c.vars) {
        Ok(mut client) => {
            // Comme le bureau noyau : la zone utile et le foyer, des le depart.
            client.envoie_configuration(focus);
            if c.chemin == crate::gui::client::CHEMIN_NAVIGATEUR {
                crate::gui::services::enregistre(client.pid);
            }
            crate::serial_println!(
                "BOUCHAUD_BUREAU_RING3_CLIENT pid={} chemin={} zone={}x{} variables={} rang={}",
                client.pid, c.chemin, c.zone.0, c.zone.1, c.vars.len(), rang
            );
            Some(super::fenetre::Fenetre::neuve(client))
        }
        Err(e) => {
            crate::serial_println!("BOUCHAUD_BUREAU_RING3_ECHEC raison=client chemin={} detail={}", c.chemin, e);
            None
        }
    }
}

/// `bo_display_relance_client(affichage, rang)` : relance le client configure
/// de ce rang et rend une NOUVELLE capacite `Fenetre` (BOUCHAUD_BUREAU_RING3_2B).
/// Exige la capacite d'affichage ; ne lance jamais que ce que l'image a
/// configure (`CONFIG_BUREAU`) ; au plus `RELANCES_MAX` fois par client.
pub fn relance(args: [u64; 6]) -> Result<u64> {
    handle::current_table()
        .lookup_kind(HandleId::from_raw(args[0]), Rights::DISPLAY_PRESENT, ObjectKind::Display)
        .map_err(|e| refuse(e, "capacite"))?;
    let rang = usize::try_from(args[1]).map_err(|_| Error::InvalidArgument)?;
    let client = {
        let clients = CLIENTS_CONFIGURES.lock();
        let c = clients.get(rang).ok_or_else(|| refuse(Error::InvalidArgument, "relance-rang"))?;
        ClientBureau { chemin: c.chemin.clone(), zone: c.zone, vars: c.vars.clone(), differe: c.differe }
    };
    if RELANCES[rang].fetch_add(1, Ordering::AcqRel) >= RELANCES_MAX {
        RELANCES[rang].fetch_sub(1, Ordering::AcqRel);
        return Err(refuse(Error::NoSpace, "relances-epuisees"));
    }
    let fenetre = lance_client(&client, rang, true).ok_or(Error::NotFound)?;
    let h = handle::install(Object::Fenetre(fenetre), super::fenetre::DROITS_COMPOSITEUR)?;
    crate::serial_println!("BOUCHAUD_BUREAU_RING3_RELANCE rang={} n={}", rang, RELANCES[rang].load(Ordering::Relaxed));
    Ok(h.raw())
}

pub fn lance_compositeur() -> Option<u32> {
    use alloc::string::{String, ToString};
    crate::fs::ramfs::fs().resolve(COMPOSITEUR, 0)?;
    let mut fenetres: alloc::vec::Vec<super::fenetre::Fenetre> = alloc::vec::Vec::new();
    let mut rangs: alloc::vec::Vec<usize> = alloc::vec::Vec::new();
    let mut scenario = None;
    let mut dock = false;
    let mut noms: alloc::vec::Vec<String> = alloc::vec::Vec::new();
    let mut faute = false;
    if let Some((clients, sc, dk, ft)) = config_bureau() {
        scenario = sc;
        dock = dk;
        // La faute volontaire n'est donnee qu'au premier compositeur : celui
        // qui le remplace doit vivre.
        faute = ft && REPRISES.load(Ordering::Acquire) == 0;
        // Le dernier client LANCE au demarrage a le foyer.
        let dernier = clients.iter().rposition(|c| !c.differe);
        for (i, c) in clients.iter().enumerate() {
            noms.push(nom_application(&c.chemin));
            if c.differe {
                crate::serial_println!("BOUCHAUD_BUREAU_RING3_DIFFERE chemin={} rang={}", c.chemin, i);
                continue;
            }
            if let Some(f) = lance_client(c, i, Some(i) == dernier) {
                fenetres.push(f);
                rangs.push(i);
            }
        }
        *CLIENTS_CONFIGURES.lock() = clients;
    }
    // Avec un dock, le bureau ring 3 vaut meme sans fenetre au demarrage :
    // les applications s'ouvrent depuis le dock.
    let bureau = !fenetres.is_empty() || (dock && !noms.is_empty());
    let mode = if bureau { "bureau" } else { "presentation" };
    if bureau {
        BUREAU_RING3.store(true, Ordering::Release);
    }
    let argv = [COMPOSITEUR.to_string(), mode.to_string()];
    let env = [String::from("PATH=/bin:/usr/bin")];
    let mut fenetres = Some(fenetres);
    let mut prepare = |process: &crate::kernel::task::Process| -> alloc::vec::Vec<String> {
        let mut vars = alloc::vec::Vec::new();
        if let Ok(h) = handle::install_for(process.pid, Object::Display, DROITS_COMPOSITEUR) {
            vars.push(alloc::format!("BO_DISPLAY={}", h.raw()));
        }
        let mut poignees = alloc::vec::Vec::new();
        for f in fenetres.take().unwrap_or_default() {
            if let Ok(h) = handle::install_for(process.pid, Object::Fenetre(f), super::fenetre::DROITS_COMPOSITEUR) {
                poignees.push(alloc::format!("{}", h.raw()));
            }
        }
        if !poignees.is_empty() {
            vars.push(alloc::format!("BO_FENETRES={}", poignees.join(",")));
        }
        if dock {
            // Rang de configuration de chaque fenetre lancee, noms de TOUTES
            // les applications configurees (le dock les montre par rang).
            vars.push(alloc::format!("BO_RANGS={}",
                rangs.iter().map(|r| alloc::format!("{}", r)).collect::<alloc::vec::Vec<_>>().join(",")));
            vars.push(alloc::format!("BO_APPS={}", noms.join(",")));
            vars.push(String::from("BO_BUREAU_DOCK=1"));
        }
        if faute {
            vars.push(String::from("BO_COMPOSITED_FAUTE_F12=1"));
        }
        if let Some(sc) = scenario.as_ref() {
            vars.push(alloc::format!("BO_BUREAU_SCENARIO={}", sc));
        }
        vars
    };
    match crate::kernel::exec::lance_detache(COMPOSITEUR, &argv, &env, 0, &mut prepare) {
        Ok(pid) => {
            crate::serial_println!(
                "BOUCHAUD_COMPOSITEUR_LANCE pid={} capacite=affichage droits={:#x}",
                pid, DROITS_COMPOSITEUR.0
            );
            COMPOSITEUR_PID.store(pid, Ordering::Release);
            if bureau && !SURVEILLANCE.swap(true, Ordering::AcqRel) {
                crate::kernel::task::spawn_noyau(surveille_compositeur, "compositeur-reprise");
            }
            Some(pid)
        }
        Err(e) => {
            // Revue de la PR #295 : sans compositeur, l'ecran n'est reserve a
            // personne -- le bureau noyau doit pouvoir le reprendre. Le client
            // deja lance meurt avec sa `Fenetre`, rendue ici.
            BUREAU_RING3.store(false, Ordering::SeqCst);
            drop(fenetres.take());
            crate::serial_println!("BOUCHAUD_COMPOSITEUR_ECHEC raison={} reservation_ecran=levee", e);
            None
        }
    }
}

// BOUCHAUD_REPRISE_COMPOSITEUR_V1 -- le compositeur ring 3 est un processus
// comme un autre : il peut mourir. Ses capacites `Fenetre` meurent avec lui
// (leurs clients sont termines), l'ecran reste reserve au bureau ring 3. Un
// fil noyau le surveille et le RELANCE, avec les applications non differees
// et une capacite d'affichage neuve, au plus `REPRISES_MAX` fois : un
// compositeur qui meurt en boucle ne relance pas des processus sans fin.
pub const REPRISES_MAX: u32 = 3;
static REPRISES: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static COMPOSITEUR_PID: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static SURVEILLANCE: AtomicBool = AtomicBool::new(false);

fn surveille_compositeur() -> ! {
    loop {
        crate::kernel::task::sleep_ticks(250);
        let pid = COMPOSITEUR_PID.load(Ordering::Acquire);
        if pid == 0 || crate::kernel::task::processus_actif(pid) {
            continue;
        }
        let n = REPRISES.load(Ordering::Acquire);
        if n >= REPRISES_MAX {
            crate::serial_println!("BOUCHAUD_COMPOSITEUR_REPRISE_EPUISEE ancien_pid={} reprises={}", pid, n);
            COMPOSITEUR_PID.store(0, Ordering::Release);
            continue;
        }
        REPRISES.store(n + 1, Ordering::Release);
        crate::serial_println!("BOUCHAUD_COMPOSITEUR_MORT pid={} code={:?} reprise={}",
            pid, crate::kernel::task::code_de_sortie(pid), n + 1);
        match lance_compositeur() {
            Some(nouveau) => crate::serial_println!(
                "BOUCHAUD_COMPOSITEUR_REPRISE n={} ancien_pid={} nouveau_pid={}", n + 1, pid, nouveau),
            None => {
                crate::serial_println!("BOUCHAUD_COMPOSITEUR_REPRISE_ECHEC n={} ancien_pid={}", n + 1, pid);
                COMPOSITEUR_PID.store(0, Ordering::Release);
            }
        }
    }
}

fn refuse(e: Error, raison: &str) -> Error {
    REFUS.fetch_add(1, Ordering::Relaxed);
    crate::serial_println!("BOUCHAUD_DISPLAY_PRESENT_REFUS raison={}", raison);
    e
}

/// `bo_display_info(affichage)` : (largeur << 32) | hauteur de l'ecran.
pub fn info(args: [u64; 6]) -> Result<u64> {
    handle::current_table()
        .lookup_kind(HandleId::from_raw(args[0]), Rights::DISPLAY_PRESENT, ObjectKind::Display)
        .map_err(|e| refuse(e, "capacite"))?;
    if BUREAU_NOYAU.load(Ordering::Acquire) {
        return Err(refuse(Error::WouldBlock, "bureau-noyau"));
    }
    let deja = crate::drivers::gfx::is_active();
    crate::drivers::gfx::entre_si_inactif();
    if !crate::drivers::gfx::is_active() {
        return Err(Error::NotFound);
    }
    let (l, h) = (crate::drivers::gfx::width() as u64, crate::drivers::gfx::height() as u64);
    if !deja && !ECRAN_AU_COMPOSITEUR.load(Ordering::Acquire) {
        crate::drivers::gfx::leave();
    }
    Ok((l << 32) | h)
}

const JOURNAL_COMPLET: u64 = 64;
const JOURNAL_PAS: u64 = 256;
static JOURNALISEES: AtomicU64 = AtomicU64::new(0);
static CUMUL_POSE_NS: AtomicU64 = AtomicU64::new(0);
static CUMUL_TRANSFERT_NS: AtomicU64 = AtomicU64::new(0);
static CUMUL_RELECTURE_NS: AtomicU64 = AtomicU64::new(0);
static CUMUL_JOURNAL_NS: AtomicU64 = AtomicU64::new(0);

pub fn present(args: [u64; 6]) -> Result<u64> {
    let table = handle::current_table();
    // 1. La capacite : bon type ET bon droit, sinon rien n'est lu.
    table.lookup_kind(HandleId::from_raw(args[0]), Rights::DISPLAY_PRESENT, ObjectKind::Display)
        .map_err(|e| refuse(e, "capacite"))?;
    // 2. La region, lue avec les droits de l'appelant.
    let region = table.lookup_kind(HandleId::from_raw(args[1]), Rights::READ, ObjectKind::SharedRegion)
        .map_err(|e| refuse(e, "region"))?;
    let region = match region.object.as_ref() {
        Object::SharedRegion(r) => r,
        _ => return Err(Error::WrongType),
    };
    // 3. La demande.
    let octets = super::abi::usercopy::read(args[2], contrat::DEMANDE_OCTETS)?;
    let mut fil = [0u8; contrat::DEMANDE_OCTETS];
    fil.copy_from_slice(&octets);
    let d = contrat::Demande::depuis_octets(&fil);
    // 4. L'ecran : refuse tant que le bureau noyau le tient.
    if BUREAU_NOYAU.load(Ordering::Acquire) {
        return Err(refuse(Error::WouldBlock, "bureau-noyau"));
    }
    let deja_graphique = crate::drivers::gfx::is_active();
    crate::drivers::gfx::entre_si_inactif();
    if !crate::drivers::gfx::is_active() {
        return Err(refuse(Error::NotFound, "pas-d-ecran"));
    }
    // Revue de la PR #286 : l'ecran n'est au compositeur qu'une fois la
    // demande ACCEPTEE ; un refus rend le mode texte si cet appel l'avait quitte.
    let rend = |e| {
        if !deja_graphique && !ECRAN_AU_COMPOSITEUR.load(Ordering::Acquire) {
            crate::drivers::gfx::leave();
        }
        e
    };
    let (el, eh) = (crate::drivers::gfx::width() as u32, crate::drivers::gfx::height() as u32);
    contrat::valide(&d, region.len(), el, eh).map_err(|r| {
        let raison = match r {
            contrat::Refus::Vide => "vide",
            contrat::Refus::TropGrand => "trop-grand",
            contrat::Refus::SurfaceHorsRegion => "surface-hors-region",
            contrat::Refus::HorsSurface => "hors-surface",
            contrat::Refus::HorsEcran => "hors-ecran",
        };
        rend(refuse(Error::InvalidArgument, raison))
    })?;
    // Prise de l'ecran, puis relecture de celle du bureau noyau (voir
    // `bureau_noyau_demarre`) : si les deux se croisent, l'un renonce.
    ECRAN_AU_COMPOSITEUR.store(true, Ordering::SeqCst);
    if BUREAU_NOYAU.load(Ordering::SeqCst) {
        return Err(refuse(Error::WouldBlock, "bureau-noyau"));
    }
    // 5. Pose, ligne a ligne, puis presentation du seul rectangle.
    let t_pose = crate::kernel::timer::monotonic_ns();
    // Le verrou de la region est pris UNE fois, sans allocation par ligne
    // (avant : un `read` -> Vec par ligne). `contrat::valide` a deja borne
    // chaque ligne dans la region ; le `get` garde un refus plutot qu'une
    // panique si la region changeait de taille (elle ne le fait pas).
    let ligne_octets = d.l as usize * 4;
    region.avec_octets(|octets| -> Result<()> {
        for r in 0..d.h {
            let o = d.decalage_ligne(r);
            let ligne = octets.get(o..o + ligne_octets).ok_or(Error::InvalidArgument)?;
            let dst = crate::drivers::gfx::ligne_mut(d.dest_x as usize, (d.dest_y + r) as usize, d.l as usize);
            for (px, src) in dst.iter_mut().zip(ligne.chunks_exact(4)) {
                *px = u32::from_le_bytes([src[0], src[1], src[2], src[3]]) & 0x00ff_ffff;
            }
        }
        Ok(())
    })?;
    let t_flush = crate::kernel::timer::monotonic_ns();
    // Presentation JUGEE : un refus du peripherique (transfert ou flush virtio,
    // LFB absent...) est rendu au client, jamais une empreinte du tampon arriere.
    if let Err(raison) = crate::drivers::gfx::present_rect_verifie(
        d.dest_x as usize, d.dest_y as usize, d.l as usize, d.h as usize) {
        return Err(refuse(Error::DeviceError, raison));
    }
    let t_relecture = crate::kernel::timer::monotonic_ns();
    // 6. Relecture du tampon AFFICHE (pas du tampon arriere) : l'empreinte
    // rendue au client est celle de ce que l'ecran montre.
    // Ligne par ligne (`ligne_affichee`), dans le meme ordre de pixels que
    // l'empreinte l'a toujours ete : la valeur rendue ne change pas.
    let mut ligne = alloc::vec![0u32; d.l as usize];
    let mut h = contrat::EMPREINTE_DEPART;
    for r in 0..d.h {
        if !crate::drivers::gfx::ligne_affichee(d.dest_x as usize, (d.dest_y + r) as usize, &mut ligne) {
            return Err(refuse(Error::DeviceError, "relecture"));
        }
        h = ligne.iter().fold(h, |h, &p| contrat::empreinte_pixel(h, p));
    }
    let e = contrat::empreinte_fin(h);
    let t_fin = crate::kernel::timer::monotonic_ns();
    let n = PRESENTATIONS.fetch_add(1, Ordering::Relaxed) + 1;
    // Les trois phases, chronometrees : pose (region -> tampon arriere),
    // transfert (tampon arriere -> ecran), relecture (empreinte).
    let (pose, transfert, relecture) = (t_flush - t_pose, t_relecture - t_flush, t_fin - t_relecture);
    CUMUL_POSE_NS.fetch_add(pose, Ordering::Relaxed);
    CUMUL_TRANSFERT_NS.fetch_add(transfert, Ordering::Relaxed);
    CUMUL_RELECTURE_NS.fetch_add(relecture, Ordering::Relaxed);
    // BOUCHAUD_JOURNAL_PRESENT_ECHANTILLONNE_V1. Mesure (banc des entrees,
    // TCG, 3192 presentations) : la ligne serie coutait ~105 us, plus que tout
    // le travail d'une petite presentation (85 us : pose + transfert +
    // relecture) -- 98 % des presentations du bureau. Les JOURNAL_COMPLET
    // premieres restent toutes journalisees (sondes courtes, demarrage), puis
    // une sur JOURNAL_PAS, qui porte les CUMULS de toutes : rien ne disparait
    // du bilan. Les refus gardent leur ligne propre (`refuse`).
    if n <= JOURNAL_COMPLET || n % JOURNAL_PAS == 0 {
        crate::serial_println!(
            "BOUCHAUD_DISPLAY_PRESENT n={} rect={}x{}@{},{} ecran={}x{} empreinte={:#x} refus={} pose_us={} transfert_us={} relecture_us={} \
             cumul_pose_us={} cumul_transfert_us={} cumul_relecture_us={} cumul_journal_us={} journalisees={}",
            n, d.l, d.h, d.dest_x, d.dest_y, el, eh, e, REFUS.load(Ordering::Relaxed),
            pose / 1000, transfert / 1000, relecture / 1000,
            CUMUL_POSE_NS.load(Ordering::Relaxed) / 1000, CUMUL_TRANSFERT_NS.load(Ordering::Relaxed) / 1000,
            CUMUL_RELECTURE_NS.load(Ordering::Relaxed) / 1000, CUMUL_JOURNAL_NS.load(Ordering::Relaxed) / 1000,
            JOURNALISEES.load(Ordering::Relaxed) + 1
        );
        JOURNALISEES.fetch_add(1, Ordering::Relaxed);
        CUMUL_JOURNAL_NS.fetch_add(crate::kernel::timer::monotonic_ns() - t_fin, Ordering::Relaxed);
    }
    Ok(e)
}
