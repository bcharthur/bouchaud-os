//! Journal noyau (`dmesg`) : tampon circulaire d'evenements a taille fixe.
//!
//! Aucune allocation dynamique : les messages sont stockes dans des tableaux
//! statiques. Chaque appel a `log` est aussi recopie sur la sortie serie COM1
//! afin de tracer le boot dans QEMU.

use crate::serial_println;

const DMESG_MAX: usize = 64;
const DMESG_LEN: usize = 96;

struct Dmesg {
    entries: [[u8; DMESG_LEN]; DMESG_MAX],
    lens: [usize; DMESG_MAX],
    count: usize,
}

static mut DMESG: Dmesg = Dmesg {
    entries: [[0; DMESG_LEN]; DMESG_MAX],
    lens: [0; DMESG_MAX],
    count: 0,
};

impl Dmesg {
    fn push(&mut self, msg: &str) {
        let index = if self.count < DMESG_MAX { self.count } else { DMESG_MAX - 1 };
        if self.count >= DMESG_MAX {
            for i in 1..DMESG_MAX {
                self.entries[i - 1] = self.entries[i];
                self.lens[i - 1] = self.lens[i];
            }
        }
        let bytes = msg.as_bytes();
        let mut i = 0;
        while i < bytes.len() && i < DMESG_LEN {
            self.entries[index][i] = bytes[i];
            i += 1;
        }
        self.lens[index] = i;
        if self.count < DMESG_MAX { self.count += 1; }
    }

    fn print(&self) {
        for i in 0..self.count {
            crate::print!("[{:02}] ", i);
            for j in 0..self.lens[i] {
                crate::print!("{}", self.entries[i][j] as char);
            }
            crate::println!("");
        }
    }
}

/// Reinitialise le journal noyau.
pub fn init() {
    unsafe {
        DMESG.count = 0;
        DMESG.push("dmesg: tampon circulaire initialise");
    }
    serial_println!("[dmesg] tampon circulaire initialise");
}

/// Enregistre un evenement noyau : tampon `dmesg` + sortie serie COM1.
pub fn log(msg: &str) {
    unsafe { DMESG.push(msg); }
    serial_println!("[kernel] {}", msg);
}

/// Affiche tout le journal noyau (commande `dmesg`).
pub fn print() {
    unsafe { DMESG.print(); }
}

/// Nombre d'evenements actuellement journalises.
pub fn count() -> usize {
    unsafe { DMESG.count }
}
