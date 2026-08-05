//! Pile reseau de Bouchaud OS, organisee par couches du modele OSI.
//!
//! ```text
//!   link/        L2  liaison       ethernet, arp
//!   internet/    L3  reseau        ipv4, icmp
//!   transport/   L4  transport     tcp, udp
//!   security/    L5/6 session+pres tls (1.3 : handshake, record, crypto, x509)
//!   encoding/    L6  presentation  inflate (deflate/gzip), brotli
//!   application/ L7  application   dns, dhcp, http, http2, hpack, html
//!   stack.rs         moteur de pile (loopback) + ce module : interface,
//!                    routage, fetch HTTP(S), commandes (ping, ifconfig...).
//! ```
//!
//! Etat : loopback `lo` (127.0.0.1) actif ; `eth0` via driver e1000 (ARP/IP/
//! UDP/TCP reels) ; DNS/DHCP, HTTP/1.1+2, TLS 1.3 fonctionnels.

// Couches OSI.
pub mod link;
pub mod internet;
pub mod transport;
pub mod security;
pub mod encoding;
pub mod application;
pub mod stack;

// Re-exports a plat : conserve les chemins `net::<module>` historiques tout en
// rangeant physiquement les fichiers par couche.
pub use link::{ethernet, arp};
pub use internet::{ipv4, icmp};
pub use transport::{tcp, udp};
pub use security::tls;
pub use encoding::{inflate, brotli};
pub use application::{dns, dhcp, http, http2, hpack, html};

use crate::arch::x86_64::pci;
use crate::drivers::e1000;
use crate::drivers::vga::{self, COLOR_CYAN, COLOR_GREEN, COLOR_YELLOW, COLOR_DEFAULT};
use alloc::format;
use alloc::string::String;
use crate::net::ipv4::Ipv4Addr;

/// Adresse de l'interface loopback.
pub const LO_ADDR: Ipv4Addr = [127, 0, 0, 1];

// Configuration eth0 (par defaut statique SLIRP ; DHCP peut la remplacer).
static mut OUR_IP: Ipv4Addr = [10, 0, 2, 15];
static mut GW_IP: Ipv4Addr = [10, 0, 2, 2];
static mut DNS_IP: Ipv4Addr = [10, 0, 2, 3];

/// Adresse IPv4 d'eth0.
pub fn our_ip() -> Ipv4Addr { unsafe { OUR_IP } }
/// Passerelle par defaut.
pub fn gateway() -> Ipv4Addr { unsafe { GW_IP } }
/// Serveur DNS configure.
pub fn dns_server() -> Ipv4Addr { unsafe { DNS_IP } }

/// Applique une configuration reseau (ex. obtenue par DHCP). Invalide le cache ARP.
pub fn set_config(ip: Ipv4Addr, gw: Ipv4Addr, dns: Ipv4Addr) {
    unsafe { OUR_IP = ip; GW_IP = gw; DNS_IP = dns; GW_MAC = None; }
}

/// Indique si une interface routable vers l'exterieur est active.
pub fn external_enabled() -> bool {
    e1000::is_ready()
}

// ---------------------------------------------------------------------------
// eth0 / e1000 : activation et ARP reel
// ---------------------------------------------------------------------------

/// Active l'interface eth0 (initialise le driver e1000).
pub fn ifup() {
    if e1000::init() {
        let m = e1000::mac();
        vga::set_color(COLOR_GREEN);
        println!("eth0 active");
        vga::set_color(COLOR_DEFAULT);
        println!("  MAC : {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", m[0], m[1], m[2], m[3], m[4], m[5]);
        crate::print!("  inet: "); ipv4::print_addr(&our_ip()); println!("  lien={}", if e1000::link_up() { "UP" } else { "DOWN" });
    } else {
        vga::set_color(COLOR_YELLOW);
        println!("ifup: echec d'initialisation e1000 (lance QEMU avec -device e1000 -netdev user,id=n0)");
        vga::set_color(COLOR_DEFAULT);
    }
}

