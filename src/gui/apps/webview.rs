//! WebView — navigation interactive sur le web moderne via le proxy Chromium.
//!
//! Architecture navigateur "cloud" (Opera Mini, Puffin, Amazon Silk) : un
//! Chromium headless tourne sur l'hote (`tools/render-proxy`), la fenetre de
//! l'OS affiche la capture PNG du viewport et lui renvoie chaque interaction
//! (clic, molette, clavier). Tout ce que Chrome sait faire marche donc ici :
//! JavaScript, SPA, formulaires, connexion a un compte...
//!
//! Coordonnees 1:1 : le viewport Chromium est aligne sur la taille du corps
//! de fenetre (parametres `w`/`h` envoyes a chaque action), la capture n'est
//! jamais re-echelonnee -- un clic en (x, y) dans la fenetre est un clic en
//! (x, y) dans la page.
//!
//! La barre d'adresse est locale a l'OS ; le reste du clavier part vers la
//! page distante des qu'on a clique dans le contenu.

use crate::browser::loader::{pct_encode, proxy_host};
use crate::drivers::keyboard::Key;
use crate::gui::framebuffer as fb;
use crate::gui::image::{self, Image};
use alloc::format;
use alloc::string::{String, ToString};

/// Hauteur de la barre d'outils (boutons + barre d'adresse).
pub const TOOLBAR_H: usize = 26;

pub struct WebViewState {
    /// Texte de la barre d'adresse (edition locale).
    pub input: String,
    /// URL courante rapportee par le proxy.
    pub url: String,
    /// Derniere capture du viewport distant.
    pub img: Option<Image>,
    /// Message d'etat (accueil, chargement, erreurs).
    pub status: String,
    /// Focus clavier : barre d'adresse (true) ou page distante (false).
    pub focus_addr: bool,
}

impl WebViewState {
    pub fn new() -> Self {
        WebViewState {
            input: "https://example.com".to_string(),
            url: String::new(),
            img: None,
            status: "Entre une URL puis Entree. (Proxy Chromium requis sur l'hote : \
                     cd tools/render-proxy && npm start)"
                .to_string(),
            focus_addr: true,
        }
    }
}

// ── Reseau ────────────────────────────────────────────────────────────────────

/// Taille du viewport distant pour un corps de fenetre donne.
fn viewport(bw: usize, bh: usize) -> (usize, usize) {
    (bw.max(160), bh.saturating_sub(TOOLBAR_H).max(120))
}

/// Affiche un bandeau de chargement immediatement (le fetch est bloquant).
fn draw_busy(state: &WebViewState, bx: usize, by: usize, bw: usize, label: &str) {
    let _ = state;
    fb::fill_rect_rgb(bx, by + TOOLBAR_H, bw, 16, 0x2d5a88);
    fb::draw_text_rgb(bx + 6, by + TOOLBAR_H + 4, label, 0xffffff, 1);
    fb::present();
}

/// Appelle une action `/wv/...` du proxy et met a jour la capture.
/// `sync_url` recharge ensuite l'URL courante (actions de navigation).
fn api(state: &mut WebViewState, action: &str, params: &str, bw: usize, bh: usize, sync_url: bool) {
    let (vw, vh) = viewport(bw, bh);
    let url = format!("http://{}/wv/{}?{}{}w={}&h={}", proxy_host(), action, params,
                      if params.is_empty() { "" } else { "&" }, vw, vh);
    let doc = crate::net::fetch_document(&url);
    if doc.ok && doc.content_type.contains("image/png") {
        if let Some(img) = image::decode(&doc.body) {
            state.img = Some(img);
            state.status.clear();
        } else {
            state.status = "capture illisible (decodeur PNG)".to_string();
        }
    } else if doc.ok {
        // Le proxy repond mais en texte : message d'erreur cote Chromium.
        let mut msg = String::new();
        for &b in doc.body.iter().take(160) { msg.push(b as char); }
        state.status = msg;
    } else {
        state.status = format!(
            "proxy injoignable sur {} (npm start dans tools/render-proxy + pare-feu port 8080)",
            proxy_host()
        );
    }
    if sync_url && state.img.is_some() {
        let doc = crate::net::fetch_document(&format!("http://{}/wv/url", proxy_host()));
        if doc.ok {
            let mut u = String::new();
            for &b in doc.body.iter().take(300) { u.push(b as char); }
            let u = u.trim().to_string();
            if !u.is_empty() && u != "about:blank" {
                state.url = u.clone();
                if !state.focus_addr {
                    state.input = u;
                }
            }
        }
    }
}

fn navigate(state: &mut WebViewState, bx: usize, by: usize, bw: usize, bh: usize) {
    let mut target = state.input.trim().to_string();
    if target.is_empty() { return; }
    // Un mot sans point -> recherche ; sinon URL (https par defaut cote proxy).
    if !target.contains('.') && !target.contains("://") {
        target = format!("https://duckduckgo.com/?q={}", pct_encode(&target));
    }
    draw_busy(state, bx, by, bw, "Chargement...");
    let params = format!("url={}", pct_encode(&target));
    api(state, "open", &params, bw, bh, true);
    state.focus_addr = false;
}

