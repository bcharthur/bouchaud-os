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
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
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
/// Vrai pendant qu'un processus ring 3 possede logiquement la sortie video.
///
/// Le mode BGA reste actif : seul `present()` du bureau est suspendu. Le
/// client userland peut donc ecrire `/dev/fb0` sans transition par le VGA texte.
static mut USERLAND_OWNS_DISPLAY: bool = false;
/// Adresse *physique* du framebuffer lineaire, memorisee pour pouvoir le
/// remapper dans un espace d'adressage utilisateur (`mmap` de `/dev/fb0`).
static mut LFB_PHYS: u64 = 0;

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
    unsafe { LFB_PHYS = phys; }
    Some(memory::phys_to_virt(phys) as *mut u32)
}

/// Adresse physique du framebuffer lineaire, si la carte a ete localisee.
///
/// Utilisee par `/dev/fb0` : `mmap` mappe ces pages telles quelles dans
/// l'espace utilisateur, ce qui evite toute copie entre un serveur graphique
/// en ring 3 et l'ecran.
pub fn lfb_phys() -> Option<u64> {
    let phys = unsafe { LFB_PHYS };
    if phys == 0 {
        // Le mode HD n'a pas encore ete active : on interroge le PCI.
        let dev = pci::find_display()?;
        let bar0 = pci::bar(&dev, 0);
        let phys = (bar0 & 0xFFFF_FFF0) as u64;
        if phys == 0 { return None; }
        unsafe { LFB_PHYS = phys; }
        return Some(phys);
    }
    Some(phys)
}

/// Resolution courante du framebuffer (largeur, hauteur) en pixels.
pub fn resolution() -> (usize, usize) {
    (WIDTH, HEIGHT)
}

// --- Entree / sortie du mode graphique --------------------------------------

/// Le bureau graphique HD est-il actif ? Utilise par les commandes qui lisent
/// le clavier en direct (ex. REPL Python) : en mode graphique, le clavier est
/// pompe par le window manager, une lecture bloquante gelerait le bureau.
pub fn is_active() -> bool {
    unsafe { HD_ACTIVE }
}

/// Le framebuffer physique est-il temporairement cede a un client ring 3 ?
pub fn userland_owns_display() -> bool {
    unsafe { USERLAND_OWNS_DISPLAY }
}

/// Cede logiquement l'ecran au userland sans repasser par le mode VGA texte.
///
/// Le gestionnaire de fenetres conserve son backbuffer en RAM. Tant que le
/// handoff est actif, `present()` devient un no-op ; le client peut donc mapper
/// `/dev/fb0` et peindre le LFB sans etre ecrase par une trame du bureau.
pub fn handoff_to_userland() -> bool {
    if !is_active() {
        enter();
    }
    if !is_active() {
        crate::serial_println!("[gfx] handoff userland impossible : BGA inactif");
        return false;
    }
    unsafe { USERLAND_OWNS_DISPLAY = true; }
    crate::drivers::gpu::note_handoff(true);
    crate::serial_println!("[gfx] framebuffer cede au userland (BGA conserve)");
    true
}

/// Reprend la presentation du bureau apres la fin du client userland.
pub fn resume_from_userland() {
    if !is_active() {
        // Filet de securite : le plugin Qt ne devrait pas couper BGA car ses
        // KDSETMODE sont des no-op dans l'ABI, mais on sait se reconstruire si
        // un futur backend le fait.
        enter();
    }
    unsafe { USERLAND_OWNS_DISPLAY = false; }
    crate::drivers::gpu::note_handoff(false);
    crate::serial_println!("[gfx] framebuffer repris par le bureau");
}

