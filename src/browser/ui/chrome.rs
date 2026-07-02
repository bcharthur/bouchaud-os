//! Chrome du navigateur Nautile : onglets, barre d'outils, contenu, ascenseur.
//!
//! Inspiré du chrome de Chrome/Firefox :
//!   ┌─────────────────────────────────────────────┐
//!   │ [Onglet 1 ×] [Onglet 2 ×] [+]              │  ← TABS_H (20 px)
//!   ├─────────────────────────────────────────────┤
//!   │ [<][>][R][~]  [ barre d'adresse           ] │  ← TOOLBAR_H (22 px)
//!   ├─────────────────────────────────────────────┤
//!   │                                             │
//!   │              Zone de contenu                │  ← bh - CHROME_H
//!   │                                      [|||]  │
//!   └─────────────────────────────────────────────┘

use crate::gui::framebuffer as fb;
use crate::gui::web;
use crate::gui::event::Key;
use super::theme::*;
use super::super::state::BrowserState;
use alloc::format;
use alloc::string::{String, ToString};

// ── Événement généré par l'interaction avec le chrome ────────────────────────

/// Résultat d'une interaction clavier, souris ou molette avec le chrome.
pub enum ChromeEvent {
    None,
    Navigate(String),   // charger cette URL (barre d'adresse, clic de lien)
    Back,               // reculer dans l'historique
    Forward,            // avancer dans l'historique
    Refresh,            // recharger l'URL courante
    Home,               // aller sur about:bouchaud
    NewTab,             // ouvrir un nouvel onglet vide
    CloseTab(usize),    // fermer l'onglet i
    SelectTab(usize),   // activer l'onglet i
    ScrollTo(i32),      // positionner le défilement
    InputChar(char),    // ajouter un caractère dans la barre d'adresse
    InputBackspace,     // supprimer un caractère dans la barre d'adresse
    DispatchJs(String), // exécuter du JS dans la page courante
    FocusField(usize),  // donner le focus au champ de formulaire i de la page
    FieldChar(char),    // taper un caractère dans le champ focalisé
    FieldBackspace,     // effacer un caractère dans le champ focalisé
    SubmitField,        // soumettre le formulaire du champ focalisé (Entrée)
    Blur,               // retirer le focus du champ (clic ailleurs)
}

// ── Dessin principal ──────────────────────────────────────────────────────────

/// Dessine le navigateur complet dans la zone (bx, by, bw, bh).
/// La zone est le corps de la fenêtre (titre déjà exclu).
pub fn draw(state: &BrowserState, bx: usize, by: usize, bw: usize, bh: usize) {
    if bw < 20 || bh < CHROME_H + 10 { return; }

    draw_tabbar(state, bx, by, bw);
    draw_toolbar(state, bx, by + TABS_H, bw);

    let cy = by + CHROME_H;
    let ch = bh.saturating_sub(CHROME_H);
    let tab = state.tab();
    let needs_scroll = (tab.page.height - ch as i32) > 0;
    let cw = if needs_scroll { bw.saturating_sub(SCROLL_W) } else { bw };

    web::paint(&tab.page, tab.scroll, bx, cy, cw, ch);
    draw_focused_field(tab, bx, cy, ch);

    if needs_scroll {
        draw_scrollbar(&tab.page, tab.scroll, bx + cw, cy, SCROLL_W, ch);
    }
}

// ── Barre d'onglets ───────────────────────────────────────────────────────────

