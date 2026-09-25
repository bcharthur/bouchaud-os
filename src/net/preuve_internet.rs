//! BOUCHAUD_HOTFIX10_INTERNET_PROOF_CHAIN_V1
//!
//! Preuve active et asynchrone de la chaine Internet native :
//! lien/config -> ARP gateway -> DNS -> TCP:80 -> TCP:443 -> TLS -> HTTP.
//!
//! Ce module ne repare rien et ne change aucune configuration. Il exerce les
//! chemins existants et publie le premier etage manquant.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const HOTE: &str = "example.com";

pub const ETAPE_IDLE: u64 = 0;
pub const ETAPE_CONFIG: u64 = 1;
pub const ETAPE_ARP: u64 = 2;
pub const ETAPE_DNS: u64 = 3;
pub const ETAPE_TCP80: u64 = 4;
pub const ETAPE_TCP443: u64 = 5;
pub const ETAPE_TLS: u64 = 6;
pub const ETAPE_HTTP: u64 = 7;
pub const ETAPE_TERMINEE: u64 = 8;

pub const ECHEC_AUCUN: u64 = 0;
pub const ECHEC_LIEN: u64 = 1;
pub const ECHEC_IPV4: u64 = 2;
pub const ECHEC_PASSERELLE: u64 = 3;
pub const ECHEC_DNS_CONFIG: u64 = 4;
pub const ECHEC_ARP: u64 = 5;
pub const ECHEC_DNS: u64 = 6;
pub const ECHEC_TCP443: u64 = 7;
pub const ECHEC_TLS_HANDSHAKE: u64 = 8;
pub const ECHEC_TLS_CERTIFICAT: u64 = 9;
pub const ECHEC_HTTP_ENVOI: u64 = 10;
pub const ECHEC_HTTP_REPONSE: u64 = 11;
pub const ECHEC_SPAWN: u64 = 12;

static GENERATION: AtomicU64 = AtomicU64::new(0);
static EN_COURS: AtomicBool = AtomicBool::new(false);
static TERMINEE: AtomicBool = AtomicBool::new(false);
static CHAINE_OK: AtomicBool = AtomicBool::new(false);
static ETAPE: AtomicU64 = AtomicU64::new(ETAPE_IDLE);
static PREMIER_ECHEC: AtomicU64 = AtomicU64::new(ECHEC_AUCUN);
static DEBUT_MS: AtomicU64 = AtomicU64::new(0);
static FIN_MS: AtomicU64 = AtomicU64::new(0);

static LIEN: AtomicBool = AtomicBool::new(false);
static BAIL: AtomicBool = AtomicBool::new(false);
static IP: AtomicU64 = AtomicU64::new(0);
static PASSERELLE: AtomicU64 = AtomicU64::new(0);
static DNS: AtomicU64 = AtomicU64::new(0);

static ARP_OK: AtomicBool = AtomicBool::new(false);
static ARP_MS: AtomicU64 = AtomicU64::new(0);
static ARP_ROUTEES_DELTA: AtomicU64 = AtomicU64::new(0);
static ARP_VUES_DELTA: AtomicU64 = AtomicU64::new(0);
static ARP_RESOLUES_DELTA: AtomicU64 = AtomicU64::new(0);
static ARP_ECHECS_DELTA: AtomicU64 = AtomicU64::new(0);
static ARP_NON_EMISES_DELTA: AtomicU64 = AtomicU64::new(0);

static DNS_OK: AtomicBool = AtomicBool::new(false);
static DNS_MS: AtomicU64 = AtomicU64::new(0);
static DNS_IP: AtomicU64 = AtomicU64::new(0);
static DNS_DELTA: [AtomicU64; crate::net::sonde_dns::BARREAUX] = [
    AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0),
    AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0),
    AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0),
];

static TCP80_OK: AtomicBool = AtomicBool::new(false);
static TCP80_MS: AtomicU64 = AtomicU64::new(0);
static TCP80_OCTETS: AtomicU64 = AtomicU64::new(0);
static TCP80_STATUS: AtomicU64 = AtomicU64::new(0);

static TCP443_OK: AtomicBool = AtomicBool::new(false);
static TCP443_MS: AtomicU64 = AtomicU64::new(0);
static TCP_POIGNEES_DELTA: AtomicU64 = AtomicU64::new(0);
static TCP_SYN_RETX_DELTA: AtomicU64 = AtomicU64::new(0);

