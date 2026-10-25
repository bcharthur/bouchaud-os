//! Journal noyau (`dmesg`) : tampon circulaire d'evenements a taille fixe.
//!
//! Aucune allocation dynamique : les messages sont stockes dans des tableaux
//! statiques. Chaque appel a `log` est aussi recopie sur la sortie serie COM1
//! afin de tracer le boot dans QEMU.

use crate::serial_println;
use crate::kernel::sync::SpinLockIrq;

const DMESG_MAX: usize = 64;
const DMESG_LEN: usize = 96;

#[derive(Clone)]
struct Dmesg {
    entries: [[u8; DMESG_LEN]; DMESG_MAX],
    lens: [usize; DMESG_MAX],
    count: usize,
    next: usize,
}

static DMESG: SpinLockIrq<Dmesg> = SpinLockIrq::new(Dmesg {
    entries: [[0; DMESG_LEN]; DMESG_MAX],
    lens: [0; DMESG_MAX],
    count: 0,
    next: 0,
});

impl Dmesg {
    fn push(&mut self, msg: &str) {
        // Une seule ligne copiee sous IF masque, meme apres saturation.
        // L'ancien decalage de 63 lignes n'est pas necessaire a un anneau.
        let index = self.next;
        let bytes = msg.as_bytes();
        let mut i = 0;
        while i < bytes.len() && i < DMESG_LEN {
            self.entries[index][i] = bytes[i];
            i += 1;
        }
        self.lens[index] = i;
        if self.count < DMESG_MAX { self.count += 1; }
        self.next = (index + 1) % DMESG_MAX;
    }

    fn print(&self) {
        for i in 0..self.count {
            let index = (self.next + DMESG_MAX - self.count + i) % DMESG_MAX;
            crate::print!("[{:02}] ", i);
            for j in 0..self.lens[index] {
                crate::print!("{}", self.entries[index][j] as char);
            }
            crate::println!("");
        }
    }
}

/// Reinitialise le journal noyau.
pub fn init() {
    {
        let mut journal = DMESG.lock();
        journal.count = 0;
        journal.next = 0;
        journal.push("dmesg: tampon circulaire initialise");
    }
    serial_println!("[dmesg] tampon circulaire initialise");
}

/// Enregistre un evenement noyau : tampon `dmesg` + sortie serie COM1.
pub fn log(msg: &str) {
    // Rendre le verrou et restaurer IF AVANT la console, qui peut attendre.
    DMESG.lock().push(msg);
    serial_println!("[kernel] {}", msg);
}

/// Variante formatee de [`log`] (`log_fmt(format_args!("..."))`).
///
/// Necessite le tas : a n'utiliser qu'apres `heap::init`.
pub fn log_fmt(args: core::fmt::Arguments) {
    log(&alloc::format!("{}", args));
}

/// Affiche tout le journal noyau (commande `dmesg`).
pub fn print() {
    // Instantane coherent (6,7 Kio sur la pile), aucune impression sous
    // verrou. Pas d'allocation ni de reference vers un tampon mutable.
    let instantane = DMESG.lock().clone();
    instantane.print();
}

/// Nombre d'evenements actuellement journalises.
pub fn count() -> usize {
    DMESG.lock().count
}
