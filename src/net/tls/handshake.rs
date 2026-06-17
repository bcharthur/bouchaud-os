//! Handshake TLS 1.3 cote client : ClientHello, lecture du flight serveur
//! chiffre, verification X.509/CertificateVerify/Finished, puis cles applicatives.
//!
//! Cette version garde une pile simple mais adopte un ClientHello plus proche
//! d'un navigateur moderne, tout en forcant HTTP/1.1 dans ALPN pour que le
//! client applicatif actuel puisse parler aux frontaux Google/GitHub.

use super::record::{self, CipherSuite, DirKeys, KeySchedule, CT_HANDSHAKE, CT_ALERT, CT_APPLICATION_DATA};
use super::{hash, x25519, rng, x509, validate};
use crate::net::tcp::TcpConn;
use alloc::vec::Vec;
use alloc::string::String;
use alloc::format;

// Suites TLS 1.3 reellement implementees.
const TLS_AES_128_GCM_SHA256: u16 = record::TLS_AES_128_GCM_SHA256_ID;
const TLS_AES_256_GCM_SHA384: u16 = record::TLS_AES_256_GCM_SHA384_ID;

// Groupes et signatures.
const GROUP_X25519: u16 = 0x001d;
const GROUP_SECP256R1: u16 = 0x0017;
const GROUP_SECP384R1: u16 = 0x0018;
const GREASE_0A0A: u16 = 0x0a0a;

const SIG_ECDSA_P256_SHA256: u16 = 0x0403;
const SIG_ECDSA_P384_SHA384: u16 = 0x0503;
const SIG_RSA_PSS_RSAE_SHA256: u16 = 0x0804;
const SIG_RSA_PKCS1_SHA256: u16 = 0x0401;

// Types de message handshake.
const HS_CLIENT_HELLO: u8 = 1;
const HS_SERVER_HELLO: u8 = 2;
const HS_ENCRYPTED_EXTENSIONS: u8 = 8;
const HS_CERTIFICATE: u8 = 11;
const HS_CERTIFICATE_VERIFY: u8 = 15;
const HS_FINISHED: u8 = 20;

/// Rapport sur la validation du certificat serveur.
pub struct CertReport {
    pub trusted: bool,
    pub hostname_ok: bool,
    pub expired: bool,
    pub detail: String,
    pub subject_cn: String,
    pub cipher_suite: &'static str,
}

/// Session TLS etablie : prete pour les donnees applicatives.
pub struct Session {
    pub conn: TcpConn,
    c_ap: DirKeys,
    s_ap: DirKeys,
    pub report: CertReport,
    /// Etat TCP observe immediatement apres le Finished client, avant tout GET HTTP.
    pub post_finished_rx: usize,
    pub post_finished_peer_fin: bool,
    pub post_finished_closed: bool,
    pub post_finished_rst: bool,
    pub post_finished_fin_seen: bool,
    rx_plain: Vec<u8>,
}

// --- petits utilitaires d'encodage ---

fn push_u16(v: &mut Vec<u8>, x: u16) { v.extend_from_slice(&x.to_be_bytes()); }

fn with_u16_len(body: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(body.len() + 2);
    push_u16(&mut v, body.len() as u16);
    v.extend_from_slice(body);
    v
}

fn push_ext(ext: &mut Vec<u8>, typ: u16, data: &[u8]) {
    push_u16(ext, typ);
    ext.extend_from_slice(&with_u16_len(data));
}

fn handshake_msg(msg_type: u8, body: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(body.len() + 4);
    v.push(msg_type);
    let l = body.len();
    v.push((l >> 16) as u8);
    v.push((l >> 8) as u8);
    v.push(l as u8);
    v.extend_from_slice(body);
    v
}

fn transcript_hash(suite: CipherSuite, transcript: &[u8]) -> Vec<u8> {
    hash::digest(suite.hash_alg(), transcript)
}

