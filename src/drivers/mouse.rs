//! Pilote souris PS/2 (port auxiliaire du controleur 8042), pilote par IRQ12.
//!
//! Initialise a l'entree du bureau graphique. Le gestionnaire d'IRQ12 (voir
//! `arch::x86_64::idt`) transmet chaque octet a `handle_byte`, qui reconstitue
//! les paquets de 3 octets et met a jour la position et les boutons.

use crate::arch::x86_64::ports::{inb, outb};
use crate::drivers::gfx::{WIDTH, HEIGHT};

static mut MX: i32 = (WIDTH / 2) as i32;
static mut MY: i32 = (HEIGHT / 2) as i32;
static mut BTN: u8 = 0;
static mut CYCLE: u8 = 0;
static mut PKT: [u8; 3] = [0; 3];

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
        mouse_cmd(0xF4); // active le reporting

        // Demasque IRQ2 (cascade vers le PIC esclave) et IRQ12 (souris).
        let m1 = inb(0x21);
        outb(0x21, m1 & !(1 << 2));
        let m2 = inb(0xA1);
        outb(0xA1, m2 & !(1 << 4)); // IRQ12 = bit 4 du PIC esclave

        MX = (WIDTH / 2) as i32;
        MY = (HEIGHT / 2) as i32;
        CYCLE = 0;
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
            2 => {
                PKT[2] = b;
                CYCLE = 0;
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
            }
            _ => {}
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
