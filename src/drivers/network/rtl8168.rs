//! Pilote minimal Realtek RTL8111/RTL8168 PCIe Gigabit Ethernet.
//!
//! Cible materielle Stage 2 TRIGKEY : PCI 10EC:8168. Le pilote fonctionne en
//! polling, sans MSI/MSI-X, et utilise les anneaux DMA 64 bits du controleur.
//! Il est volontairement strict : aucun autre identifiant Realtek n'est accepte.
//!
//! Le contrat de registres/descripteurs suit le pilote r8169 du noyau Linux et
//! le pilote rtl8169 d'U-Boot. Les sequences PHY/EEE specifiques a une revision
//! ne sont pas fabriquees ici : si la liaison physique ne monte pas sur une
//! revision particuliere, l'initialisation echoue proprement et le bureau reste
//! utilisable hors ligne.

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{compiler_fence, Ordering};

use crate::arch::x86_64::pci::{self, Bar, PciDevice};
use crate::kernel::{dmesg, memory};

pub const VENDOR_REALTEK: u16 = 0x10EC;
pub const DEVICE_RTL8168: u16 = 0x8168;

const REG_MAC0: u32 = 0x00;
const REG_MAR0: u32 = 0x08;
const REG_TX_DESC_LOW: u32 = 0x20;
const REG_TX_DESC_HIGH: u32 = 0x24;
const REG_CHIP_CMD: u32 = 0x37;
const REG_TX_POLL: u32 = 0x38;
const REG_INTR_MASK: u32 = 0x3C;
const REG_INTR_STATUS: u32 = 0x3E;
const REG_TX_CONFIG: u32 = 0x40;
const REG_RX_CONFIG: u32 = 0x44;
const REG_RX_MISSED: u32 = 0x4C;
const REG_CFG9346: u32 = 0x50;
const REG_PHY_STATUS: u32 = 0x6C;
const REG_RX_MAX_SIZE: u32 = 0xDA;
const REG_CPLUS_CMD: u32 = 0xE0;
const REG_RX_DESC_LOW: u32 = 0xE4;
const REG_RX_DESC_HIGH: u32 = 0xE8;

const CMD_RESET: u8 = 0x10;
const CMD_RX_ENABLE: u8 = 0x08;
const CMD_TX_ENABLE: u8 = 0x04;
const TX_POLL_NPQ: u8 = 0x40;
const CFG9346_UNLOCK: u8 = 0xC0;
const CFG9346_LOCK: u8 = 0x00;
const PHY_LINK_STATUS: u8 = 0x02;

const ACCEPT_BROADCAST: u32 = 0x08;
const ACCEPT_MULTICAST: u32 = 0x04;
const ACCEPT_MY_PHYS: u32 = 0x02;
const RX_FIFO_THRESH: u32 = 7 << 13;
const RX_DMA_BURST: u32 = 7 << 8;
const TX_DMA_BURST: u32 = 7 << 8;
const TX_INTERFRAME_GAP: u32 = 3 << 24;
const CPLUS_PCIDAC: u16 = 1 << 4;

const DESC_OWN: u32 = 1 << 31;
const DESC_EOR: u32 = 1 << 30;
const DESC_FS: u32 = 1 << 29;
const DESC_LS: u32 = 1 << 28;
const DESC_RX_ERROR: u32 = (1 << 22) | (1 << 21) | (1 << 20) | (1 << 19);
const DESC_LEN_MASK: u32 = 0x3FFF;

const N_RX: usize = 64;
const N_TX: usize = 16;
const BUF_SIZE: usize = 2048;
const DESC_SIZE: usize = 16;
const RESET_SPINS: usize = 1_000_000;
const LINK_WAIT_MS: u64 = 3_000;

static mut MMIO: u64 = 0;
static mut READY: bool = false;
static mut MAC: [u8; 6] = [0; 6];
static mut RX_RING: *mut u8 = core::ptr::null_mut();
static mut TX_RING: *mut u8 = core::ptr::null_mut();
static mut RX_BUFFER_V: *mut u8 = core::ptr::null_mut();
static mut RX_BUFFER_P: u64 = 0;
static mut TX_BUFFER_V: *mut u8 = core::ptr::null_mut();
static mut TX_BUFFER_P: u64 = 0;
static mut RX_CUR: usize = 0;
static mut TX_CUR: usize = 0;
static mut TX_RING_FULL: u64 = 0;