/// Construit un ClientHello TLS 1.3 plus proche d'un navigateur.
/// Note : ALPN annonce seulement HTTP/1.1. Annoncer `h2` ferait probablement
/// choisir HTTP/2 par Google/GitHub, mais la couche applicative n'est pas encore
/// capable d'emettre des frames HTTP/2.
fn build_client_hello(hostname: &str, random: &[u8; 32], _session_id: &[u8; 32], pubkey: &[u8; 32]) -> Vec<u8> {
    let mut body = Vec::new();
    push_u16(&mut body, 0x0303); // legacy_version
    body.extend_from_slice(random);

    // Mode TLS 1.3 propre : pas de legacy_session_id, donc pas de dummy CCS.
    body.push(0);

    // Liste de suites volontairement plus riche pour le fingerprint : seules
    // les suites TLS 1.3 0x1301/0x1302 sont acceptees par la couche record.
    let mut suites = Vec::new();
    push_u16(&mut suites, GREASE_0A0A);
    push_u16(&mut suites, TLS_AES_128_GCM_SHA256);
    push_u16(&mut suites, TLS_AES_256_GCM_SHA384);
    // Suites TLS 1.2 presentes pour ressembler a un client moderne, mais
    // supported_versions force TLS 1.3, donc elles ne seront pas negociees.
    for s in [0xc02b, 0xc02f, 0xc02c, 0xc030, 0xcca9, 0xcca8, 0x009e, 0x009c] {
        push_u16(&mut suites, s);
    }
    body.extend_from_slice(&with_u16_len(&suites));

    body.push(1); // compression_methods len
    body.push(0); // null

    let mut ext = Vec::new();

    // GREASE extension vide.
    push_ext(&mut ext, GREASE_0A0A, &[]);

    // server_name (SNI)
    {
        let mut sni = Vec::new();
        sni.push(0); // host_name
        sni.extend_from_slice(&with_u16_len(hostname.as_bytes()));
        let sni_list = with_u16_len(&sni);
        push_ext(&mut ext, 0, &sni_list);
    }

    // extended_master_secret (TLS 1.2, ignore en 1.3) + renegotiation_info.
    push_ext(&mut ext, 23, &[]);
    push_ext(&mut ext, 0xff01, &[0x00]);

    // supported_groups
    {
        let mut g = Vec::new();
        for group in [GREASE_0A0A, GROUP_X25519, GROUP_SECP256R1, GROUP_SECP384R1] {
            push_u16(&mut g, group);
        }
        let body = with_u16_len(&g);
        push_ext(&mut ext, 10, &body);
    }

    // ec_point_formats (TLS 1.2, ignore en 1.3)
    push_ext(&mut ext, 11, &[1, 0]);

    // signature_algorithms : on n'annonce que ce que le noyau sait verifier.
    {
        let mut s = Vec::new();
        for alg in [SIG_ECDSA_P256_SHA256, SIG_ECDSA_P384_SHA384, SIG_RSA_PSS_RSAE_SHA256, SIG_RSA_PKCS1_SHA256] {
            push_u16(&mut s, alg);
        }
        let body = with_u16_len(&s);
        push_ext(&mut ext, 13, &body);
    }

    // ALPN : HTTP/1.1 uniquement pour rester coherent avec la couche HTTP.
    {
        let mut proto = Vec::new();
        proto.push(8);
        proto.extend_from_slice(b"http/1.1");
        let list = with_u16_len(&proto);
        push_ext(&mut ext, 16, &list);
    }

    // status_request OCSP.
    push_ext(&mut ext, 5, &[1, 0, 0, 0, 0]);

    // signed_certificate_timestamp.
    push_ext(&mut ext, 18, &[]);

    // supported_versions : TLS 1.3 uniquement.
    {
        let mut v = Vec::new();
        v.push(2);
        push_u16(&mut v, 0x0304);
        push_ext(&mut ext, 43, &v);
    }

    // psk_key_exchange_modes : obligatoire pour certains frontaux TLS 1.3.
    push_ext(&mut ext, 45, &[1, 1]);

    // key_share : X25519.
    {
        let mut entry = Vec::new();
        push_u16(&mut entry, GROUP_X25519);
        entry.extend_from_slice(&with_u16_len(pubkey));
        let list = with_u16_len(&entry);
        push_ext(&mut ext, 51, &list);
    }

    // Padding pour eviter les tailles de ClientHello trop atypiques. Le contenu
    // exact n'est pas critique, les serveurs l'ignorent.
    if ext.len() < 300 {
        let mut pad = Vec::new();
        pad.resize(300 - ext.len(), 0);
        push_ext(&mut ext, 21, &pad);
    }

    body.extend_from_slice(&with_u16_len(&ext));
    handshake_msg(HS_CLIENT_HELLO, &body)
}

