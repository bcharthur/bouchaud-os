//! Pilote série UART 16550 sur COM1 (port 0x3F8).
//!
//! V16.2 garde COM1 comme sortie de diagnostic mais retire son coût du chemin
//! critique autant que possible :
//! - 115200 bauds au lieu de 38400 ;
//! - FIFO 16 octets ;
//! - un tampon de formatage sur pile pour que `write_fmt` n'attende pas THRE à
//!   chaque fragment de `fmt` ;
//! - le préfixe de journal peut être émis d'un seul bloc sans réentrer dans le
//!   formateur série.

use core::fmt;
use crate::arch::x86_64::ports::{inb, outb};

const COM1: u16 = 0x3F8;
const PROFONDEUR_FIFO: usize = 16;
const FORMAT_BUFFER_SIZE: usize = 2048;

/// État global du port série, pour éviter d'écrire avant l'init.
static mut INITIALISED: bool = false;

/// Le prochain octet normal écrit commence-t-il une ligne ?
static mut DEBUT_LIGNE: bool = true;
/// Garde de réentrance pendant la génération du préfixe.
static mut DANS_PREFIXE: bool = false;

pub struct SerialPort;
pub struct SerialBrut;

static mut SERIAL_BRUT: SerialBrut = SerialBrut;

/// Initialise COM1 : 115200 bauds, 8N1, FIFO active.
///
/// QEMU émule un 16550 ; un diviseur de 1 est le débit standard maximal du
/// périphérique et divise par trois la durée de vidage par rapport à l'ancien
/// diviseur 3 (38400 bauds).
pub fn init() {
    unsafe {
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x80);
        outb(COM1 + 0, 0x01); // diviseur bas : 1 -> 115200
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x03); // 8N1
        outb(COM1 + 2, 0xC7); // FIFO active, purge, seuil RX 14
        outb(COM1 + 4, 0x0B);
        INITIALISED = true;
    }
}

pub fn is_ready() -> bool {
    unsafe { INITIALISED }
}

#[inline]
fn transmit_empty() -> bool {
    unsafe { inb(COM1 + 5) & 0x20 != 0 }
}

#[inline]
fn attends_place() {
    let mut spin = 0u32;
    while !transmit_empty() {
        spin = spin.wrapping_add(1);
        if spin > 100_000 {
            break;
        }
        core::hint::spin_loop();
    }
}

fn write_lot(octets: &[u8]) {
    let mut pose = 0usize;
    while pose < octets.len() {
        attends_place();
        let fin = (pose + PROFONDEUR_FIFO).min(octets.len());
        while pose < fin {
            unsafe { outb(COM1, octets[pose]); }
            pose += 1;
        }
    }
}

/// Écrit directement sur COM1 sans déclencher de préfixe de journal.
///
/// Utilisé par `journal::ecris_prefixe`: le préfixe est déjà entièrement
/// construit sur pile, le refaire passer par `ecris_octets` récursivement
/// recréerait précisément le coût que V16.2 veut supprimer.
pub fn ecris_octets_sans_prefixe(octets: &[u8]) {
    if !is_ready() || octets.is_empty() {
        return;
    }
    let mut lot = [0u8; 64];
    super::lots::en_lots(octets, &mut lot, write_lot);
    if let Some(&dernier) = octets.last() {
        unsafe { DEBUT_LIGNE = dernier == b'\n'; }
    }
}

/// Émet des octets normaux, préfixe de journal compris.
pub fn ecris_octets(octets: &[u8]) {
    if !is_ready() || octets.is_empty() {
        return;
    }

    let mut lot = [0u8; 64];
    let mut debut = 0usize;

    for (indice, &byte) in octets.iter().enumerate() {
        unsafe {
            if DEBUT_LIGNE && byte != b'\n' && !DANS_PREFIXE {
                super::lots::en_lots(&octets[debut..indice], &mut lot, write_lot);
                debut = indice;

                DANS_PREFIXE = true;
                crate::kernel::journal::ecris_prefixe();
                DANS_PREFIXE = false;
            }
            DEBUT_LIGNE = byte == b'\n';
        }
    }

    super::lots::en_lots(&octets[debut..], &mut lot, write_lot);
}

// BOUCHAUD_V16_2_SERIAL_FORMAT_BUFFER
//
// `fmt::write` appelle `write_str` plusieurs fois pour une seule ligne
// formatée. L'ancien `SerialPort::write_str` descendait jusqu'au UART à chaque
// fragment ; sous TCG, chaque vérification THRE est un I/O émulé très coûteux.
// Ce tampon sur pile transforme une ligne ordinaire en un ou quelques gros
// envois, sans allocation et sans état global supplémentaire.
struct TamponFormat {
    donnees: [u8; FORMAT_BUFFER_SIZE],
    len: usize,
}

impl TamponFormat {
    const fn neuf() -> Self {
        Self { donnees: [0; FORMAT_BUFFER_SIZE], len: 0 }
    }

    fn vide(&mut self) {
        if self.len == 0 {
            return;
        }
        ecris_octets(&self.donnees[..self.len]);
        self.len = 0;
    }
}

impl fmt::Write for TamponFormat {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let mut reste = s.as_bytes();
        while !reste.is_empty() {
            if self.len == self.donnees.len() {
                self.vide();
            }
            let place = self.donnees.len() - self.len;
            let n = place.min(reste.len());
            self.donnees[self.len..self.len + n].copy_from_slice(&reste[..n]);
            self.len += n;
            reste = &reste[n..];
        }
        Ok(())
    }
}

impl fmt::Write for SerialPort {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        ecris_octets(s.as_bytes());
        Ok(())
    }
}

/// Implémentation derrière `serial_print!` / `serial_println!`.
pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;
    if !is_ready() {
        return;
    }

    let mut sortie = TamponFormat::neuf();
    let _ = sortie.write_fmt(args);
    sortie.vide();
}

impl fmt::Write for SerialBrut {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        ecris_octets_sans_prefixe(s.as_bytes());
        Ok(())
    }
}

/// Sortie brute de forensic/panique : aucun préfixe.
///
/// Le chemin de panique privilégie la simplicité : il ne dépend ni du journal
/// ni du tampon de formatage normal.
pub fn _print_raw(args: fmt::Arguments) {
    use core::fmt::Write;
    if !is_ready() {
        return;
    }
    unsafe {
        let _ = SERIAL_BRUT.write_fmt(args);
    }
}

#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => {{
        $crate::drivers::serial::_print(format_args!($($arg)*))
    }};
}

#[macro_export]
macro_rules! serial_print_brut {
    ($($arg:tt)*) => {{
        $crate::drivers::serial::_print_raw(format_args!($($arg)*))
    }};
}

#[macro_export]
macro_rules! serial_println_brut {
    () => {{ $crate::serial_print_brut!("\n") }};
    ($fmt:expr) => {{ $crate::serial_print_brut!(concat!($fmt, "\n")) }};
    ($fmt:expr, $($arg:tt)*) => {{
        $crate::serial_print_brut!(concat!($fmt, "\n"), $($arg)*)
    }};
}

#[macro_export]
macro_rules! serial_println {
    () => {{ $crate::serial_print!("\n") }};
    ($fmt:expr) => {{ $crate::serial_print!(concat!($fmt, "\n")) }};
    ($fmt:expr, $($arg:tt)*) => {{
        $crate::serial_print!(concat!($fmt, "\n"), $($arg)*)
    }};
}
