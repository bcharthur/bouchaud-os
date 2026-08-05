//! Decouverte CPU via CPUID et primitives bas niveau (halt, rdtsc).

use core::arch::asm;

/// Boucle d'arret du CPU. Utilisee par le panic handler et l'arret du noyau.
pub fn halt_loop() -> ! {
    loop {
        unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)); }
    }
}

/// Pause le CPU jusqu'a la prochaine interruption (PIT/clavier/souris).
/// Reduit la charge CPU dans les boucles d'evenements.
///
/// N'appeler que si les interruptions sont **actives** : un `hlt` avec `IF=0`
/// arrete la machine pour de bon. Dans une attente de tache, preferer
/// [`wait_for_interrupt`], qui garantit la condition.
pub fn hlt() {
    unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)); }
}

/// Attend la prochaine interruption, interruptions actives quoi qu'il arrive.
///
/// C'est la primitive d'attente des boucles bloquantes du noyau (`poll`,
/// `select`, `futex`, sommeil, attente d'un fils). Un `hlt` seul y serait un
/// piege : il suffit que l'appelant ait herite d'un `IF=0` pour que le CPU
/// s'arrete sans plus jamais recevoir le tick qui devait le reveiller — la
/// machine entiere gele, sans faute ni message.
///
/// `sti` ne prend effet qu'apres l'instruction suivante : la paire `sti; hlt`
/// est donc indivisible, aucune interruption ne peut se glisser entre les deux
/// et etre perdue.
pub fn wait_for_interrupt() {
    unsafe { asm!("sti; hlt", options(nomem, nostack, preserves_flags)); }
}

/// Les interruptions sont-elles actives (drapeau `IF` de RFLAGS) ?
pub fn interrupts_enabled() -> bool {
    let flags: u64;
    unsafe { asm!("pushfq; pop {}", out(reg) flags, options(nomem, preserves_flags)); }
    flags & (1 << 9) != 0
}

/// Lit le compteur de cycles (Time Stamp Counter).
///
/// Sert de mesure de temps grossiere tant que le timer materiel (PIT/APIC) et
/// ses interruptions ne sont pas actives.
pub fn rdtsc() -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe {
        asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack, preserves_flags));
    }
    ((hi as u64) << 32) | (lo as u64)
}

/// Renvoie l'identifiant constructeur du CPU (12 octets ASCII).
#[cfg(target_arch = "x86_64")]
pub fn vendor() -> [u8; 12] {
    use core::arch::x86_64::__cpuid;
    let res = __cpuid(0);
    let mut vendor = [0u8; 12];
    vendor[0..4].copy_from_slice(&res.ebx.to_le_bytes());
    vendor[4..8].copy_from_slice(&res.edx.to_le_bytes());
    vendor[8..12].copy_from_slice(&res.ecx.to_le_bytes());
    vendor
}

fn bit(value: u32, index: u32) -> &'static str {
    if value & (1u32 << index) != 0 { "yes" } else { "no" }
}

/// Affiche les informations CPU detaillees (commande `cpuinfo`).
#[cfg(target_arch = "x86_64")]
pub fn print_cpuinfo() {
    use core::arch::x86_64::__cpuid;
    let vendor = vendor();
    crate::print!("vendor_id: ");
    for b in vendor { crate::print!("{}", b as char); }
    println!("");

    let leaf1 = __cpuid(1);
    let family = (leaf1.eax >> 8) & 0xf;
    let model = (leaf1.eax >> 4) & 0xf;
    let stepping = leaf1.eax & 0xf;
    println!("family: {}", family);
    println!("model: {}", model);
    println!("stepping: {}", stepping);
    println!("features:");
    println!("  sse3={} pclmulqdq={} vmx={} ssse3={}", bit(leaf1.ecx, 0), bit(leaf1.ecx, 1), bit(leaf1.ecx, 5), bit(leaf1.ecx, 9));
    println!("  sse={} sse2={} htt={}", bit(leaf1.edx, 25), bit(leaf1.edx, 26), bit(leaf1.edx, 28));
}
