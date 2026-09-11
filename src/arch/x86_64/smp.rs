//! SMP x86_64 reel pour Bouchaud OS.
//!
//! Le BSP prepare un trampoline real-mode a 0x8000, reveille les AP par
//! INIT/SIPI, puis chaque AP rejoint le noyau 64 bits avec la PML4 du BSP.
//! Les AP chargent ensuite leur GDT/TSS, IDT, GS per-CPU et MSR syscall avant
//! d'entrer dans la boucle secondaire du scheduler.
//!
//! Strategie du premier scheduler SMP : affinite par processus. Tous les threads
//! d'un meme espace d'adressage restent sur le meme CPU, ce qui evite de rendre
//! obligatoires les TLB shootdowns dans ce jalon tout en parallelisant les
//! processus Ladybird independants (BrowserHost, WebContent, Compositor,
//! RequestServer, ImageDecoder...).

use core::arch::{asm, x86_64::__cpuid};
use core::hint::spin_loop;
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use crate::arch::x86_64::{cpu_local, gdt, idt, interrupts, usermode};
use crate::kernel::{dmesg, memory, vmm};

pub const MAX_CPUS: usize = 16;
pub const RESCHEDULE_VECTOR: u8 = 0xF1;
pub const TLB_SHOOTDOWN_VECTOR: u8 = 0xF2;
pub const SCHED_QUANTUM_TICKS: u64 = 4;

const IA32_APIC_BASE: u32 = 0x1B;
const APIC_ENABLE: u64 = 1 << 11;
const X2APIC_ENABLE: u64 = 1 << 10;
const X2APIC_EOI: u32 = 0x80B;
const X2APIC_SVR: u32 = 0x80F;
const X2APIC_ICR: u32 = 0x830;
const X2APIC_LVT_TIMER: u32 = 0x832;
const X2APIC_TIMER_INITIAL: u32 = 0x838;
const X2APIC_TIMER_CURRENT: u32 = 0x839;
const X2APIC_TIMER_DIVIDE: u32 = 0x83E;
const IA32_TSC_DEADLINE: u32 = 0x6E0;

const LAPIC_EOI: usize = 0xB0;
const LAPIC_SVR: usize = 0xF0;
const LAPIC_ICR_LOW: usize = 0x300;
const LAPIC_ICR_HIGH: usize = 0x310;
const LAPIC_LVT_TIMER: usize = 0x320;
const LAPIC_TIMER_INITIAL: usize = 0x380;
const LAPIC_TIMER_CURRENT: usize = 0x390;
const LAPIC_TIMER_DIVIDE: usize = 0x3E0;

/// Diviseur du timer local : seize.
///
/// Un diviseur de un ferait deborder le compteur trente-deux bits pendant la
/// calibration sur un bus rapide ; seize laisse une marge confortable sans
/// rendre le quantum granuleux.
const LAPIC_DIVISEUR_16: u32 = 0b0011;

/// Bit de masquage d'une entree LVT.
const LVT_MASQUE: u32 = 1 << 16;
/// Mode periodique : bits 18:17 = 01.
const LVT_PERIODIQUE: u32 = 1 << 17;
/// Mode TSC-deadline : bits 18:17 = 10.
const LVT_TSC_DEADLINE: u32 = 2 << 17;

/// Comment le quantum local est arme sur CE coeur.
///
/// 0 = aucun, 1 = TSC-deadline, 2 = LAPIC periodique.
static MODE_TIMER_LOCAL: core::sync::atomic::AtomicU8 =
    core::sync::atomic::AtomicU8::new(0);
const MODE_AUCUN: u8 = 0;
const MODE_TSC_DEADLINE: u8 = 1;
const MODE_LAPIC_PERIODIQUE: u8 = 2;

/// Compte initial du timer local, calibre une fois par le premier coeur.
///
/// La frequence du bus APIC est la meme pour tous les coeurs d'un paquet : la
/// calibrer par coeur multiplierait un travail identique par seize, et chaque
/// calibration coute une fenetre d'attente.
static LAPIC_COMPTE_QUANTUM: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);
static LAPIC_HZ: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

const TRAMPOLINE_PHYS: u64 = 0x8000;
const MAILBOX_PHYS: u64 = 0x9000;
const BOOT_PML4_PHYS: u64 = 0xA000;
const SIPI_VECTOR: u32 = (TRAMPOLINE_PHYS >> 12) as u32;
const AP_STACK_SIZE: usize = 64 * 1024;

#[repr(align(16))]
#[derive(Clone, Copy)]
struct ApStack([u8; AP_STACK_SIZE]);

static mut AP_STACKS: [ApStack; MAX_CPUS] = [ApStack([0; AP_STACK_SIZE]); MAX_CPUS];

