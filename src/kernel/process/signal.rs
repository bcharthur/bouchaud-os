//! Signaux POSIX : gestionnaires en ring 3, masques, actions par defaut.
//!
//! Livrer un signal, c'est detourner un programme *entre deux instructions*
//! vers une fonction qu'il a lui-meme installee, puis le remettre exactement
//! ou il en etait. Le noyau n'a aucun moyen d'appeler du code ring 3 : il
//! **fabrique un appel de fonction** en ecrivant sur la pile utilisateur une
//! trame contenant tout l'etat sauvegarde, puis fait repartir la tache sur le
//! gestionnaire. Quand celui-ci retourne, il tombe sur une adresse de retour
//! placee par le noyau (`sa_restorer`), qui appelle `rt_sigreturn` ; le noyau
//! relit alors la trame et restaure l'etat.
//!
//! ## Disposition de la trame
//!
//! La disposition est imposee par l'ABI Linux, pas choisie : le gestionnaire
//! recoit `&ucontext` en troisieme argument et une libc y lit le contexte a des
//! offsets figes.
//!
//! ```text
//!   rsp ->  pretcode   (adresse de retour = sa_restorer)
//!   +8      ucontext   (dont uc_mcontext : les registres sauvegardes)
//!   +944    siginfo
//! ```
//!
//! ## Ou la livraison a lieu
//!
//! Au retour d'un appel systeme, juste avant de rendre la main au ring 3 : la
//! [`TrapFrame`] est alors accessible et modifiable. Un signal envoye a une
//! tache qui calcule sans rien demander au noyau attend donc son prochain appel
//! systeme. En pratique tout programme en appelle constamment ; la seule
//! exception serait une boucle de calcul pur, cas ou Linux lui-meme n'offre
//! aucune garantie de delai.

use alloc::vec::Vec;

use crate::arch::x86_64::usermode::TrapFrame;
use crate::kernel::abi::{user_read, user_write};

// --- Numeros de signaux ------------------------------------------------------

pub const SIGHUP: u32 = 1;
pub const SIGINT: u32 = 2;
pub const SIGQUIT: u32 = 3;
pub const SIGILL: u32 = 4;
pub const SIGABRT: u32 = 6;
pub const SIGFPE: u32 = 8;
pub const SIGKILL: u32 = 9;
pub const SIGSEGV: u32 = 11;
pub const SIGPIPE: u32 = 13;
pub const SIGALRM: u32 = 14;
pub const SIGTERM: u32 = 15;
pub const SIGCHLD: u32 = 17;
pub const SIGCONT: u32 = 18;
pub const SIGSTOP: u32 = 19;
pub const SIGTSTP: u32 = 20;
pub const SIGURG: u32 = 23;
pub const SIGWINCH: u32 = 28;

/// Nombre de signaux geres (1..64).
pub const NSIG: usize = 64;

/// `SIG_DFL` : action par defaut.
pub const SIG_DFL: u64 = 0;
/// `SIG_IGN` : signal ignore.
pub const SIG_IGN: u64 = 1;

/// `sa_flags`
pub const SA_SIGINFO: u64 = 0x0000_0004;
pub const SA_RESTORER: u64 = 0x0400_0000;
pub const SA_RESTART: u64 = 0x1000_0000;
pub const SA_NODEFER: u64 = 0x4000_0000;
pub const SA_RESETHAND: u64 = 0x8000_0000;

/// `how` de `rt_sigprocmask`
pub const SIG_BLOCK: i32 = 0;
pub const SIG_UNBLOCK: i32 = 1;
pub const SIG_SETMASK: i32 = 2;

/// Action associee a un signal, telle que `rt_sigaction` l'installe.
#[derive(Clone, Copy, Default)]
pub struct SigAction {
    /// Adresse du gestionnaire, ou `SIG_DFL` / `SIG_IGN`.
    pub handler: u64,
    pub flags: u64,
    /// Trampoline de retour fourni par la libc (`SA_RESTORER`).
    pub restorer: u64,
    /// Signaux bloques pendant l'execution du gestionnaire.
    pub mask: u64,
}

