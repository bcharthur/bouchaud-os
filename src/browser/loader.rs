//! Chargement d'URL pour Nautile.
//!
//! Inspiré du pipeline réseau de Firefox (Necko) et du gestionnaire de navigation
//! de Chromium (NavigationController). Principe : local-first souverain — le rendu
//! est assuré par le moteur web intégré de l'OS ; le mode compat (proxy Chromium
//! externe) n'est accessible que via le préfixe `compat:`.

use crate::gui::web::{Page, Session};
use crate::fs::ramfs;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Active le proxy Chromium pour TOUTES les URL http/https (non recommandé).
/// `false` = navigation souveraine par défaut. Le mode compat reste accessible
/// via le préfixe `compat:` sans modifier cette constante.
const ENABLE_COMPAT_PROXY: bool = false;

/// Hôtes candidats du service de rendu déporté (voir `tools/render-proxy`),
/// essayés dans l'ordre : la passerelle SLIRP standard de QEMU (10.0.2.2 =
/// l'hôte, marche sans configuration réseau) PUIS l'IP LAN de l'hôte (utile si
/// l'OS tourne sur du matériel réel ou un réseau ponté). Le premier qui répond
/// à `/healthz` est mémorisé pour la session.
pub(crate) const PROXY_HOSTS: &[&str] = &["10.0.2.2:8080", "192.168.1.187:8080"];

/// Hôte du proxy effectivement joignable (sondé une fois via /healthz, puis
/// mémorisé). Tant qu'aucun ne répond, on ne mémorise rien (nouvel essai au
/// prochain appel — le service a pu être lancé entre-temps).
pub(crate) fn proxy_host() -> &'static str {
    static mut CACHED: Option<&'static str> = None;
    unsafe {
        if let Some(h) = *core::ptr::addr_of!(CACHED) {
            return h;
        }
    }
    for h in PROXY_HOSTS {
        let doc = crate::net::fetch_document(&format!("http://{}/healthz", h));
        if doc.ok {
            unsafe { CACHED = Some(h); }
            return h;
        }
    }
    PROXY_HOSTS[0]
}

// ── API publique ──────────────────────────────────────────────────────────────

/// Charge une URL et renvoie (Session interactive, Page rendue).
/// C'est le point d'entrée unique du pipeline de chargement de Nautile.
pub fn open(url: &str, width: i32, height: i32) -> (Session, Page) {
    let width = width.max(80);
    let height = height.max(80);
    // Pages internes
    if url == "about:bouchaud" {
        return from_html(super::pages::bouchaud_home().as_bytes(), url, width, height);
    }

    if url == "about:calc" {
        return from_html(super::pages::CALC_APP.as_bytes(), url, width, height);
    }
    if url == "about:wasm" {
        return from_html(super::pages::WASM_DEMO.as_bytes(), url, width, height);
    }
    if url == "about:modern" {
        return from_html(super::pages::modern_demo().as_bytes(), url, width, height);
    }
    if url == "about:system" {
        return from_html(super::pages::system_info().as_bytes(), url, width, height);
    }
    // Matrice de compatibilite : la page teste le moteur (DOM/URL/events/
    // modules/JS/storage) et logge son score COMPAT dans le journal.
    if url == "about:compat" {
        return from_html(super::pages::COMPAT_SUITE.as_bytes(), url, width, height);
    }
    // Journal d'activite de la pile (devlog) — diagnostic du rendu.
    if url == "about:log" {
        return from_html(crate::diag::render_html().as_bytes(), url, width, height);
    }
    // Fichier local (RAMFS)
    if let Some(path) = url.strip_prefix("file:") {
        return load_file(path, url, width, height);
    }
    // Mode compat forcé par préfixe
    if let Some(rest) = url.strip_prefix("compat:") {
        return compat_render(rest, width, height);
    }
    // HTTP / HTTPS : récupération + rendu par le moteur intégré (souverain).
    if url.starts_with("http://") || url.starts_with("https://") {
        if ENABLE_COMPAT_PROXY { return compat_render(url, width, height); }
        return local_render(url, width, height);
    }
    let html = format!(
        "<h2>Page inconnue</h2><p>{}</p>\
         <p>Essaie <a href=\"about:bouchaud\">about:bouchaud</a> \
         ou <a href=\"https://example.com/\">example.com</a>.</p>",
        esc(url)
    );
    from_html(html.as_bytes(), url, width, height)
}

/// Normalise la saisie de la barre d'adresse en URL.
/// Un nombre seul suit le lien correspondant dans la page courante.
pub fn resolve_input(input: &str, page: &Page) -> String {
    let t = input.trim();
    if !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) {
        if let Ok(n) = t.parse::<usize>() {
            if n >= 1 && n <= page.links.len() {
                return page.links[n - 1].href.clone();
            }
        }
    }
    if t.contains("://") || t.starts_with("about:") || t.starts_with("file:") || t.starts_with("compat:") {
        return t.to_string();
    }
    if t.is_empty() { return t.to_string(); }
    // Raccourcis moteur "!bang" : !g google, !ddg duckduckgo, !b bing,
    // !w wikipedia, !yt youtube, !so stackoverflow. Sinon moteur par defaut.
    if let Some(rest) = t.strip_prefix('!') {
        let (bang, query) = match rest.find(' ') {
            Some(i) => (&rest[..i], rest[i + 1..].trim()),
            None => (rest, ""),
        };
        if !query.is_empty() {
            return search_url(bang, query);
        }
    }
    // Un mot ressemblant à un domaine (un point, pas d'espace) -> URL directe.
    if t.contains('.') && !t.contains(' ') {
        return format!("https://{}", t);
    }
    // Sinon : recherche via le moteur par defaut (barre d'adresse = recherche).
    search_url(DEFAULT_ENGINE, t)
}

