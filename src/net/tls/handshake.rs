//! Handshake TLS 1.3 (RFC 8446) cote client : ClientHello, lecture du flight
//! serveur (chiffre), verification CertificateVerify + Finished, etablissement
//! des cles applicatives.

use super::sha256::Sha256;
use super::record::{self, DirKeys, KeySchedule, CT_HANDSHAKE, CT_ALERT, CT_APPLICATION_DATA};
use super::{x25519, rng, x509, validate};
use crate::net::tcp::TcpConn;
use alloc::vec::Vec;
use alloc::string::String;

// Suites et identifiants.
const TLS_AES_128_GCM_SHA256: u16 = 0x1301;
const GROUP_X25519: u16 = 0x001d;
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
}

/// Session TLS etablie : prete pour les donnees applicatives.
pub struct Session {
    pub conn: TcpConn,
    c_ap: DirKeys,
    s_ap: DirKeys,
    pub report: CertReport,
    rx_plain: Vec<u8>,
}

// --- petits utilitaires d'encodage ---

fn push_u16(v: &mut Vec<u8>, x: u16) { v.extend_from_slice(&x.to_be_bytes()); }

// Encadre `body` dans une longueur sur 2 octets.
fn with_u16_len(body: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(body.len() + 2);
    push_u16(&mut v, body.len() as u16);
    v.extend_from_slice(body);
    v
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

/// Construit le ClientHello (message handshake brut, type 1).
fn build_client_hello(hostname: &str, random: &[u8; 32], session_id: &[u8; 32], pubkey: &[u8; 32]) -> Vec<u8> {
    let mut body = Vec::new();
    push_u16(&mut body, 0x0303); // legacy_version
    body.extend_from_slice(random);
    body.push(32); // legacy_session_id length
    body.extend_from_slice(session_id);
    // cipher_suites : uniquement AES_128_GCM_SHA256 (tout reste en SHA-256).
    push_u16(&mut body, 2);
    push_u16(&mut body, TLS_AES_128_GCM_SHA256);
    // compression : null
    body.push(1);
    body.push(0);

    // --- extensions ---
    let mut ext = Vec::new();

    // server_name (0)
    {
        let mut sni = Vec::new();
        sni.push(0); // type host_name
        sni.extend_from_slice(&with_u16_len(hostname.as_bytes()));
        let sni_list = with_u16_len(&sni);
        push_u16(&mut ext, 0); // extension type
        ext.extend_from_slice(&with_u16_len(&sni_list));
    }
    // supported_groups (10) : x25519
    {
        let mut g = Vec::new();
        push_u16(&mut g, GROUP_X25519);
        let body = with_u16_len(&g);
        push_u16(&mut ext, 10);
        ext.extend_from_slice(&with_u16_len(&body));
    }
    // signature_algorithms (13)
    {
        let mut s = Vec::new();
        push_u16(&mut s, SIG_ECDSA_P256_SHA256);
        push_u16(&mut s, SIG_ECDSA_P384_SHA384);
        push_u16(&mut s, SIG_RSA_PSS_RSAE_SHA256);
        push_u16(&mut s, SIG_RSA_PKCS1_SHA256);
        let body = with_u16_len(&s);
        push_u16(&mut ext, 13);
        ext.extend_from_slice(&with_u16_len(&body));
    }
    // supported_versions (43) : TLS 1.3
    {
        let mut v = Vec::new();
        v.push(2); // list length (1 octet)
        push_u16(&mut v, 0x0304);
        push_u16(&mut ext, 43);
        ext.extend_from_slice(&with_u16_len(&v));
    }
    // application_layer_protocol_negotiation (16) : force HTTP/1.1.
    // Certains frontaux modernes ferment le canal applicatif si aucun ALPN
    // n'est annonce, meme apres un handshake TLS valide.
    {
        let mut proto = Vec::new();
        proto.push(8); // longueur de "http/1.1"
        proto.extend_from_slice(b"http/1.1");
        let list = with_u16_len(&proto);
        push_u16(&mut ext, 16);
        ext.extend_from_slice(&with_u16_len(&list));
    }
    // key_share (51) : x25519
    {
        let mut entry = Vec::new();
        push_u16(&mut entry, GROUP_X25519);
        entry.extend_from_slice(&with_u16_len(pubkey));
        let list = with_u16_len(&entry);
        push_u16(&mut ext, 51);
        ext.extend_from_slice(&with_u16_len(&list));
    }

    body.extend_from_slice(&with_u16_len(&ext));
    handshake_msg(HS_CLIENT_HELLO, &body)
}

struct ServerHello {
    server_pub: [u8; 32],
    cipher: u16,
}

fn parse_server_hello(msg: &[u8]) -> Result<ServerHello, &'static str> {
    // msg = handshake (type 2). Saute l'en-tete (4 octets) deja garanti par l'appelant.
    if msg.len() < 4 || msg[0] != HS_SERVER_HELLO { return Err("pas un ServerHello"); }
    let body = &msg[4..];
    let mut p = 0usize;
    let need = |p: usize, n: usize| -> Result<(), &'static str> {
        if p + n > body.len() { Err("ServerHello tronque") } else { Ok(()) }
    };
    need(p, 2 + 32 + 1)?;
    p += 2; // legacy_version
    // detection HelloRetryRequest
    const HRR: [u8; 32] = [
        0xcf,0x21,0xad,0x74,0xe5,0x9a,0x61,0x11,0xbe,0x1d,0x8c,0x02,0x1e,0x65,0xb8,0x91,
        0xc2,0xa2,0x11,0x16,0x7a,0xbb,0x8c,0x5e,0x07,0x9e,0x09,0xe2,0xc8,0xa8,0x33,0x9c,
    ];
    if body[p..p + 32] == HRR { return Err("HelloRetryRequest (groupe non offert)"); }
    p += 32; // random
    let sid_len = body[p] as usize; p += 1;
    need(p, sid_len + 3)?;
    p += sid_len;
    let cipher = ((body[p] as u16) << 8) | body[p + 1] as u16; p += 2;
    p += 1; // compression
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
        if etype == 51 {
            // key_share : group(2) || length(2) || key
            if edata.len() >= 4 {
                let g = ((edata[0] as u16) << 8) | edata[1] as u16;
                let klen = ((edata[2] as usize) << 8) | edata[3] as usize;
                if g == GROUP_X25519 && klen == 32 && edata.len() >= 4 + 32 {
                    server_pub.copy_from_slice(&edata[4..36]);
                    found_ks = true;
                }
            }
        }
        p += elen;
    }
    if !found_ks { return Err("ServerHello sans key_share x25519"); }
    Ok(ServerHello { server_pub, cipher })
}

