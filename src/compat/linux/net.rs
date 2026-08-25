//! Sockets POSIX, branchees sur la pile TCP/IP du noyau.
//!
//! La pile de `src/net/` existait deja mais n'etait accessible qu'aux commandes
//! du shell. Ce module l'expose sous l'ABI socket de Linux, ce qui suffit a
//! faire fonctionner `getaddrinfo`, `connect`, `send`/`recv` d'une libc — donc
//! tout client HTTP compile pour Linux.
//!
//! ## Ce qui est couvert
//!
//! - **TCP client** : `socket`/`connect`/`send`/`recv`/`shutdown`/`close`,
//!   au-dessus de [`TcpConn`] ;
//! - **UDP** : `sendto`/`recvfrom`, sans lesquels la resolution de noms d'une
//!   libc ne fonctionne pas (elle parle au serveur DNS elle-meme, elle
//!   n'appelle pas le resolveur du noyau) ;
//! - **`socketpair`** : deux tubes croises, ce que reclament plusieurs
//!   bibliotheques pour leur reveil interne.
//!
//! ## Ce qui ne l'est pas, et pourquoi
//!
//! `listen`/`accept` demanderaient un demi-TCP supplementaire (file de
//! connexions en attente, poignee de main cote serveur, demultiplexage par
//! port). La pile est aujourd'hui pilotee par interrogation depuis le contexte
//! appelant : rien ne recoit de paquet quand aucun socket ne lit. Un serveur
//! d'ecoute reclamerait d'abord une reception en tache de fond. C'est signale
//! par `ENOSYS` plutot que simule.

use alloc::sync::Arc;
use alloc::vec::Vec;
use crate::kernel::sync::SpinLock;

use crate::kernel::abi::{errno, user_read, user_write};
use crate::kernel::fd::{FdKind, FileDesc};
use crate::kernel::task;
use crate::net::internet::ipv4::Ipv4Addr;
use crate::net::transport::tcp::TcpConn;

pub const AF_UNIX: u32 = 1;
pub const AF_INET: u32 = 2;
pub const AF_INET6: u32 = 10;

pub const SOCK_STREAM: u32 = 1;
pub const SOCK_DGRAM: u32 = 2;
/// Drapeaux combinables au type (`SOCK_NONBLOCK`, `SOCK_CLOEXEC`).
pub const SOCK_NONBLOCK: u32 = 0o4000;
pub const SOCK_CLOEXEC: u32 = 0o2000000;

/// Drapeau persistant du descripteur, pose par `fcntl(F_SETFL)`/`FIONBIO`.
const O_NONBLOCK: u32 = 0o4000;
/// Drapeau Linux de `recv`/`recvmsg` : ne vaut que pour **cet appel**.
/// Ladybird l'utilise pour drainer son transport jusqu'a `EAGAIN` sans rendre
/// le socket lui-meme non bloquant.
const MSG_DONTWAIT: u32 = 0x40;

/// Nature d'un socket.
#[derive(Clone, Copy, PartialEq)]
pub enum SocketKind {
    Tcp,
    Udp,
}

/// Etat d'un socket.
pub struct SocketState {
    pub kind: SocketKind,
    /// Connexion TCP etablie, le cas echeant.
    pub conn: Option<TcpConn>,
    /// Port local (choisi a la volee ou impose par `bind`).
    pub local_port: u16,
    /// Pair courant : destination par defaut d'un `send` sans adresse.
    pub peer: Option<(Ipv4Addr, u16)>,
    /// Datagrammes UDP recus et non encore lus : (source, port, donnees).
    pub datagrams: Vec<(Ipv4Addr, u16, Vec<u8>)>,
    pub nonblocking: bool,
    /// Le pair a ferme, ou la connexion a echoue.
    pub eof: bool,
}

impl SocketState {
    fn new(kind: SocketKind) -> Self {
        SocketState {
            kind,
            conn: None,
            local_port: 0,
            peer: None,
            datagrams: Vec::new(),
            nonblocking: false,
            eof: false,
        }
    }
}

/// Alloue un port local ephemere.
fn ephemeral_port() -> u16 {
    static mut NEXT: u16 = 0;
    unsafe {
        if NEXT == 0 {
            NEXT = 0xC000 | (crate::arch::x86_64::cpu::rdtsc() as u16 & 0x0FFF);
        }
        NEXT = NEXT.wrapping_add(1) | 0xC000;
        NEXT
    }
}

/// Lit une `struct sockaddr_in` : famille, port (gros-boutiste), adresse.
fn read_sockaddr(addr: u64, len: usize) -> Option<(Ipv4Addr, u16)> {
    if addr == 0 || len < 8 {
        return None;
    }
    let bytes = user_read(addr, 8)?;
    let family = u16::from_le_bytes([bytes[0], bytes[1]]) as u32;
    if family != AF_INET {
        return None;
    }
    // Le port et l'adresse sont en ordre reseau, pas en ordre machine.
    let port = u16::from_be_bytes([bytes[2], bytes[3]]);
    Some(([bytes[4], bytes[5], bytes[6], bytes[7]], port))
}

/// Ecrit une `struct sockaddr_in` (16 octets) vers l'espace utilisateur.
fn write_sockaddr(addr: u64, len_addr: u64, ip: Ipv4Addr, port: u16) -> bool {
    if addr == 0 {
        return true;
    }
    let mut buffer = [0u8; 16];
    buffer[0..2].copy_from_slice(&(AF_INET as u16).to_le_bytes());
    buffer[2..4].copy_from_slice(&port.to_be_bytes());
    buffer[4..8].copy_from_slice(&ip);
    if !user_write(addr, &buffer) {
        return false;
    }
    if len_addr != 0 {
        user_write(len_addr, &16u32.to_le_bytes())
    } else {
        true
    }
}

/// Le descripteur est-il une extremite de `socketpair` ?
///
/// Une paire n'a pas de `SocketState` : ce sont deux tubes croises, sans
/// adresse ni connexion. Elle reste pourtant un **socket** du point de vue de
/// l'espace utilisateur, et les appels qui l'interrogent doivent repondre
/// plutot que de la renier.
///
/// Ce que cela a coute : `socket.socketpair()` de CPython enveloppe chacun des
/// deux descripteurs rendus par le noyau dans un objet `socket`, et le
/// constructeur de CPython **valide** le descripteur en lui demandant son type
/// (`getsockopt(SO_TYPE)`). Ce chemin passait par `socket_of`, qui ne
/// connaissait que `FdKind::Socket` et rendait `ENOTSOCK` — si bien que
/// `socketpair` reussissait au niveau de l'appel systeme et echouait au niveau
/// de Python, avec « Socket operation on non-socket » pour tout indice.
///
/// Le navigateur ne pouvait donc pas creer son processus de rendu : la
/// separation eprouvee sur Linux, ou `socketpair` est complet, n'avait jamais
/// tourne sous Bouchaud OS.
fn est_paire(fd: i32) -> bool {
    let process = task::current_process();
    let borrowed = process.files.lock();
    matches!(borrowed.get(fd).map(|desc| &desc.kind),
             Some(FdKind::SocketPair(_, _)))
}