struct ServerHello {
    server_pub: [u8; 32],
    cipher: u16,
}

fn parse_server_hello(msg: &[u8]) -> Result<ServerHello, &'static str> {
    if msg.len() < 4 || msg[0] != HS_SERVER_HELLO { return Err("pas un ServerHello"); }
    let body = &msg[4..];
    let mut p = 0usize;
    let need = |p: usize, n: usize| -> Result<(), &'static str> {
        if p + n > body.len() { Err("ServerHello tronque") } else { Ok(()) }
    };
    need(p, 2 + 32 + 1)?;
    p += 2;
    const HRR: [u8; 32] = [
        0xcf,0x21,0xad,0x74,0xe5,0x9a,0x61,0x11,0xbe,0x1d,0x8c,0x02,0x1e,0x65,0xb8,0x91,
        0xc2,0xa2,0x11,0x16,0x7a,0xbb,0x8c,0x5e,0x07,0x9e,0x09,0xe2,0xc8,0xa8,0x33,0x9c,
    ];
    if body[p..p + 32] == HRR { return Err("HelloRetryRequest (groupe non offert)"); }
    p += 32;
    let sid_len = body[p] as usize; p += 1;
    need(p, sid_len + 3)?;
    p += sid_len;
    let cipher = ((body[p] as u16) << 8) | body[p + 1] as u16; p += 2;
    p += 1;
    need(p, 2)?;
    let ext_len = ((body[p] as usize) << 8) | body[p + 1] as usize; p += 2;
    need(p, ext_len)?;
    let ext_end = p + ext_len;
    let mut server_pub = [0u8; 32];
    let mut found_ks = false;
    while p + 4 <= ext_end {
        let etype = ((body[p] as u16) << 8) | body[p + 1] as u16;
        let elen = ((body[p + 2] as usize) << 8) | body[p + 3] as usize;
        p += 4;
        if p + elen > ext_end { return Err("extension SH tronquee"); }
        let edata = &body[p..p + elen];
        if etype == 51 && edata.len() >= 4 {
            let g = ((edata[0] as u16) << 8) | edata[1] as u16;
            let klen = ((edata[2] as usize) << 8) | edata[3] as usize;
            if g == GROUP_X25519 && klen == 32 && edata.len() >= 4 + 32 {
                server_pub.copy_from_slice(&edata[4..36]);
                found_ks = true;
            }
        }
        p += elen;
    }
    if !found_ks { return Err("ServerHello sans key_share x25519"); }
    Ok(ServerHello { server_pub, cipher })
}

fn read_raw_record(conn: &mut TcpConn) -> Option<([u8; 5], Vec<u8>)> {
    if !conn.fill(5) { return None; }
    let mut hdr = [0u8; 5];
    hdr.copy_from_slice(&conn.rx[..5]);
    let len = ((hdr[3] as usize) << 8) | hdr[4] as usize;
    if !conn.fill(5 + len) { return None; }
    let _ = conn.take(5);
    let body = conn.take(len);
    if body.len() < len { return None; }
    Some((hdr, body))
}

fn send_plaintext(conn: &mut TcpConn, ct: u8, data: &[u8]) {
    let mut rec = Vec::with_capacity(data.len() + 5);
    rec.push(ct);
    rec.extend_from_slice(&[0x03, 0x03]);
    push_u16(&mut rec, data.len() as u16);
    rec.extend_from_slice(data);
    conn.send(&rec);
}

fn cert_verify_content(transcript_hash: &[u8]) -> Vec<u8> {
    let mut c = Vec::new();
    c.extend_from_slice(&[0x20; 64]);
    c.extend_from_slice(b"TLS 1.3, server CertificateVerify");
    c.push(0x00);
    c.extend_from_slice(transcript_hash);
    c
}

