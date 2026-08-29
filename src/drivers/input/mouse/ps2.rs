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
        HAS_WHEEL.store(id == 3 || id == 4, Ordering::Release);
        mouse_cmd(0xF4);

        crate::arch::x86_64::interrupts::unmask_irq(1);
        crate::arch::x86_64::interrupts::unmask_irq(12);

        MX.store((WIDTH / 2) as i32, Ordering::Release);
        MY.store((HEIGHT / 2) as i32, Ordering::Release);
        BTN.store(0, Ordering::Release);
        WHEEL_DELTA.store(0, Ordering::Release);
        CYCLE = 0;

        crate::kernel::dmesg::log_fmt(format_args!(
            "mouse: 8042 config={:#04x}, id={}, wheel={}",
            status,
            id,
            HAS_WHEEL.load(Ordering::Acquire)
        ));
    });

    crate::drivers::keyboard::rearm_after_8042_reconfigure();
}
