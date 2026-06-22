//! Pilote graphique HD truecolor via Bochs VBE / BGA (carte `-vga std` QEMU).
//!
//! On programme l'interface DISPI (ports 0x01CE/0x01CF) pour passer en
//! 1280x720x32, on recupere le framebuffer lineaire dans le BAR0 PCI de la carte
//! graphique et on le mappe via l'offset de memoire physique du bootloader. Un
//! double-buffer 32 bits en RAM evite le scintillement ; `present()` le copie
//! tel quel vers le framebuffer (format XRGB8888 little-endian).
//!
//! L'API publique (couleurs en index `u8`, `WIDTH/HEIGHT`, primitives) est
//! conservee : le reste du GUI fonctionne sans modification, mais en HD et en
//! vraies couleurs. `leave()` restaure le mode texte VGA 80x25 pour le shell.

use alloc::vec;
use alloc::vec::Vec;
use crate::arch::x86_64::ports::{inb, outb};
use crate::arch::x86_64::pci;
use crate::kernel::memory;

/// Resolution HD du bureau.
pub const WIDTH: usize = 1280;
pub const HEIGHT: usize = 720;

// Index de palette (API stable). Les valeurs RGB associees sont dans PALETTE.
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
pub const C_DESKTOP: u8 = 10; // fond du bureau
pub const C_TITLE: u8 = 11;   // barre de titre

/// Palette index -> couleur XRGB8888 (truecolor 24 bits effectifs).
const PALETTE: [u32; 16] = [
    0x0000_0000, // 0 noir
    0x00F0_F0F0, // 1 blanc doux
    0x00B0_B0B0, // 2 gris
    0x0050_5058, // 3 gris fonce
    0x002D_7DD2, // 4 bleu
    0x0014_2138, // 5 bleu fonce
    0x0036_B37A, // 6 vert
    0x00E0_5A5A, // 7 rouge
    0x004F_C3D9, // 8 cyan
    0x00F2_C744, // 9 jaune
    0x001B_2A4A, // 10 fond bureau (bleu nuit)
    0x002C_4373, // 11 barre de titre
    0x0040_3010, // 12 (libre)
    0x000A_4030, // 13 (libre)
    0x0070_7078, // 14 (libre)
    0x0030_3038, // 15 (libre)
];

#[inline]
fn rgb(index: u8) -> u32 {
    PALETTE[(index as usize) & 0x0f]
}

static mut BACK: Option<Vec<u32>> = None;
static mut LFB: *mut u32 = core::ptr::null_mut();
static mut HD_ACTIVE: bool = false;

// --- Interface DISPI (Bochs VBE Extensions / BGA) ---------------------------

const VBE_DISPI_INDEX: u16 = 0x01CE;
const VBE_DISPI_DATA: u16 = 0x01CF;

const DISPI_INDEX_ID: u16 = 0;
const DISPI_INDEX_XRES: u16 = 1;
const DISPI_INDEX_YRES: u16 = 2;
const DISPI_INDEX_BPP: u16 = 3;
const DISPI_INDEX_ENABLE: u16 = 4;
const DISPI_INDEX_VIRT_WIDTH: u16 = 6;
const DISPI_INDEX_X_OFFSET: u16 = 8;
const DISPI_INDEX_Y_OFFSET: u16 = 9;

const DISPI_DISABLED: u16 = 0x00;
const DISPI_ENABLED: u16 = 0x01;
const DISPI_LFB_ENABLED: u16 = 0x40;

unsafe fn outw(port: u16, value: u16) {
    core::arch::asm!("out dx, ax", in("dx") port, in("ax") value, options(nomem, nostack, preserves_flags));
}
unsafe fn inw(port: u16) -> u16 {
    let value: u16;
    core::arch::asm!("in ax, dx", out("ax") value, in("dx") port, options(nomem, nostack, preserves_flags));
    value
}