fn draw_tabbar(state: &BrowserState, bx: usize, by: usize, bw: usize) {
    fb::fill_rect_rgb(bx, by, bw, TABS_H, TAB_INACTIVE_BG);

    let (tab_w, _avail) = tab_geometry(state.tabs.len(), bw);

    for (i, tab) in state.tabs.iter().enumerate() {
        let active = i == state.active;
        let tx = bx + i * tab_w;

        let bg = if active { TAB_ACTIVE_BG } else { TAB_INACTIVE_BG };
        fb::fill_rect_rgb(tx, by, tab_w, TABS_H, bg);

        // Séparateur droite
        fb::fill_rect_rgb(tx + tab_w.saturating_sub(1), by + 2, 1, TABS_H - 4, TAB_BORDER);

        // Indicateur onglet actif : ligne bleue en bas
        if active {
            fb::fill_rect_rgb(tx, by + TABS_H - 2, tab_w.saturating_sub(1), 2, TAB_ACCENT);
        }

        // Titre (tronqué, police proportionnelle 11 px)
        let fg = if active { TAB_TEXT_ACTIVE } else { TAB_TEXT_INACT };
        let has_close = tab_w >= TAB_MIN_W + TAB_CLOSE_W;
        let title_max_w = if has_close {
            tab_w.saturating_sub(TAB_CLOSE_W + 6)
        } else {
            tab_w.saturating_sub(4)
        };
        let max_chars = title_max_w / 7; // ~7 px par glyphe à 11 px
        let title = clip_str(&tab.title, max_chars);
        fb::draw_text_prop(tx + 4, by + 4, title, fg, 11.0, false);

        // Bouton fermer
        if has_close {
            let cx = tx + tab_w - TAB_CLOSE_W + 1;
            fb::draw_text_prop(cx, by + 4, "×", TAB_TEXT_INACT, 11.0, false);
        }
    }

    // Bouton [+] nouvel onglet
    let nx = bx + state.tabs.len() * tab_w + 3;
    if nx + NEW_TAB_W < bx + bw {
        fb::fill_rect_rgb(nx, by + 4, NEW_TAB_W, TABS_H - 8, 0xe0e4e8);
        fb::draw_text_prop(nx + 4, by + 4, "+", 0x3c4043, 11.0, false);
    }

    // Bordure basse de la barre d'onglets
    fb::fill_rect_rgb(bx, by + TABS_H - 1, bw, 1, TAB_BORDER);
}

/// Calcule la largeur de chaque onglet et la largeur totale disponible.
fn tab_geometry(count: usize, bw: usize) -> (usize, usize) {
    let avail = bw.saturating_sub(NEW_TAB_W + 6);
    let w = if count == 0 { TAB_MAX_W } else { (avail / count).clamp(TAB_MIN_W, TAB_MAX_W) };
    (w, avail)
}

// ── Barre d'outils ────────────────────────────────────────────────────────────

fn draw_toolbar(state: &BrowserState, bx: usize, by: usize, bw: usize) {
    let tab = state.tab();
    fb::fill_rect_rgb(bx, by, bw, TOOLBAR_H, TOOLBAR_BG);

    let btn_y = by + (TOOLBAR_H - BTN_H) / 2;

    // Boutons de navigation
    draw_nav_btn(bx + BACK_BX,    btn_y, "<", tab.can_back());
    draw_nav_btn(bx + FWD_BX,     btn_y, ">", tab.can_forward());
    draw_nav_btn(bx + REFRESH_BX, btn_y, "R", true);
    draw_nav_btn(bx + HOME_BX,    btn_y, "~", true);

    // Barre d'adresse
    let addr_right = bw.saturating_sub(ADDR_R_PAD);
    if ADDR_BX >= addr_right { return; }
    let addr_w = addr_right - ADDR_BX;
    let addr_bx = bx + ADDR_BX;
    let addr_by = by + (TOOLBAR_H - ADDR_H) / 2;

    let is_typing = tab.input != tab.url;
    let border_c = if is_typing { ADDR_BDR_FOC } else { ADDR_BDR_DEF };

    fb::fill_rect_rgb(addr_bx, addr_by, addr_w, ADDR_H, ADDR_BG);
    rect_rgb(addr_bx, addr_by, addr_w, ADDR_H, border_c);

    // Icône de protocole (proportionnel 11 px)
    let (proto_icon, proto_fg) = proto_indicator(&tab.url);
    fb::draw_text_prop(addr_bx + 3, addr_by + 2, proto_icon, proto_fg, 11.0, true);

    // Texte de l'URL (proportionnel 11 px, tronqué si trop long)
    let text_x  = addr_bx + 14;
    let max_chars = addr_w.saturating_sub(16) / 7; // ~7 px/glyph à 11 px
    let raw_url = if is_typing { &tab.input } else { &tab.url };
    let shown   = if raw_url.len() > max_chars { &raw_url[raw_url.len() - max_chars..] } else { raw_url };
    let display = if is_typing { format!("{}_", shown) } else { shown.to_string() };
    fb::draw_text_prop(text_x, addr_by + 2, clip_str(&display, max_chars + 1), ADDR_FG, 11.0, false);

    // Séparateur bas
    fb::fill_rect_rgb(bx, by + TOOLBAR_H - 1, bw, 1, SEP_COLOR);
}