/// Moteur de recherche par défaut (clé de `search_url`). DuckDuckGo HTML :
/// 100% rendu côté serveur (sans JavaScript), donc affichable de façon fiable
/// par le moteur autonome Nautile — contrairement à Google (page 100% JS que
/// Nautile ne peut pas rendre). Modifiable via les bangs (`!g`, `!b`, ...).
const DEFAULT_ENGINE: &str = "ddg";

/// Construit l'URL de recherche pour un moteur donné. Couvre les principaux
/// moteurs ; un bang inconnu retombe sur Google.
fn search_url(engine: &str, query: &str) -> String {
    let q = pct_encode(query);
    match engine {
        // Endpoint HTML de DuckDuckGo : 100% server-rendered (sans JS), rendu
        // fiable par le moteur — ideal comme moteur de secours.
        "ddg" | "d" | "duck" => format!("https://html.duckduckgo.com/html/?q={}", q),
        "b" | "bing" => format!("https://www.bing.com/search?q={}", q),
        "w" | "wiki" => format!("https://fr.wikipedia.org/w/index.php?search={}", q),
        "yt" | "youtube" => format!("https://www.youtube.com/results?search_query={}", q),
        "so" => format!("https://stackoverflow.com/search?q={}", q),
        "br" | "brave" => format!("https://search.brave.com/search?q={}", q),
        "sp" | "startpage" => format!("https://www.startpage.com/sp/search?query={}", q),
        _ => format!("https://www.google.com/search?q={}", q),
    }
}

// ── Rendu local (souverain) ───────────────────────────────────────────────────

/// Mettre a `true` pour deverser le HTML brut recu sur la console serie
/// (boot.log / -serial stdio). Utile pour inspecter ce que renvoie un site.
const DUMP_PAGE_HTML: bool = false;

/// Deverse le HTML brut sur la console serie, encadre de marqueurs, en
/// morceaux pour ne pas saturer le tampon UART. A lire dans `boot.log`.
fn dump_page_to_serial(url: &str, body: &[u8]) {
    if !DUMP_PAGE_HTML { return; }
    let text = String::from_utf8_lossy(body);
    crate::serial_println!("");
    crate::serial_println!("===== NAUTILE PAGE DUMP BEGIN ({} o) {} =====", body.len(), url);
    // Ecrit par tranches de lignes pour rester lisible et laisser respirer l'UART.
    for line in text.split('\n') {
        crate::serial_println!("{}", line);
    }
    crate::serial_println!("===== NAUTILE PAGE DUMP END {} =====", url);
    crate::serial_println!("");
}

// Chronometre le chargement complet (reseau + DOM + CSS + layout) et logge un
// total en millisecondes reelles a la fin : les logs par phase (Mc/ms dans
// NET/DOM/CSS/LAYOUT) permettent de reperer QUELLE etape est lente, cette
// ligne repond directement a "combien de temps a pris ce chargement ?".
fn local_render(url: &str, width: i32, height: i32) -> (Session, Page) {
    let t0 = crate::kernel::timer::cycles_since_boot();
    let result = local_render_inner(url, width, height);
    let ms = crate::kernel::timer::cycles_to_ms(crate::kernel::timer::cycles_since_boot().wrapping_sub(t0));
    crate::dlog!(crate::diag::Cat::Info, "--- chargement termine: {} en {}ms ---", url, ms);
    result
}

fn local_render_inner(url: &str, width: i32, height: i32) -> (Session, Page) {
    crate::dlog!(crate::diag::Cat::Info, "--- navigation: {} ---", url);
    let doc = crate::net::fetch_document(url);
    if doc.ok && doc.is_html && !doc.body.is_empty() {
        dump_page_to_serial(&doc.final_url, &doc.body);
        // Pages reseau : on execute le JS inline de la page (best-effort), borne
        // par le budget du moteur JS (max steps, scripts > 256 Ko ignores), pour
        // rendre le contenu construit dynamiquement. Repli statique si le JS echoue.
        return from_html(&doc.body, &doc.final_url, width, height);
    }
    if doc.ok {
        // Document non-HTML : aperçu texte + métadonnées
        let mut preview = String::new();
        for &b in doc.body.iter().take(40_000) {
            match b {
                b'\n' | b'\r' | b'\t' => preview.push(b as char),
                0x20..=0x7e => preview.push(b as char),
                _ => preview.push('.'),
            }
        }
        let mut info = String::new();
        for line in doc.banner.iter().take(6) { info.push_str(&esc(line)); info.push('\n'); }
        let html = format!(
            "<h2>Document non-HTML</h2>\
             <ul><li>URL : {u}</li><li>Type : {ct}</li>\
             <li>Taille : {sz} octets</li></ul>\
             <pre>{info}</pre><hr><pre>{pv}</pre>",
            u = esc(&doc.final_url), ct = esc(&doc.content_type),
            sz = doc.body.len(), info = info, pv = esc(&preview)
        );
        return from_html(html.as_bytes(), &doc.final_url, width, height);
    }
    // Erreur réseau / DNS / TLS
    let mut info = String::new();
    for line in doc.banner.iter().take(12) { info.push_str(&esc(line)); info.push('\n'); }
    let html = format!(
        "<h2>Impossible de charger la page</h2>\
         <p>URL : {u}</p><pre>{info}</pre>\
         <p><a href=\"about:bouchaud\">← Accueil Nautile</a></p>",
        u = esc(url), info = info
    );
    from_html(html.as_bytes(), url, width, height)
}

