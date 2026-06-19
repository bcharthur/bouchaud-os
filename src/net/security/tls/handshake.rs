//! Handshake TLS 1.3 cote client : ClientHello, lecture du flight serveur
//! chiffre, verification X.509/CertificateVerify/Finished, puis cles applicatives.
//!
//! Cette version garde une pile simple mais adopte un ClientHello plus proche
//! d'un navigateur moderne, tout en forcant HTTP/1.1 dans ALPN pour que le
//! client applicatif actuel puisse parler aux frontaux Google/GitHub.

use super::record::{self, CipherSuite, DirKeys, KeySchedule, CT_HANDSHAKE, CT_ALERT, CT_APPLICATION_DATA};
use super::{hash, x25519, p256, rng, x509, validate};
use crate::net::tcp::TcpConn;
use alloc::vec::Vec;
use alloc::string::String;
use alloc::format;

// Suites TLS 1.3 reellement implementees.
const TLS_AES_128_GCM_SHA256: u16 = record::TLS_AES_128_GCM_SHA256_ID;
const TLS_AES_256_GCM_SHA384: u16 = record::TLS_AES_256_GCM_SHA384_ID;
const TLS_CHACHA20_POLY1305_SHA256: u16 = record::TLS_CHACHA20_POLY1305_SHA256_ID;

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

/// Groupe d'echange de cles ECDHE supporte pour le `key_share` TLS 1.3.
#[derive(Clone, Copy, PartialEq, Eq)]
enum KxGroup {
    X25519,
    Secp256r1,
}

impl KxGroup {
    fn id(self) -> u16 {
        match self {
            KxGroup::X25519 => GROUP_X25519,
            KxGroup::Secp256r1 => GROUP_SECP256R1,
        }
    }
    fn from_id(id: u16) -> Option<KxGroup> {
        match id {
            GROUP_X25519 => Some(KxGroup::X25519),
            GROUP_SECP256R1 => Some(KxGroup::Secp256r1),
            _ => None,
        }
    }
    fn name(self) -> &'static str {
        match self {
            KxGroup::X25519 => "x25519",
            KxGroup::Secp256r1 => "secp256r1",
        }
    }
}

/// Paire de cles ECDHE ephemere : scalaire prive + cle publique encodee.
struct KeyPair {
    group: KxGroup,
    private: [u8; 32],
    public: Vec<u8>,
}

impl KeyPair {
    /// Genere une paire ephemere pour le groupe demande.
    fn generate(group: KxGroup) -> KeyPair {
        let private = rng::random32();
        let public = match group {
            KxGroup::X25519 => x25519::base_mul(&private).to_vec(),
            // Point non compresse 0x04||X||Y (65 octets).
            KxGroup::Secp256r1 => p256::derive_pubkey(&private),
        };
        KeyPair { group, private, public }
    }

    /// Calcule le secret partage ECDHE a partir de la cle publique du serveur.
    fn shared(&self, peer: &[u8]) -> Option<Vec<u8>> {
        match self.group {
            KxGroup::X25519 => {
                if peer.len() != 32 { return None; }
                let mut p = [0u8; 32];
                p.copy_from_slice(peer);
                Some(x25519::x25519(&self.private, &p).to_vec())
            }
            KxGroup::Secp256r1 => p256::ecdh(&self.private, peer).map(|x| x.to_vec()),
        }
    }
}

/// Rapport sur la validation du certificat serveur.
pub struct CertReport {
    pub trusted: bool,
    pub hostname_ok: bool,
    pub expired: bool,
    pub detail: String,
    pub subject_cn: String,
    pub cipher_suite: &'static str,
    pub kx_group: &'static str,
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
    /// Protocole ALPN selectionne par le serveur ("h2", "http/1.1" ou "").
    pub alpn: String,
    rx_plain: Vec<u8>,
}

