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
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::arch::x86_64::{gdt, idt, interrupts, usermode};
use crate::kernel::{dmesg, memory, vmm};

pub const MAX_CPUS: usize = 16;
pub const RESCHEDULE_VECTOR: u8 = 0xF1;
pub const SCHED_QUANTUM_TICKS: u64 = 4;

const IA32_APIC_BASE: u32 = 0x1B;
const APIC_ENABLE: u64 = 1 << 11;
const X2APIC_ENABLE: u64 = 1 << 10;
const X2APIC_EOI: u32 = 0x80B;
const X2APIC_SVR: u32 = 0x80F;
const X2APIC_ICR: u32 = 0x830;

const LAPIC_EOI: usize = 0xB0;
const LAPIC_SVR: usize = 0xF0;
const LAPIC_ICR_LOW: usize = 0x300;
const LAPIC_ICR_HIGH: usize = 0x310;

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

pub fn hardware_apic_id() -> usize {
    ((__cpuid(1).ebx >> 24) & 0xff) as usize
}

/// Bouchaud utilise les APIC IDs contigus de QEMU comme index per-CPU.
/// `run.ps1` borne deja `-smp` a 16.
pub fn cpu_index() -> usize {
    let via_gs = usermode::cpu_index();
    if via_gs < MAX_CPUS {
        via_gs
    } else {
        hardware_apic_id().min(MAX_CPUS - 1)
    }
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

/// Active l'entree effective des AP dans la runqueue. Appele par le BSP une fois
/// les sous-systemes noyau initialises, afin qu'un AP ne touche pas l'allocateur
/// pendant le boot mono-CPU.
pub fn enable_scheduler() {
    SCHEDULER_ENABLED.store(true, Ordering::Release);
    broadcast_reschedule();
    dmesg::log_fmt(format_args!(
        "SMP4_SCHEDULER online={} mode=SMP-process-affinity quantum={}ms",
        schedulable_cpus(),
        SCHED_QUANTUM_TICKS
    ));
}

pub fn init_probe() {
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
}

/// Point d'entree 64 bits des AP, appele par le trampoline physique.
#[no_mangle]
#[inline(never)]
pub extern "C" fn bouchaud_ap_entry() -> ! {
    let cpu = hardware_apic_id();
    if cpu == 0 || cpu >= MAX_CPUS {
        loop {
            unsafe { asm!("cli; hlt", options(nomem, nostack)); }
        }
    }

    gdt::init_ap(cpu);
    idt::load_ap();
    usermode::init_ap(cpu);
    unsafe { enable_local_apic(); }

    let bit = 1usize << cpu;
    let before = ONLINE_MASK.fetch_or(bit, Ordering::AcqRel);
    if before & bit == 0 {
        ONLINE_CPUS.fetch_add(1, Ordering::AcqRel);
    }

    interrupts::enable_ap();

    // Le BSP termine l'initialisation des structures non-SMP avant de nous
    // autoriser a toucher la runqueue / le tas noyau.
    while !SCHEDULER_ENABLED.load(Ordering::Acquire) {
        crate::arch::x86_64::cpu::wait_for_interrupt();
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