fn verify_cert_verify(body: &[u8], leaf: &x509::Certificate, transcript_hash: &[u8]) -> bool {
    if body.len() < 4 { return false; }
    let scheme = ((body[0] as u16) << 8) | body[1] as u16;
    let sig_len = ((body[2] as usize) << 8) | body[3] as usize;
    if 4 + sig_len > body.len() { return false; }
    let sig = &body[4..4 + sig_len];
    let content = cert_verify_content(transcript_hash);

    match (scheme, &leaf.pubkey) {
        (SIG_RSA_PSS_RSAE_SHA256, x509::PubKey::Rsa { n, e }) => {
            let key = super::rsa::RsaPubKey::new(n, e);
            super::rsa::verify_pss_sha256(&key, &content, sig)
        }
        (SIG_RSA_PKCS1_SHA256, x509::PubKey::Rsa { n, e }) => {
            let key = super::rsa::RsaPubKey::new(n, e);
            super::rsa::verify_pkcs1_sha256(&key, &content, sig)
        }
        (SIG_ECDSA_P256_SHA256, x509::PubKey::EcP256 { point }) => verify_ecdsa_der(sig, |r, s| {
            super::p256::verify_ecdsa_sha256(point, &content, r, s)
        }),
        (SIG_ECDSA_P384_SHA384, x509::PubKey::EcP384 { point }) => verify_ecdsa_der(sig, |r, s| {
            super::p384::verify_ecdsa_sha384(point, &content, r, s)
        }),
        _ => false,
    }
}

fn verify_ecdsa_der<F: FnOnce(&[u8], &[u8]) -> bool>(sig: &[u8], f: F) -> bool {
    if let Some((seq, _)) = super::asn1::read_tag(sig, super::asn1::TAG_SEQUENCE) {
        let mut si = seq.children();
        if let (Some(r), Some(s)) = (si.next(), si.next()) { return f(strip0(r.content), strip0(s.content)); }
    }
    false
}

fn strip0(b: &[u8]) -> &[u8] {
    let mut i = 0;
    while i + 1 < b.len() && b[i] == 0 { i += 1; }
    &b[i..]
}

fn subject_cn(cert: &x509::Certificate) -> String {
    let mut out = String::new();
    if let Some((name, _)) = super::asn1::read(&cert.subject) {
        for rdn in name.children() {
            for atv in rdn.children() {
                let mut it = atv.children();
                if let (Some(oid), Some(val)) = (it.next(), it.next()) {
                    if oid.content == [0x55, 0x04, 0x03] {
                        for &b in val.content { out.push(b as char); }
                        return out;
                    }
                }
            }
        }
    }
    out
}

