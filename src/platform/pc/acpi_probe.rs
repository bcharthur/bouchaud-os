//! Sonde ACPI read-only pour le bring-up physique.
//!
//! Le RSDP vient du BootInfo UEFI. Aucune table n'est modifiee.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;
use crate::boot::BootInfo;

const SDT_HEADER_LEN: usize = 36;
const MAX_TABLE_LEN: usize = 1024 * 1024;
const MAX_ROOT_ENTRIES: usize = 256;

#[derive(Clone, Debug)]
pub struct TableInfo {
    pub signature: String,
    pub address: u64,
    pub length: u32,
    pub checksum_ok: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct HpetInfo {
    pub table_address: u64,
    pub address_space: u8,
    pub base_address: u64,
    pub capabilities: Option<u64>,
    pub configuration: Option<u64>,
    pub main_counter: Option<u64>,
}

#[derive(Clone, Copy, Debug)]
pub struct McfgEntry {
    pub base_address: u64,
    pub segment: u16,
    pub start_bus: u8,
    pub end_bus: u8,
}

#[derive(Clone, Debug)]
pub struct Summary {
    pub rsdp_address: Option<u64>,
    pub revision: u8,
    pub rsdp_checksum_ok: bool,
    pub root_kind: &'static str,
    pub root_address: Option<u64>,
    pub root_checksum_ok: bool,
    pub tables: Vec<TableInfo>,
    pub hpet: Option<HpetInfo>,
    pub mcfg: Vec<McfgEntry>,
    pub madt_address: Option<u64>,
    pub fadt_address: Option<u64>,
}

impl Summary {
    fn absent(rsdp: Option<u64>) -> Self {
        Self {
            rsdp_address: rsdp,
            revision: 0,
            rsdp_checksum_ok: false,
            root_kind: "none",
            root_address: None,
            root_checksum_ok: false,
            tables: Vec::new(),
            hpet: None,
            mcfg: Vec::new(),
            madt_address: None,
            fadt_address: None,
        }
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "ACPI");
        let _ = writeln!(
            out,
            "rsdp={:#x} revision={} checksum={}",
            self.rsdp_address.unwrap_or(0),
            self.revision,
            self.rsdp_checksum_ok
        );
        let _ = writeln!(
            out,
            "root={} address={:#x} checksum={}",
            self.root_kind,
            self.root_address.unwrap_or(0),
            self.root_checksum_ok
        );
        for table in &self.tables {
            let _ = writeln!(
                out,
                "table {} addr={:#x} len={} checksum={}",
                table.signature, table.address, table.length, table.checksum_ok
            );
        }
        if let Some(hpet) = self.hpet {
            let _ = writeln!(
                out,
                "HPET table={:#x} space={} base={:#x} caps={:#x} config={:#x} counter={:#x}",
                hpet.table_address,
                hpet.address_space,
                hpet.base_address,
                hpet.capabilities.unwrap_or(0),
                hpet.configuration.unwrap_or(0),
                hpet.main_counter.unwrap_or(0),
            );
        } else {
            let _ = writeln!(out, "HPET absent");
        }
        for entry in &self.mcfg {
            let _ = writeln!(
                out,
                "MCFG base={:#x} segment={} buses={:02x}-{:02x}",
                entry.base_address, entry.segment, entry.start_bus, entry.end_bus
            );
        }
        let _ = writeln!(out, "MADT={:#x}", self.madt_address.unwrap_or(0));
        let _ = writeln!(out, "FADT={:#x}", self.fadt_address.unwrap_or(0));
        out
    }
}

#[inline]
unsafe fn read8(phys: u64) -> u8 {
    core::ptr::read_volatile(crate::kernel::memory::phys_to_virt(phys) as *const u8)
}

unsafe fn read16(phys: u64) -> u16 {
    u16::from_le_bytes([read8(phys), read8(phys + 1)])
}

unsafe fn read32(phys: u64) -> u32 {
    u32::from_le_bytes([read8(phys), read8(phys + 1), read8(phys + 2), read8(phys + 3)])
}

unsafe fn read64(phys: u64) -> u64 {
    u64::from_le_bytes([
        read8(phys), read8(phys + 1), read8(phys + 2), read8(phys + 3),
        read8(phys + 4), read8(phys + 5), read8(phys + 6), read8(phys + 7),
    ])
}

