//! HTTP/2 (RFC 7540) minimal cote client : une requete GET sur une session TLS
//! dont l'ALPN a negocie `h2`.
//!
//! Sequence : preface de connexion + SETTINGS + WINDOW_UPDATE + HEADERS (HPACK),
//! puis lecture des frames (SETTINGS/PING acquittees, HEADERS/CONTINUATION
//! decodees, DATA accumulees) jusqu'a END_STREAM. Le resultat est re-synthetise
//! en reponse HTTP/1.1 brute pour reutiliser tout le decodage existant
//! (`net::http` : Content-Encoding gzip/deflate/br, redirections, is_html).

use super::hpack;
use crate::net::tls::handshake::Session;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

const FT_DATA: u8 = 0x0;
const FT_HEADERS: u8 = 0x1;
const FT_RST_STREAM: u8 = 0x3;
const FT_SETTINGS: u8 = 0x4;
const FT_PING: u8 = 0x6;
const FT_GOAWAY: u8 = 0x7;
const FT_WINDOW_UPDATE: u8 = 0x8;
const FT_CONTINUATION: u8 = 0x9;

const FLAG_ACK: u8 = 0x1;
const FLAG_END_STREAM: u8 = 0x1;
const FLAG_END_HEADERS: u8 = 0x4;
const FLAG_PADDED: u8 = 0x8;
const FLAG_PRIORITY: u8 = 0x20;

const SETTINGS_ENABLE_PUSH: u16 = 0x2;
const SETTINGS_INITIAL_WINDOW_SIZE: u16 = 0x4;

fn build_frame(ftype: u8, flags: u8, stream: u32, payload: &[u8]) -> Vec<u8> {
    let len = payload.len();
    let mut f = Vec::with_capacity(9 + len);
    f.push((len >> 16) as u8);
    f.push((len >> 8) as u8);
    f.push(len as u8);
    f.push(ftype);
    f.push(flags);
    f.extend_from_slice(&(stream & 0x7fff_ffff).to_be_bytes());
    f.extend_from_slice(payload);
    f
}

fn push_setting(out: &mut Vec<u8>, id: u16, val: u32) {
    out.extend_from_slice(&id.to_be_bytes());
    out.extend_from_slice(&val.to_be_bytes());
}

