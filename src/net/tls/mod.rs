//! TLS 1.3 from-scratch (HTTPS) pour Bouchaud OS.
//!
//! Pile cryptographique ecrite a la main (aucune dependance externe) :
//!   - SHA-256 / HMAC / HKDF (`sha256`) et SHA-384/SHA-512 (`sha512`)
//!   - AES-128/256 + GCM AEAD (`aes`, `gcm`)
//!   - X25519 (`x25519`) et ECDSA P-256/P-384 (`p256`, `p384`)
//!   - RSA + bignum (`bignum`, `rsa`)
//!   - ASN.1/DER + X.509 + validation de chaine (`asn1`, `x509`, `roots`)
//!   - CSPRNG (`rng`)
//!   - handshake et couche record TLS 1.3 (`handshake`, `record`)

pub mod sha256;
pub mod sha512;
pub mod hash;
pub mod aes;
pub mod gcm;
pub mod chacha;
pub mod x25519;
pub mod ec;
pub mod p256;
pub mod p384;
pub mod bignum;
pub mod rsa;
pub mod asn1;
pub mod x509;
pub mod roots;
pub mod validate;
pub mod record;
pub mod handshake;
pub mod rng;

use alloc::string::String;
use alloc::vec::Vec;

/// Versions TLS.
pub const TLS_1_2: u16 = 0x0303;
pub const TLS_1_3: u16 = 0x0304;

/// Lance tous les auto-tests crypto et renvoie le nombre de tests OK / total.
/// Affiche le detail (commande `tls-selftest`).
pub fn selftest() {
    let tests: &[(&str, fn() -> Result<(), &'static str>)] = &[
        ("SHA-256/HMAC/HKDF", sha256::selftest),
        ("SHA-384/SHA-512", sha512::selftest),
        ("HMAC/HKDF SHA-256/SHA-384", hash::selftest),
        ("AES-128/256", aes::selftest),
        ("AES-GCM", gcm::selftest),
        ("ChaCha20-Poly1305", chacha::selftest),
        ("X25519", x25519::selftest),
        ("P-256/ECDSA", p256::selftest),
        ("P-384/ECDSA", p384::selftest),
        ("RSA PKCS#1v1.5 + PSS", rsa::selftest),
        ("X.509 (parsing racines)", x509_selftest),
    ];
    let mut ok = 0;
    for (name, f) in tests {
        match f() {
            Ok(()) => { println!("  [OK]   {}", name); ok += 1; }
            Err(e) => { println!("  [FAIL] {} : {}", name, e); }
        }
    }
    println!("tls-selftest : {}/{} modules crypto valides", ok, tests.len());
}

// Verifie que les racines embarquees se parsent et qu'au moins une racine
// auto-signee verifie sa propre signature (preuve du chemin RSA/ECDSA + X.509).
fn x509_selftest() -> Result<(), &'static str> {
    let parsed = roots::parsed();
    if parsed.len() != roots::count() {
        return Err("racine non parsable");
    }
    let mut self_signed_ok = 0;
    for c in &parsed {
        if c.subject == c.issuer && x509::verify_signed_by(c, &c.pubkey) {
            self_signed_ok += 1;
        }
    }
    if self_signed_ok == 0 {
        return Err("aucune racine auto-signee verifiee");
    }
    Ok(())
}

/// Etat d'implementation, pour les messages utilisateur.
pub fn status() -> &'static str {
    "TLS 1.3 (X25519/AES-128/256-GCM/ChaCha20-Poly1305 + SHA-256/384 + X.509 RSA/ECDSA P-256/P-384)"
}

/// Reponse HTTPS brute : banniere TLS + octets HTTP dechiffres.
pub struct HttpsFetchResult {
    pub banner: Vec<String>,
    pub raw: Vec<u8>,
}

/// Resultat d'une requete HTTPS : lignes a afficher.
pub fn https_get(hostname: &str, port: u16, path: &str) -> Vec<String> {
    let r = https_fetch(hostname, port, path);
    let mut out = r.banner;
    if r.raw.is_empty() {
        if !out.iter().any(|l| l.contains("reponse vide")) {
            out.push("reponse vide (chiffree)".into());
        }
        return out;
    }

    // Decode la reponse (dechunk + decompression gzip/deflate) via net::http.
    let (status, body) = match crate::net::http::parse_response(&r.raw) {
        Some(resp) => (resp.status_line, resp.body),
        None => {
            let mut s = String::new();
            for &b in r.raw.iter().take_while(|&&b| b != b'\r' && b != b'\n') { s.push(b as char); }
            (s, r.raw.clone())
        }
    };
    out.push(status);
    let mut line = String::new();
    for &b in &body {
        match b {
            b'\n' => { out.push(core::mem::take(&mut line)); if out.len() > 200 { break; } }
            b'\r' => {}
            0x20..=0x7e => line.push(b as char),
            _ => line.push('.'),
        }
    }
    if !line.is_empty() { out.push(line); }
    out
}

