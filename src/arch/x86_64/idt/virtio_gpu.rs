// BOUCHAUD_VIRTIO_GPU_IRQ_V1 (P10+.1)
//
// L'achevement d'une commande de la file de controle virtio-gpu, livre par
// MSI-X sur un vecteur LAPIC dedie. Le gestionnaire n'execute aucune
// commande et ne prend aucun verrou : il compte, et reveille la file des
// achevements (`virtio_gpu::interruption`). L'attendant relit l'anneau
// lui-meme ; une interruption de trop n'acheve rien.
extern "x86-interrupt" fn virtio_gpu_interrupt_handler(stack: InterruptStackFrame) {
    smp::note_premiere_irq(
        crate::drivers::virtio_gpu_irq::VECTEUR,
        stack.instruction_pointer.as_u64(),
        stack.stack_pointer.as_u64(),
    );
    let _gs = GsGuard::enter(&stack);
    crate::drivers::virtio_gpu::interruption();
    smp::eoi_local();
}