fn proto_indicator(url: &str) -> (&'static str, u32) {
    if url.starts_with("https://") { ("S", ADDR_HTTPS_FG) }
    else if url.starts_with("http://") { ("!", ADDR_HTTP_FG) }
    else { ("@", ADDR_HINT_FG) }
}

fn draw_nav_btn(bx: usize, by: usize, label: &str, enabled: bool) {
    let fg = if enabled { BTN_FG_ON } else { BTN_FG_OFF };
    fb::fill_rect_rgb(bx, by, BTN_W, BTN_H, BTN_BG);
    fb::draw_text_prop(bx + 3, by + 1, label, fg, 12.0, true);
}

// ── Ascenseur ─────────────────────────────────────────────────────────────────

/// Overlay du champ de formulaire focalise : redessine le fond, le texte en
/// cours de saisie et un caret, par-dessus la page peinte (evite un relayout
/// complet a chaque frappe).
fn draw_focused_field(tab: &crate::browser::state::Tab, bx: usize, cy: usize, ch: usize) {
    let Some(i) = tab.focused_field else { return };
    let Some(fld) = tab.page.fields.get(i) else { return };
    let y_scr = fld.y - tab.scroll;
    if y_scr + fld.h <= 0 || y_scr >= ch as i32 || fld.x < 0 { return; }
    let (x, y) = (bx as i32 + fld.x, cy as i32 + y_scr);
    // Fond blanc + bord bleu (focus) + texte + caret.
    fb::fill_rect_rgb(x as usize, y as usize, fld.w.max(2) as usize, fld.h.max(2) as usize, 0x1a73e8);
    fb::fill_rect_rgb((x + 1) as usize, (y + 1) as usize, (fld.w - 2).max(1) as usize, (fld.h - 2).max(1) as usize, 0xffffff);
    let shown: alloc::string::String = alloc::format!("{}_", tab.field_text);
    let maxc = ((fld.w - 12) / 8).max(1) as usize;
    let clip = if shown.len() > maxc { &shown[shown.len() - maxc..] } else { &shown[..] };
    fb::draw_text_prop(x as usize + 6, y as usize + (fld.h.max(18) as usize).saturating_sub(16) / 2, clip, 0x202124, 13.0, false);
}

fn draw_scrollbar(page: &web::Page, scroll: i32, bx: usize, by: usize, bw: usize, bh: usize) {
    let max_s = (page.height - bh as i32).max(0);
    if max_s <= 0 || bh < 12 { return; }

    fb::fill_rect_rgb(bx, by, bw, bh, SCROLL_TRACK);

    let track_h = bh as i32;
    let thumb_h = ((track_h * track_h) / page.height.max(track_h)).clamp(10, track_h);
    let travel  = (track_h - thumb_h).max(1);
    let thumb_y = by as i32 + (scroll.clamp(0, max_s) * travel) / max_s;

    fb::fill_rect_rgb(bx + 1, thumb_y as usize, bw.saturating_sub(2), thumb_h as usize, SCROLL_THUMB);
}

// ── Écran de chargement ───────────────────────────────────────────────────────

/// Affiche un écran de chargement animé (à appeler avant le fetch bloquant).
pub fn draw_loading(url: &str, bx: usize, by: usize, bw: usize, bh: usize) {
    fb::fill_rect_rgb(bx, by, bw, bh, 0xffffff);
    fb::draw_text_prop(bx + 10, by + 14, "Chargement...", 0x1a73e8, 20.0, false);
    let max_u = bw.saturating_sub(10) / 7;
    fb::draw_text_prop(bx + 10, by + 44, clip_url(url, max_u), 0x9aa0a6, 11.0, false);
    // Barre de progression
    let pw = (bw * 2 / 5).max(4);
    fb::fill_rect_rgb(bx, by + bh.saturating_sub(3), pw, LOAD_H, LOAD_FG);
}