// BOUCHAUD_SMP_PHYSICAL_TICKET_V3
// Trampoline .code16/.code64 a 0x8000. Aucun APIC ID n'est utilise comme
// indice de tableau avant Rust : un `lock xadd` attribue un stack bootstrap.
// Mailbox :
//   +0x00 u32 CR3 bootstrap bas (copie PML4 a 0xA000)
//   +0x08 u64 ap_entry
//   +0x10 u32 next_stack_ticket
//   +0x18 u64 CR3 BSP reel (64 bits)
//   +0x20 u64 stack_top[16]
const TRAMPOLINE: [u8; 174] = [
    0xFA, 0xFC, 0x31, 0xC0, 0x8E, 0xD8, 0x8E, 0xC0, 0x8E, 0xD0, 0x0F, 0x01, 0x16, 0xA8, 0x80, 0x0F,
    0x20, 0xE0, 0x66, 0x83, 0xC8, 0x20, 0x0F, 0x22, 0xE0, 0x66, 0xA1, 0x00, 0x90, 0x0F, 0x22, 0xD8,
    0x66, 0xB9, 0x80, 0x00, 0x00, 0xC0, 0x0F, 0x32, 0x66, 0x0D, 0x00, 0x09, 0x00, 0x00, 0x0F, 0x30,
    0x0F, 0x20, 0xC0, 0x66, 0x0D, 0x01, 0x00, 0x00, 0x80, 0x0F, 0x22, 0xC0, 0x66, 0xEA, 0x44, 0x80,
    0x00, 0x00, 0x08, 0x00, 0x66, 0xB8, 0x10, 0x00, 0x8E, 0xD8, 0x8E, 0xC0, 0x8E, 0xD0, 0x48, 0x8B,
    0x14, 0x25, 0x18, 0x90, 0x00, 0x00, 0x0F, 0x22, 0xDA, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xF0, 0x0F,
    0xC1, 0x04, 0x25, 0x10, 0x90, 0x00, 0x00, 0x83, 0xF8, 0x0F, 0x77, 0x1B, 0x48, 0x8B, 0x24, 0xC5,
    0x20, 0x90, 0x00, 0x00, 0x48, 0x85, 0xE4, 0x74, 0x0E, 0x48, 0x83, 0xE4, 0xF0, 0x48, 0x8B, 0x04,
    0x25, 0x08, 0x90, 0x00, 0x00, 0xFF, 0xD0, 0xFA, 0xF4, 0xEB, 0xFD, 0x0F, 0x1F, 0x44, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x9A, 0xAF, 0x00,
    0xFF, 0xFF, 0x00, 0x00, 0x00, 0x92, 0xCF, 0x00, 0x17, 0x00, 0x90, 0x80, 0x00, 0x00,
];

static DISCOVERED: AtomicUsize = AtomicUsize::new(1);
static ONLINE_CPUS: AtomicUsize = AtomicUsize::new(1);
static ONLINE_MASK: AtomicUsize = AtomicUsize::new(1);
static SCHEDULER_ENABLED: AtomicBool = AtomicBool::new(false);
static LOCAL_SCHED_TIMER: AtomicBool = AtomicBool::new(false);

// BOUCHAUD_SMP_NG3_TLB_SHOOTDOWN_V2
// Une slot par CPU emetteur remplace la mailbox globale NG2. Un CPU ne peut
// avoir qu'un shootdown synchrone en vol (le noyau n'est pas preemptible), mais
// tous les CPU peuvent publier simultanement sans ecraser leurs parametres.
// `sequence == 0` signifie libre; les autres valeurs sont des generations.
struct TlbSlot {
    sequence: AtomicU64,
    pml4: AtomicU64,
    start: AtomicU64,
    len: AtomicU64,
    targets: AtomicU64,
    acknowledgements: AtomicU64,
}

impl TlbSlot {
    const fn new() -> Self {
        Self {
            sequence: AtomicU64::new(0),
            pml4: AtomicU64::new(0),
            start: AtomicU64::new(0),
            len: AtomicU64::new(0),
            targets: AtomicU64::new(0),
            acknowledgements: AtomicU64::new(0),
        }
    }
}

static TLB_SLOTS: [TlbSlot; MAX_CPUS] = [const { TlbSlot::new() }; MAX_CPUS];
static TLB_NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static TLB_SHOOTDOWN_COUNT: AtomicU64 = AtomicU64::new(0);

/// Best APIC identifier exposed by CPUID. It is an identifier, never a runtime
/// array index after SMP-NG1.
pub fn hardware_apic_id() -> usize {
    cpu_local::hardware_apic_id() as usize
}

/// Dense Bouchaud logical CPU index.
///
/// GS carries the logical CpuId after usermode::init_cpu(). The APIC fallback is
/// used only during early bring-up and is translated through the CPU registry.
pub fn cpu_index() -> usize {
    if let Some(via_gs) = usermode::cpu_index_from_gs() {
        return via_gs;
    }

    cpu_local::logical_for_apic(hardware_apic_id() as u32)
        .map(|id| id.as_usize())
        .unwrap_or(0)
}

pub fn discovered_cpus() -> usize {
    DISCOVERED.load(Ordering::Acquire)
}

pub fn started_aps() -> usize {
    ONLINE_CPUS.load(Ordering::Acquire).saturating_sub(1)
}

pub fn schedulable_cpus() -> usize {
    ONLINE_CPUS.load(Ordering::Acquire).max(1).min(MAX_CPUS)
}

pub fn is_online(cpu: usize) -> bool {
    cpu < MAX_CPUS && ONLINE_MASK.load(Ordering::Acquire) & (1usize << cpu) != 0
}

pub fn scheduler_enabled() -> bool {
    SCHEDULER_ENABLED.load(Ordering::Acquire)
}

fn spin_delay(iterations: usize) {
    for _ in 0..iterations {
        spin_loop();
    }
}

unsafe fn lapic_read(base: *mut u8, offset: usize) -> u32 {
    read_volatile(base.add(offset) as *const u32)
}

unsafe fn lapic_write(base: *mut u8, offset: usize, value: u32) {
    write_volatile(base.add(offset) as *mut u32, value);
}

unsafe fn wait_xapic_delivery(base: *mut u8) {
    for _ in 0..1_000_000 {
        if lapic_read(base, LAPIC_ICR_LOW) & (1 << 12) == 0 {
            return;
        }
        spin_loop();
    }
}

unsafe fn local_apic() -> (bool, *mut u8) {
    let mut apic_base = usermode::read_msr(IA32_APIC_BASE);
    if apic_base & APIC_ENABLE == 0 {
        apic_base |= APIC_ENABLE;
        usermode::write_msr(IA32_APIC_BASE, apic_base);
    }
    let x2 = apic_base & X2APIC_ENABLE != 0;
    let phys = apic_base & 0x000f_ffff_ffff_f000;
    (x2, memory::phys_to_virt(phys))
}

unsafe fn enable_local_apic() {
    let (x2, lapic) = local_apic();
    if x2 {
        let svr = usermode::read_msr(X2APIC_SVR);
        usermode::write_msr(X2APIC_SVR, svr | 0x100 | 0xFF);
    } else {
        let svr = lapic_read(lapic, LAPIC_SVR);
        lapic_write(lapic, LAPIC_SVR, svr | 0x100 | 0xFF);
    }
}