static TLS_OK: AtomicBool = AtomicBool::new(false);
static TLS_MS: AtomicU64 = AtomicU64::new(0);
static TLS_ERREUR: AtomicU64 = AtomicU64::new(0);
static TLS_TRUSTED: AtomicBool = AtomicBool::new(false);
static TLS_HOSTNAME_OK: AtomicBool = AtomicBool::new(false);
static TLS_EXPIRED: AtomicBool = AtomicBool::new(false);
static TLS_CIPHER: AtomicU64 = AtomicU64::new(0);
static TLS_KX: AtomicU64 = AtomicU64::new(0);
static TLS_ALPN: AtomicU64 = AtomicU64::new(0);

static HTTP_SENT: AtomicBool = AtomicBool::new(false);
static HTTP_OK: AtomicBool = AtomicBool::new(false);
static HTTP_MS: AtomicU64 = AtomicU64::new(0);
static HTTP_RAW: AtomicU64 = AtomicU64::new(0);
static HTTP_BODY: AtomicU64 = AtomicU64::new(0);
static HTTP_STATUS: AtomicU64 = AtomicU64::new(0);
static HTTP_HTML: AtomicBool = AtomicBool::new(false);
static HTTP_COMPLETE: AtomicBool = AtomicBool::new(false);

static RX_START: AtomicU64 = AtomicU64::new(0);
static RX_END: AtomicU64 = AtomicU64::new(0);
static TX_START: AtomicU64 = AtomicU64::new(0);
static TX_END: AtomicU64 = AtomicU64::new(0);
static RESET_START: AtomicU64 = AtomicU64::new(0);
static RESET_END: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug)]
pub struct Releve {
    pub generation: u64,
    pub en_cours: bool,
    pub terminee: bool,
    pub chaine_ok: bool,
    pub etape: u64,
    pub premier_echec: u64,
    pub debut_ms: u64,
    pub fin_ms: u64,
    pub lien: bool,
    pub bail: bool,
    pub ip: [u8; 4],
    pub passerelle: [u8; 4],
    pub dns: [u8; 4],
    pub arp_ok: bool,
    pub arp_ms: u64,
    pub arp_routees_delta: u64,
    pub arp_vues_delta: u64,
    pub arp_resolues_delta: u64,
    pub arp_echecs_delta: u64,
    pub arp_non_emises_delta: u64,
    pub dns_ok: bool,
    pub dns_ms: u64,
    pub dns_ip: [u8; 4],
    pub dns_delta: [u64; crate::net::sonde_dns::BARREAUX],
    pub tcp80_ok: bool,
    pub tcp80_ms: u64,
    pub tcp80_octets: u64,
    pub tcp80_status: u64,
    pub tcp443_ok: bool,
    pub tcp443_ms: u64,
    pub tcp_poignees_delta: u64,
    pub tcp_syn_retx_delta: u64,
    pub tls_ok: bool,
    pub tls_ms: u64,
    pub tls_erreur: u64,
    pub tls_trusted: bool,
    pub tls_hostname_ok: bool,
    pub tls_expired: bool,
    pub tls_cipher: u64,
    pub tls_kx: u64,
    pub tls_alpn: u64,
    pub http_sent: bool,
    pub http_ok: bool,
    pub http_ms: u64,
    pub http_raw: u64,
    pub http_body: u64,
    pub http_status: u64,
    pub http_html: bool,
    pub http_complete: bool,
    pub rx_start: u64,
    pub rx_end: u64,
    pub tx_start: u64,
    pub tx_end: u64,
    pub reset_start: u64,
    pub reset_end: u64,
}

fn ip_pack(ip: [u8; 4]) -> u64 { u32::from_be_bytes(ip) as u64 }
fn ip_unpack(v: u64) -> [u8; 4] { (v as u32).to_be_bytes() }
fn zero_ip(ip: [u8; 4]) -> bool { ip == [0, 0, 0, 0] }

fn note_echec(code: u64) {
    let _ = PREMIER_ECHEC.compare_exchange(
        ECHEC_AUCUN, code, Ordering::AcqRel, Ordering::Acquire,
    );
}

