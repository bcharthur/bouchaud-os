//! Pilote VGA texte (buffer memoire 0xb8000, 80x25).
//!
//! Fournit le writer global utilise par les macros `print!` / `println!` ainsi
//! que la gestion des couleurs et du defilement.

use core::fmt;

const VGA_BUFFER: usize = 0xb8000;
const VGA_WIDTH: usize = 80;
const VGA_HEIGHT: usize = 25;

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

static mut VGA: VgaWriter = VgaWriter {
    row: 0,
    col: 0,
    color: COLOR_DEFAULT,
};

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
        let offset = (row * VGA_WIDTH + col) * 2;
        unsafe {
            let ptr = (VGA_BUFFER + offset) as *mut u8;
            core::ptr::write_volatile(ptr, byte);
            core::ptr::write_volatile(ptr.add(1), color);
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
    unsafe { VGA.clear(); }
}

/// Change la couleur d'affichage courante.
pub fn set_color(color: u8) {
    unsafe { VGA.set_color(color); }
}

/// Tampon de capture optionnel : quand il est actif, la sortie texte y est
/// redirigee au lieu d'aller a l'ecran (utilise par les redirections `>`/`>>`).
static mut CAPTURE: Option<alloc::string::String> = None;

/// Demarre la capture de la sortie texte dans un tampon.
pub fn capture_start() {
    unsafe { CAPTURE = Some(alloc::string::String::new()); }
}

/// Termine la capture et renvoie le texte accumule.
pub fn capture_take() -> Option<alloc::string::String> {
    unsafe { CAPTURE.take() }
}

/// Implementation reelle derriere les macros `print!` / `println!`.
pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;
    unsafe {
        if let Some(ref mut buf) = CAPTURE {
            let _ = buf.write_fmt(args);
            return;
        }
        let _ = VGA.write_fmt(args);
    }
}