unsafe fn send_all_excluding_self(low: u32) {
    let (x2, lapic) = local_apic();
    let low = low | (3 << 18); // shorthand: all excluding self
    if x2 {
        usermode::write_msr(X2APIC_ICR, low as u64);
    } else {
        lapic_write(lapic, LAPIC_ICR_HIGH, 0);
        lapic_write(lapic, LAPIC_ICR_LOW, low);
        wait_xapic_delivery(lapic);
    }
}

/// Renvoie le vecteur de shootdown a UN seul CPU.
///
/// `send_all_excluding_self` est une diffusion, envoyee une seule fois. Elle
/// suppose que chaque envoi produise un passage distinct dans le handler. Ce
/// contrat n'est pas garanti face a une livraison APIC retardee ou a plusieurs
/// occurrences pendantes du meme vecteur. Reemettre demande de viser
/// precisement ceux qui manquent, comme le fait `reschedule_cpu`.
fn renvoie_shootdown(cpu: usize) {
    if cpu >= schedulable_cpus() || cpu == cpu_index() {
        return;
    }
    let Some(id) = cpu_local::CpuId::from_index(cpu) else { return; };
    let Some(target) = cpu_local::descriptor(id) else { return; };
    unsafe {
        let (x2, lapic) = local_apic();
        if x2 {
            usermode::write_msr(
                X2APIC_ICR,
                ((target.apic_id as u64) << 32) | TLB_SHOOTDOWN_VECTOR as u64,
            );
        } else {
            lapic_write(lapic, LAPIC_ICR_HIGH, (target.legacy_apic_id as u32) << 24);
            lapic_write(lapic, LAPIC_ICR_LOW, TLB_SHOOTDOWN_VECTOR as u32);
            wait_xapic_delivery(lapic);
        }
    }
}

/// Shootdowns ayant exige au moins une reemission.
static TLB_RELANCES: AtomicU64 = AtomicU64::new(0);
/// IPI de shootdown reemis, tous CPU confondus.
static TLB_IPI_REEMIS: AtomicU64 = AtomicU64::new(0);
/// Shootdowns arretes fail-closed apres absence persistante d'ACK.
static TLB_ECHECS: AtomicU64 = AtomicU64::new(0);

pub fn tlb_relances() -> (u64, u64) {
    (
        TLB_RELANCES.load(Ordering::Relaxed),
        TLB_IPI_REEMIS.load(Ordering::Relaxed),
    )
}

pub fn eoi_local() {
    unsafe {
        let (x2, lapic) = local_apic();
        if x2 {
            usermode::write_msr(X2APIC_EOI, 0);
        } else {
            lapic_write(lapic, LAPIC_EOI, 0);
        }
    }
}

/// Reveille les CPU secondaires et leur demande un point d'ordonnancement.
/// Vecteur d'arret de panique : le CPU qui le recoit s'arrete definitivement.
/// Delai avant de reemettre un IPI de shootdown a un CPU qui n'a pas encore
/// acquitte. Assez long pour qu'un acquittement normal arrive largement
/// avant, assez court pour qu'un IPI perdu ne coute qu'un sursaut.
const RELANCE_SHOOTDOWN_NS: u64 = 2_000_000;
/// Une traduction perimee ne doit jamais etre acceptee. Apres cette borne on
/// panique donc explicitement au lieu de boucler pour toujours ou de continuer.
const ECHEC_SHOOTDOWN_NS: u64 = 2_000_000_000;

pub const PANIC_STOP_VECTOR: u8 = 0xF3;

/// Arrete tous les autres CPU apres une faute fatale.
///
/// # Pourquoi un vecteur dedie
///
/// Le vecteur de replanification prend le gros verrou et peut commuter : c'est
/// exactement ce qu'il ne faut pas faire quand l'etat du noyau est douteux. Ce
/// vecteur-ci ne fait rien d'autre que `cli; hlt`.
///
/// Un seul envoi, en diffusion. Pas de boucle d'attente, pas d'accuse : un CPU
/// qui ne repond pas est deja perdu, et l'attendre empecherait la sortie de
/// panique de partir.
pub fn arrete_les_autres_cpu() {
    unsafe { send_all_excluding_self(PANIC_STOP_VECTOR as u32) };
}

pub fn broadcast_reschedule() {
    if schedulable_cpus() <= 1 {
        return;
    }
    unsafe { send_all_excluding_self(RESCHEDULE_VECTOR as u32) };
}

/// Wake one logical CPU. Normal task wakeups use this path; broadcasts are
/// reserved for exceptional machine-wide state changes.
pub fn reschedule_cpu(cpu: usize) {
    if cpu >= schedulable_cpus() || cpu == cpu_index() {
        return;
    }
    let Some(id) = cpu_local::CpuId::from_index(cpu) else { return; };
    let Some(target) = cpu_local::descriptor(id) else { return; };
    unsafe {
        let (x2, lapic) = local_apic();
        if x2 {
            usermode::write_msr(
                X2APIC_ICR,
                ((target.apic_id as u64) << 32) | RESCHEDULE_VECTOR as u64,
            );
        } else {
            lapic_write(lapic, LAPIC_ICR_HIGH, (target.legacy_apic_id as u32) << 24);
            lapic_write(lapic, LAPIC_ICR_LOW, RESCHEDULE_VECTOR as u32);
            wait_xapic_delivery(lapic);
        }
    }
}

fn tsc_deadline_supported() -> bool {
    __cpuid(1).ecx & (1 << 24) != 0 && crate::kernel::timer::tsc_hz().is_some()
}

/// Ecrit un registre du timer local, quel que soit le mode APIC.
unsafe fn timer_local_ecrit(x2: bool, lapic: *mut u8, msr: u32, offset: usize, valeur: u32) {
    if x2 {
        usermode::write_msr(msr, valeur as u64);
    } else {
        lapic_write(lapic, offset, valeur);
    }
}