fn snapshot_nic_start() {
    let n = crate::drivers::rtl8168::releve();
    RX_START.store(n.rx_paquets, Ordering::Relaxed);
    TX_START.store(n.tx_paquets, Ordering::Relaxed);
    RESET_START.store(n.reinitialisations, Ordering::Relaxed);
    RX_END.store(n.rx_paquets, Ordering::Relaxed);
    TX_END.store(n.tx_paquets, Ordering::Relaxed);
    RESET_END.store(n.reinitialisations, Ordering::Relaxed);
}

fn snapshot_nic_end() {
    let n = crate::drivers::rtl8168::releve();
    RX_END.store(n.rx_paquets, Ordering::Relaxed);
    TX_END.store(n.tx_paquets, Ordering::Relaxed);
    RESET_END.store(n.reinitialisations, Ordering::Relaxed);
}

fn reset_mesures(generation: u64) {
    TERMINEE.store(false, Ordering::Release);
    CHAINE_OK.store(false, Ordering::Release);
    ETAPE.store(ETAPE_CONFIG, Ordering::Release);
    PREMIER_ECHEC.store(ECHEC_AUCUN, Ordering::Release);
    DEBUT_MS.store(crate::kernel::timer::monotonic_ms(), Ordering::Relaxed);
    FIN_MS.store(0, Ordering::Relaxed);

    LIEN.store(false, Ordering::Relaxed);
    BAIL.store(false, Ordering::Relaxed);
    IP.store(0, Ordering::Relaxed);
    PASSERELLE.store(0, Ordering::Relaxed);
    DNS.store(0, Ordering::Relaxed);

    ARP_OK.store(false, Ordering::Relaxed);
    ARP_MS.store(0, Ordering::Relaxed);
    ARP_ROUTEES_DELTA.store(0, Ordering::Relaxed);
    ARP_VUES_DELTA.store(0, Ordering::Relaxed);
    ARP_RESOLUES_DELTA.store(0, Ordering::Relaxed);
    ARP_ECHECS_DELTA.store(0, Ordering::Relaxed);
    ARP_NON_EMISES_DELTA.store(0, Ordering::Relaxed);

    DNS_OK.store(false, Ordering::Relaxed);
    DNS_MS.store(0, Ordering::Relaxed);
    DNS_IP.store(0, Ordering::Relaxed);
    for c in &DNS_DELTA { c.store(0, Ordering::Relaxed); }

    TCP80_OK.store(false, Ordering::Relaxed);
    TCP80_MS.store(0, Ordering::Relaxed);
    TCP80_OCTETS.store(0, Ordering::Relaxed);
    TCP80_STATUS.store(0, Ordering::Relaxed);

    TCP443_OK.store(false, Ordering::Relaxed);
    TCP443_MS.store(0, Ordering::Relaxed);
    TCP_POIGNEES_DELTA.store(0, Ordering::Relaxed);
    TCP_SYN_RETX_DELTA.store(0, Ordering::Relaxed);

    TLS_OK.store(false, Ordering::Relaxed);
    TLS_MS.store(0, Ordering::Relaxed);
    TLS_ERREUR.store(0, Ordering::Relaxed);
    TLS_TRUSTED.store(false, Ordering::Relaxed);
    TLS_HOSTNAME_OK.store(false, Ordering::Relaxed);
    TLS_EXPIRED.store(false, Ordering::Relaxed);
    TLS_CIPHER.store(0, Ordering::Relaxed);
    TLS_KX.store(0, Ordering::Relaxed);
    TLS_ALPN.store(0, Ordering::Relaxed);

    HTTP_SENT.store(false, Ordering::Relaxed);
    HTTP_OK.store(false, Ordering::Relaxed);
    HTTP_MS.store(0, Ordering::Relaxed);
    HTTP_RAW.store(0, Ordering::Relaxed);
    HTTP_BODY.store(0, Ordering::Relaxed);
    HTTP_STATUS.store(0, Ordering::Relaxed);
    HTTP_HTML.store(false, Ordering::Relaxed);
    HTTP_COMPLETE.store(false, Ordering::Relaxed);

    snapshot_nic_start();
    crate::serial_println!(
        "BOUCHAUD_INTERNET_PROOF generation={} stage=start", generation
    );
}

