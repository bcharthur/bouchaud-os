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
pub mod aes;
pub mod gcm;
pub mod x25519;
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
        ("AES-128/256", aes::selftest),
        ("AES-GCM", gcm::selftest),
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
    "TLS 1.3 (X25519/AES-128-GCM/SHA-256/HKDF + X.509 RSA/ECDSA P-256/P-384)"
}

/// Resultat d'une requete HTTPS : lignes a afficher.
pub fn https_get(hostname: &str, port: u16, path: &str) -> Vec<String> {
    // Google ferme parfois le canal applicatif sur l'apex `google.com` avec
    // notre pile TLS/HTTP minimale, alors que le frontend canonique `www` sert
    // bien une page HTTP/1.1. Un navigateur reel suit cette canonicalisation ;
    // on la fait ici uniquement si la premiere tentative TLS valide ne renvoie
    // aucune donnee applicative.
    let first = https_get_once(hostname, port, path);
    if response_has_body_or_status(&first) || hostname.starts_with("www.") || hostname.matches('.').count() != 1 {
        return first;
    }

    use alloc::format;
    let www = format!("www.{}", hostname);
    let retry = https_get_once(&www, port, path);
    if response_has_body_or_status(&retry) {
        return retry;
    }
    first
}

fn response_has_body_or_status(lines: &[String]) -> bool {
    for l in lines {
        if l.starts_with("HTTP/") { return true; }
    }
    false
}

fn https_get_once(hostname: &str, port: u16, path: &str) -> Vec<String> {
    use alloc::format;
    use alloc::string::ToString;
    let mut out: Vec<String> = Vec::new();

    let ip = match crate::net::resolve(hostname) {
        Some(ip) => ip,
        None => { out.push(format!("DNS: echec pour {}", hostname)); return out; }
    };

    let conn = match crate::net::tcp::TcpConn::connect(ip, port) {
        Some(c) => c,
        None => { out.push(format!("connexion TCP echouee vers {}:{}", hostname, port)); return out; }
    };

    let mut sess = match handshake::connect(conn, hostname) {
        Ok(s) => s,
        Err(e) => { out.push(format!("handshake TLS echoue: {}", e)); return out; }
    };

    // Bandeau de securite (resultat de la validation de chaine).
    let r = &sess.report;
    let lock = if r.trusted && r.hostname_ok && !r.expired { "[TLS OK]" } else { "[TLS !]" };
    out.push(format!("{} {} (CN={})", lock, r.detail, r.subject_cn));

    // Requete HTTP/1.1 sur le canal chiffre. `Accept-Encoding: identity` evite
    // de recevoir uniquement un flux compresse illisible par le navigateur VGA.
    let req = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: BouchaudOS-TLS\r\nConnection: close\r\nAccept: text/html,*/*\r\nAccept-Encoding: identity\r\n\r\n",
        path, hostname
    );
    let mut trace: Vec<String> = Vec::new();
    trace.push(format!(
        "post_finished: rx={} peer_fin={} closed={} rst={} fin_seen={} tcp_seq={}",
        sess.post_finished_rx, sess.post_finished_peer_fin, sess.post_finished_closed,
        sess.post_finished_rst, sess.post_finished_fin_seen, sess.post_finished_tcp_seq,
    ));
    let seq_before = sess.conn.seq_no();
    let sent = sess.send_app(req.as_bytes());
    let seq_after = sess.conn.seq_no();
    trace.push(format!(
        "send_app: sent={} app_tcp_bytes={} rx={} peer_fin={} closed={} rst={} fin_seen={} seq_before={} seq_after={} peer_ack={} last_flags=0x{:02x} last_plen={}",
        sent, seq_after.wrapping_sub(seq_before), sess.conn.rx.len(), sess.conn.peer_fin, sess.conn.closed, sess.conn.rst_seen, sess.conn.fin_seen,
        seq_before, seq_after, sess.conn.last_peer_ack(), sess.conn.last_flags(), sess.conn.last_plen(),
    ));
    let resp = sess.recv_all_trace(200_000, &mut trace);
    sess.close();

    if resp.is_empty() {
        out.push("reponse vide (chiffree)".to_string());
        for line in trace {
            out.push(format!("  {}", line));
            if out.len() >= 24 { break; }
        }
        return out;
    }

    // Ligne de statut.
    let mut i = 0;
    while i < resp.len() && resp[i] != b'\r' && resp[i] != b'\n' { i += 1; }
    let mut status = String::new();
    for &b in &resp[..i] { status.push(b as char); }
    out.push(status);

    // Corps (apres \r\n\r\n).
    let body_off = find_body(&resp).unwrap_or(0);
    let mut line = String::new();
    for &b in &resp[body_off..] {
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
