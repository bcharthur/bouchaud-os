// Configuration 8042 / IntelliMouse.

fn wait_write() {
    for _ in 0..100_000 {
        if unsafe { inb(0x64) } & 0x02 == 0 { return; }
    }
}

fn wait_read() {
    for _ in 0..100_000 {
        if unsafe { inb(0x64) } & 0x01 != 0 { return; }
    }
}

unsafe fn ctl(cmd: u8) { wait_write(); outb(0x64, cmd); }
unsafe fn wr(data: u8) { wait_write(); outb(0x60, data); }
unsafe fn rd() -> u8 { wait_read(); inb(0x60) }
unsafe fn mouse_cmd(v: u8) { ctl(0xD4); wr(v); let _ = rd(); }

unsafe fn set_sample_rate(rate: u8) {
    mouse_cmd(0xF3);
    mouse_cmd(rate);
}

/// Active la souris et l'IRQ12.
pub fn init() {
    interrupts::without_interrupts(|| unsafe {
        ctl(0xAE);
        ctl(0xA8);

        ctl(0x20);
        let mut status = rd();
        status |= 0x03;
        status |= 0x40;
        status &= !0x10;
        status &= !0x20;
        ctl(0x60);
        wr(status);

        mouse_cmd(0xF6);
        set_sample_rate(200);
        set_sample_rate(100);
        set_sample_rate(80);
        mouse_cmd(0xF2);
        let id = rd();

        // BOUCHAUD_PS2_IDENTITE_AVANT_IRQ_V1
        //
        // ON N'ARME PAS L'IRQ D'UN PERIPHERIQUE QUI N'EST PAS LA.
        //
        // Une souris PS/2 repond a `GET_DEVICE_ID` par 0x00 (standard), 0x03
        // (molette) ou 0x04 (cinq boutons). La machine de reference a repondu
        // 0xFE -- « RESEND » : un controleur 8042 emule par le micrologiciel,
        // sans aucune souris derriere. C'est la seule reponse que le protocole
        // ne permet PAS d'interpreter comme une identite.
        //
        // On demasquait quand meme IRQ12, et la machine est morte sur une
        // double faute dans la seconde qui a suivi -- releve du 12 septembre
        // 18:40, `vecteur=0x8`, tache `desktop`, pile a peine entamee. Aucun
        // des demarrages precedents n'etait passe par ici : ils avaient tous
        // une souris USB, et ce repli ne s'arme que lorsqu'il n'y en a pas.
        //
        // Armer une ligne d'interruption pour un peripherique absent n'apporte
        // rien, meme quand cela ne casse rien : il n'y aura jamais de paquet a
        // decoder.
        const ID_STANDARD: u8 = 0x00;
        const ID_MOLETTE: u8 = 0x03;
        const ID_CINQ_BOUTONS: u8 = 0x04;
        let presente = matches!(id, ID_STANDARD | ID_MOLETTE | ID_CINQ_BOUTONS);
        HAS_WHEEL.store(id == ID_MOLETTE || id == ID_CINQ_BOUTONS, Ordering::Release);
        PRESENTE.store(presente, Ordering::Release);

        if presente {
            mouse_cmd(0xF4);
        }

        // Le CLAVIER, lui, reste arme quoi qu'il arrive : sa ligne est IRQ1 et
        // son identite n'a rien a voir avec celle de la souris.
        crate::arch::x86_64::interrupts::unmask_irq(1);
        if presente {
            crate::arch::x86_64::interrupts::unmask_irq(12);
        }

        MX.store((crate::drivers::gfx::width() / 2) as i32, Ordering::Release);
        MY.store((crate::drivers::gfx::height() / 2) as i32, Ordering::Release);
        BTN.store(0, Ordering::Release);
        WHEEL_DELTA.store(0, Ordering::Release);
        CYCLE = 0;

        crate::kernel::dmesg::log_fmt(format_args!(
            "mouse: 8042 config={:#04x}, id={:#04x}, presente={}, wheel={}, irq12={}",
            status,
            id,
            presente,
            HAS_WHEEL.load(Ordering::Acquire),
            presente,
        ));
        crate::serial_println!(
            "BOUCHAUD_PS2_SOURIS id={:#04x} presente={} irq12_armee={}",
            id, presente as u8, presente as u8,
        );
    });

    crate::drivers::keyboard::rearm_after_8042_reconfigure();
}