unsafe fn checksum_ok(phys: u64, len: usize) -> bool {
    if len == 0 || len > MAX_TABLE_LEN { return false; }
    let mut sum = 0u8;
    for offset in 0..len {
        sum = sum.wrapping_add(read8(phys + offset as u64));
    }
    sum == 0
}

unsafe fn signature4(phys: u64) -> String {
    let bytes = [read8(phys), read8(phys + 1), read8(phys + 2), read8(phys + 3)];
    match core::str::from_utf8(&bytes) {
        Ok(s) => String::from(s),
        Err(_) => String::from("????"),
    }
}

unsafe fn sdt_length(phys: u64) -> Option<usize> {
    let len = read32(phys + 4) as usize;
    (len >= SDT_HEADER_LEN && len <= MAX_TABLE_LEN).then_some(len)
}

unsafe fn parse_hpet(table: u64) -> Option<HpetInfo> {
    let len = sdt_length(table)?;
    if len < 56 { return None; }
    let address_space = read8(table + 40);
    let base_address = read64(table + 44);

    let mut capabilities = None;
    let mut configuration = None;
    let mut main_counter = None;
    if address_space == 0 && base_address != 0 {
        let base = crate::kernel::memory::phys_to_virt(base_address);
        capabilities = Some(core::ptr::read_volatile(base as *const u64));
        configuration = Some(core::ptr::read_volatile(base.add(0x10) as *const u64));
        main_counter = Some(core::ptr::read_volatile(base.add(0xF0) as *const u64));
    }

    Some(HpetInfo {
        table_address: table,
        address_space,
        base_address,
        capabilities,
        configuration,
        main_counter,
    })
}

unsafe fn parse_mcfg(table: u64) -> Vec<McfgEntry> {
    let mut out = Vec::new();
    let Some(len) = sdt_length(table) else { return out };
    if len < 44 { return out; }
    let mut offset = 44usize;
    while offset + 16 <= len && out.len() < 32 {
        out.push(McfgEntry {
            base_address: read64(table + offset as u64),
            segment: read16(table + offset as u64 + 8),
            start_bus: read8(table + offset as u64 + 10),
            end_bus: read8(table + offset as u64 + 11),
        });
        offset += 16;
    }
    out
}

pub fn probe(boot: &BootInfo) -> Summary {
    probe_rsdp(boot.rsdp_address)
}

/// BOUCHAUD_SMP_MADT_V1 -- le RSDP d'un amorcage BIOS : le chargeur 0.9 ne le
/// transmet pas. Recherche de la specification ACPI (5.2.5.1) : le premier
/// Kio de l'EBDA, puis 0xE0000..0xFFFFF, sur des frontieres de 16 octets,
/// signature ET somme des 20 premiers octets. Lecture seule.
pub fn rsdp_bios() -> Option<u64> {
    unsafe {
        let ebda = (read16(0x40E) as u64) << 4;
        let mut zones: [(u64, u64); 2] = [(0xE0000, 0x100000), (0, 0)];
        if (0x80000..0xA0000).contains(&ebda) { zones = [(ebda, ebda + 1024), (0xE0000, 0x100000)]; }
        for (debut, fin) in zones {
            let mut p = debut;
            while p + 20 <= fin {
                if read64(p) == u64::from_le_bytes(*b"RSD PTR ") && checksum_ok(p, 20) {
                    return Some(p);
                }
                p += 16;
            }
        }
    }
    None
}

/// Le RSDP de cet amorcage : transmis par UEFI, ou cherche en memoire BIOS.
pub fn rsdp(boot: &BootInfo) -> Option<u64> {
    match boot.rsdp_address {
        Some(r) => Some(r),
        None if boot.firmware == crate::boot::FirmwareKind::LegacyBios => rsdp_bios(),
        None => None,
    }
}

