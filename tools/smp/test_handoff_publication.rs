//! Regression #264: order of scheduler handoff publication.
//! Structural guard plus atomic litmus; KVM SMP4 soak remains required.
use std::fs;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
const NO_TASK: usize = usize::MAX;
fn source() -> String { fs::read_to_string("src/kernel/process/thread/courant.rs").unwrap() }
fn section<'a>(s: &'a str, begin: &str, end: &str) -> &'a str {
    let a = s.find(begin).expect("debut introuvable");
    let b = s[a..].find(end).expect("fin introuvable") + a;
    &s[a..b]
}
#[test]
fn flags_before_pending_release() {
    let s = source();
    let f = section(&s, "fn prepare_switch_handoff(", "/// Publie la tache sortante");
    let flag = f.find("task.switching_out.range(true)").unwrap();
    let remise = f.find("SWITCH_REMISE[cpu].store(remise").unwrap();
    let ptr = f.find("SWITCH_PENDING[cpu].compare_exchange(").unwrap();
    assert!(flag < remise && remise < ptr, "pending publie avant drapeau");
    assert!(f[ptr..].contains("Ordering::AcqRel"));
    assert!(f.contains("task.switching_out.range(false)"), "rollback du drapeau manquant");
}
#[test]
fn completion_irq_atomic_and_publish_outside() {
    let s = source();
    let f = section(&s, "fn complete_switch_handoff()", "/// PID du programme");
    let guard = f.find("interrupts::without_interrupts(||").unwrap();
    let load = f.find("SWITCH_PENDING[cpu].load(Ordering::Acquire)").unwrap();
    let clear = f.find("SWITCH_PENDING[cpu].store(NO_TASK, Ordering::Release)").unwrap();
    let end_guard = f.find("Some((outgoing, publish, remise))").unwrap();
    let publish = f.find("publie_pret(outgoing").unwrap();
    assert!(guard < load && load < clear && clear < end_guard && end_guard < publish);
    assert!(f[guard..end_guard].contains("task.switching_out.range(false)"));
}
#[test]
fn model_release_acquire_excludes_half_published_task() {
    let pending = Arc::new(AtomicUsize::new(NO_TASK));
    let switching = Arc::new(AtomicBool::new(false));
    let p = pending.clone();
    let s = switching.clone();
    let producer = thread::spawn(move || {
        for n in 0..2500usize {
            while p.load(Ordering::Acquire) != NO_TASK { thread::yield_now(); }
            s.store(true, Ordering::Relaxed);
            p.store(n, Ordering::Release);
            while p.load(Ordering::Acquire) != NO_TASK { thread::yield_now(); }
            s.store(false, Ordering::Relaxed);
        }
    });
    let mut seen = 0;
    while seen < 2500 {
        let index = pending.load(Ordering::Acquire);
        if index != NO_TASK {
            assert_eq!(index, seen);
            assert!(switching.load(Ordering::Relaxed), "pending publie sans flag");
            pending.store(NO_TASK, Ordering::Release);
            seen += 1;
        } else { thread::yield_now(); }
    }
    producer.join().unwrap();
}
