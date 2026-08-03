//! Pont entre le navigateur Nautile (Python) et le noyau.
//!
//! Nautile est ecrit en Python et s'execute par RustPython compile en WASM
//! (`crate::lang::python`). Le module WASM ne peut appeler le noyau que par
//! les fonctions WASI exposees, et cote Bouchaud OS ce sont les fonctions
//! **fichiers**. Un module Python natif serait plus direct, mais
//! `rustpython.wasm` est precompile hors ligne : lui en ajouter un imposerait
//! de reconstruire tout l'interpreteur.
//!
//! Le pont passe donc par trois pseudo-fichiers du RAMFS :
//!
//! | `/dev/nautile/net`    | requetes reseau, servies par la pile TCP/TLS |
//! | `/dev/nautile/gpu`    | commandes de dessin vers le framebuffer      |
//! | `/dev/nautile/events` | evenements clavier et souris                 |
//!
//! Le protocole est du texte, une commande par ligne : c'est ce qui se code
//! le plus simplement ici (pas de serialisation binaire a tenir des deux
//! cotes) et cela reste inspectable avec `cat`.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::fs::ramfs;
use crate::gui::framebuffer as fb;

/// Repertoire d'installation du paquet Python.
pub const PACKAGE_DIR: &str = "/usr/lib/python/nautile";
/// Repertoire des pseudo-peripheriques.
pub const DEVICE_DIR: &str = "/dev/nautile";

/// Inodes des peripheriques, resolus a l'installation (0 = non installe).
static mut DEV_NET: usize = 0;
static mut DEV_GPU: usize = 0;
static mut DEV_EVENTS: usize = 0;

/// Evenements en attente de lecture par le navigateur.
static mut EVENTS: Vec<String> = Vec::new();

/// Zone de contenu attribuee au navigateur dans le framebuffer.
static mut VIEW: (usize, usize, usize, usize) = (0, 0, 0, 0);

/// Rectangle de clip courant, pose par la commande `clip`.
static mut CLIP: Option<(i32, i32, i32, i32)> = None;

/// Fichiers du paquet Python, embarques dans le binaire comme les polices
/// et les certificats CA. Chemin relatif -> contenu.
static PACKAGE: &[(&str, &str)] = &[
    ("__init__.py", include_str!("../assets/python/nautile/__init__.py")),
    ("__main__.py", include_str!("../assets/python/nautile/__main__.py")),
    ("version.py", include_str!("../assets/python/nautile/version.py")),
    ("url.py", include_str!("../assets/python/nautile/url.py")),
    ("net.py", include_str!("../assets/python/nautile/net.py")),
    ("dom.py", include_str!("../assets/python/nautile/dom.py")),
    ("text.py", include_str!("../assets/python/nautile/text.py")),
    ("style.py", include_str!("../assets/python/nautile/style.py")),
    ("layout.py", include_str!("../assets/python/nautile/layout.py")),
    ("display_list.py", include_str!("../assets/python/nautile/display_list.py")),
    ("browser.py", include_str!("../assets/python/nautile/browser.py")),
    ("pages.py", include_str!("../assets/python/nautile/pages.py")),
    ("ua.css", include_str!("../assets/python/nautile/ua.css")),
    ("html/__init__.py", include_str!("../assets/python/nautile/html/__init__.py")),
    ("html/entities.py", include_str!("../assets/python/nautile/html/entities.py")),
    ("html/tokenizer.py", include_str!("../assets/python/nautile/html/tokenizer.py")),
    ("html/parser.py", include_str!("../assets/python/nautile/html/parser.py")),
    ("css/__init__.py", include_str!("../assets/python/nautile/css/__init__.py")),
    ("css/color.py", include_str!("../assets/python/nautile/css/color.py")),
    ("css/values.py", include_str!("../assets/python/nautile/css/values.py")),
    ("css/parser.py", include_str!("../assets/python/nautile/css/parser.py")),
    ("css/select.py", include_str!("../assets/python/nautile/css/select.py")),
    ("backends/__init__.py", include_str!("../assets/python/nautile/backends/__init__.py")),
    ("backends/base.py", include_str!("../assets/python/nautile/backends/base.py")),
    ("backends/text.py", include_str!("../assets/python/nautile/backends/text.py")),
    ("backends/bouchaud.py", include_str!("../assets/python/nautile/backends/bouchaud.py")),
];