fn dispi_write(index: u16, value: u16) {
    unsafe {
        outw(VBE_DISPI_INDEX, index);
        outw(VBE_DISPI_DATA, value);
    }
}
fn dispi_read(index: u16) -> u16 {
    unsafe {
        outw(VBE_DISPI_INDEX, index);
        inw(VBE_DISPI_DATA)
    }
}

// Programme la carte en mode lineaire 32 bits a la resolution voulue.
fn bga_set_mode(w: u16, h: u16) {
    dispi_write(DISPI_INDEX_ENABLE, DISPI_DISABLED);
    dispi_write(DISPI_INDEX_XRES, w);
    dispi_write(DISPI_INDEX_YRES, h);
    dispi_write(DISPI_INDEX_BPP, 32);
    dispi_write(DISPI_INDEX_VIRT_WIDTH, w);
    dispi_write(DISPI_INDEX_X_OFFSET, 0);
    dispi_write(DISPI_INDEX_Y_OFFSET, 0);
    dispi_write(DISPI_INDEX_ENABLE, DISPI_ENABLED | DISPI_LFB_ENABLED);
}

// Localise le framebuffer lineaire (BAR0 de la carte graphique) et le mappe.
fn locate_lfb() -> Option<*mut u32> {
    let dev = pci::find_display()?;
    pci::enable_bus_master(&dev);
    let bar0 = pci::bar(&dev, 0);
    // BAR memoire : on masque les 4 bits de poids faible (drapeaux).
    let phys = (bar0 & 0xFFFF_FFF0) as u64;
    if phys == 0 { return None; }
    Some(memory::phys_to_virt(phys) as *mut u32)
}

// --- Entree / sortie du mode graphique --------------------------------------

/// Passe en mode graphique HD (1280x720x32) et alloue le double-buffer.
/// Si la carte BGA est absente, le double-buffer existe quand meme mais
/// `present()` est sans effet (le shell texte reste accessible via Echap).
pub fn enter() {
    let id = dispi_read(DISPI_INDEX_ID);
    let lfb = locate_lfb();
    unsafe {
        BACK = Some(vec![0u32; WIDTH * HEIGHT]);
        match (id >= 0xB0C0 && id <= 0xB0C5, lfb) {
            (true, Some(p)) => {
                bga_set_mode(WIDTH as u16, HEIGHT as u16);
                LFB = p;
                HD_ACTIVE = true;
                crate::serial_println!("[gfx] BGA HD actif (1280x720x32, id={:#x})", id);
            }
            _ => {
                LFB = core::ptr::null_mut();
                HD_ACTIVE = false;
                crate::serial_println!("[gfx] BGA indisponible (id={:#x}) : present() inactif", id);
            }
        }
    }
}

/// Restaure le mode texte 80x25 (mode 03h) pour rendre la main au shell, apres
/// avoir desactive BGA et recharge la police texte (detruite par le graphique).
pub fn leave() {
    dispi_write(DISPI_INDEX_ENABLE, DISPI_DISABLED);
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
        load_text_font();
        BACK = None;
        LFB = core::ptr::null_mut();
        HD_ACTIVE = false;
    }
    crate::serial_println!("[gfx] retour mode texte");
}

fn reverse_bits(mut b: u8) -> u8 {
    let mut r = 0u8;
    for _ in 0..8 {
        r = (r << 1) | (b & 1);
        b >>= 1;
    }
    r
}

