//! Bascule ring 0 <-> ring 3 : instruction `syscall`, `sysretq`, `iretq`.
//!
//! Trois chemins coexistent :
//!
//! 1. **Entree** par l'instruction `syscall` (chemin rapide, celui que musl
//!    utilise). Le CPU ne sauvegarde rien : il met RIP dans RCX, RFLAGS dans
//!    R11, charge CS/SS depuis `IA32_STAR` et saute a `IA32_LSTAR`. C'est donc
//!    a nous de basculer sur la pile noyau (via `swapgs` + une structure
//!    par-CPU) puis de reconstruire une [`TrapFrame`] complete.
//! 2. **Sortie** par `sysretq`, qui restaure RIP depuis RCX et RFLAGS depuis
//!    R11 : symetrique de l'entree, quelques dizaines de cycles.
//! 3. **Entree/reprise** par `iretq` ([`resume_usermode`]) : utilise au premier
//!    passage en ring 3 et apres un changement de tache, ou `sysret` ne peut
//!    pas servir puisqu'on ne revient pas d'un `syscall`.
//!
//! ## Format de la trame
//!
//! [`TrapFrame`] est dispose exactement comme la trame `iretq` attendue par le
//! CPU : les cinq derniers champs (`rip`, `cs`, `rflags`, `rsp`, `ss`) sont dans
//! l'ordre materiel. Reprendre une tache se reduit donc a `mov rsp, frame` +
//! une serie de `pop` + `iretq`, sans recopie intermediaire.
//!
//! ## Convention GS (invariant)
//!
//! > **En ring 0, `GS_BASE` designe toujours [`PerCpu`] ; en ring 3, il
//! > designe le GS de l'utilisateur.**
//!
//! Chaque franchissement de frontiere fait donc exactement un `swapgs` :
//! entree `syscall`, sortie `sysretq`, `iretq` de [`resume_usermode`], et
//! entree/sortie des gestionnaires d'interruption venus du ring 3 (voir
//! `idt.rs`).
//!
//! Formuler la regle par etat — et non « un swapgs a l'entree, un a la
//! sortie » — n'est pas cosmetique : une tache qui appelle `exit_group` quitte
//! le noyau **sans** repasser par le `sysretq` de son stub d'entree. Avec une
//! regle par paires, son `swapgs` de sortie manquerait et le programme suivant
//! trouverait `GS_BASE = 0` : son premier `mov gs:[8], rsp` ecrirait a
//! l'adresse 8 et ferait paniquer le noyau. Avec l'invariant ci-dessus, chaque
//! chemin sait quoi faire sans rien connaitre du precedent.

use core::arch::{asm, naked_asm};

use crate::arch::x86_64::gdt;

/// MSR `IA32_EFER` : bit 0 = System Call Extensions.
const MSR_EFER: u32 = 0xC000_0080;
/// MSR `IA32_STAR` : selecteurs charges par syscall/sysret.
const MSR_STAR: u32 = 0xC000_0081;
/// MSR `IA32_LSTAR` : adresse du gestionnaire `syscall` 64 bits.
const MSR_LSTAR: u32 = 0xC000_0082;
/// MSR `IA32_FMASK` : bits de RFLAGS effaces a l'entree du syscall.
const MSR_FMASK: u32 = 0xC000_0084;
/// MSR `IA32_FS_BASE` : base du segment FS (TLS de la libc).
pub const MSR_FS_BASE: u32 = 0xC000_0100;
/// MSR `IA32_GS_BASE`.
pub const MSR_GS_BASE: u32 = 0xC000_0101;
/// MSR `IA32_KERNEL_GS_BASE` : valeur echangee par `swapgs`.
pub const MSR_KERNEL_GS_BASE: u32 = 0xC000_0102;