/// Installe le paquet Python et les peripheriques dans le RAMFS.
/// Idempotent : rappeler la fonction ne duplique rien.
pub fn install() {
    install_package();
    install_devices();
}

fn mkdir_path(path: &str) -> usize {
    let fs = ramfs::fs();
    let mut current = 0usize;
    for segment in path.split('/') {
        if segment.is_empty() {
            continue;
        }
        current = match fs.find_child(current, segment) {
            Some(existing) => existing,
            None => match fs.mkdir_at(current, segment) {
                Ok(created) => created,
                Err(_) => return current,
            },
        };
    }
    current
}

fn install_package() {
    let root = mkdir_path(PACKAGE_DIR);
    if root == 0 {
        return;
    }
    for (relative, contents) in PACKAGE {
        // Cree les repertoires intermediaires (`html/`, `css/`, `backends/`).
        let mut parent = root;
        let mut name = *relative;
        if let Some(slash) = relative.rfind('/') {
            let fs = ramfs::fs();
            let mut current = root;
            for segment in relative[..slash].split('/') {
                if segment.is_empty() {
                    continue;
                }
                current = match fs.find_child(current, segment) {
                    Some(existing) => existing,
                    None => match fs.mkdir_at(current, segment) {
                        Ok(created) => created,
                        Err(_) => break,
                    },
                };
            }
            parent = current;
            name = &relative[slash + 1..];
        }

        let fs = ramfs::fs();
        let node = match fs.find_child(parent, name) {
            Some(existing) => existing,
            None => match fs.touch_at(parent, name) {
                Ok(created) => created,
                Err(_) => continue,
            },
        };
        fs.write_node_bytes(node, contents.as_bytes());
    }
}

fn install_devices() {
    let dir = mkdir_path(DEVICE_DIR);
    if dir == 0 {
        return;
    }
    let fs = ramfs::fs();
    let mut make = |name: &str| -> usize {
        let node = match fs.find_child(dir, name) {
            Some(existing) => existing,
            None => match fs.touch_at(dir, name) {
                Ok(created) => created,
                Err(_) => return 0,
            },
        };
        // Lecture et ecriture pour tous : le navigateur tourne en session
        // utilisateur, pas en root.
        fs.nodes[node].mode = 0o666;
        node
    };
    let net = make("net");
    let gpu = make("gpu");
    let events = make("events");
    unsafe {
        DEV_NET = net;
        DEV_GPU = gpu;
        DEV_EVENTS = events;
    }
}

/// Vrai si `node` est l'un des peripheriques Nautile.
pub fn is_device(node: usize) -> bool {
    if node == 0 {
        return false;
    }
    unsafe { node == DEV_NET || node == DEV_GPU || node == DEV_EVENTS }
}

/// Interception d'une ecriture WASI. Renvoie `true` si le pont a traite
/// l'ecriture — l'appelant ne doit alors rien ecrire dans le RAMFS.
pub fn device_write(node: usize, bytes: &[u8]) -> bool {
    if node == 0 {
        return false;
    }
    let (net, gpu) = unsafe { (DEV_NET, DEV_GPU) };
    if node == net {
        handle_net_request(node, bytes);
        return true;
    }
    if node == gpu {
        handle_gpu_commands(bytes);
        return true;
    }
    // `/dev/nautile/events` est en lecture seule pour le navigateur, mais le
    // noyau y depose ses evenements : une ecriture depuis WASM est ignoree.
    unsafe { node == DEV_EVENTS }
}

/// Interception d'une lecture WASI, appelee avant de servir les octets.
/// `pos` est la position courante du descripteur.
pub fn device_before_read(node: usize, pos: usize) {
    if node == 0 || pos != 0 {
        return;
    }
    let events = unsafe { DEV_EVENTS };
    if node != events {
        return;
    }
    // Une lecture depuis le debut consomme le prochain evenement ; les
    // lectures suivantes voient la fin de fichier.
    let next = unsafe {
        if EVENTS.is_empty() {
            String::new()
        } else {
            EVENTS.remove(0)
        }
    };
    let fs = ramfs::fs();
    if next.is_empty() {
        fs.write_node_bytes(node, b"");
    } else {
        let mut line = next;
        line.push('\n');
        fs.write_node_bytes(node, line.as_bytes());
    }
}

