#[path = "../../src/arch/x86_64/irq_context.rs"]
mod irq_context;
use irq_context::IrqContext;

#[test]
fn nested_handlers_balance_without_erasing_the_outer_irq() {
    let cpu = IrqContext::new();
    cpu.enter(); cpu.enter();
    assert_eq!(cpu.depth(), 2);
    cpu.exit(); assert_eq!(cpu.depth(), 1);
    cpu.exit(); assert_eq!(cpu.depth(), 0);
    assert_eq!(cpu.entries(), 2);
    assert_eq!(cpu.maximum(), 2);
}

#[test]
fn suspended_irq_follows_its_stack_across_cpus_and_idle() {
    let a = IrqContext::new(); let b = IrqContext::new();
    a.enter();
    let saved = a.suspend();
    assert_eq!(a.depth(), 0); // fresh task / idle on the old CPU
    a.enter(); a.exit(); // unrelated IRQ must not inherit the suspended one
    b.restore(saved);
    b.enter(); b.exit(); // nested interrupt after migration
    assert_eq!(b.depth(), 1);
    b.exit();
    assert_eq!((a.depth(), b.depth()), (0, 0));
}

#[test]
fn terminated_irq_stack_does_not_poison_the_next_task() {
    let cpu = IrqContext::new();
    cpu.enter(); let _abandoned = cpu.suspend();
    cpu.restore(0);
    assert_eq!(cpu.depth(), 0);
}

#[test]
fn user_page_fault_becomes_a_sleepable_task_before_sti() {
    let a = IrqContext::new(); let b = IrqContext::new();
    a.enter(); // GS et CR2 sont captures avec IF=0.
    a.quitte_trappe_utilisateur();
    assert_eq!(a.depth(), 0, "console et point sur exigent zero IRQ");
    a.enter(); // timer reel pendant le peuplement utilisateur
    assert_eq!(a.depth(), 1, "timer non imbrique dans une IRQ materielle");
    a.enter(); assert_eq!(a.depth(), 2); // vraie imbrication reste interdite
    a.exit(); a.exit();
    let saved = a.suspend(); b.restore(saved);
    assert_eq!(b.depth(), 0, "la faute peut attendre et migrer comme un syscall");
    b.enter(); b.exit();
    assert_eq!((a.depth(), b.depth()), (0, 0));
}

#[test]
#[should_panic(expected = "user trap nested in an IRQ context")]
fn a_kernel_fault_nested_in_an_irq_cannot_become_a_user_task() {
    let cpu = IrqContext::new(); cpu.enter(); cpu.enter();
    cpu.quitte_trappe_utilisateur();
}

#[test]
#[should_panic(expected = "irq_depth underflow")]
fn unmatched_exit_is_rejected() { IrqContext::new().exit(); }

#[test]
#[should_panic(expected = "inherited another IRQ context")]
fn restoring_over_a_live_frame_is_rejected() {
    let cpu = IrqContext::new(); cpu.enter(); cpu.restore(1);
}
