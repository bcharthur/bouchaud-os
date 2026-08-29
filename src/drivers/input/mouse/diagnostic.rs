// Rapport périodique du chemin IRQ12 -> bottom-half GUI.

#[inline]
fn phase_name(p: u8) -> &'static str {
    match p {
        PHASE_ENTER => "enter",
        PHASE_READ => "read",
        PHASE_DECODE => "decode",
        PHASE_PUBLISH => "publish",
        PHASE_EOI => "eoi",
        PHASE_EXIT => "exit",
        _ => "idle",
    }
}

pub fn log_diagnostic() {
    let now = crate::kernel::timer::monotonic_ns();
    let last_irq = LAST_IRQ_NS.load(Ordering::Acquire);
    let last_packet = LAST_PACKET_NS.load(Ordering::Acquire);
    let (irq_signals, irq_flushes, irq_woken, irq_pending) =
        crate::kernel::sync::reveil::INTERFACE.irq_statistiques();

    let phase = IRQ_PHASE.load(Ordering::Acquire);
    crate::serial_println!(
        "[MOUSE-IRQ] phase={}({}) entries={} bytes={} eoi={} exit={} packets={} changed={} deferred={} irq_signals={} irq_flushes={} irq_woken={} pending={} last_irq_age_ns={} last_packet_age_ns={} status={:#x} byte={:#x} pos={},{} btn={:#x} wheel={}",
        phase,
        phase_name(phase),
        IRQ_ENTRIES.load(Ordering::Relaxed),
        IRQ_BYTES.load(Ordering::Relaxed),
        IRQ_EOI.load(Ordering::Relaxed),
        IRQ_EXIT.load(Ordering::Relaxed),
        PACKETS.load(Ordering::Relaxed),
        PACKETS_CHANGED.load(Ordering::Relaxed),
        DEFERRED_SIGNALS.load(Ordering::Relaxed),
        irq_signals,
        irq_flushes,
        irq_woken,
        irq_pending as u8,
        if last_irq == 0 { 0 } else { now.saturating_sub(last_irq) },
        if last_packet == 0 { 0 } else { now.saturating_sub(last_packet) },
        LAST_STATUS.load(Ordering::Relaxed),
        LAST_BYTE.load(Ordering::Relaxed),
        MX.load(Ordering::Acquire),
        MY.load(Ordering::Acquire),
        BTN.load(Ordering::Acquire),
        WHEEL_DELTA.load(Ordering::Acquire),
    );
}
