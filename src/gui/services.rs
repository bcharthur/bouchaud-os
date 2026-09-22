//! Session Ladybird supervisee par le bureau. Les moteurs gardent leurs IPC natifs.
use core::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};
pub const DEMARRER: u8 = 1;
pub const ARRETER: u8 = 2;
static COMMANDE: AtomicU8 = AtomicU8::new(0);
static RACINE: AtomicU32 = AtomicU32::new(0);
static ECHEC: AtomicU8 = AtomicU8::new(0);
pub fn demande(action: u8) {
    COMMANDE.store(action, Ordering::Release);
    crate::kernel::sync::reveil::INTERFACE.signale(crate::kernel::sync::reveil::Source::Explicite);
}
pub fn prend_commande() -> u8 { COMMANDE.swap(0, Ordering::AcqRel) }
pub fn enregistre(pid: u32) { RACINE.store(pid, Ordering::Release); ECHEC.store(0, Ordering::Release); }
pub fn echec() { ECHEC.store(1, Ordering::Release); }
pub fn racine() -> u32 { RACINE.load(Ordering::Acquire) }
pub fn en_echec() -> bool { ECHEC.load(Ordering::Acquire) != 0 }
/// Arret borne de l'arbre, y compris les moteurs sans fenetre.
pub fn arrete() {
    let pid = RACINE.swap(0, Ordering::AcqRel);
    if pid != 0 {
        for child in crate::kernel::task::arbre_de(pid) {
            crate::kernel::task::tue_processus(child, 0);
        }
    }
}

// Reuse the existing five-second process sample, no additional MM scans on paint.
static CPU: AtomicU64 = AtomicU64::new(0);
static RSS: AtomicU64 = AtomicU64::new(0);
static SAMPLE_MS: AtomicU64 = AtomicU64::new(0);
/// Le nom de processus Ladybird correspondant a chaque service du registre.
///
/// # Ce tableau ne definit PLUS le contenu de la fenetre
///
/// Il le faisait : la vue Services parcourait cette liste et n'affichait
/// qu'elle -- six lignes, et rien du systeme ni du reseau. Il ne sert plus
/// qu'a une chose, la bonne : traduire un nom de processus en identifiant de
/// service, pour que les mesures CPU et RSS aillent alimenter le registre.
/// C'est le registre, et lui seul, qui decide ce que la fenetre montre.
const PROCESSUS_VERS_SERVICE: [(&str, &str); 6] = [
    ("BouchaudBrowserHost", "browser.host"),
    ("RequestServer", "browser.request_server"),
    ("WebContent", "browser.web_content"),
    ("ImageDecoder", "browser.image_decoder"),
    ("WebWorker", "browser.web_worker"),
    ("Compositor", "browser.compositor"),
];

pub fn observe(rows: &[crate::kernel::task::Mesure], window: u64) {
    use crate::kernel::services::{self, registre::Kpi, Etat};
    let root = racine();
    let mut ticks = 0u64;
    let mut rss = 0u64;
    for row in rows {
        if root != 0 && row.resource_group_id == root {
            ticks = ticks.saturating_add(row.ticks);
            rss = rss.saturating_add(row.rss_octets);
        }
    }
    CPU.store(if window == 0 { 0 } else { ticks.saturating_mul(100)/window }, Ordering::Relaxed);
    RSS.store(rss, Ordering::Relaxed);
    SAMPLE_MS.store(crate::kernel::timer::monotonic_ms(), Ordering::Release);

    // LES MESURES REJOIGNENT LE REGISTRE, au lieu de rester ici.
    //
    // Le releve existait deja -- cinq secondes, une seule passe sur les
    // processus. Il alimentait une fenetre qui ne montrait que Ladybird ; il
    // alimente maintenant les noeuds `browser.*` du registre, que la fenetre,
    // la ligne de commande et la boite noire lisent toutes les trois.
    for (nom, service) in PROCESSUS_VERS_SERVICE {
        // BOUCHAUD_C24_PLUSIEURS_INSTANCES
        //
        // La boucle s'arretait au PREMIER processus trouve (`break`). Le
        // portage lance un WebContent PAR ONGLET, et des WebWorker a la
        // demande : sur trois onglets, Services montrait le CPU et la memoire
        // d'un seul des trois, avec le PID d'un seul des trois. Ce n'est pas
        // une approximation, c'est un chiffre faux -- on croit regarder le
        // processus qui consomme alors qu'on en regarde un de ses freres, et
        // on cherche la lenteur dans le mauvais.
        //
        // Les instances sont donc CUMULEES, et leur nombre est publie. Le
        // `pid` ne reste rempli que s'il n'y en a qu'une : au-dela, un PID
        // unique designerait arbitrairement l'un d'eux.
        let mut instances = 0u32;
        let mut ticks = 0u64;
        let mut rss = 0u64;
        let mut vss = 0u64;
        let mut pid_unique = 0u32;
        let mut fautes = crate::kernel::fautes::Compte::default();
        for row in rows {
            if root != 0 && row.resource_group_id != root && row.pid != root {
                continue;
            }
            let base = row.nom.rsplit('/').next().unwrap_or(&row.nom);
            let correspond = base == nom || (nom == "BouchaudBrowserHost" && row.pid == root);
            if !correspond {
                continue;
            }
            instances += 1;
            pid_unique = row.pid;
            ticks = ticks.saturating_add(row.ticks);
            rss = rss.saturating_add(row.rss_octets);
            vss = vss.saturating_add(row.vss_octets);
            if let Some(compte) = crate::kernel::task::fautes_du_processus(row.pid) {
                fautes.fusionne(&compte);
            }
        }
        if instances > 0 {
            services::kpi(
                service,
                Kpi {
                    cpu_pour_mille: Some(if window == 0 {
                        0
                    } else {
                        (ticks.saturating_mul(1000) / window) as u32
                    }),
                    rss_octets: Some(rss),
                    vss_octets: Some(vss),
                    pid: if instances == 1 { Some(pid_unique) } else { None },
                    instances: Some(instances),
                    // `None` et non `Some(0)` quand rien n'a ete mesure : une
                    // colonne a zero affirme « ce processus ne faute pas »,
                    // alors que le livre peut simplement ne rien savoir de lui
                    // -- il est borne et il chasse.
                    fautes_nombre: if fautes.vide() { None } else { Some(fautes.nombre) },
                    fautes_total_us: if fautes.vide() { None } else { Some(fautes.total_ns / 1_000) },
                    fautes_pire_us: if fautes.vide() { None } else { Some(fautes.pire_ns / 1_000) },
                    ..Kpi::default()
                },
            );
            services::etat(service, Etat::Actif);
        }
        if instances == 0 {
            // ARRETE, ET TOUJOURS DANS L'ARBRE. Un `WebWorker` a la demande
            // doit se lire « arrete », pas disparaitre.
            services::kpi(service, Kpi::default());
            services::etat(service, Etat::Arrete);
        }
    }
}
pub fn usage() -> (u64, u64, u64) {
    let sample = SAMPLE_MS.load(Ordering::Acquire);
    (CPU.load(Ordering::Relaxed), RSS.load(Ordering::Relaxed), sample)
}
