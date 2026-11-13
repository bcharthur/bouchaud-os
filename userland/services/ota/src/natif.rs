//! Les appels systeme dont l'agent a besoin hors de la bibliotheque standard :
//! le canal de controle de `serviced` (ABI native Bouchaud), `poll` (le noyau
//! ignore SO_RCVTIMEO : sans lui, une lecture sur un lien coupe ne rendrait
//! jamais la main) et `getrandom`.
use core::arch::asm;

const NATIVE: u64 = 0x424f_0000;
const CHANNEL_SEND: u64 = NATIVE + 0x11;
const CHANNEL_RECV: u64 = NATIVE + 0x12;
const WAITSET_CREATE: u64 = NATIVE + 0x30;
const WAITSET_ADD: u64 = NATIVE + 0x31;
const WAITSET_WAIT: u64 = NATIVE + 0x34;

const E_WOULD_BLOCK: i64 = 6;
const E_PEER_CLOSED: i64 = 9;
const E_TIMED_OUT: i64 = 14;

const SYS_POLL: u64 = 7;
const SYS_GETRANDOM: u64 = 318;

#[inline(always)]
unsafe fn sys6(nr: u64, a: u64, b: u64, c: u64, d: u64, e: u64, f: u64) -> i64 {
    let ret: i64;
    asm!("syscall",
         inlateout("rax") nr as i64 => ret,
         in("rdi") a, in("rsi") b, in("rdx") c, in("r10") d, in("r8") e, in("r9") f,
         lateout("rcx") _, lateout("r11") _,
         options(nostack));
    ret
}

/// `getrandom` du noyau (son CSPRNG). Ne rend jamais d'octets non tires :
/// un echec est fatal, jamais des zeros.
pub fn aleatoire(out: &mut [u8]) {
    let mut fait = 0;
    while fait < out.len() {
        let r = unsafe { sys6(SYS_GETRANDOM, out[fait..].as_mut_ptr() as u64, (out.len() - fait) as u64, 0, 0, 0, 0) };
        if r <= 0 {
            println!("BOUCHAUD_OTA_ALEATOIRE_ABSENT rc={}", r);
            std::process::abort();
        }
        fait += r as usize;
    }
}

/// Attend que `fd` soit lisible, au plus `ms`. Vrai : lisible (ou ferme).
pub fn attend_lisible(fd: i32, ms: i32) -> bool {
    #[repr(C)]
    struct PollFd { fd: i32, events: i16, revents: i16 }
    let mut p = PollFd { fd, events: 0x001 /* POLLIN */, revents: 0 };
    let r = unsafe { sys6(SYS_POLL, &mut p as *mut PollFd as u64, 1, ms as u64, 0, 0, 0) };
    r > 0
}

#[repr(C)]
struct WaitEvent { key: u64, signals: u32, reserved: u32 }
#[repr(C)]
struct RecvMeta { bytes: u64, handles: u64 }

/// Le premier handle d'amorcage (`BO_BOOTSTRAP=<id>,...`) : le canal de
/// controle que `serviced` a donne. Absent : l'agent a ete lance a la main.
pub fn canal_de_controle() -> Option<u64> {
    let v = std::env::var("BO_BOOTSTRAP").ok()?;
    v.split(',').next()?.trim().parse().ok()
}

/// Le protocole de `serviced` (comme healthd) : hello -> ready, ping -> pong,
/// stop -> bye puis sortie. Tourne dans son propre fil : un telechargement
/// lent ne fait jamais manquer un battement.
pub fn controle(h: u64) -> ! {
    let ws = unsafe { sys6(WAITSET_CREATE, 0, 0, 0, 0, 0, 0) };
    if ws < 0 || unsafe { sys6(WAITSET_ADD, ws as u64, h, 1, 0, 0, 0) } != 0 {
        println!("BOUCHAUD_OTA_CONTROLE_ECHEC etape=waitset rc={}", ws);
        std::process::exit(3);
    }
    let mut ev = [WaitEvent { key: 0, signals: 0, reserved: 0 }, WaitEvent { key: 0, signals: 0, reserved: 0 }];
    let mut entree = [0u8; 128];
    loop {
        let n = unsafe { sys6(WAITSET_WAIT, ws as u64, ev.as_mut_ptr() as u64, 2, 5_000_000_000, 0, 0) };
        if n == -E_TIMED_OUT || n == 0 { continue; }
        if n < 0 { std::process::exit(4); }
        let mut meta = RecvMeta { bytes: 0, handles: 0 };
        let lu = unsafe { sys6(CHANNEL_RECV, h, entree.as_mut_ptr() as u64, entree.len() as u64, 0, 0, &mut meta as *mut RecvMeta as u64) };
        if lu == -E_PEER_CLOSED { std::process::exit(0); }
        if lu == -E_WOULD_BLOCK { continue; }
        if lu < 0 { std::process::exit(5); }
        let m = &entree[..lu as usize];
        let reponse: Vec<u8> = if let Some(r) = m.strip_prefix(b"ping ") {
            [b"pong ".as_slice(), r].concat()
        } else if let Some(r) = m.strip_prefix(b"hello ") {
            [b"ready ".as_slice(), r].concat()
        } else if m.starts_with(b"stop") {
            let _ = unsafe { sys6(CHANNEL_SEND, h, b"bye".as_ptr() as u64, 3, 0, 0, 0) };
            std::process::exit(0);
        } else {
            b"error".to_vec()
        };
        if unsafe { sys6(CHANNEL_SEND, h, reponse.as_ptr() as u64, reponse.len() as u64, 0, 0, 0) } < 0 {
            std::process::exit(0);
        }
    }
}