/// Recharge une police 8x16 dans le plan 2 (generateur de caracteres texte).
unsafe fn load_text_font() {
    outb(0x3C4, 0x00); outb(0x3C5, 0x01);
    outb(0x3C4, 0x02); outb(0x3C5, 0x04);
    outb(0x3C4, 0x04); outb(0x3C5, 0x07);
    outb(0x3C4, 0x00); outb(0x3C5, 0x03);
    outb(0x3CE, 0x04); outb(0x3CF, 0x02);
    outb(0x3CE, 0x05); outb(0x3CF, 0x00);
    outb(0x3CE, 0x06); outb(0x3CF, 0x00);

    let base = 0xA0000 as *mut u8;
    for c in 0u16..256 {
        let glyph = font::glyph(c as u8);
        for r in 0..16usize {
            let src = glyph[r / 2];
            let byte = reverse_bits(src);
            core::ptr::write_volatile(base.add((c as usize) * 32 + r), byte);
        }
    }

    outb(0x3C4, 0x00); outb(0x3C5, 0x01);
    outb(0x3C4, 0x02); outb(0x3C5, 0x03);
    outb(0x3C4, 0x04); outb(0x3C5, 0x03);
    outb(0x3C4, 0x00); outb(0x3C5, 0x03);
    outb(0x3CE, 0x04); outb(0x3CF, 0x00);
    outb(0x3CE, 0x05); outb(0x3CF, 0x10);
    outb(0x3CE, 0x06); outb(0x3CF, 0x0E);
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

// --- Dessin sur le double-buffer (32 bits) ----------------------------------

fn back() -> &'static mut [u32] {
    unsafe { BACK.as_mut().map(|v| v.as_mut_slice()).unwrap_or(&mut []) }
}

pub fn clear(color: u8) {
    let c = rgb(color);
    for p in back().iter_mut() { *p = c; }
}

#[inline]
pub fn pixel(x: usize, y: usize, color: u8) {
    if x < WIDTH && y < HEIGHT {
        back()[y * WIDTH + x] = rgb(color);
    }
}

