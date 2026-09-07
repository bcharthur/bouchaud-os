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
    let Some(rsdp) = boot.rsdp_address else {
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
