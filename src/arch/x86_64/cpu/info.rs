// CPUID / cpuinfo.

#[cfg(target_arch = "x86_64")]
pub fn vendor() -> [u8; 12] {
    use core::arch::x86_64::__cpuid;
    let res = unsafe { __cpuid(0) };
    let mut vendor = [0u8; 12];
    vendor[0..4].copy_from_slice(&res.ebx.to_le_bytes());
    vendor[4..8].copy_from_slice(&res.edx.to_le_bytes());
    vendor[8..12].copy_from_slice(&res.ecx.to_le_bytes());
    vendor
}

fn bit(value: u32, index: u32) -> &'static str {
    if value & (1u32 << index) != 0 {
        "yes"
    } else {
        "no"
    }
}

#[cfg(target_arch = "x86_64")]
pub fn print_cpuinfo() {
    use core::arch::x86_64::__cpuid;

    let vendor = vendor();
    crate::print!("vendor_id: ");
    for b in vendor {
        crate::print!("{}", b as char);
    }
    println!("");

    let leaf1 = unsafe { __cpuid(1) };
    let family = (leaf1.eax >> 8) & 0xf;
    let model = (leaf1.eax >> 4) & 0xf;
    let stepping = leaf1.eax & 0xf;
    println!("family: {}", family);
    println!("model: {}", model);
    println!("stepping: {}", stepping);
    println!("features:");
    println!(
        "  sse3={} pclmulqdq={} vmx={} ssse3={}",
        bit(leaf1.ecx, 0),
        bit(leaf1.ecx, 1),
        bit(leaf1.ecx, 5),
        bit(leaf1.ecx, 9)
    );
    println!(
        "  sse={} sse2={} htt={}",
        bit(leaf1.edx, 25),
        bit(leaf1.edx, 26),
        bit(leaf1.edx, 28)
    );
    println!(
        "  smp_online={} logical_cpu={} apic_id={}",
        smp::schedulable_cpus(),
        hardware_cpu_index(),
        smp::hardware_apic_id()
    );
}