/// Etat des signaux d'un processus.
#[derive(Clone)]
pub struct SignalState {
    pub actions: [SigAction; NSIG],
    /// Signaux en attente de livraison (bit n-1 pour le signal n).
    pub pending: u64,
    /// Signaux bloques par `rt_sigprocmask`.
    pub blocked: u64,
}

impl Default for SignalState {
    fn default() -> Self {
        SignalState { actions: [SigAction::default(); NSIG], pending: 0, blocked: 0 }
    }
}

impl SignalState {
    /// Remet toutes les actions a leur valeur par defaut, en conservant les
    /// signaux ignores — c'est le comportement d'`execve`.
    pub fn reset_for_exec(&mut self) {
        for action in self.actions.iter_mut() {
            if action.handler != SIG_IGN {
                *action = SigAction::default();
            } else {
                action.flags = 0;
                action.restorer = 0;
                action.mask = 0;
            }
        }
        self.pending = 0;
    }

    /// Marque un signal en attente.
    pub fn raise(&mut self, signal: u32) {
        if signal >= 1 && (signal as usize) <= NSIG {
            self.pending |= 1 << (signal - 1);
        }
    }

    /// Premier signal livrable (non bloque), s'il y en a un.
    pub fn next_deliverable(&self) -> Option<u32> {
        let ready = self.pending & !self.blocked;
        if ready == 0 {
            return None;
        }
        // SIGKILL et SIGSTOP ne sont jamais bloquables : ils sortent en tete.
        for signal in [SIGKILL, SIGSTOP] {
            if ready & (1 << (signal - 1)) != 0 {
                return Some(signal);
            }
        }
        Some(ready.trailing_zeros() + 1)
    }

    /// Retire un signal de la file d'attente.
    pub fn clear(&mut self, signal: u32) {
        self.pending &= !(1 << (signal - 1));
    }
}

/// Que faire d'un signal sans gestionnaire installe.
pub enum DefaultAction {
    /// Termine le processus.
    Terminate,
    /// N'a aucun effet.
    Ignore,
}

/// Action par defaut d'un signal.
pub fn default_action(signal: u32) -> DefaultAction {
    match signal {
        SIGCHLD | SIGURG | SIGWINCH | SIGCONT => DefaultAction::Ignore,
        // Les signaux d'arret n'ont pas de sens sans controle de tache : on les
        // ignore plutot que de geler un processus que rien ne relancerait.
        SIGSTOP | SIGTSTP => DefaultAction::Ignore,
        _ => DefaultAction::Terminate,
    }
}

/// Nom court d'un signal (diagnostic).
pub fn name(signal: u32) -> &'static str {
    match signal {
        SIGHUP => "SIGHUP",
        SIGINT => "SIGINT",
        SIGQUIT => "SIGQUIT",
        SIGILL => "SIGILL",
        SIGABRT => "SIGABRT",
        SIGFPE => "SIGFPE",
        SIGKILL => "SIGKILL",
        SIGSEGV => "SIGSEGV",
        SIGPIPE => "SIGPIPE",
        SIGALRM => "SIGALRM",
        SIGTERM => "SIGTERM",
        SIGCHLD => "SIGCHLD",
        SIGCONT => "SIGCONT",
        SIGSTOP => "SIGSTOP",
        SIGWINCH => "SIGWINCH",
        _ => "SIG?",
    }
}

// --- Disposition binaire de la trame ----------------------------------------

/// Taille reservee a `ucontext_t` : celle de musl et de la glibc, zone de
/// registres flottants comprise.
const UCONTEXT_SIZE: u64 = 936;
/// Decalage de `uc_mcontext` dans `ucontext_t`.
const UC_MCONTEXT: u64 = 40;
/// Decalage de `uc_sigmask` dans `ucontext_t`.
const UC_SIGMASK: u64 = UC_MCONTEXT + 256;
/// Taille de la trame complete : pretcode + ucontext + siginfo.
const SIGFRAME_SIZE: u64 = 8 + UCONTEXT_SIZE + 128;