// ── Evenements ────────────────────────────────────────────────────────────────

pub fn on_key(state: &mut WebViewState, k: Key, bx: usize, by: usize, bw: usize, bh: usize) {
    if state.focus_addr {
        match k {
            Key::Enter => navigate(state, bx, by, bw, bh),
            Key::Backspace => { state.input.pop(); }
            Key::Char(c) => {
                if state.input.len() < 300 { state.input.push(c as char); }
            }
            Key::Tab => state.focus_addr = false,
            _ => {}
        }
        return;
    }
    // Focus page : tout part vers le Chromium distant.
    let key_name = match k {
        Key::Char(c) => {
            let s = format!("text={}", pct_encode(&(c as char).to_string()));
            api(state, "type", &s, bw, bh, false);
            return;
        }
        Key::Enter => "Enter",
        Key::Backspace => "Backspace",
        Key::Tab => "Tab",
        Key::Up => "ArrowUp",
        Key::Down => "ArrowDown",
        Key::Left => "ArrowLeft",
        Key::Right => "ArrowRight",
        _ => return,
    };
    if key_name == "Enter" {
        draw_busy(state, bx, by, bw, "...");
    }
    let params = format!("k={}", key_name);
    api(state, "key", &params, bw, bh, key_name == "Enter");
}

/// Clic dans le corps de fenetre, coordonnees relatives au corps.
pub fn on_click(state: &mut WebViewState, rel_x: i32, rel_y: i32, bx: usize, by: usize, bw: usize, bh: usize) {
    if rel_y < TOOLBAR_H as i32 {
        // Barre d'outils : [<] [>] [O] [ adresse... ]
        match rel_x {
            x if x < 22 => {
                draw_busy(state, bx, by, bw, "Retour...");
                api(state, "back", "", bw, bh, true);
            }
            x if x < 44 => {
                draw_busy(state, bx, by, bw, "Suivant...");
                api(state, "forward", "", bw, bh, true);
            }
            x if x < 66 => {
                draw_busy(state, bx, by, bw, "Rechargement...");
                api(state, "reload", "", bw, bh, true);
            }
            _ => state.focus_addr = true,
        }
        return;
    }
    // Contenu : clic transmis 1:1 au Chromium distant.
    state.focus_addr = false;
    let px = rel_x.max(0);
    let py = rel_y - TOOLBAR_H as i32;
    if py < 0 { return; }
    draw_busy(state, bx, by, bw, "...");
    let params = format!("x={}&y={}", px, py);
    api(state, "click", &params, bw, bh, true);
}

pub fn on_wheel(state: &mut WebViewState, delta: i32, bw: usize, bh: usize) {
    if delta == 0 { return; }
    // delta > 0 = molette vers le haut -> la page distante defile vers le haut.
    let dy = -delta * 80;
    let params = format!("dy={}", dy);
    api(state, "scroll", &params, bw, bh, false);
}

// ── Rendu ─────────────────────────────────────────────────────────────────────

pub fn draw(state: &WebViewState, bx: usize, by: usize, bw: usize, bh: usize) {
    // Barre d'outils.
    fb::fill_rect_rgb(bx, by, bw, TOOLBAR_H, 0xd8d8d8);
    for (i, label) in ["<", ">", "O"].iter().enumerate() {
        let x = bx + 2 + i * 22;
        fb::fill_rect_rgb(x, by + 3, 20, TOOLBAR_H - 6, 0xbfbfbf);
        fb::draw_text_rgb(x + 7, by + 8, label, 0x000000, 1);
    }
    // Barre d'adresse.
    let ax = bx + 70;
    let aw = bw.saturating_sub(74);
    let abg = if state.focus_addr { 0xffffff } else { 0xf0f0f0 };
    fb::fill_rect_rgb(ax, by + 3, aw, TOOLBAR_H - 6, abg);
    let max_chars = (aw.saturating_sub(10)) / 8;
    let shown: String = if state.input.len() > max_chars {
        state.input[state.input.len() - max_chars..].to_string()
    } else {
        state.input.clone()
    };
    let caret = if state.focus_addr { "_" } else { "" };
    fb::draw_text_rgb(ax + 4, by + 8, &format!("{}{}", shown, caret), 0x000000, 1);

    // Corps : capture distante ou message d'etat.
    let cy = by + TOOLBAR_H;
    let ch = bh.saturating_sub(TOOLBAR_H);
    fb::fill_rect_rgb(bx, cy, bw, ch, 0xffffff);
    if let Some(img) = &state.img {
        fb::blit_rgb(bx, cy, img.w, img.h.min(ch), &img.pix, bx, cy, bw, ch);
    }
    if !state.status.is_empty() {
        // Message d'etat sur plusieurs lignes de 8 px.
        let cols = (bw.saturating_sub(16) / 8).max(10);
        let mut y = cy + 10;
        let mut rest = state.status.as_str();
        while !rest.is_empty() && y + 10 < cy + ch {
            let n = rest.len().min(cols);
            let (line, r) = rest.split_at(n);
            fb::draw_text_rgb(bx + 8, y, line, 0x333333, 1);
            rest = r;
            y += 12;
        }
    }
}
