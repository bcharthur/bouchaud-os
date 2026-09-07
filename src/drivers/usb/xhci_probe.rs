//! Fondation xHCI read-only.
//!
//! Aucun registre n'est ecrit. Cette sonde confirme la topologie necessaire au
//! futur driver reset/DCBAA/rings/HID/mass-storage.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;

#[derive(Clone, Copy, Debug)]
pub struct PortInfo {
    pub port: u8,
    pub portsc: u32,
    pub connected: bool,
    pub enabled: bool,
    pub powered: bool,
    pub speed_id: u8,
}

#[derive(Clone, Debug)]
pub struct XhciInfo {
    pub vendor: u16,
    pub device: u16,
    pub bus: u8,
    pub slot: u8,
    pub function: u8,
    pub bar0: u64,
    pub cap_length: u8,
    pub hci_version: u16,
    pub max_slots: u8,
    pub max_interruptors: u16,
    pub max_ports: u8,
    pub hcsparams2: u32,
    pub hccparams1: u32,
    pub dboff: u32,
    pub rtsoff: u32,
    pub usbcmd: u32,
    pub usbsts: u32,
    pub config: u32,
    pub legacy_bios_owned: Option<bool>,
    pub legacy_os_owned: Option<bool>,
    pub ports: Vec<PortInfo>,
}

#[inline]
unsafe fn mmio8(base: *mut u8, offset: usize) -> u8 {
    core::ptr::read_volatile(base.add(offset) as *const u8)
}
#[inline]
unsafe fn mmio16(base: *mut u8, offset: usize) -> u16 {
    core::ptr::read_volatile(base.add(offset) as *const u16)
}
#[inline]
unsafe fn mmio32(base: *mut u8, offset: usize) -> u32 {
    core::ptr::read_volatile(base.add(offset) as *const u32)
}

impl XhciInfo {
    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "xHCI {:04x}:{:04x} pci={:02x}:{:02x}.{} bar0={:#x}",
            self.vendor, self.device, self.bus, self.slot, self.function, self.bar0
        );
        let _ = writeln!(
            out,
            "caplen={:#x} version={:#06x} slots={} interrupters={} ports={}",
            self.cap_length, self.hci_version, self.max_slots,
            self.max_interruptors, self.max_ports
        );
        let _ = writeln!(
            out,
            "hcsparams2={:#010x} hccparams1={:#010x} dboff={:#x} rtsoff={:#x}",
            self.hcsparams2, self.hccparams1, self.dboff, self.rtsoff
        );
        let _ = writeln!(
            out,
            "USBCMD={:#010x} USBSTS={:#010x} CONFIG={:#010x}",
            self.usbcmd, self.usbsts, self.config
        );
        let _ = writeln!(
            out,
            "legacy_bios_owned={:?} legacy_os_owned={:?}",
            self.legacy_bios_owned, self.legacy_os_owned
        );
        for port in &self.ports {
            let _ = writeln!(
                out,
                "port{} PORTSC={:#010x} connected={} enabled={} powered={} speed_id={}",
                port.port, port.portsc, port.connected, port.enabled, port.powered, port.speed_id
            );
        }
        out
    }
}

pub fn probe() -> Option<XhciInfo> {
    let dev = crate::arch::x86_64::pci::find_xhci()?;
    let bar0 = crate::arch::x86_64::pci::bar_decode(&dev, 0).adresse();
    if bar0 == 0 { return None; }

    let base = crate::kernel::memory::phys_to_virt(bar0);
    unsafe {
        let cap_length = mmio8(base, 0x00);
        let hci_version = mmio16(base, 0x02);
        let hcs1 = mmio32(base, 0x04);
        let hcsparams2 = mmio32(base, 0x08);
        let hccparams1 = mmio32(base, 0x10);
        let dboff = mmio32(base, 0x14) & !0x3;
        let rtsoff = mmio32(base, 0x18) & !0x1F;
        if !(0x20..=0x80).contains(&cap_length) { return None; }

        let max_slots = (hcs1 & 0xFF) as u8;
        let max_interruptors = ((hcs1 >> 8) & 0x7FF) as u16;
        let max_ports = ((hcs1 >> 24) & 0xFF) as u8;

        let op = base.add(cap_length as usize);
        let usbcmd = mmio32(op, 0x00);
        let usbsts = mmio32(op, 0x04);
        let config = mmio32(op, 0x38);

        let xecp_dwords = ((hccparams1 >> 16) & 0xFFFF) as usize;
        let (legacy_bios_owned, legacy_os_owned) = if xecp_dwords != 0 {
            let ext = base.add(xecp_dwords * 4);
            let header = mmio32(ext, 0);
            if (header & 0xFF) == 1 {
                (Some(header & (1 << 16) != 0), Some(header & (1 << 24) != 0))
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        let mut ports = Vec::new();
        for port in 1..=max_ports.min(32) {
            let portsc = mmio32(op, 0x400 + ((port as usize - 1) * 0x10));
            ports.push(PortInfo {
                port,
                portsc,
                connected: portsc & 1 != 0,
                enabled: portsc & (1 << 1) != 0,
                powered: portsc & (1 << 9) != 0,
                speed_id: ((portsc >> 10) & 0xF) as u8,
            });
        }

        Some(XhciInfo {
            vendor: dev.vendor, device: dev.device,
            bus: dev.bus, slot: dev.slot, function: dev.func,
            bar0, cap_length, hci_version, max_slots, max_interruptors, max_ports,
            hcsparams2, hccparams1, dboff, rtsoff, usbcmd, usbsts, config,
            legacy_bios_owned, legacy_os_owned, ports,
        })
    }
}