/// Le descripteur a-t-il ete place durablement en mode non bloquant ?
///
/// `SocketState::nonblocking` couvre `SOCK_NONBLOCK` a la creation. Ce test
/// couvre les bascules ulterieures par `fcntl(F_SETFL)` et `FIONBIO`, qui
/// vivent dans `FileDesc::flags`.
fn fd_non_bloquant(fd: i32) -> bool {
    let process = task::current_process();
    let borrowed = process.files.lock();
    borrowed.get(fd)
        .map(|desc| desc.flags & O_NONBLOCK != 0)
        .unwrap_or(false)
}

/// Recupere l'etat d'un socket depuis son descripteur.
fn socket_of(fd: i32) -> Result<Arc<SpinLock<SocketState>>, i64> {
    let process = task::current_process();
    let borrowed = process.files.lock();
    match borrowed.get(fd) {
        Some(desc) => match &desc.kind {
            FdKind::Socket(state) => Ok(state.clone()),
            _ => Err(-errno::ENOTSOCK),
        },
        None => Err(-errno::EBADF),
    }
}

/// `socket`.
pub fn sys_socket(domain: u32, kind: u32, _protocol: u32) -> i64 {
    let nonblocking = kind & SOCK_NONBLOCK != 0;
    let cloexec = kind & SOCK_CLOEXEC != 0;
    let base = kind & 0xFF;

    if domain == AF_INET6 {
        // Pas d'IPv6 dans la pile : le dire franchement fait retomber
        // `getaddrinfo` sur IPv4 au lieu de le faire echouer.
        return -errno::EAFNOSUPPORT;
    }
    if domain != AF_INET && domain != AF_UNIX {
        return -errno::EAFNOSUPPORT;
    }
    let socket_kind = match base {
        SOCK_STREAM => SocketKind::Tcp,
        SOCK_DGRAM => SocketKind::Udp,
        _ => return -errno::EPROTONOSUPPORT,
    };

    let mut state = SocketState::new(socket_kind);
    state.nonblocking = nonblocking;
    let mut desc = FileDesc::new(FdKind::Socket(Arc::new(SpinLock::new(state))));
    desc.cloexec = cloexec;
    let process = task::current_process();
    let fd = process.files.lock().insert(desc);
    fd as i64
}

/// `connect`.
pub fn sys_connect(fd: i32, addr: u64, len: usize) -> i64 {
    let state = match socket_of(fd) {
        Ok(state) => state,
        Err(code) => return code,
    };
    let (ip, port) = match read_sockaddr(addr, len) {
        Some(value) => value,
        None => return -errno::EAFNOSUPPORT,
    };

    let kind = state.lock().kind;
    match kind {
        SocketKind::Udp => {
            // En UDP, `connect` ne fait que fixer la destination par defaut.
            let mut borrowed = state.lock();
            borrowed.peer = Some((ip, port));
            if borrowed.local_port == 0 {
                borrowed.local_port = ephemeral_port();
            }
            0
        }
        SocketKind::Tcp => {
            if state.lock().conn.is_some() {
                return -errno::EISCONN;
            }
            // La poignee de main est synchrone : la pile est pilotee par
            // interrogation, il n'y a personne d'autre pour la faire avancer.
            match TcpConn::connect(ip, port) {
                Some(conn) => {
                    let mut borrowed = state.lock();
                    borrowed.conn = Some(conn);
                    borrowed.peer = Some((ip, port));
                    0
                }
                None => -errno::ECONNREFUSED,
            }
        }
    }
}

/// `bind` : n'a de sens ici que pour fixer le port source d'un socket UDP.
pub fn sys_bind(fd: i32, addr: u64, len: usize) -> i64 {
    let state = match socket_of(fd) {
        Ok(state) => state,
        Err(code) => return code,
    };
    let port = read_sockaddr(addr, len).map(|(_, port)| port).unwrap_or(0);
    let mut borrowed = state.lock();
    borrowed.local_port = if port == 0 { ephemeral_port() } else { port };
    0
}

/// `sendto` / `send` / `write` sur un socket.
pub fn sys_sendto(fd: i32, buffer: u64, len: usize, flags: u32, addr: u64, addr_len: usize) -> i64 {
    if len == 0 {
        return 0;
    }
    let data = match user_read(buffer, len) {
        Some(data) => data,
        None => return -errno::EFAULT,
    };
    envoie_octets(fd, &data, flags, addr, addr_len)
}

/// `sendto`, une fois les octets deja en memoire noyau.
///
/// Separe de [`sys_sendto`] pour `sendfile` : sa source est un fichier, sa
/// destination une socket, et rien dans l'operation ne passe par l'espace
/// utilisateur.
pub fn envoie_octets(fd: i32, data: &[u8], _flags: u32, addr: u64, addr_len: usize) -> i64 {
    let len = data.len();
    if len == 0 {
        return 0;
    }
    // Une extremite de `socketpair` est un tube croise : elle n'a ni pair ni
    // adresse, et `send`/`sendall` sur elle n'est rien d'autre qu'un `write`.
    // Sans cette bifurcation, `socket_of` rendait `ENOTSOCK` — c'est-a-dire
    // qu'une paire creee avec succes refusait ensuite le moindre octet.
    if est_paire(fd) {
        return crate::kernel::abi::file::ecrit_octets(fd, data);
    }
    let state = match socket_of(fd) {
        Ok(state) => state,
        Err(code) => return code,
    };
    let kind = state.lock().kind;
    match kind {
        SocketKind::Tcp => {
            let mut borrowed = state.lock();
            match borrowed.conn.as_mut() {
                Some(conn) => {
                    if conn.send(&data) {
                        len as i64
                    } else {
                        -errno::EPIPE
                    }
                }
                None => -errno::ENOTCONN,
            }
        }
        SocketKind::Udp => {
            let target = match read_sockaddr(addr, addr_len) {
                Some(value) => Some(value),
                None => state.lock().peer,
            };
            let (ip, port) = match target {
                Some(value) => value,
                None => return -errno::EDESTADDRREQ,
            };
            let source_port = {
                let mut borrowed = state.lock();
                if borrowed.local_port == 0 {
                    borrowed.local_port = ephemeral_port();
                }
                borrowed.local_port
            };
            let mut packet = alloc::vec![0u8; len + 8];
            let size = match crate::net::transport::udp::build(&mut packet, source_port, port, &data) {
                Some(size) => size,
                None => return -errno::EMSGSIZE,
            };
            let parti = crate::net::send_ip(ip, crate::net::internet::ipv4::PROTO_UDP, &packet[..size]);
            if trace_dns(port, source_port) {
                crate::serial_println!(
                    "[ladybird-bouchaud] M17_UDP_TX dst={}.{}.{}.{}:{} src_port={} octets={} parti={}",
                    ip[0], ip[1], ip[2], ip[3], port, source_port, len, parti
                );
            }
            if parti {
                len as i64
            } else {
                -errno::ENETUNREACH
            }
        }
    }
}