/// Effectue le handshake TLS 1.3 complet sur une connexion TCP deja ouverte.
pub fn connect(mut conn: TcpConn, hostname: &str) -> Result<Session, &'static str> {
    let priv_key = rng::random32();
    let pubkey = x25519::base_mul(&priv_key);
    let random = rng::random32();
    let session_id = rng::random32();

    let ch = build_client_hello(hostname, &random, &session_id, &pubkey);
    let mut transcript: Vec<u8> = Vec::new();
    transcript.extend_from_slice(&ch);
    send_plaintext(&mut conn, CT_HANDSHAKE, &ch);

    let sh_msg = loop {
        let (hdr, body) = read_raw_record(&mut conn).ok_or("pas de ServerHello (timeout)")?;
        match hdr[0] {
            record::CT_CHANGE_CIPHER_SPEC => continue,
            CT_ALERT => return Err("alerte TLS pendant ServerHello"),
            CT_HANDSHAKE => break body,
            _ => return Err("record inattendu avant ServerHello"),
        }
    };
    transcript.extend_from_slice(&sh_msg);
    let sh = parse_server_hello(&sh_msg)?;
    let suite = CipherSuite::from_id(sh.cipher).ok_or("suite TLS 1.3 non implementee")?;

    let shared = x25519::x25519(&priv_key, &sh.server_pub);
    let th_ch_sh = transcript_hash(suite, &transcript);
    let ks = KeySchedule::derive_handshake(suite, &shared, &th_ch_sh);
    let mut s_hs = DirKeys::new(suite, &ks.server_hs);

    let mut hs_buf: Vec<u8> = Vec::new();
    let mut certs_der: Vec<Vec<u8>> = Vec::new();
    let mut leaf: Option<x509::Certificate> = None;
    let mut th_through_cert: Option<Vec<u8>> = None;

    let feed = |conn: &mut TcpConn, hs_buf: &mut Vec<u8>, s_hs: &mut DirKeys| -> Result<(), &'static str> {
        loop {
            let (hdr, body) = read_raw_record(conn).ok_or("flight serveur incomplet")?;
            match hdr[0] {
                record::CT_CHANGE_CIPHER_SPEC => continue,
                CT_APPLICATION_DATA => {
                    let (inner_type, pt) = s_hs.decrypt(&hdr, &body).ok_or("dechiffrement handshake echoue")?;
                    match inner_type {
                        CT_HANDSHAKE => { hs_buf.extend_from_slice(&pt); return Ok(()); }
                        CT_ALERT => return Err("alerte TLS pendant le handshake"),
                        _ => continue,
                    }
                }
                CT_ALERT => return Err("alerte TLS (clair) pendant le handshake"),
                _ => return Err("record inattendu dans le flight serveur"),
            }
        }
    };

    loop {
        while hs_buf.len() < 4 {
            feed(&mut conn, &mut hs_buf, &mut s_hs)?;
        }
        let mlen = ((hs_buf[1] as usize) << 16) | ((hs_buf[2] as usize) << 8) | hs_buf[3] as usize;
        while hs_buf.len() < 4 + mlen {
            feed(&mut conn, &mut hs_buf, &mut s_hs)?;
        }
        let msg_type = hs_buf[0];
        let full: Vec<u8> = hs_buf[..4 + mlen].to_vec();
        let body: Vec<u8> = hs_buf[4..4 + mlen].to_vec();
        hs_buf.drain(..4 + mlen);

        match msg_type {
            HS_ENCRYPTED_EXTENSIONS => {
                transcript.extend_from_slice(&full);
            }
            HS_CERTIFICATE => {
                parse_certificate_msg(&body, &mut certs_der);
                if let Some(d) = certs_der.first() {
                    leaf = x509::parse(d);
                }
                transcript.extend_from_slice(&full);
                th_through_cert = Some(transcript_hash(suite, &transcript));
            }
            HS_CERTIFICATE_VERIFY => {
                let th = th_through_cert.as_ref().ok_or("CertificateVerify sans Certificate")?;
                let leaf_ref = leaf.as_ref().ok_or("certificat feuille manquant")?;
                if !verify_cert_verify(&body, leaf_ref, th) {
                    return Err("CertificateVerify invalide (signature serveur)");
                }
                transcript.extend_from_slice(&full);
            }
            HS_FINISHED => {
                let th_cv = transcript_hash(suite, &transcript);
                let expected = record::finished_verify(suite, &ks.server_hs, &th_cv);
                if body.len() != expected.len() || body[..] != expected[..] {
                    return Err("Finished serveur invalide (verify_data)");
                }
                transcript.extend_from_slice(&full);
                break;
            }
            _ => {
                transcript.extend_from_slice(&full);
            }
        }
    }

    let th_sf = transcript_hash(suite, &transcript);
    let (c_ap_secret, s_ap_secret) = ks.derive_application(&th_sf);

    // Validation X.509 avant le Finished client : ensuite on enchaine Finished -> GET
    // sans long calcul CPU entre les deux.
    let chain = validate::parse_chain(&certs_der);
    let now = validate::now_stamp();
    let v = validate::validate(&chain, hostname, now);
    let cn = leaf.as_ref().map(subject_cn).unwrap_or_default();
    let report = CertReport {
        trusted: v.trusted,
        hostname_ok: v.hostname_ok,
        expired: v.expired,
        detail: String::from(v.detail),
        subject_cn: cn,
        cipher_suite: suite.name(),
    };

    // Finished client chiffre avec la cle handshake client, sans pump juste apres.
    let cfin = record::finished_verify(suite, &ks.client_hs, &th_sf);
    let fin_msg = handshake_msg(HS_FINISHED, &cfin);
    let mut c_hs = DirKeys::new(suite, &ks.client_hs);
    let rec = c_hs.encrypt(CT_HANDSHAKE, &fin_msg);
    conn.send_no_pump(&rec);

    let post_finished_rx = conn.rx.len();
    let post_finished_peer_fin = conn.peer_fin;
    let post_finished_closed = conn.closed;
    let post_finished_rst = conn.rst_seen;
    let post_finished_fin_seen = conn.fin_seen;

    let c_ap = DirKeys::new(suite, &c_ap_secret);
    let s_ap = DirKeys::new(suite, &s_ap_secret);

    Ok(Session {
        conn,
        c_ap,
        s_ap,
        report,
        post_finished_rx,
        post_finished_peer_fin,
        post_finished_closed,
        post_finished_rst,
        post_finished_fin_seen,
        rx_plain: Vec::new(),
    })
}

