//! Stage 2 — bureau Bouchaud vivant sur le backend GOP.
//!
//! Ce module reutilise les vrais widgets du bureau et prouve d'abord la chaine
//! IRQ12 -> souris -> rendu GOP.

fn paint(mouse_pos: (usize, usize)) {
    use crate::gui::widgets;

    widgets::draw_desktop(&[]);
    widgets::draw_taskbar(&[], false);
    widgets::draw_cursor(mouse_pos.0, mouse_pos.1);
    crate::drivers::gfx::present();
}

pub fn render_once() {
    use crate::gui::framebuffer as fb;

    fb::enter();
    if !crate::drivers::gfx::is_active() {
        panic!("stage2: backend graphique inactif");
    }

    let pos = crate::drivers::mouse::pos();
    paint(pos);

    crate::serial_println!(
        "[STAGE2] desktop rendered icons={} logical={}x{} mouse={},{}",
        crate::gui::window::ICONS.len(),
        fb::width(),
        fb::height(),
        pos.0,
        pos.1,
    );
}

pub fn interactive_loop() -> ! {
    let mut last_pos = crate::drivers::mouse::pos();
    let mut last_buttons = crate::drivers::mouse::buttons();
    let mut pending = false;
    let mut first_motion_logged = false;
    let mut last_frame_ms = crate::kernel::timer::monotonic_ms();

    loop {
        let pos = crate::drivers::mouse::pos();
        let buttons = crate::drivers::mouse::buttons();

        if pos != last_pos || buttons != last_buttons {
            last_pos = pos;
            last_buttons = buttons;
            pending = true;
        }

        let now = crate::kernel::timer::monotonic_ms();
        if pending && now.wrapping_sub(last_frame_ms) >= 16 {
            paint(last_pos);
            last_frame_ms = now;
            pending = false;

            if !first_motion_logged {
                crate::serial_println!(
                    "BOUCHAUD_STAGE2_MOUSE_MOVED x={} y={} buttons={:#x}",
                    last_pos.0,
                    last_pos.1,
                    last_buttons,
                );
                first_motion_logged = true;
            }
        }

        x86_64::instructions::hlt();
    }
}
