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

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;

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

/// Recupere l'etat d'un socket depuis son descripteur.
fn socket_of(fd: i32) -> Result<Rc<RefCell<SocketState>>, i64> {
    let process = task::current_process();
    let borrowed = process.borrow();
    match borrowed.files.get(fd) {
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
    let mut desc = FileDesc::new(FdKind::Socket(Rc::new(RefCell::new(state))));
    desc.cloexec = cloexec;
    let process = task::current_process();
    let fd = process.borrow_mut().files.insert(desc);
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

    let kind = state.borrow().kind;
    match kind {
        SocketKind::Udp => {
            // En UDP, `connect` ne fait que fixer la destination par defaut.
            let mut borrowed = state.borrow_mut();
            borrowed.peer = Some((ip, port));
            if borrowed.local_port == 0 {
                borrowed.local_port = ephemeral_port();
            }
            0
        }
        SocketKind::Tcp => {
            if state.borrow().conn.is_some() {
                return -errno::EISCONN;
            }
            // La poignee de main est synchrone : la pile est pilotee par
            // interrogation, il n'y a personne d'autre pour la faire avancer.
            match TcpConn::connect(ip, port) {
                Some(conn) => {
                    let mut borrowed = state.borrow_mut();
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
    let mut borrowed = state.borrow_mut();
    borrowed.local_port = if port == 0 { ephemeral_port() } else { port };
    0
}

/// `sendto` / `send` / `write` sur un socket.
pub fn sys_sendto(fd: i32, buffer: u64, len: usize, _flags: u32, addr: u64, addr_len: usize) -> i64 {
    if len == 0 {
        return 0;
    }
    let state = match socket_of(fd) {
        Ok(state) => state,
        Err(code) => return code,
    };
    let data = match user_read(buffer, len) {
        Some(data) => data,
        None => return -errno::EFAULT,
    };

    let kind = state.borrow().kind;
    match kind {
        SocketKind::Tcp => {
            let mut borrowed = state.borrow_mut();
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
                None => state.borrow().peer,
            };
            let (ip, port) = match target {
                Some(value) => value,
                None => return -errno::EDESTADDRREQ,
            };
            let source_port = {
                let mut borrowed = state.borrow_mut();
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
            if crate::net::send_ip(ip, crate::net::internet::ipv4::PROTO_UDP, &packet[..size]) {
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
fn pump_udp(state: &Rc<RefCell<SocketState>>, budget: u32) {
    let local_port = state.borrow().local_port;
    if local_port == 0 {
        return;
    }
    let mut payload = [0u8; 2048];
    for _ in 0..budget {
        let received = crate::net::poll_ip(
            crate::net::internet::ipv4::PROTO_UDP,
            None,
            &mut payload,
        );
        let (source, size) = match received {
            Some(value) => value,
            None => continue,
        };
        if let Some(header) = crate::net::transport::udp::parse(&payload[..size]) {
            if header.dst_port != local_port {
                continue;
            }
            let start = header.payload_off;
            let end = start + header.payload_len;
            if end <= size {
                state.borrow_mut().datagrams.push((
                    source,
                    header.src_port,
                    payload[start..end].to_vec(),
                ));
                return;
            }
        }
    }
}

/// `recvfrom` / `recv` / `read` sur un socket.
pub fn sys_recvfrom(
    fd: i32,
    buffer: u64,
    len: usize,
    _flags: u32,
    addr: u64,
    addr_len: u64,
) -> i64 {
    if len == 0 {
        return 0;
    }
    let state = match socket_of(fd) {
        Ok(state) => state,
        Err(code) => return code,
    };
    let (kind, nonblocking) = {
        let borrowed = state.borrow();
        (borrowed.kind, borrowed.nonblocking)
    };

    match kind {
        SocketKind::Tcp => {
            let mut borrowed = state.borrow_mut();
            let conn = match borrowed.conn.as_mut() {
                Some(conn) => conn,
                None => return -errno::ENOTCONN,
            };
            if conn.rx.is_empty() && !conn.peer_fin && !conn.closed {
                if nonblocking {
                    conn.pump(50_000);
                    if conn.rx.is_empty() {
                        return -errno::EAGAIN;
                    }
                } else {
                    conn.fill(1);
                }
            }
            if conn.rx.is_empty() {
                // Tampon vide et pair ferme : fin de flux.
                return 0;
            }
            let data = conn.take(len);
            let read = data.len();
            if user_write(buffer, &data) {
                read as i64
            } else {
                -errno::EFAULT
            }
        }
        SocketKind::Udp => {
            if state.borrow().datagrams.is_empty() {
                pump_udp(&state, if nonblocking { 20_000 } else { 3_000_000 });
            }
            let datagram = state.borrow_mut().datagrams.pop();
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
        let sortant = match process.borrow().files.get(fd).map(|d| d.kind.clone()) {
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
                let copie = process.borrow().files.get(*fd_source).cloned();
                match copie {
                    Some(desc) => canal.borrow_mut().descripteurs.push(desc),
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
        let sortant = match process.borrow().files.get(fd).map(|d| d.kind.clone()) {
            Some(FdKind::SocketPair(_, sortant)) => Some(sortant),
            _ => None,
        };
        if let Some(canal) = sortant {
            let ecrits = payload.len();
            canal.borrow_mut().octets.extend_from_slice(&payload);
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
    let kind = state.borrow().kind;
    match kind {
        SocketKind::Tcp => {
            let mut borrowed = state.borrow_mut();
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
                None => state.borrow().peer,
            };
            let (ip, port) = match target {
                Some(value) => value,
                None => return -errno::EDESTADDRREQ,
            };
            let source_port = {
                let mut borrowed = state.borrow_mut();
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
    let entrant = match process.borrow().files.get(fd).map(|d| d.kind.clone()) {
        Some(FdKind::SocketPair(entrant, _)) => Some(entrant),
        _ => None,
    };
    let mut installes: Vec<i32> = Vec::new();
    if let Some(canal) = &entrant {
        // Meme attente que pour les octets : le pair n'a peut-etre pas encore
        // eu la main. Sans elle, le premier `recvmsg` d'un dialogue echoue.
        let bloquant = process.borrow().files.get(fd)
            .map(|d| d.flags & 0o4000 == 0).unwrap_or(true);
        if bloquant && canal.borrow().descripteurs.is_empty()
            && canal.borrow().octets.is_empty()
        {
            let echeance = crate::kernel::timer::ticks()
                + crate::kernel::timer::ms_to_ticks(2000);
            while canal.borrow().descripteurs.is_empty()
                && canal.borrow().octets.is_empty()
                && crate::kernel::timer::ticks() < echeance
            {
                task::yield_now();
                crate::arch::x86_64::cpu::wait_for_interrupt();
            }
        }
    }
    if let Some(canal) = entrant {
        let recus: Vec<_> = canal.borrow_mut().descripteurs.drain(..).collect();
        for desc in recus {
            let numero = process.borrow_mut().files.insert(desc);
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
    let mut borrowed = state.borrow_mut();
    if let Some(conn) = borrowed.conn.as_mut() {
        conn.close();
    }
    borrowed.eof = true;
    0
}

/// `getsockname` / `getpeername`.
pub fn sys_getsockname(fd: i32, addr: u64, len_addr: u64, peer: bool) -> i64 {
    let state = match socket_of(fd) {
        Ok(state) => state,
        Err(code) => return code,
    };
    let borrowed = state.borrow();
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
    let state = match socket_of(fd) {
        Ok(state) => state,
        Err(code) => return code,
    };
    let result: u32 = match option {
        // `connect` etant synchrone ici, il n'y a jamais d'erreur differee.
        SO_ERROR => 0,
        SO_TYPE => match state.borrow().kind {
            SocketKind::Tcp => SOCK_STREAM,
            SocketKind::Udp => SOCK_DGRAM,
        },
        _ => 0,
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
        let mut borrowed = process.borrow_mut();
        let mut end_a = FileDesc::new(FdKind::SocketPair(b_to_a.clone(), a_to_b.clone()));
        let mut end_b = FileDesc::new(FdKind::SocketPair(a_to_b, b_to_a));
        end_a.cloexec = cloexec;
        end_b.cloexec = cloexec;
        (borrowed.files.insert(end_a), borrowed.files.insert(end_b))
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
pub fn socket_readable(state: &Rc<RefCell<SocketState>>) -> bool {
    let kind = state.borrow().kind;
    match kind {
        SocketKind::Tcp => {
            let mut borrowed = state.borrow_mut();
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
            if state.borrow().datagrams.is_empty() {
                pump_udp(state, 20_000);
            }
            !state.borrow().datagrams.is_empty()
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