/// Lit le compteur courant du timer local.
unsafe fn timer_local_courant(x2: bool, lapic: *mut u8) -> u32 {
    if x2 {
        usermode::read_msr(X2APIC_TIMER_CURRENT) as u32
    } else {
        lapic_read(lapic, LAPIC_TIMER_CURRENT)
    }
}

/// Mesure la frequence du timer local en la comparant au TSC.
///
/// # Pourquoi contre le TSC et pas contre le PIT
///
/// Le PIT n'existe que sur un coeur -- c'est tout le probleme qu'on corrige
/// ici. Le TSC, lui, est lisible depuis n'importe quel coeur et sa frequence
/// est deja etablie au demarrage (`BOUCHAUD_TSC_EARLY_CALIBRATION_OK`).
///
/// Le timer est MASQUE pendant la mesure : une interruption de quantum au
/// milieu d'une calibration fausserait la calibration ET arriverait avant que
/// l'ordonnanceur ne soit pret a la recevoir.
fn calibre_timer_local() -> Option<u32> {
    let tsc_hz = crate::kernel::timer::tsc_hz()?;
    // Dix millisecondes : assez long pour que la granularite du diviseur
    // disparaisse, assez court pour ne pas retarder l'amorcage.
    let cycles_fenetre = tsc_hz / 100;
    if cycles_fenetre == 0 {
        return None;
    }
    unsafe {
        let (x2, lapic) = local_apic();
        timer_local_ecrit(x2, lapic, X2APIC_TIMER_DIVIDE, LAPIC_TIMER_DIVIDE, LAPIC_DIVISEUR_16);
        timer_local_ecrit(
            x2, lapic, X2APIC_LVT_TIMER, LAPIC_LVT_TIMER,
            RESCHEDULE_VECTOR as u32 | LVT_MASQUE,
        );
        timer_local_ecrit(x2, lapic, X2APIC_TIMER_INITIAL, LAPIC_TIMER_INITIAL, u32::MAX);

        let depart = crate::arch::x86_64::cpu::rdtsc();
        while crate::arch::x86_64::cpu::rdtsc().wrapping_sub(depart) < cycles_fenetre {
            core::hint::spin_loop();
        }
        let reste = timer_local_courant(x2, lapic);
        let ecoule_tsc = crate::arch::x86_64::cpu::rdtsc().wrapping_sub(depart);
        // Arreter le compteur : un timer laisse libre continuerait de decompter
        // jusqu'a zero et leverait une interruption hors quantum.
        timer_local_ecrit(x2, lapic, X2APIC_TIMER_INITIAL, LAPIC_TIMER_INITIAL, 0);

        let tics = u32::MAX.wrapping_sub(reste) as u64;
        // Un compteur qui n'a pas bouge veut dire que le timer local ne compte
        // pas. Deviner une frequence a partir de zero armerait un quantum au
        // hasard : mieux vaut rendre None et rester sur le repli.
        if tics == 0 || ecoule_tsc == 0 {
            return None;
        }
        let hz = tics.saturating_mul(tsc_hz) / ecoule_tsc;
        LAPIC_HZ.store(hz, Ordering::Relaxed);
        let compte = hz.saturating_mul(SCHED_QUANTUM_TICKS) / 1000;
        u32::try_from(compte.max(1)).ok()
    }
}

/// Programme le quantum local de CE coeur.
///
/// # LE DEFAUT QUE CECI CORRIGE, MESURE SUR LA MACHINE DE REFERENCE
///
/// Seul le mode TSC-deadline etait implemente. Or TSC-deadline est une
/// fonctionnalite Intel : `CPUID.1:ECX[24]`. Le TRIGKEY porte un Ryzen 7
/// 5800H -- `BOUCHAUD_HWPROBE_CPU vendor=AuthenticAMD logical=16` -- qui ne
/// l'expose pas.
///
/// Consequence : `init_local_scheduler_timer` rendait faux, AUCUN coeur
/// n'avait de timer local, et le seul battement du systeme restait le PIT, qui
/// est une source unique routee vers un seul coeur. L'enregistreur de vol
/// physique ne contient que des evenements de `cpu=0`, et les compteurs de
/// tick le confirment a chaque echantillon : `timer0` avance de deux mille a
/// treize mille, `timer1`, `timer2` et `timer3` restent a zero.
///
/// Quinze coeurs sur seize etaient donc en ligne, comptes dans
/// `SMP4_AP_STARTED count=15`, et incapables de preempter quoi que ce soit.
///
/// # Le repli
///
/// Le mode PERIODIQUE du timer LAPIC existe depuis le Pentium et n'a jamais
/// ete propre a un fondeur. Il se rearme tout seul : c'est meme plus simple
/// que TSC-deadline, qui exige une reecriture du MSR a chaque interruption.
fn init_local_scheduler_timer() -> bool {
    // TSC-deadline d'abord quand il existe : il derive du TSC, donc il ne
    // depend d'aucune frequence de bus a calibrer.
    if tsc_deadline_supported() {
        unsafe {
            let (x2, lapic) = local_apic();
            let lvt = RESCHEDULE_VECTOR as u32 | LVT_TSC_DEADLINE;
            timer_local_ecrit(x2, lapic, X2APIC_LVT_TIMER, LAPIC_LVT_TIMER, lvt);
        }
        MODE_TIMER_LOCAL.store(MODE_TSC_DEADLINE, Ordering::Release);
        arm_local_scheduler_timer();
        return true;
    }

    // Sinon le mode periodique. Le compte est calibre UNE fois : la frequence
    // du bus APIC est la meme pour tous les coeurs d'un paquet.
    let mut compte = LAPIC_COMPTE_QUANTUM.load(Ordering::Acquire);
    if compte == 0 {
        let Some(mesure) = calibre_timer_local() else { return false };
        LAPIC_COMPTE_QUANTUM.store(mesure, Ordering::Release);
        compte = mesure;
    }
    unsafe {
        let (x2, lapic) = local_apic();
        timer_local_ecrit(x2, lapic, X2APIC_TIMER_DIVIDE, LAPIC_TIMER_DIVIDE, LAPIC_DIVISEUR_16);
        timer_local_ecrit(
            x2, lapic, X2APIC_LVT_TIMER, LAPIC_LVT_TIMER,
            RESCHEDULE_VECTOR as u32 | LVT_PERIODIQUE,
        );
        // L'ecriture du compte initial DEMARRE le timer : elle vient en
        // dernier, une fois le vecteur et le mode en place.
        timer_local_ecrit(x2, lapic, X2APIC_TIMER_INITIAL, LAPIC_TIMER_INITIAL, compte);
    }
    MODE_TIMER_LOCAL.store(MODE_LAPIC_PERIODIQUE, Ordering::Release);
    true
}