fn termine(ok: bool) {
    snapshot_nic_end();
    CHAINE_OK.store(ok, Ordering::Release);
    ETAPE.store(ETAPE_TERMINEE, Ordering::Release);
    FIN_MS.store(crate::kernel::timer::monotonic_ms(), Ordering::Relaxed);
    TERMINEE.store(true, Ordering::Release);
    EN_COURS.store(false, Ordering::Release);
    let r = releve();
    crate::serial_println!(
        "BOUCHAUD_INTERNET_PROOF generation={} stage=final ok={} first_failure={} \
rx_delta={} tx_delta={} reset_delta={} http_status={}",
        r.generation, r.chaine_ok as u8, r.premier_echec,
        r.rx_end.saturating_sub(r.rx_start),
        r.tx_end.saturating_sub(r.tx_start),
        r.reset_end.saturating_sub(r.reset_start),
        r.http_status,
    );
}

fn cipher_code(name: &str) -> u64 {
    match name {
        "TLS_AES_128_GCM_SHA256" => 0x1301,
        "TLS_AES_256_GCM_SHA384" => 0x1302,
        "TLS_CHACHA20_POLY1305_SHA256" => 0x1303,
        _ => 0xffff,
    }
}
fn kx_code(name: &str) -> u64 {
    match name { "x25519" => 1, "secp256r1" => 2, _ => 255 }
}
fn alpn_code(name: &str) -> u64 {
    match name { "" => 0, "http/1.1" => 1, "h2" => 2, _ => 255 }
}
fn tls_error_class(e: &str) -> u64 {
    if e.contains("ServerHello") || e.contains("timeout") { 1 }
    else if e.contains("alerte") || e.contains("alert") { 2 }
    else if e.contains("groupe") || e.contains("suite") || e.contains("key_share") || e.contains("ECDHE") { 3 }
    else if e.contains("dechiffrement") || e.contains("flight") || e.contains("record") { 4 }
    else if e.contains("Certificate") || e.contains("certificat") { 5 }
    else if e.contains("Finished") { 6 }
    else { 255 }
}

