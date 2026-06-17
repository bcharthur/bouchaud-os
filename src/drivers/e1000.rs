//! Pilote carte reseau Intel e1000 (82540EM, ce que QEMU emule par defaut).
//!
//! Acces aux registres via MMIO (BAR0 mappe grace a l'offset memoire physique).
//! Anneaux RX/TX et tampons alloues dans l'arene DMA (`kernel::memory`). Permet
//! la lecture de l'adresse MAC, l'envoi et la reception de trames Ethernet.
//!
//! Initialise a la demande (commande reseau), pas au boot : si quelque chose se
//! passe mal, le reste du systeme n'est pas affecte.

use core::ptr::{read_volatile, write_volatile};
use crate::arch::x86_64::pci;
use crate::kernel::{dmesg, memory};

// Registres e1000 (offsets en octets).
const REG_CTRL: u32 = 0x0000;
const REG_STATUS: u32 = 0x0008;
const REG_ICR: u32 = 0x00C0;
const REG_IMC: u32 = 0x00D8;
const REG_RCTL: u32 = 0x0100;
const REG_TCTL: u32 = 0x0400;
const REG_TIPG: u32 = 0x0410;
const REG_RDBAL: u32 = 0x2800;
const REG_RDBAH: u32 = 0x2804;
const REG_RDLEN: u32 = 0x2808;
const REG_RDH: u32 = 0x2810;
const REG_RDT: u32 = 0x2818;
const REG_TDBAL: u32 = 0x3800;
const REG_TDBAH: u32 = 0x3804;
const REG_TDLEN: u32 = 0x3808;
const REG_TDH: u32 = 0x3810;
const REG_TDT: u32 = 0x3818;
const REG_RAL0: u32 = 0x5400;
const REG_RAH0: u32 = 0x5404;
const REG_MTA: u32 = 0x5200;

const N_RX: usize = 64; // Google peut envoyer un burst de segments TLS; 16 etait trop juste.
const N_TX: usize = 16;
const BUF: usize = 2048;
const DESC_SZ: usize = 16;

static mut MMIO: u64 = 0;
static mut READY: bool = false;
static mut MAC: [u8; 6] = [0; 6];

static mut RX_RING: *mut u8 = core::ptr::null_mut();
static mut TX_RING: *mut u8 = core::ptr::null_mut();
static mut RX_BUF_V: *mut u8 = core::ptr::null_mut();
static mut RX_BUF_P: u64 = 0;
static mut TX_BUF_V: *mut u8 = core::ptr::null_mut();
static mut TX_BUF_P: u64 = 0;
static mut RX_CUR: usize = 0;
static mut TX_CUR: usize = 0;

unsafe fn reg_read(off: u32) -> u32 {
    read_volatile((MMIO + off as u64) as *const u32)
}
unsafe fn reg_write(off: u32, val: u32) {
    write_volatile((MMIO + off as u64) as *mut u32, val);
}

// Acces aux champs d'un descripteur (par offset, pour eviter les refs packed).
unsafe fn desc_set_u64(ring: *mut u8, i: usize, off: usize, v: u64) {
    write_volatile(ring.add(i * DESC_SZ + off) as *mut u64, v);
}
unsafe fn desc_set_u16(ring: *mut u8, i: usize, off: usize, v: u16) {
    write_volatile(ring.add(i * DESC_SZ + off) as *mut u16, v);
}
unsafe fn desc_set_u8(ring: *mut u8, i: usize, off: usize, v: u8) {
    write_volatile(ring.add(i * DESC_SZ + off), v);
}
unsafe fn desc_get_u8(ring: *mut u8, i: usize, off: usize) -> u8 {
    read_volatile(ring.add(i * DESC_SZ + off))
}
unsafe fn desc_get_u16(ring: *mut u8, i: usize, off: usize) -> u16 {
    read_volatile(ring.add(i * DESC_SZ + off) as *const u16)
}