/// Effectue un GET HTTP/2 et renvoie une reponse HTTP/1.1 brute synthetisee.
pub fn fetch(sess: &mut Session, host: &str, path: &str, trace: &mut Vec<String>) -> Option<Vec<u8>> {
    // Burst initial : preface + SETTINGS + WINDOW_UPDATE connexion + HEADERS.
    let mut initial = Vec::new();
    initial.extend_from_slice(PREFACE);

    let mut settings = Vec::new();
    push_setting(&mut settings, SETTINGS_ENABLE_PUSH, 0);
    push_setting(&mut settings, SETTINGS_INITIAL_WINDOW_SIZE, 0x7fff_ffff);
    initial.extend_from_slice(&build_frame(FT_SETTINGS, 0, 0, &settings));

    // Releve la fenetre de controle de flux au niveau connexion pour ne pas
    // bloquer la reception d'une grande page (la fenetre initiale = 65535).
    let inc: u32 = 0x7fff_0000;
    initial.extend_from_slice(&build_frame(FT_WINDOW_UPDATE, 0, 0, &inc.to_be_bytes()));

    let req = hpack::encode_request(&[
        (":method", "GET"),
        (":path", path),
        (":scheme", "https"),
        (":authority", host),
        ("user-agent", "BouchaudOS"),
        ("accept", "*/*"),
        ("accept-encoding", "gzip, deflate, br"),
    ]);
    initial.extend_from_slice(&build_frame(
        FT_HEADERS,
        FLAG_END_HEADERS | FLAG_END_STREAM,
        1,
        &req,
    ));
    sess.send_app(&initial);

    let mut buf: Vec<u8> = Vec::new();
    let mut dec = hpack::Decoder::new();
    let mut header_block: Vec<u8> = Vec::new();
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut body: Vec<u8> = Vec::new();
    let mut got_headers = false;
    let mut done = false;
    let mut frames = 0u32;

    // Remplit `buf` jusqu'a `need` octets en lisant la session ; false si fin.
    fn ensure(sess: &mut Session, buf: &mut Vec<u8>, need: usize) -> bool {
        let mut empty = 0u32;
        while buf.len() < need {
            match sess.recv_some() {
                Some(d) => buf.extend_from_slice(&d),
                None => return false,
            }
            empty += 1;
            if empty > 10_000 { return false; }
        }
        true
    }

    while !done {
        if !ensure(sess, &mut buf, 9) { break; }
        let flen = ((buf[0] as usize) << 16) | ((buf[1] as usize) << 8) | buf[2] as usize;
        let ftype = buf[3];
        let flags = buf[4];
        let stream = u32::from_be_bytes([buf[5] & 0x7f, buf[6], buf[7], buf[8]]);
        if !ensure(sess, &mut buf, 9 + flen) { break; }
        let payload = buf[9..9 + flen].to_vec();
        buf.drain(..9 + flen);
        frames += 1;
        if frames > 100_000 { break; }

        match ftype {
            FT_SETTINGS => {
                if flags & FLAG_ACK == 0 {
                    sess.send_app(&build_frame(FT_SETTINGS, FLAG_ACK, 0, &[]));
                }
            }
            FT_PING => {
                if flags & FLAG_ACK == 0 {
                    sess.send_app(&build_frame(FT_PING, FLAG_ACK, 0, &payload));
                }
            }
            FT_HEADERS => {
                let mut off = 0usize;
                let mut end = payload.len();
                if flags & FLAG_PADDED != 0 && !payload.is_empty() {
                    let pad = payload[0] as usize;
                    off = 1;
                    if end >= pad { end -= pad; }
                }
                if flags & FLAG_PRIORITY != 0 { off += 5; }
                if off <= end { header_block.extend_from_slice(&payload[off..end]); }
                if flags & FLAG_END_HEADERS != 0 {
                    match dec.decode(&header_block) {
                        Some(hs) => { headers = hs; got_headers = true; }
                        None => { trace.push(String::from("h2: echec HPACK")); }
                    }
                    header_block.clear();
                }
                if flags & FLAG_END_STREAM != 0 && stream == 1 { done = true; }
            }
            FT_CONTINUATION => {
                header_block.extend_from_slice(&payload);
                if flags & FLAG_END_HEADERS != 0 {
                    if let Some(hs) = dec.decode(&header_block) { headers = hs; got_headers = true; }
                    header_block.clear();
                }
            }
            FT_DATA => {
                let mut off = 0usize;
                let mut end = payload.len();
                if flags & FLAG_PADDED != 0 && !payload.is_empty() {
                    let pad = payload[0] as usize;
                    off = 1;
                    if end >= pad { end -= pad; }
                }
                if off <= end { body.extend_from_slice(&payload[off..end]); }
                if flags & FLAG_END_STREAM != 0 && stream == 1 { done = true; }
            }
            FT_RST_STREAM => {
                if stream == 1 { trace.push(String::from("h2: RST_STREAM")); done = true; }
            }
            FT_GOAWAY => { trace.push(String::from("h2: GOAWAY")); done = true; }
            FT_WINDOW_UPDATE => {}
            _ => {}
        }
    }

    if !got_headers {
        trace.push(String::from("h2: pas d'en-tetes de reponse"));
        return None;
    }
    trace.push(format!("h2: {} frames, {} octets de corps", frames, body.len()));
    Some(synth_http1(&headers, &body))
}

// Re-synthetise une reponse HTTP/1.1 brute a partir des en-tetes/corps HTTP/2.
fn synth_http1(headers: &[(String, String)], body: &[u8]) -> Vec<u8> {
    let status = headers
        .iter()
        .find(|(n, _)| n == ":status")
        .map(|(_, v)| v.as_str())
        .unwrap_or("200");
    let mut s = format!("HTTP/1.1 {}\r\n", status);
    for (n, v) in headers {
        if n.starts_with(':') { continue; } // pseudo-en-tetes
        let nl = n.to_ascii_lowercase();
        // On reconstruit la longueur nous-memes ; pas de framing chunked en h2.
        if nl == "transfer-encoding" || nl == "content-length" { continue; }
        s.push_str(n);
        s.push_str(": ");
        s.push_str(v);
        s.push_str("\r\n");
    }
    s.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
    let mut out = s.into_bytes();
    out.extend_from_slice(body);
    out
}