// BOUCHAUD_HOTFIX14_INTERNET_PROOF_DROP_SCOPE_V1
// La logique retourne normalement afin que Rust execute Drop sur tous
// les Vec/String/Session/TcpConn avant l'abandon definitif de la pile.
fn execute_preuve() -> i32 {
    use alloc::vec::Vec;

    let generation = GENERATION.load(Ordering::Acquire);

    ETAPE.store(ETAPE_CONFIG, Ordering::Release);
    let lien = crate::drivers::e1000::link_up();
    let ip = crate::net::our_ip();
    let gw = crate::net::gateway();
    let dns = crate::net::dns_server();
    let bail = crate::net::bail_obtenu();

    LIEN.store(lien, Ordering::Relaxed);
    BAIL.store(bail, Ordering::Relaxed);
    IP.store(ip_pack(ip), Ordering::Relaxed);
    PASSERELLE.store(ip_pack(gw), Ordering::Relaxed);
    DNS.store(ip_pack(dns), Ordering::Relaxed);

    crate::serial_println!(
        "BOUCHAUD_INTERNET_PROOF generation={} stage=config link={} bail={} \
ip={}.{}.{}.{} gw={}.{}.{}.{} dns={}.{}.{}.{}",
        generation, lien as u8, bail as u8,
        ip[0], ip[1], ip[2], ip[3],
        gw[0], gw[1], gw[2], gw[3],
        dns[0], dns[1], dns[2], dns[3],
    );

    if !lien { note_echec(ECHEC_LIEN); termine(false); return 1; }
    if zero_ip(ip) { note_echec(ECHEC_IPV4); termine(false); return 2; }
    if zero_ip(gw) { note_echec(ECHEC_PASSERELLE); termine(false); return 3; }
    if zero_ip(dns) { note_echec(ECHEC_DNS_CONFIG); termine(false); return 4; }

    ETAPE.store(ETAPE_ARP, Ordering::Release);
    let a0 = crate::net::compteurs_routage();
    crate::net::oublie_voisin(gw);
    let t0 = crate::kernel::timer::monotonic_ms();
    let arp = crate::net::resout_voisin(gw);
    let ms = crate::kernel::timer::monotonic_ms().saturating_sub(t0);
    let a1 = crate::net::compteurs_routage();

    ARP_OK.store(arp.is_some(), Ordering::Relaxed);
    ARP_MS.store(ms, Ordering::Relaxed);
    ARP_ROUTEES_DELTA.store(a1.0.saturating_sub(a0.0), Ordering::Relaxed);
    ARP_VUES_DELTA.store(a1.1.saturating_sub(a0.1), Ordering::Relaxed);
    ARP_RESOLUES_DELTA.store(a1.3.saturating_sub(a0.3), Ordering::Relaxed);
    ARP_ECHECS_DELTA.store(a1.4.saturating_sub(a0.4), Ordering::Relaxed);
    ARP_NON_EMISES_DELTA.store(a1.5.saturating_sub(a0.5), Ordering::Relaxed);

    crate::serial_println!(
        "BOUCHAUD_INTERNET_PROOF generation={} stage=arp ok={} ms={} routees={} vues={} resolues={} echecs={} non_emises={}",
        generation, arp.is_some() as u8, ms,
        a1.0.saturating_sub(a0.0), a1.1.saturating_sub(a0.1),
        a1.3.saturating_sub(a0.3), a1.4.saturating_sub(a0.4),
        a1.5.saturating_sub(a0.5),
    );
    if arp.is_none() { note_echec(ECHEC_ARP); termine(false); return 5; }

    ETAPE.store(ETAPE_DNS, Ordering::Release);
    let d0 = crate::net::sonde_dns::releve();
    let t0 = crate::kernel::timer::monotonic_ms();
    let resolved = crate::net::resolve(HOTE);
    let ms = crate::kernel::timer::monotonic_ms().saturating_sub(t0);
    let d1 = crate::net::sonde_dns::releve();

    for i in 0..crate::net::sonde_dns::BARREAUX {
        DNS_DELTA[i].store(d1[i].saturating_sub(d0[i]), Ordering::Relaxed);
    }
    DNS_OK.store(resolved.is_some(), Ordering::Relaxed);
    DNS_MS.store(ms, Ordering::Relaxed);
    if let Some(v) = resolved { DNS_IP.store(ip_pack(v), Ordering::Relaxed); }

    crate::serial_println!(
        "BOUCHAUD_INTERNET_PROOF generation={} stage=dns ok={} ms={} eth={} ipv4={} udp={} queued={} dequeued={} socket_match={} socket_busy={} socket_delivered={} poll_ready={} recv_ok={} recv_empty={}",
        generation, resolved.is_some() as u8, ms,
        DNS_DELTA[0].load(Ordering::Relaxed), DNS_DELTA[1].load(Ordering::Relaxed),
        DNS_DELTA[2].load(Ordering::Relaxed), DNS_DELTA[3].load(Ordering::Relaxed),
        DNS_DELTA[4].load(Ordering::Relaxed), DNS_DELTA[5].load(Ordering::Relaxed),
        DNS_DELTA[6].load(Ordering::Relaxed), DNS_DELTA[7].load(Ordering::Relaxed),
        DNS_DELTA[8].load(Ordering::Relaxed), DNS_DELTA[9].load(Ordering::Relaxed),
        DNS_DELTA[10].load(Ordering::Relaxed),
    );

    let Some(resolved) = resolved else {
        note_echec(ECHEC_DNS); termine(false); return 6;
    };

    ETAPE.store(ETAPE_TCP80, Ordering::Release);
    let mut out80: Vec<u8> = Vec::new();
    let request80 = b"GET / HTTP/1.0\r\nHost: example.com\r\nConnection: close\r\n\r\n";
    let t0 = crate::kernel::timer::monotonic_ms();
    let transport80 = crate::net::transport::smol_tcp::fetch(resolved, 80, request80, &mut out80);
    let ms = crate::kernel::timer::monotonic_ms().saturating_sub(t0);
    let status80 = crate::net::http::parse_response(&out80).map(|r| r.status_code as u64).unwrap_or(0);
    TCP80_OK.store(transport80 && !out80.is_empty(), Ordering::Relaxed);
    TCP80_MS.store(ms, Ordering::Relaxed);
    TCP80_OCTETS.store(out80.len() as u64, Ordering::Relaxed);
    TCP80_STATUS.store(status80, Ordering::Relaxed);
    crate::serial_println!(
        "BOUCHAUD_INTERNET_PROOF generation={} stage=tcp80 ok={} ms={} bytes={} http_status={}",
        generation, TCP80_OK.load(Ordering::Relaxed) as u8, ms, out80.len(), status80
    );

    ETAPE.store(ETAPE_TCP443, Ordering::Release);
    let h0 = crate::net::transport::retransmission::stats_poignee();
    let t0 = crate::kernel::timer::monotonic_ms();
    let conn = crate::net::tcp::TcpConn::connect(resolved, 443);
    let ms = crate::kernel::timer::monotonic_ms().saturating_sub(t0);
    let h1 = crate::net::transport::retransmission::stats_poignee();
    TCP443_OK.store(conn.is_some(), Ordering::Relaxed);
    TCP443_MS.store(ms, Ordering::Relaxed);
    TCP_POIGNEES_DELTA.store(h1.0.saturating_sub(h0.0), Ordering::Relaxed);
    TCP_SYN_RETX_DELTA.store(h1.1.saturating_sub(h0.1), Ordering::Relaxed);
    crate::serial_println!(
        "BOUCHAUD_INTERNET_PROOF generation={} stage=tcp443 ok={} ms={} handshakes={} syn_retx={}",
        generation, conn.is_some() as u8, ms,
        h1.0.saturating_sub(h0.0), h1.1.saturating_sub(h0.1),
    );
    let Some(conn) = conn else {
        note_echec(ECHEC_TCP443); termine(false); return 7;
    };

    ETAPE.store(ETAPE_TLS, Ordering::Release);
    let t0 = crate::kernel::timer::monotonic_ms();
    let session = crate::net::tls::handshake::connect(conn, HOTE);
    let ms = crate::kernel::timer::monotonic_ms().saturating_sub(t0);
    TLS_MS.store(ms, Ordering::Relaxed);

    let mut sess = match session {
        Ok(s) => s,
        Err(e) => {
            let cl = tls_error_class(e);
            TLS_ERREUR.store(cl, Ordering::Relaxed);
            note_echec(ECHEC_TLS_HANDSHAKE);
            crate::serial_println!(
                "BOUCHAUD_INTERNET_PROOF generation={} stage=tls ok=0 ms={} error_class={}",
                generation, ms, cl
            );
            termine(false);
            return 8;
        }
    };

    TLS_OK.store(true, Ordering::Relaxed);
    TLS_TRUSTED.store(sess.report.trusted, Ordering::Relaxed);
    TLS_HOSTNAME_OK.store(sess.report.hostname_ok, Ordering::Relaxed);
    TLS_EXPIRED.store(sess.report.expired, Ordering::Relaxed);
    TLS_CIPHER.store(cipher_code(sess.report.cipher_suite), Ordering::Relaxed);
    TLS_KX.store(kx_code(sess.report.kx_group), Ordering::Relaxed);
    TLS_ALPN.store(alpn_code(sess.alpn.as_str()), Ordering::Relaxed);

    let cert_ok = sess.report.trusted && sess.report.hostname_ok && !sess.report.expired;
    if !cert_ok { note_echec(ECHEC_TLS_CERTIFICAT); }

    crate::serial_println!(
        "BOUCHAUD_INTERNET_PROOF generation={} stage=tls ok=1 ms={} trusted={} hostname_ok={} expired={} cipher={} kx={} alpn={}",
        generation, ms, sess.report.trusted as u8, sess.report.hostname_ok as u8,
        sess.report.expired as u8, TLS_CIPHER.load(Ordering::Relaxed),
        TLS_KX.load(Ordering::Relaxed), TLS_ALPN.load(Ordering::Relaxed),
    );

    ETAPE.store(ETAPE_HTTP, Ordering::Release);
    let t0 = crate::kernel::timer::monotonic_ms();

    let raw = if sess.alpn == "h2" {
        let mut trace = Vec::new();
        HTTP_SENT.store(true, Ordering::Relaxed);
        crate::net::http2::fetch(&mut sess, HOTE, "/", &mut trace).unwrap_or_default()
    } else {
        let request = b"GET / HTTP/1.1\r\nHost: example.com\r\nUser-Agent: Bouchaud-Internet-Proof/1\r\nAccept: text/html,*/*\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n";
        let sent = sess.send_app(request);
        HTTP_SENT.store(sent, Ordering::Relaxed);
        if sent { sess.recv_http(1_000_000) } else { Vec::new() }
    };

    let ms = crate::kernel::timer::monotonic_ms().saturating_sub(t0);
    HTTP_MS.store(ms, Ordering::Relaxed);
    HTTP_RAW.store(raw.len() as u64, Ordering::Relaxed);
    HTTP_COMPLETE.store(crate::net::http::is_complete(&raw), Ordering::Relaxed);

    let mut status = 0u64;
    let mut body = 0u64;
    let mut html = false;
    let mut parsed = false;
    if let Some(resp) = crate::net::http::parse_response(&raw) {
        status = resp.status_code as u64;
        body = resp.body.len() as u64;
        html = resp.is_html();
        parsed = resp.status_code != 0;
    }
    HTTP_STATUS.store(status, Ordering::Relaxed);
    HTTP_BODY.store(body, Ordering::Relaxed);
    HTTP_HTML.store(html, Ordering::Relaxed);
    HTTP_OK.store(parsed, Ordering::Relaxed);

    if !HTTP_SENT.load(Ordering::Relaxed) { note_echec(ECHEC_HTTP_ENVOI); }
    else if !parsed { note_echec(ECHEC_HTTP_REPONSE); }

    crate::serial_println!(
        "BOUCHAUD_INTERNET_PROOF generation={} stage=http sent={} ok={} ms={} raw={} body={} status={} html={} complete={}",
        generation, HTTP_SENT.load(Ordering::Relaxed) as u8, parsed as u8, ms,
        raw.len(), body, status, html as u8, HTTP_COMPLETE.load(Ordering::Relaxed) as u8,
    );

    sess.close();
    let ok = cert_ok && parsed && HTTP_SENT.load(Ordering::Relaxed);
    termine(ok);
    return if ok { 0 } else { 9 };
}

