//! Handler de panique noyau.
//!
//! Affiche l'erreur a l'ecran (VGA, en rouge) et sur la sortie serie COM1, puis
//! arrete le CPU. Avec `panic = "abort"` il n'y a pas de deroulement de pile.

use core::panic::PanicInfo;
use crate::arch::x86_64::cpu;
use crate::drivers::vga;
use crate::serial_println;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    vga::set_color(vga::COLOR_RED);
    println!("");
    println!("*** KERNEL PANIC ***");
    println!("{}", info);
    vga::set_color(vga::COLOR_DEFAULT);

    serial_println!("*** KERNEL PANIC ***");
    serial_println!("{}", info);

    cpu::halt_loop();
}
