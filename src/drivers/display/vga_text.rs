//! Pilote VGA texte (buffer memoire 0xb8000, 80x25).
//!
//! Fournit le writer global utilise par les macros `print!` / `println!` ainsi
//! que la gestion des couleurs et du defilement.

use core::fmt;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use crate::arch::x86_64::ports::outb;
use crate::kernel::sync::SpinLockIrq;

const VGA_BUFFER: usize = 0xb8000;
const VGA_WIDTH: usize = 80;
const VGA_HEIGHT: usize = 25;

/// Dimensions de l'ecran texte, exposees pour l'editeur plein ecran.
pub const WIDTH: usize = VGA_WIDTH;
pub const HEIGHT: usize = VGA_HEIGHT;

pub const COLOR_DEFAULT: u8 = 0x0f;
pub const COLOR_GREEN: u8 = 0x0a;
pub const COLOR_CYAN: u8 = 0x0b;
pub const COLOR_RED: u8 = 0x0c;
pub const COLOR_YELLOW: u8 = 0x0e;

pub struct VgaWriter {
    row: usize,
    col: usize,
    color: u8,
}

// BOUCHAUD_CONSOLE_VERROU_V1
//
// L'ecrivain VGA et la pile de captures etaient deux `static mut`. Le gros
// verrou du noyau ne les protegeait qu'a moitie : `write(1)` d'un programme le
// prenait, le bureau le tenait, mais chaque fil noyau qui fait un `println!`
// (services reseau, echantillonneur, ...) ecrivait deja sans lui -- et pouvait
// pousser dans le `String` de capture pendant qu'un autre coeur le faisait
// grandir, ou `pop` la pile sous ses pieds.
//
// Les deux vivent maintenant sous un seul verrou a interruptions masquees
// (un `println!` peut partir d'un gestionnaire). La prise est BORNEE : un
// coeur qui panique en tenant la console, puis y reecrit, perdrait sinon la
// machine dans une attente sans fin ; il perd seulement la copie VGA de son
// message, que COM1 et l'ecran GOP recoivent de toute facon.
struct Console {
    vga: VgaWriter,
    /// Pile de tampons de capture. Quand elle n'est pas vide, la sortie texte
    /// est ecrite dans le tampon du sommet au lieu d'aller a l'ecran. La pile
    /// permet d'imbriquer redirections (`>`) et pipes (`|`).
    captures: alloc::vec::Vec<alloc::string::String>,
}

static CONSOLE: SpinLockIrq<Console> = SpinLockIrq::new(Console {
    vga: VgaWriter {
        row: 0,
        col: 0,
        color: COLOR_DEFAULT,
    },
    captures: alloc::vec::Vec::new(),
});

/// Tentatives de prise avant d'abandonner une ecriture de console.
const PRISE_CONSOLE_TENTATIVES: u32 = 1 << 22;

/// Ecritures de console abandonnees faute d'avoir pu prendre le verrou.
static CONSOLE_ABANDONS: AtomicU64 = AtomicU64::new(0);

/// Execute `f` sous le verrou de la console. `None` si la prise a echoue apres
/// `PRISE_CONSOLE_TENTATIVES` essais : voir BOUCHAUD_CONSOLE_VERROU_V1.
fn avec_console<R>(f: impl FnOnce(&mut Console) -> R) -> Option<R> {
    for _ in 0..PRISE_CONSOLE_TENTATIVES {
        if let Some(mut console) = CONSOLE.try_lock() {
            return Some(f(&mut console));
        }
        core::hint::spin_loop();
    }
    CONSOLE_ABANDONS.fetch_add(1, Ordering::Relaxed);
    None
}

/// Ecritures de console abandonnees depuis le demarrage (diagnostic).
pub fn abandons_console() -> u64 {
    CONSOLE_ABANDONS.load(Ordering::Relaxed)
}

impl VgaWriter {
    fn clear(&mut self) {
        for row in 0..VGA_HEIGHT {
            for col in 0..VGA_WIDTH {
                self.write_cell(row, col, b' ', self.color);
            }
        }
        self.row = 0;
        self.col = 0;
    }

    fn set_color(&mut self, color: u8) {
        self.color = color;
    }

    fn write_cell(&self, row: usize, col: usize, byte: u8, color: u8) {
        #[cfg(feature = "reference-desktop")]
        {
            // BOUCHAUD_STAGE2_NO_LEGACY_VGA_MMIO
            // UEFI/GOP: 0xb8000 n'est pas un backend valide du reference device.
            let _ = (row, col, byte, color);
            return;
        }

        #[cfg(not(feature = "reference-desktop"))]
        {
            let offset = (row * VGA_WIDTH + col) * 2;
            unsafe {
                let ptr = (VGA_BUFFER + offset) as *mut u8;
                core::ptr::write_volatile(ptr, byte);
                core::ptr::write_volatile(ptr.add(1), color);
            }
        }
    }

    fn newline(&mut self) {
        self.col = 0;
        if self.row + 1 >= VGA_HEIGHT {
            self.scroll();
        } else {
            self.row += 1;
        }
    }