// Lit un record TLS brut depuis la connexion : renvoie (en-tete 5 octets, corps).
//
// Important : on ne consomme pas l'en-tete tant que le record complet n'est pas
// disponible. Google envoie souvent la reponse HTTP dans un gros record TLS
// fragmente en plusieurs segments TCP. L'ancienne version faisait `take(5)`
// avant d'attendre le corps ; en cas de timeout/reception partielle, le flux TLS
// etait desynchronise et on finissait par afficher "reponse vide (chiffree)"
// alors que le handshake etait correct.
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

// Construit le contenu signe par CertificateVerify (cote serveur).
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

// Extrait le CN du sujet (pour affichage).
fn subject_cn(cert: &x509::Certificate) -> String {
    // subject = SEQUENCE OF RDN ; cherche l'AttributeTypeAndValue commonName (2.5.4.3).
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
    // 1. Paire de cles ephemere X25519.
    let priv_key = rng::random32();
    let pubkey = x25519::base_mul(&priv_key);
    let random = rng::random32();
    let session_id = rng::random32();

    // 2. ClientHello + transcript.
    let ch = build_client_hello(hostname, &random, &session_id, &pubkey);
    let mut transcript = Sha256::new();
    transcript.update(&ch);
    send_plaintext(&mut conn, CT_HANDSHAKE, &ch);

    // 3. ServerHello (en clair). Ignore d'eventuels ChangeCipherSpec.
    let sh_msg = loop {
        let (hdr, body) = read_raw_record(&mut conn).ok_or("pas de ServerHello (timeout)")?;
        match hdr[0] {
            record::CT_CHANGE_CIPHER_SPEC => continue,
            CT_ALERT => return Err("alerte TLS pendant ServerHello"),
            CT_HANDSHAKE => break body,
            _ => return Err("record inattendu avant ServerHello"),
        }
    };
    transcript.update(&sh_msg);
    let sh = parse_server_hello(&sh_msg)?;
    if sh.cipher != TLS_AES_128_GCM_SHA256 {
        return Err("le serveur n'a pas choisi AES_128_GCM_SHA256");
    }

    // 4. Secret partage ECDHE.
    let shared = x25519::x25519(&priv_key, &sh.server_pub);

    // 5. Cles de handshake.
    let th_ch_sh = transcript.clone().finalize();
    let ks = KeySchedule::derive_handshake(&shared, &th_ch_sh);
    let mut s_hs = DirKeys::new(&ks.server_hs);

    // 6. Lecture du flight serveur chiffre (EE, Certificate, CertVerify, Finished).
    let mut hs_buf: Vec<u8> = Vec::new();
    let mut certs_der: Vec<Vec<u8>> = Vec::new();
    let mut leaf: Option<x509::Certificate> = None;
    let mut th_through_cert: Option<[u8; 32]> = None;

    // Pompe : renvoie de quoi alimenter hs_buf depuis les records chiffres.
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
        // Assure un message handshake complet en tete de hs_buf.
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
                transcript.update(&full);
            }
            HS_CERTIFICATE => {
                parse_certificate_msg(&body, &mut certs_der);
                if let Some(d) = certs_der.first() {
                    leaf = x509::parse(d);
                }
                transcript.update(&full);
                th_through_cert = Some(transcript.clone().finalize());
            }
            HS_CERTIFICATE_VERIFY => {
                let th = th_through_cert.ok_or("CertificateVerify sans Certificate")?;
                let leaf_ref = leaf.as_ref().ok_or("certificat feuille manquant")?;
                if !verify_cert_verify(&body, leaf_ref, &th) {
                    return Err("CertificateVerify invalide (signature serveur)");
                }
                transcript.update(&full);
            }
            HS_FINISHED => {
                let th_cv = transcript.clone().finalize();
                let expected = record::finished_verify(&ks.server_hs, &th_cv);
                if body.len() != expected.len() || body[..] != expected[..] {
                    return Err("Finished serveur invalide (verify_data)");
                }
                transcript.update(&full);
                break;
            }
            _ => {
                // Message handshake inattendu : on l'incorpore au transcript.
                transcript.update(&full);
            }
        }
    }

    // 7. Secrets applicatifs (transcript jusqu'au Finished serveur).
    let th_sf = transcript.clone().finalize();
    let (c_ap_secret, s_ap_secret) = ks.derive_application(&th_sf);

    // 8. Finished client (verify_data sur le meme transcript).
    let cfin = record::finished_verify(&ks.client_hs, &th_sf);
    let fin_msg = handshake_msg(HS_FINISHED, &cfin);
    // ChangeCipherSpec de compatibilite, puis Finished chiffre avec la cle handshake client.
    send_plaintext(&mut conn, record::CT_CHANGE_CIPHER_SPEC, &[0x01]);
    let mut c_hs = DirKeys::new(&ks.client_hs);
    let rec = c_hs.encrypt(CT_HANDSHAKE, &fin_msg);
    conn.send(&rec);

    // 9. Validation de la chaine de certificats.
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
    };

    // 10. Passage aux cles applicatives.
    let c_ap = DirKeys::new(&c_ap_secret);
    let s_ap = DirKeys::new(&s_ap_secret);

    Ok(Session { conn, c_ap, s_ap, report, rx_plain: Vec::new() })
}