fn delay(loops: u32) {
    for _ in 0..loops {
        unsafe { core::arch::asm!("pause", options(nomem, nostack)); }
    }
}

/// Indique si la carte est initialisee.
pub fn is_ready() -> bool {
    unsafe { READY }
}

/// Adresse MAC lue sur la carte.
pub fn mac() -> [u8; 6] {
    unsafe { MAC }
}

/// Initialise la carte e1000 (idempotent). Renvoie false si absente/echec.
pub fn init() -> bool {
    unsafe {
        if READY { return true; }
    }
    let dev = match pci::find_network() {
        Some(d) => d,
        None => { dmesg::log("e1000: aucune carte reseau PCI"); return false; }
    };
    // Seules les cartes Intel sont gerees ici.
    if dev.vendor != 0x8086 {
        dmesg::log("e1000: carte non Intel, driver non charge");
        return false;
    }

    pci::enable_bus_master(&dev);
    let bar0 = (pci::bar(&dev, 0) & 0xFFFF_FFF0) as u64;
    if bar0 == 0 {
        dmesg::log("e1000: BAR0 invalide");
        return false;
    }

    unsafe {
        MMIO = memory::phys_offset() + bar0;

        // Reset du controleur.
        reg_write(REG_CTRL, reg_read(REG_CTRL) | 0x0400_0000); // CTRL.RST
        delay(200_000);
        // Masque toutes les interruptions (on fonctionne en polling).
        reg_write(REG_IMC, 0xFFFF_FFFF);
        let _ = reg_read(REG_ICR);

        // Lecture de l'adresse MAC depuis RAL0/RAH0.
        let ral = reg_read(REG_RAL0);
        let rah = reg_read(REG_RAH0);
        MAC = [
            ral as u8, (ral >> 8) as u8, (ral >> 16) as u8, (ral >> 24) as u8,
            rah as u8, (rah >> 8) as u8,
        ];

        // Table de filtrage multicast a zero.
        for i in 0..128 {
            reg_write(REG_MTA + i * 4, 0);
        }

        // --- Anneau de reception ---
        let (rx_ring_p, rx_ring_v) = match memory::alloc_dma(N_RX * DESC_SZ) {
            Some(v) => v, None => { dmesg::log("e1000: alloc DMA RX echec"); return false; }
        };
        let (rx_buf_p, rx_buf_v) = match memory::alloc_dma(N_RX * BUF) {
            Some(v) => v, None => { dmesg::log("e1000: alloc DMA RX buf echec"); return false; }
        };
        RX_RING = rx_ring_v;
        RX_BUF_V = rx_buf_v;
        RX_BUF_P = rx_buf_p;
        for i in 0..N_RX {
            desc_set_u64(RX_RING, i, 0, rx_buf_p + (i * BUF) as u64);
            desc_set_u8(RX_RING, i, 12, 0); // status
        }
        reg_write(REG_RDBAL, rx_ring_p as u32);
        reg_write(REG_RDBAH, (rx_ring_p >> 32) as u32);
        reg_write(REG_RDLEN, (N_RX * DESC_SZ) as u32);
        reg_write(REG_RDH, 0);
        reg_write(REG_RDT, (N_RX - 1) as u32);
        RX_CUR = 0;
        // RCTL: EN | BAM | UPE | MPE | SECRC ; BSIZE 2048 (bits 16-17 = 00).
        reg_write(REG_RCTL, 0x2 | 0x8000 | 0x8 | 0x10 | 0x0400_0000);

        // --- Anneau d'emission ---
        let (tx_ring_p, tx_ring_v) = match memory::alloc_dma(N_TX * DESC_SZ) {
            Some(v) => v, None => { dmesg::log("e1000: alloc DMA TX echec"); return false; }
        };
        let (tx_buf_p, tx_buf_v) = match memory::alloc_dma(N_TX * BUF) {
            Some(v) => v, None => { dmesg::log("e1000: alloc DMA TX buf echec"); return false; }
        };
        TX_RING = tx_ring_v;
        TX_BUF_V = tx_buf_v;
        TX_BUF_P = tx_buf_p;
        for i in 0..N_TX {
            desc_set_u64(TX_RING, i, 0, tx_buf_p + (i * BUF) as u64);
            desc_set_u8(TX_RING, i, 12, 1); // status DD = libre
        }
        reg_write(REG_TDBAL, tx_ring_p as u32);
        reg_write(REG_TDBAH, (tx_ring_p >> 32) as u32);
        reg_write(REG_TDLEN, (N_TX * DESC_SZ) as u32);
        reg_write(REG_TDH, 0);
        reg_write(REG_TDT, 0);
        TX_CUR = 0;
        // TCTL: EN | PSP | CT=0x0F | COLD=0x40.
        reg_write(REG_TCTL, 0x2 | 0x8 | (0x0F << 4) | (0x40 << 12));
        reg_write(REG_TIPG, 0x0060_200A);

        // Active la liaison (Set Link Up + Auto-Speed Detect).
        reg_write(REG_CTRL, reg_read(REG_CTRL) | 0x40 | 0x20);

        READY = true;
    }
    dmesg::log("e1000: initialise (RX/TX prets)");
    true
}