pub fn arm_local_scheduler_timer() {
    // EN MODE PERIODIQUE, IL N'Y A RIEN A REARMER, ET SURTOUT RIEN A ECRIRE.
    //
    // `IA32_TSC_DEADLINE` n'existe pas sur un processeur sans TSC-deadline :
    // l'ecrire y leve une faute de protection generale. Le mode doit donc
    // decider AVANT toute ecriture, et non le seul drapeau « un timer local
    // est actif ».
    match MODE_TIMER_LOCAL.load(Ordering::Acquire) {
        MODE_LAPIC_PERIODIQUE => {}
        MODE_TSC_DEADLINE => {
            if let Some(hz) = crate::kernel::timer::tsc_hz() {
                let delta = (hz as u128)
                    .saturating_mul(SCHED_QUANTUM_TICKS as u128)
                    .div_ceil(1000)
                    .min(u64::MAX as u128) as u64;
                unsafe {
                    usermode::write_msr(
                        IA32_TSC_DEADLINE,
                        crate::arch::x86_64::cpu::rdtsc().saturating_add(delta.max(1)),
                    );
                }
            }
        }
        _ => {}
    }
}

/// Le mode de quantum local de ce coeur, pour le diagnostic.
pub fn mode_timer_local() -> &'static str {
    match MODE_TIMER_LOCAL.load(Ordering::Acquire) {
        MODE_TSC_DEADLINE => "tsc-deadline",
        MODE_LAPIC_PERIODIQUE => "lapic-periodique",
        _ => "aucun",
    }
}

/// Frequence mesuree du timer local, en hertz. Zero en mode TSC-deadline.
pub fn lapic_hz() -> u64 {
    LAPIC_HZ.load(Ordering::Relaxed)
}

/// Combien de coeurs battent VRAIMENT, mesure et non deduite.
///
/// # Pourquoi une mesure et pas un compte
///
/// `schedulable_cpus()` dit combien de coeurs sont en ligne. Il ne dit pas
/// combien en recoivent une interruption de quantum, et la difference n'est
/// pas theorique : sur la machine de reference, seize coeurs etaient en ligne
/// et un seul battait, parce que le seul timer implemente -- TSC-deadline --
/// est une fonctionnalite Intel absente d'un Ryzen.
///
/// Un coeur qui ne recoit rien ne preempte rien : il ne peut executer que ce
/// qu'il prend volontairement, et une tache qui s'y endort ne se reveille pas.
/// Le compter comme disponible est un mensonge que rien ne contredit.
///
/// La mesure prend deux releves separes par `fenetre_ms` et rend, pour chaque
/// coeur, le nombre d'interruptions de quantum recues entre les deux.
pub fn mesure_battement_par_coeur(fenetre_ms: u64) -> (usize, usize) {
    let coeurs = schedulable_cpus().min(MAX_CPUS);
    let mut avant = [0u64; MAX_CPUS];
    for cpu in 0..coeurs {
        avant[cpu] = crate::kernel::task::quantums_recus(cpu);
    }
    let echeance = crate::kernel::timer::monotonic_ns()
        .saturating_add(fenetre_ms.saturating_mul(1_000_000));
    // Une attente bornee et non un sommeil : la mesure doit valoir depuis
    // n'importe quel contexte, y compris l'amorcage ou aucune tache n'existe.
    crate::kernel::timer::attente_bornee(fenetre_ms.saturating_add(50), || {
        crate::kernel::timer::monotonic_ns() >= echeance
    });
    let mut battants = 0usize;
    for cpu in 0..coeurs {
        let delta = crate::kernel::task::quantums_recus(cpu).saturating_sub(avant[cpu]);
        crate::serial_println!(
            "BOUCHAUD_SMP_BATTEMENT cpu={} quantums={} fenetre_ms={} bat={}",
            cpu, delta, fenetre_ms, (delta > 0) as u8,
        );
        if delta > 0 {
            battants += 1;
        }
    }
    crate::serial_println!(
        "BOUCHAUD_SMP_BATTEMENT_BILAN battants={} en_ligne={} mode={}",
        battants, coeurs, mode_timer_local(),
    );
    (battants, coeurs)
}

pub fn local_scheduler_timer_enabled() -> bool {
    LOCAL_SCHED_TIMER.load(Ordering::Acquire)
}

