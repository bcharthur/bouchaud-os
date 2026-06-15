//! Historique des commandes du shell.
//!
//! Tampon circulaire a taille fixe (aucune allocation dynamique), sur le meme
//! principe que `kernel::dmesg`. Chaque commande est aussi recopiee sur la
//! sortie serie COM1 : avec QEMU lance en `-serial stdio`, on obtient un
//! transcript complet de la session, facile a copier/coller et a partager.

use crate::serial_println;

const HIST_MAX: usize = 64;
const HIST_LEN: usize = 128;

struct History {
    entries: [[u8; HIST_LEN]; HIST_MAX],
    lens: [usize; HIST_MAX],
    /// Nombre d'entrees stockees (plafonne a HIST_MAX).
    count: usize,
    /// Nombre total de commandes vues depuis le boot (pour la numerotation).
    total: usize,
}

static mut HISTORY: History = History {
    entries: [[0; HIST_LEN]; HIST_MAX],
    lens: [0; HIST_MAX],
    count: 0,
    total: 0,
};

impl History {
    fn push(&mut self, line: &str) {
        let index = if self.count < HIST_MAX { self.count } else { HIST_MAX - 1 };
        if self.count >= HIST_MAX {
            for i in 1..HIST_MAX {
                self.entries[i - 1] = self.entries[i];
                self.lens[i - 1] = self.lens[i];
            }
        }
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() && i < HIST_LEN {
            self.entries[index][i] = bytes[i];
            i += 1;
        }
        self.lens[index] = i;
        if self.count < HIST_MAX { self.count += 1; }
        self.total += 1;
    }

    fn print(&self) {
        // Numerotation continue : meme apres rotation du tampon, les numeros
        // restent coherents avec l'ordre de frappe depuis le boot.
        let first = self.total - self.count;
        for i in 0..self.count {
            crate::print!("{:>4}  ", first + i + 1);
            for j in 0..self.lens[i] {
                crate::print!("{}", self.entries[i][j] as char);
            }
            crate::println!("");
        }
    }
}

/// Enregistre une commande : tampon historique + transcript serie COM1.
pub fn record(line: &str) {
    unsafe {
        HISTORY.push(line);
        serial_println!("$ {}", line);
    }
}

/// Affiche l'historique numerote (commande `history`).
pub fn print() {
    unsafe { HISTORY.print(); }
}

/// Vide l'historique (commande `history clear`).
pub fn clear() {
    unsafe {
        HISTORY.count = 0;
        HISTORY.total = 0;
    }
    serial_println!("[history] efface");
}

/// Nombre total de commandes saisies depuis le boot.
pub fn total() -> usize {
    unsafe { HISTORY.total }
}

/// Nombre d'entrees actuellement memorisees.
pub fn len() -> usize {
    unsafe { HISTORY.count }
}

/// Renvoie la n-ieme commande en partant de la plus recente (0 = derniere).
pub fn nth_recent(n: usize) -> Option<&'static str> {
    unsafe {
        if n >= HISTORY.count { return None; }
        let idx = HISTORY.count - 1 - n;
        Some(core::str::from_utf8_unchecked(&HISTORY.entries[idx][..HISTORY.lens[idx]]))
    }
}
