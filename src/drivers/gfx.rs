//! Pilote graphique VGA mode 13h (320x200, 256 couleurs, framebuffer 0xA0000).
//!
//! On reprogramme les registres VGA pour passer du mode texte au mode graphique,
//! avec un double-buffer en memoire (via `alloc`) pour eviter le scintillement.
//! Fournit primitives (pixels, rectangles) et rendu de texte (police 8x8).

use alloc::vec;
use alloc::vec::Vec;
use crate::arch::x86_64::ports::{inb, outb};

pub const WIDTH: usize = 320;
pub const HEIGHT: usize = 200;
const FB: usize = 0xA0000;

// Palette (index -> couleur), reglee via le DAC. Quelques couleurs utiles.
pub const C_BLACK: u8 = 0;
pub const C_WHITE: u8 = 1;
pub const C_GRAY: u8 = 2;
pub const C_DKGRAY: u8 = 3;
pub const C_BLUE: u8 = 4;
pub const C_DKBLUE: u8 = 5;
pub const C_GREEN: u8 = 6;
pub const C_RED: u8 = 7;
pub const C_CYAN: u8 = 8;
pub const C_YELLOW: u8 = 9;
pub const C_DESKTOP: u8 = 10; // bleu bureau
pub const C_TITLE: u8 = 11;   // barre de titre

/// (r,g,b) sur 0..63 (DAC 6 bits) pour chaque index utilise.
const PALETTE: &[(u8, u8, u8)] = &[
    (0, 0, 0),     // 0 noir
    (63, 63, 63),  // 1 blanc
    (42, 42, 42),  // 2 gris
    (21, 21, 21),  // 3 gris fonce
    (20, 40, 63),  // 4 bleu
    (8, 16, 40),   // 5 bleu fonce
    (20, 50, 20),  // 6 vert
    (60, 20, 20),  // 7 rouge
    (20, 55, 60),  // 8 cyan
    (60, 60, 15),  // 9 jaune
    (12, 26, 52),  // 10 bleu bureau
    (30, 36, 56),  // 11 barre titre
];

static mut BACK: Option<Vec<u8>> = None;

// --- Programmation des registres VGA (mode 13h / mode texte 03h) -------------

const MISC: u8 = 0x63;
const SEQ: [u8; 5] = [0x03, 0x01, 0x0F, 0x00, 0x0E];
const CRTC_13H: [u8; 25] = [
    0x5F, 0x4F, 0x50, 0x82, 0x54, 0x80, 0xBF, 0x1F, 0x00, 0x41, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x9C, 0x0E, 0x8F, 0x28, 0x40, 0x96, 0xB9, 0xA3, 0xFF,
];
const GC_13H: [u8; 9] = [0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x05, 0x0F, 0xFF];
const AC_13H: [u8; 21] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C,
    0x0D, 0x0E, 0x0F, 0x41, 0x00, 0x0F, 0x00, 0x00,
];

unsafe fn write_regs(crtc: &[u8; 25], gc: &[u8; 9], ac: &[u8; 21]) {
    outb(0x3C2, MISC);
    for (i, &v) in SEQ.iter().enumerate() {
        outb(0x3C4, i as u8);
        outb(0x3C5, v);
    }
    // Deverrouille les registres CRTC (bit de protection sur index 0x11).
    outb(0x3D4, 0x03);
    outb(0x3D5, inb(0x3D5) | 0x80);
    outb(0x3D4, 0x11);
    outb(0x3D5, inb(0x3D5) & !0x80);
    let mut crtc = *crtc;
    crtc[0x03] |= 0x80;
    crtc[0x11] &= !0x80;
    for (i, &v) in crtc.iter().enumerate() {
        outb(0x3D4, i as u8);
        outb(0x3D5, v);
    }
    for (i, &v) in gc.iter().enumerate() {
        outb(0x3CE, i as u8);
        outb(0x3CF, v);
    }
    for (i, &v) in ac.iter().enumerate() {
        let _ = inb(0x3DA); // reset du flip-flop adresse/donnee
        outb(0x3C0, i as u8);
        outb(0x3C0, v);
    }
    let _ = inb(0x3DA);
    outb(0x3C0, 0x20); // reactive l'affichage
}

fn set_palette() {
    unsafe {
        for (i, &(r, g, b)) in PALETTE.iter().enumerate() {
            outb(0x3C8, i as u8);
            outb(0x3C9, r);
            outb(0x3C9, g);
            outb(0x3C9, b);
        }
    }
}