// Extrait le protocole ALPN selectionne d'un message EncryptedExtensions.
fn parse_alpn(ee_body: &[u8]) -> Option<String> {
    if ee_body.len() < 2 { return None; }
    let ext_len = ((ee_body[0] as usize) << 8) | ee_body[1] as usize;
    let mut p = 2usize;
    let end = (2 + ext_len).min(ee_body.len());
    while p + 4 <= end {
        let etype = ((ee_body[p] as u16) << 8) | ee_body[p + 1] as u16;
        let elen = ((ee_body[p + 2] as usize) << 8) | ee_body[p + 3] as usize;
        p += 4;
        if p + elen > end { break; }
        if etype == 16 && elen >= 3 {
            // ProtocolNameList : u16 list_len, puis u8 name_len || name.
            let name_len = ee_body[p + 2] as usize;
            if p + 3 + name_len <= p + elen {
                let name = &ee_body[p + 3..p + 3 + name_len];
                return core::str::from_utf8(name).ok().map(String::from);
            }
        }
        p += elen;
    }
    None
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
/// ALPN propose `h2` puis `http/1.1` ; la couche applicative choisit la pile
/// HTTP correspondant au protocole selectionne par le serveur.
///
/// `kp` porte le groupe ECDHE et la cle publique du `key_share` offert.
/// `cookie` (extension 44) est echoe tel quel apres un HelloRetryRequest.
fn build_client_hello(hostname: &str, random: &[u8; 32], kp: &KeyPair, cookie: Option<&[u8]>) -> Vec<u8> {
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
    push_u16(&mut suites, TLS_CHACHA20_POLY1305_SHA256);
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

    // ALPN : HTTP/1.1 uniquement. Le chemin HTTP/1.1 (chunked + gzip/deflate/br)
    // est robuste et eprouve ; on evite que Google/Cloudflare imposent h2 (dont
    // la lecture de frames n'est pas encore fiable cote client).
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

    // cookie (extension 44) : echo obligatoire apres un HelloRetryRequest.
    if let Some(c) = cookie {
        push_ext(&mut ext, 44, c);
    }

    // key_share : un seul groupe offert (celui de `kp`).
    {
        let mut entry = Vec::new();
        push_u16(&mut entry, kp.group.id());
        entry.extend_from_slice(&with_u16_len(&kp.public));
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

// Aleatoire magique signalant un HelloRetryRequest (RFC 8446 §4.1.3).
const HRR_RANDOM: [u8; 32] = [
    0xcf,0x21,0xad,0x74,0xe5,0x9a,0x61,0x11,0xbe,0x1d,0x8c,0x02,0x1e,0x65,0xb8,0x91,
    0xc2,0xa2,0x11,0x16,0x7a,0xbb,0x8c,0x5e,0x07,0x9e,0x09,0xe2,0xc8,0xa8,0x33,0x9c,
];

enum ServerHelloKind {
    /// ServerHello reel : key_share du serveur (taille variable selon le groupe).
    Hello { group: u16, server_pub: Vec<u8>, cipher: u16 },
    /// HelloRetryRequest : le serveur impose un autre groupe (+ cookie eventuel).
    Retry { cipher: u16, group: u16, cookie: Option<Vec<u8>> },
}

fn parse_server_hello(msg: &[u8]) -> Result<ServerHelloKind, &'static str> {
    if msg.len() < 4 || msg[0] != HS_SERVER_HELLO { return Err("pas un ServerHello"); }
    let body = &msg[4..];
    let mut p = 0usize;
    let need = |p: usize, n: usize| -> Result<(), &'static str> {
        if p + n > body.len() { Err("ServerHello tronque") } else { Ok(()) }
    };
    need(p, 2 + 32 + 1)?;
    p += 2; // legacy_version
    let is_hrr = body[p..p + 32] == HRR_RANDOM;
    p += 32;
    let sid_len = body[p] as usize; p += 1;
    need(p, sid_len + 3)?;
    p += sid_len;
    let cipher = ((body[p] as u16) << 8) | body[p + 1] as u16; p += 2;
    p += 1; // legacy_compression_method
    need(p, 2)?;
    let ext_len = ((body[p] as usize) << 8) | body[p + 1] as usize; p += 2;
    need(p, ext_len)?;
    let ext_end = p + ext_len;

    let mut server_pub: Option<Vec<u8>> = None;
    let mut sel_group: Option<u16> = None;
    let mut cookie: Option<Vec<u8>> = None;
    while p + 4 <= ext_end {
        let etype = ((body[p] as u16) << 8) | body[p + 1] as u16;
        let elen = ((body[p + 2] as usize) << 8) | body[p + 3] as usize;
        p += 4;
        if p + elen > ext_end { return Err("extension SH tronquee"); }
        let edata = &body[p..p + elen];
        match etype {
            51 => {
                if is_hrr {
                    // KeyShareHelloRetryRequest = juste le groupe selectionne.
                    if edata.len() >= 2 {
                        sel_group = Some(((edata[0] as u16) << 8) | edata[1] as u16);
                    }
                } else if edata.len() >= 4 {
                    // KeyShareEntry = group(2) || key_exchange<u16>.
                    let g = ((edata[0] as u16) << 8) | edata[1] as u16;
                    let klen = ((edata[2] as usize) << 8) | edata[3] as usize;
                    if edata.len() >= 4 + klen {
                        sel_group = Some(g);
                        server_pub = Some(edata[4..4 + klen].to_vec());
                    }
                }
            }
            44 => {
                // cookie : echo brut (l'extension contient deja sa longueur interne).
                cookie = Some(edata.to_vec());
            }
            _ => {}
        }
        p += elen;
    }

    if is_hrr {
        let group = sel_group.ok_or("HelloRetryRequest sans groupe")?;
        return Ok(ServerHelloKind::Retry { cipher, group, cookie });
    }
    let group = sel_group.ok_or("ServerHello sans key_share")?;
    let server_pub = server_pub.ok_or("ServerHello sans cle publique")?;
    Ok(ServerHelloKind::Hello { group, server_pub, cipher })
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

// Lit un message handshake (ServerHello / HRR) en sautant les CCS, et traduit
// une alerte recue en message lisible.
fn read_server_hello(conn: &mut TcpConn) -> Result<Vec<u8>, &'static str> {
    loop {
        let (hdr, body) = read_raw_record(conn).ok_or("pas de ServerHello (timeout)")?;
        match hdr[0] {
            record::CT_CHANGE_CIPHER_SPEC => continue,
            CT_ALERT => {
                let code = if body.len() >= 2 { body[1] } else { 0 };
                return Err(super::alert::handshake_error(code));
            }
            CT_HANDSHAKE => return Ok(body),
            _ => return Err("record inattendu avant ServerHello"),
        }
    }
}

/// Effectue le handshake TLS 1.3 complet sur une connexion TCP deja ouverte.
pub fn connect(mut conn: TcpConn, hostname: &str) -> Result<Session, &'static str> {
    // Premier ClientHello : key_share x25519 (accepte par la quasi-totalite des
    // serveurs). Si le serveur impose un autre groupe, il repond par un
    // HelloRetryRequest et on rejoue avec le groupe demande (ex. secp256r1).
    let mut kp = KeyPair::generate(KxGroup::X25519);
    let random = rng::random32();

    let ch = build_client_hello(hostname, &random, &kp, None);
    let mut transcript: Vec<u8> = Vec::new();
    transcript.extend_from_slice(&ch);
    send_plaintext(&mut conn, CT_HANDSHAKE, &ch);

    let sh1 = read_server_hello(&mut conn)?;

    let (server_pub, suite) = match parse_server_hello(&sh1)? {
        ServerHelloKind::Retry { cipher, group, cookie } => {
            let suite = CipherSuite::from_id(cipher).ok_or("suite TLS 1.3 non implementee (HRR)")?;
            let new_group = KxGroup::from_id(group)
                .ok_or("HelloRetryRequest: groupe ECDHE non supporte")?;

            // Transcript apres HRR (RFC 8446 §4.4.1) : ClientHello1 est remplace
            // par un message synthetique message_hash(254) || len || H(CH1).
            let ch1_hash = transcript_hash(suite, &transcript);
            let mut t = Vec::new();
            t.push(254);
            t.push(0);
            t.push((ch1_hash.len() >> 8) as u8);
            t.push(ch1_hash.len() as u8);
            t.extend_from_slice(&ch1_hash);
            t.extend_from_slice(&sh1); // HelloRetryRequest

            // Rejoue le ClientHello avec le groupe impose + echo du cookie.
            kp = KeyPair::generate(new_group);
            let ch2 = build_client_hello(hostname, &random, &kp, cookie.as_deref());
            t.extend_from_slice(&ch2);
            send_plaintext(&mut conn, CT_HANDSHAKE, &ch2);
            transcript = t;

            let sh2 = read_server_hello(&mut conn)?;
            transcript.extend_from_slice(&sh2);
            match parse_server_hello(&sh2)? {
                ServerHelloKind::Hello { group, server_pub, cipher } => {
                    if group != new_group.id() {
                        return Err("ServerHello apres HRR : groupe inattendu");
                    }
                    let suite = CipherSuite::from_id(cipher).ok_or("suite TLS 1.3 non implementee")?;
                    (server_pub, suite)
                }
                ServerHelloKind::Retry { .. } => return Err("deux HelloRetryRequest (interdit)"),
            }
        }
        ServerHelloKind::Hello { group, server_pub, cipher } => {
            if group != kp.group.id() {
                return Err("ServerHello : groupe key_share non offert");
            }
            transcript.extend_from_slice(&sh1);
            let suite = CipherSuite::from_id(cipher).ok_or("suite TLS 1.3 non implementee")?;
            (server_pub, suite)
        }
    };

    let shared = kp.shared(&server_pub).ok_or("echange ECDHE invalide")?;
    let th_ch_sh = transcript_hash(suite, &transcript);
    let ks = KeySchedule::derive_handshake(suite, &shared, &th_ch_sh);
    let mut s_hs = DirKeys::new(suite, &ks.server_hs);

    let mut hs_buf: Vec<u8> = Vec::new();
    let mut certs_der: Vec<Vec<u8>> = Vec::new();
    let mut leaf: Option<x509::Certificate> = None;
    let mut th_through_cert: Option<Vec<u8>> = None;
    let mut alpn = String::new();

    let feed = |conn: &mut TcpConn, hs_buf: &mut Vec<u8>, s_hs: &mut DirKeys| -> Result<(), &'static str> {
        loop {
            let (hdr, body) = read_raw_record(conn).ok_or("flight serveur incomplet")?;
            match hdr[0] {
                record::CT_CHANGE_CIPHER_SPEC => continue,
                CT_APPLICATION_DATA => {
                    let (inner_type, pt) = s_hs.decrypt(&hdr, &body).ok_or("dechiffrement handshake echoue")?;
                    match inner_type {
                        CT_HANDSHAKE => { hs_buf.extend_from_slice(&pt); return Ok(()); }
                        CT_ALERT => {
                            let code = if pt.len() >= 2 { pt[1] } else { 0 };
                            return Err(super::alert::handshake_error(code));
                        }
                        _ => continue,
                    }
                }
                CT_ALERT => {
                    let code = if body.len() >= 2 { body[1] } else { 0 };
                    return Err(super::alert::handshake_error(code));
                }
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
                if let Some(p) = parse_alpn(&body) { alpn = p; }
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
        kx_group: kp.group.name(),
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
        alpn,
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

    /// Lit le prochain bloc de donnees applicatives dechiffrees (un ou plusieurs
    /// records), sans attendre la fermeture. Renvoie `None` a la fin du flux
    /// (peer_fin/closed, alerte, ou plus aucun record disponible). Utilise par la
    /// pile HTTP/2 qui doit lire/repondre des frames de maniere interactive.
    pub fn recv_some(&mut self) -> Option<Vec<u8>> {
        if !self.rx_plain.is_empty() {
            return Some(core::mem::take(&mut self.rx_plain));
        }
        let mut empty_reads = 0u32;
        loop {
            match read_raw_record(&mut self.conn) {
                Some((hdr, body)) => match hdr[0] {
                    record::CT_CHANGE_CIPHER_SPEC => continue,
                    CT_APPLICATION_DATA => match self.s_ap.decrypt(&hdr, &body) {
                        Some((CT_APPLICATION_DATA, pt)) => {
                            if pt.is_empty() { continue; }
                            return Some(pt);
                        }
                        // NewSessionTicket / KeyUpdate post-handshake : ignores.
                        Some((CT_HANDSHAKE, _)) => continue,
                        Some((CT_ALERT, _)) => return None,
                        _ => continue,
                    },
                    CT_ALERT => return None,
                    _ => return None,
                },
                None => {
                    if self.conn.peer_fin || self.conn.closed { return None; }
                    empty_reads += 1;
                    if empty_reads >= 2 { return None; }
                }
            }
        }
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
                    if empty_reads >= 2 { break; }
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
                                        trace.push(format!(
                                            "recv: alerte TLS {} {} ({})",
                                            super::alert::level_name(pt[0]),
                                            super::alert::description(pt[1]),
                                            pt[1],
                                        ));
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
                    if body.len() >= 2 {
                        trace.push(format!(
                            "recv: alerte TLS claire {} {} ({})",
                            super::alert::level_name(body[0]),
                            super::alert::description(body[1]),
                            body[1],
                        ));
                    } else {
                        trace.push(format!("recv: alerte TLS claire len={}", body.len()));
                    }
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
