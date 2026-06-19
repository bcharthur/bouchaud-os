//! Pilote souris PS/2 (port auxiliaire du controleur 8042), pilote par IRQ12.
//!
//! Initialise a l'entree du bureau graphique. Le gestionnaire d'IRQ12 (voir
//! `arch::x86_64::idt`) transmet chaque octet a `handle_byte`, qui reconstitue
//! les paquets de 3 octets (ou 4 octets avec molette IntelliMouse) et met a
//! jour la position, les boutons et le delta de roulette.

use crate::arch::x86_64::ports::{inb, outb};
use crate::drivers::gfx::{WIDTH, HEIGHT};

static mut MX: i32 = (WIDTH / 2) as i32;
static mut MY: i32 = (HEIGHT / 2) as i32;
static mut BTN: u8 = 0;
static mut CYCLE: u8 = 0;
static mut PKT: [u8; 4] = [0; 4];
static mut HAS_WHEEL: bool = false;
static mut WHEEL_DELTA: i32 = 0;
static mut WHEEL_DESYNC: u8 = 0;

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
    unsafe {
        ctl(0xA8); // active le peripherique auxiliaire (souris)
        // Active la generation d'IRQ12 dans la config du controleur.
        ctl(0x20);
        let mut status = rd();
        status |= 0x02;  // IRQ12 active
        status &= !0x20; // horloge souris active
        ctl(0x60);
        wr(status);
        mouse_cmd(0xF6); // parametres par defaut
        // Sequence d'activation IntelliMouse : les souris compatibles passent
        // en mode paquet 4 octets et exposent la roulette avec l'ID 3 (ou 4).
        set_sample_rate(200);
        set_sample_rate(100);
        set_sample_rate(80);
        mouse_cmd(0xF2); // Get Device ID (ACK deja consomme par mouse_cmd)
        let id = rd();
        HAS_WHEEL = id == 3 || id == 4;
        mouse_cmd(0xF4); // active le reporting

        // Demasque IRQ2 (cascade vers le PIC esclave) et IRQ12 (souris).
        let m1 = inb(0x21);
        outb(0x21, m1 & !(1 << 2));
        let m2 = inb(0xA1);
        outb(0xA1, m2 & !(1 << 4)); // IRQ12 = bit 4 du PIC esclave

        MX = (WIDTH / 2) as i32;
        MY = (HEIGHT / 2) as i32;
        CYCLE = 0;
        WHEEL_DELTA = 0;
        WHEEL_DESYNC = 0;
    }
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
                // Si le peripherique a annonce l'ID molette mais continue en
                // paquets 3 octets, l'octet attendu comme roulette est en fait
                // le header du paquet suivant (bit de synchro 0x08). Sans ce
                // garde-fou, la suite est decalee et la roulette ressemble a
                // un deplacement horizontal de la souris.
                if b & 0x08 != 0 && b & 0xC0 == 0 {
                    apply_packet(false);
                    WHEEL_DESYNC = WHEEL_DESYNC.saturating_add(1);
                    if WHEEL_DESYNC >= 2 {
                        HAS_WHEEL = false;
                    }
                    PKT[0] = b;
                    CYCLE = 1;
                    return;
                }
                PKT[3] = b;
                CYCLE = 0;
                WHEEL_DESYNC = 0;
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
        WHEEL_DELTA += PKT[3] as i8 as i32;
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

/// Delta de roulette accumule depuis le dernier appel.
pub fn take_wheel() -> i32 {
    unsafe {
        let d = WHEEL_DELTA;
        WHEEL_DELTA = 0;
        d
    }
}