/// Invalide sur les autres CPU les traductions appartenant a `pml4`.
///
/// Le broadcast est volontaire au premier jalon thread-SMP: tous les CPU
/// repondent, mais seul celui dont CR3 correspond au PML4 cible invalide son
/// TLB. Cela privilegie la correction et evite de dependre d'un active_cpus
/// parfait pendant la migration. `AddressSpace::active_cpus` est tout de meme
/// maintenu pour le diagnostic et l'optimisation future.
pub fn shootdown_tlb(pml4: u64, active_cpus: u64, start: u64, len: u64) {
    let current = cpu_index();
    let online = ONLINE_MASK.load(Ordering::Acquire) as u64;
    let self_bit = if current < 64 { 1u64 << current } else { 0 };
    let targets = active_cpus & online & !self_bit;
    if targets == 0 {
        return;
    }

    debug_assert!(
        !crate::kernel::smp_lock::held_by_current_cpu(),
        "TLB shootdown: attente ACK sous BKL interdite"
    );

    let slot = &TLB_SLOTS[current];
    debug_assert_eq!(slot.sequence.load(Ordering::Acquire), 0);
    let mut sequence = TLB_NEXT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    if sequence == 0 {
        sequence = TLB_NEXT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    }
    slot.pml4.store(pml4, Ordering::Relaxed);
    slot.start.store(start, Ordering::Relaxed);
    slot.len.store(len, Ordering::Relaxed);
    slot.targets.store(targets, Ordering::Relaxed);
    slot.acknowledgements.store(0, Ordering::Relaxed);
    // Publication release: le handler qui observe la generation voit tous les
    // champs precedents. La slot n'est rendue libre qu'apres tous les ACK.
    slot.sequence.store(sequence, Ordering::Release);
    TLB_SHOOTDOWN_COUNT.fetch_add(1, Ordering::Relaxed);

    unsafe { send_all_excluding_self(TLB_SHOOTDOWN_VECTOR as u32) };

    // BOUCHAUD_C1_SHOOTDOWN_REEMISSION_V1
    //
    // L'attente etait un `spin_loop` NU : une diffusion envoyee une fois, puis
    // une boucle sans borne, sans reemission et sans trace. Elle suppose que
    // l'IPI arrive toujours. Le blocage mm-ng6 a montre que non :
    //
    //     cpu=0 tlb_cibles=0x2 tlb_acks=0x0 idle=false   <- attend le CPU 1
    //     cpu=1                             idle=true    <- en hlt, muet
    //
    // 285 secondes dans le meme `munmap`, sans gros verrou, sans faute de page,
    // sans un seul message. Un IPI perdu -- un CPU dans sa fenetre `cli`
    // d'endormissement, ou deux IPI de meme vecteur fusionnes par l'APIC --
    // suffisait a figer la machine pour toujours.
    //
    // Reemettre est SUR : le gestionnaire ignore un creneau qu'il a deja
    // acquitte, donc un IPI de trop ne coute qu'une interruption. Ne pas
    // reemettre, lui, coute la machine. On vise precisement les manquants
    // plutot que de rediffuser a tous, pour ne pas reveiller ceux qui ont
    // deja repondu.
    //
    // Il n'existe aucun chemin de continuation apres une absence persistante
    // d'ACK : cela accepterait une traduction perimee. La borne ci-dessous est
    // donc FAIL-CLOSED (panic explicite), pas un abandon du shootdown.
    let debut_attente = crate::kernel::timer::monotonic_ns();
    let mut prochaine_relance = debut_attente.saturating_add(RELANCE_SHOOTDOWN_NS);
    let echeance = debut_attente.saturating_add(ECHEC_SHOOTDOWN_NS);
    let mut relance_comptee = false;
    loop {
        let acquittements = slot.acknowledgements.load(Ordering::Acquire);
        if acquittements & targets == targets {
            break;
        }
        let maintenant = crate::kernel::timer::monotonic_ns();
        if maintenant >= echeance {
            let manquants = targets & !acquittements;
            TLB_ECHECS.fetch_add(1, Ordering::Relaxed);
            crate::serial_println_brut!(
                "[TLB-SHOOTDOWN-ECHEC] cpu={} sequence={} cibles={:#x} acks={:#x} manquants={:#x} attente_ns={}",
                current,
                sequence,
                targets,
                acquittements,
                manquants,
                maintenant.saturating_sub(debut_attente),
            );
            panic!("TLB shootdown: acquittement manquant, arret fail-closed");
        }
        if maintenant >= prochaine_relance {
            let manquants = targets & !acquittements;
            if !relance_comptee {
                TLB_RELANCES.fetch_add(1, Ordering::Relaxed);
                relance_comptee = true;
            }
            for cpu in 0..MAX_CPUS.min(64) {
                if manquants & (1u64 << cpu) != 0 {
                    TLB_IPI_REEMIS.fetch_add(1, Ordering::Relaxed);
                    renvoie_shootdown(cpu);
                }
            }
            prochaine_relance = maintenant.saturating_add(RELANCE_SHOOTDOWN_NS);
        }
        spin_loop();
    }
    slot.sequence.store(0, Ordering::Release);
}

/// Handler minimal appele directement depuis l'IDT, sans BKL.
pub fn handle_tlb_shootdown() {
    let cpu = cpu_index();
    let bit = if cpu < 64 { 1u64 << cpu } else { 0 };

    if bit != 0 {
        for slot in TLB_SLOTS.iter() {
            let sequence = slot.sequence.load(Ordering::Acquire);
            if sequence == 0 || slot.targets.load(Ordering::Relaxed) & bit == 0 {
                continue;
            }
            if slot.acknowledgements.load(Ordering::Relaxed) & bit != 0 {
                continue;
            }

            let pml4 = slot.pml4.load(Ordering::Relaxed);
            if vmm::current_pml4() == pml4 {
                vmm::flush_local_range(
                    slot.start.load(Ordering::Relaxed),
                    slot.len.load(Ordering::Relaxed),
                );

                if let Some(id) = cpu_local::CpuId::from_index(cpu) {
                    cpu_local::local(id).note_tlb_shootdown();
                }
            }
            // AcqRel ordonne l'invalidation avant l'ACK observe par l'emetteur.
            slot.acknowledgements.fetch_or(bit, Ordering::AcqRel);
        }
    }
    eoi_local();
}

