//! Pilote serie UART 16550 sur COM1 (port 0x3F8).
//!
//! Sortie de debug pour QEMU lance avec `-serial stdio` : les logs noyau
//! importants y sont copies, ce qui permet de tracer le boot meme si l'ecran
//! VGA est efface. Fournit les macros `serial_print!` / `serial_println!`.

use core::fmt;
use crate::arch::x86_64::ports::{inb, outb};

const COM1: u16 = 0x3F8;

/// Etat global du port serie, pour eviter d'ecrire avant l'init.
static mut INITIALISED: bool = false;

pub struct SerialPort;

static mut SERIAL: SerialPort = SerialPort;

/// Initialise COM1 : 38400 bauds, 8N1, FIFO active.
pub fn init() {
    unsafe {
        outb(COM1 + 1, 0x00); // desactive les interruptions
        outb(COM1 + 3, 0x80); // active DLAB pour regler le diviseur
        outb(COM1 + 0, 0x03); // diviseur bas  (3 -> 38400 bauds)
        outb(COM1 + 1, 0x00); // diviseur haut
        outb(COM1 + 3, 0x03); // 8 bits, pas de parite, 1 stop (8N1)
        outb(COM1 + 2, 0xC7); // active et purge le FIFO, seuil 14 octets
        outb(COM1 + 4, 0x0B); // IRQ active, RTS/DSR positionnes
        INITIALISED = true;
    }
}

/// Indique si COM1 a ete initialise.
pub fn is_ready() -> bool {
    unsafe { INITIALISED }
}

fn transmit_empty() -> bool {
    unsafe { inb(COM1 + 5) & 0x20 != 0 }
}

fn write_byte(byte: u8) {
    // Convertit les sauts de ligne Unix en CRLF pour les terminaux serie.
    if byte == b'\n' {
        write_raw(b'\r');
    }
    write_raw(byte);
}

fn write_raw(byte: u8) {
    let mut spin = 0u32;
    while !transmit_empty() {
        spin += 1;
        if spin > 100_000 { break; } // garde-fou si COM1 absent
    }
    unsafe { outb(COM1, byte); }
}

impl fmt::Write for SerialPort {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            write_byte(byte);
        }
        Ok(())
    }
}

/// Implementation reelle derriere `serial_print!` / `serial_println!`.
pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;
    if !is_ready() { return; }
    unsafe { let _ = SERIAL.write_fmt(args); }
}

#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => {{ $crate::drivers::serial::_print(format_args!($($arg)*)) }};
}

#[macro_export]
macro_rules! serial_println {
    () => {{ $crate::serial_print!("\n") }};
    ($fmt:expr) => {{ $crate::serial_print!(concat!($fmt, "\n")) }};
    ($fmt:expr, $($arg:tt)*) => {{ $crate::serial_print!(concat!($fmt, "\n"), $($arg)*) }};
}