// ----------------------------------------------------------------------------
// Reseau
// ----------------------------------------------------------------------------

/// Traite `GET <url>` : effectue la requete avec la pile du noyau et depose
/// une reponse HTTP brute que Python reanalyse avec son propre analyseur.
fn handle_net_request(node: usize, bytes: &[u8]) {
    let request = core::str::from_utf8(bytes).unwrap_or("").trim();
    let mut parts = request.split_whitespace();
    let _method = parts.next().unwrap_or("GET");
    let url = match parts.next() {
        Some(u) => u,
        None => {
            store_net_error(node, "requete illisible");
            return;
        }
    };

    let document = crate::net::fetch_document(url);
    let fs = ramfs::fs();

    if !document.ok {
        let mut body = String::from("erreur reseau\n");
        for line in &document.banner {
            body.push_str(line);
            body.push('\n');
        }
        let response = alloc::format!(
            "HTTP/1.1 599 Erreur reseau\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        fs.write_node_bytes(node, response.as_bytes());
        return;
    }

    // Le corps est deja dechunke et decompresse par le noyau : on n'annonce
    // donc ni `Transfer-Encoding` ni `Content-Encoding`.
    let content_type = if document.content_type.is_empty() {
        if document.is_html { "text/html" } else { "application/octet-stream" }
    } else {
        &document.content_type
    };
    let head = alloc::format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nX-Nautile-Final-Url: {}\r\n\r\n",
        content_type,
        document.body.len(),
        document.final_url
    );
    let mut raw: Vec<u8> = Vec::with_capacity(head.len() + document.body.len());
    raw.extend_from_slice(head.as_bytes());
    raw.extend_from_slice(&document.body);
    fs.write_node_bytes(node, &raw);
}

fn store_net_error(node: usize, message: &str) {
    let response = alloc::format!(
        "HTTP/1.1 599 Erreur\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}",
        message.len(),
        message
    );
    ramfs::fs().write_node_bytes(node, response.as_bytes());
}

// ----------------------------------------------------------------------------
// Dessin
// ----------------------------------------------------------------------------

/// Definit la zone du framebuffer ou peindre (x, y, largeur, hauteur).
pub fn set_viewport(x: usize, y: usize, w: usize, h: usize) {
    unsafe {
        VIEW = (x, y, w, h);
    }
}

pub fn viewport() -> (usize, usize, usize, usize) {
    unsafe { VIEW }
}

/// Execute un bloc de commandes de dessin.
fn handle_gpu_commands(bytes: &[u8]) {
    let text = core::str::from_utf8(bytes).unwrap_or("");
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        execute_command(line);
    }
}

fn execute_command(line: &str) {
    let mut parts = line.split(' ');
    let command = parts.next().unwrap_or("");

    match command {
        "rect" => {
            if let (Some(x), Some(y), Some(w), Some(h), Some(rgb)) =
                (next_i32(&mut parts), next_i32(&mut parts), next_i32(&mut parts),
                 next_i32(&mut parts), next_u32(&mut parts))
            {
                fill_clipped(x, y, w, h, rgb);
            }
        }
        "round" => {
            // Les coins arrondis ne sont pas rasterises : un aplat rend la
            // meme surface, et l'ecart n'est pas perceptible a ces rayons.
            if let (Some(x), Some(y), Some(w), Some(h), Some(_r), Some(rgb)) =
                (next_i32(&mut parts), next_i32(&mut parts), next_i32(&mut parts),
                 next_i32(&mut parts), next_i32(&mut parts), next_u32(&mut parts))
            {
                fill_clipped(x, y, w, h, rgb);
            }
        }
        "text" => {
            if let (Some(x), Some(baseline), Some(size), Some(flags), Some(rgb)) =
                (next_i32(&mut parts), next_i32(&mut parts), next_i32(&mut parts),
                 next_i32(&mut parts), next_u32(&mut parts))
            {
                // Le reste de la ligne est le texte : il contient des espaces.
                let rest: Vec<&str> = parts.collect();
                let content = rest.join(" ");
                if !content.is_empty() {
                    draw_text_at(x, baseline, size, flags, rgb, &content);
                }
            }
        }
        "image" => {
            if let (Some(x), Some(y), Some(w), Some(h)) =
                (next_i32(&mut parts), next_i32(&mut parts), next_i32(&mut parts),
                 next_i32(&mut parts))
            {
                // Cadre en attendant le decodage : le moteur Python ne decode
                // pas les images, le noyau le fera dans un second temps.
                fill_clipped(x, y, w, h, 0xEEF0F3);
                fill_clipped(x, y, w, 1, 0xC7CCD1);
                fill_clipped(x, y + h - 1, w, 1, 0xC7CCD1);
                fill_clipped(x, y, 1, h, 0xC7CCD1);
                fill_clipped(x + w - 1, y, 1, h, 0xC7CCD1);
            }
        }
        "clip" => {
            if let (Some(x), Some(y), Some(w), Some(h)) =
                (next_i32(&mut parts), next_i32(&mut parts), next_i32(&mut parts),
                 next_i32(&mut parts))
            {
                unsafe { CLIP = Some((x, y, w, h)) };
            }
        }
        "unclip" => unsafe { CLIP = None },
        "flush" => {
            unsafe { CLIP = None };
            fb::present();
        }
        _ => {}
    }
}

