//! BOUCHAUD_B3 : une reparation invoquee n'est pas une reception restauree.
//!
//! # Le compteur qui ne prouvait rien
//!
//! Le releve physique Trigkey (`d131f2a`) se lit ainsi :
//!
//! ```text
//! rx_stall=39  repair_req=39  repair_exec=39  recoveries=39
//! recovery_failures=0  rx_ok_without_progress=54
//! ```
//!
//! Trente-neuf reparations, aucun echec. Sauf que, dans le code, `recoveries`
//! compte les EXECUTIONS de `repare_reception`, et `recovery_failures` ne
//! monte que si la reprogrammation materielle de la puce echoue. Ces deux
//! nombres sont donc parfaitement compatibles avec une reception morte du
//! debut a la fin du releve.
//!
//! Un verdict de progression existait bien dans le pilote -- fenetre bornee,
//! comparaison des compteurs -- mais il n'alimentait qu'une SERIE d'echecs
//! remise a zero au premier succes, et cette serie n'etait exportee nulle
//! part. Il n'existait aucun compteur de « reparation efficace ».
//!
//! # Ce que ce module ajoute
//!
//! Deux lignes encadrantes, et deux compteurs qui, eux, disent le resultat :
//!
//! ```text
//! RX_RECOVERY_BEGIN t_ns= cause= driver= attempt= xid= rx_packets_before= ...
//! RX_RECOVERY_END   t_ns= duration_us= result=effective|ineffective ...
//! ```
//!
//! Le verdict lui-meme vit dans `anneau_rx::verdict_recuperation`, qui ne
//! depend d'aucun materiel et se prouve en test hote. Ici il n'y a que la
//! mecanique : photographier avant, attendre, photographier apres, conclure.
//!
//! # Pourquoi le verdict est DIFFERE
//!
//! Aucune trame ne peut arriver dans les microsecondes qui suivent un
//! rearmement d'anneau. Comparer les compteurs juste avant et juste apres
//! l'appel declarerait toute reparation inefficace, et l'escalade se ferait
//! alors au NOMBRE de tentatives plutot qu'a leur echec -- c'est exactement le
//! defaut que le pilote RTL8168 documente deja dans `repare_si_demande`. On
//! laisse donc au moteur le temps de montrer qu'il est reparti.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use crate::drivers::anneau_rx as anneau;

/// Le temps laisse au materiel pour montrer qu'il est reparti.
pub const FENETRE_VERDICT_NS: u64 = 1_500_000_000;

static EFFECTIVES: AtomicU64 = AtomicU64::new(0);
static INEFFECTIVES: AtomicU64 = AtomicU64::new(0);

static EN_COURS: AtomicBool = AtomicBool::new(false);
static DEBUT_NS: AtomicU64 = AtomicU64::new(0);
static TENTATIVE: AtomicU32 = AtomicU32::new(0);
static XID: AtomicU32 = AtomicU32::new(0);

// L'instantane d'avant, eclate en atomiques : ce module est traverse par le
// drainage verrouille et par le veilleur, et un `SpinLock` ici prendrait un
// verrou sur un chemin qui en tient deja un.
static AV_PAQUETS: AtomicU64 = AtomicU64::new(0);
static AV_CUR: AtomicU64 = AtomicU64::new(0);
static AV_TETE: AtomicU64 = AtomicU64::new(0);
static AV_DESC_CPU: AtomicU64 = AtomicU64::new(0);
static AV_OWN: AtomicU64 = AtomicU64::new(0);
static AV_ISR: AtomicU64 = AtomicU64::new(0);
static AV_SANS_PROGRES: AtomicU64 = AtomicU64::new(0);

fn range(i: &anneau::InstantaneRx) {
    AV_PAQUETS.store(i.rx_paquets, Ordering::Relaxed);
    AV_CUR.store(i.rx_cur as u64, Ordering::Relaxed);
    AV_TETE.store(i.rx_tete_materiel as u64, Ordering::Relaxed);
    AV_DESC_CPU.store(i.dernier_desc_cpu as u64, Ordering::Relaxed);
    AV_OWN.store(i.own_rendus, Ordering::Relaxed);
    AV_ISR.store(i.isr_rx_ok, Ordering::Relaxed);
    AV_SANS_PROGRES.store(i.rx_ok_sans_progres, Ordering::Relaxed);
}

fn reprend() -> anneau::InstantaneRx {
    anneau::InstantaneRx {
        rx_paquets: AV_PAQUETS.load(Ordering::Relaxed),
        rx_cur: AV_CUR.load(Ordering::Relaxed) as usize,
        rx_tete_materiel: AV_TETE.load(Ordering::Relaxed) as u32,
        dernier_desc_cpu: AV_DESC_CPU.load(Ordering::Relaxed) as usize,
        own_rendus: AV_OWN.load(Ordering::Relaxed),
        isr_rx_ok: AV_ISR.load(Ordering::Relaxed),
        rx_ok_sans_progres: AV_SANS_PROGRES.load(Ordering::Relaxed),
    }
}