#[inline]
unsafe fn read8(offset: u32) -> u8 {
    read_volatile((MMIO + u64::from(offset)) as *const u8)
}

#[inline]
unsafe fn read16(offset: u32) -> u16 {
    read_volatile((MMIO + u64::from(offset)) as *const u16)
}

#[inline]
unsafe fn read32(offset: u32) -> u32 {
    read_volatile((MMIO + u64::from(offset)) as *const u32)
}

#[inline]
unsafe fn write8(offset: u32, value: u8) {
    write_volatile((MMIO + u64::from(offset)) as *mut u8, value);
}

#[inline]
unsafe fn write16(offset: u32, value: u16) {
    write_volatile((MMIO + u64::from(offset)) as *mut u16, value);
}

#[inline]
unsafe fn write32(offset: u32, value: u32) {
    write_volatile((MMIO + u64::from(offset)) as *mut u32, value);
}

#[inline]
unsafe fn desc_read32(ring: *mut u8, index: usize, offset: usize) -> u32 {
    read_volatile(ring.add(index * DESC_SIZE + offset) as *const u32)
}

#[inline]
unsafe fn desc_write32(ring: *mut u8, index: usize, offset: usize, value: u32) {
    write_volatile(ring.add(index * DESC_SIZE + offset) as *mut u32, value);
}

#[inline]
unsafe fn desc_write64(ring: *mut u8, index: usize, offset: usize, value: u64) {
    write_volatile(ring.add(index * DESC_SIZE + offset) as *mut u64, value);
}

fn memory_bar(device: &PciDevice) -> Option<u64> {
    let mut index = 0u8;
    while index < 6 {
        let bar = pci::bar_decode(device, index);
        match bar {
            Bar::Memoire32 { adresse, .. } => return Some(u64::from(adresse)),
            Bar::Memoire64 { adresse, .. } => return Some(adresse),
            Bar::Port(_) | Bar::Absent => {}
        }
        index += if bar.double() { 2 } else { 1 };
    }
    None
}

fn valid_mac(mac: &[u8; 6]) -> bool {
    mac.iter().any(|&b| b != 0) && mac.iter().any(|&b| b != 0xFF)
}

pub fn is_supported(device: &PciDevice) -> bool {
    device.vendor == VENDOR_REALTEK && device.device == DEVICE_RTL8168
}

pub fn is_ready() -> bool {
    unsafe { READY }
}

pub fn mac() -> [u8; 6] {
    unsafe { MAC }
}

pub fn tx_anneau_plein() -> u64 {
    unsafe { TX_RING_FULL }
}