/// Frontiere de terminaison de tache.
/// `execute_preuve()` revient normalement : tous ses destructeurs sont executes
/// avant que le wrapper abandonne definitivement la pile noyau.
fn travailleur() -> ! {
    let code = execute_preuve();
    crate::kernel::task::exit_current(code);
}

pub fn lance() -> bool {
    if EN_COURS.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
        return false;
    }
    let generation = GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    reset_mesures(generation);

    if !crate::kernel::task::spawn_noyau_priorite(
        travailleur, "internet-proof", crate::kernel::task::Priorite::Normale,
    ) {
        note_echec(ECHEC_SPAWN);
        EN_COURS.store(false, Ordering::Release);
        TERMINEE.store(true, Ordering::Release);
        ETAPE.store(ETAPE_TERMINEE, Ordering::Release);
        FIN_MS.store(crate::kernel::timer::monotonic_ms(), Ordering::Relaxed);
        return false;
    }
    true
}

pub fn releve() -> Releve {
    Releve {
        generation: GENERATION.load(Ordering::Acquire),
        en_cours: EN_COURS.load(Ordering::Acquire),
        terminee: TERMINEE.load(Ordering::Acquire),
        chaine_ok: CHAINE_OK.load(Ordering::Acquire),
        etape: ETAPE.load(Ordering::Acquire),
        premier_echec: PREMIER_ECHEC.load(Ordering::Acquire),
        debut_ms: DEBUT_MS.load(Ordering::Relaxed),
        fin_ms: FIN_MS.load(Ordering::Relaxed),
        lien: LIEN.load(Ordering::Relaxed),
        bail: BAIL.load(Ordering::Relaxed),
        ip: ip_unpack(IP.load(Ordering::Relaxed)),
        passerelle: ip_unpack(PASSERELLE.load(Ordering::Relaxed)),
        dns: ip_unpack(DNS.load(Ordering::Relaxed)),
        arp_ok: ARP_OK.load(Ordering::Relaxed),
        arp_ms: ARP_MS.load(Ordering::Relaxed),
        arp_routees_delta: ARP_ROUTEES_DELTA.load(Ordering::Relaxed),
        arp_vues_delta: ARP_VUES_DELTA.load(Ordering::Relaxed),
        arp_resolues_delta: ARP_RESOLUES_DELTA.load(Ordering::Relaxed),
        arp_echecs_delta: ARP_ECHECS_DELTA.load(Ordering::Relaxed),
        arp_non_emises_delta: ARP_NON_EMISES_DELTA.load(Ordering::Relaxed),
        dns_ok: DNS_OK.load(Ordering::Relaxed),
        dns_ms: DNS_MS.load(Ordering::Relaxed),
        dns_ip: ip_unpack(DNS_IP.load(Ordering::Relaxed)),
        dns_delta: core::array::from_fn(|i| DNS_DELTA[i].load(Ordering::Relaxed)),
        tcp80_ok: TCP80_OK.load(Ordering::Relaxed),
        tcp80_ms: TCP80_MS.load(Ordering::Relaxed),
        tcp80_octets: TCP80_OCTETS.load(Ordering::Relaxed),
        tcp80_status: TCP80_STATUS.load(Ordering::Relaxed),
        tcp443_ok: TCP443_OK.load(Ordering::Relaxed),
        tcp443_ms: TCP443_MS.load(Ordering::Relaxed),
        tcp_poignees_delta: TCP_POIGNEES_DELTA.load(Ordering::Relaxed),
        tcp_syn_retx_delta: TCP_SYN_RETX_DELTA.load(Ordering::Relaxed),
        tls_ok: TLS_OK.load(Ordering::Relaxed),
        tls_ms: TLS_MS.load(Ordering::Relaxed),
        tls_erreur: TLS_ERREUR.load(Ordering::Relaxed),
        tls_trusted: TLS_TRUSTED.load(Ordering::Relaxed),
        tls_hostname_ok: TLS_HOSTNAME_OK.load(Ordering::Relaxed),
        tls_expired: TLS_EXPIRED.load(Ordering::Relaxed),
        tls_cipher: TLS_CIPHER.load(Ordering::Relaxed),
        tls_kx: TLS_KX.load(Ordering::Relaxed),
        tls_alpn: TLS_ALPN.load(Ordering::Relaxed),
        http_sent: HTTP_SENT.load(Ordering::Relaxed),
        http_ok: HTTP_OK.load(Ordering::Relaxed),
        http_ms: HTTP_MS.load(Ordering::Relaxed),
        http_raw: HTTP_RAW.load(Ordering::Relaxed),
        http_body: HTTP_BODY.load(Ordering::Relaxed),
        http_status: HTTP_STATUS.load(Ordering::Relaxed),
        http_html: HTTP_HTML.load(Ordering::Relaxed),
        http_complete: HTTP_COMPLETE.load(Ordering::Relaxed),
        rx_start: RX_START.load(Ordering::Relaxed),
        rx_end: RX_END.load(Ordering::Relaxed),
        tx_start: TX_START.load(Ordering::Relaxed),
        tx_end: TX_END.load(Ordering::Relaxed),
        reset_start: RESET_START.load(Ordering::Relaxed),
        reset_end: RESET_END.load(Ordering::Relaxed),
    }
}

