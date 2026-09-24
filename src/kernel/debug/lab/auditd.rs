//! `bouchaud-auditd` : le fil qui regarde, et qui n'a besoin de personne.
//!
//! # Ce qu'il doit survivre
//!
//! Le releve TRIGKEY a vu disparaitre, dans la meme session, les trois canaux
//! d'enquete : COM1 n'existe pas (`serial_bytes=0`), la reception meurt apres
//! soixante-quatre trames, la boite noire s'arrete deux cents secondes avant
//! la fin. Un auditeur qui dependrait de l'un des trois n'aurait rien vu.
//!
//! Celui-ci n'ecrit que dans l'anneau LAB -- de la RAM, statique, sans
//! allocation. Il fonctionne sans interface graphique, sans Ladybird, sans
//! COM1, sans reseau et sans debugger distant. Les canaux viennent y lire
//! quand ils existent.
//!
//! # La cadence, et pourquoi elle est bornee des deux cotes
//!
//! Un hertz en croisiere : assez pour dater une panne a la seconde, assez peu
//! pour ne rien couter a l'ordonnanceur qu'on mesure par ailleurs. Dix hertz
//! autour d'une anomalie, pendant cinq secondes seulement -- la panne
//! RTL8168 dure cent cinquante et une secondes, et l'accompagner a dix hertz
//! tout ce temps fausserait la mesure qu'on essaie de prendre.
//!
//! # Ce fil ne repare rien
//!
//! Il observe et il capture. La reprise appartient au pilote, sous son verrou
//! de reception ; y toucher d'ici rouvrirait exactement la course que ce
//! verrou existe pour fermer.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use super::auditeur::{self, Auditeur, Observation, Verdict};
use super::{id, Categorie};

static LANCE: AtomicBool = AtomicBool::new(false);
static TOURS: AtomicU64 = AtomicU64::new(0);
static VERDICTS: AtomicU64 = AtomicU64::new(0);
static CAPTURES: AtomicU64 = AtomicU64::new(0);
static CADENCE_HZ: AtomicU64 = AtomicU64::new(auditeur::CADENCE_NORMALE_HZ as u64);
static DERNIER_VERDICT: AtomicU64 = AtomicU64::new(0);
static DERNIER_VERDICT_NS: AtomicU64 = AtomicU64::new(0);

/// L'auditeur lui-meme. Touche par le SEUL fil `bouchaud-auditd`.
///
/// Un `static mut` plutot que des atomiques : la machine a etats compose une
/// decision -- poser un temoin, comparer, conclure -- et des atomiques lues
/// separement ne composent pas. C'est le meme raisonnement que pour
/// `SUIVI_VERDICT` dans le pilote RTL8168, et la meme discipline : un seul
/// ecrivain, nomme.
static mut AUDITEUR: Auditeur = Auditeur::nouveau();

/// Ce que l'auditeur lit, rassemble depuis les pilotes.
///
/// # Aucune lecture ne modifie ce qu'elle observe
///
/// `rtl8168::releve()` n'acquitte aucun statut et n'ecrit dans aucun
/// registre : un diagnostic qui modifie son objet efface la panne qu'il doit
/// nommer. C'est une propriete du pilote, gardee par
/// `tools/verifie-reseau-sans-triche.py`, et ce fil en depend.
fn observe(t_ns: u64) -> Observation {
    let r = crate::drivers::rtl8168::releve();
    let tambour = crate::kernel::blackbox::tambour();
    let (_, _, prochain_a_poser) = crate::kernel::blackbox::vidage_compteurs();

    Observation {
        t_ns,
        lien: crate::drivers::rtl8168::link_up(),

        rx_paquets: r.rx_paquets,
        rx_cur: r.rx_cur,
        rx_tours_cpu: r.rx_tours_cpu,
        rx_rendus_tour1: r.rx_rendus_tour1,
        rx_rendus_tour2: r.rx_rendus_tour2,
        rx_rearmes_tour1: r.rx_rearmes_tour1,
        rx_reutilises_tour2: r.rx_reutilises_tour2,
        desc_materiel: r.rx_desc_materiel,
        desc_processeur: r.rx_desc_processeur,
        rx_own_rendus: r.rx_own_rendus,

        isr_rx_ok: r.isr_rx_ok,
        isr_rx_err: r.isr_rx_err,
        isr_rx_overflow: r.isr_rx_overflow,
        isr_rx_fifo_over: r.isr_rx_fifo_over,
        isr_system_error: r.isr_system_error,
        rx_missed: r.rx_missed,
        chip_cmd: r.chip_cmd,

        invariant_casse: r.invariant.is_some(),
        invariant_code: code_invariant(r.invariant),

        bb_storage_ready: crate::drivers::xhci_active::blackbox_storage_ready(),
        bb_records_ram: tambour.poses,
        // `PROCHAIN_A_POSER` est le numero du prochain, donc le compte des
        // confirmes est celui d'avant. Un tambour neuf vaut un, pas zero.
        bb_records_persistes: prochain_a_poser.saturating_sub(1),
    }
}