pub fn init_with_device(device: &PciDevice) -> bool {
    unsafe {
        if READY {
            return true;
        }
    }

    if !is_supported(device) {
        return false;
    }

    let bar = match memory_bar(device) {
        Some(address) if address != 0 => address,
        _ => {
            dmesg::log("rtl8168: aucun BAR MMIO exploitable");
            return false;
        }
    };

    crate::serial_println!(
        "BOUCHAUD_TRIGKEY_RTL8168_DETECTED pci={:04x}:{:04x} bus={:02x}:{:02x}.{} mmio={:#x}",
        device.vendor,
        device.device,
        device.bus,
        device.slot,
        device.func,
        bar,
    );

    pci::enable_bus_master(device);

    unsafe {
        MMIO = memory::phys_offset().wrapping_add(bar);

        // Arrete Rx/Tx puis reinitialise le MAC. Boucle bornee : un controleur
        // qui ne sort pas du reset ne doit jamais figer le boot du bureau.
        write8(REG_CHIP_CMD, 0);
        write16(REG_INTR_MASK, 0);
        write16(REG_INTR_STATUS, 0xFFFF);
        write8(REG_CHIP_CMD, CMD_RESET);

        let mut reset_ok = false;
        for _ in 0..RESET_SPINS {
            if read8(REG_CHIP_CMD) & CMD_RESET == 0 {
                reset_ok = true;
                break;
            }
            core::hint::spin_loop();
        }
        if !reset_ok {
            dmesg::log("rtl8168: reset timeout");
            MMIO = 0;
            return false;
        }

        let mut detected_mac = [0u8; 6];
        for (index, byte) in detected_mac.iter_mut().enumerate() {
            *byte = read8(REG_MAC0 + index as u32);
        }
        if !valid_mac(&detected_mac) {
            dmesg::log("rtl8168: adresse MAC invalide");
            MMIO = 0;
            return false;
        }
        MAC = detected_mac;

        let (rx_ring_p, rx_ring_v) = match memory::alloc_dma(N_RX * DESC_SIZE) {
            Some(pair) => pair,
            None => {
                dmesg::log("rtl8168: allocation DMA anneau RX impossible");
                MMIO = 0;
                return false;
            }
        };
        let (tx_ring_p, tx_ring_v) = match memory::alloc_dma(N_TX * DESC_SIZE) {
            Some(pair) => pair,
            None => {
                dmesg::log("rtl8168: allocation DMA anneau TX impossible");
                MMIO = 0;
                return false;
            }
        };
        let (rx_buffer_p, rx_buffer_v) = match memory::alloc_dma(N_RX * BUF_SIZE) {
            Some(pair) => pair,
            None => {
                dmesg::log("rtl8168: allocation DMA buffers RX impossible");
                MMIO = 0;
                return false;
            }
        };
        let (tx_buffer_p, tx_buffer_v) = match memory::alloc_dma(N_TX * BUF_SIZE) {
            Some(pair) => pair,
            None => {
                dmesg::log("rtl8168: allocation DMA buffers TX impossible");
                MMIO = 0;
                return false;
            }
        };

        RX_RING = rx_ring_v;
        TX_RING = tx_ring_v;
        RX_BUFFER_P = rx_buffer_p;
        RX_BUFFER_V = rx_buffer_v;
        TX_BUFFER_P = tx_buffer_p;
        TX_BUFFER_V = tx_buffer_v;
        RX_CUR = 0;
        TX_CUR = 0;

        for index in 0..N_RX {
            let eor = if index + 1 == N_RX { DESC_EOR } else { 0 };
            desc_write32(RX_RING, index, 0, DESC_OWN | eor | BUF_SIZE as u32);
            desc_write32(RX_RING, index, 4, 0);
            desc_write64(
                RX_RING,
                index,
                8,
                rx_buffer_p + (index * BUF_SIZE) as u64,
            );
        }
        for index in 0..N_TX {
            let eor = if index + 1 == N_TX { DESC_EOR } else { 0 };
            desc_write32(TX_RING, index, 0, eor);
            desc_write32(TX_RING, index, 4, 0);
            desc_write64(
                TX_RING,
                index,
                8,
                tx_buffer_p + (index * BUF_SIZE) as u64,
            );
        }

        // Les anneaux peuvent vivre au-dessus de 4 Gio dans l'arene DMA.
        // RTL8168 sait les adresser en 64 bits via PCIDAC + registres HIGH.
        write8(REG_CFG9346, CFG9346_UNLOCK);
        write16(REG_CPLUS_CMD, read16(REG_CPLUS_CMD) | CPLUS_PCIDAC);
        write16(REG_RX_MAX_SIZE, (BUF_SIZE - 1) as u16);

        write32(REG_TX_DESC_LOW, tx_ring_p as u32);
        write32(REG_TX_DESC_HIGH, (tx_ring_p >> 32) as u32);
        write32(REG_RX_DESC_LOW, rx_ring_p as u32);
        write32(REG_RX_DESC_HIGH, (rx_ring_p >> 32) as u32);

        write32(
            REG_RX_CONFIG,
            RX_FIFO_THRESH
                | RX_DMA_BURST
                | ACCEPT_BROADCAST
                | ACCEPT_MULTICAST
                | ACCEPT_MY_PHYS,
        );
        write32(REG_TX_CONFIG, TX_DMA_BURST | TX_INTERFRAME_GAP);
        write32(REG_MAR0, 0xFFFF_FFFF);
        write32(REG_MAR0 + 4, 0xFFFF_FFFF);
        write32(REG_RX_MISSED, 0);
        write16(REG_INTR_MASK, 0);
        write16(REG_INTR_STATUS, 0xFFFF);

        compiler_fence(Ordering::Release);
        write8(REG_CHIP_CMD, CMD_TX_ENABLE | CMD_RX_ENABLE);
        write8(REG_CFG9346, CFG9346_LOCK);

        READY = true;
    }

    let m = mac();
    let raw_version = unsafe { read32(REG_TX_CONFIG) };
    dmesg::log_fmt(format_args!(
        "rtl8168: initialise MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} txcfg={:#010x}",
        m[0], m[1], m[2], m[3], m[4], m[5], raw_version,
    ));
    crate::serial_println!(
        "BOUCHAUD_TRIGKEY_RTL8168_DRIVER_OK mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        m[0], m[1], m[2], m[3], m[4], m[5],
    );
    // Une autonegociation cuivre gigabit prend couramment plus d'une seconde.
    // `net::demarre()` teste le lien juste apres `init()`, donc on lui laisse
    // une fenetre BORNEE avant de conclure que le cable est debranche.
    let deadline = crate::kernel::timer::monotonic_ms().saturating_add(LINK_WAIT_MS);
    while !link_up() && crate::kernel::timer::monotonic_ms() < deadline {
        core::hint::spin_loop();
    }

    if link_up() {
        crate::serial_println!("BOUCHAUD_TRIGKEY_RTL8168_LINK_UP");
    } else {
        crate::serial_println!("BOUCHAUD_TRIGKEY_RTL8168_LINK_DOWN");
    }
    true
}

