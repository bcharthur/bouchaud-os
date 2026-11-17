//! Gate 0 — preuve hote du handoff post-switch.
//!
//! Propriete : aucun CPU ne peut reprendre la tache sortante tant que l'ancien
//! CPU n'a pas physiquement abandonne sa pile noyau.

use std::sync::atomic::{AtomicBool, AtomicI8, AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::mpsc::sync_channel;
use std::time::Duration;

const NO_TASK: usize = usize::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Etat { Pret, Bloque, Zombie }

#[derive(Clone, Debug)]
struct Tache {
    etat: Etat,
    on_cpu: i8,
    switching_out: bool,
    runq_cpu: usize,
    ctx_rsp: u64,
}

struct Modele {
    taches: Vec<Tache>,
    files: Vec<Vec<usize>>,
    pending: Vec<usize>,
    stack_left: Vec<bool>,
}

impl Modele {
    fn neuf(cpu: usize, taches: Vec<Tache>) -> Self {
        Self {
            taches,
            files: vec![Vec::new(); cpu],
            pending: vec![NO_TASK; cpu],
            stack_left: vec![false; cpu],
        }
    }

    fn eligible(&self, index: usize, cpu: usize) -> bool {
        let t = &self.taches[index];
        t.etat == Etat::Pret && t.on_cpu < 0 && !t.switching_out && t.runq_cpu == cpu
    }

    fn publish_ready(&mut self, index: usize) {
        let target = self.taches[index].runq_cpu;
        if self.taches[index].etat != Etat::Pret
            || self.taches[index].on_cpu >= 0
            || self.taches[index].switching_out
        { return; }
        if !self.files[target].contains(&index) { self.files[target].push(index); }
    }

    fn prepare(&mut self, cpu: usize, index: usize) {
        assert_eq!(self.pending[cpu], NO_TASK, "pending ecrase");
        assert_eq!(self.taches[index].on_cpu, cpu as i8);
        assert!(!self.taches[index].switching_out);
        self.pending[cpu] = index;
        self.taches[index].switching_out = true;
        self.stack_left[cpu] = false;
    }

    fn save_rsp(&mut self, index: usize, rsp: u64) { self.taches[index].ctx_rsp = rsp; }
    fn leave_stack(&mut self, cpu: usize) { self.stack_left[cpu] = true; }

    fn complete(&mut self, cpu: usize) {
        let index = self.pending[cpu];
        if index == NO_TASK { return; }
        assert!(self.stack_left[cpu], "publication avant abandon de pile");
        let t = &mut self.taches[index];
        assert!(t.switching_out);
        assert_eq!(t.on_cpu, cpu as i8);
        t.on_cpu = -1;
        t.switching_out = false;
        t.runq_cpu = cpu;
        self.pending[cpu] = NO_TASK;
        if t.etat == Etat::Pret { self.publish_ready(index); }
    }

    fn wake(&mut self, index: usize) {
        self.taches[index].etat = Etat::Pret;
        self.publish_ready(index);
    }

    fn reusable(&self, index: usize) -> bool {
        self.taches[index].etat == Etat::Zombie
            && self.taches[index].on_cpu < 0
            && !self.taches[index].switching_out
    }
}

fn running(etat: Etat, cpu: usize, rsp: u64) -> Tache {
    Tache { etat, on_cpu: cpu as i8, switching_out: false, runq_cpu: cpu, ctx_rsp: rsp }
}

const OLD_RSP: u64 = 0xFFFF_8000_0010_0000;
const NEW_RSP: u64 = 0xFFFF_8000_0020_0000;

#[test]
fn ancien_ordre_permettait_le_vol_avec_rsp_perime() {
    let mut t = running(Etat::Pret, 0, OLD_RSP);
    t.on_cpu = -1;
    let stolen = t.ctx_rsp;
    t.ctx_rsp = NEW_RSP;
    assert_eq!(stolen, OLD_RSP);
}

#[test]
fn sentinelle_rsp_reste_fausse_apres_save_avant_stack_leave() {
    let mut t = running(Etat::Pret, 0, OLD_RSP);
    t.ctx_rsp = 0;
    t.on_cpu = -1;
    assert_eq!(t.ctx_rsp, 0);

    // mov [rdi], rsp : la sentinelle est levee.
    t.ctx_rsp = NEW_RSP;

    // mov rsp, rsi n'est pas encore passe, mais l'ancien predicat dirait OK.
    assert!(t.etat == Etat::Pret && t.on_cpu < 0 && t.ctx_rsp != 0);
}

#[test]
fn handoff_final_refuse_avant_stack_left_meme_apres_save_rsp() {
    let mut m = Modele::neuf(2, vec![running(Etat::Pret, 0, OLD_RSP)]);
    m.prepare(0, 0);
    m.save_rsp(0, NEW_RSP);
    assert!(m.files[0].is_empty());
    assert_eq!(m.taches[0].on_cpu, 0);
    assert!(m.taches[0].switching_out);
    assert!(!m.eligible(0, 0));

    m.leave_stack(0);
    m.complete(0);
    assert_eq!(m.taches[0].on_cpu, -1);
    assert!(!m.taches[0].switching_out);
    assert_eq!(m.files[0], vec![0]);
}

#[test]
fn wake_concurrent_pending_n_est_pas_perdu_ni_publie_trop_tot() {
    let mut m = Modele::neuf(2, vec![running(Etat::Bloque, 0, OLD_RSP)]);
    m.prepare(0, 0);
    m.save_rsp(0, NEW_RSP);
    m.wake(0);
    assert_eq!(m.taches[0].etat, Etat::Pret);
    assert!(m.files[0].is_empty());
    m.leave_stack(0);
    m.complete(0);
    assert_eq!(m.files[0], vec![0]);
}

#[test]
fn blocked_reste_hors_runqueue_apres_completion() {
    let mut m = Modele::neuf(1, vec![running(Etat::Bloque, 0, OLD_RSP)]);
    m.prepare(0, 0);
    m.save_rsp(0, NEW_RSP);
    m.leave_stack(0);
    m.complete(0);
    assert_eq!(m.taches[0].on_cpu, -1);
    assert!(m.files[0].is_empty());
}

#[test]
fn zombie_recyclable_uniquement_apres_stack_left() {
    let mut m = Modele::neuf(1, vec![running(Etat::Zombie, 0, OLD_RSP)]);
    m.prepare(0, 0);
    m.save_rsp(0, NEW_RSP);
    assert!(!m.reusable(0));
    m.leave_stack(0);
    m.complete(0);
    assert!(m.reusable(0));
}

#[test]
#[should_panic(expected = "pending ecrase")]
fn pending_ne_peut_pas_etre_ecrase() {
    let mut m = Modele::neuf(
        1,
        vec![running(Etat::Pret, 0, OLD_RSP), running(Etat::Pret, 0, OLD_RSP + 0x1000)],
    );
    m.prepare(0, 0);
    m.prepare(0, 1);
}

#[test]
fn completion_apres_vidage_est_idempotente() {
    let mut m = Modele::neuf(1, vec![running(Etat::Pret, 0, OLD_RSP)]);
    m.prepare(0, 0);
    m.save_rsp(0, NEW_RSP);
    m.leave_stack(0);
    m.complete(0);
    m.complete(0);
    assert_eq!(m.files[0], vec![0]);
}

#[test]
fn stress_concurrent_aucune_publication_avant_stack_left() {
    // Test CONCURRENT mais orchestre : on agrandit volontairement la fenetre
    // save-rsp -> stack-left et on force l'observateur a verifier l'invariant
    // PENDANT cette fenetre. Aucun Barrier reutilise, donc pas de deadlock
    // sporadique du harnais lui-meme.
    const TOURS: u64 = 2_000;
    const TIMEOUT: Duration = Duration::from_secs(2);

    let on_cpu = Arc::new(AtomicI8::new(0));
    let switching = Arc::new(AtomicBool::new(false));
    let saved = Arc::new(AtomicBool::new(false));
    let stack_left = Arc::new(AtomicBool::new(false));
    let epoch = Arc::new(AtomicU64::new(0));

    // Quatre rendez-vous explicites :
    // start -> saved -> autorise_leave -> done.
    let (start_tx, start_rx) = sync_channel::<u64>(0);
    let (saved_tx, saved_rx) = sync_channel::<u64>(0);
    let (leave_tx, leave_rx) = sync_channel::<u64>(0);
    let (done_tx, done_rx) = sync_channel::<u64>(0);

    let a_on = Arc::clone(&on_cpu);
    let a_sw = Arc::clone(&switching);
    let a_saved = Arc::clone(&saved);
    let a_left = Arc::clone(&stack_left);
    let a_epoch = Arc::clone(&epoch);

    let commutateur = std::thread::spawn(move || {
        for tour in 1..=TOURS {
            let ordre = start_rx.recv_timeout(TIMEOUT).expect("start timeout");
            assert_eq!(ordre, tour);

            // prepare_switch_handoff()
            a_on.store(0, Ordering::Release);
            a_sw.store(true, Ordering::Release);
            a_saved.store(false, Ordering::Release);
            a_left.store(false, Ordering::Release);
            a_epoch.store(tour, Ordering::Release);

            // Equivalent au `mov [rdi], rsp`.
            a_saved.store(true, Ordering::Release);
            saved_tx.send(tour).expect("saved channel");

            // L'observateur garde volontairement le writer ICI :
            // le CPU sortant utilise encore sa pile.
            let autorise = leave_rx.recv_timeout(TIMEOUT).expect("leave timeout");
            assert_eq!(autorise, tour);

            // Equivalent au `mov rsp, rsi`, puis completion depuis la pile
            // entrante.
            a_left.store(true, Ordering::Release);
            a_on.store(-1, Ordering::Release);
            a_sw.store(false, Ordering::Release);

            done_tx.send(tour).expect("done channel");
        }
    });

    for tour in 1..=TOURS {
        start_tx.send(tour).expect("start channel");

        let saved_tour = saved_rx.recv_timeout(TIMEOUT).expect("saved timeout");
        assert_eq!(saved_tour, tour);
        assert_eq!(epoch.load(Ordering::Acquire), tour);
        assert!(saved.load(Ordering::Acquire));

        // Point decisif : RSP est DEJA sauvegarde, mais l'ancien CPU n'a PAS
        // encore quitte la pile. La tache doit rester residente/ineligible.
        assert!(!stack_left.load(Ordering::Acquire));
        assert_eq!(
            on_cpu.load(Ordering::Acquire),
            0,
            "publication avant stack-left au tour {tour}"
        );
        assert!(
            switching.load(Ordering::Acquire),
            "switching_out perdu avant stack-left au tour {tour}"
        );

        leave_tx.send(tour).expect("leave channel");

        let done_tour = done_rx.recv_timeout(TIMEOUT).expect("done timeout");
        assert_eq!(done_tour, tour);
        assert!(stack_left.load(Ordering::Acquire));
        assert_eq!(on_cpu.load(Ordering::Acquire), -1);
        assert!(!switching.load(Ordering::Acquire));
    }

    commutateur.join().expect("commutateur");
}
