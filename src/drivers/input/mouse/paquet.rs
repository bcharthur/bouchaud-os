// Décodeur IRQ12 et état de souris.
//
// Règle V7 : cette couche ne prend JAMAIS le BKL et ne parcourt JAMAIS la table
// des tâches. Elle publie seulement l'état et un signal différé atomique.

#[inline]
pub fn irq_note_enter() {
    IRQ_PHASE.store(PHASE_ENTER, Ordering::Release);
    IRQ_ENTRIES.fetch_add(1, Ordering::Relaxed);
    LAST_IRQ_NS.store(crate::kernel::timer::monotonic_ns(), Ordering::Release);
}

#[inline]
pub fn irq_note_read(status: u8, byte: u8) {
    LAST_STATUS.store(status, Ordering::Relaxed);
    LAST_BYTE.store(byte, Ordering::Relaxed);
    IRQ_BYTES.fetch_add(1, Ordering::Relaxed);
    IRQ_PHASE.store(PHASE_READ, Ordering::Release);
}

#[inline]
pub fn irq_note_eoi() {
    IRQ_EOI.fetch_add(1, Ordering::Relaxed);
    IRQ_PHASE.store(PHASE_EOI, Ordering::Release);
}

#[inline]
pub fn irq_note_exit() {
    IRQ_EXIT.fetch_add(1, Ordering::Relaxed);
    IRQ_PHASE.store(PHASE_EXIT, Ordering::Release);
}

/// Traite un octet reçu de la souris. Appelé depuis IRQ12/IRQ1.
///
/// Aucun verrou, aucune allocation, aucun journal série.
pub fn handle_byte(b: u8) {
    IRQ_PHASE.store(PHASE_DECODE, Ordering::Release);
    unsafe {
        match CYCLE {
            0 => {
                if b & 0x08 == 0 { return; }
                PKT[0] = b;
                CYCLE = 1;
            }
            1 => { PKT[1] = b; CYCLE = 2; }
            2 if HAS_WHEEL.load(Ordering::Acquire) => {
                PKT[2] = b;
                CYCLE = 3;
            }
            2 => {
                PKT[2] = b;
                CYCLE = 0;
                apply_packet(false);
            }
            3 => {
                PKT[3] = b;
                CYCLE = 0;
                apply_packet(true);
            }
            _ => { CYCLE = 0; }
        }
    }
}

unsafe fn apply_packet(with_wheel: bool) {
    PACKETS.fetch_add(1, Ordering::Relaxed);
    LAST_PACKET_NS.store(crate::kernel::timer::monotonic_ns(), Ordering::Release);

    let flags = PKT[0];
    let dx = PKT[1] as i8 as i32;
    let dy = PKT[2] as i8 as i32;

    let old_x = MX.load(Ordering::Relaxed);
    let old_y = MY.load(Ordering::Relaxed);
    let old_btn = BTN.load(Ordering::Relaxed);
    let old_wheel = WHEEL_DELTA.load(Ordering::Relaxed);

    let new_x = old_x.saturating_add(dx).clamp(0, WIDTH as i32 - 1);
    let new_y = old_y.saturating_sub(dy).clamp(0, HEIGHT as i32 - 1);
    let new_btn = flags & 0x07;

    MX.store(new_x, Ordering::Release);
    MY.store(new_y, Ordering::Release);
    BTN.store(new_btn, Ordering::Release);

    if with_wheel {
        let wheel = PKT[3] as i8 as i32;
        if wheel != 0 {
            let _ = WHEEL_DELTA.fetch_update(
                Ordering::AcqRel,
                Ordering::Acquire,
                |v| Some(v.saturating_add(wheel)),
            );
        }
    }

    let new_wheel = WHEEL_DELTA.load(Ordering::Acquire);
    if (new_x, new_y, new_btn, new_wheel) != (old_x, old_y, old_btn, old_wheel) {
        PACKETS_CHANGED.fetch_add(1, Ordering::Relaxed);
        DEFERRED_SIGNALS.fetch_add(1, Ordering::Relaxed);
        IRQ_PHASE.store(PHASE_PUBLISH, Ordering::Release);

        // Crucial V7: no WaitQueue wake and no BKL from the hard input IRQ.
        crate::kernel::sync::reveil::signale_interface_irq(
            crate::kernel::sync::reveil::Source::Souris,
        );
    }
}

/// Position courante du curseur.
pub fn pos() -> (usize, usize) {
    (
        MX.load(Ordering::Acquire).max(0) as usize,
        MY.load(Ordering::Acquire).max(0) as usize,
    )
}

pub fn left_down() -> bool {
    BTN.load(Ordering::Acquire) & 0x01 != 0
}

pub fn buttons() -> u8 {
    BTN.load(Ordering::Acquire) & 0x07
}

pub fn wheel_pending() -> bool {
    WHEEL_DELTA.load(Ordering::Acquire) != 0
}

pub fn take_wheel() -> i32 {
    WHEEL_DELTA.swap(0, Ordering::AcqRel)
}