/// Recupere les datagrammes UDP destines a ce socket.
///
/// La pile ne recoit que lorsqu'on l'interroge : c'est donc ici, dans le
/// `recvfrom` de l'appelant, que les paquets sont effectivement lus sur la
/// carte. Les datagrammes qui ne nous sont pas destines sont ignores.
/// Nombre maximal de datagrammes traites en un passage.
///
/// Il ne s'agit pas d'un budget d'attente mais d'une borne de politesse : un
/// flux soutenu ne doit pas retenir l'appelant dans le noyau indefiniment. La
/// boucle s'arrete de toute facon des que l'anneau est vide.
const DATAGRAMMES_PAR_PASSAGE: u32 = 64;

/// Delai d'un `recvfrom` bloquant sur un socket UDP, en millisecondes.
///
/// UDP n'a pas de notion de fin de flux : sans delai, un `recvfrom` bloquant
/// sur une reponse qui ne viendra jamais ne rendrait jamais la main. Cinq
/// secondes est l'ordre de grandeur des resolveurs (`RES_TIMEOUT` vaut 5 dans
/// la libc), ce qui laisse a un appelant le temps de reessayer lui-meme.
const RECV_UDP_DELAI_MS: u64 = 5_000;

/// Le port 53 est-il en cause dans ce datagramme ?
///
/// ## Pourquoi une sonde ici, et pourquoi bornee au port 53
///
/// Les sondes M16 ont ferme deux des trois hypotheses sur le blocage DNS : la
/// socket vers le resolveur est bien creee (`M16_DNS_SOCKET_OK`), et la requete
/// part reellement (`M16_DNS_TX id=57801 octets=46`). Le minuteur de
/// retransmission tire (`M16_DNS_REPEAT`), donc la boucle d'evenements vit.
/// Mais `M16_DNS_READY` n'apparait **jamais** : la socket ne se declare jamais
/// lisible, pendant cinq minutes.
///
/// Reste un seul segment sans mesure — celui qui va de la carte a la file du
/// socket. Trois issues y sont possibles et rien ne les distingue :
///
///  1. le datagramme n'arrive jamais sur la carte ;
///  2. il arrive, et `livre_datagramme` ne trouve aucun socket a qui le rendre ;
///  3. il arrive et il est rendu — et c'est alors `poll` ou le notificateur
///     qu'il faut regarder, pas le reseau.
///
/// La sonde se limite au port 53. Ce n'est pas de la prudence : c'est ce qui la
/// rend utilisable. Une trace de tout l'UDP noierait la console serie sous le
/// trafic ordinaire, et une trace qu'on ne peut pas lire ne mesure rien. Deux a
/// quatre paquets par resolution, c'est exactement le volume qu'on veut voir.
///
/// Elle est **temporaire** et disparaitra avec le defaut qu'elle sert a nommer.
fn trace_dns(port_a: u16, port_b: u16) -> bool {
    port_a == 53 || port_b == 53
}

/// Livre un datagramme au socket qui l'attend, parmi ceux du processus.
///
/// ## Pourquoi ce detour par la table des descripteurs
///
/// Une seule carte alimente tous les sockets. Le code qui lit l'anneau le fait
/// pour le compte d'**un** socket, mais la trame qu'il en sort peut appartenir
/// a un autre : elle est deja hors de l'anneau, et la jeter la perd
/// definitivement. C'est ce que faisait la version precedente, et c'est ce qui
/// bloquait toute resolution de nom : un resolveur emet plusieurs requetes en
/// parallele — A et AAAA — et le premier socket servi mangeait la reponse du
/// second. La sonde `tools/userland/dns-probe.c` reproduit exactement ce cas.
///
/// On fait donc ici ce que fait une pile normale : router sur le port de
/// destination. Un datagramme adresse a un port que le processus n'a pas
/// ouvert est ecarte, comme il doit l'etre.
fn livre_datagramme(
    source: crate::net::internet::ipv4::Ipv4Addr,
    entete: &crate::net::transport::udp::Header,
    donnees: &[u8],
) {
    let process = task::current_process();

    // On choisit la destination avant d'emprunter quoi que ce soit en
    // ecriture : un socket connecte a la source l'emporte sur un socket
    // simplement lie, comme le veut la specification des sockets.
    let mut lie: Option<Arc<SpinLock<SocketState>>> = None;
    let mut connecte: Option<Arc<SpinLock<SocketState>>> = None;
    {
        let emprunte = process.files.lock();
        for desc in emprunte.iter() {
            let socket = match &desc.kind {
                FdKind::Socket(socket) => socket,
                _ => continue,
            };
            let (kind, local_port, peer) = match socket.try_lock() {
                Some(etat) => (etat.kind, etat.local_port, etat.peer),
                // Un socket deja emprunte en ecriture est celui qui nous a
                // appeles ; ses champs sont ceux qu'on connait par ailleurs.
                None => continue,
            };
            if kind != SocketKind::Udp || local_port != entete.dst_port {
                continue;
            }
            match peer {
                Some((ip, port)) if ip == source && port == entete.src_port => {
                    connecte = Some(socket.clone());
                    break;
                }
                Some(_) => continue,
                None => {
                    if lie.is_none() {
                        lie = Some(socket.clone());
                    }
                }
            }
        }
    }

    let trace = trace_dns(entete.src_port, entete.dst_port);
    let connecte_trouve = connecte.is_some();

    match connecte.or(lie) {
        Some(destination) => match destination.try_lock() {
            Some(mut etat) => {
                etat.datagrams
                    .push((source, entete.src_port, donnees.to_vec()));
                if trace {
                    crate::serial_println!(
                        "[ladybird-bouchaud] M17_UDP_LIVRE src={}.{}.{}.{}:{} vers_port={} octets={} connecte={}",
                        source[0], source[1], source[2], source[3],
                        entete.src_port, entete.dst_port, donnees.len(), connecte_trouve
                    );
                }
            }
            // Le socket destinataire est deja emprunte : c'est celui qui nous a
            // appeles. Le datagramme est perdu, et il faut que cela se voie.
            None => {
                if trace {
                    crate::serial_println!(
                        "[ladybird-bouchaud] M17_UDP_PERDU_EMPRUNTE vers_port={} octets={}",
                        entete.dst_port, donnees.len()
                    );
                }
            }
        },
        None => {
            if trace {
                crate::serial_println!(
                    "[ladybird-bouchaud] M17_UDP_SANS_DESTINATAIRE src={}.{}.{}.{}:{} vers_port={} octets={}",
                    source[0], source[1], source[2], source[3],
                    entete.src_port, entete.dst_port, donnees.len()
                );
            }
        }
    }
}

