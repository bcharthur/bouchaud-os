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

    let max_x = crate::drivers::gfx::width().saturating_sub(1) as i32;
    let max_y = crate::drivers::gfx::height().saturating_sub(1) as i32;
    let new_x = old_x.saturating_add(dx).clamp(0, max_x);
    let new_y = old_y.saturating_sub(dy).clamp(0, max_y);
    let new_btn = recompose_boutons(SOURCE_PS2, flags & 0x07);

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

/// Publie l'etat des boutons de `source` et rend l'union de toutes les sources.
///
/// Voir `BOUCHAUD_SOURIS_BOUTONS_PAR_SOURCE_V1` dans `mouse/etat.rs` pour le
/// defaut que cette union corrige. La boucle est bornee par une constante et
/// ne prend aucun verrou : elle est appelee depuis un contexte d'interruption.
fn recompose_boutons(source: usize, boutons: u8) -> u8 {
    if source < SOURCES_BOUTONS {
        BOUTONS_PAR_SOURCE[source].store(boutons & 0x07, Ordering::Release);
    }
    let mut union = 0u8;
    for etat in BOUTONS_PAR_SOURCE.iter() {
        union |= etat.load(Ordering::Acquire);
    }
    union & 0x07
}

/// Reserve une source de boutons pour un nouveau point de terminaison souris.
///
/// Au-dela de `SOURCES_BOUTONS`, la derniere source est PARTAGEE plutot que
/// de deborder du tableau : le service reste degrade mais jamais faux.
pub fn reserve_source_souris() -> usize {
    loop {
        let occupees = SOURCES_OCCUPEES.load(Ordering::Acquire);
        let mut libre = SOURCES_BOUTONS;
        for index in 1..SOURCES_BOUTONS {
            if occupees & (1u32 << index) == 0 {
                libre = index;
                break;
            }
        }
        if libre == SOURCES_BOUTONS {
            return SOURCES_BOUTONS - 1;
        }
        if SOURCES_OCCUPEES
            .compare_exchange(
                occupees,
                occupees | (1u32 << libre),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            BOUTONS_PAR_SOURCE[libre].store(0, Ordering::Release);
            return libre;
        }
    }
}

/// Un peripherique disparait : sa source ne tient plus aucun bouton.
///
/// Sans cette remise a zero, debrancher une souris bouton enfonce laisserait
/// le bureau croire le bouton maintenu pour toujours -- et la source
/// reservee ne reviendrait jamais, si bien qu'une dizaine de rebranchements
/// epuiseraient le tableau.
pub fn libere_source_souris(source: usize) {
    if source == SOURCE_PS2 || source >= SOURCES_BOUTONS {
        return;
    }
    BOUTONS_PAR_SOURCE[source].store(0, Ordering::Release);
    SOURCES_OCCUPEES.fetch_and(!(1u32 << source), Ordering::AcqRel);
    let mut union = 0u8;
    for etat in BOUTONS_PAR_SOURCE.iter() {
        union |= etat.load(Ordering::Acquire);
    }
    BTN.store(union & 0x07, Ordering::Release);
}

// BOUCHAUD_USB_HID_MOUSE_BRIDGE_V3
/// Injecte un rapport HID boot mouse dans le MEME etat que le chemin PS/2.
/// HID utilise +Y vers le bas, contrairement aux paquets PS/2 (+Y vers le haut).
///
/// `source` identifie le point de terminaison qui parle : ses boutons ne
/// valent que pour lui, et l'etat global est l'union de toutes les sources.
/// Sans cela, une interface au repos effacait le bouton maintenu sur une
/// autre -- voir `recompose_boutons`.
pub fn inject_usb_report(source: usize, buttons: u8, dx: i8, dy: i8, wheel: i8) {
    PACKETS.fetch_add(1, Ordering::Relaxed);
    LAST_PACKET_NS.store(crate::kernel::timer::monotonic_ns(), Ordering::Release);

    let old_x = MX.load(Ordering::Relaxed);
    let old_y = MY.load(Ordering::Relaxed);
    let old_btn = BTN.load(Ordering::Relaxed);
    let old_wheel = WHEEL_DELTA.load(Ordering::Relaxed);
    let max_x = crate::drivers::gfx::width().saturating_sub(1) as i32;
    let max_y = crate::drivers::gfx::height().saturating_sub(1) as i32;
    let new_x = old_x.saturating_add(dx as i32).clamp(0, max_x);
    let new_y = old_y.saturating_add(dy as i32).clamp(0, max_y);
    let new_btn = recompose_boutons(source, buttons & 0x07);

    MX.store(new_x, Ordering::Release);
    MY.store(new_y, Ordering::Release);
    BTN.store(new_btn, Ordering::Release);
    if wheel != 0 {
        let wheel = wheel as i32;
        let _ = WHEEL_DELTA.fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| {
            Some(v.saturating_add(wheel))
        });
    }
    let new_wheel = WHEEL_DELTA.load(Ordering::Acquire);
    if (new_x, new_y, new_btn, new_wheel) != (old_x, old_y, old_btn, old_wheel) {
        PACKETS_CHANGED.fetch_add(1, Ordering::Relaxed);
        DEFERRED_SIGNALS.fetch_add(1, Ordering::Relaxed);
        crate::kernel::sync::reveil::signale_interface_irq(
            crate::kernel::sync::reveil::Source::Souris,
        );
    }
}