/// Selecteur de code utilisateur (5e entree de la GDT, RPL 3).
pub const USER_CS: u64 = 0x20 | 3;
/// Selecteur de donnees utilisateur (4e entree de la GDT, RPL 3).
pub const USER_SS: u64 = 0x18 | 3;

/// Etat complet d'une tache utilisateur interrompue.
///
/// La disposition **est** un format binaire : le stub d'entree l'empile champ
/// par champ et [`resume_usermode`] la depile. Les cinq derniers champs forment
/// la trame `iretq` materielle et ne doivent etre ni reordonnes ni deplaces.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct TrapFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    // --- trame iretq materielle ---
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

impl TrapFrame {
    /// Trame d'un demarrage de programme : ring 3, interruptions actives.
    pub fn new_user(entry: u64, stack: u64) -> Self {
        TrapFrame {
            rip: entry,
            cs: USER_CS,
            // Bit 1 toujours a 1, IF (bit 9) actif pour que le timer preempte.
            rflags: 0x202,
            rsp: stack,
            ss: USER_SS,
            ..Default::default()
        }
    }

    /// Numero d'appel systeme (RAX) et ses six arguments, convention Linux.
    pub fn syscall_args(&self) -> (u64, [u64; 6]) {
        (self.rax, [self.rdi, self.rsi, self.rdx, self.r10, self.r8, self.r9])
    }
}

/// Structure par-CPU adressee via GS pendant l'entree syscall.
#[repr(C)]
pub struct PerCpu {
    /// Sommet de la pile noyau de la tache courante (offset 0).
    pub kernel_rsp: u64,
    /// Emplacement de sauvegarde du RSP utilisateur (offset 8).
    pub user_rsp: u64,
    /// Tache courante, pour le diagnostic (offset 16).
    pub current: u64,
}

static mut PER_CPU: PerCpu = PerCpu { kernel_rsp: 0, user_rsp: 0, current: 0 };

/// Acces a la structure par-CPU.
pub fn per_cpu() -> &'static mut PerCpu {
    unsafe { &mut *core::ptr::addr_of_mut!(PER_CPU) }
}

/// Definit la pile noyau des entrees depuis le ring 3.
///
/// Deux mecanismes doivent connaitre cette pile : la structure par-CPU (chemin
/// `syscall`, qui charge RSP lui-meme) et RSP0 du TSS (interruptions
/// materielles, ou c'est le CPU qui change de pile).
pub fn set_kernel_stack(top: u64) {
    per_cpu().kernel_rsp = top;
    gdt::set_kernel_stack(top);
}

/// Lit un MSR.
pub fn read_msr(msr: u32) -> u64 {
    let (high, low): (u32, u32);
    unsafe {
        asm!("rdmsr", in("ecx") msr, out("eax") low, out("edx") high, options(nomem, nostack, preserves_flags));
    }
    ((high as u64) << 32) | (low as u64)
}

/// Ecrit un MSR.
///
/// # Securite
/// Ecrire un MSR modifie l'etat du CPU (segments, gestionnaires) : l'appelant
/// doit garantir que la valeur est valide pour ce MSR.
pub unsafe fn write_msr(msr: u32, value: u64) {
    asm!(
        "wrmsr",
        in("ecx") msr,
        in("eax") (value & 0xFFFF_FFFF) as u32,
        in("edx") (value >> 32) as u32,
        options(nomem, nostack, preserves_flags)
    );
}

static mut READY: bool = false;

/// Etat de la couche user-mode, pour les commandes systeme.
pub fn state() -> &'static str {
    if unsafe { READY } {
        "active (syscall/sysretq, ring 3, FS base TLS, SSE)"
    } else {
        "inactive"
    }
}

/// La bascule ring 3 est-elle armee ?
pub fn ready() -> bool {
    unsafe { READY }
}