/// Passe en mode graphique 13h et alloue le double-buffer.
pub fn enter() {
    unsafe {
        write_regs(&CRTC_13H, &GC_13H, &AC_13H);
        BACK = Some(vec![0u8; WIDTH * HEIGHT]);
    }
    set_palette();
    crate::serial_println!("[gfx] mode 13h actif (320x200x256)");
}

/// Restaure le mode texte 80x25 (registres standard mode 03h).
pub fn leave() {
    // Table mode texte 03h.
    const CRTC_03H: [u8; 25] = [
        0x5F, 0x4F, 0x50, 0x82, 0x55, 0x81, 0xBF, 0x1F, 0x00, 0x4F, 0x0D, 0x0E, 0x00,
        0x00, 0x00, 0x00, 0x9C, 0x0E, 0x8F, 0x28, 0x1F, 0x96, 0xB9, 0xA3, 0xFF,
    ];
    const GC_03H: [u8; 9] = [0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x0E, 0x00, 0xFF];
    const AC_03H: [u8; 21] = [
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x14, 0x07, 0x38, 0x39, 0x3A, 0x3B, 0x3C,
        0x3D, 0x3E, 0x3F, 0x0C, 0x00, 0x0F, 0x08, 0x00,
    ];
    unsafe {
        outb(0x3C2, 0x67);
        write_regs_text(&CRTC_03H, &GC_03H, &AC_03H);
        BACK = None;
    }
    crate::serial_println!("[gfx] retour mode texte");
}

unsafe fn write_regs_text(crtc: &[u8; 25], gc: &[u8; 9], ac: &[u8; 21]) {
    const SEQ_T: [u8; 5] = [0x03, 0x00, 0x03, 0x00, 0x02];
    for (i, &v) in SEQ_T.iter().enumerate() {
        outb(0x3C4, i as u8);
        outb(0x3C5, v);
    }
    outb(0x3D4, 0x11);
    outb(0x3D5, inb(0x3D5) & !0x80);
    for (i, &v) in crtc.iter().enumerate() {
        outb(0x3D4, i as u8);
        outb(0x3D5, v);
    }
    for (i, &v) in gc.iter().enumerate() {
        outb(0x3CE, i as u8);
        outb(0x3CF, v);
    }
    for (i, &v) in ac.iter().enumerate() {
        let _ = inb(0x3DA);
        outb(0x3C0, i as u8);
        outb(0x3C0, v);
    }
    let _ = inb(0x3DA);
    outb(0x3C0, 0x20);
}

// --- Dessin sur le double-buffer --------------------------------------------

fn back() -> &'static mut [u8] {
    unsafe { BACK.as_mut().map(|v| v.as_mut_slice()).unwrap_or(&mut []) }
}

pub fn clear(color: u8) {
    for p in back().iter_mut() { *p = color; }
}

#[inline]
pub fn pixel(x: usize, y: usize, color: u8) {
    if x < WIDTH && y < HEIGHT {
        back()[y * WIDTH + x] = color;
    }
}

pub fn fill_rect(x: usize, y: usize, w: usize, h: usize, color: u8) {
    let buf = back();
    let x1 = (x + w).min(WIDTH);
    let y1 = (y + h).min(HEIGHT);
    let mut yy = y;
    while yy < y1 {
        let row = yy * WIDTH;
        let mut xx = x;
        while xx < x1 { buf[row + xx] = color; xx += 1; }
        yy += 1;
    }
}

pub fn rect(x: usize, y: usize, w: usize, h: usize, color: u8) {
    if w == 0 || h == 0 { return; }
    fill_rect(x, y, w, 1, color);
    fill_rect(x, y + h - 1, w, 1, color);
    fill_rect(x, y, 1, h, color);
    fill_rect(x + w - 1, y, 1, h, color);
}

/// Recopie le double-buffer vers la memoire video (presentation).
pub fn present() {
    let buf = back();
    if buf.is_empty() { return; }
    unsafe {
        let dst = FB as *mut u8;
        core::ptr::copy_nonoverlapping(buf.as_ptr(), dst, WIDTH * HEIGHT);
    }
}

// --- Texte (police 8x8) -----------------------------------------------------

pub fn draw_char(x: usize, y: usize, c: u8, color: u8) {
    let glyph = font::glyph(c);
    for (row, bits) in glyph.iter().enumerate() {
        for col in 0..8 {
            if bits & (1 << col) != 0 {
                pixel(x + col, y + row, color);
            }
        }
    }
}

pub fn draw_text(x: usize, y: usize, s: &str, color: u8) {
    let mut cx = x;
    for b in s.bytes() {
        draw_char(cx, y, b, color);
        cx += 8;
    }
}

pub mod font;