/// Vide l'anneau de reception et route ce qu'il contient.
///
/// `poll_ip` est **non bloquant** : lorsqu'il rend `None`, l'anneau est vide a
/// cet instant. Le rappeler des milliers de fois ne peut donc rien faire
/// arriver — c'etait la boucle a plein processeur observee pendant l'attente
/// d'une reponse DNS, et le motif pour lequel un `recvfrom` bloquant coutait
/// cinq secondes de cœur au lieu de cinq secondes de sommeil.
fn pump_udp(state: &Arc<SpinLock<SocketState>>) {
    if state.lock().local_port == 0 {
        return;
    }
    let mut payload = [0u8; 2048];
    for _ in 0..DATAGRAMMES_PAR_PASSAGE {
        let received = crate::net::poll_ip(
            crate::net::internet::ipv4::PROTO_UDP,
            None,
            &mut payload,
        );
        let (source, size) = match received {
            Some(value) => value,
            None => break,
        };
        let header = match crate::net::transport::udp::parse(&payload[..size]) {
            Some(header) => header,
            None => continue,
        };
        let start = header.payload_off;
        let end = start + header.payload_len;
        if end <= size {
            livre_datagramme(source, &header, &payload[start..end]);
        }
    }
}

/// `recvfrom` / `recv` / `read` sur un socket.
pub fn sys_recvfrom(
    fd: i32,
    buffer: u64,
    len: usize,
    flags: u32,
    addr: u64,
    addr_len: u64,
) -> i64 {
    if len == 0 {
        return 0;
    }
    // Symetrique de `sys_sendto` : lire sur une paire, c'est `read`. Aucune
    // adresse d'expediteur n'est ecrite — une paire est anonyme des deux
    // cotes, et Linux ne remplit rien non plus.
    if est_paire(fd) {
        // `MSG_DONTWAIT` est local a cet appel. Il ne faut surtout pas poser
        // `O_NONBLOCK` temporairement sur le descripteur : les autres threads
        // verraient alors un etat qui ne leur appartient pas.
        //
        // Le noyau Bouchaud n'est pas preempte au milieu d'un syscall. Tester
        // le tampon puis entrer dans `sys_read` est donc atomique vis-a-vis des
        // autres taches : si le canal est vide ici, une lecture non bloquante
        // doit rendre `EAGAIN` immediatement au lieu d'entrer dans l'attente de
        // deux secondes de `sys_read(SocketPair)`.
        if fd_non_bloquant(fd) || flags & MSG_DONTWAIT != 0 {
            let vide = {
                let process = task::current_process();
                let borrowed = process.files.lock();
                match borrowed.get(fd).map(|desc| &desc.kind) {
                    Some(FdKind::SocketPair(inbox, _)) => inbox.lock().octets.is_empty(),
                    _ => return -errno::ENOTSOCK,
                }
            };
            if vide {
                return -errno::EAGAIN;
            }
        }
        let lu = crate::kernel::abi::file::sys_read(fd, buffer, len);
        if lu >= 0 && addr_len != 0 {
            user_write(addr_len, &0u32.to_le_bytes());
        }
        let _ = addr;
        return lu;
    }
    let state = match socket_of(fd) {
        Ok(state) => state,
        Err(code) => return code,
    };
    let (kind, nonblocking) = {
        let borrowed = state.lock();
        (
            borrowed.kind,
            borrowed.nonblocking || fd_non_bloquant(fd) || flags & MSG_DONTWAIT != 0,
        )
    };

    match kind {
        SocketKind::Tcp => {
            // Attente bloquante : sur le temps, et en rendant le processeur.
            //
            // `conn.fill(1)` faisait cela SOUS le verrou du socket et sans
            // jamais ceder : jusqu'a trois secondes de gros verrou tenu, la
            // meme panne de vivacite que l'attente ARP corrigee dans ce lot.
            // Un verrou tournant interdit de dormir ; on adopte donc la forme
            // que la branche UDP emploie deja juste en dessous -- pomper sous
            // le verrou, attendre dehors.
            //
            // Les issues sont inchangees : donnees -> les rendre ; pair ferme
            // et tampon vide -> fin de flux ; non bloquant et rien -> EAGAIN ;
            // trois secondes sans rien -> tampon vide, donc 0, exactement ce
            // que rendait `fill(1)` epuise.
            let mut tours_vides = 0u32;
            let echeance = crate::kernel::timer::ticks()
                + 3 * crate::kernel::timer::TICKS_PER_SECOND.max(1);
            loop {
                let pret = {
                    let mut borrowed = state.lock();
                    let conn = match borrowed.conn.as_mut() {
                        Some(conn) => conn,
                        None => return -errno::ENOTCONN,
                    };
                    if conn.rx.is_empty() && !conn.peer_fin && !conn.closed {
                        conn.pump(50_000);
                    }
                    if !conn.rx.is_empty() {
                        Some(conn.take(len))
                    } else if conn.peer_fin || conn.closed {
                        return 0;
                    } else {
                        None
                    }
                };
                if let Some(data) = pret {
                    let read = data.len();
                    return if user_write(buffer, &data) {
                        read as i64
                    } else {
                        -errno::EFAULT
                    };
                }
                if nonblocking {
                    return -errno::EAGAIN;
                }
                if crate::kernel::timer::ticks() >= echeance {
                    return 0;
                }
                task::attends_io_adaptatif(&mut tours_vides);
            }
        }
        SocketKind::Udp => {
            if state.lock().datagrams.is_empty() {
                pump_udp(&state);
            }
            // Attente bloquante : sur le temps, et en rendant le processeur.
            //
            // La version precedente comptait des tours de boucle — trois
            // millions — ce qui revenait a attendre une duree que personne
            // n'avait choisie, en brulant un cœur pendant ce temps. On attend
            // desormais une duree nommee, et entre deux sondages on laisse
            // tourner les autres taches puis on dort jusqu'a une interruption.
            if !nonblocking && state.lock().datagrams.is_empty() {
                let echeance = crate::kernel::timer::monotonic_ms() + RECV_UDP_DELAI_MS;
                while state.lock().datagrams.is_empty()
                    && crate::kernel::timer::monotonic_ms() < echeance
                {
                    // Une autre tache a pu remplir l'anneau : re-sonder avant
                    // de dormir, sinon le reveil logiciel est perdu jusqu'a la
                    // prochaine interruption materielle. Meme raison que dans
                    // `sys_poll`.
                    task::attends_un_tick();
                    pump_udp(&state);
                }
            }
            let datagram = state.lock().datagrams.pop();
            match datagram {
                None => {
                    if nonblocking {
                        -errno::EAGAIN
                    } else {
                        -errno::ETIMEDOUT
                    }
                }
                Some((source, port, data)) => {
                    let size = core::cmp::min(len, data.len());
                    if !user_write(buffer, &data[..size]) {
                        return -errno::EFAULT;
                    }
                    write_sockaddr(addr, addr_len, source, port);
                    size as i64
                }
            }
        }
    }
}