fn parse_certificate_msg(body: &[u8], out: &mut Vec<Vec<u8>>) {
    if body.is_empty() { return; }
    let ctx_len = body[0] as usize;
    let mut p = 1 + ctx_len;
    if p + 3 > body.len() { return; }
    let list_len = ((body[p] as usize) << 16) | ((body[p + 1] as usize) << 8) | body[p + 2] as usize;
    p += 3;
    let end = (p + list_len).min(body.len());
    while p + 3 <= end {
        let clen = ((body[p] as usize) << 16) | ((body[p + 1] as usize) << 8) | body[p + 2] as usize;
        p += 3;
        if p + clen > end { break; }
        out.push(body[p..p + clen].to_vec());
        p += clen;
        if p + 2 > end { break; }
        let ext_len = ((body[p] as usize) << 8) | body[p + 1] as usize;
        p += 2 + ext_len;
    }
}

fn tls_ct_name(t: u8) -> &'static str {
    match t {
        record::CT_CHANGE_CIPHER_SPEC => "ccs",
        CT_ALERT => "alert",
        CT_HANDSHAKE => "handshake",
        CT_APPLICATION_DATA => "application_data",
        _ => "unknown",
    }
}

impl Session {
    pub fn send_app(&mut self, data: &[u8]) -> bool {
        let rec = self.c_ap.encrypt(CT_APPLICATION_DATA, data);
        self.conn.send(&rec)
    }

    pub fn recv_all(&mut self, max: usize) -> Vec<u8> {
        let mut trace_sink: Vec<String> = Vec::new();
        self.recv_all_trace(max, &mut trace_sink)
    }

    pub fn recv_all_trace(&mut self, max: usize, trace: &mut Vec<String>) -> Vec<u8> {
        let mut out = core::mem::take(&mut self.rx_plain);
        let mut empty_reads = 0u32;
        let mut records = 0u32;
        loop {
            let rec = match read_raw_record(&mut self.conn) {
                Some(r) => r,
                None => {
                    trace.push(format!(
                        "recv: pas de record TLS complet (rx={} fin={} closed={} rst={} fin_seen={} empty={})",
                        self.conn.rx.len(), self.conn.peer_fin, self.conn.closed, self.conn.rst_seen, self.conn.fin_seen, empty_reads + 1,
                    ));
                    if self.conn.peer_fin || self.conn.closed { break; }
                    empty_reads += 1;
                    if empty_reads >= 6 { break; }
                    continue;
                }
            };
            empty_reads = 0;
            records += 1;
            let (hdr, body) = rec;
            trace.push(format!("recv: record #{} outer={} len={} reste_tcp={}", records, tls_ct_name(hdr[0]), body.len(), self.conn.rx.len()));
            match hdr[0] {
                record::CT_CHANGE_CIPHER_SPEC => continue,
                CT_APPLICATION_DATA => {
                    match self.s_ap.decrypt(&hdr, &body) {
                        Some((inner_type, pt)) => {
                            trace.push(format!("recv: inner={} plain={} octets", tls_ct_name(inner_type), pt.len()));
                            match inner_type {
                                CT_APPLICATION_DATA => {
                                    out.extend_from_slice(&pt);
                                    if out.len() >= max { break; }
                                }
                                CT_HANDSHAKE => {
                                    // NewSessionTicket/KeyUpdate ignore, sequence AEAD consommee.
                                }
                                CT_ALERT => {
                                    if pt.len() >= 2 {
                                        trace.push(format!("recv: alerte TLS niveau={} description={}", pt[0], pt[1]));
                                    }
                                    break;
                                }
                                _ => {}
                            }
                        }
                        None => {
                            trace.push(format!("recv: echec de dechiffrement AEAD len={}", body.len()));
                            break;
                        }
                    }
                }
                CT_ALERT => {
                    trace.push(format!("recv: alerte TLS claire len={}", body.len()));
                    break;
                }
                _ => {
                    trace.push(format!("recv: record inattendu outer={} len={}", hdr[0], body.len()));
                    break;
                }
            }
            if self.conn.peer_fin && self.conn.rx.len() == 0 { break; }
        }
        trace.push(format!("recv: total plaintext={} octets", out.len()));
        out
    }

    pub fn close(&mut self) {
        let rec = self.c_ap.encrypt(CT_ALERT, &[0x01, 0x00]);
        self.conn.send(&rec);
        self.conn.close();
    }
}
