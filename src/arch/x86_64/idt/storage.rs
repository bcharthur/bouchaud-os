extern "x86-interrupt" fn ata_primary_handler(stack: InterruptStackFrame) {
    let _gs = GsGuard::enter(&stack);
    let _kernel = crate::kernel::smp_lock::enter();
    let _ = unsafe { ports::inb(0x1F7) };
    notify_end_of_interrupt(InterruptIndex::AtaPrimary.as_u8());
}

extern "x86-interrupt" fn ata_secondary_handler(stack: InterruptStackFrame) {
    let _gs = GsGuard::enter(&stack);
    let _kernel = crate::kernel::smp_lock::enter();
    let _ = unsafe { ports::inb(0x177) };
    notify_end_of_interrupt(InterruptIndex::AtaSecondary.as_u8());
}