pub fn etape_nom(v: u64) -> &'static str {
    match v {
        ETAPE_IDLE => "idle", ETAPE_CONFIG => "config", ETAPE_ARP => "arp",
        ETAPE_DNS => "dns", ETAPE_TCP80 => "tcp80", ETAPE_TCP443 => "tcp443",
        ETAPE_TLS => "tls", ETAPE_HTTP => "http", ETAPE_TERMINEE => "terminee",
        _ => "inconnue",
    }
}
pub fn echec_nom(v: u64) -> &'static str {
    match v {
        ECHEC_AUCUN => "aucun", ECHEC_LIEN => "lien", ECHEC_IPV4 => "ipv4",
        ECHEC_PASSERELLE => "passerelle", ECHEC_DNS_CONFIG => "dns-config",
        ECHEC_ARP => "arp", ECHEC_DNS => "dns", ECHEC_TCP443 => "tcp443",
        ECHEC_TLS_HANDSHAKE => "tls-handshake", ECHEC_TLS_CERTIFICAT => "tls-certificat",
        ECHEC_HTTP_ENVOI => "http-envoi", ECHEC_HTTP_REPONSE => "http-reponse",
        ECHEC_SPAWN => "spawn", _ => "inconnu",
    }
}
pub fn tls_erreur_nom(v: u64) -> &'static str {
    match v {
        0 => "aucune", 1 => "serverhello-timeout", 2 => "alerte",
        3 => "negociation-groupe-suite", 4 => "record-dechiffrement-flight",
        5 => "certificat", 6 => "finished", 255 => "autre", _ => "inconnue",
    }
}
pub fn cipher_nom(v: u64) -> &'static str {
    match v {
        0x1301 => "TLS_AES_128_GCM_SHA256",
        0x1302 => "TLS_AES_256_GCM_SHA384",
        0x1303 => "TLS_CHACHA20_POLY1305_SHA256",
        0 => "aucune", _ => "inconnue",
    }
}
pub fn kx_nom(v: u64) -> &'static str {
    match v { 1 => "x25519", 2 => "secp256r1", 0 => "aucun", _ => "inconnu" }
}
pub fn alpn_nom(v: u64) -> &'static str {
    match v { 0 => "non-negocie", 1 => "http/1.1", 2 => "h2", _ => "autre" }
}
