// Primitives IF/TSC.

pub fn interrupts_enabled() -> bool {
    let flags: u64;
    unsafe {
        asm!(
            "pushfq; pop {}",
            out(reg) flags,
            options(nomem, preserves_flags)
        );
    }
    flags & (1 << 9) != 0
}

pub fn rdtsc() -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe {
        asm!(
            "rdtsc",
            out("eax") lo,
            out("edx") hi,
            options(nomem, nostack, preserves_flags)
        );
    }
    ((hi as u64) << 32) | lo as u64
}

/// Lecture TSC ordonnée pour le timekeeping.
pub fn read_tsc_ordered(rdtscp: bool) -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe {
        if rdtscp {
            asm!(
                "rdtscp",
                out("eax") lo,
                out("edx") hi,
                out("ecx") _,
                options(nomem, nostack)
            );
        } else {
            asm!(
                "lfence",
                "rdtsc",
                out("eax") lo,
                out("edx") hi,
                options(nomem, nostack)
            );
        }
    }
    ((hi as u64) << 32) | lo as u64
}