/// Programme les MSR de `syscall`, active SSE et prepare la structure par-CPU.
pub fn init() {
    let sel = gdt::selectors();
    // Les selecteurs utilisateur sont codes en dur dans le stub d'entree
    // (impossible d'y lire un static sans registre libre) : on verifie ici que
    // la GDT n'a pas bouge.
    assert_eq!(sel.user_code.0 as u64, USER_CS, "gdt: ordre des selecteurs user modifie");
    assert_eq!(sel.user_data.0 as u64, USER_SS, "gdt: ordre des selecteurs user modifie");

    unsafe {
        // SCE : sans ce bit, `syscall` provoque #UD.
        write_msr(MSR_EFER, read_msr(MSR_EFER) | 1);

        // STAR[47:32] = CS noyau ; STAR[63:48] = base des selecteurs user.
        // Avec l'ordre de la GDT : base+8 = donnees user, base+16 = code user.
        let syscall_cs = sel.kernel_code.0 as u64;
        let sysret_base = (sel.kernel_data.0 as u64) | 3;
        write_msr(MSR_STAR, (sysret_base << 48) | (syscall_cs << 32));

        write_msr(MSR_LSTAR, syscall_entry as *const () as usize as u64);

        // Bits de RFLAGS effaces a l'entree : IF (pas d'interruption tant que la
        // pile noyau n'est pas installee), DF (les fonctions memoire supposent
        // DF=0), TF/AC/NT (deroutements parasites depuis le ring 3).
        write_msr(MSR_FMASK, 0x0004_0700);

        // Invariant : en ring 0, GS_BASE = structure par-CPU (voir l'entete).
        // Le noyau demarre en ring 0, l'etat initial est donc celui-la.
        write_msr(MSR_GS_BASE, core::ptr::addr_of!(PER_CPU) as u64);
        write_msr(MSR_KERNEL_GS_BASE, 0);

        per_cpu().kernel_rsp = gdt::kernel_stack();

        enable_sse();
        READY = true;
    }
    crate::kernel::dmesg::log("usermode: syscall/sysret armes (MSR STAR/LSTAR/FMASK), SSE actif");
}

/// Active SSE pour le code utilisateur.
///
/// Le noyau est compile `-sse +soft-float` (cible `x86_64-bouchaud_os`), mais
/// tout binaire compile pour Linux x86-64 utilise SSE des le prologue de
/// `_start` : sans OSFXSR/OSXMMEXCPT, la premiere instruction `movaps` leverait
/// #UD.
unsafe fn enable_sse() {
    use x86_64::registers::control::{Cr0, Cr0Flags, Cr4, Cr4Flags};
    let mut cr0 = Cr0::read();
    cr0.remove(Cr0Flags::EMULATE_COPROCESSOR);
    cr0.insert(Cr0Flags::MONITOR_COPROCESSOR);
    Cr0::write(cr0);
    let mut cr4 = Cr4::read();
    cr4.insert(Cr4Flags::OSFXSR);
    cr4.insert(Cr4Flags::OSXMMEXCPT_ENABLE);
    Cr4::write(cr4);
    asm!("fninit", options(nomem, nostack));
}

/// Definit la base FS du thread courant (TLS de la libc, `arch_prctl`).
pub fn set_fs_base(base: u64) {
    unsafe { write_msr(MSR_FS_BASE, base) };
}

/// Base FS courante.
pub fn fs_base() -> u64 {
    read_msr(MSR_FS_BASE)
}

/// Sauvegarde l'etat FPU/SSE dans une zone `fxsave` (512 octets, alignee 16).
///
/// # Securite
/// `area` doit pointer sur 512 octets alignes sur 16.
pub unsafe fn fxsave(area: *mut u8) {
    asm!("fxsave64 [{}]", in(reg) area, options(nostack));
}

/// Restaure l'etat FPU/SSE depuis une zone `fxsave`.
///
/// # Securite
/// `area` doit pointer sur une zone ecrite par [`fxsave`].
pub unsafe fn fxrstor(area: *const u8) {
    asm!("fxrstor64 [{}]", in(reg) area, options(nostack));
}

