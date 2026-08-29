//! Handler de panique noyau.
//!
//! Le chemin panic ne prend aucun verrou. Le premier CPU paniqueur vide d'abord
//! les enregistreurs atomiques, puis produit le contexte riche et arrête SMP.

use core::panic::PanicInfo;
use crate::arch::x86_64::{idt, smp};
use crate::drivers::vga;
use crate::serial_println;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    unsafe { core::arch::asm!("cli", options(nomem, nostack)) };

    let cpu = smp::cpu_index();
    if !idt::prends_la_panique(cpu) {
        idt::arret_definitif();
    }

    vga::set_color(vga::COLOR_RED);
    println!("");
    println!("*** KERNEL PANIC ***");
    println!("{}", info);
    vga::set_color(vga::COLOR_DEFAULT);

    serial_println!("");
    serial_println!("======== [KERNEL PANIC] ========");
    serial_println!("*** KERNEL PANIC *** cpu={}", cpu);
    serial_println!("{}", info);

    // Bouchaud Performance Observatory : le ring ne lit que des atomiques.
    // Il passe avant tout relevé de structures riches potentiellement corrompues.
    crate::kernel::perf::dump_flight_recorder();

    // Enregistreur BKL existant, lui aussi conçu pour le chemin de panique.
    crate::kernel::smp_lock::vide_enregistreur();

    idt::releve_contexte_courant(cpu, None);
    serial_println!("======== fin du releve ========");

    smp::arrete_les_autres_cpu();
    idt::arret_definitif();
}