pub fn fill_rect(x: usize, y: usize, w: usize, h: usize, color: u8) {
    let c = rgb(color);
    let buf = back();
    if buf.is_empty() { return; }
    let x1 = (x + w).min(WIDTH);
    let y1 = (y + h).min(HEIGHT);
    let mut yy = y;
    while yy < y1 {
        let row = yy * WIDTH;
        let mut xx = x;
        while xx < x1 { buf[row + xx] = c; xx += 1; }
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

/// Copie le double-buffer vers le framebuffer lineaire (sans effet si BGA off).
pub fn present() {
    let buf = back();
    if buf.is_empty() { return; }
    let lfb = unsafe { LFB };
    if lfb.is_null() { return; }
    unsafe {
        core::ptr::copy_nonoverlapping(buf.as_ptr(), lfb, WIDTH * HEIGHT);
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
    for c in s.chars() {
        draw_char(cx, y, font::fold(c), color);
        cx += 8;
    }
}

/// Dessine un caractere agrandi `scale` fois (chaque pixel de la police 8x8
/// devient un bloc scale x scale). `scale=1` equivaut a `draw_char`.
pub fn draw_char_scaled(x: usize, y: usize, c: u8, color: u8, scale: usize) {
    if scale <= 1 { draw_char(x, y, c, color); return; }
    let glyph = font::glyph(c);
    for (row, bits) in glyph.iter().enumerate() {
        for col in 0..8 {
            if bits & (1 << col) != 0 {
                fill_rect(x + col * scale, y + row * scale, scale, scale, color);
            }
        }
    }
}

/// Dessine une chaine agrandie `scale` fois (cellule de `8*scale` px de large).
pub fn draw_text_scaled(x: usize, y: usize, s: &str, color: u8, scale: usize) {
    let mut cx = x;
    for c in s.chars() {
        draw_char_scaled(cx, y, font::fold(c), color, scale);
        cx += 8 * scale;
    }
}

pub mod font;

// --- Primitives truecolor (RGB direct, 0x00RRGGBB) --------------------------
// Utilisees par le moteur de rendu web (couleurs CSS + images) ; le framebuffer
// est deja 32 bits, donc on ecrit la valeur RGB telle quelle.

#[inline]
pub fn pixel_rgb(x: usize, y: usize, rgb: u32) {
    if x < WIDTH && y < HEIGHT {
        back()[y * WIDTH + x] = rgb;
    }
}

/// Melange `rgb` sur le pixel (x,y) selon une couverture `alpha` (0..=255).
/// Lit le pixel de fond et compose : sert au rendu de police antialiasee.
pub fn blend_rgb(x: usize, y: usize, rgb: u32, alpha: u8) {
    if x >= WIDTH || y >= HEIGHT || alpha == 0 { return; }
    let buf = back();
    if buf.is_empty() { return; }
    let idx = y * WIDTH + x;
    if alpha >= 255 { buf[idx] = rgb & 0x00ff_ffff; return; }
    let a = alpha as u32;
    let inv = 255 - a;
    let dst = buf[idx];
    let dr = (dst >> 16) & 0xff; let dg = (dst >> 8) & 0xff; let db = dst & 0xff;
    let sr = (rgb >> 16) & 0xff; let sg = (rgb >> 8) & 0xff; let sb = rgb & 0xff;
    let r = (sr * a + dr * inv) / 255;
    let g = (sg * a + dg * inv) / 255;
    let b = (sb * a + db * inv) / 255;
    buf[idx] = (r << 16) | (g << 8) | b;
}

pub fn fill_rect_rgb(x: usize, y: usize, w: usize, h: usize, rgb: u32) {
    let buf = back();
    if buf.is_empty() { return; }
    let x1 = (x + w).min(WIDTH);
    let y1 = (y + h).min(HEIGHT);
    let mut yy = y;
    while yy < y1 {
        let row = yy * WIDTH;
        let mut xx = x;
        while xx < x1 { buf[row + xx] = rgb; xx += 1; }
        yy += 1;
    }
}

fn draw_char_rgb(x: usize, y: usize, c: u8, rgb: u32, scale: usize) {
    // Rendu NET (plus proche voisin) : chaque pixel de la police 8x8 devient un
    // bloc scale x scale. Le lissage (antialiasing) viendra avec une vraie
    // police vectorielle ; le bilineaire sur une police 8x8 rendait le texte flou.
    let glyph = font::glyph(c);
    for (row, bits) in glyph.iter().enumerate() {
        for col in 0..8 {
            if bits & (1 << col) != 0 {
                if scale <= 1 {
                    pixel_rgb(x + col, y + row, rgb);
                } else {
                    fill_rect_rgb(x + col * scale, y + row * scale, scale, scale, rgb);
                }
            }
        }
    }
}

/// Dessine une chaine en couleur RGB arbitraire, agrandie `scale` fois.
pub fn draw_text_rgb(x: usize, y: usize, s: &str, rgb: u32, scale: usize) {
    let mut cx = x;
    let step = 8 * scale.max(1);
    for c in s.chars() {
        draw_char_rgb(cx, y, font::fold(c), rgb, scale.max(1));
        cx += step;
    }
}

/// Copie un bloc d'image RGB (`pix` de `iw`x`ih`) a la position (x,y), borne
/// a la zone (clip_x,clip_y,clip_w,clip_h). Pixels hors zone ignores.
pub fn blit_rgb(x: usize, y: usize, iw: usize, ih: usize, pix: &[u32],
                clip_x: usize, clip_y: usize, clip_w: usize, clip_h: usize) {
    let buf = back();
    if buf.is_empty() { return; }
    let cx1 = (clip_x + clip_w).min(WIDTH);
    let cy1 = (clip_y + clip_h).min(HEIGHT);
    for row in 0..ih {
        let py = match y.checked_add(row) {
            Some(v) => v,
            None => continue,
        };
        if py < clip_y || py >= cy1 { continue; }
        let base = match row.checked_mul(iw) {
            Some(v) => v,
            None => continue,
        };
        for col in 0..iw {
            let px = match x.checked_add(col) {
                Some(v) => v,
                None => continue,
            };
            if px < clip_x || px >= cx1 { continue; }
            if base + col < pix.len() {
                buf[py * WIDTH + px] = pix[base + col];
            }
        }
    }
}