// ── Mode COMPAT (optionnel, non souverain) ────────────────────────────────────

fn compat_render(url: &str, width: i32, height: i32) -> (Session, Page) {
    let purl = format!("http://{}/render?url={}", proxy_host(), pct_encode(url));
    let doc  = crate::net::fetch_document(&purl);
    if doc.ok && !doc.body.is_empty() {
        if let Some(img) = crate::gui::image::decode(&doc.body) {
            let cw     = width.max(1) as usize;
            let scaled = crate::gui::image::downscale(&img, cw, 1_000_000);
            let iw = scaled.w as i32;
            let ih = scaled.h as i32;
            let page = Page {
                title: url.to_string(),
                items: alloc::vec![crate::gui::web::Item::Image { x: 0, y: 0, w: iw, h: ih, idx: 0 }],
                links: Vec::new(),
                fields: Vec::new(),
                images: alloc::vec![scaled],
                height: ih.max(1),
                bg: 0xffffff,
                layers: Vec::new(),
            };
            let (sess, _) = Session::open(b"", url, width, height);
            return (sess, page);
        }
    }
    let mut info = String::new();
    for line in doc.banner.iter().take(6) { info.push_str(&esc(line)); info.push('\n'); }
    let html = format!(
        "<h2>Mode compat indisponible</h2>\
         <p>Le proxy Chromium externe est injoignable ou n'a pas renvoyé d'image.</p>\
         <p>Lance le service :</p>\
         <pre>cd tools/render-proxy\nnpm run setup\nnpm start</pre>\
         <p>Proxy attendu : <b>http://{host}</b></p>\
         <p>URL : {u}</p><pre>{info}</pre>\
         <p><a href=\"about:bouchaud\">← Accueil</a></p>",
        host = proxy_host(), u = esc(url), info = info
    );
    from_html(html.as_bytes(), url, width, height)
}

// ── Fichier local ─────────────────────────────────────────────────────────────

fn load_file(path: &str, url: &str, width: i32, height: i32) -> (Session, Page) {
    let p    = path.trim_start_matches('/');
    let full = format!("/{}", p);
    let fs   = ramfs::fs();
    let body = match fs.resolve_checked(&full, 0) {
        Ok(idx) if fs.nodes[idx].kind == ramfs::NodeKind::File && fs.can(idx, ramfs::PERM_R) => {
            let mut s = String::new();
            s.push_str(&fs.nodes[idx].content_str());
            format!("<h2>file:{}</h2><pre>{}</pre>", full, esc(&s))
        }
        Ok(_) => format!("<h2>Permission refusée</h2><p>{}</p>", full),
        _     => format!("<h2>Fichier introuvable</h2><p>{}</p>", full),
    };
    from_html(body.as_bytes(), url, width, height)
}

// ── Utilitaires ───────────────────────────────────────────────────────────────

/// Parse HTML → Session + Page (plafonnée à 4 Mo pour éviter les OOM).
/// Execute le JS inline : reserve aux pages internes (about:*, file:) qui
/// embarquent des mini-applications.
fn from_html(html: &[u8], base: &str, width: i32, height: i32) -> (Session, Page) {
    let capped = &html[..html.len().min(4_000_000)];
    Session::open(capped, base, width, height)
}

/// Variante souveraine pour les pages reseau : DOM + CSS + images SANS executer
/// le JS de la page. Voir `Session::open_static`.
fn from_html_static(html: &[u8], base: &str, width: i32, height: i32) -> (Session, Page) {
    let capped = &html[..html.len().min(4_000_000)];
    Session::open_static(capped, base, width, height)
}

fn esc(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c { '&' => o.push_str("&amp;"), '<' => o.push_str("&lt;"), '>' => o.push_str("&gt;"), _ => o.push(c) }
    }
    o
}

pub(crate) fn pct_encode(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') { o.push(b as char); }
        else { o.push('%'); o.push(hexd(b >> 4)); o.push(hexd(b)); }
    }
    o
}

fn hexd(n: u8) -> char {
    core::char::from_digit((n & 0x0f) as u32, 16).unwrap_or('0').to_ascii_uppercase()
}