/// Resout l'adresse MAC d'une IP via ARP. Renvoie None en cas de timeout.
fn arp_resolve(target: Ipv4Addr) -> Option<[u8; 6]> {
    let mac = e1000::mac();
    let mut arp_buf = [0u8; arp::PACKET_LEN];
    arp::build(&mut arp_buf, arp::OP_REQUEST, mac, our_ip(), [0; 6], target)?;
    let mut frame = [0u8; ethernet::HEADER_LEN + arp::PACKET_LEN];
    let flen = ethernet::build_frame(&mut frame, ethernet::BROADCAST, mac, ethernet::ETHERTYPE_ARP, &arp_buf)?;
    e1000::send(&frame[..flen]);

    let mut buf = [0u8; 2048];
    for _ in 0..3_000_000u32 {
        if let Some(n) = e1000::receive(&mut buf) {
            if n >= ethernet::HEADER_LEN + arp::PACKET_LEN {
                if let Some(h) = ethernet::parse_header(&buf[..n]) {
                    if h.ethertype == ethernet::ETHERTYPE_ARP {
                        if let Some(p) = arp::parse(&buf[ethernet::HEADER_LEN..n]) {
                            if p.op == arp::OP_REPLY && p.sender_ip == target {
                                return Some(p.sender_mac);
                            }
                            // Le pair nous interroge en meme temps qu'on
                            // l'interroge : c'est le cas normal d'une premiere
                            // prise de contact, et ne pas repondre ici ferait
                            // echouer sa moitie de la resolution.
                            if p.op == arp::OP_REQUEST {
                                repond_arp(&buf[..n]);
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Envoie une requete ARP et attend une reponse (commande `arping <ip>`).
pub fn arping(argc: usize, argv: &[&str; 12]) {
    if argc < 2 { println!("usage: arping <ip>"); return; }
    let target = match ipv4::parse_addr(argv[1]) {
        Some(a) => a,
        None => { println!("arping: adresse invalide"); return; }
    };
    if !e1000::is_ready() && !e1000::init() {
        println!("arping: carte reseau indisponible (essaie 'ifup')");
        return;
    }
    crate::print!("ARP qui a "); ipv4::print_addr(&target); println!(" ?");
    match arp_resolve(target) {
        Some(m) => {
            vga::set_color(COLOR_GREEN);
            crate::print!("reponse de "); ipv4::print_addr(&target);
            println!(" : {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                m[0], m[1], m[2], m[3], m[4], m[5]);
            vga::set_color(COLOR_DEFAULT);
        }
        None => println!("arping: pas de reponse (timeout)"),
    }
}

/// Meme reseau /24 que eth0 ?
fn same_subnet(ip: &Ipv4Addr) -> bool {
    ip[0] == our_ip()[0] && ip[1] == our_ip()[1] && ip[2] == our_ip()[2]
}

static mut IP_ID: u16 = 0x4000;
static mut GW_MAC: Option<[u8; 6]> = None;

fn next_ip_id() -> u16 {
    unsafe { IP_ID = IP_ID.wrapping_add(1); IP_ID }
}

/// MAC du prochain saut pour atteindre `dst` (cache la MAC de la passerelle).
fn hop_mac(dst: &Ipv4Addr) -> Option<[u8; 6]> {
    if same_subnet(dst) {
        return arp_resolve(*dst);
    }
    unsafe {
        if let Some(m) = GW_MAC { return Some(m); }
        let m = arp_resolve(gateway())?;
        GW_MAC = Some(m);
        Some(m)
    }
}

/// Emet un paquet IPv4 (`proto`/`payload`) vers `dst` via e1000.
pub(crate) fn send_ip(dst: Ipv4Addr, proto: u8, payload: &[u8]) -> bool {
    if !e1000::is_ready() && !e1000::init() { return false; }
    let mac = match hop_mac(&dst) { Some(m) => m, None => return false };
    let mut ip = [0u8; 1500];
    let ipl = match ipv4::build_packet(&mut ip, our_ip(), dst, proto, next_ip_id(), payload) {
        Some(n) => n, None => return false,
    };
    let mut frame = [0u8; 1514];
    let fl = match ethernet::build_frame(&mut frame, mac, e1000::mac(), ethernet::ETHERTYPE_IPV4, &ip[..ipl]) {
        Some(n) => n, None => return false,
    };
    e1000::send(&frame[..fl])
}

/// Recoit un paquet IPv4 du protocole `proto` (et source optionnelle). Copie la
/// charge utile dans `out`, renvoie (source, longueur). Non bloquant.
/// Repond a une requete ARP qui nous designe.
///
/// Sans cela, un transfert un peu long s'arrete net au milieu. La passerelle
/// revalide periodiquement son entree ARP ; si personne ne repond, elle
/// l'oublie et cesse de nous router les paquets. Un petit transfert se termine
/// avant l'echeance, un gros non — d'ou un defaut qui n'apparaissait que sur
/// les gros fichiers, et jamais sur une page.
fn repond_arp(trame: &[u8]) -> bool {
    let entete = match ethernet::parse_header(trame) {
        Some(h) => h,
        None => return false,
    };
    if entete.ethertype != ethernet::ETHERTYPE_ARP {
        return false;
    }
    let paquet = match arp::parse(&trame[ethernet::HEADER_LEN..]) {
        Some(p) => p,
        None => return false,
    };
    if paquet.op != arp::OP_REQUEST || paquet.target_ip != our_ip() {
        return true; // c'est de l'ARP, mais pas pour nous : deja consomme
    }

    let mac = e1000::mac();
    let mut reponse = [0u8; arp::PACKET_LEN];
    if arp::build(&mut reponse, arp::OP_REPLY, mac, our_ip(),
                  paquet.sender_mac, paquet.sender_ip).is_none() {
        return true;
    }
    let mut sortie = [0u8; ethernet::HEADER_LEN + arp::PACKET_LEN];
    if let Some(longueur) = ethernet::build_frame(
        &mut sortie, paquet.sender_mac, mac, ethernet::ETHERTYPE_ARP, &reponse) {
        e1000::send(&sortie[..longueur]);
    }
    true
}

/// Nombre de trames examinees par appel.
///
/// Une seule trame par appel obligeait l'appelant a boucler pour trouver ce
/// qu'il attend, et faisait passer chaque paquet non pertinent pour une absence
/// de trafic. En traiter plusieurs vide l'anneau de reception plus vite, ce qui
/// compte quand l'emetteur envoie par rafales.
const TRAMES_PAR_PASSAGE: usize = 32;

pub(crate) fn poll_ip(proto: u8, src_filter: Option<Ipv4Addr>, out: &mut [u8]) -> Option<(Ipv4Addr, usize)> {
    let mut buf = [0u8; 2048];
    for _ in 0..TRAMES_PAR_PASSAGE {
        let n = match e1000::receive(&mut buf) {
            Some(n) => n,
            None => return None, // plus rien a lire
        };
        // L'ARP passe avant tout : c'est du service de lien, il ne doit jamais
        // etre jete au motif qu'on attendait autre chose.
        if repond_arp(&buf[..n]) {
            continue;
        }
        let h = match ethernet::parse_header(&buf[..n]) { Some(h) => h, None => continue };
        if h.ethertype != ethernet::ETHERTYPE_IPV4 { continue; }
        let iph = match ipv4::parse_header(&buf[ethernet::HEADER_LEN..n]) {
            Some(h) => h, None => continue,
        };
        if iph.proto != proto { continue; }
        if let Some(s) = src_filter {
            if iph.src != s { continue; }
        }
        let start = ethernet::HEADER_LEN + iph.header_len;
        let end = ethernet::HEADER_LEN + iph.total_len;
        if start > end || end > n { continue; }
        let len = end - start;
        let m = len.min(out.len());
        out[..m].copy_from_slice(&buf[start..start + m]);
        return Some((iph.src, m));
    }
    None
}

// ── Cache DNS (nom -> IPv4) ───────────────────────────────────────────────────
// Une page moderne charge des dizaines de sous-ressources sur le meme hote ;
// re-resoudre le nom a chaque fois coute un aller-retour UDP (jusqu'a 3 essais
// de 2,5M iterations). On memorise donc les resolutions reussies. Mono-thread
// (boucle GUI), borne en nombre d'entrees (purge FIFO).
static mut DNS_CACHE: Option<alloc::vec::Vec<(String, Ipv4Addr)>> = None;
const DNS_CACHE_MAX: usize = 64;

fn dns_cache_get(name: &str) -> Option<Ipv4Addr> {
    unsafe {
        let slot = &*core::ptr::addr_of!(DNS_CACHE);
        if let Some(c) = slot.as_ref() {
            if let Some((_, ip)) = c.iter().find(|(n, _)| n == name) { return Some(*ip); }
        }
    }
    None
}

fn dns_cache_put(name: &str, ip: Ipv4Addr) {
    use alloc::string::ToString;
    unsafe {
        let slot = &mut *core::ptr::addr_of_mut!(DNS_CACHE);
        let c = slot.get_or_insert_with(alloc::vec::Vec::new);
        if c.iter().any(|(n, _)| n == name) { return; }
        if c.len() >= DNS_CACHE_MAX { c.remove(0); }
        c.push((name.to_string(), ip));
    }
}

/// Resout un nom d'hote en IPv4 via DNS (None en cas d'echec/timeout).
pub fn resolve(name: &str) -> Option<Ipv4Addr> {
    // Deja une IP ?
    if let Some(ip) = ipv4::parse_addr(name) { return Some(ip); }
    // Cache : evite un aller-retour DNS par sous-ressource du meme hote.
    if let Some(ip) = dns_cache_get(name) { return Some(ip); }
    if !e1000::is_ready() && !e1000::init() { return None; }

    let mut payload = [0u8; 1500];
    // Plusieurs essais : la 1re requete echoue souvent (ARP passerelle a chaud,
    // reponse DNS manquee/perdue au demarrage). Chaque essai a un ID frais.
    for _attempt in 0..3u32 {
        let id = (next_ip_id() ^ 0x1234) as u16;
        let mut q = [0u8; 256];
        let qlen = dns::build_query(&mut q, id, name)?;
        let mut udp_buf = [0u8; 300];
        let ulen = udp::build(&mut udp_buf, 0xC000, 53, &q[..qlen])?;
        send_ip(dns_server(), ipv4::PROTO_UDP, &udp_buf[..ulen]);

        for _ in 0..2_500_000u32 {
            if let Some((_src, n)) = poll_ip(ipv4::PROTO_UDP, Some(dns_server()), &mut payload) {
                if let Some(u) = udp::parse(&payload[..n]) {
                    if u.dst_port == 0xC000 {
                        let off = u.payload_off;
                        if let Some(ip) = dns::parse_response(&payload[off..off + u.payload_len], id) {
                            dns_cache_put(name, ip);
                            return Some(ip);
                        }
                    }
                }
            }
        }
    }
    None
}

/// Commande `dns <nom>` / `nslookup`.
pub fn dns_cmd(argc: usize, argv: &[&str; 12]) {
    if argc < 2 { println!("usage: dns <nom>"); return; }
    if !e1000::is_ready() && !e1000::init() {
        println!("dns: carte reseau indisponible (essaie 'ifup')");
        return;
    }
    match resolve(argv[1]) {
        Some(ip) => { crate::print!("{} -> ", argv[1]); ipv4::print_addr(&ip); println!(""); }
        None => println!("dns: pas de reponse pour {}", argv[1]),
    }
}

/// Document brut recupere par le navigateur graphique.
pub struct Document {
    pub banner: alloc::vec::Vec<String>, // lignes de diagnostic (TLS, statut, erreurs)
    pub final_url: String,               // URL apres redirections
    pub content_type: String,
    pub body: alloc::vec::Vec<u8>,       // corps decode (dechunke + decompresse)
    pub is_html: bool,
    pub ok: bool,
}

/// Recupere une URL HTTP(S) et renvoie le document brut (corps decode), en
/// suivant les redirections. Utilise par le moteur de rendu graphique.
pub fn fetch_document(url: &str) -> Document {
    use alloc::string::ToString;
    use crate::diag::Cat;
    let t0 = crate::kernel::timer::cycles_since_boot();
    let mc = || crate::kernel::timer::cycles_since_boot().wrapping_sub(t0) / 1_000_000;
    // Temps reel (ms) calibre sur le PIT, independant de la vitesse d'emulation
    // QEMU — voir kernel::timer::calibrate(). Complementaire au Mc (comparaison
    // relative entre requetes).
    let ms = || crate::kernel::timer::cycles_to_ms(crate::kernel::timer::cycles_since_boot().wrapping_sub(t0));
    let mut banner: alloc::vec::Vec<String> = alloc::vec::Vec::new();

    if !e1000::is_ready() && !e1000::init() {
        crate::dlog!(Cat::Err, "reseau indisponible (ifup) pour {}", url);
        banner.push("reseau indisponible (lance 'ifup')".to_string());
        return Document { banner, final_url: url.to_string(), content_type: String::new(), body: alloc::vec::Vec::new(), is_html: false, ok: false };
    }

    let mut current = String::from(url);
    for hop in 0..8u32 {
        let (scheme, hostname, port, path) = split_url(&current);
        let (mut b, raw) = if scheme == "https" {
            let r = tls::https_fetch(&hostname, port, &path);
            (r.banner, r.raw)
        } else {
            let ip = match resolve(&hostname) {
                Some(ip) => ip,
                None => { crate::dlog!(Cat::Err, "DNS echec {} ({}Mc, {}ms)", hostname, mc(), ms()); banner.push(format!("DNS: echec pour {}", hostname)); return Document { banner, final_url: current, content_type: String::new(), body: alloc::vec::Vec::new(), is_html: false, ok: false }; }
            };
            let req = http::build_get(&hostname, &path);
            let mut resp: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
            // HTTP en clair : on passe par la VRAIE pile TCP (smoltcp, RFC 793 :
            // controle de congestion, fast-retransmit, fenetre glissante). La
            // pile maison (tcp::fetch) souffrait de pertes non recuperees sur
            // les gros transferts (retransmissions au RTO -> ~250 s pour 44 Ko) ;
            // smoltcp fait le meme transfert en < 1 s. Repli sur la pile maison
            // si smoltcp echoue (robustesse).
            if !transport::smol_tcp::fetch(ip, port, req.as_bytes(), &mut resp) {
                resp.clear();
                if !tcp::fetch(ip, port, req.as_bytes(), &mut resp) {
                    crate::dlog!(Cat::Err, "TCP echec {}:{} ({}Mc, {}ms)", hostname, port, mc(), ms());
                    banner.push(format!("connexion TCP echouee vers {}:{}", hostname, port));
                    return Document { banner, final_url: current, content_type: String::new(), body: alloc::vec::Vec::new(), is_html: false, ok: false };
                }
            }
            (alloc::vec::Vec::new(), resp)
        };
        // Diagnostic TLS / handshake : remonte les lignes du sous-systeme.
        for line in &b {
            if line.contains("TLS") || line.contains("handshake") || line.contains("echoue") {
                crate::dlog!(Cat::Net, "{}", line);
            }
        }
        banner.append(&mut b);
        if raw.is_empty() {
            crate::dlog!(Cat::Err, "{} {} : reponse vide ({}Mc, {}ms)", scheme, hostname, mc(), ms());
            return Document { banner, final_url: current, content_type: String::new(), body: alloc::vec::Vec::new(), is_html: false, ok: false };
        }
        // Cookies : memorise les Set-Cookie de la reponse (y compris ceux des
        // redirections -- c'est la que Google pose CONSENT/NID).
        application::cookies::store_from_raw(&hostname, &raw);
        match http::parse_response(&raw) {
            Some(r) if r.is_redirect() && hop < 7 => {
                let loc = r.location.clone().unwrap_or_default();
                crate::dlog!(Cat::Info, "redir {} {} -> {}", r.status_code, hostname, loc);
                banner.push(format!("{} -> {}", r.status_code, loc));
                current = http::resolve_location(scheme, &hostname, &loc);
            }
            Some(r) => {
                let is_html = r.is_html();
                let ct = r.content_type.clone().unwrap_or_default();
                crate::dlog!(Cat::Net, "{} {} {} {}o {} {}Mc ({}ms)", scheme, r.status_code, hostname,
                    r.body.len(), if is_html { "html" } else { ct.as_str() }, mc(), ms());
                banner.push(r.status_line.clone());
                return Document { banner, final_url: current, content_type: ct, body: r.body, is_html, ok: true };
            }
            None => {
                let mut status = String::new();
                for &c in raw.iter().take_while(|&&c| c != b'\r' && c != b'\n') { status.push(c as char); }
                crate::dlog!(Cat::Warn, "reponse HTTP illisible {} : {}", hostname, status);
                banner.push(status);
                return Document { banner, final_url: current, content_type: String::new(), body: alloc::vec::Vec::new(), is_html: false, ok: false };
            }
        }
    }
    crate::dlog!(Cat::Warn, "trop de redirections pour {}", url);
    banner.push("trop de redirections".to_string());
    Document { banner, final_url: current, content_type: String::new(), body: alloc::vec::Vec::new(), is_html: false, ok: false }
}

// ── Cache de ressources (sous-ressources : CSS, JS, images) ───────────────────
// Cache global URL -> octets, partage entre toutes les pages et onglets, pour
// eviter de re-telecharger les feuilles de style / scripts / images. Borne en
// nombre d'entrees et en memoire (purge FIFO). Acces mono-thread (boucle GUI).

static mut RES_CACHE: Option<alloc::vec::Vec<(String, alloc::vec::Vec<u8>)>> = None;
const RES_CACHE_MAX_ENTRIES: usize = 128;
const RES_CACHE_MAX_BYTES: usize = 12_000_000;

// ── Budget temps des sous-ressources ───────────────────────────────────────
// Une page peut referencer des dizaines d'hotes tiers (pub, analytics, CDN de
// polices...) : chacun d'eux, s'il est injoignable ou lent, coute jusqu'a 3s
// (TLS, cf. TcpConn::fill) voire 30s (HTTP simple, cf. tcp::fetch) d'attente
// AVANT d'echouer. Sans plafond global, un budget d'images de 96 (cf. web.rs)
// peut donc a lui seul faire gonfler un chargement de quelques centaines de
// ms a plusieurs MINUTES, sans qu'aucune phase individuelle (DOM/CSS/LAYOUT)
// ne le laisse voir : c'est l'origine du "195004ms" observe en pratique alors
// que la somme des phases loggees ne faisait que ~3s. On borne donc le temps
// TOTAL consacre aux sous-ressources d'une page (CSS externe, JS, images) :
// une fois le budget epuise, les fetch_cached() suivants echouent tout de
// suite au lieu de retenter une connexion complete.
static mut SUBRES_DEADLINE_TSC: Option<u64> = None;
static mut SUBRES_LOGGED: bool = false;

/// (Re)demarre le budget temps des sous-ressources pour une nouvelle page.
/// A appeler une fois au debut du pipeline HTML->layout (voir web.rs).
pub fn start_page_budget(ms: u64) {
    let cycles = crate::kernel::timer::ms_to_cycles(ms);
    unsafe {
        SUBRES_DEADLINE_TSC = if cycles == 0 { None } else {
            Some(crate::kernel::timer::cycles_since_boot().wrapping_add(cycles))
        };
        SUBRES_LOGGED = false;
    }
}

fn subres_budget_exhausted() -> bool {
    unsafe {
        match SUBRES_DEADLINE_TSC {
            Some(deadline) if crate::kernel::timer::cycles_since_boot() >= deadline => {
                if !SUBRES_LOGGED {
                    SUBRES_LOGGED = true;
                    crate::dlog!(crate::diag::Cat::Warn, "budget reseau sous-ressources epuise : fetch restants ignores");
                }
                true
            }
            _ => false,
        }
    }
}

/// Recupere une sous-ressource (CSS/JS/image) avec mise en cache. Renvoie les
/// octets du corps si la requete reussit (ou un hit cache), sinon None.
pub fn fetch_cached(url: &str) -> Option<alloc::vec::Vec<u8>> {
    use alloc::string::ToString;
    unsafe {
        let slot = &mut *core::ptr::addr_of_mut!(RES_CACHE);
        if slot.is_none() { *slot = Some(alloc::vec::Vec::new()); }
        if let Some(c) = slot.as_ref() {
            if let Some((_, b)) = c.iter().find(|(u, _)| u == url) {
                crate::dlog!(crate::diag::Cat::Cache, "HIT {}o {}", b.len(), url);
                return Some(b.clone());
            }
        }
    }
    if subres_budget_exhausted() { return None; }
    let doc = fetch_document(url);
    if !doc.ok || doc.body.is_empty() { return None; }
    let body = doc.body;
    unsafe {
        let slot = &mut *core::ptr::addr_of_mut!(RES_CACHE);
        let c = slot.get_or_insert_with(alloc::vec::Vec::new);
        let mut total: usize = c.iter().map(|(_, b)| b.len()).sum();
        // Purge FIFO tant que l'ajout depasse les bornes.
        while (!c.is_empty()) && (c.len() >= RES_CACHE_MAX_ENTRIES || total + body.len() > RES_CACHE_MAX_BYTES) {
            let removed = c.remove(0);
            total = total.saturating_sub(removed.1.len());
        }
        if body.len() <= RES_CACHE_MAX_BYTES {
            c.push((url.to_string(), body.clone()));
        }
    }
    Some(body)
}

/// Vide le cache de ressources (ex. rechargement force).
pub fn clear_cache() {
    unsafe {
        let slot = &mut *core::ptr::addr_of_mut!(RES_CACHE);
        if let Some(c) = slot.as_mut() { c.clear(); }
    }
    // Abandonne aussi la session HTTPS keep-alive en attente.
    tls::pool_clear();
}

/// Recupere une URL HTTP(S) et renvoie les lignes a afficher (statut + corps),
/// en suivant jusqu'a 5 redirections (301/302/303/307/308 via `Location`).
/// Utilise par la commande `wget` et par le navigateur.
pub fn http_get(url: &str) -> alloc::vec::Vec<String> {
    use alloc::string::ToString;
    let mut out: alloc::vec::Vec<String> = alloc::vec::Vec::new();

    if !e1000::is_ready() && !e1000::init() {
        out.push("reseau indisponible (lance 'ifup')".to_string());
        return out;
    }

    let mut current = String::from(url);
    for hop in 0..6u32 {
        let (scheme, hostname, port, path) = split_url(&current);

        // Recupere la reponse brute (banniere TLS eventuelle + octets HTTP).
        let (mut banner, raw) = if scheme == "https" {
            let r = tls::https_fetch(&hostname, port, &path);
            (r.banner, r.raw)
        } else {
            let ip = match resolve(&hostname) {
                Some(ip) => ip,
                None => { out.push(format!("DNS: echec pour {}", hostname)); return out; }
            };
            let req = http::build_get(&hostname, &path);
            let mut resp: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
            if !tcp::fetch(ip, port, req.as_bytes(), &mut resp) {
                out.push(format!("connexion TCP echouee vers {}:{}", hostname, port));
                return out;
            }
            (alloc::vec::Vec::new(), resp)
        };

        out.append(&mut banner);
        if !raw.is_empty() {
            application::cookies::store_from_raw(&hostname, &raw);
        }
        if raw.is_empty() {
            // La banniere contient deja la trace de diagnostic (canal muet).
            if scheme != "https" { out.push("reponse vide".to_string()); }
            return out;
        }

        match http::parse_response(&raw) {
            Some(r) if r.is_redirect() && hop < 5 => {
                let loc = r.location.clone().unwrap_or_default();
                out.push(format!("{} -> {}", r.status_code, loc));
                current = http::resolve_location(scheme, &hostname, &loc);
            }
            Some(r) => {
                let is_html = r.is_html();
                out.push(r.status_line);
                if is_html {
                    // Rendu type navigateur texte : titre, contenu sans balises,
                    // entites decodees, et liste numerotee des liens.
                    let page = html::render(&r.body, &current);
                    if !page.title.is_empty() { out.push(format!("== {} ==", page.title)); }
                    for l in page.lines {
                        out.push(l);
                        if out.len() > 200 { break; }
                    }
                    if !page.links.is_empty() {
                        out.push(String::new());
                        out.push(format!("--- {} liens ---", page.links.len()));
                        for (n, link) in page.links.iter().enumerate() {
                            out.push(format!("[{}] {}", n + 1, link));
                            if out.len() > 260 { break; }
                        }
                    }
                } else {
                    append_body_lines(&mut out, &r.body);
                }
                return out;
            }
            None => {
                let mut status = String::new();
                for &b in raw.iter().take_while(|&&b| b != b'\r' && b != b'\n') { status.push(b as char); }
                out.push(status);
                return out;
            }
        }
    }
    out.push("trop de redirections".to_string());
    out
}

// Decoupe une URL en (scheme, hostname, port, path).
fn split_url(url: &str) -> (&'static str, String, u16, String) {
    use alloc::string::ToString;
    let (rest, scheme, default_port) = if let Some(r) = url.strip_prefix("https://") {
        (r, "https", 443u16)
    } else if let Some(r) = url.strip_prefix("http://") {
        (r, "http", 80u16)
    } else {
        (url, "http", 80u16)
    };
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (hostname, port) = match host.find(':') {
        Some(i) => (&host[..i], host[i + 1..].parse::<u16>().unwrap_or(default_port)),
        None => (host, default_port),
    };
    (scheme, hostname.to_string(), port, path.to_string())
}

// Ajoute un corps de reponse a `out`, ligne par ligne (non imprimables -> '.').
fn append_body_lines(out: &mut alloc::vec::Vec<String>, body: &[u8]) {
    let mut line = String::new();
    for &b in body {
        match b {
            b'\n' => { out.push(core::mem::take(&mut line)); if out.len() > 200 { break; } }
            b'\r' => {}
            0x20..=0x7e => line.push(b as char),
            _ => line.push('.'),
        }
    }
    if !line.is_empty() { out.push(line); }
}

/// Commande `wget`/`curl`/`http`/`https <url>`.
pub fn wget_cmd(argc: usize, argv: &[&str; 12]) {
    if argc < 2 {
        println!("usage: {} <url>", argv[0]);
        return;
    }
    // La commande `https` force le schema TLS si absent.
    let url = argv[1];
    let prefixed: alloc::string::String;
    let target = if argv[0] == "https" && !url.contains("://") {
        prefixed = alloc::format!("https://{}", url);
        prefixed.as_str()
    } else {
        url
    };
    for l in http_get(target) {
        println!("{}", l);
    }
}

/// Commande `smoltest <hote> [port] [chemin]` : test manuel du backend TCP
/// experimental `transport::smol_tcp` (vraie pile RFC 793 via la crate
/// `smoltcp`, cf. commentaire en tete de ce module). Non branche par defaut
/// (fetch_document continue d'utiliser tcp.rs/TcpConn) : sert a verifier a
/// L'EXECUTION -- ce module n'a ete verifie qu'a la COMPILATION -- avant
/// d'envisager de le faire remplacer la pile maison en production.
pub fn smoltest_cmd(argc: usize, argv: &[&str; 12]) {
    use alloc::string::String;
    if argc < 2 {
        println!("usage: smoltest <hote> [port] [chemin]");
        return;
    }
    let host = argv[1];
    let port: u16 = if argc >= 3 { argv[2].parse().unwrap_or(80) } else { 80 };
    let path = if argc >= 4 { argv[3] } else { "/" };
    let ip = match resolve(host) {
        Some(ip) => ip,
        None => { println!("DNS: echec pour {}", host); return; }
    };
    println!("smoltcp: connexion a {}:{} ({}.{}.{}.{})...", host, port, ip[0], ip[1], ip[2], ip[3]);
    let req = http::build_get(host, path);
    let t0 = crate::kernel::timer::cycles_since_boot();
    let mut out: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
    let ok = transport::smol_tcp::fetch(ip, port, req.as_bytes(), &mut out);
    let ms = crate::kernel::timer::cycles_to_ms(crate::kernel::timer::cycles_since_boot().wrapping_sub(t0));
    if !ok {
        println!("smoltcp: echec ({}ms)", ms);
        return;
    }
    println!("smoltcp: {} octets recus en {}ms", out.len(), ms);
    let preview = String::from_utf8_lossy(&out[..out.len().min(300)]);
    for line in preview.lines().take(8) { println!("  {}", line); }
}

/// Commande `tls [hote]` : diagnostics TLS et magasin de CA racines.
pub fn tls_cmd(argc: usize, argv: &[&str; 12]) {
    vga::set_color(COLOR_CYAN);
    println!("TLS : {}", tls::status());
    vga::set_color(COLOR_DEFAULT);
    println!("  magasin de CA racines : {} ancres de confiance", tls::roots::count());
    println!("  suite : TLS_AES_128_GCM_SHA256, groupe x25519");
    println!("  signatures : RSA PKCS#1v1.5 / RSA-PSS / ECDSA P-256/SHA-256 + P-384/SHA-384");
    if argc >= 2 {
        println!("");
        println!("test handshake https://{}/ ...", argv[1]);
        for l in tls::https_get(argv[1], 443, "/") {
            println!("{}", l);
        }
    } else {
        println!("  (astuce : 'tls example.com' teste un vrai handshake)");
        println!("  (astuce : 'tls-selftest' valide la crypto par vecteurs de reference)");
    }
}

/// Ping reel via e1000 : ARP -> ICMP echo sur 4 paquets.
fn ping_remote(target: Ipv4Addr) {
    // Adresse de niveau lien : la cible si locale, sinon la passerelle.
    let next_hop = if same_subnet(&target) { target } else { gateway() };
    let dst_mac = match arp_resolve(next_hop) {
        Some(m) => m,
        None => {
            vga::set_color(COLOR_YELLOW);
            crate::print!("ping: ARP sans reponse pour "); ipv4::print_addr(&next_hop); println!("");
            vga::set_color(COLOR_DEFAULT);
            return;
        }
    };
    let our_mac = e1000::mac();
    let payload = b"bouchaud-os-ping";
    let id = 0x4243u16;
    let mut sent = 0u32;
    let mut recv = 0u32;

    for seq in 0..4u16 {
        let mut icmp_buf = [0u8; 64];
        let il = match icmp::build(&mut icmp_buf, icmp::ECHO_REQUEST, id, seq, payload) {
            Some(n) => n, None => continue,
        };
        let mut ip_buf = [0u8; 128];
        let ipl = match ipv4::build_packet(&mut ip_buf, our_ip(), target, ipv4::PROTO_ICMP, seq, &icmp_buf[..il]) {
            Some(n) => n, None => continue,
        };
        let mut frame = [0u8; ethernet::HEADER_LEN + 128];
        let fl = match ethernet::build_frame(&mut frame, dst_mac, our_mac, ethernet::ETHERTYPE_IPV4, &ip_buf[..ipl]) {
            Some(n) => n, None => continue,
        };
        e1000::send(&frame[..fl]);
        sent += 1;

        // Attend l'echo reply correspondant.
        let mut buf = [0u8; 2048];
        let mut got = false;
        for _ in 0..3_000_000u32 {
            if let Some(n) = e1000::receive(&mut buf) {
                if let Some(h) = ethernet::parse_header(&buf[..n]) {
                    if h.ethertype == ethernet::ETHERTYPE_IPV4 {
                        if let Some(iph) = ipv4::parse_header(&buf[ethernet::HEADER_LEN..n]) {
                            if iph.proto == ipv4::PROTO_ICMP && iph.src == target {
                                let off = ethernet::HEADER_LEN + iph.header_len;
                                if off < n {
                                    if let Some(m) = icmp::parse(&buf[off..n]) {
                                        if m.msg_type == icmp::ECHO_REPLY && m.id == id && m.seq == seq {
                                            recv += 1;
                                            crate::print!("reponse de "); ipv4::print_addr(&target);
                                            println!(" : icmp_seq={} ttl=64", seq);
                                            got = true;
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if !got {
            println!("  delai depasse pour icmp_seq={}", seq);
        }
    }
    let lost = if sent > 0 { (sent - recv) * 100 / sent } else { 100 };
    println!("--- statistiques ping ---");
    println!("{} transmis, {} recus, {}% perdus", sent, recv, lost);
}

// ---------------------------------------------------------------------------
// ping
// ---------------------------------------------------------------------------

pub fn ping(argc: usize, argv: &[&str; 12]) {
    if argc < 2 {
        println!("usage: ping <ip>");
        return;
    }
    let target = match ipv4::parse_addr(argv[1]) {
        Some(a) => a,
        None => { println!("ping: adresse invalide (attendu a.b.c.d)"); return; }
    };

    crate::print!("PING ");
    ipv4::print_addr(&target);
    println!(" : 16 octets de donnees");

    if ipv4::is_loopback(&target) {
        ping_loopback(target);
    } else if e1000::is_ready() || e1000::init() {
        // Ping reel via la carte e1000.
        ping_remote(target);
    } else {
        vga::set_color(COLOR_YELLOW);
        println!("ping: carte reseau indisponible");
        vga::set_color(COLOR_DEFAULT);
        match pci::find_network() {
            Some(d) => println!("  eth0 {:04x}:{:04x} detectee ; lance 'ifup' (QEMU -device e1000 -netdev user,id=n0)", d.vendor, d.device),
            None => println!("  aucune carte reseau PCI; lance QEMU avec -device e1000 -netdev user,id=n0"),
        }
    }
}

/// Envoie 4 echo requests sur loopback en passant par la vraie pile ICMP.
fn ping_loopback(target: Ipv4Addr) {
    let payload = b"bouchaud-os-ping";
    let mut sent = 0u32;
    let mut recv = 0u32;

    for seq in 0..4u16 {
        let mut icmp_buf = [0u8; 64];
        let il = match icmp::build(&mut icmp_buf, icmp::ECHO_REQUEST, 0x4243, seq, payload) {
            Some(n) => n,
            None => continue,
        };
        let mut pkt = [0u8; 128];
        let pl = match ipv4::build_packet(&mut pkt, LO_ADDR, target, ipv4::PROTO_ICMP, seq, &icmp_buf[..il]) {
            Some(n) => n,
            None => continue,
        };
        sent += 1;

        // Le paquet "boucle" : il est traite par notre propre moteur de pile.
        let mut out = [0u8; 128];
        if let Some(rl) = stack::handle_ipv4(&pkt[..pl], &mut out) {
            if let Some(h) = ipv4::parse_header(&out[..rl]) {
                let reply = &out[h.header_len..h.total_len];
                if let Some(m) = icmp::parse(reply) {
                    if m.msg_type == icmp::ECHO_REPLY && m.seq == seq {
                        recv += 1;
                        crate::print!("{} octets de ", h.total_len);
                        ipv4::print_addr(&h.src);
                        println!(": icmp_seq={} ttl=64 temps<1ms (loopback)", seq);
                    }
                }
            }
        }
    }

    let lost = if sent > 0 { (sent - recv) * 100 / sent } else { 100 };
    println!("--- statistiques ping ---");
    println!("{} paquets transmis, {} recus, {}% perdus", sent, recv, lost);
}

// ---------------------------------------------------------------------------
// ifconfig / ip / route / arp
// ---------------------------------------------------------------------------

fn print_eth0_state() {
    if e1000::is_ready() {
        let m = e1000::mac();
        crate::print!("eth0: flags=<UP,RUNNING>  inet ");
        ipv4::print_addr(&our_ip());
        println!("  lien={}", if e1000::link_up() { "UP" } else { "DOWN" });
        println!("      ether {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            m[0], m[1], m[2], m[3], m[4], m[5]);
        return;
    }
    match pci::find_network() {
        Some(d) => {
            crate::print!("eth0: flags=<DOWN>  carte PCI {:04x}:{:04x} (", d.vendor, d.device);
            crate::print!("{}", pci::vendor_name(d.vendor));
            println!(") - driver non charge (lance 'ifup')");
        }
        None => println!("eth0: absente (aucune carte reseau PCI detectee)"),
    }
}

pub fn ifconfig() {
    crate::print!("lo: flags=<UP,LOOPBACK,RUNNING>  inet ");
    ipv4::print_addr(&LO_ADDR);
    println!("  netmask 255.0.0.0");
    print_eth0_state();
}

pub fn ip_cmd() {
    println!("1: lo: <UP,LOOPBACK>");
    crate::print!("   inet ");
    ipv4::print_addr(&LO_ADDR);
    println!("/8 scope host lo");
    println!("2: eth0:");
    print_eth0_state();
}

pub fn route_cmd() {
    println!("Table de routage IPv4:");
    println!("  Destination     Masque          Interface");
    println!("  127.0.0.0       255.0.0.0       lo");
    println!("  (pas de route par defaut: eth0 DOWN, driver NIC non charge)");
}

pub fn arp_cmd() {
    println!("Cache ARP:");
    println!("  Adresse         HWaddr             Iface");
    println!("  (vide: aucune trame Ethernet emise tant que le driver NIC manque)");
}

// ---------------------------------------------------------------------------
// Roadmap + placeholders (couches non encore actives)
// ---------------------------------------------------------------------------

/// Affiche la feuille de route OSI (commande `roadmap`, section reseau).
pub fn print_roadmap() {
    vga::set_color(COLOR_CYAN);
    println!("pile reseau OSI:");
    vga::set_color(COLOR_DEFAULT);
    println!("  L2 Ethernet    encode/decode                   [code OK]");
    println!("  ARP            encode/decode                   [code OK]");
    println!("  L3 IPv4        en-tete + checksum              [code OK]");
    println!("  ICMP           echo (ping loopback)            [actif sur lo]");
    println!("  interface lo   127.0.0.1                       [active]");
    println!("  driver NIC     e1000/virtio-net (RX/TX DMA)    [a ecrire]");
    println!("  UDP/DHCP/DNS                                   [actif]");
    println!("  TCP/HTTP                                       [actif]");
    println!("  TLS 1.3  X25519+AES-GCM+SHA256+HKDF            [actif]");
    println!("  X.509    ASN.1/DER + chaine RSA/ECDSA          [actif]");
}

fn missing_layer(cmd: &str) -> &'static str {
    match cmd {
        "dhcp" => "driver NIC + UDP + client DHCP",
        "dns" => "driver NIC + UDP + resolveur DNS",
        "wget" | "curl" => "driver NIC + TCP + HTTP",
        _ => "driver NIC + couches superieures",
    }
}

/// Message standard d'une commande reseau pas encore active.
pub fn placeholder(cmd: &str) {
    vga::set_color(COLOR_YELLOW);
    println!("{}: non disponible (couches superieures non actives)", cmd);
    vga::set_color(COLOR_DEFAULT);
    println!("  couche manquante: {}", missing_layer(cmd));
    match pci::find_network() {
        Some(d) => println!("  note: carte reseau {:04x}:{:04x} detectee, driver a ecrire", d.vendor, d.device),
        None => println!("  note: aucune carte reseau PCI (essaie QEMU avec -device e1000)"),
    }
}
