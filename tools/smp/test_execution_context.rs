#[path = "../../src/kernel/scheduler/continuation.rs"]
mod continuation;
use continuation::{contexte, ContexteExecution::*};

#[test]
fn no_task_distinguishes_bsp_caller_from_parked_continuation_and_ap_idle() {
    assert_eq!(contexte(0, None, false), ContinuationBsp);
    assert_eq!(contexte(0, None, true), Idle);
    assert_eq!(contexte(1, None, false), Idle);
    assert_eq!(contexte(1, None, true), Idle);
}

#[test]
fn installed_tasks_own_their_blocking_state() {
    for cpu in 0..4 {
        for parked in [false, true] {
            assert_eq!(contexte(cpu, Some(17), parked), Tache(17));
        }
    }
}