    fn scroll(&mut self) {
        for row in 1..VGA_HEIGHT {
            for col in 0..VGA_WIDTH {
                let src_offset = (row * VGA_WIDTH + col) * 2;
                let dst_offset = ((row - 1) * VGA_WIDTH + col) * 2;
                unsafe {
                    let src = (VGA_BUFFER + src_offset) as *const u8;
                    let dst = (VGA_BUFFER + dst_offset) as *mut u8;
                    let ch = core::ptr::read_volatile(src);
                    let color = core::ptr::read_volatile(src.add(1));
                    core::ptr::write_volatile(dst, ch);
                    core::ptr::write_volatile(dst.add(1), color);
                }
            }
        }
        for col in 0..VGA_WIDTH {
            self.write_cell(VGA_HEIGHT - 1, col, b' ', self.color);
        }
        self.row = VGA_HEIGHT - 1;
        self.col = 0;
    }

    fn backspace(&mut self) {
        if self.col > 0 {
            self.col -= 1;
            self.write_cell(self.row, self.col, b' ', self.color);
        }
    }

    fn byte(&mut self, byte: u8) {
        match byte {
            b'\n' => self.newline(),
            8 => self.backspace(),
            byte => {
                if self.col >= VGA_WIDTH {
                    self.newline();
                }
                self.write_cell(self.row, self.col, byte, self.color);
                self.col += 1;
            }
        }
    }
}

impl fmt::Write for VgaWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            match byte {
                0x08 | 0x20..=0x7e | b'\n' => self.byte(byte),
                _ => self.byte(b'?'),
            }
        }
        Ok(())
    }
}

/// Efface l'ecran et replace le curseur en haut a gauche.
pub fn clear() {
    avec_console(|console| {
        #[cfg(feature = "reference-desktop")]
        {
            console.vga.row = 0;
            console.vga.col = 0;
        }

        #[cfg(not(feature = "reference-desktop"))]
        console.vga.clear();
    });
}

/// Change la couleur d'affichage courante.
pub fn set_color(color: u8) {
    avec_console(|console| console.vga.set_color(color));
}

/// Positionne le curseur (texte + curseur materiel) sur (row, col).
pub fn set_cursor(row: usize, col: usize) {
    avec_console(|console| {
        let vga = &mut console.vga;
        vga.row = if row >= VGA_HEIGHT { VGA_HEIGHT - 1 } else { row };
        vga.col = if col >= VGA_WIDTH { VGA_WIDTH - 1 } else { col };

        #[cfg(not(feature = "reference-desktop"))]
        {
            let pos = vga.row * VGA_WIDTH + vga.col;
            unsafe {
                outb(0x3D4, 0x0F);
                outb(0x3D5, (pos & 0xFF) as u8);
                outb(0x3D4, 0x0E);
                outb(0x3D5, ((pos >> 8) & 0xFF) as u8);
            }
        }
    });
}

/// Profondeur de journalisation d'une commande de terminal.
static TERMINAL_TRACE_DEPTH: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

pub fn terminal_trace_begin() {
    TERMINAL_TRACE_DEPTH.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
}

pub fn terminal_trace_end() {
    let _ = TERMINAL_TRACE_DEPTH.fetch_update(
        core::sync::atomic::Ordering::Relaxed,
        core::sync::atomic::Ordering::Relaxed,
        |depth| Some(depth.saturating_sub(1)),
    );
}

fn terminal_trace_active() -> bool {
    TERMINAL_TRACE_DEPTH.load(core::sync::atomic::Ordering::Relaxed) != 0
}


/// Demarre une capture (empile un tampon vide).
pub fn capture_start() {
    avec_console(|console| console.captures.push(alloc::string::String::new()));
}

/// Termine la capture courante et renvoie le texte accumule.
pub fn capture_take() -> Option<alloc::string::String> {
    avec_console(|console| console.captures.pop()).flatten()
}

/// Recopie sur COM1 tout ce qui part a l'ecran.
///
/// Sert au mode non interactif : la sortie des commandes doit atteindre l'hote
/// pour qu'il puisse l'analyser. La recopie se fait au fil de l'eau et non a la
/// fin, de sorte qu'une panique laisse quand meme voir tout ce qui l'a precedee.
static SERIAL_MIRROR: AtomicBool = AtomicBool::new(false);

/// Active ou coupe la recopie de la sortie texte vers COM1.
pub fn set_serial_mirror(on: bool) {
    SERIAL_MIRROR.store(on, Ordering::Relaxed);
}

/// La sortie texte est-elle recopiee sur COM1 ?
pub fn serial_mirror() -> bool {
    SERIAL_MIRROR.load(Ordering::Relaxed)
}

/// Implementation reelle derriere les macros `print!` / `println!`.
pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;

    // La blackbox voit la sortie AVANT la capture VGA/pipeline.
    if terminal_trace_active() {
        crate::kernel::blackbox::terminal_sortie(args.clone());
    }

    // Les captures du shell restent prioritaires : les commandes du terminal
    // graphique continuent a reutiliser println! sans toucher au VGA physique.
    // Le test et l'ecriture se font sous la meme prise : un `capture_take`
    // concurrent ne peut pas retirer le tampon entre les deux.
    let capturee = avec_console(|console| match console.captures.last_mut() {
        Some(top) => {
            let _ = top.write_fmt(args);
            true
        }
        None => false,
    });
    if capturee == Some(true) {
        return;
    }

    #[cfg(feature = "reference-desktop")]
    {
        // UEFI/GOP: sortie texte de secours vers COM1 uniquement.
        crate::drivers::serial::_print(args);
        return;
    }

    #[cfg(not(feature = "reference-desktop"))]
    {
        // COM1 hors du verrou de la console : l'emission serie dure 87 us par
        // octet, et elle a deja son propre jeton.
        if serial_mirror() {
            crate::drivers::serial::_print(args);
        }
        avec_console(|console| {
            let _ = console.vga.write_fmt(args);
        });
    }
}