// ── Gestion des événements ────────────────────────────────────────────────────

/// Gère un clic dans la zone du navigateur.
/// `rel_x/rel_y` sont relatifs au coin supérieur-gauche du corps de fenêtre.
pub fn on_click(state: &BrowserState, rel_x: i32, rel_y: i32, bw: usize, bh: usize) -> ChromeEvent {
    if bh < CHROME_H { return ChromeEvent::None; }

    if rel_y >= 0 && (rel_y as usize) < TABS_H {
        return click_tabbar(state, rel_x, bw);
    }
    if rel_y >= TABS_H as i32 && (rel_y as usize) < CHROME_H {
        let ty = rel_y - TABS_H as i32;
        return click_toolbar(state, rel_x, ty, bw);
    }
    let cy = rel_y - CHROME_H as i32;
    let ch = bh.saturating_sub(CHROME_H);
    click_content(state, rel_x, cy, bw, ch)
}

fn click_tabbar(state: &BrowserState, rel_x: i32, bw: usize) -> ChromeEvent {
    let (tab_w, _) = tab_geometry(state.tabs.len(), bw);
    for i in 0..state.tabs.len() {
        let tx = i * tab_w;
        if rel_x >= tx as i32 && rel_x < (tx + tab_w) as i32 {
            // Clic sur le X ?
            if tab_w >= TAB_MIN_W + TAB_CLOSE_W {
                let close_x = tx + tab_w - TAB_CLOSE_W;
                if rel_x >= close_x as i32 { return ChromeEvent::CloseTab(i); }
            }
            return ChromeEvent::SelectTab(i);
        }
    }
    // Bouton [+]
    let nx = state.tabs.len() * tab_w + 3;
    if rel_x >= nx as i32 && rel_x < (nx + NEW_TAB_W) as i32 {
        return ChromeEvent::NewTab;
    }
    ChromeEvent::None
}

fn click_toolbar(state: &BrowserState, rel_x: i32, rel_y: i32, bw: usize) -> ChromeEvent {
    let tab = state.tab();
    let btn_y0 = (TOOLBAR_H - BTN_H) / 2;
    let btn_y1 = btn_y0 + BTN_H;
    let in_row = rel_y >= btn_y0 as i32 && rel_y < btn_y1 as i32;
    if in_row {
        if in_btn(rel_x, BACK_BX)    { return ChromeEvent::Back; }
        if in_btn(rel_x, FWD_BX)     { return ChromeEvent::Forward; }
        if in_btn(rel_x, REFRESH_BX) { return ChromeEvent::Refresh; }
        if in_btn(rel_x, HOME_BX)    { return ChromeEvent::Home; }
    }
    // Clic dans la barre d'adresse → focus (pas d'action)
    let addr_right = bw.saturating_sub(ADDR_R_PAD);
    if rel_x >= ADDR_BX as i32 && (rel_x as usize) < addr_right {
        let addr_by0 = (TOOLBAR_H - ADDR_H) / 2;
        let addr_by1 = addr_by0 + ADDR_H;
        if rel_y >= addr_by0 as i32 && rel_y < addr_by1 as i32 {
            let _ = tab;
            return ChromeEvent::None;
        }
    }
    ChromeEvent::None
}