pub fn link_up() -> bool {
    unsafe { READY && (read8(REG_PHY_STATUS) & PHY_LINK_STATUS != 0) }
}

pub fn send(frame: &[u8]) -> bool {
    if frame.is_empty() || frame.len() > BUF_SIZE {
        return false;
    }

    unsafe {
        if !READY {
            return false;
        }

        let index = TX_CUR;
        let previous = desc_read32(TX_RING, index, 0);
        if previous & DESC_OWN != 0 {
            TX_RING_FULL = TX_RING_FULL.saturating_add(1);
            return false;
        }

        let destination = TX_BUFFER_V.add(index * BUF_SIZE);
        core::ptr::copy_nonoverlapping(frame.as_ptr(), destination, frame.len());

        let eor = if index + 1 == N_TX { DESC_EOR } else { 0 };
        desc_write64(
            TX_RING,
            index,
            8,
            TX_BUFFER_P + (index * BUF_SIZE) as u64,
        );
        desc_write32(TX_RING, index, 4, 0);
        compiler_fence(Ordering::Release);
        desc_write32(
            TX_RING,
            index,
            0,
            DESC_OWN | DESC_FS | DESC_LS | eor | frame.len() as u32,
        );
        compiler_fence(Ordering::Release);
        write8(REG_TX_POLL, TX_POLL_NPQ);
        TX_CUR = (index + 1) % N_TX;
        true
    }
}

pub fn receive(out: &mut [u8]) -> Option<usize> {
    unsafe {
        if !READY {
            return None;
        }

        let index = RX_CUR;
        let status = desc_read32(RX_RING, index, 0);
        if status & DESC_OWN != 0 {
            return None;
        }

        compiler_fence(Ordering::Acquire);
        let raw_len = (status & DESC_LEN_MASK) as usize;
        let good = status & DESC_RX_ERROR == 0 && raw_len >= 4 && raw_len <= BUF_SIZE;
        let payload_len = raw_len.saturating_sub(4); // RTL8168 livre aussi le FCS.
        let copied = if good {
            let n = payload_len.min(out.len());
            let source = RX_BUFFER_V.add(index * BUF_SIZE);
            core::ptr::copy_nonoverlapping(source, out.as_mut_ptr(), n);
            Some(n)
        } else {
            None
        };

        let eor = if index + 1 == N_RX { DESC_EOR } else { 0 };
        desc_write64(
            RX_RING,
            index,
            8,
            RX_BUFFER_P + (index * BUF_SIZE) as u64,
        );
        desc_write32(RX_RING, index, 4, 0);
        compiler_fence(Ordering::Release);
        desc_write32(RX_RING, index, 0, DESC_OWN | eor | BUF_SIZE as u32);
        RX_CUR = (index + 1) % N_RX;
        copied
    }
}

pub fn print_info() {
    if !is_ready() {
        crate::println!("rtl8168: non initialise");
        return;
    }
    let m = mac();
    crate::println!(
        "rtl8168: MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} lien={}",
        m[0],
        m[1],
        m[2],
        m[3],
        m[4],
        m[5],
        if link_up() { "UP" } else { "DOWN" },
    );
}