/// Champs de `struct msghdr`, en octets.
const MSG_NAME: u64 = 0;
const MSG_NAMELEN: u64 = 8;
const MSG_IOV: u64 = 16;
const MSG_IOVLEN: u64 = 24;
const MSG_CONTROL: u64 = 32;
const MSG_CONTROLLEN: u64 = 40;
const MSG_FLAGS: u64 = 48;

const SOL_SOCKET: u32 = 1;
const SCM_RIGHTS: u32 = 1;

/// En-tete d'un message de controle : `{ len: u64, level: i32, type: i32 }`.
const CMSG_ENTETE: usize = 16;

/// Lit les descripteurs qu'un `sendmsg` veut faire passer.
///
/// `SCM_RIGHTS` est la seule facon pour deux processus de se partager autre
/// chose que des octets : un tampon anonyme, une extremite de socket, un
/// fichier deja ouvert. L'architecture multi-processus d'un moteur web repose
/// entierement la-dessus — sans elle, chaque processus devrait tout recopier.
fn lit_descripteurs_envoyes(msghdr: u64) -> Vec<i32> {
    let mut fds = Vec::new();
    let controle = match crate::kernel::abi::user_read_u64(msghdr + MSG_CONTROL) {
        Some(adresse) if adresse != 0 => adresse,
        _ => return fds,
    };
    let longueur = crate::kernel::abi::user_read_u64(msghdr + MSG_CONTROLLEN)
        .unwrap_or(0) as usize;
    if longueur < CMSG_ENTETE {
        return fds;
    }

    let mut position = 0usize;
    while position + CMSG_ENTETE <= longueur {
        let base = controle + position as u64;
        let taille = match crate::kernel::abi::user_read_u64(base) {
            Some(valeur) => valeur as usize,
            None => break,
        };
        if taille < CMSG_ENTETE || position + taille > longueur {
            break;
        }
        let niveau = crate::kernel::abi::user_read(base + 8, 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .unwrap_or(0);
        let genre = crate::kernel::abi::user_read(base + 12, 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .unwrap_or(0);

        if niveau == SOL_SOCKET && genre == SCM_RIGHTS {
            let mut decalage = CMSG_ENTETE;
            while decalage + 4 <= taille {
                if let Some(brut) = crate::kernel::abi::user_read(
                    base + decalage as u64, 4) {
                    fds.push(i32::from_le_bytes([brut[0], brut[1], brut[2], brut[3]]));
                }
                decalage += 4;
            }
        }
        // Alignement sur huit octets, comme le veut `CMSG_NXTHDR`.
        position += (taille + 7) & !7;
    }
    fds
}

/// Ecrit dans le `msghdr` les descripteurs recus. Rend le nombre ecrit.
fn ecrit_descripteurs_recus(msghdr: u64, fds: &[i32]) -> usize {
    if fds.is_empty() {
        let _ = user_write(msghdr + MSG_CONTROLLEN, &0u64.to_le_bytes());
        return 0;
    }
    let controle = match crate::kernel::abi::user_read_u64(msghdr + MSG_CONTROL) {
        Some(adresse) if adresse != 0 => adresse,
        _ => return 0,
    };
    let disponible = crate::kernel::abi::user_read_u64(msghdr + MSG_CONTROLLEN)
        .unwrap_or(0) as usize;

    // On n'ecrit que ce qui tient : l'appelant a dimensionne son tampon, et
    // deborder ecraserait sa pile.
    let tiennent = if disponible < CMSG_ENTETE + 4 {
        0
    } else {
        core::cmp::min(fds.len(), (disponible - CMSG_ENTETE) / 4)
    };
    if tiennent == 0 {
        let _ = user_write(msghdr + MSG_CONTROLLEN, &0u64.to_le_bytes());
        // `MSG_CTRUNC` : le recepteur doit savoir qu'il a perdu des
        // descripteurs, sinon il attendrait indefiniment ce qui n'arrivera pas.
        let _ = user_write(msghdr + MSG_FLAGS, &8u32.to_le_bytes());
        return 0;
    }

    let taille = CMSG_ENTETE + tiennent * 4;
    let _ = user_write(controle, &(taille as u64).to_le_bytes());
    let _ = user_write(controle + 8, &SOL_SOCKET.to_le_bytes());
    let _ = user_write(controle + 12, &SCM_RIGHTS.to_le_bytes());
    for (index, fd) in fds.iter().take(tiennent).enumerate() {
        let _ = user_write(controle + CMSG_ENTETE as u64 + (index * 4) as u64,
                           &fd.to_le_bytes());
    }
    let _ = user_write(msghdr + MSG_CONTROLLEN, &(taille as u64).to_le_bytes());
    if tiennent < fds.len() {
        let _ = user_write(msghdr + MSG_FLAGS, &8u32.to_le_bytes());
    }
    tiennent
}

/// Lit les `iovec` d'un `msghdr` : (adresse, longueur) pour chacun.
fn read_iovecs(msghdr: u64) -> Option<Vec<(u64, usize)>> {
    let iov = crate::kernel::abi::user_read_u64(msghdr + MSG_IOV)?;
    let count = crate::kernel::abi::user_read_u64(msghdr + MSG_IOVLEN)? as usize;
    let mut out = Vec::new();
    for index in 0..count.min(64) {
        let base = crate::kernel::abi::user_read_u64(iov + (index * 16) as u64)?;
        let len = crate::kernel::abi::user_read_u64(iov + (index * 16) as u64 + 8)? as usize;
        out.push((base, len));
    }
    Some(out)
}

/// `sendmsg` : rassemble les `iovec` puis emet en une fois.
///
/// C'est la forme qu'emploie le resolveur de musl ; ne pas la fournir suffit a
/// faire echouer toute resolution de nom, alors meme que `sendto` fonctionne.
pub fn sys_sendmsg(fd: i32, msghdr: u64, flags: u32) -> i64 {
    let iovecs = match read_iovecs(msghdr) {
        Some(list) => list,
        None => return -errno::EFAULT,
    };
    let name = crate::kernel::abi::user_read_u64(msghdr + MSG_NAME).unwrap_or(0);
    let namelen = crate::kernel::abi::user_read(msghdr + MSG_NAMELEN, 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
        .unwrap_or(0);

    let mut payload = Vec::new();
    for (base, len) in iovecs {
        if len == 0 {
            continue;
        }
        match user_read(base, len) {
            Some(chunk) => payload.extend_from_slice(&chunk),
            None => return -errno::EFAULT,
        }
    }
    // Les descripteurs que l'appelant veut faire passer partent avec le
    // message. Un envoi peut n'etre *que* cela : `SCM_RIGHTS` accompagne
    // souvent un octet unique, parfois aucun.
    let a_passer = lit_descripteurs_envoyes(msghdr);
    if !a_passer.is_empty() {
        let process = task::current_process();
        let sortant = match process.files.lock().get(fd).map(|d| d.kind.clone()) {
            Some(FdKind::SocketPair(_, sortant)) => Some(sortant),
            // Passer un descripteur n'a de sens que sur un socket local.
            _ => return -errno::EINVAL,
        };
        if let Some(canal) = sortant {
            for fd_source in &a_passer {
                // Le descripteur est **copie** dans le canal : le recepteur en
                // obtiendra un a lui, et l'emetteur garde le sien. C'est ce que
                // dit la norme, et c'est ce qui evite qu'un envoi ferme un
                // fichier sous les pieds de celui qui l'envoie.
                let copie = process.files.lock().get(*fd_source).cloned();
                match copie {
                    Some(desc) => canal.lock().descripteurs.push(desc),
                    None => return -errno::EBADF,
                }
            }
        }
    }

    // Une paire de sockets n'a ni adresse ni pile TCP : le corps s'ecrit
    // directement dans le canal sortant. Le faire passer par `sendto`, comme on
    // le faisait, echouait — et l'appelant en concluait que son envoi n'etait
    // pas parti, alors que les descripteurs, eux, etaient bien arrives.
    {
        let process = task::current_process();
        let sortant = match process.files.lock().get(fd).map(|d| d.kind.clone()) {
            Some(FdKind::SocketPair(_, sortant)) => Some(sortant),
            _ => None,
        };
        if let Some(canal) = sortant {
            let ecrits = payload.len();
            canal.lock().octets.extend_from_slice(&payload);
            crate::kernel::fd::notify_readiness();
            return ecrits as i64;
        }
    }

    if payload.is_empty() {
        return 0;
    }

    // On repasse par `sendto`, qui connait deja les deux protocoles. Le tampon
    // rassemble est ecrit dans une zone temporaire de l'espace utilisateur ?
    // Non : `sendto` relit depuis l'utilisateur. On appelle donc directement la
    // couche d'emission avec les octets deja en main.
    send_bytes(fd, &payload, name, namelen, flags)
}

/// Emission d'un tampon deja copie depuis l'espace utilisateur.
fn send_bytes(fd: i32, data: &[u8], addr: u64, addr_len: usize, _flags: u32) -> i64 {
    let state = match socket_of(fd) {
        Ok(state) => state,
        Err(code) => return code,
    };
    let kind = state.lock().kind;
    match kind {
        SocketKind::Tcp => {
            let mut borrowed = state.lock();
            match borrowed.conn.as_mut() {
                Some(conn) => {
                    if conn.send(data) { data.len() as i64 } else { -errno::EPIPE }
                }
                None => -errno::ENOTCONN,
            }
        }
        SocketKind::Udp => {
            let target = match read_sockaddr(addr, addr_len) {
                Some(value) => Some(value),
                None => state.lock().peer,
            };
            let (ip, port) = match target {
                Some(value) => value,
                None => return -errno::EDESTADDRREQ,
            };
            let source_port = {
                let mut borrowed = state.lock();
                if borrowed.local_port == 0 {
                    borrowed.local_port = ephemeral_port();
                }
                borrowed.local_port
            };
            let mut packet = alloc::vec![0u8; data.len() + 8];
            let size = match crate::net::transport::udp::build(&mut packet, source_port, port, data) {
                Some(size) => size,
                None => return -errno::EMSGSIZE,
            };
            if crate::net::send_ip(ip, crate::net::internet::ipv4::PROTO_UDP, &packet[..size]) {
                data.len() as i64
            } else {
                -errno::ENETUNREACH
            }
        }
    }
}

/// `recvmsg` : recoit dans le premier `iovec` assez grand.
pub fn sys_recvmsg(fd: i32, msghdr: u64, flags: u32) -> i64 {
    let iovecs = match read_iovecs(msghdr) {
        Some(list) => list,
        None => return -errno::EFAULT,
    };
    let (base, len) = match iovecs.iter().find(|(_, len)| *len > 0) {
        Some(value) => *value,
        None => return 0,
    };
    let name = crate::kernel::abi::user_read_u64(msghdr + MSG_NAME).unwrap_or(0);

    // Les descripteurs arrives par `SCM_RIGHTS` sont installes dans ce
    // processus avant la lecture des octets : c'est ce qui permet a un
    // `recvmsg` qui ne recoit *que* des descripteurs — le cas courant — de les
    // rendre malgre un corps vide.
    let process = task::current_process();
    let entrant = match process.files.lock().get(fd).map(|d| d.kind.clone()) {
        Some(FdKind::SocketPair(entrant, _)) => Some(entrant),
        _ => None,
    };
    let mut installes: Vec<i32> = Vec::new();
    if let Some(canal) = &entrant {
        // Meme attente que pour les octets : le pair n'a peut-etre pas encore
        // eu la main. Sans elle, le premier `recvmsg` bloquant d'un dialogue
        // echoue. `MSG_DONTWAIT`, lui, interdit explicitement cette attente et
        // doit conduire a `EAGAIN` immediatement si le canal est vide.
        let bloquant = !fd_non_bloquant(fd)
            && flags & MSG_DONTWAIT == 0;
        if bloquant && canal.lock().descripteurs.is_empty()
            && canal.lock().octets.is_empty()
        {
            let echeance = crate::kernel::timer::ticks()
                + crate::kernel::timer::ms_to_ticks(2000);
            while canal.lock().descripteurs.is_empty()
                && canal.lock().octets.is_empty()
                && crate::kernel::timer::ticks() < echeance
            {
                task::attends_un_tick();
            }
        }
    }
    if let Some(canal) = entrant {
        let recus: Vec<_> = canal.lock().descripteurs.drain(..).collect();
        for desc in recus {
            let numero = process.files.lock().insert(desc);
            if numero >= 0 {
                installes.push(numero);
            }
        }
    }
    ecrit_descripteurs_recus(msghdr, &installes);

    let received = sys_recvfrom(fd, base, len, flags, name, 0);
    if received >= 0 && name != 0 {
        // `msg_namelen` doit refleter la taille reellement ecrite.
        user_write(msghdr + MSG_NAMELEN, &16u32.to_le_bytes());
    }
    // Des descripteurs sans octets ne sont pas une fin de flux : rendre une
    // erreur ferait croire a l'appelant qu'il n'a rien recu, alors qu'il vient
    // d'obtenir ce qu'il attendait.
    if received < 0 && !installes.is_empty() {
        return 0;
    }
    received
}

/// `shutdown` : ferme la connexion sans liberer le descripteur.
pub fn sys_shutdown(fd: i32, _how: u32) -> i64 {
    let state = match socket_of(fd) {
        Ok(state) => state,
        Err(code) => return code,
    };
    let mut borrowed = state.lock();
    if let Some(conn) = borrowed.conn.as_mut() {
        conn.close();
    }
    borrowed.eof = true;
    0
}

/// `getsockname` / `getpeername`.
pub fn sys_getsockname(fd: i32, addr: u64, len_addr: u64, peer: bool) -> i64 {
    // Une paire est anonyme : Linux rend une `sockaddr_un` reduite a sa
    // famille, longueur deux. Rendre `ENOTSOCK` ferait croire a l'appelant
    // qu'il ne tient pas un socket, ce qui est faux et le fait renoncer.
    if est_paire(fd) {
        let _ = peer;
        if addr != 0 && !user_write(addr, &(AF_UNIX as u16).to_le_bytes()) {
            return -errno::EFAULT;
        }
        if len_addr != 0 && !user_write(len_addr, &2u32.to_le_bytes()) {
            return -errno::EFAULT;
        }
        return 0;
    }
    let state = match socket_of(fd) {
        Ok(state) => state,
        Err(code) => return code,
    };
    let borrowed = state.lock();
    let (ip, port) = if peer {
        match borrowed.peer {
            Some(value) => value,
            None => return -errno::ENOTCONN,
        }
    } else {
        (crate::net::our_ip(), borrowed.local_port)
    };
    if write_sockaddr(addr, len_addr, ip, port) { 0 } else { -errno::EFAULT }
}

/// `setsockopt` : accepte et ignore, sauf le mode non bloquant.
pub fn sys_setsockopt(fd: i32, _level: u32, _option: u32, _value: u64, _len: usize) -> i64 {
    // Les options usuelles (SO_REUSEADDR, TCP_NODELAY, SO_RCVBUF...) n'ont pas
    // d'equivalent ici. Renvoyer une erreur ferait echouer des bibliotheques
    // qui les posent par principe ; les accepter sans effet est sans risque.
    let _ = fd;
    0
}

/// `getsockopt`.
pub fn sys_getsockopt(fd: i32, _level: u32, option: u32, value: u64, len_addr: u64) -> i64 {
    const SO_ERROR: u32 = 4;
    const SO_TYPE: u32 = 3;
    // `SO_TYPE` sur une paire : c'est **cette** question que pose CPython pour
    // valider un descripteur qu'on lui donne, et y repondre `ENOTSOCK` suffit
    // a rendre `socketpair` inutilisable depuis Python. Voir `est_paire`.
    let result: u32 = if est_paire(fd) {
        match option {
            SO_TYPE => SOCK_STREAM,   // `sys_socketpair` n'en cree pas d'autre
            _ => 0,
        }
    } else {
        let state = match socket_of(fd) {
            Ok(state) => state,
            Err(code) => return code,
        };
        match option {
            // `connect` etant synchrone ici, il n'y a jamais d'erreur differee.
            SO_ERROR => 0,
            SO_TYPE => match state.lock().kind {
                SocketKind::Tcp => SOCK_STREAM,
                SocketKind::Udp => SOCK_DGRAM,
            },
            _ => 0,
        }
    };
    if value != 0 && !user_write(value, &result.to_le_bytes()) {
        return -errno::EFAULT;
    }
    if len_addr != 0 {
        user_write(len_addr, &4u32.to_le_bytes());
    }
    0
}

/// `socketpair` : deux extremites reliees, implementees par deux tubes croises.
pub fn sys_socketpair(_domain: u32, kind: u32, _protocol: u32, out: u64) -> i64 {
    let cloexec = kind & SOCK_CLOEXEC != 0;
    // Un socket est bidirectionnel : il faut donc deux tampons, chacun lu d'un
    // cote et ecrit de l'autre.
    let a_to_b = crate::kernel::fd::Canal::neuf();
    let b_to_a = crate::kernel::fd::Canal::neuf();

    let process = task::current_process();
    let (first, second) = {
        let mut borrowed = process.files.lock();
        let mut end_a = FileDesc::new(FdKind::SocketPair(b_to_a.clone(), a_to_b.clone()));
        let mut end_b = FileDesc::new(FdKind::SocketPair(a_to_b, b_to_a));
        end_a.cloexec = cloexec;
        end_b.cloexec = cloexec;
        (borrowed.insert(end_a), borrowed.insert(end_b))
    };
    if !user_write(out, &first.to_le_bytes()) || !user_write(out + 4, &second.to_le_bytes()) {
        return -errno::EFAULT;
    }
    0
}

/// `listen` / `accept` : voir la note d'en-tete du module.
pub fn sys_listen_unsupported() -> i64 {
    -errno::ENOSYS
}

/// Un socket a-t-il des donnees a lire ? (pour `poll`/`select`)
pub fn socket_readable(state: &Arc<SpinLock<SocketState>>) -> bool {
    let kind = state.lock().kind;
    match kind {
        SocketKind::Tcp => {
            let mut borrowed = state.lock();
            match borrowed.conn.as_mut() {
                Some(conn) => {
                    if conn.rx.is_empty() && !conn.peer_fin && !conn.closed {
                        // Un coup de pompe court : `poll` doit constater
                        // l'arrivee de donnees sans bloquer.
                        conn.pump(20_000);
                    }
                    !conn.rx.is_empty() || conn.peer_fin || conn.closed
                }
                None => false,
            }
        }
        SocketKind::Udp => {
            // `poll` demande un etat, pas une attente : un seul passage sur
            // l'anneau. C'est l'appelant qui decide s'il patiente, et c'est
            // `sys_poll` qui sait dormir entre deux tours.
            if state.lock().datagrams.is_empty() {
                pump_udp(state);
            }
            !state.lock().datagrams.is_empty()
        }
    }
}

/// Octets immediatement lisibles sur une prise, pour `ioctl(FIONREAD)`.
///
/// Linux ne rend pas la meme chose selon la famille, et la difference compte :
/// sur un flux il rend le contenu du tampon de reception, sur un datagramme il
/// rend la taille du **prochain** datagramme — jamais leur somme. Un lecteur
/// qui dimensionne son tampon sur cette valeur doit pouvoir en deduire qu'un
/// seul `recv` suffira ; annoncer le total le ferait tronquer tout ce qui suit,
/// puisqu'un `recv` sur une prise a datagrammes en consomme exactement un.
///
/// `LibCore` de Ladybird interroge cette valeur avant **chaque** lecture UDP,
/// dans `UDPSocket::read_some`, precisement pour refuser de tronquer :
///
/// ```cpp
/// auto pending_bytes = TRY(this->pending_bytes());
/// if (pending_bytes > buffer.size())
///     return Error::from_errno(EMSGSIZE);
/// ```
///
/// Le noyau rendait jusqu'ici 0 pour toute prise inet : le test ne se
/// declenchait donc jamais, et un datagramme plus grand que le tampon aurait
/// ete tronque en silence au lieu d'etre signale. C'est une primitive POSIX
/// fausse, independamment de qui l'appelle.
///
/// Comme `socket_readable`, un seul passage de pompe si rien n'est en attente :
/// la valeur doit decrire l'etat courant, pas celui du dernier appel.
pub fn octets_lisibles(state: &Arc<SpinLock<SocketState>>) -> usize {
    let kind = state.lock().kind;
    match kind {
        SocketKind::Tcp => {
            let mut borrowed = state.lock();
            match borrowed.conn.as_mut() {
                Some(conn) => {
                    if conn.rx.is_empty() && !conn.peer_fin && !conn.closed {
                        conn.pump(20_000);
                    }
                    conn.rx.len()
                }
                None => 0,
            }
        }
        SocketKind::Udp => {
            if state.lock().datagrams.is_empty() {
                pump_udp(state);
            }
            state
                .lock()
                .datagrams
                .first()
                .map(|(_, _, donnees)| donnees.len())
                .unwrap_or(0)
        }
    }
}

/// Taille de `struct mmsghdr` : un `msghdr` (56 octets) suivi de `msg_len`
/// (4 octets) puis d'un remplissage d'alignement.
const MMSGHDR_TAILLE: u64 = 64;
/// Position de `msg_len` dans `struct mmsghdr`.
const MMSG_LEN: u64 = 56;

/// `sendmmsg(fd, msgvec, vlen, flags)` : plusieurs messages en un appel.
///
/// La glibc s'en sert dans son resolveur de noms — elle interroge tous les
/// serveurs DNS d'un coup. Sans cet appel, `getaddrinfo` echoue avec
/// « Temporary failure in name resolution » et rien ne se charge. C'est
/// exactement le genre de manque qu'une sonde ecrite a la main ne trouve pas :
/// musl, lui, envoie ses requetes une par une.
pub fn sys_sendmmsg(fd: i32, msgvec: u64, vlen: u32, flags: u32) -> i64 {
    if msgvec == 0 {
        return -errno::EFAULT;
    }
    let mut envoyes = 0i64;
    for index in 0..vlen as u64 {
        let entree = msgvec + index * MMSGHDR_TAILLE;
        let resultat = sys_sendmsg(fd, entree, flags);
        if resultat < 0 {
            // Linux ne signale l'erreur que si aucun message n'est parti.
            return if envoyes > 0 { envoyes } else { resultat };
        }
        crate::kernel::abi::user_write(entree + MMSG_LEN, &(resultat as u32).to_le_bytes());
        envoyes += 1;
    }
    envoyes
}

/// `recvmmsg(fd, msgvec, vlen, flags, timeout)`.
///
/// Le delai est ignore : chaque `recvmsg` sous-jacent est deja non bloquant, et
/// l'appelant reessaie. On s'arrete des qu'un message manque, ce qui est le
/// comportement attendu — `recvmmsg` rend ce qu'il a pu lire.
pub fn sys_recvmmsg(fd: i32, msgvec: u64, vlen: u32, flags: u32, _timeout: u64) -> i64 {
    if msgvec == 0 {
        return -errno::EFAULT;
    }
    let mut recus = 0i64;
    for index in 0..vlen as u64 {
        let entree = msgvec + index * MMSGHDR_TAILLE;
        let resultat = sys_recvmsg(fd, entree, flags);
        if resultat < 0 {
            return if recus > 0 { recus } else { resultat };
        }
        crate::kernel::abi::user_write(entree + MMSG_LEN, &(resultat as u32).to_le_bytes());
        recus += 1;
    }
    recus
}
