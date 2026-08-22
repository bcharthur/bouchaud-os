//! SMP4 hardware bring-up probe for Bouchaud OS.
//!
//! With QEMU `-smp 4`, the BSP wakes the three APs through the Local APIC
//! INIT/SIPI sequence. Each AP executes a tiny 16-bit trampoline at physical
//! 0x8000, increments a shared byte at 0x9000, then halts.
//!
//! This deliberately does NOT enter the current `kernel::task` scheduler on APs:
//! that scheduler is UP and uses global static state/Rc<RefCell>. Running it on
//! several CPUs before a dedicated synchronization refactor would corrupt memory.

use core::arch::x86_64::__cpuid;
use core::hint::spin_loop;
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::arch::x86_64::usermode;
use crate::kernel::{dmesg, memory};

const IA32_APIC_BASE: u32 = 0x1B;
const APIC_ENABLE: u64 = 1 << 11;
const X2APIC_ENABLE: u64 = 1 << 10;
const X2APIC_SVR: u32 = 0x80F;
const X2APIC_ICR: u32 = 0x830;

const LAPIC_SVR: usize = 0xF0;
const LAPIC_ICR_LOW: usize = 0x300;
const LAPIC_ICR_HIGH: usize = 0x310;

const TRAMPOLINE_PHYS: u64 = 0x8000;
const COUNTER_PHYS: u64 = 0x9000;
const SIPI_VECTOR: u32 = (TRAMPOLINE_PHYS >> 12) as u32;

// Real-mode bytes:
// cli; cld; xor ax,ax; mov ds,ax;
// lock inc byte ptr [0x9000]; hlt; jmp hlt
const TRAMPOLINE: [u8; 14] = [
    0xFA, 0xFC,
    0x31, 0xC0,
    0x8E, 0xD8,
    0xF0, 0xFE, 0x06, 0x00, 0x90,
    0xF4,
    0xEB, 0xFD,
];

static DISCOVERED: AtomicUsize = AtomicUsize::new(1);
static STARTED_APS: AtomicUsize = AtomicUsize::new(0);

pub fn discovered_cpus() -> usize {
    DISCOVERED.load(Ordering::Acquire)
}

pub fn started_aps() -> usize {
    STARTED_APS.load(Ordering::Acquire)
}

pub fn schedulable_cpus() -> usize {
    1
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

unsafe fn send_all_excluding_self(x2apic: bool, lapic: *mut u8, low: u32) {
    let low = low | (3 << 18);
    if x2apic {
        usermode::write_msr(X2APIC_ICR, low as u64);
    } else {
        lapic_write(lapic, LAPIC_ICR_HIGH, 0);
        lapic_write(lapic, LAPIC_ICR_LOW, low);
        wait_xapic_delivery(lapic);
    }
}

pub fn init_probe() {
    let leaf1 = __cpuid(1);
    let exposed = (((leaf1.ebx >> 16) & 0xff) as usize).max(1).min(16);
    DISCOVERED.store(exposed, Ordering::Release);
    dmesg::log_fmt(format_args!("SMP4_DISCOVERED count={}", exposed));

    if exposed <= 1 {
        dmesg::log("SMP4_AP_STARTED count=0 reason=single-vcpu");
        return;
    }

    unsafe {
        let trampoline = memory::phys_to_virt(TRAMPOLINE_PHYS);
        for (index, byte) in TRAMPOLINE.iter().copied().enumerate() {
            write_volatile(trampoline.add(index), byte);
        }

        let counter = memory::phys_to_virt(COUNTER_PHYS);
        write_volatile(counter, 0u8);

        let mut apic_base_msr = usermode::read_msr(IA32_APIC_BASE);
        if apic_base_msr & APIC_ENABLE == 0 {
            apic_base_msr |= APIC_ENABLE;
            usermode::write_msr(IA32_APIC_BASE, apic_base_msr);
        }

        let x2apic = apic_base_msr & X2APIC_ENABLE != 0;
        let lapic_phys = apic_base_msr & 0x000f_ffff_ffff_f000;
        let lapic = memory::phys_to_virt(lapic_phys);

        if x2apic {
            let svr = usermode::read_msr(X2APIC_SVR);
            usermode::write_msr(X2APIC_SVR, svr | 0x100 | 0xFF);
        } else {
            let svr = lapic_read(lapic, LAPIC_SVR);
            lapic_write(lapic, LAPIC_SVR, svr | 0x100 | 0xFF);
        }

        // INIT assert/deassert then two SIPIs, destination shorthand =
        // all excluding BSP. Keeping the explicit deassert makes the probe
        // compatible with the legacy xAPIC startup sequence too.
        send_all_excluding_self(x2apic, lapic, 0x0000_C500);
        spin_delay(5_000_000);
        send_all_excluding_self(x2apic, lapic, 0x0000_8500);
        spin_delay(500_000);
        send_all_excluding_self(x2apic, lapic, 0x0000_0600 | SIPI_VECTOR);
        spin_delay(500_000);
        send_all_excluding_self(x2apic, lapic, 0x0000_0600 | SIPI_VECTOR);

        let expected = exposed.saturating_sub(1).min(15);
        let mut reached = 0usize;
        for _ in 0..20_000_000 {
            reached = read_volatile(counter) as usize;
            if reached >= expected {
                break;
            }
            spin_loop();
        }

        STARTED_APS.store(reached, Ordering::Release);
        dmesg::log_fmt(format_args!("SMP4_AP_STARTED count={} expected={}", reached, expected));
        dmesg::log("SMP4_SCHEDULER online=1 mode=UP-pending-refactor");
    }
}

pub fn state() -> &'static str {
    if discovered_cpus() > 1 {
        "AP bring-up INIT/SIPI actif; ordonnanceur utilisateur encore UP"
    } else {
        "un seul CPU expose"
    }
}