/// Nombre de reparations dont la reception a REELLEMENT repris, et des autres.
pub fn compteurs() -> (u64, u64) {
    (
        EFFECTIVES.load(Ordering::Relaxed),
        INEFFECTIVES.load(Ordering::Relaxed),
    )
}

/// Une reparation vient d'etre executee : photographie et arme le verdict.
///
/// `repair_req` / `repair_exec` sont ceux du pilote, passes tels quels : la
/// ligne doit permettre de recoller ce module aux compteurs de la boite noire
/// sans supposer qu'ils coincident.
pub fn debut(
    cause: &str,
    pilote: &str,
    tentative: u32,
    xid: u32,
    avant: &anneau::InstantaneRx,
    repair_req: u64,
    repair_exec: u64,
) {
    // Une reparation qui en chevauche une autre n'est pas jugeable : le
    // second rearmement changerait l'anneau pendant la fenetre du premier. On
    // conclut celle qui court -- avec ce qu'on sait -- avant d'ouvrir celle-ci.
    if EN_COURS.load(Ordering::Relaxed) {
        conclure(avant, true);
    }
    let t = crate::kernel::timer::monotonic_ns();
    range(avant);
    DEBUT_NS.store(t, Ordering::Relaxed);
    TENTATIVE.store(tentative, Ordering::Relaxed);
    XID.store(xid, Ordering::Relaxed);
    EN_COURS.store(true, Ordering::Release);
    crate::kernel::dmesg::log_fmt(format_args!(
        "RX_RECOVERY_BEGIN t_ns={} cause={} driver={} attempt={} xid={:#010x} \
rx_packets_before={} rx_cur_before={} rx_hw_head_before={} rx_last_desc_cpu={} \
own_before={} isr_rx_ok_before={} rx_ok_without_progress_before={} \
repair_req={} repair_exec={}",
        t, cause, pilote, tentative, xid,
        avant.rx_paquets, avant.rx_cur, avant.rx_tete_materiel,
        avant.dernier_desc_cpu, avant.own_rendus, avant.isr_rx_ok,
        avant.rx_ok_sans_progres, repair_req, repair_exec,
    ));
}

/// Si une fenetre est ouverte ET ecoulee, rend le verdict.
///
/// Appelee depuis le drainage verrouille, qui repasse regulierement. Rendre
/// le verdict plus tot serait le rendre faux ; ne jamais le rendre serait
/// pire encore.
pub fn conclure_si_du(apres: &anneau::InstantaneRx) {
    if !EN_COURS.load(Ordering::Acquire) {
        return;
    }
    let debut = DEBUT_NS.load(Ordering::Relaxed);
    if crate::kernel::timer::monotonic_ns().saturating_sub(debut) < FENETRE_VERDICT_NS {
        return;
    }
    conclure(apres, false);
}

fn conclure(apres: &anneau::InstantaneRx, ecourte: bool) {
    if !EN_COURS.swap(false, Ordering::AcqRel) {
        return;
    }
    let t = crate::kernel::timer::monotonic_ns();
    let debut = DEBUT_NS.load(Ordering::Relaxed);
    let avant = reprend();
    let v = anneau::verdict_recuperation(&avant, apres);
    if v.effective {
        EFFECTIVES.fetch_add(1, Ordering::Relaxed);
    } else {
        INEFFECTIVES.fetch_add(1, Ordering::Relaxed);
    }
    crate::kernel::dmesg::log_fmt(format_args!(
        "RX_RECOVERY_END t_ns={} duration_us={} result={} raison={} ecourte={} \
attempt={} xid={:#010x} rx_packets_after={} rx_cur_after={} rx_hw_head_after={} \
rx_last_desc_cpu_after={} own_after={} isr_rx_ok_after={} \
rx_ok_without_progress_after={} effectives={} ineffectives={}",
        t,
        t.saturating_sub(debut) / 1_000,
        if v.effective { "effective" } else { "ineffective" },
        v.raison,
        ecourte as u8,
        TENTATIVE.load(Ordering::Relaxed),
        XID.load(Ordering::Relaxed),
        apres.rx_paquets, apres.rx_cur, apres.rx_tete_materiel,
        apres.dernier_desc_cpu, apres.own_rendus, apres.isr_rx_ok,
        apres.rx_ok_sans_progres,
        EFFECTIVES.load(Ordering::Relaxed),
        INEFFECTIVES.load(Ordering::Relaxed),
    ));
}