/// L'invariant, en nombre : l'anneau ne transporte pas de chaines.
///
/// Le texte reste la reference ; ce code n'existe que pour qu'un evenement de
/// quatre entiers puisse dire LEQUEL des invariants a cede, et le client PC
/// le retraduit.
fn code_invariant(invariant: Option<&'static str>) -> u32 {
    match invariant {
        None => 0,
        Some(texte) => match texte {
            "eor-absent" => 1,
            "eor-double" => 2,
            "longueur-nulle" => 3,
            "adresse-deplacee" => 4,
            _ => 99,
        },
    }
}

/// Un tour d'audit : observer, juger, emettre, capturer.
///
/// Extrait du fil pour que le shell et le debugger distant puissent demander
/// un tour immediat (`audit run`) sans dupliquer la regle.
pub fn tour() -> usize {
    let t = crate::kernel::timer::monotonic_ns();
    let obs = observe(t);
    let rapport = unsafe { (*core::ptr::addr_of_mut!(AUDITEUR)).examine(&obs) };

    let n = TOURS.fetch_add(1, Ordering::Relaxed) + 1;
    CADENCE_HZ.store(rapport.cadence_hz as u64, Ordering::Relaxed);

    if rapport.cadence_changee {
        super::emets_a(
            t,
            Categorie::Audit,
            id::AUDIT_CADENCE,
            [rapport.cadence_hz as u64, rapport.len() as u64, 0, 0],
        );
    }

    for v in rapport.verdicts() {
        VERDICTS.fetch_add(1, Ordering::Relaxed);
        DERNIER_VERDICT_NS.store(t, Ordering::Relaxed);
        emets_verdict(t, &v);
    }

    // LA CAPTURE EST AUTOMATIQUE, et c'est tout l'interet.
    //
    // Les campagnes precedentes demandaient un vidage a la main, donc apres
    // coup, donc sur un etat deja retombe. Ici, l'etat materiel est
    // photographie DANS LA SECONDE ou la regle a conclu.
    if rapport.demande_capture() {
        CAPTURES.fetch_add(1, Ordering::Relaxed);
        crate::drivers::rtl8168::capture_lab(
            crate::drivers::rtl8168::raison_capture::DMA_STALL,
        );
    }

    // Une trace de vie, meme sans verdict : un auditeur muet et un auditeur
    // mort se ressemblent trop. A un hertz, une ligne toutes les soixante
    // secondes suffit a les distinguer sans noyer l'anneau.
    if rapport.is_empty() && n % 60 == 0 {
        super::emets_a(
            t,
            Categorie::Audit,
            id::AUDIT_SAIN,
            [n, 0, 0, 0],
        );
    }

    rapport.len()
}

