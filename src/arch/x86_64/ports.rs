//! Acces aux ports d'E/S x86 (instructions `in` / `out`).

use core::arch::asm;

/// Lit un octet depuis un port d'E/S.
pub unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    asm!("in al, dx", in("dx") port, out("al") value, options(nomem, nostack, preserves_flags));
    value
}

/// Ecrit un octet sur un port d'E/S.
pub unsafe fn outb(port: u16, value: u8) {
    asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
}