/// L'etat du creneau de shootdown d'un CPU : (sequence, cibles, acquittements).
///
/// L'attente d'acquittement de `demande_shootdown` est un `spin_loop` SANS
/// borne : si un seul CPU cible n'acquitte jamais, l'emetteur y reste pour
/// toujours. C'est ce que montre le blocage mm-ng6 -- `munmap` immobile
/// pendant 285 secondes, sans faute de page et sans gros verrou. Ces trois
/// nombres disent QUEL CPU manque a l'appel.
pub fn tlb_slot_etat(cpu: usize) -> (u64, u64, u64) {
    if cpu >= MAX_CPUS {
        return (0, 0, 0);
    }
    let slot = &TLB_SLOTS[cpu];
    (
        slot.sequence.load(Ordering::Relaxed),
        slot.targets.load(Ordering::Relaxed),
        slot.acknowledgements.load(Ordering::Relaxed),
    )
}

pub fn tlb_shootdown_count() -> u64 {
    TLB_SHOOTDOWN_COUNT.load(Ordering::Relaxed)
}

/// Active l'entree effective des AP dans la runqueue. Appele par le BSP une fois
/// les sous-systemes noyau initialises, afin qu'un AP ne touche pas l'allocateur
/// pendant le boot mono-CPU.
pub fn enable_scheduler() {
    if init_local_scheduler_timer() {
        LOCAL_SCHED_TIMER.store(true, Ordering::Release);
    }
    SCHEDULER_ENABLED.store(true, Ordering::Release);
    // LE COUP DE POUCE EST TOUJOURS NECESSAIRE, MEME AVEC UN TIMER LOCAL.
    //
    // Il ne l'etait qu'en l'absence de timer local, et c'etait un verrou
    // d'amorcage que rien ne signalait.
    //
    // Un AP attend `SCHEDULER_ENABLED` en `hlt` -- il ne scrute pas, il dort.
    // Il n'arme SON timer local qu'apres etre sorti de cette attente. Tant
    // qu'aucune interruption ne le reveille, il ne sort pas ; et sans etre
    // sorti, il n'a pas de timer pour se reveiller. Le timer local ne peut
    // donc pas s'amorcer tout seul.
    //
    // La mesure le montrait sans ambiguite : quarante-neuf quantums sur le
    // coeur zero en deux cents millisecondes, zero sur les trois autres, avec
    // un timer pourtant declare actif.
    //
    // Un seul reveil suffit : ensuite chaque coeur bat de lui-meme.
    broadcast_reschedule();
    crate::serial_println!(
        "BOUCHAUD_SMP_TIMER_LOCAL mode={} lapic_hz={} compte={} coeurs={}",
        mode_timer_local(),
        lapic_hz(),
        LAPIC_COMPTE_QUANTUM.load(Ordering::Acquire),
        schedulable_cpus(),
    );
    dmesg::log_fmt(format_args!(
        "SMP_NG2_SCHEDULER online={} mode=thread-load-balance+work-steal quantum={}ms",
        schedulable_cpus(),
        SCHED_QUANTUM_TICKS
    ));
    // LA PREUVE, TOUT DE SUITE, ET PAS LA PROMESSE.
    //
    // Programmer un timer local ne prouve pas qu'il se declenche. La mesure
    // coute deux cents millisecondes une seule fois a l'amorcage, et elle est
    // la difference entre « seize coeurs en ligne » et « seize coeurs qui
    // battent » -- deux affirmations que la machine de reference a montre
    // etre distinctes.
    if schedulable_cpus() > 1 {
        let _ = mesure_battement_par_coeur(200);
    }
}

