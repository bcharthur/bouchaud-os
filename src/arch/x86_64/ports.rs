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

/// Lit un mot de 16 bits depuis un port d'E/S.
///
/// Les registres de donnees ATA et VBE sont larges de 16 bits : les lire octet
/// par octet donnerait des valeurs decalees.
pub unsafe fn inw(port: u16) -> u16 {
    let value: u16;
    asm!("in ax, dx", in("dx") port, out("ax") value, options(nomem, nostack, preserves_flags));
    value
}

/// Ecrit un mot de 16 bits sur un port d'E/S.
pub unsafe fn outw(port: u16, value: u16) {
    asm!("out dx, ax", in("dx") port, in("ax") value, options(nomem, nostack, preserves_flags));
}