fn probe_rsdp(rsdp: Option<u64>) -> Summary {
    let Some(rsdp) = rsdp else {
        return Summary::absent(None);
    };

    unsafe {
        let sig = [
            read8(rsdp), read8(rsdp + 1), read8(rsdp + 2), read8(rsdp + 3),
            read8(rsdp + 4), read8(rsdp + 5), read8(rsdp + 6), read8(rsdp + 7),
        ];
        if &sig != b"RSD PTR " {
            return Summary::absent(Some(rsdp));
        }

        let revision = read8(rsdp + 15);
        let rsdp_len = if revision >= 2 {
            (read32(rsdp + 20) as usize).clamp(20, 4096)
        } else {
            20
        };
        let xsdt = if revision >= 2 { read64(rsdp + 24) } else { 0 };
        let rsdt = read32(rsdp + 16) as u64;
        let (root_kind, root, entry_size) = if xsdt != 0 {
            ("XSDT", xsdt, 8usize)
        } else if rsdt != 0 {
            ("RSDT", rsdt, 4usize)
        } else {
            ("none", 0, 0usize)
        };

        let mut summary = Summary::absent(Some(rsdp));
        summary.revision = revision;
        summary.rsdp_checksum_ok = checksum_ok(rsdp, rsdp_len);
        summary.root_kind = root_kind;
        summary.root_address = (root != 0).then_some(root);

        if root == 0 { return summary; }
        let Some(root_len) = sdt_length(root) else { return summary };
        summary.root_checksum_ok = checksum_ok(root, root_len);

        let count = ((root_len - SDT_HEADER_LEN) / entry_size).min(MAX_ROOT_ENTRIES);
        for index in 0..count {
            let ptr = root + SDT_HEADER_LEN as u64 + (index * entry_size) as u64;
            let table = if entry_size == 8 { read64(ptr) } else { read32(ptr) as u64 };
            if table == 0 { continue; }
            let signature = signature4(table);
            let length = sdt_length(table).unwrap_or(0);
            let valid = length != 0 && checksum_ok(table, length);
            summary.tables.push(TableInfo {
                signature: signature.clone(),
                address: table,
                length: length as u32,
                checksum_ok: valid,
            });
            match signature.as_str() {
                "HPET" => summary.hpet = parse_hpet(table),
                "MCFG" => summary.mcfg = parse_mcfg(table),
                "APIC" => summary.madt_address = Some(table),
                "FACP" => summary.fadt_address = Some(table),
                _ => {}
            }
        }
        summary
    }
}

/// BOUCHAUD_SMP_MADT_V1 -- les processeurs que le micrologiciel DECLARE
/// (MADT, entrees Local APIC et x2APIC « Enabled »).
///
/// `CPUID.1:EBX[23:16]` n'est PAS un nombre de processeurs : c'est le nombre
/// maximal d'identifiants logiques adressables dans UN boitier. Il vaut 1 sous
/// WHPX (Hyper-V le fixe), 1 sous QEMU `-smp N,sockets=N`, et sur un vrai
/// multi-socket ; il peut DEPASSER le nombre reel de coeurs sur un processeur
/// grand public. La MADT est la liste que l'OS doit demarrer.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProcesseursMadt {
    /// Entrees Local APIC / x2APIC avec le bit Enabled.
    pub actifs: usize,
    /// Entrees seulement « Online Capable » (branchables plus tard) : comptees
    /// a part, jamais demarrees a l'amorcage.
    pub branchables: usize,
}

pub fn processeurs_madt(boot: &BootInfo) -> Option<ProcesseursMadt> {
    let summary = probe_rsdp(rsdp(boot));
    let table = summary.madt_address?;
    unsafe {
        let len = sdt_length(table)?;
        if !checksum_ok(table, len) { return None; }
        let mut out = ProcesseursMadt::default();
        let mut vus: [u32; 64] = [u32::MAX; 64];
        let mut n_vus = 0usize;
        let mut offset = SDT_HEADER_LEN + 8;
        while offset + 2 <= len {
            let p = table + offset as u64;
            let kind = read8(p);
            let entry_len = read8(p + 1) as usize;
            if entry_len < 2 || offset + entry_len > len { break; }
            let (id, flags) = match (kind, entry_len) {
                (0, l) if l >= 8 => (read8(p + 3) as u32, read32(p + 4)),
                (9, l) if l >= 16 => (read32(p + 4), read32(p + 8)),
                _ => { offset += entry_len; continue; }
            };
            // Une meme unite decrite deux fois (Local APIC et x2APIC) ne compte qu'une fois.
            if !vus[..n_vus].contains(&id) {
                if n_vus < vus.len() { vus[n_vus] = id; n_vus += 1; }
                if flags & 1 != 0 { out.actifs += 1; } else if flags & 2 != 0 { out.branchables += 1; }
            }
            offset += entry_len;
        }
        Some(out)
    }
}
