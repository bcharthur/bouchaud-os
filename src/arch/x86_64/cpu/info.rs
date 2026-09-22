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

/// Les fils logiques par coeur physique, selon le MATERIEL.
///
/// BOUCHAUD_C26_SMT_MESURE_ET_NON_SUPPOSE
///
/// `sysroot.rs` supposait deux fils par coeur, en dur, parce que la TRIGKEY
/// porte un Ryzen 7 5700U. La supposition est juste sur cette machine-la et
/// fausse partout ailleurs : QEMU lance `-smp 8` en huit paquets d'un seul
/// fil, et `/proc/cpuinfo` annoncait alors « cpu cores: 4 » sur une machine
/// qui en a huit. Une bibliotheque qui dimensionne son parallelisme sur
/// `cpu cores` -- et Skia en est une -- en aurait utilise la moitie.
///
/// La feuille 0x1F, puis 0x0B a defaut : c'est l'enumeration de topologie
/// etendue. Son sous-niveau de type 1 est le niveau SMT, et son EBX donne le
/// nombre de processeurs logiques a ce niveau -- c'est-a-dire les fils d'un
/// coeur.
///
/// Rend `None` quand le materiel ne repond pas. `None` et non `1` : « le
/// materiel ne le dit pas » et « il y a un fil par coeur » sont deux faits
/// differents, et c'est a l'appelant de choisir son repli.
#[cfg(target_arch = "x86_64")]
pub fn fils_par_coeur() -> Option<usize> {
    use core::arch::x86_64::{__cpuid, __cpuid_count};

    // La feuille maximale supportee. Interroger une feuille au-dela rend des
    // valeurs d'une AUTRE feuille sur bien des processeurs, donc un nombre
    // plausible et faux.
    let maximale = unsafe { __cpuid(0) }.eax;

    for feuille in [0x1Fu32, 0x0B] {
        if maximale < feuille {
            continue;
        }
        for niveau in 0..8u32 {
            let r = unsafe { __cpuid_count(feuille, niveau) };
            if r.ebx == 0 {
                break;
            }
            let type_de_niveau = (r.ecx >> 8) & 0xff;
            if type_de_niveau == 1 {
                let fils = (r.ebx & 0xffff) as usize;
                if fils > 0 {
                    return Some(fils);
                }
            }
        }
    }
    None
}

#[cfg(not(target_arch = "x86_64"))]
pub fn fils_par_coeur() -> Option<usize> {
    None
}