/// Recupere une URL HTTPS et renvoie les octets HTTP dechiffres.
/// Utilise par `wget`/`https` et par le navigateur, pour permettre le suivi
/// des redirections et le decodage `chunked` dans `net::http`.
pub fn https_fetch(hostname: &str, port: u16, path: &str) -> HttpsFetchResult {
    let first = https_fetch_once(hostname, port, path);
    if !first.raw.is_empty() || hostname.starts_with("www.") || hostname.matches('.').count() != 1 {
        return first;
    }

    // Petit comportement navigateur : si l'apex reste muet apres un TLS valide,
    // retente www.<host>. Cela aide google.com -> www.google.com sans casser
    // les domaines avec plusieurs labels.
    use alloc::format;
    let www = format!("www.{}", hostname);
    let retry = https_fetch_once(&www, port, path);
    if !retry.raw.is_empty() { return retry; }
    first
}

fn https_fetch_once(hostname: &str, port: u16, path: &str) -> HttpsFetchResult {
    use alloc::format;
    use alloc::string::ToString;

    let mut banner: Vec<String> = Vec::new();

    let ip = match crate::net::resolve(hostname) {
        Some(ip) => ip,
        None => {
            banner.push(format!("DNS: echec pour {}", hostname));
            return HttpsFetchResult { banner, raw: Vec::new() };
        }
    };

    let conn = match crate::net::tcp::TcpConn::connect(ip, port) {
        Some(c) => c,
        None => {
            banner.push(format!("connexion TCP echouee vers {}:{}", hostname, port));
            return HttpsFetchResult { banner, raw: Vec::new() };
        }
    };

    let mut sess = match handshake::connect(conn, hostname) {
        Ok(s) => s,
        Err(e) => {
            banner.push(format!("handshake TLS echoue: {}", e));
            return HttpsFetchResult { banner, raw: Vec::new() };
        }
    };

    let r = &sess.report;
    let lock = if r.trusted && r.hostname_ok && !r.expired { "[TLS OK]" } else { "[TLS !]" };
    banner.push(format!("{} {} (CN={}, suite={})", lock, r.detail, r.subject_cn, r.cipher_suite));

    let req = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36\r\nAccept: text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8\r\nAccept-Language: fr-FR,fr;q=0.9,en-US;q=0.8,en;q=0.7\r\nAccept-Encoding: gzip, deflate\r\nUpgrade-Insecure-Requests: 1\r\nSec-Fetch-Dest: document\r\nSec-Fetch-Mode: navigate\r\nSec-Fetch-Site: none\r\nSec-Fetch-User: ?1\r\nConnection: close\r\n\r\n",
        path, hostname
    );

    let mut trace: Vec<String> = Vec::new();
    trace.push(format!(
        "post_finished: rx={} peer_fin={} closed={} rst={} fin_seen={}",
        sess.post_finished_rx, sess.post_finished_peer_fin, sess.post_finished_closed,
        sess.post_finished_rst, sess.post_finished_fin_seen,
    ));
    let sent = sess.send_app(req.as_bytes());
    trace.push(format!(
        "send_app: sent={} rx={} peer_fin={} closed={} rst={} fin_seen={}",
        sent, sess.conn.rx.len(), sess.conn.peer_fin, sess.conn.closed, sess.conn.rst_seen, sess.conn.fin_seen,
    ));
    let raw = sess.recv_all_trace(200_000, &mut trace);
    sess.close();

    if raw.is_empty() {
        banner.push("reponse vide (chiffree)".to_string());
        for line in trace {
            banner.push(format!("  {}", line));
            if banner.len() >= 24 { break; }
        }
    }

    HttpsFetchResult { banner, raw }
}

fn find_body(resp: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i + 3 < resp.len() {
        if resp[i] == b'\r' && resp[i + 1] == b'\n' && resp[i + 2] == b'\r' && resp[i + 3] == b'\n' {
            return Some(i + 4);
        }
        i += 1;
    }
    None
}