/// Champs de `struct sigcontext`, en octets depuis son debut.
const SC_R8: u64 = 0;
const SC_R9: u64 = 8;
const SC_R10: u64 = 16;
const SC_R11: u64 = 24;
const SC_R12: u64 = 32;
const SC_R13: u64 = 40;
const SC_R14: u64 = 48;
const SC_R15: u64 = 56;
const SC_RDI: u64 = 64;
const SC_RSI: u64 = 72;
const SC_RBP: u64 = 80;
const SC_RBX: u64 = 88;
const SC_RDX: u64 = 96;
const SC_RAX: u64 = 104;
const SC_RCX: u64 = 112;
const SC_RSP: u64 = 120;
const SC_RIP: u64 = 128;
const SC_EFLAGS: u64 = 136;
const SC_CS: u64 = 144;

/// Ecrit la trame de signal sur la pile utilisateur et detourne la tache vers
/// son gestionnaire.
///
/// Renvoie `false` si la pile utilisateur n'est pas accessible en ecriture —
/// auquel cas le processus est irrecuperable et doit etre tue.
pub fn deliver(frame: &mut TrapFrame, signal: u32, action: &SigAction, old_mask: u64) -> bool {
    // Zone rouge de l'ABI System V : 128 octets sous rsp qu'une fonction feuille
    // peut utiliser sans les reserver. Les ecraser corromprait le programme.
    let mut sp = frame.rsp - 128;
    sp -= SIGFRAME_SIZE;
    // A l'entree d'une fonction, rsp+8 doit etre aligne sur 16. Le noyau simule
    // un `call` en deposant l'adresse de retour : rsp doit donc valoir 8 modulo
    // 16, exactement comme apres un vrai `call`.
    sp = (sp & !0xF) - 8;

    let uc = sp + 8;
    let mcontext = uc + UC_MCONTEXT;
    let siginfo = uc + UCONTEXT_SIZE;

    // Remise a zero : le gestionnaire peut lire des champs qu'on ne renseigne
    // pas (uc_link, pile alternative), ils doivent etre nuls et non aleatoires.
    let zeros = alloc::vec![0u8; SIGFRAME_SIZE as usize];
    if !user_write(sp, &zeros) {
        return false;
    }

    let put = |offset: u64, value: u64| user_write(offset, &value.to_le_bytes());

    // Adresse de retour : le trampoline de la libc, qui appellera rt_sigreturn.
    let mut ok = put(sp, action.restorer);

    // uc_mcontext : l'integralite de l'etat interrompu.
    ok &= put(mcontext + SC_R8, frame.r8);
    ok &= put(mcontext + SC_R9, frame.r9);
    ok &= put(mcontext + SC_R10, frame.r10);
    ok &= put(mcontext + SC_R11, frame.r11);
    ok &= put(mcontext + SC_R12, frame.r12);
    ok &= put(mcontext + SC_R13, frame.r13);
    ok &= put(mcontext + SC_R14, frame.r14);
    ok &= put(mcontext + SC_R15, frame.r15);
    ok &= put(mcontext + SC_RDI, frame.rdi);
    ok &= put(mcontext + SC_RSI, frame.rsi);
    ok &= put(mcontext + SC_RBP, frame.rbp);
    ok &= put(mcontext + SC_RBX, frame.rbx);
    ok &= put(mcontext + SC_RDX, frame.rdx);
    ok &= put(mcontext + SC_RAX, frame.rax);
    ok &= put(mcontext + SC_RCX, frame.rcx);
    ok &= put(mcontext + SC_RSP, frame.rsp);
    ok &= put(mcontext + SC_RIP, frame.rip);
    ok &= put(mcontext + SC_EFLAGS, frame.rflags);
    ok &= put(mcontext + SC_CS, frame.cs);

    // uc_sigmask : le masque a restaurer au retour du gestionnaire.
    ok &= put(uc + UC_SIGMASK, old_mask);

    // siginfo : le minimum exploitable (numero, code SI_USER).
    ok &= put(siginfo, signal as u64);
    ok &= put(siginfo + 8, 0);

    if !ok {
        return false;
    }

    // Detournement : le gestionnaire recoit (signum, &siginfo, &ucontext).
    frame.rsp = sp;
    frame.rip = action.handler;
    frame.rdi = signal as u64;
    frame.rsi = siginfo;
    frame.rdx = uc;
    // L'ABI n'exige pas de valeur particuliere pour rax, mais une libc peut
    // supposer que les registres variadiques sont coherents : 0 = pas de
    // registre vectoriel utilise.
    frame.rax = 0;
    true
}

