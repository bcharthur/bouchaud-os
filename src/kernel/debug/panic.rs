//! Handler de panique noyau.
//!
//! Le chemin panic ne prend aucun verrou. Le premier CPU paniqueur vide d'abord
//! les enregistreurs atomiques, puis produit le contexte riche et arrête SMP.

use core::fmt::{self, Write};
use core::panic::PanicInfo;
use crate::arch::x86_64::{idt, smp};
use crate::drivers::vga;
use crate::serial_println;

/// Le message de panique, formate SUR LA PILE.
///
/// Le chemin de panique ne peut pas allouer : il y arrive parfois PARCE QUE
/// l'allocation a echoue, et rappeler l'allocateur ferait paniquer dans le
/// handler de panique -- ou plus rien ne s'affiche.
struct MessagePanique {
    octets: [u8; 192],
    n: usize,
}

impl MessagePanique {
    const fn neuf() -> Self {
        Self { octets: [0; 192], n: 0 }
    }

    fn texte(&self) -> &str {
        core::str::from_utf8(&self.octets[..self.n]).unwrap_or("<message illisible>")
    }
}

impl Write for MessagePanique {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &octet in s.as_bytes() {
            if self.n >= self.octets.len() {
                break;
            }
            // Un octet de continuation tronque rendrait tout le message
            // illisible a `from_utf8` : on coupe sur une frontiere.
            self.octets[self.n] = octet;
            self.n += 1;
        }
        while self.n > 0 && core::str::from_utf8(&self.octets[..self.n]).is_err() {
            self.n -= 1;
        }
        Ok(())
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    unsafe { core::arch::asm!("cli", options(nomem, nostack)) };

    let cpu = smp::cpu_index();
    if !idt::prends_la_panique(cpu) {
        idt::arret_definitif();
    }

    // LE MESSAGE, FORMATE SANS ALLOUER.
    //
    // La panique du 17 septembre venait de `handle_alloc_error` : le tas a
    // refuse une allocation. Formater son message avec `alloc` rappellerait
    // l'allocateur qui vient d'echouer -- et une panique dans le handler de
    // panique n'affiche plus rien du tout. Ce tampon vit sur la pile.
    let mut message = MessagePanique::neuf();
    let _ = write!(&mut message, "{}", info.message());

    // L'ecran GOP d'abord. `println!` va dans le tampon texte VGA, que le
    // materiel de reference -- demarre en UEFI, sans mode texte -- n'affiche
    // pas : sans cet appel, une panique noyau y est indiscernable d'un gel.
    match info.location() {
        Some(lieu) => crate::platform::pc::ecran_faute::affiche_panique(
            lieu.file(),
            lieu.line(),
            Some(message.texte()),
        ),
        None => crate::platform::pc::ecran_faute::affiche_panique("?", 0, Some(message.texte())),
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

    // LA TRACE PART SUR LA CLE, PARCE QUE COM1 N'EXISTE PAS ICI.
    //
    // Tout ce qui precede va sur COM1. La machine de reference n'en a pas : la
    // panique du 17 septembre n'a donc laisse que huit lignes a l'ecran, et
    // l'archive ne contenait pas un mot de la session. Quatre sessions passees
    // a rendre l'enregistreur increvable, et le seul evenement qu'il devait
    // absolument retenir ne lui etait jamais confie.
    //
    // `fatal_best_effort` pose l'enregistrement EN MEMOIRE d'abord -- ce qui ne
    // peut pas echouer -- puis tente un vidage borne vers la cle. Le budget est
    // borne : une panique qui attendrait sans fin remplacerait un diagnostic
    // par un ecran noir.
    let lieu = info.location();
    crate::kernel::blackbox::panique(
        cpu,
        lieu.map(|l| l.file()).unwrap_or("?"),
        lieu.map(|l| l.line()).unwrap_or(0),
        message.texte(),
    );

    smp::arrete_les_autres_cpu();
    idt::arret_definitif();
}