fn click_content(state: &BrowserState, rel_x: i32, rel_y: i32, bw: usize, ch: usize) -> ChromeEvent {
    let tab = state.tab();
    let needs_scroll = (tab.page.height - ch as i32) > 0;
    let content_w = if needs_scroll { bw.saturating_sub(SCROLL_W) } else { bw };

    // Clic dans l'ascenseur
    if needs_scroll && rel_x >= content_w as i32 {
        let max_s  = (tab.page.height - ch as i32).max(0);
        let track_h = ch as i32;
        let thumb_h = ((track_h * track_h) / tab.page.height.max(track_h)).clamp(10, track_h);
        let travel  = (track_h - thumb_h).max(1);
        let y       = rel_y.clamp(0, track_h - 1);
        let new_s   = ((y - thumb_h / 2).clamp(0, travel) * max_s) / travel;
        return ChromeEvent::ScrollTo(new_s);
    }

    // Clic sur un lien dans le contenu
    let cy_doc = rel_y + tab.scroll;
    for lnk in &tab.page.links {
        if rel_x >= lnk.x && rel_x < lnk.x + lnk.w && cy_doc >= lnk.y && cy_doc < lnk.y + lnk.h {
            let href = lnk.href.clone();
            if let Some(code) = href.strip_prefix("javascript:") {
                return ChromeEvent::DispatchJs(code.to_string());
            }
            return ChromeEvent::Navigate(href);
        }
    }
    // Clic sur un champ de formulaire : prend le focus (saisie clavier ensuite).
    for (i, fld) in tab.page.fields.iter().enumerate() {
        if rel_x >= fld.x && rel_x < fld.x + fld.w && cy_doc >= fld.y && cy_doc < fld.y + fld.h {
            return ChromeEvent::FocusField(i);
        }
    }
    // Clic ailleurs : retire le focus eventuel.
    if tab.focused_field.is_some() { return ChromeEvent::Blur; }
    ChromeEvent::None
}

/// Gère une touche clavier dans la barre d'adresse ou la page.
pub fn on_key(state: &BrowserState, key: Key, bh: usize) -> ChromeEvent {
    let tab = state.tab();
    let ch  = bh.saturating_sub(CHROME_H);
    let max_s = (tab.page.height - ch as i32).max(0);
    // Un champ de page a le focus : le clavier tape dedans (Entree = soumettre).
    if tab.focused_field.is_some() {
        match key {
            Key::Enter     => return ChromeEvent::SubmitField,
            Key::Backspace => return ChromeEvent::FieldBackspace,
            Key::Char(c)   => return ChromeEvent::FieldChar(c as char),
            _ => {}
        }
    }
    match key {
        Key::Enter    => ChromeEvent::Navigate(tab.input.clone()),
        Key::Up       => ChromeEvent::ScrollTo((tab.scroll - 48).max(0)),
        Key::Down     => ChromeEvent::ScrollTo((tab.scroll + 48).min(max_s)),
        Key::Backspace => ChromeEvent::InputBackspace,
        Key::Char(c)  => ChromeEvent::InputChar(c as char),
        _             => ChromeEvent::None,
    }
}

/// Gère la molette de la souris dans la zone de contenu.
pub fn on_wheel(state: &BrowserState, delta: i32, bh: usize) -> ChromeEvent {
    if delta == 0 { return ChromeEvent::None; }
    let tab   = state.tab();
    let ch    = bh.saturating_sub(CHROME_H);
    let max_s = (tab.page.height - ch as i32).max(0);
    ChromeEvent::ScrollTo((tab.scroll - delta * 48).clamp(0, max_s))
}

// ── Utilitaires internes ──────────────────────────────────────────────────────

fn in_btn(rel_x: i32, btn_bx: usize) -> bool {
    rel_x >= btn_bx as i32 && rel_x < (btn_bx + BTN_W) as i32
}

fn rect_rgb(x: usize, y: usize, w: usize, h: usize, color: u32) {
    if w < 2 || h < 2 { return; }
    fb::fill_rect_rgb(x,         y,         w, 1, color);
    fb::fill_rect_rgb(x,         y + h - 1, w, 1, color);
    fb::fill_rect_rgb(x,         y,         1, h, color);
    fb::fill_rect_rgb(x + w - 1, y,         1, h, color);
}

fn clip_str(s: &str, n: usize) -> &str {
    if s.len() <= n { s } else { &s[..n] }
}

fn clip_url(s: &str, n: usize) -> &str {
    if s.len() <= n { s } else { &s[s.len() - n..] }
}
