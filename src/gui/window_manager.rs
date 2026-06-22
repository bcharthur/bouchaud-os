//! Boucle d'evenements du gestionnaire de fenetres : entree souris/clavier,
//! focus / z-order, deplacement / redimensionnement, et rendu.

use crate::gui::apps;
use crate::gui::event::Key;
use crate::gui::framebuffer as fb;
use crate::gui::mouse;
use crate::gui::widgets;
use crate::gui::window::{
    clamp_win, icon_rect, make_app, menu_rect, start_btn, taskbar_btn, toggle_max, Drag, Win,
    BAR_H, ICONS, MENU, MIN_H, MIN_W, TITLE_H,
};
use crate::drivers::keyboard;
use crate::fs::ramfs;
use crate::users;
use alloc::vec::Vec;

/// Lance le bureau (bloquant jusqu'a Quitter).
pub fn run() {
    fb::enter();
    mouse::init();
    crate::serial_println!("[gui] window manager demarre");

    let home = ramfs::fs().resolve(users::session().home(), 0).unwrap_or(0);
    let mut wins: Vec<Win> = Vec::new();
    let mut menu_open = false;
    let mut prev_left = false;
    let mut drag: Option<Drag> = None;
    let mut spawn_n = 0i32;

    wins.push(make_app(2, home, &mut spawn_n)); // navigateur d'accueil

    let mut quit = false;
    while !quit {
        // ---- Clavier (non bloquant) ----
        while let Some(k) = keyboard::try_key() {
            match k {
                Key::Other => {
                    if menu_open { menu_open = false; }
                    else if wins.pop().is_none() { quit = true; }
                }
                other => {
                    if let Some(w) = wins.last_mut() {
                        if apps::key_to_app(w, other, home) { wins.pop(); }
                    }
                }
            }
        }

        // ---- Souris ----
        let (mxu, myu) = mouse::pos();
        let mx = mxu as i32;
        let my = myu as i32;
        let wheel = mouse::take_wheel();
        let left = mouse::left_down();
        let click = left && !prev_left;
        prev_left = left;

        if left {
            if let Some(d) = drag {
                if let Some(w) = wins.last_mut() {
                    match d {
                        Drag::Move(ox, oy) => { w.x = mx - ox; w.y = my - oy; }
                        Drag::Resize => {
                            w.w = (mx - w.x).max(MIN_W);
                            w.h = (my - w.y).max(MIN_H);
                            if w.x + w.w > fb::WIDTH as i32 { w.w = fb::WIDTH as i32 - w.x; }
                            if w.y + w.h > fb::HEIGHT as i32 - BAR_H as i32 { w.h = fb::HEIGHT as i32 - BAR_H as i32 - w.y; }
                        }
                    }
                    clamp_win(w);
                }
            }
        } else {
            drag = None;
        }

        if click {
            handle_click(mx, my, &mut wins, &mut menu_open, &mut drag, &mut quit, home, &mut spawn_n);
        }
        if wheel != 0 {
            handle_wheel(mx, my, wheel, &mut wins);
        }

        widgets::draw_desktop(&wins);
        if menu_open { widgets::draw_menu(); }
        widgets::draw_taskbar(&wins, menu_open);
        widgets::draw_cursor(mxu, myu);
        crate::kernel::timer::mark_frame();
        fb::present();
    }

    fb::leave();
    crate::serial_println!("[gui] window manager ferme");
}

fn handle_wheel(mx: i32, my: i32, delta: i32, wins: &mut Vec<Win>) {
    for i in (0..wins.len()).rev() {
        let w = &wins[i];
        if !w.min && mx >= w.x && mx < w.x + w.w && my >= w.y && my < w.y + w.h {
            apps::wheel_to_app(&mut wins[i], mx, my, delta);
            break;
        }
    }
}

fn handle_click(
    mx: i32, my: i32,
    wins: &mut Vec<Win>,
    menu_open: &mut bool,
    drag: &mut Option<Drag>,
    quit: &mut bool,
    home: usize,
    spawn_n: &mut i32,
) {
    if *menu_open {
        let mr = menu_rect();
        if mr.hit(mx, my) {
            let row = ((my - mr.y - 1) / 10) as usize;
            if row < MENU.len() {
                if row == MENU.len() - 1 { *quit = true; }
                else { wins.push(make_app(row, home, spawn_n)); }
            }
        }
        *menu_open = false;
        return;
    }
    if start_btn().hit(mx, my) { *menu_open = true; return; }

    // Barre des taches : restaure (si minimisee) et donne le focus.
    for i in 0..wins.len() {
        if taskbar_btn(i).hit(mx, my) {
            let mut w = wins.remove(i);
            w.min = false;
            wins.push(w);
            return;
        }
    }

    // Fenetres visibles, du dessus vers le dessous.
    let mut hit: Option<usize> = None;
    for i in (0..wins.len()).rev() {
        let w = &wins[i];
        if !w.min && mx >= w.x && mx < w.x + w.w && my >= w.y && my < w.y + w.h {
            hit = Some(i);
            break;
        }
    }
    if hit.is_none() {
        // Clic sur le bureau (aucune fenetre) : lance l'icone touchee.
        for j in 0..ICONS.len() {
            if icon_rect(j).hit(mx, my) {
                wins.push(make_app(ICONS[j].1, home, spawn_n));
                return;
            }
        }
    }
    if let Some(i) = hit {
        let w = wins.remove(i);
        wins.push(w);
        let top = wins.last_mut().unwrap();
        let r = top.x + top.w;
        let on_title = my >= top.y + 1 && my < top.y + TITLE_H;
        if on_title && mx >= r - 10 && mx < r - 1 {
            wins.pop();
        } else if on_title && mx >= r - 19 && mx < r - 10 {
            toggle_max(top);
        } else if on_title && mx >= r - 28 && mx < r - 19 {
            top.min = true;
            let m = wins.pop().unwrap();
            wins.insert(0, m);
        } else if my >= top.y + top.h - 8 && mx >= r - 8 {
            *drag = Some(Drag::Resize);
        } else if my < top.y + TITLE_H {
            *drag = Some(Drag::Move(mx - top.x, my - top.y));
        } else {
            apps::app_click(top, mx, my, home);
        }
    }
}
