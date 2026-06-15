//! Bureau graphique minimal (mode VGA 13h) : fond, barre des taches, horloge,
//! une fenetre d'informations deplacable a la souris, et un curseur.
//!
//! Lance par la commande `desktop`. On quitte avec Echap, ce qui restaure le
//! mode texte et rend la main au shell. Tout est gate ici : si le graphique
//! pose probleme, l'OS texte reste intact (reboot).

use crate::drivers::gfx::{self, C_DESKTOP, C_TITLE, C_GRAY, C_DKGRAY, C_WHITE, C_BLUE, C_YELLOW, WIDTH, HEIGHT};
use crate::drivers::{keyboard, mouse};
use crate::arch::x86_64::rtc;
use crate::kernel::timer;
use crate::users;
use alloc::format;

const WW: i32 = 200; // largeur fenetre
const WH: i32 = 110; // hauteur fenetre
const BAR_H: usize = 11;

/// Lance le bureau graphique (bloquant jusqu'a Echap).
pub fn run() {
    gfx::enter();
    mouse::init();
    crate::serial_println!("[gui] bureau demarre");

    let mut wx: i32 = 55;
    let mut wy: i32 = 35;
    let mut dragging = false;
    let mut offx = 0i32;
    let mut offy = 0i32;

    loop {
        // Touche Echap (scancode 0x01) -> quitter.
        if let Some(sc) = keyboard::try_scancode() {
            if sc == 0x01 { break; }
        }

        let (mxu, myu) = mouse::pos();
        let mx = mxu as i32;
        let my = myu as i32;

        // Deplacement de la fenetre par la barre de titre.
        if mouse::left_down() {
            if !dragging && my >= wy && my < wy + BAR_H as i32 && mx >= wx && mx < wx + WW {
                dragging = true;
                offx = mx - wx;
                offy = my - wy;
            }
            if dragging {
                wx = mx - offx;
                wy = my - offy;
            }
        } else {
            dragging = false;
        }
        // Garde la fenetre a l'ecran.
        if wx < 0 { wx = 0; }
        if wy < BAR_H as i32 { wy = BAR_H as i32; }
        if wx + WW > WIDTH as i32 { wx = WIDTH as i32 - WW; }
        if wy + WH > HEIGHT as i32 - BAR_H as i32 { wy = HEIGHT as i32 - BAR_H as i32 - WH; }

        draw(wx as usize, wy as usize);
        draw_cursor(mxu, myu);
        gfx::present();
    }

    gfx::leave();
    crate::serial_println!("[gui] bureau ferme");
}

fn draw(wx: usize, wy: usize) {
    gfx::clear(C_DESKTOP);

    // Barre du haut + horloge.
    gfx::fill_rect(0, 0, WIDTH, BAR_H, C_TITLE);
    gfx::draw_text(2, 2, "Bouchaud OS", C_WHITE);
    let dt = rtc::now();
    let clk = format!("{:02}:{:02}:{:02}", dt.hour, dt.minute, dt.second);
    gfx::draw_text(WIDTH - clk.len() * 8 - 2, 2, &clk, C_YELLOW);

    // Fenetre "Systeme".
    window(wx, wy, "Systeme");
    let tx = wx + 4;
    let mut ty = wy + BAR_H + 3;
    let line = |s: &str, y: usize, col: u8| gfx::draw_text(wx + 4, y, s, col);
    let _ = line;
    gfx::draw_text(tx, ty, &format!("Version : {}", crate::VERSION), C_WHITE); ty += 10;
    gfx::draw_text(tx, ty, &format!("Session : {}", users::session().username()), C_WHITE); ty += 10;
    gfx::draw_text(tx, ty, &format!("Date    : {:04}-{:02}-{:02}", dt.year, dt.month, dt.day), C_WHITE); ty += 10;
    gfx::draw_text(tx, ty, &format!("Uptime  : {} s", timer::seconds()), C_WHITE); ty += 12;
    gfx::draw_text(tx, ty, "OS souverain francais", C_YELLOW); ty += 10;
    gfx::draw_text(tx, ty, "experimental", C_YELLOW);

    // Barre des taches en bas.
    gfx::fill_rect(0, HEIGHT - BAR_H, WIDTH, BAR_H, C_TITLE);
    gfx::draw_text(2, HEIGHT - BAR_H + 2, "Echap=quitter  clic-titre=deplacer", C_WHITE);
}

fn window(x: usize, y: usize, title: &str) {
    let w = WW as usize;
    let h = WH as usize;
    // Ombre.
    gfx::fill_rect(x + 3, y + 3, w, h, C_DKGRAY);
    // Corps.
    gfx::fill_rect(x, y, w, h, C_GRAY);
    gfx::rect(x, y, w, h, C_WHITE);
    // Barre de titre.
    gfx::fill_rect(x, y, w, BAR_H, C_BLUE);
    gfx::draw_text(x + 3, y + 2, title, C_WHITE);
}

/// Curseur fleche 8x8 (blanc avec contour noir grossier).
fn draw_cursor(mx: usize, my: usize) {
    const CUR: [u8; 8] = [
        0b00000001,
        0b00000011,
        0b00000111,
        0b00001111,
        0b00011111,
        0b00000111,
        0b00001101,
        0b00011000,
    ];
    for (row, bits) in CUR.iter().enumerate() {
        for col in 0..8 {
            if bits & (1 << col) != 0 {
                gfx::pixel(mx + col, my + row, C_WHITE);
            }
        }
    }
}
