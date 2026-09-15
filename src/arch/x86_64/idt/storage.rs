extern "x86-interrupt" fn ata_primary_handler(stack: InterruptStackFrame) {
    smp::note_premiere_irq(InterruptIndex::AtaPrimary.as_u8(), stack.instruction_pointer.as_u64(), stack.stack_pointer.as_u64());
    let _gs = GsGuard::enter(&stack);
    // Ni le port de statut ni l'accuse de fin d'interruption ne demandent le
    // gros verrou : ce sont deux acces port, atomiques par le materiel. Les
    // gestionnaires clavier et souris accusent deja sans lui ; ceux-ci le
    // prenaient par habitude.
    let _ = unsafe { ports::inb(0x1F7) };
    notify_end_of_interrupt(InterruptIndex::AtaPrimary.as_u8());
}

// L'ATA SECONDAIRE N'A PLUS DE GESTIONNAIRE A LUI.
//
// Son vecteur (0x2F) est aussi celui du parasite du PIC esclave. Un
// gestionnaire qui acquitte inconditionnellement acquitte donc, une fois sur
// deux, une interruption que l'esclave n'avait pas mise en service. Les deux
// cas sont desormais distingues dans `imprevus.rs`, qui lit l'ISR avant de
// decider -- et qui fait la lecture du registre de statut 0x177 quand l'IRQ15
// est reelle.