fn next_i32(parts: &mut core::str::Split<'_, char>) -> Option<i32> {
    parts.next().and_then(|t| t.parse::<i32>().ok())
}

fn next_u32(parts: &mut core::str::Split<'_, char>) -> Option<u32> {
    parts.next().and_then(|t| t.parse::<u32>().ok())
}

/// Remplit un rectangle en coordonnees viewport, borne par le clip courant
/// et par la zone attribuee au navigateur.
fn fill_clipped(x: i32, y: i32, w: i32, h: i32, rgb: u32) {
    if w <= 0 || h <= 0 {
        return;
    }
    let (vx, vy, vw, vh) = unsafe { VIEW };
    let mut left = x;
    let mut top = y;
    let mut right = x + w;
    let mut bottom = y + h;

    if let Some((cx, cy, cw, ch)) = unsafe { CLIP } {
        left = left.max(cx);
        top = top.max(cy);
        right = right.min(cx + cw);
        bottom = bottom.min(cy + ch);
    }

    left = left.max(0);
    top = top.max(0);
    right = right.min(vw as i32);
    bottom = bottom.min(vh as i32);
    if right <= left || bottom <= top {
        return;
    }

    fb::fill_rect_rgb(
        vx + left as usize,
        vy + top as usize,
        (right - left) as usize,
        (bottom - top) as usize,
        rgb,
    );
}

fn draw_text_at(x: i32, baseline: i32, size: i32, flags: i32, rgb: u32, content: &str) {
    let (vx, vy, vw, vh) = unsafe { VIEW };
    // Nautile pose le texte sur sa ligne de base ; le pilote attend le haut
    // du cadratin. L'ascendante vaut environ 80 % de la taille.
    let top = baseline - (size * 4) / 5;
    if top < -size || top >= vh as i32 || x >= vw as i32 {
        return;
    }
    if let Some((cx, cy, cw, ch)) = unsafe { CLIP } {
        if top + size < cy || top > cy + ch || x < cx - size || x > cx + cw {
            return;
        }
    }
    let bold = flags & 1 != 0;
    fb::draw_text_prop(
        vx + x.max(0) as usize,
        vy + top.max(0) as usize,
        content,
        rgb,
        size as f32,
        bold,
    );
}

// ----------------------------------------------------------------------------
// Evenements
// ----------------------------------------------------------------------------

/// Depose un evenement a destination du navigateur.
pub fn push_event(event: &str) {
    unsafe {
        // Garde-fou : si le navigateur ne lit plus, la file ne doit pas
        // grossir sans fin.
        if EVENTS.len() >= 64 {
            EVENTS.remove(0);
        }
        EVENTS.push(event.to_string());
    }
}

pub fn push_click(x: i32, y: i32) {
    push_event(&alloc::format!("click {} {}", x, y));
}

pub fn push_scroll(delta: i32) {
    push_event(&alloc::format!("scroll {}", delta));
}

pub fn push_key(code: u8) {
    push_event(&alloc::format!("key {}", code));
}

pub fn push_resize(width: usize, height: usize) {
    push_event(&alloc::format!("resize {} {}", width, height));
}

pub fn push_navigate(url: &str) {
    push_event(&alloc::format!("navigate {}", url));
}

pub fn pending_events() -> usize {
    unsafe { EVENTS.len() }
}

pub fn clear_events() {
    unsafe { EVENTS.clear() };
}