pub fn init_probe() {
    // BOUCHAUD_SMP_NG1_CPU_FOUNDATION
    // CpuId 0 is always the BSP; APIC IDs are stored as hardware metadata.
    let bsp = cpu_local::register_bsp();
    cpu_local::mark_online(bsp, true);

    let exposed = (((__cpuid(1).ebx >> 16) & 0xff) as usize)
        .max(1)
        .min(MAX_CPUS);
    DISCOVERED.store(exposed, Ordering::Release);
    dmesg::log_fmt(format_args!("SMP4_DISCOVERED count={}", exposed));

    if exposed <= 1 {
        dmesg::log("SMP4_AP_STARTED count=0 reason=single-vcpu");
        dmesg::log("SMP4_SCHEDULER online=1 mode=UP");
        return;
    }

    if !vmm::identity_map_kernel_page(TRAMPOLINE_PHYS)
        || !vmm::identity_map_kernel_page(MAILBOX_PHYS)
        || !vmm::identity_map_kernel_page(BOOT_PML4_PHYS)
    {
        dmesg::log("SMP4_AP_STARTED count=0 reason=bootstrap-identity-map-failed");
        return;
    }

    dmesg::log("SMP4_STAGE bootstrap-identity-ok");

    unsafe {
        enable_local_apic();
        dmesg::log("SMP4_STAGE lapic-enabled");

        let trampoline = memory::phys_to_virt(TRAMPOLINE_PHYS);
        for (index, byte) in TRAMPOLINE.iter().copied().enumerate() {
            write_volatile(trampoline.add(index), byte);
        }

        let mailbox = memory::phys_to_virt(MAILBOX_PHYS);
        core::ptr::write_bytes(mailbox, 0, 4096);

        let cr3 = vmm::current_pml4();
        // Real mode cannot load a >4GiB CR3. Copy ONLY the top-level PML4 to
        // a known low page; its entries still point at the real page-table
        // hierarchy. Once IA-32e mode is entered, the trampoline immediately
        // reloads the original 64-bit CR3 from mailbox+0x18.
        let boot_pml4 = memory::phys_to_virt(BOOT_PML4_PHYS);
        if cr3 != BOOT_PML4_PHYS {
            core::ptr::copy_nonoverlapping(memory::phys_to_virt(cr3), boot_pml4, 4096);
        }
        write_volatile(mailbox as *mut u32, BOOT_PML4_PHYS as u32);
        write_volatile(mailbox.add(0x10) as *mut u32, 0); // atomic stack ticket
        write_volatile(mailbox.add(0x18) as *mut u64, cr3); // full BSP CR3
        crate::serial_println!(
            "SMP4_BOOTSTRAP_V3 low_cr3={:#x} real_cr3={:#x} stack_selector=atomic-ticket",
            BOOT_PML4_PHYS,
            cr3,
        );
        // BOUCHAUD_AP_ENTRY_STABLE: point d'entree AP exporte + mailbox verifiee avant SIPI.
        let ap_entry_addr = bouchaud_ap_entry as *const () as usize as u64;
        write_volatile(mailbox.add(8) as *mut u64, ap_entry_addr);
        let stored_ap_entry = read_volatile(mailbox.add(8) as *const u64);
        if stored_ap_entry != ap_entry_addr {
            dmesg::log_fmt(format_args!(
                "SMP4_AP_STARTED count=0 reason=ap-entry-mailbox-corrupt expected={:#x} stored={:#x}",
                ap_entry_addr,
                stored_ap_entry
            ));
            return;
        }
        dmesg::log_fmt(format_args!(
            "SMP4_AP_ENTRY addr={:#x} mailbox={:#x}",
            ap_entry_addr,
            stored_ap_entry
        ));

        for cpu in 0..MAX_CPUS {
            let base = core::ptr::addr_of!(AP_STACKS[cpu].0) as *const u8 as u64;
            let top = (base + AP_STACK_SIZE as u64) & !0xF;
            write_volatile(mailbox.add(0x20 + cpu * 8) as *mut u64, top);
        }

        // INIT assert/deassert puis deux SIPI. Les marqueurs restent cote
        // BSP: ils permettent de distinguer un crash AP d'un probleme LAPIC.
        dmesg::log("SMP4_STAGE init-assert");
        send_all_excluding_self(0x0000_C500);
        spin_delay(5_000_000);
        dmesg::log("SMP4_STAGE init-deassert");
        send_all_excluding_self(0x0000_8500);
        spin_delay(500_000);
        dmesg::log("SMP4_STAGE sipi-1");
        send_all_excluding_self(0x0000_0600 | SIPI_VECTOR);
        spin_delay(500_000);
        dmesg::log("SMP4_STAGE sipi-2");
        send_all_excluding_self(0x0000_0600 | SIPI_VECTOR);
    }

    let expected = exposed.saturating_sub(1);
    for _ in 0..30_000_000 {
        if started_aps() >= expected {
            break;
        }
        spin_loop();
    }
    dmesg::log_fmt(format_args!(
        "SMP4_AP_STARTED count={} expected={}",
        started_aps(), expected
    ));
    if started_aps() >= expected {
        crate::serial_println!(
            "BOUCHAUD_SMP_PHYSICAL_GREEN online={} reported={}",
            started_aps().saturating_add(1),
            exposed,
        );
    } else {
        crate::serial_println!(
            "BOUCHAUD_SMP_PHYSICAL_PARTIAL online={} reported={}",
            started_aps().saturating_add(1),
            exposed,
        );
    }
    log_cpu_topology();
}

fn log_cpu_topology() {
    for index in 0..cpu_local::registered_cpus() {
        let Some(id) = cpu_local::CpuId::from_index(index) else { continue; };
        let Some(desc) = cpu_local::descriptor(id) else { continue; };
        dmesg::log_fmt(format_args!(
            "[SMP-NG] CPU logical={} apic={} legacy={} package={} core={} thread={} online={}",
            desc.logical_id.as_usize(),
            desc.apic_id,
            desc.legacy_apic_id,
            desc.package_id,
            desc.core_id,
            desc.thread_id,
            desc.online as u8,
        ));
    }
}

/// Point d'entree 64 bits des AP, appele par le trampoline physique.
#[no_mangle]
#[inline(never)]
pub extern "C" fn bouchaud_ap_entry() -> ! {
    use crate::arch::x86_64::pat;
    // The trampoline has already selected a bootstrap stack. From this point on
    // hardware APIC identity is translated once into a dense Bouchaud CpuId.
    let cpu_id = match cpu_local::register_current_ap() {
        Some(id) if id != cpu_local::CpuId::BSP => id,
        _ => loop {
            unsafe { asm!("cli; hlt", options(nomem, nostack)); }
        },
    };
    let cpu = cpu_id.as_usize();

    // La PAT est par coeur : celui-ci reprogramme la sienne, sans quoi il
    // verrait le framebuffer non cachable la ou les autres le voient
    // combinable -- et une trame dessinee par lui couterait cent fois plus.
    pat::configure_ce_coeur();
    gdt::init_ap(cpu);
    idt::load_ap();
    usermode::init_ap(cpu);
    unsafe { enable_local_apic(); }

    let bit = 1usize << cpu;
    let before = ONLINE_MASK.fetch_or(bit, Ordering::AcqRel);
    if before & bit == 0 {
        ONLINE_CPUS.fetch_add(1, Ordering::AcqRel);
    }
    cpu_local::mark_online(cpu_id, true);

    interrupts::enable_ap();

    // Le BSP termine l'initialisation des structures non-SMP avant de nous
    // autoriser a toucher la runqueue / le tas noyau.
    while !SCHEDULER_ENABLED.load(Ordering::Acquire) {
        crate::arch::x86_64::cpu::wait_for_interrupt();
    }
    if LOCAL_SCHED_TIMER.load(Ordering::Acquire) {
        let _ = init_local_scheduler_timer();
    }

    // Pas de log ici : au moment ou le BSP libere les AP, il peut encore etre
    // en train d'emettre la banniere/autorun. Le marqueur agrege du BSP suffit
    // et evite de rendre la console serie elle-meme SMP avant le BKL.
    crate::kernel::task::secondary_cpu_loop()
}

pub fn state() -> &'static str {
    if schedulable_cpus() > 1 && scheduler_enabled() {
        "SMP actif: AP 64 bits + GDT/TSS/GS per-CPU + scheduler multi-CPU par affinite processus"
    } else if discovered_cpus() > 1 {
        "AP demarres; scheduler SMP en attente d'activation"
    } else {
        "un seul CPU expose"
    }
}