/// Echange `GS_BASE` et `IA32_KERNEL_GS_BASE`.
///
/// A n'utiliser qu'aux frontieres ring 0 / ring 3, pour maintenir l'invariant
/// decrit en tete de module.
///
/// # Securite
/// Un `swapgs` non apparie laisse le CPU dans un etat ou toute reference
/// `gs:[...]` du noyau vise la memoire utilisateur.
pub unsafe fn swapgs() {
    asm!("swapgs", options(nomem, nostack, preserves_flags));
}

/// Point d'entree de l'instruction `syscall` (adresse chargee dans LSTAR).
///
/// Reconstruit une [`TrapFrame`] sur la pile noyau, appelle le dispatcher Rust,
/// puis restaure et repart en ring 3 par `sysretq`.
#[unsafe(naked)]
unsafe extern "C" fn syscall_entry() {
    naked_asm!(
        // GS <- structure par-CPU, RSP utilisateur mis de cote.
        "swapgs",
        "mov gs:[8], rsp",
        "mov rsp, gs:[0]",

        // Construction de la TrapFrame : on empile du dernier champ au premier.
        "push {user_ss}",          // ss
        "push qword ptr gs:[8]",   // rsp utilisateur
        "push r11",                // rflags (sauvegarde par syscall)
        "push {user_cs}",          // cs
        "push rcx",                // rip (sauvegarde par syscall)
        "push rax",
        "push rbx",
        "push rcx",
        "push rdx",
        "push rsi",
        "push rdi",
        "push rbp",
        "push r8",
        "push r9",
        "push r10",
        "push r11",
        "push r12",
        "push r13",
        "push r14",
        "push r15",

        // Les interruptions peuvent reprendre : la pile noyau est en place.
        "sti",
        "mov rdi, rsp",
        "call {dispatch}",
        "cli",

        // Restauration miroir.
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "pop rbp",
        "pop rdi",
        "pop rsi",
        "pop rdx",
        "pop rcx",
        "pop rbx",
        "pop rax",
        // rcx/r11 sont detruits par `syscall` cote utilisateur : on peut y
        // remettre RIP et RFLAGS, ce qu'attend `sysretq`.
        "pop rcx",                 // rip
        "add rsp, 8",              // cs (impose par sysretq)
        "pop r11",                 // rflags
        "pop rsp",                 // rsp utilisateur (ss ignore)

        "swapgs",
        "sysretq",
        dispatch = sym syscall_dispatch,
        user_cs = const USER_CS,
        user_ss = const USER_SS,
    )
}

/// Trampoline Rust appele par le stub : delegue a la couche ABI POSIX.
unsafe extern "C" fn syscall_dispatch(frame: *mut TrapFrame) {
    crate::kernel::abi::handle(&mut *frame);
}

/// Reprend (ou demarre) l'execution d'une tache utilisateur.
///
/// Ne revient jamais : le controle repartira par un `syscall` ou une
/// interruption.
///
/// # Securite
/// `frame` doit decrire un etat ring 3 coherent, dont RIP et RSP mappes `USER`
/// dans l'espace d'adressage actif ; la pile noyau ([`set_kernel_stack`]) doit
/// etre installee. Doit etre appelee depuis le ring 0 (invariant GS).
#[unsafe(naked)]
pub unsafe extern "C" fn resume_usermode(frame: *const TrapFrame) -> ! {
    naked_asm!(
        "cli",
        // On quitte le ring 0 : GS repasse cote utilisateur (invariant).
        "swapgs",
        // La trame est dessinee comme la pile attendue par iretq : on l'utilise
        // directement comme pile le temps des `pop`.
        "mov rsp, rdi",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "pop rbp",
        "pop rdi",
        "pop rsi",
        "pop rdx",
        "pop rcx",
        "pop rbx",
        "pop rax",
        "iretq",
    )
}
