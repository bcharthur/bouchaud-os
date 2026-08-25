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
const IA32_TSC_DEADLINE: u32 = 0x6E0;

const LAPIC_EOI: usize = 0xB0;
const LAPIC_SVR: usize = 0xF0;
const LAPIC_ICR_LOW: usize = 0x300;
const LAPIC_ICR_HIGH: usize = 0x310;
const LAPIC_LVT_TIMER: usize = 0x320;

const TRAMPOLINE_PHYS: u64 = 0x8000;
const MAILBOX_PHYS: u64 = 0x9000;
const SIPI_VECTOR: u32 = (TRAMPOLINE_PHYS >> 12) as u32;
const AP_STACK_SIZE: usize = 64 * 1024;

#[repr(align(16))]
#[derive(Clone, Copy)]
struct ApStack([u8; AP_STACK_SIZE]);

static mut AP_STACKS: [ApStack; MAX_CPUS] = [ApStack([0; AP_STACK_SIZE]); MAX_CPUS];

// Trampoline assemble en .code16/.code64 avec base physique fixe 0x8000.
// Mailbox :
//   +0x00 u32 CR3
//   +0x08 u64 ap_entry
//   +0x10 u64 stack_top[16] indexe par APIC ID QEMU
const TRAMPOLINE: [u8; 166] = [
    0xFA, 0xFC, 0x31, 0xC0, 0x8E, 0xD8, 0x8E, 0xC0, 0x8E, 0xD0, 0x0F, 0x01, 0x16, 0xA0, 0x80, 0x0F,
    0x20, 0xE0, 0x66, 0x83, 0xC8, 0x20, 0x0F, 0x22, 0xE0, 0x66, 0xA1, 0x00, 0x90, 0x0F, 0x22, 0xD8,
    0x66, 0xB9, 0x80, 0x00, 0x00, 0xC0, 0x0F, 0x32, 0x66, 0x0D, 0x00, 0x09, 0x00, 0x00, 0x0F, 0x30,
    0x0F, 0x20, 0xC0, 0x66, 0x0D, 0x01, 0x00, 0x00, 0x80, 0x0F, 0x22, 0xC0, 0x66, 0xEA, 0x44, 0x80,
    0x00, 0x00, 0x08, 0x00, 0x66, 0xB8, 0x10, 0x00, 0x8E, 0xD8, 0x8E, 0xC0, 0x8E, 0xD0, 0x31, 0xC0,
    0xB8, 0x01, 0x00, 0x00, 0x00, 0x0F, 0xA2, 0xC1, 0xEB, 0x18, 0x81, 0xE3, 0xFF, 0x00, 0x00, 0x00,
    0x83, 0xFB, 0x0F, 0x77, 0x1D, 0x89, 0xD8, 0x48, 0x8B, 0x24, 0xC5, 0x10, 0x90, 0x00, 0x00, 0x48,
    0x85, 0xE4, 0x74, 0x0E, 0x48, 0x83, 0xE4, 0xF0, 0x48, 0x8B, 0x04, 0x25, 0x08, 0x90, 0x00, 0x00,
    0xFF, 0xD0, 0xFA, 0xF4, 0xEB, 0xFD, 0x66, 0x90, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0xFF, 0xFF, 0x00, 0x00, 0x00, 0x9A, 0xAF, 0x00, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x92, 0xCF, 0x00,
    0x17, 0x00, 0x88, 0x80, 0x00, 0x00,
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
    let via_gs = usermode::cpu_index();
    if via_gs < MAX_CPUS {
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

/// Programme un quantum local TSC-deadline. Chaque CPU rearme son propre
/// timer; aucun broadcast BSP n'est requis dans ce mode.
fn init_local_scheduler_timer() -> bool {
    if !tsc_deadline_supported() {
        return false;
    }
    unsafe {
        let (x2, lapic) = local_apic();
        let lvt = RESCHEDULE_VECTOR as u32 | (2 << 17); // mode TSC deadline
        if x2 {
            usermode::write_msr(X2APIC_LVT_TIMER, lvt as u64);
        } else {
            lapic_write(lapic, LAPIC_LVT_TIMER, lvt);
        }
    }
    arm_local_scheduler_timer();
    true
}

pub fn arm_local_scheduler_timer() {
    if !LOCAL_SCHED_TIMER.load(Ordering::Acquire) && !tsc_deadline_supported() {
        return;
    }
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

    while slot.acknowledgements.load(Ordering::Acquire) & targets != targets {
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
    if !local_scheduler_timer_enabled() {
        broadcast_reschedule();
    }
    dmesg::log_fmt(format_args!(
        "SMP_NG2_SCHEDULER online={} mode=thread-load-balance+work-steal quantum={}ms",
        schedulable_cpus(),
        SCHED_QUANTUM_TICKS
    ));
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
        if cr3 > u32::MAX as u64 {
            dmesg::log_fmt(format_args!(
                "SMP4_AP_STARTED count=0 reason=cr3-above-4g cr3={:#x}", cr3
            ));
            return;
        }
        write_volatile(mailbox as *mut u32, cr3 as u32);
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
            write_volatile(mailbox.add(16 + cpu * 8) as *mut u64, top);
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
    // The trampoline has already selected a bootstrap stack. From this point on
    // hardware APIC identity is translated once into a dense Bouchaud CpuId.
    let cpu_id = match cpu_local::register_current_ap() {
        Some(id) if id != cpu_local::CpuId::BSP => id,
        _ => loop {
            unsafe { asm!("cli; hlt", options(nomem, nostack)); }
        },
    };
    let cpu = cpu_id.as_usize();

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