/// `rt_sigreturn` : restaure l'etat sauvegarde par [`deliver`].
///
/// Renvoie le masque de signaux a retablir, ou `None` si la trame est
/// illisible.
pub fn restore(frame: &mut TrapFrame) -> Option<u64> {
    // Le trampoline a depile l'adresse de retour : rsp designe `ucontext`.
    let uc = frame.rsp;
    let mcontext = uc + UC_MCONTEXT;

    let read = |offset: u64| -> Option<u64> {
        let bytes = user_read(offset, 8)?;
        let mut value = [0u8; 8];
        value.copy_from_slice(&bytes);
        Some(u64::from_le_bytes(value))
    };

    frame.r8 = read(mcontext + SC_R8)?;
    frame.r9 = read(mcontext + SC_R9)?;
    frame.r10 = read(mcontext + SC_R10)?;
    frame.r11 = read(mcontext + SC_R11)?;
    frame.r12 = read(mcontext + SC_R12)?;
    frame.r13 = read(mcontext + SC_R13)?;
    frame.r14 = read(mcontext + SC_R14)?;
    frame.r15 = read(mcontext + SC_R15)?;
    frame.rdi = read(mcontext + SC_RDI)?;
    frame.rsi = read(mcontext + SC_RSI)?;
    frame.rbp = read(mcontext + SC_RBP)?;
    frame.rbx = read(mcontext + SC_RBX)?;
    frame.rdx = read(mcontext + SC_RDX)?;
    frame.rax = read(mcontext + SC_RAX)?;
    frame.rcx = read(mcontext + SC_RCX)?;
    frame.rsp = read(mcontext + SC_RSP)?;
    frame.rip = read(mcontext + SC_RIP)?;
    // On ne restaure que les drapeaux modifiables depuis le ring 3, et on
    // impose IF : un gestionnaire ne doit pas pouvoir revenir avec les
    // interruptions masquees.
    let flags = read(mcontext + SC_EFLAGS)?;
    frame.rflags = (flags & 0x0000_08D5) | 0x202;

    read(uc + UC_SIGMASK)
}

/// Lit une `struct sigaction` (disposition noyau) depuis l'espace utilisateur.
pub fn read_sigaction(addr: u64) -> Option<SigAction> {
    let bytes = user_read(addr, 32)?;
    let word = |i: usize| {
        let mut value = [0u8; 8];
        value.copy_from_slice(&bytes[i * 8..i * 8 + 8]);
        u64::from_le_bytes(value)
    };
    Some(SigAction { handler: word(0), flags: word(1), restorer: word(2), mask: word(3) })
}

/// Ecrit une `struct sigaction` vers l'espace utilisateur.
pub fn write_sigaction(addr: u64, action: &SigAction) -> bool {
    let mut bytes = Vec::with_capacity(32);
    bytes.extend_from_slice(&action.handler.to_le_bytes());
    bytes.extend_from_slice(&action.flags.to_le_bytes());
    bytes.extend_from_slice(&action.restorer.to_le_bytes());
    bytes.extend_from_slice(&action.mask.to_le_bytes());
    user_write(addr, &bytes)
}
