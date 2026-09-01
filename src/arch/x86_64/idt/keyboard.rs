extern "x86-interrupt" fn keyboard_interrupt_handler(stack: InterruptStackFrame) {
    let _gs = GsGuard::enter(&stack);
    let interrupted_user = from_user(&stack);

    let status = unsafe { ports::inb(0x64) };
    let data = unsafe { ports::inb(0x60) };

    if status & 0x20 == 0 {
        // Plus de gros verrou : la file de scancodes a le sien, un
        // `SpinLockIrq` dont la section critique fait quelques instructions.
        keyboard::push_scancode(data);
    } else {
        // 8042 can route an auxiliary byte through IRQ1. Keep that mouse byte
        // on the same BKL-free hard-IRQ path as IRQ12.
        mouse::irq_note_enter();
        mouse::irq_note_read(status, data);
        mouse::handle_byte(data);
    }

    notify_end_of_interrupt(InterruptIndex::Keyboard.as_u8());

    if status & 0x20 != 0 {
        mouse::irq_note_eoi();
        mouse::irq_note_exit();
    }

    if interrupted_user && crate::kernel::task::in_user_task() {
        crate::kernel::task::request_deferred_preempt();
    }
}