/// Lien physique etabli ?
pub fn link_up() -> bool {
    unsafe {
        if !READY { return false; }
        reg_read(REG_STATUS) & 0x2 != 0 // STATUS.LU
    }
}

/// Emet une trame Ethernet complete. Renvoie false si non prete/trop grande.
pub fn send(frame: &[u8]) -> bool {
    unsafe {
        if !READY || frame.is_empty() || frame.len() > BUF { return false; }
        let i = TX_CUR;
        // Copie la trame dans le tampon DMA de ce descripteur.
        let dst = TX_BUF_V.add(i * BUF);
        core::ptr::copy_nonoverlapping(frame.as_ptr(), dst, frame.len());
        desc_set_u64(TX_RING, i, 0, TX_BUF_P + (i * BUF) as u64);
        desc_set_u16(TX_RING, i, 8, frame.len() as u16); // length
        desc_set_u8(TX_RING, i, 11, 0x1 | 0x2 | 0x8);     // cmd: EOP|IFCS|RS
        desc_set_u8(TX_RING, i, 12, 0);                   // status
        TX_CUR = (i + 1) % N_TX;
        reg_write(REG_TDT, TX_CUR as u32);
        // Attend que la carte signale l'envoi (DD), avec garde-fou.
        for _ in 0..1_000_000 {
            if desc_get_u8(TX_RING, i, 12) & 0x1 != 0 { return true; }
            core::arch::asm!("pause", options(nomem, nostack));
        }
        true
    }
}

/// Tente de recevoir une trame ; copie dans `out`, renvoie sa longueur.
pub fn receive(out: &mut [u8]) -> Option<usize> {
    unsafe {
        if !READY { return None; }
        let i = RX_CUR;
        let status = desc_get_u8(RX_RING, i, 12);
        if status & 0x1 == 0 { return None; } // pas de DD
        let len = desc_get_u16(RX_RING, i, 8) as usize;
        let n = len.min(out.len());
        let src = RX_BUF_V.add(i * BUF);
        core::ptr::copy_nonoverlapping(src, out.as_mut_ptr(), n);
        desc_set_u8(RX_RING, i, 12, 0); // libere le descripteur
        reg_write(REG_RDT, i as u32);
        RX_CUR = (i + 1) % N_RX;
        Some(n)
    }
}

/// Affiche l'etat de la carte (commande `ethinfo`).
pub fn print_info() {
    if !is_ready() {
        crate::println!("e1000: non initialise (lance 'ifup')");
        return;
    }
    let m = mac();
    crate::println!("e1000: MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}  lien={}",
        m[0], m[1], m[2], m[3], m[4], m[5], if link_up() { "UP" } else { "DOWN" });
}
