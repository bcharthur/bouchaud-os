//! Pot de cookies en mémoire (session uniquement — pas de persistance disque,
//! l'OS n'a pas encore de stockage persistant, voir docs/AUDIT.md).
//!
//! Suffisant pour les usages réels immédiats : consentements, préférences de
//! langue/région (Google `NID`/`CONSENT`), sessions de connexion le temps du
//! boot. Simplifications assumées :
//!   - `Path` ignoré (envoyé sur tout le site) ;
//!   - `Expires` ignoré (cookies de session), sauf suppression explicite
//!     (`Max-Age=0` ou date 1970) qui retire le cookie ;
//!   - `Secure`/`HttpOnly`/`SameSite` acceptés mais non contraints (mono-
//!     utilisateur, pas de JS tiers).
//!
//! Envoi : `header_for(hôte)` — cookie « host-only » sur l'hôte exact,
//! cookie avec `Domain=` sur le domaine et ses sous-domaines.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

struct Cookie {
    domain: String,   // sans point initial, en minuscules
    host_only: bool,  // pas d'attribut Domain= : hôte exact seulement
    name: String,
    value: String,
}

// Mono-thread (comme le reste du noyau).
static mut JAR: Option<Vec<Cookie>> = None;
const MAX_COOKIES: usize = 200;

fn jar() -> &'static mut Vec<Cookie> {
    unsafe {
        if JAR.is_none() { JAR = Some(Vec::new()); }
        JAR.as_mut().unwrap()
    }
}

/// Enregistre tous les `Set-Cookie` de la zone d'en-tête d'une réponse brute
/// (HTTP/1.1 réel ou reconstruit depuis HTTP/2 par `synth_http1`).
pub fn store_from_raw(host: &str, raw: &[u8]) {
    // Zone d'en-tête = jusqu'au double CRLF.
    let head_end = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(raw.len());
    let head = &raw[..head_end];
    for line in head.split(|&c| c == b'\n') {
        let line = core::str::from_utf8(line).unwrap_or("").trim_end_matches('\r');
        if line.len() > 11 && line[..11].eq_ignore_ascii_case("set-cookie:") {
            store_one(host, line[11..].trim());
        }
    }
}

/// Enregistre un cookie depuis la valeur d'un en-tête `Set-Cookie`.
pub fn store_one(host: &str, value: &str) {
    let host = host.to_ascii_lowercase();
    let mut parts = value.split(';');
    let nv = match parts.next() { Some(p) => p.trim(), None => return };
    let eq = match nv.find('=') { Some(i) => i, None => return };
    let (name, val) = (nv[..eq].trim().to_string(), nv[eq + 1..].trim().to_string());
    if name.is_empty() || name.len() + val.len() > 4096 { return; }

    let mut domain = host.clone();
    let mut host_only = true;
    let mut delete = false;
    for attr in parts {
        let a = attr.trim();
        let (k, v) = match a.find('=') {
            Some(i) => (a[..i].trim().to_ascii_lowercase(), a[i + 1..].trim()),
            None => (a.to_ascii_lowercase(), ""),
        };
        match k.as_str() {
            "domain" => {
                let d = v.trim_start_matches('.').to_ascii_lowercase();
                // Le domaine demande doit couvrir l'hote (anti vol de cookie).
                if !d.is_empty() && (host == d || host.ends_with(&alloc::format!(".{}", d))) {
                    domain = d;
                    host_only = false;
                }
            }
            "max-age" => {
                if v.trim().parse::<i64>().map(|n| n <= 0).unwrap_or(false) { delete = true; }
            }
            "expires" => {
                // Pas d'horloge calendaire fiable : on ne gere que la
                // suppression explicite par date passee canonique.
                if v.contains("1970") || v.contains("1969") { delete = true; }
            }
            _ => {}
        }
    }

    let j = jar();
    if let Some(i) = j.iter().position(|c| c.name == name && c.domain == domain) {
        if delete { j.remove(i); } else { j[i].value = val; }
        return;
    }
    if delete { return; }
    if j.len() >= MAX_COOKIES { j.remove(0); } // FIFO simple
    j.push(Cookie { domain, host_only, name, value: val });
}

/// Valeur de l'en-tête `Cookie:` pour cet hôte (None si aucun cookie).
pub fn header_for(host: &str) -> Option<String> {
    let host = host.to_ascii_lowercase();
    let mut out = String::new();
    for c in jar().iter() {
        let matches = if c.host_only {
            host == c.domain
        } else {
            host == c.domain || host.ends_with(&alloc::format!(".{}", c.domain))
        };
        if matches {
            if !out.is_empty() { out.push_str("; "); }
            out.push_str(&c.name);
            out.push('=');
            out.push_str(&c.value);
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

/// Nombre de cookies en mémoire (diagnostic).
pub fn count() -> usize { jar().len() }

/// Vide le pot (commande shell / vie privée).
pub fn clear() { jar().clear(); }
