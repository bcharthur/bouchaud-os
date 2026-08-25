//! Pilote souris PS/2 (port auxiliaire du controleur 8042), pilote par IRQ12.
//!
//! Initialise a l'entree du bureau graphique. Le gestionnaire d'IRQ12 (voir
//! `arch::x86_64::idt`) transmet chaque octet a `handle_byte`, qui reconstitue
//! les paquets de 3 octets (ou 4 octets avec molette IntelliMouse) et met a
//! jour la position, les boutons et le delta de roulette.

use crate::arch::x86_64::ports::{inb, outb};
use x86_64::instructions::interrupts;
use crate::drivers::gfx::{WIDTH, HEIGHT};

static mut MX: i32 = (WIDTH / 2) as i32;
static mut MY: i32 = (HEIGHT / 2) as i32;
static mut BTN: u8 = 0;
static mut CYCLE: u8 = 0;
static mut PKT: [u8; 4] = [0; 4];
static mut HAS_WHEEL: bool = false;
static mut WHEEL_DELTA: i32 = 0;

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
unsafe fn mouse_cmd(v: u8) { ctl(0xD4); wr(v); let _ = rd(); /* ACK 0xFA */ }

unsafe fn set_sample_rate(rate: u8) {
    mouse_cmd(0xF3);
    mouse_cmd(rate);
}

/// Active la souris et l'IRQ12. A appeler en entrant dans le bureau.

pub fn init() {
    // Le 8042 est partage avec le clavier. Les ACK et les reponses du
    // controleur passent par 0x60 : transaction atomique vis-a-vis d'IRQ1/12.
    interrupts::without_interrupts(|| unsafe {
        ctl(0xAE); // clavier actif
        ctl(0xA8); // souris active

        ctl(0x20);
        let mut status = rd();
        status |= 0x03;   // IRQ1 + IRQ12
        status |= 0x40;   // traduction Set-1 clavier
        status &= !0x10;  // horloge clavier active
        status &= !0x20;  // horloge souris active
        ctl(0x60);
        wr(status);

        mouse_cmd(0xF6);
        set_sample_rate(200);
        set_sample_rate(100);
        set_sample_rate(80);
        mouse_cmd(0xF2);
        let id = rd();
        HAS_WHEEL = id == 3 || id == 4;
        mouse_cmd(0xF4);

        crate::arch::x86_64::interrupts::unmask_irq(1);
        crate::arch::x86_64::interrupts::unmask_irq(12);

        MX = (WIDTH / 2) as i32;
        MY = (HEIGHT / 2) as i32;
        CYCLE = 0;
        WHEEL_DELTA = 0;

        crate::kernel::dmesg::log_fmt(format_args!(
            "mouse: 8042 config={:#04x}, id={}, wheel={}",
            status, id, HAS_WHEEL
        ));
    });

    // F4 cote clavier APRES toute la negociation IntelliMouse.
    crate::drivers::keyboard::rearm_after_8042_reconfigure();
}

/// Traite un octet recu de la souris (appele depuis l'IRQ12).
pub fn handle_byte(b: u8) {
    unsafe {
        match CYCLE {
            0 => {
                if b & 0x08 == 0 { return; } // bit de synchro absent : on resync
                PKT[0] = b;
                CYCLE = 1;
            }
            1 => { PKT[1] = b; CYCLE = 2; }
            2 if HAS_WHEEL => {
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
            _ => {}
        }
    }
}

unsafe fn apply_packet(with_wheel: bool) {
    let flags = PKT[0];
    let dx = PKT[1] as i8 as i32;
    let dy = PKT[2] as i8 as i32;
    MX += dx;
    MY -= dy; // l'axe Y ecran est inverse
    if MX < 0 { MX = 0; }
    if MX >= WIDTH as i32 { MX = WIDTH as i32 - 1; }
    if MY < 0 { MY = 0; }
    if MY >= HEIGHT as i32 { MY = HEIGHT as i32 - 1; }
    BTN = flags & 0x07;
    if with_wheel {
        // En mode IntelliMouse 3 boutons, l'octet 4 est un delta signe.
        let wheel = PKT[3] as i8 as i32;
        WHEEL_DELTA += wheel;
        if wheel != 0 {
            crate::serial_println!(
                "[INPUT-WHEEL] raw={} dx={} dy={} x={} y={}",
                wheel, dx, dy, MX, MY,
            );
        }
    }
}

/// Position courante du curseur.
pub fn pos() -> (usize, usize) {
    unsafe { (MX as usize, MY as usize) }
}

/// Bouton gauche enfonce ?
pub fn left_down() -> bool {
    unsafe { BTN & 0x01 != 0 }
}

/// Etat brut des boutons : bit 0 gauche, bit 1 droit, bit 2 milieu.
///
/// Meme codage que les bits de drapeau du paquet PS/2, ce qui permet a la
/// couche evdev de detecter les changements par un simple XOR.
pub fn buttons() -> u8 {
    unsafe { BTN & 0x07 }
}

/// Un delta de molette est-il en attente ? (consultation sans consommation)
pub fn wheel_pending() -> bool {
    unsafe { WHEEL_DELTA != 0 }
}

/// Delta de roulette accumule depuis le dernier appel.
pub fn take_wheel() -> i32 {
    unsafe {
        let d = WHEEL_DELTA;
        WHEEL_DELTA = 0;
        d
    }
}