fn emets_verdict(t: u64, v: &Verdict) {
    let (event, args) = match *v {
        Verdict::RxDmaStall { silence_ns, isr_delta, desc_processeur } => (
            id::AUDIT_RX_DMA_STALL,
            [0, isr_delta, desc_processeur as u64, silence_ns],
        ),
        Verdict::SecondTourAbsent { rendus_tour1, rendus_tour2, attente_ns } => (
            id::AUDIT_SECOND_TOUR_ABSENT,
            [rendus_tour1, rendus_tour2, attente_ns, 0],
        ),
        Verdict::AnneauInvariant { code, rx_cur } => (
            id::AUDIT_ANNEAU_INVARIANT,
            [code as u64, rx_cur as u64, 0, 0],
        ),
        Verdict::BlackboxPersistenceStall {
            records_ram,
            records_persistes,
            fige_depuis_ns,
        } => (
            id::AUDIT_BLACKBOX_PERSISTENCE_STALL,
            [records_ram, records_persistes, fige_depuis_ns, 0],
        ),
        Verdict::MoteurRxArrete { chip_cmd } => (
            id::AUDIT_ANNEAU_INVARIANT,
            [98, chip_cmd as u64, 0, 0],
        ),
    };
    DERNIER_VERDICT.store(event as u64, Ordering::Relaxed);

    // `AUDIT_RX_DMA_STALL` veut `rx_paquets` en premier argument, et la regle
    // ne le porte pas : elle juge des ECARTS. On le relit ici, au plus pres
    // de l'emission.
    let args = if event == id::AUDIT_RX_DMA_STALL {
        let r = crate::drivers::rtl8168::releve();
        [r.rx_paquets, args[1], args[2], args[3]]
    } else {
        args
    };
    super::emets_a(t, Categorie::Audit, event, args);

    // LE SECOND TOUR ABSENT MERITE SON PROPRE EVENEMENT COTE PILOTE.
    //
    // `AUDIT_SECOND_TOUR_ABSENT` est le verdict de la regle ;
    // `RING_SECOND_LAP_TIMEOUT` est le fait materiel qu'il designe. Les
    // separer permet au client PC de chercher l'un sans connaitre l'autre.
    if event == id::AUDIT_SECOND_TOUR_ABSENT {
        if let Verdict::SecondTourAbsent { rendus_tour2, attente_ns, .. } = *v {
            let r = crate::drivers::rtl8168::releve();
            super::emets_a(
                t,
                Categorie::Rtl8168,
                id::RING_SECOND_LAP_TIMEOUT,
                [attente_ns, r.rx_paquets, rendus_tour2, r.isr_rx_ok],
            );
            crate::drivers::rtl8168::capture_lab(
                crate::drivers::rtl8168::raison_capture::SECOND_TOUR_ABSENT,
            );
        }
    }
}

fn fil_auditd() -> ! {
    loop {
        tour();
        // La cadence est relue A CHAQUE TOUR : une anomalie vue au tour N doit
        // resserrer le tour N+1, pas le tour d'apres.
        let hz = CADENCE_HZ.load(Ordering::Relaxed).max(1);
        let periode_ms = 1_000 / hz;
        crate::kernel::task::sleep_ticks(crate::kernel::timer::ms_to_ticks(periode_ms));
    }
}

/// Lance l'auditeur. Idempotent.
pub fn demarre() -> bool {
    if LANCE.load(Ordering::Acquire) {
        return true;
    }
    super::demarre();
    if crate::kernel::task::spawn_noyau_priorite(
        fil_auditd,
        "bouchaud-auditd",
        crate::kernel::task::Priorite::Normale,
    ) {
        LANCE.store(true, Ordering::Release);
        return true;
    }
    false
}

pub fn lance() -> bool {
    LANCE.load(Ordering::Relaxed)
}

/// Tours, verdicts, captures, cadence courante.
pub fn compteurs() -> (u64, u64, u64, u64) {
    (
        TOURS.load(Ordering::Relaxed),
        VERDICTS.load(Ordering::Relaxed),
        CAPTURES.load(Ordering::Relaxed),
        CADENCE_HZ.load(Ordering::Relaxed),
    )
}

/// Dernier verdict rendu, et quand. Zero : aucun depuis l'amorcage.
pub fn dernier_verdict() -> (u32, u64) {
    (
        DERNIER_VERDICT.load(Ordering::Relaxed) as u32,
        DERNIER_VERDICT_NS.load(Ordering::Relaxed),
    )
}

/// L'auditeur a-t-il rendu un verdict depuis l'amorcage ?
pub fn sain() -> bool {
    VERDICTS.load(Ordering::Relaxed) == 0
}
