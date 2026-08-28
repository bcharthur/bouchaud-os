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

// BOUCHAUD_SERIE_FIFO_V1
//
// # Ce que coutait un octet a la fois
//
// `write_raw` attendait THRE -- « registre de maintien vide » -- AVANT CHAQUE
// octet, puis en poussait un seul. Chaque tour de cette attente est un `inb`,
// c'est-a-dire une sortie du mode traduit sous TCG ; chaque octet en coutait
// donc au moins un, souvent plusieurs.
//
// Et `write` s'execute sous le gros verrou du noyau. Un programme qui ecrit
// quelques centaines d'octets sur sa sortie standard -- Ladybird en produit
// sans arret -- serialisait les quatre coeurs derriere un port serie, un octet
// a la fois. C'est ce que `[BKL-SYSCALL]` mesurait : `write`, jusqu'a 152 ms
// de detention pour un seul appel.
//
// # Ce que le 16550 permet
//
// Ce n'est pas un 8250 : il a un FIFO d'emission de seize octets, active a
// l'initialisation (`outb(COM1 + 2, 0xC7)`). Quand THRE monte, la place
// disponible n'est pas d'un octet mais de seize. Attendre une fois puis en
// pousser seize divise par seize le nombre d'attentes -- a debit serie
// rigoureusement identique, puisque c'est la ligne qui impose le debit, pas
// le pilote.
//
// Aucun octet n'est perdu ni reordonne : l'ordre d'ecriture dans le FIFO est
// l'ordre d'emission.
const PROFONDEUR_FIFO: usize = 16;

/// Attend que le registre de maintien se vide, avec le meme garde-fou qu'avant
/// pour le cas ou COM1 n'existe pas.
fn attends_place() {
    let mut spin = 0u32;
    while !transmit_empty() {
        spin += 1;
        if spin > 100_000 { break; }
    }
}

/// Pousse un lot d'octets deja convertis en CRLF.
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

/// Le prochain octet ecrit commence-t-il une ligne ?
static mut DEBUT_LIGNE: bool = true;
/// Vrai pendant l'ecriture du prefixe lui-meme (garde de reentrance).
static mut DANS_PREFIXE: bool = false;

/// Emet des octets sur COM1, prefixe de journal compris.
///
/// C'est le corps de `write_str`, extrait pour que `write(2)` puisse pousser la
/// sortie d'un programme SANS la faire passer par un formateur.
///
/// `console_write` ecrivait `serial_print!("{}", octet as char)` -- un
/// `write_fmt` complet par octet. Outre le cout, c'etait faux : un octet de
/// 0x80 a 0xFF devient un `char` Latin-1, que le formateur reencode en DEUX
/// octets UTF-8. Toute sortie UTF-8 d'un programme arrivait donc en mojibake
/// sur la console serie. Passer les octets tels quels est a la fois plus rapide
/// et correct.
pub fn ecris_octets(octets: &[u8]) {
    if !is_ready() { return }
    // Tampon d'etape, LOCAL : `ecris_prefixe` reentre ici, et une pile de
    // tampons locaux garde naturellement l'ordre -- le segment qui precede le
    // prefixe est deja emis quand la reentrance se produit.
    let mut lot = [0u8; 64];
    let mut debut = 0usize;

    for (indice, &byte) in octets.iter().enumerate() {
        // Le prefixe est pose au premier octet non vide d'une ligne, et non
        // a chaque appel : une ligne construite par plusieurs `serial_print!`
        // n'en recoit donc qu'un, et une ligne vide n'en recoit aucun.
        unsafe {
            if DEBUT_LIGNE && byte != b'\n' && !DANS_PREFIXE {
                // Emettre AVANT de reentrer : ce qui precede le prefixe a ete
                // ecrit avant lui.
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

impl fmt::Write for SerialPort {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        ecris_octets(s.as_bytes());
        Ok(())
    }
}

/// Implementation reelle derriere `serial_print!` / `serial_println!`.
pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;
    if !is_ready() { return; }
    unsafe { let _ = SERIAL.write_fmt(args); }
}

// BOUCHAUD_P0_SERIE_BRUTE_V1
//
// POURQUOI UNE SECONDE SORTIE
// ---------------------------
// `SerialPort` pose un prefixe de journal au premier octet de chaque ligne, et
// ce prefixe n'est pas gratuit : `journal::ecris_prefixe` lit l'horloge RTC par
// ports d'E/S, puis la charge CPU, la memoire et le disque.
//
// Ce sont des lectures parfaitement raisonnables pour un journal. Elles n'ont
// rien a faire dans le chemin de forensic d'une panique : a ce moment precis
// l'etat du noyau est, par definition, celui qu'on ne comprend pas. Le vidage
// de l'enregistreur de vol du gros verrou doit dependre du strict minimum --
// un `outb` sur COM1 -- et de rien d'autre.
//
// Ce que cette sortie ne fait pas : pas de prefixe, pas d'allocation, pas de
// verrou, aucun appel hors de ce fichier.
pub struct SerialBrut;

static mut SERIAL_BRUT: SerialBrut = SerialBrut;

impl fmt::Write for SerialBrut {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        // Chemin de panique : les memes lots, mais sans prefixe. Un octet a la
        // fois y coutait aussi cher qu'ailleurs, et une panique a d'autant plus
        // besoin d'etre ecrite vite qu'elle peut etre suivie d'un triple faute.
        //
        // Tenir `DEBUT_LIGNE` a jour malgre l'absence de prefixe : sinon un
        // `serial_println!` ordinaire qui suivrait une ligne brute croirait
        // etre en milieu de ligne, et sauterait SON prefixe.
        if let Some(dernier) = s.bytes().last() {
            unsafe { DEBUT_LIGNE = dernier == b'\n'; }
        }
        let mut lot = [0u8; 64];
        super::lots::en_lots(s.as_bytes(), &mut lot, write_lot);
        Ok(())
    }
}

/// Sortie serie brute : COM1 directement, sans prefixe de journal.
///
/// Reservee au chemin de diagnostic de panique. Pour tout le reste,
/// `serial_println!` reste la bonne macro : un journal sans horodatage est
/// beaucoup moins utile qu'il n'y parait.
pub fn _print_raw(args: fmt::Arguments) {
    use core::fmt::Write;
    if !is_ready() { return; }
    unsafe { let _ = SERIAL_BRUT.write_fmt(args); }
}

#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => {{ $crate::drivers::serial::_print(format_args!($($arg)*)) }};
}

/// Comme `serial_print!`, mais sans prefixe de journal. Voir [`_print_raw`].
#[macro_export]
macro_rules! serial_print_brut {
    ($($arg:tt)*) => {{ $crate::drivers::serial::_print_raw(format_args!($($arg)*)) }};
}

/// Comme `serial_println!`, mais sans prefixe de journal. Voir [`_print_raw`].
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
    ($fmt:expr, $($arg:tt)*) => {{ $crate::serial_print!(concat!($fmt, "\n"), $($arg)*) }};
}