/// Passe en mode graphique HD (1280x720x32) et alloue le double-buffer.
/// Si la carte BGA est absente, le double-buffer existe quand meme mais
/// `present()` est sans effet (le shell texte reste accessible via Echap).
pub fn enter() {
    let id = dispi_read(DISPI_INDEX_ID);
    let lfb = locate_lfb();
    unsafe {
        BACK = Some(vec![0u32; WIDTH * HEIGHT]);
        USERLAND_OWNS_DISPLAY = false;
        match (id >= 0xB0C0 && id <= 0xB0C5, lfb) {
            (true, Some(p)) => {
                bga_set_mode(WIDTH as u16, HEIGHT as u16);
                LFB = p;
                HD_ACTIVE = true;
                crate::drivers::gpu::activate_bga(WIDTH, HEIGHT, 32, LFB_PHYS);
                crate::serial_println!("[gfx] BGA HD actif (1280x720x32, id={:#x})", id);
            }
            _ => {
                LFB = core::ptr::null_mut();
                HD_ACTIVE = false;
                crate::drivers::gpu::deactivate();
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
        USERLAND_OWNS_DISPLAY = false;
    }
    crate::drivers::gpu::deactivate();
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

/// Voie palette 16 couleurs : console de demarrage et affichage de secours.
///
/// Volontairement HORS de la decoupe (`BOUCHAUD_GUI_CLIP_V1`). C'est par ici
/// que passe ce qui doit s'afficher quand plus rien d'autre ne marche -- une
/// panique, un message de demarrage. Le decouper reviendrait a pouvoir masquer
/// exactement le message qu'on a besoin de lire.
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
    if userland_owns_display() { return; }
    let buf = back();
    if buf.is_empty() { return; }
    let lfb = unsafe { LFB };
    if lfb.is_null() { return; }
    unsafe {
        core::ptr::copy_nonoverlapping(buf.as_ptr(), lfb, WIDTH * HEIGHT);
    }
    crate::drivers::gpu::note_present(WIDTH * HEIGHT * core::mem::size_of::<u32>());
}

/// Copie uniquement une region du double-buffer vers le scanout lineaire.
///
/// BGA n'offre pas de commande de damage: la bonne primitive est donc un
/// memcpy par ligne. Les coordonnees sont rognees avant toute arithmetique afin
/// qu'un rectangle client hostile ne puisse sortir du framebuffer.
// BOUCHAUD_GFX_PRESENT_TRACE_V1
//
// POURQUOI CETTE FONCTION EST INSTRUMENTEE
// ----------------------------------------
// `present_rect` a cinq sorties anticipees et aucune ne dit rien. Le
// compositeur, lui, compte sa presentation AVANT d'appeler. Les compteurs
// `presents`, `presented_pixels` et `frames_composed` peuvent donc monter
// regulierement pendant qu'AUCUN pixel n'atteint le framebuffer lineaire :
//
//   * l'affichage a ete remis a un programme ring 3 et ne revient pas ;
//   * le backbuffer n'est pas alloue ;
//   * le LFB n'est pas mappe ;
//   * le rectangle est vide une fois ramene a l'ecran.
//
// C'est exactement la forme d'un bureau « vivant mais fige » : le noyau tourne,
// la boucle tourne, les trames se composent, et l'ecran ne change pas. Sans ces
// compteurs, rien dans la trace ne permet de distinguer ce cas d'un compositeur
// qui ne compose plus.
//
// `lfb_present_generation` ne monte que si des pixels ont ete ECRITS dans le
// LFB. C'est le dernier maillon de la chaine, et le seul qui prouve l'affichage.

static PRESENTS_DEMANDES: AtomicU64 = AtomicU64::new(0);
static PRESENTS_COPIES: AtomicU64 = AtomicU64::new(0);
static PIXELS_COPIES_LFB: AtomicU64 = AtomicU64::new(0);
static REFUS_USERLAND: AtomicU64 = AtomicU64::new(0);
static REFUS_TAMPON: AtomicU64 = AtomicU64::new(0);
static REFUS_LFB: AtomicU64 = AtomicU64::new(0);
static REFUS_RECT_VIDE: AtomicU64 = AtomicU64::new(0);
static DERNIER_PRESENT_NS: AtomicU64 = AtomicU64::new(0);
/// Dernier rectangle reellement copie, empaquete `x | y << 16 | w << 32 | h << 48`.
static DERNIER_PRESENT_RECT: AtomicU64 = AtomicU64::new(0);

fn empaquete_rect(x: usize, y: usize, largeur: usize, hauteur: usize) -> u64 {
    ((x as u64) & 0xffff)
        | (((y as u64) & 0xffff) << 16)
        | (((largeur as u64) & 0xffff) << 32)
        | (((hauteur as u64) & 0xffff) << 48)
}

/// `(x, y, largeur, hauteur)` du dernier rectangle copie dans le LFB.
pub fn dernier_present_rect() -> (usize, usize, usize, usize) {
    let brut = DERNIER_PRESENT_RECT.load(Ordering::Relaxed);
    (
        (brut & 0xffff) as usize,
        ((brut >> 16) & 0xffff) as usize,
        ((brut >> 32) & 0xffff) as usize,
        ((brut >> 48) & 0xffff) as usize,
    )
}

/// Trace du dernier maillon : `(demandes, copies, pixels_copies, refus_userland,
/// refus_tampon, refus_lfb, refus_rect_vide, dernier_present_ns)`.
pub fn trace_present() -> (u64, u64, u64, u64, u64, u64, u64, u64) {
    (
        PRESENTS_DEMANDES.load(Ordering::Relaxed),
        PRESENTS_COPIES.load(Ordering::Relaxed),
        PIXELS_COPIES_LFB.load(Ordering::Relaxed),
        REFUS_USERLAND.load(Ordering::Relaxed),
        REFUS_TAMPON.load(Ordering::Relaxed),
        REFUS_LFB.load(Ordering::Relaxed),
        REFUS_RECT_VIDE.load(Ordering::Relaxed),
        DERNIER_PRESENT_NS.load(Ordering::Relaxed),
    )
}

/// Nombre de presentations qui ont reellement ecrit dans le LFB.
///
/// Le seul compteur qui prouve que l'ecran a change.
pub fn lfb_present_generation() -> u64 {
    PRESENTS_COPIES.load(Ordering::Relaxed)
}

pub fn present_rect(x: usize, y: usize, width: usize, height: usize) {
    PRESENTS_DEMANDES.fetch_add(1, Ordering::Relaxed);
    if userland_owns_display() {
        REFUS_USERLAND.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let buf = back();
    if buf.is_empty() {
        REFUS_TAMPON.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let lfb = unsafe { LFB };
    if lfb.is_null() {
        REFUS_LFB.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let x1 = x.saturating_add(width).min(WIDTH);
    let y1 = y.saturating_add(height).min(HEIGHT);
    if x >= x1 || y >= y1 {
        REFUS_RECT_VIDE.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let count = x1 - x;
    for row in y..y1 {
        let offset = row * WIDTH + x;
        unsafe {
            core::ptr::copy_nonoverlapping(buf.as_ptr().add(offset), lfb.add(offset), count);
        }
    }
    // Ici, et seulement ici, des pixels ont atteint l'ecran.
    PRESENTS_COPIES.fetch_add(1, Ordering::Relaxed);
    PIXELS_COPIES_LFB.fetch_add((count * (y1 - y)) as u64, Ordering::Relaxed);
    DERNIER_PRESENT_RECT.store(empaquete_rect(x, y, count, y1 - y), Ordering::Relaxed);
    DERNIER_PRESENT_NS.store(crate::kernel::timer::monotonic_ns(), Ordering::Relaxed);
    crate::drivers::gpu::note_present(count * (y1 - y) * core::mem::size_of::<u32>());
}

// --- Texte bitmap 8×8 (zéro allocation, zéro tas) ---------------------------

fn draw_char_bmp(x: usize, y: usize, c: u8, color: u8) {
    let glyph = font::glyph(c);
    for (row, bits) in glyph.iter().enumerate() {
        for col in 0..8 {
            if bits & (1 << col) != 0 { pixel(x + col, y + row, color); }
        }
    }
}

/// Dessine `s` en police bitmap 8×8, couleur de palette (zéro alloc).
pub fn draw_text(x: usize, y: usize, s: &str, color: u8) {
    for (i, ch) in s.chars().enumerate() {
        draw_char_bmp(x + i * 8, y, ch as u8, color);
    }
}

/// Police bitmap agrandie `scale` fois, couleur de palette.
pub fn draw_text_scaled(x: usize, y: usize, s: &str, color: u8, scale: usize) {
    let sc = scale.max(1);
    let fg = rgb(color);
    for (i, ch) in s.chars().enumerate() {
        let glyph = font::glyph(ch as u8);
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..8usize {
                if bits & (1 << col) != 0 {
                    for dy in 0..sc { for dx in 0..sc {
                        pixel_rgb(x + i*8*sc + col*sc + dx, y + row*sc + dy, fg);
                    }}
                }
            }
        }
    }
}

/// Police bitmap `scale` fois, couleur RGB 24 bits directe.
pub fn draw_text_rgb(x: usize, y: usize, s: &str, rgb_color: u32, scale: usize) {
    let sc = scale.max(1);
    for (i, ch) in s.chars().enumerate() {
        let glyph = font::glyph(ch as u8);
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..8usize {
                if bits & (1 << col) != 0 {
                    for dy in 0..sc { for dx in 0..sc {
                        pixel_rgb(x + i*8*sc + col*sc + dx, y + row*sc + dy, rgb_color);
                    }}
                }
            }
        }
    }
}

/// Dessine `s` en mode **proportionnel** via le rasterizer TTF from-scratch
/// (DejaVu Sans, antialiasé, BTreeMap cache). Repli bitmap si indisponible.
/// Retourne la position X finale.
pub fn draw_text_prop(x: usize, y: usize, s: &str, rgb_color: u32, px: f32, bold: bool) -> usize {
    use crate::gui::font as ftf;
    if !ftf::draw_text(x as i32, y as i32, s, rgb_color, px as i32, bold) {
        for (i, ch) in s.chars().enumerate() {
            draw_char_bmp(x + i * 8, y, ch as u8, 1);
        }
    }
    x + ftf::text_width(s, px as i32) as usize
}

/// Largeur en pixels d'une chaîne proportionnelle (sans dessin).
pub fn text_width(s: &str, px: f32, _bold: bool) -> usize {
    crate::gui::font::text_width(s, px as i32) as usize
}

pub mod font;

// --- Primitives truecolor (RGB direct, 0x00RRGGBB) --------------------------
// Utilisees par le moteur de rendu web (couleurs CSS + images) ; le framebuffer
// est deja 32 bits, donc on ecrit la valeur RGB telle quelle.

// BOUCHAUD_GUI_CLIP_V1
//
// Rectangle de decoupe du backbuffer.
//
// # Le probleme qu'il resout
//
// Le compositeur bornait deja la COPIE finale vers l'ecran (`present_rect`),
// mais pas le DESSIN. `draw_wallpaper` repeignait un degrade plein ecran ligne
// par ligne a chaque trame -- 921 600 ecritures a 1280x720 -- meme quand seul
// un curseur de 14x22 avait bouge. Les icones, la barre, les cadres et tout le
// texte suivaient.
//
// Plutot qu'un parametre de region a chaque widget -- « une foret de conditions
// specifiques » -- la decoupe vit au seul endroit par ou passent tous les
// pixels : les quatre ecrivains ci-dessous. Aucun widget ne change, et aucun ne
// peut l'oublier.
//
// # Pourquoi c'est sur
//
// Le backbuffer persiste d'une trame a l'autre. Ne pas dessiner hors de la
// decoupe n'est donc correct que si l'on ne PRESENTE jamais hors d'elle. Le
// compositeur pose la decoupe egale au rectangle qu'il va presenter, ce qui
// tient l'invariant par construction : ce qui n'est pas dessine n'est pas
// copie, et ce qui est copie vient d'etre dessine.
static CLIP_X0: AtomicUsize = AtomicUsize::new(0);
static CLIP_Y0: AtomicUsize = AtomicUsize::new(0);
static CLIP_X1: AtomicUsize = AtomicUsize::new(WIDTH);
static CLIP_Y1: AtomicUsize = AtomicUsize::new(HEIGHT);
static PIXELS_DESSINES: AtomicU64 = AtomicU64::new(0);

// BOUCHAUD_GFX_TEXTE_SEGMENT_V1
//
// Pixels de TEXTE melanges dans le backbuffer. Comptes a part, et non ajoutes
// a `PIXELS_DESSINES`, pour que les releves d'avant et d'apres restent
// comparables : le texte n'a jamais ete compte, il l'est desormais, et
// confondre les deux ferait passer une mesure nouvelle pour une regression.
//
// C'est la mesure qui manquait. Le rendu du texte etait le seul chemin de
// dessin invisible aux metriques, et c'est precisement la que le compositeur
// depensait le plus par rectangle de degat.
static PIXELS_TEXTE: AtomicU64 = AtomicU64::new(0);

/// Borne le dessin a ce rectangle jusqu'au prochain [`reset_clip`].
pub fn set_clip(x: usize, y: usize, w: usize, h: usize) {
    CLIP_X0.store(x.min(WIDTH), Ordering::Relaxed);
    CLIP_Y0.store(y.min(HEIGHT), Ordering::Relaxed);
    CLIP_X1.store((x + w).min(WIDTH), Ordering::Relaxed);
    CLIP_Y1.store((y + h).min(HEIGHT), Ordering::Relaxed);
}

/// Rend le dessin a l'ecran entier.
pub fn reset_clip() {
    CLIP_X0.store(0, Ordering::Relaxed);
    CLIP_Y0.store(0, Ordering::Relaxed);
    CLIP_X1.store(WIDTH, Ordering::Relaxed);
    CLIP_Y1.store(HEIGHT, Ordering::Relaxed);
}

#[inline]
fn clip() -> (usize, usize, usize, usize) {
    (
        CLIP_X0.load(Ordering::Relaxed),
        CLIP_Y0.load(Ordering::Relaxed),
        CLIP_X1.load(Ordering::Relaxed),
        CLIP_Y1.load(Ordering::Relaxed),
    )
}

#[inline]
fn dans_clip(x: usize, y: usize) -> bool {
    let (x0, y0, x1, y1) = clip();
    x >= x0 && x < x1 && y >= y0 && y < y1
}

/// Intersection de deux rectangles en bornes `(x0, y0, x1, y1)` exclusives.
///
/// Extraite pour etre exercee sur l'hote : une intersection fausse ne produit
/// aucune erreur, seulement des pixels au mauvais endroit -- et quelques
/// pixels de decalage ne se voient pas dans un journal.
///
/// Rend un rectangle VIDE (x1 <= x0 ou y1 <= y0) quand les deux sont
/// disjoints ; l'appelant sort alors sans rien parcourir.
pub fn intersection(
    a: (usize, usize, usize, usize),
    b: (usize, usize, usize, usize),
) -> (usize, usize, usize, usize) {
    (a.0.max(b.0), a.1.max(b.1), a.2.min(b.2), a.3.min(b.3))
}

/// Rectangle de decoupe courant, en bornes exclusives `(x0, y0, x1, y1)`.
///
/// Pour les rares appelants qui copient par lignes entieres et ne peuvent pas
/// passer par les primitives ci-dessus -- la recopie d'une surface cliente.
pub fn clip_rect() -> (usize, usize, usize, usize) {
    clip()
}

// BOUCHAUD_GFX_CULLING_AMONT_V1
//
// La trame va-t-elle presenter un seul pixel de ce rectangle ?
//
// # Pourquoi un predicat, alors que la decoupe rejette deja
//
// Parce que la decoupe rejette les PIXELS, pas le TRAVAIL qui les produit.
//
// Le compositeur dessine une fois par rectangle de degat. `apps::draw_app`
// etait donc rappele pour chacun -- y compris pour un degat qui ne touche que
// la barre de titre. L'explorateur de fichiers y parcourt le RAMFS pour
// compter ses entrees et alloue une chaine par ligne ; le moniteur systeme y
// lit l'horloge temps reel par ports d'E/S, les statistiques du tas et le
// nombre de processus. Tout cela pour des pixels que la decoupe jetait.
//
// Sous TCG et sous le gros verrou du noyau, ce n'est pas un detail : ce sont
// des acces materiel et des prises de verrou faits plusieurs fois par trame
// pour rien.
//
// Un appelant qui l'oublie ne dessine pas faux -- la decoupe est toujours la.
// Il paie seulement ce qu'il aurait pu ne pas payer.
pub fn decoupe_touche(x: usize, y: usize, w: usize, h: usize) -> bool {
    if w == 0 || h == 0 { return false }
    let (x0, y0, x1, y1) = intersection(
        clip(),
        (x, y, x.saturating_add(w), y.saturating_add(h)),
    );
    x1 > x0 && y1 > y0
}

/// Compte des pixels ecrits par un chemin qui n'utilise pas les primitives.
pub fn note_pixels_dessines(nombre: u64) {
    PIXELS_DESSINES.fetch_add(nombre, Ordering::Relaxed);
}

/// Pixels de texte melanges dans le backbuffer depuis le demarrage.
pub fn pixels_texte() -> u64 {
    PIXELS_TEXTE.load(Ordering::Relaxed)
}

/// Pixels reellement ecrits dans le backbuffer depuis le demarrage.
///
/// C'est la mesure qui distingue « on copie moins » de « on dessine moins ».
/// La premiere etait deja vraie avant la decoupe ; seule la seconde compte
/// pour le temps passe.
pub fn pixels_dessines() -> u64 {
    PIXELS_DESSINES.load(Ordering::Relaxed)
}

#[inline]
pub fn pixel_rgb(x: usize, y: usize, rgb: u32) {
    if dans_clip(x, y) {
        back()[y * WIDTH + x] = rgb;
        PIXELS_DESSINES.fetch_add(1, Ordering::Relaxed);
    }
}

/// Tranche mutable d'une ligne du backbuffer, bornee a l'ecran.
///
/// C'est le chemin par lequel le compositeur ecrit la surface d'un client :
/// les pixels vont des frames partagees au double-tampon du bureau par un
/// `copy_from_slice`, sans passer par un tampon intermediaire ni par un test de
/// bornes par pixel. Rend une tranche vide si la ligne tombe hors de l'ecran,
/// ce qui laisse l'appelant ecrire `if dst.is_empty() { continue }` plutot que
/// de refaire le rognage de son cote.
pub fn ligne_mut(x: usize, y: usize, n: usize) -> &'static mut [u32] {
    if y >= HEIGHT || x >= WIDTH {
        return &mut [];
    }
    let n = n.min(WIDTH - x);
    let buf = back();
    if buf.is_empty() {
        return &mut [];
    }
    let debut = y * WIDTH + x;
    &mut buf[debut..debut + n]
}

/// Lit la couleur RGB du pixel (x,y) dans le backbuffer.
pub fn get_pixel_rgb(x: usize, y: usize) -> u32 {
    if x < WIDTH && y < HEIGHT {
        let b = back();
        if !b.is_empty() { b[y * WIDTH + x] } else { 0 }
    } else { 0 }
}

/// Melange `rgb` sur le pixel (x,y) selon une couverture `alpha` (0..=255).
/// Lit le pixel de fond et compose : sert au rendu de police antialiasee.
pub fn blend_rgb(x: usize, y: usize, rgb: u32, alpha: u8) {
    if alpha == 0 || !dans_clip(x, y) { return; }
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

// BOUCHAUD_GFX_TEXTE_SEGMENT_V1
//
// Melange une SUITE de pixels d'un glyphe sur une ligne, decoupe comprise.
//
// `blend_rgb` relit la decoupe -- quatre chargements atomiques -- a chaque
// pixel. Un glyphe de 9x12 en coutait donc 432 rien que pour decider de ne pas
// dessiner. Ici la decoupe est evaluee UNE fois pour la ligne, et le writer
// reste le seul endroit qui l'applique : `BOUCHAUD_GUI_CLIP_V1` tient.
//
// `couverture[i]` est l'alpha du pixel `x + i`. Zero ne touche rien.
pub fn blend_span(x: usize, y: usize, rgb: u32, couverture: &[u8], gras: bool) {
    if couverture.is_empty() { return; }
    let buf = back();
    if buf.is_empty() { return; }
    let (cx0, cy0, cx1, cy1) = clip();
    if y < cy0 || y >= cy1 { return; }
    let row = y * WIDTH;
    let mut ecrits = 0u64;
    for (index, &alpha) in couverture.iter().enumerate() {
        if alpha == 0 { continue }
        let px = x + index;
        // Le gras redouble chaque pixel vers la droite ; les deux passent par
        // la meme decoupe.
        for px in [px, px + 1] {
            if px < cx0 || px >= cx1 { continue }
            melange_pixel(buf, row + px, rgb, alpha);
            ecrits += 1;
            if !gras { break }
        }
    }
    if ecrits != 0 { PIXELS_TEXTE.fetch_add(ecrits, Ordering::Relaxed); }
}

/// Melange source sur destination a l'index deja verifie.
#[inline]
fn melange_pixel(buf: &mut [u32], idx: usize, rgb: u32, alpha: u8) {
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
    let (cx0, cy0, cx1, cy1) = clip();
    // L'intersection est faite UNE fois, pas par pixel : un remplissage
    // entierement hors decoupe sort sans avoir rien parcouru, et c'est
    // exactement le cas du fond d'ecran quand seul un curseur a bouge.
    let x0 = x.max(cx0);
    let y0 = y.max(cy0);
    let x1 = (x + w).min(cx1);
    let y1 = (y + h).min(cy1);
    if x1 <= x0 || y1 <= y0 { return; }
    // BOUCHAUD_GFX_REMPLISSAGE_TRANCHE_V1
    //
    // `buf[row + xx] = rgb` dans une boucle indexee, c'est une verification de
    // bornes par pixel. `fill` sur une tranche donne au compilateur ce qu'il
    // lui faut pour emettre un remplissage memoire vectorise : une seule
    // verification de bornes pour toute la ligne.
    //
    // Le fond d'ecran, les barres et desormais chaque segment de la forme des
    // fenetres passent par ici. Sur un remplissage plein ecran, cela fait 720
    // verifications de bornes au lieu de 921 600 -- et sous TCG, ou chaque
    // instruction est traduite, la boucle serree compte autant que le nombre
    // de pixels.
    let mut yy = y0;
    while yy < y1 {
        let row = yy * WIDTH;
        buf[row + x0..row + x1].fill(rgb);
        yy += 1;
    }
    PIXELS_DESSINES.fetch_add(((x1 - x0) * (y1 - y0)) as u64, Ordering::Relaxed);
}

/// Copie un bloc d'image RGB (`pix` de `iw`x`ih`) a la position (x,y), borne
/// a la zone (clip_x,clip_y,clip_w,clip_h). Pixels hors zone ignores.
pub fn blit_rgb(x: usize, y: usize, iw: usize, ih: usize, pix: &[u32],
                clip_x: usize, clip_y: usize, clip_w: usize, clip_h: usize) {
    let buf = back();
    if buf.is_empty() { return; }
    // La decoupe demandee par l'appelant ET celle de la trame : la plus
    // restrictive des deux gagne, sans quoi un widget pourrait dessiner hors
    // de la region que le compositeur va presenter.
    //
    // BOUCHAUD_GUI_CLIP_V2 : les bornes DROITES se calculent depuis les bornes
    // gauches D'ORIGINE. La version precedente faisait :
    //
    //     clip_x = max(clip_x, gx0)
    //     cx1    = min(clip_x + clip_w, gx1)   // <- clip_x deja deplace
    //
    // ce qui DECALE la fenetre vers la droite au lieu de l'intersecter : une
    // decoupe [10, 20) recadree par un global commencant a 15 devenait
    // [15, 25) et non [15, 20). Le rectangle gagnait a droite ce qu'il perdait
    // a gauche. Les ecritures restaient dans l'ecran -- `gx1` est borne par
    // `WIDTH` -- donc ce n'etait pas une faute memoire, mais un widget pouvait
    // peindre cinq pixels hors de la zone qu'il avait demandee.
    let (gx0, gy0, gx1, gy1) = clip();
    let origine_x1 = clip_x.saturating_add(clip_w);
    let origine_y1 = clip_y.saturating_add(clip_h);
    let clip_x = clip_x.max(gx0);
    let clip_y = clip_y.max(gy0);
    let cx1 = origine_x1.min(gx1);
    let cy1 = origine_y1.min(gy1);
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
