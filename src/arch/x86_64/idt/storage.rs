extern "x86-interrupt" fn ata_primary_handler(stack: InterruptStackFrame) {
    let _gs = GsGuard::enter(&stack);
    // Ni le port de statut ni l'accuse de fin d'interruption ne demandent le
    // gros verrou : ce sont deux acces port, atomiques par le materiel. Les
    // gestionnaires clavier et souris accusent deja sans lui ; ceux-ci le
    // prenaient par habitude.
    let _ = unsafe { ports::inb(0x1F7) };
    notify_end_of_interrupt(InterruptIndex::AtaPrimary.as_u8());
}

extern "x86-interrupt" fn ata_secondary_handler(stack: InterruptStackFrame) {
    let _gs = GsGuard::enter(&stack);
    let _ = unsafe { ports::inb(0x177) };
    notify_end_of_interrupt(InterruptIndex::AtaSecondary.as_u8());
}
