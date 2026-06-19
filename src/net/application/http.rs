//! HTTP/1.1 : construction de requete et decodage de reponse.
//!
//! Decode une reponse HTTP/1.1 reelle : separation en-tete/corps, lecture des
//! en-tetes (insensible a la casse), et reconstruction du corps qu'il soit
//! delimite par `Content-Length` ou encode en `Transfer-Encoding: chunked`
//! (le cas de GitHub, Google et de la plupart des sites dynamiques). Sans ce
//! decodage, `wget` afficherait les marqueurs de taille de chunk (`4000`, ...)
//! au milieu du HTML.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Construit une requete `GET` HTTP/1.1 (connexion fermee apres reponse).
///
/// La pile sait decompresser `gzip` et `deflate` (cf. `net::inflate`).
pub fn build_get(host: &str, path: &str) -> String {
    format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: BouchaudOS\r\nAccept: */*\r\nAccept-Encoding: gzip, deflate, br\r\nConnection: close\r\n\r\n",
        path, host
    )
}

/// Renvoie l'index du debut du corps (apres l'en-tete `\r\n\r\n`).
pub fn body_offset(resp: &[u8]) -> Option<usize> {
    if resp.len() < 4 { return None; }
    let mut i = 0;
    while i + 3 < resp.len() {
        if resp[i] == b'\r' && resp[i + 1] == b'\n' && resp[i + 2] == b'\r' && resp[i + 3] == b'\n' {
            return Some(i + 4);
        }
        i += 1;
    }
    None
}

// Recherche insensible a la casse d'un en-tete dans la zone d'en-tete brute.
// Renvoie la valeur (sans espaces de bordure) si presente.
fn header_value<'a>(head: &'a [u8], name: &str) -> Option<&'a str> {
    let name_l = name.as_bytes();
    let mut i = 0;
    while i < head.len() {
        // Debut de ligne : compare le nom d'en-tete jusqu'au ':'.
        let line_start = i;
        // Fin de ligne.
        let mut j = i;
        while j < head.len() && head[j] != b'\n' { j += 1; }
        let line = &head[line_start..j];
        if let Some(colon) = line.iter().position(|&c| c == b':') {
            let (k, v) = line.split_at(colon);
            if k.len() == name_l.len()
                && k.iter().zip(name_l).all(|(a, b)| a.to_ascii_lowercase() == b.to_ascii_lowercase())
            {
                // v commence par ':' ; on retire ':' puis espaces/\r.
                let val = &v[1..];
                let s = core::str::from_utf8(val).ok()?.trim();
                return Some(s);
            }
        }
        i = j + 1;
    }
    None
}

/// Reponse HTTP decodee.
pub struct Response {
    pub status_line: String,
    pub status_code: u16,
    pub location: Option<String>,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

impl Response {
    /// True si le corps est du HTML (d'apres Content-Type ou un reniflage).
    pub fn is_html(&self) -> bool {
        if let Some(ct) = &self.content_type {
            if ct.to_ascii_lowercase().contains("text/html") { return true; }
            if ct.to_ascii_lowercase().contains("xhtml") { return true; }
        }
        let head = &self.body[..self.body.len().min(512)];
        let lower: Vec<u8> = head.iter().map(|b| b.to_ascii_lowercase()).collect();
        let win = |needle: &[u8]| lower.windows(needle.len()).any(|w| w == needle);
        win(b"<!doctype html") || win(b"<html") || win(b"<head") || win(b"<body")
    }
}

impl Response {
    /// True si la reponse est une redirection avec un en-tete `Location`.
    pub fn is_redirect(&self) -> bool {
        matches!(self.status_code, 301 | 302 | 303 | 307 | 308) && self.location.is_some()
    }
}

/// True si la zone d'en-tete annonce un corps chunked.
fn is_chunked(head: &[u8]) -> bool {
    header_value(head, "Transfer-Encoding")
        .map(|v| v.to_ascii_lowercase().contains("chunked"))
        .unwrap_or(false)
}

fn content_length(head: &[u8]) -> Option<usize> {
    header_value(head, "Content-Length").and_then(|v| v.trim().parse::<usize>().ok())
}

// Decode un corps `Transfer-Encoding: chunked`. Renvoie le corps reconstruit et
// `true` si le chunk terminal (taille 0) a ete vu.
fn dechunk(body: &[u8]) -> (Vec<u8>, bool) {
    let mut out = Vec::new();
    let mut i = 0;
    loop {
        // Lit la ligne de taille (hex) jusqu'a \r\n.
        let mut j = i;
        while j < body.len() && body[j] != b'\n' { j += 1; }
        if j >= body.len() { break; } // taille incomplete
        // La taille peut etre suivie d'extensions « ; ... » : on coupe au ';'.
        let line = &body[i..j];
        let hexpart: &[u8] = match line.iter().position(|&c| c == b';') {
            Some(p) => &line[..p],
            None => line,
        };
        let size = parse_hex(hexpart);
        let after_size = j + 1; // apres le \n
        if size == 0 {
            return (out, true); // chunk terminal
        }
        let chunk_end = after_size + size;
        if chunk_end > body.len() { break; } // chunk incomplet
        out.extend_from_slice(&body[after_size..chunk_end]);
        // Saute le \r\n de fin de chunk.
        i = chunk_end;
        if i < body.len() && body[i] == b'\r' { i += 1; }
        if i < body.len() && body[i] == b'\n' { i += 1; }
    }
    (out, false)
}

fn parse_hex(s: &[u8]) -> usize {
    let mut v = 0usize;
    for &c in s {
        let d = match c {
            b'0'..=b'9' => (c - b'0') as usize,
            b'a'..=b'f' => (c - b'a' + 10) as usize,
            b'A'..=b'F' => (c - b'A' + 10) as usize,
            b' ' | b'\r' | b'\t' => continue,
            _ => break,
        };
        v = v * 16 + d;
    }
    v
}

/// Resout une cible de redirection `Location` en URL absolue, relativement a
/// l'URL courante (`scheme`, `host`). Gere les formes :
///   - absolue  : `https://autre.com/x`        -> telle quelle
///   - //host   : `//autre.com/x`              -> `scheme://autre.com/x`
///   - /chemin  : `/x`                         -> `scheme://host/x`
///   - relative : `x`                          -> `scheme://host/x`
pub fn resolve_location(scheme: &str, host: &str, location: &str) -> String {
    if location.starts_with("http://") || location.starts_with("https://") {
        return String::from(location);
    }
    if let Some(rest) = location.strip_prefix("//") {
        return format!("{}://{}", scheme, rest);
    }
    if location.starts_with('/') {
        return format!("{}://{}{}", scheme, host, location);
    }
    format!("{}://{}/{}", scheme, host, location)
}

/// Indique si `raw` contient une reponse HTTP complete (en-tete + corps entier).
/// Sert a arreter la reception sans attendre le FIN/timeout.
pub fn is_complete(raw: &[u8]) -> bool {
    let body_off = match body_offset(raw) { Some(o) => o, None => return false };
    let head = &raw[..body_off];
    let body = &raw[body_off..];
    if is_chunked(head) {
        return dechunk(body).1;
    }
    if let Some(len) = content_length(head) {
        return body.len() >= len;
    }
    // Ni chunked ni Content-Length : corps delimite par fermeture de connexion.
    false
}

/// Decode une reponse HTTP brute (en-tete + corps), en dechunkant si besoin.
pub fn parse_response(raw: &[u8]) -> Option<Response> {
    let body_off = body_offset(raw)?;
    let head = &raw[..body_off];
    let raw_body = &raw[body_off..];

    // Ligne de statut.
    let mut k = 0;
    while k < head.len() && head[k] != b'\r' && head[k] != b'\n' { k += 1; }
    let status_line: String = head[..k].iter().map(|&b| b as char).collect();
    // Code (2e champ : "HTTP/1.1 200 OK").
    let status_code = status_line
        .split(' ')
        .nth(1)
        .and_then(|c| c.parse::<u16>().ok())
        .unwrap_or(0);

    let location = header_value(head, "Location").map(String::from);
    let content_type = header_value(head, "Content-Type").map(String::from);

    let body = if is_chunked(head) {
        dechunk(raw_body).0
    } else if let Some(len) = content_length(head) {
        raw_body[..len.min(raw_body.len())].to_vec()
    } else {
        raw_body.to_vec()
    };

    // Decompression si le serveur a applique un Content-Encoding (gzip/deflate),
    // ce que font beaucoup de CDN meme quand on demande `identity`.
    let body = match header_value(head, "Content-Encoding") {
        Some(enc) if !enc.eq_ignore_ascii_case("identity") => {
            crate::net::inflate::decode_content(enc, &body).unwrap_or(body)
        }
        _ => body,
    };

    Some(Response { status_line, status_code, location, content_type, body })
}
