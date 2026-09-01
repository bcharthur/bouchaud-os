// BOUCHAUD_MOUSE_IRQ_BOTTOM_HALF_V7
//
// HARD IRQ rule: never wait for BKL, never scan tasks, never call WaitQueue.
// Decode/publish atomically, EOI, return. The PIT flushes the GUI wake later
// after a successful non-blocking BKL try_enter().
extern "x86-interrupt" fn mouse_interrupt_handler(stack: InterruptStackFrame) {
    let _gs = GsGuard::enter(&stack);

    mouse::irq_note_enter();
    let status = unsafe { ports::inb(0x64) };
    let byte = unsafe { ports::inb(0x60) };
    mouse::irq_note_read(status, byte);

    if status & 0x20 != 0 {
        mouse::handle_byte(byte);
    } else {
        // Reroutage defensif du 8042. Ce chemin de repli n'a plus besoin du
        // gros verrou non plus : la file de scancodes porte le sien.
        keyboard::push_scancode(byte);
    }

    notify_end_of_interrupt(InterruptIndex::Mouse.as_u8());
    mouse::irq_note_eoi();
    mouse::irq_note_exit();
}