fn parse_certificate_msg(body: &[u8], out: &mut Vec<Vec<u8>>) {
    // certificate_request_context (1 octet de longueur)
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

impl Session {
    /// Envoie des donnees applicatives (chiffrees).
    pub fn send_app(&mut self, data: &[u8]) {
        let rec = self.c_ap.encrypt(CT_APPLICATION_DATA, data);
        self.conn.send(&rec);
    }

    /// Recoit des donnees applicatives ; accumule jusqu'a FIN/timeout.
    /// Renvoie l'integralite du flux applicatif dechiffre.
    pub fn recv_all(&mut self, max: usize) -> Vec<u8> {
        let mut out = core::mem::take(&mut self.rx_plain);
        // Tolere plusieurs creux (la 1re reponse arrive apres un RTT complet).
        let mut empty_reads = 0u32;
        loop {
            let rec = match read_raw_record(&mut self.conn) {
                Some(r) => r,
                None => {
                    if self.conn.peer_fin || self.conn.closed { break; }
                    empty_reads += 1;
                    if empty_reads >= 6 { break; }
                    continue;
                }
            };
            empty_reads = 0;
            let (hdr, body) = rec;
            match hdr[0] {
                record::CT_CHANGE_CIPHER_SPEC => continue,
                CT_APPLICATION_DATA => {
                    match self.s_ap.decrypt(&hdr, &body) {
                        Some((inner_type, pt)) => match inner_type {
                            CT_APPLICATION_DATA => {
                                out.extend_from_slice(&pt);
                                if out.len() >= max { break; }
                            }
                            CT_HANDSHAKE => { /* NewSessionTicket/KeyUpdate : ignore */ }
                            CT_ALERT => break, // close_notify ou autre
                            _ => {}
                        },
                        None => break,
                    }
                }
                CT_ALERT => break,
                _ => break,
            }
            if self.conn.peer_fin && self.conn.rx.len() == 0 {
                break;
            }
        }
        out
    }

    pub fn close(&mut self) {
        // close_notify chiffre (alerte 0x01 0x00) puis FIN TCP.
        let rec = self.c_ap.encrypt(CT_ALERT, &[0x01, 0x00]);
        self.conn.send(&rec);
        self.conn.close();
    }
}
