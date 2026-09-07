//! xHCI + USB HID bring-up for the physical TRIGKEY reference machine.
//!
//! V3.3 takes ownership of every reachable xHCI controller, powers/resets root
//! ports, builds command/event rings, enumerates directly-attached USB devices,
//! configures HID boot keyboard/mouse interrupt endpoints, and polls them from
//! the GUI event loop. Interrupt/MSI-x support can replace polling later without
//! changing the input-facing contract.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;
use core::ptr::{copy_nonoverlapping, read_volatile, write_bytes, write_volatile};
use core::sync::atomic::{fence, AtomicBool, AtomicUsize, Ordering};

use crate::arch::x86_64::pci::{self, PciDevice};
use crate::kernel::memory;

const USBCMD_RUN: u32 = 1 << 0;
const USBCMD_HCRST: u32 = 1 << 1;
const USBSTS_HCH: u32 = 1 << 0;
const USBSTS_CNR: u32 = 1 << 11;

const HCC_CSZ: u32 = 1 << 2;
const HCC_PPC: u32 = 1 << 3;

const PORTSC_CCS: u32 = 1 << 0;
const PORTSC_PED: u32 = 1 << 1;
const PORTSC_PR: u32 = 1 << 4;
const PORTSC_PP: u32 = 1 << 9;
const PORTSC_SPEED_SHIFT: u32 = 10;
const PORTSC_WPR: u32 = 1 << 31;
const PORTSC_CHANGE_BITS: u32 =
    (1 << 17) | (1 << 18) | (1 << 19) | (1 << 20) | (1 << 21) | (1 << 22) | (1 << 23);

const TRB_SIZE: usize = 16;
const RING_BYTES: usize = 4096;
const TRBS_PER_RING: usize = RING_BYTES / TRB_SIZE;
const TRB_CYCLE: u32 = 1;
const TRB_CHAIN: u32 = 1 << 4;
const TRB_IOC: u32 = 1 << 5;
const TRB_ISP: u32 = 1 << 2;
const TRB_IDT: u32 = 1 << 6;
const TRB_DIR_IN: u32 = 1 << 16;
const LINK_TRB_TYPE: u32 = 6;
const LINK_TC: u32 = 1 << 1;

const TRB_NORMAL: u32 = 1;
const TRB_SETUP_STAGE: u32 = 2;
const TRB_DATA_STAGE: u32 = 3;
const TRB_STATUS_STAGE: u32 = 4;
const CMD_ENABLE_SLOT: u32 = 9;
const CMD_DISABLE_SLOT: u32 = 10;
const CMD_ADDRESS_DEVICE: u32 = 11;
const CMD_CONFIGURE_ENDPOINT: u32 = 12;
const CMD_EVALUATE_CONTEXT: u32 = 13;
const EVT_TRANSFER: u32 = 32;
const EVT_COMMAND_COMPLETION: u32 = 33;
const EVT_PORT_STATUS_CHANGE: u32 = 34;

const CC_SUCCESS: u8 = 1;
const CC_SHORT_PACKET: u8 = 13;

const MAX_PORTS_PER_CONTROLLER: usize = 32;
const MAX_HID_ENDPOINTS_PER_CONTROLLER: usize = 16;
const MAX_CONFIG_DESCRIPTOR: usize = 4096;
const MAX_EVENTS_PER_POLL: usize = 64;
const MAX_RUNTIME_DEVICES: usize = 32;
const WAIT_SPINS: usize = 30_000_000;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static CONNECTED: AtomicUsize = AtomicUsize::new(0);
static ENABLED: AtomicUsize = AtomicUsize::new(0);
static ADDRESS_OK: AtomicUsize = AtomicUsize::new(0);
static CONTROL_OK: AtomicUsize = AtomicUsize::new(0);
static USB_DEVICES: AtomicUsize = AtomicUsize::new(0);
static HID_KEYBOARDS: AtomicUsize = AtomicUsize::new(0);
static HID_MICE: AtomicUsize = AtomicUsize::new(0);
// BOUCHAUD_XHCI_HID_TRANSPORT_V33
static HID_POLLS: AtomicUsize = AtomicUsize::new(0);
static HID_TRANSFER_EVENTS: AtomicUsize = AtomicUsize::new(0);
static HID_REPORTS: AtomicUsize = AtomicUsize::new(0);
static HID_KEYBOARD_REPORTS: AtomicUsize = AtomicUsize::new(0);
static HID_MOUSE_REPORTS: AtomicUsize = AtomicUsize::new(0);
static HID_TRANSFER_ERRORS: AtomicUsize = AtomicUsize::new(0);
static HID_REARMS: AtomicUsize = AtomicUsize::new(0);
static HID_KICKS: AtomicUsize = AtomicUsize::new(0);
static HID_CONTROL_POLLS: AtomicUsize = AtomicUsize::new(0);
static HID_CONTROL_REPORTS: AtomicUsize = AtomicUsize::new(0);
static HID_CONTROL_FAILS: AtomicUsize = AtomicUsize::new(0);
static CONTROLLERS: AtomicUsize = AtomicUsize::new(0);
static RUNTIME_BUSY: AtomicBool = AtomicBool::new(false);
static mut RUNTIME: Option<Runtime> = None;

#[derive(Clone, Debug)]
pub struct ActiveSummary {
    pub present: bool,
    pub active: bool,
    pub controllers_seen: usize,
    pub controllers_active: usize,
    pub bar0: u64,
    pub hci_version: u16,
    pub max_slots: u8,
    pub max_ports: u8,
    pub scratchpads: usize,
    pub connected_ports: usize,
    pub enabled_ports: usize,
    pub usb_devices: usize,
    pub hid_keyboards: usize,
    pub hid_mice: usize,
    pub error: Option<&'static str>,
}

impl ActiveSummary {
    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "xHCI USB HID PHYSICAL V3");
        let _ = writeln!(out, "present={}", self.present);
        let _ = writeln!(out, "active={}", self.active);
        let _ = writeln!(out, "controllers_seen={}", self.controllers_seen);
        let _ = writeln!(out, "controllers_active={}", self.controllers_active);
        let _ = writeln!(out, "bar0_first={:#x}", self.bar0);
        let _ = writeln!(out, "hci_version_first={:#06x}", self.hci_version);
        let _ = writeln!(out, "max_slots_first={}", self.max_slots);
        let _ = writeln!(out, "max_ports_first={}", self.max_ports);
        let _ = writeln!(out, "scratchpads_total={}", self.scratchpads);
        let _ = writeln!(out, "connected_ports={}", self.connected_ports);
        let _ = writeln!(out, "enabled_ports={}", self.enabled_ports);
        let _ = writeln!(out, "usb_devices={}", self.usb_devices);
        let _ = writeln!(out, "hid_keyboards={}", self.hid_keyboards);
        let _ = writeln!(out, "hid_mice={}", self.hid_mice);
        let _ = writeln!(out, "polling={}", self.hid_keyboards + self.hid_mice != 0);
        let _ = writeln!(out, "error={}", self.error.unwrap_or("none"));
        let _ = writeln!(out, "next=MSI-X/interrupt driven xHCI + hub/hotplug expansion");
        out
    }
}

#[derive(Clone, Copy, Default)]
struct Trb {
    parameter: u64,
    status: u32,
    control: u32,
}

#[derive(Clone, Copy)]
struct ProducerRing {
    phys: u64,
    virt: usize,
    index: usize,
    cycle: u32,
}

#[derive(Clone, Copy)]
struct EventRing {
    phys: u64,
    virt: usize,
    index: usize,
    cycle: u32,
}

#[derive(Clone, Copy)]
struct HidDescriptor {
    interface: u8,
    subclass: u8,
    protocol: u8,
    kind: u8, // 1 keyboard, 2 mouse, 0 unknown HID
    report_len: u16,
    report_id: u8,
    endpoint_address: u8,
    max_packet: u16,
    interval: u8,
}

const EMPTY_HID_DESCRIPTOR: HidDescriptor = HidDescriptor {
    interface: 0,
    subclass: 0,
    protocol: 0,
    kind: 0,
    report_len: 0,
    report_id: 0,
    endpoint_address: 0,
    max_packet: 0,
    interval: 0,
};

#[derive(Clone, Copy)]
struct HidEndpoint {
    active: bool,
    slot_id: u8,
    dci: u8,
    interface: u8,
    protocol: u8,
    kind: u8,
    report_id: u8,
    max_packet: u16,
    ring: ProducerRing,
    buffer_phys: u64,
    buffer_virt: usize,
    buffer_len: usize,
    last_modifiers: u8,
    last_keys: [u8; 6],
}

const EMPTY_RING: ProducerRing = ProducerRing {
    phys: 0,
    virt: 0,
    index: 0,
    cycle: 1,
};

const EMPTY_HID_ENDPOINT: HidEndpoint = HidEndpoint {
    active: false,
    slot_id: 0,
    dci: 0,
    interface: 0,
    protocol: 0,
    kind: 0,
    report_id: 0,
    max_packet: 0,
    ring: EMPTY_RING,
    buffer_phys: 0,
    buffer_virt: 0,
    buffer_len: 0,
    last_modifiers: 0,
    last_keys: [0; 6],
};

struct Controller {
    dev: PciDevice,
    base: usize,
    op: usize,
    doorbells: usize,
    intr0: usize,
    hci_version: u16,
    max_slots: u8,
    max_ports: u8,
    context_size: usize,
    ppc: bool,
    slot_types: [u8; MAX_PORTS_PER_CONTROLLER],
    port_major: [u8; MAX_PORTS_PER_CONTROLLER],
    command: ProducerRing,
    events: EventRing,
    dcbaa_phys: u64,
    dcbaa_virt: usize,
    hids: [HidEndpoint; MAX_HID_ENDPOINTS_PER_CONTROLLER],
    hid_count: usize,
    devices: [Option<Device>; MAX_RUNTIME_DEVICES],
    connected_ports: usize,
    enabled_ports: usize,
    usb_devices: usize,
}

struct Runtime {
    controllers: Vec<Controller>,
}

#[derive(Clone, Copy)]
struct Device {
    slot_id: u8,
    root_port: u8,
    speed: u8,
    ep0_mps: u16,
    out_ctx_phys: u64,
    out_ctx_virt: usize,
    in_ctx_phys: u64,
    in_ctx_virt: usize,
    ep0: ProducerRing,
    control_phys: u64,
    control_virt: usize,
}

#[inline]
unsafe fn r8(base: usize, off: usize) -> u8 {
    read_volatile((base + off) as *const u8)
}
#[inline]
unsafe fn r16(base: usize, off: usize) -> u16 {
    read_volatile((base + off) as *const u16)
}
#[inline]
unsafe fn r32(base: usize, off: usize) -> u32 {
    read_volatile((base + off) as *const u32)
}
#[inline]
unsafe fn w32(base: usize, off: usize, value: u32) {
    write_volatile((base + off) as *mut u32, value);
}
#[inline]
unsafe fn w64(base: usize, off: usize, value: u64) {
    write_volatile((base + off) as *mut u64, value);
}

fn wait_until(mut predicate: impl FnMut() -> bool) -> bool {
    for _ in 0..WAIT_SPINS {
        if predicate() {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

fn wait_ms(ms: u64) {
    let start = crate::kernel::timer::monotonic_ns();
    let deadline = start.saturating_add(ms.saturating_mul(1_000_000));
    if start != 0 {
        while crate::kernel::timer::monotonic_ns() < deadline {
            core::hint::spin_loop();
        }
    } else {
        for _ in 0..ms.saturating_mul(100_000) {
            core::hint::spin_loop();
        }
    }
}

fn max_scratchpads(hcs2: u32) -> usize {
    // xHCI HCSPARAMS2: Max Scratchpad Buffers Hi = 31:27, Lo = 25:21.
    let hi = ((hcs2 >> 27) & 0x1f) as usize;
    let lo = ((hcs2 >> 21) & 0x1f) as usize;
    (hi << 5) | lo
}

unsafe fn legacy_handoff(base: usize, hccparams1: u32) -> bool {
    let mut off = (((hccparams1 >> 16) & 0xffff) as usize) * 4;
    for _ in 0..64 {
        if off == 0 {
            return true;
        }
        let header = r32(base, off);
        let cap_id = (header & 0xff) as u8;
        let next = ((header >> 8) & 0xff) as usize;
        if cap_id == 1 {
            // OS Owned Semaphore. Wait until BIOS Owned is released.
            w32(base, off, header | (1 << 24));
            if header & (1 << 16) == 0 {
                return true;
            }
            return wait_until(|| r32(base, off) & (1 << 16) == 0);
        }
        if next == 0 {
            return true;
        }
        off = off.saturating_add(next * 4);
    }
    false
}

fn alloc_zeroed(bytes: usize) -> Option<(u64, usize)> {
    let (phys, virt) = memory::alloc_dma(bytes)?;
    unsafe { write_bytes(virt, 0, bytes) };
    Some((phys, virt as usize))
}

fn trb_type(control: u32) -> u32 {
    (control >> 10) & 0x3f
}

fn completion_code(status: u32) -> u8 {
    (status >> 24) as u8
}

fn event_slot(event: Trb) -> u8 {
    (event.control >> 24) as u8
}

fn event_dci(event: Trb) -> u8 {
    ((event.control >> 16) & 0x1f) as u8
}

unsafe fn write_trb(virt: usize, index: usize, trb: Trb, cycle: u32) {
    let ptr = virt + index * TRB_SIZE;
    write_volatile(ptr as *mut u64, trb.parameter);
    write_volatile((ptr + 8) as *mut u32, trb.status);
    fence(Ordering::Release);
    write_volatile((ptr + 12) as *mut u32, (trb.control & !TRB_CYCLE) | cycle);
}

unsafe fn prepare_link(ring: &ProducerRing, cycle: u32) {
    let index = TRBS_PER_RING - 1;
    let ptr = ring.virt + index * TRB_SIZE;
    write_volatile(ptr as *mut u64, ring.phys);
    write_volatile((ptr + 8) as *mut u32, 0);
    fence(Ordering::Release);
    write_volatile(
        (ptr + 12) as *mut u32,
        (LINK_TRB_TYPE << 10) | LINK_TC | cycle,
    );
}

fn ring_push(ring: &mut ProducerRing, trb: Trb) -> u64 {
    unsafe {
        let pointer = ring.phys + (ring.index * TRB_SIZE) as u64;
        write_trb(ring.virt, ring.index, trb, ring.cycle);
        ring.index += 1;
        if ring.index == TRBS_PER_RING - 1 {
            prepare_link(ring, ring.cycle);
            ring.index = 0;
            ring.cycle ^= 1;
        }
        pointer
    }
}

fn alloc_producer_ring() -> Option<ProducerRing> {
    let (phys, virt) = alloc_zeroed(RING_BYTES)?;
    let ring = ProducerRing {
        phys,
        virt,
        index: 0,
        cycle: 1,
    };
    unsafe { prepare_link(&ring, 1) };
    Some(ring)
}

fn next_event(controller: &mut Controller) -> Option<Trb> {
    unsafe {
        let ptr = controller.events.virt + controller.events.index * TRB_SIZE;
        let control = read_volatile((ptr + 12) as *const u32);
        if control & TRB_CYCLE != controller.events.cycle {
            return None;
        }
        fence(Ordering::Acquire);
        let event = Trb {
            parameter: read_volatile(ptr as *const u64),
            status: read_volatile((ptr + 8) as *const u32),
            control,
        };

        controller.events.index += 1;
        if controller.events.index == TRBS_PER_RING {
            controller.events.index = 0;
            controller.events.cycle ^= 1;
        }
        let erdp = controller.events.phys
            + (controller.events.index * TRB_SIZE) as u64
            | (1 << 3); // clear EHB while advancing ERDP
        w64(controller.intr0, 0x18, erdp);
        Some(event)
    }
}

fn wait_event(controller: &mut Controller, wanted_type: u32, slot: Option<u8>, dci: Option<u8>) -> Option<Trb> {
    for _ in 0..WAIT_SPINS {
        if let Some(event) = next_event(controller) {
            let ty = trb_type(event.control);
            if ty == EVT_PORT_STATUS_CHANGE {
                continue;
            }
            if ty == wanted_type
                && slot.map_or(true, |s| event_slot(event) == s)
                && dci.map_or(true, |e| event_dci(event) == e)
            {
                return Some(event);
            }
        }
        core::hint::spin_loop();
    }
    None
}

fn ring_doorbell(controller: &Controller, slot: u8, target: u8) {
    fence(Ordering::Release);
    unsafe { w32(controller.doorbells, slot as usize * 4, target as u32) };
}

fn command_raw(controller: &mut Controller, parameter: u64, control: u32) -> Result<Trb, &'static str> {
    let pointer = ring_push(
        &mut controller.command,
        Trb { parameter, status: 0, control },
    );
    ring_doorbell(controller, 0, 0);
    let event = wait_event(controller, EVT_COMMAND_COMPLETION, None, None).ok_or("command-timeout")?;
    if event.parameter & !0xf != pointer & !0xf {
        // Command completions are ordered. A mismatched pointer means the event
        // stream is no longer the one we submitted; fail closed.
        return Err("command-pointer-mismatch");
    }
    let cc = completion_code(event.status);
    if cc != CC_SUCCESS {
        crate::serial_println!(
            "BOUCHAUD_XHCI_COMMAND_FAIL type={} slot={} cc={} event_status={:#010x}",
            trb_type(control), (control >> 24) as u8, cc, event.status,
        );
        return Err("command-completion-error");
    }
    Ok(event)
}

fn command(controller: &mut Controller, parameter: u64, command_type: u32, slot: u8) -> Result<Trb, &'static str> {
    command_raw(
        controller,
        parameter,
        (command_type << 10) | ((slot as u32) << 24),
    )
}

fn enable_slot(controller: &mut Controller, root_port: u8) -> Result<u8, &'static str> {
    let slot_type = controller
        .slot_types
        .get(root_port.saturating_sub(1) as usize)
        .copied()
        .unwrap_or(0);
    let event = command_raw(
        controller,
        0,
        (CMD_ENABLE_SLOT << 10) | ((slot_type as u32) << 16),
    )?;
    let slot = event_slot(event);
    if slot == 0 {
        Err("enable-slot-zero")
    } else {
        Ok(slot)
    }
}

fn disable_slot(controller: &mut Controller, slot: u8) {
    let _ = command(controller, 0, CMD_DISABLE_SLOT, slot);
}

fn context_ptr(base: usize, context_size: usize, index: usize) -> usize {
    base + context_size * index
}

unsafe fn ctx_r32(ctx: usize, dword: usize) -> u32 {
    read_volatile((ctx + dword * 4) as *const u32)
}
unsafe fn ctx_w32(ctx: usize, dword: usize, value: u32) {
    write_volatile((ctx + dword * 4) as *mut u32, value);
}
unsafe fn ctx_w64(ctx: usize, qword: usize, value: u64) {
    write_volatile((ctx + qword * 8) as *mut u64, value);
}

fn initial_ep0_mps(speed: u8) -> u16 {
    // Robust defaults used by mature xHCI stacks: LS=8, FS/HS=64, SS=512.
    match speed {
        2 => 8,
        1 | 3 => 64,
        4 | 5 => 512,
        _ => 64,
    }
}

fn fill_slot_context(ctx: usize, speed: u8, root_port: u8, context_entries: u8) {
    unsafe {
        ctx_w32(
            ctx,
            0,
            ((speed as u32) << 20) | ((context_entries as u32) << 27),
        );
        ctx_w32(ctx, 1, (root_port as u32) << 16);
        ctx_w32(ctx, 2, 0);
        ctx_w32(ctx, 3, 0);
    }
}

fn fill_endpoint_context(ctx: usize, ep_type: u8, max_packet: u16, interval: u8, ring: &ProducerRing) {
    unsafe {
        ctx_w32(ctx, 0, (interval as u32) << 16);
        let force_event = if ep_type == 7 { 1 } else { 0 };
        ctx_w32(
            ctx,
            1,
            force_event | (3 << 1) | ((ep_type as u32) << 3) | ((max_packet as u32) << 16),
        );
        ctx_w64(ctx, 1, ring.phys | 1); // TR Dequeue Pointer + DCS
        // xHCI 1.2 §6.2.3: EP0 Average TRB Length must be 8 bytes.
        let average = if ep_type == 4 { 8 } else { max_packet.max(1) as u32 };
        let max_esit = if ep_type == 7 { (max_packet as u32) << 16 } else { 0 };
        ctx_w32(ctx, 4, average | max_esit);
    }
}

fn clear_input(device: &Device) {
    unsafe { write_bytes(device.in_ctx_virt as *mut u8, 0, 4096) };
}

fn address_device(controller: &mut Controller, root_port: u8, speed: u8) -> Result<Device, &'static str> {
    let slot_id = enable_slot(controller, root_port)?;
    let Some((out_ctx_phys, out_ctx_virt)) = alloc_zeroed(4096) else {
        disable_slot(controller, slot_id);
        return Err("dma-output-context");
    };
    let Some((in_ctx_phys, in_ctx_virt)) = alloc_zeroed(4096) else {
        disable_slot(controller, slot_id);
        return Err("dma-input-context");
    };
    let Some(ep0) = alloc_producer_ring() else {
        disable_slot(controller, slot_id);
        return Err("dma-ep0-ring");
    };
    let Some((control_phys, control_virt)) = alloc_zeroed(4096) else {
        disable_slot(controller, slot_id);
        return Err("dma-control-buffer");
    };

    unsafe {
        write_volatile(
            (controller.dcbaa_virt + slot_id as usize * 8) as *mut u64,
            out_ctx_phys,
        );
        // Input Control Context: add Slot Context + EP0 Context.
        write_volatile((in_ctx_virt + 4) as *mut u32, (1 << 0) | (1 << 1));
    }
    let slot_ctx = context_ptr(in_ctx_virt, controller.context_size, 1);
    let ep0_ctx = context_ptr(in_ctx_virt, controller.context_size, 2);
    let ep0_mps = initial_ep0_mps(speed);
    fill_slot_context(slot_ctx, speed, root_port, 1);
    fill_endpoint_context(ep0_ctx, 4, ep0_mps, 0, &ep0);

    if let Err(error) = command(
        controller,
        in_ctx_phys,
        CMD_ADDRESS_DEVICE,
        slot_id,
    ) {
        disable_slot(controller, slot_id);
        return Err(error);
    }
    // Respect the SET_ADDRESS recovery interval before the first EP0 request.
    wait_ms(2);
    ADDRESS_OK.fetch_add(1, Ordering::Relaxed);
    crate::serial_println!(
        "BOUCHAUD_USB_ADDRESS_OK port={} slot={} speed={} ep0_mps={}",
        root_port, slot_id, speed, ep0_mps
    );

    Ok(Device {
        slot_id,
        root_port,
        speed,
        ep0_mps,
        out_ctx_phys,
        out_ctx_virt,
        in_ctx_phys,
        in_ctx_virt,
        ep0,
        control_phys,
        control_virt,
    })
}

fn setup_packet(bm_request: u8, request: u8, value: u16, index: u16, length: u16) -> u64 {
    (bm_request as u64)
        | ((request as u64) << 8)
        | ((value as u64) << 16)
        | ((index as u64) << 32)
        | ((length as u64) << 48)
}

fn control_transfer(
    controller: &mut Controller,
    device: &mut Device,
    setup: u64,
    data_len: usize,
    data_in: bool,
) -> Result<usize, &'static str> {
    if data_len > 4096 {
        return Err("control-buffer-too-small");
    }
    unsafe { write_bytes(device.control_virt as *mut u8, 0, data_len) };

    let trt = if data_len == 0 {
        0
    } else if data_in {
        3
    } else {
        2
    };
    ring_push(
        &mut device.ep0,
        Trb {
            parameter: setup,
            status: 8,
            // Setup Stage TD is exactly one Setup TRB: CH must remain zero.
            control: (TRB_SETUP_STAGE << 10) | TRB_IDT | (trt << 16),
        },
    );

    if data_len != 0 {
        ring_push(
            &mut device.ep0,
            Trb {
                parameter: device.control_phys,
                status: data_len as u32,
                // One contiguous data buffer => one Data Stage TRB: CH=0.
                control: (TRB_DATA_STAGE << 10)
                    | if data_in { TRB_DIR_IN } else { 0 },
            },
        );
    }

    let status_dir = if data_len == 0 || !data_in { TRB_DIR_IN } else { 0 };
    ring_push(
        &mut device.ep0,
        Trb {
            parameter: 0,
            status: 0,
            control: (TRB_STATUS_STAGE << 10) | TRB_IOC | status_dir,
        },
    );
    ring_doorbell(controller, device.slot_id, 1);

    let event = wait_event(
        controller,
        EVT_TRANSFER,
        Some(device.slot_id),
        Some(1),
    )
    .ok_or("control-transfer-timeout")?;
    let cc = completion_code(event.status);
    if cc != CC_SUCCESS && cc != CC_SHORT_PACKET {
        crate::serial_println!(
            "BOUCHAUD_USB_CONTROL_FAIL slot={} ep=1 cc={} setup={:#018x} len={} status={:#010x}",
            device.slot_id, cc, setup, data_len, event.status
        );
        return Err("control-transfer-error");
    }
    CONTROL_OK.fetch_add(1, Ordering::Relaxed);
    let residual = (event.status & 0x00ff_ffff) as usize;
    Ok(data_len.saturating_sub(residual.min(data_len)))
}

fn get_descriptor(
    controller: &mut Controller,
    device: &mut Device,
    descriptor_type: u8,
    index: u8,
    length: usize,
) -> Result<usize, &'static str> {
    let setup = setup_packet(
        0x80,
        6,
        ((descriptor_type as u16) << 8) | index as u16,
        0,
        length as u16,
    );
    control_transfer(controller, device, setup, length, true)
}

fn evaluate_ep0_mps(controller: &mut Controller, device: &mut Device, mps: u16) -> Result<(), &'static str> {
    if mps == device.ep0_mps {
        return Ok(());
    }
    clear_input(device);
    let ep0_out = context_ptr(device.out_ctx_virt, controller.context_size, 1);
    let ep0_ctx = context_ptr(device.in_ctx_virt, controller.context_size, 2);
    unsafe {
        // Evaluate Context: only EP0 is evaluated. Start from the controller's
        // current output context so dequeue/state fields remain identical; only
        // Max Packet Size is changed after the first 8-byte descriptor read.
        write_volatile((device.in_ctx_virt + 4) as *mut u32, 1 << 1);
        copy_nonoverlapping(
            ep0_out as *const u8,
            ep0_ctx as *mut u8,
            controller.context_size,
        );
        let dw1 = ctx_r32(ep0_ctx, 1);
        ctx_w32(ep0_ctx, 1, (dw1 & 0x0000_ffff) | ((mps as u32) << 16));
    }
    command(
        controller,
        device.in_ctx_phys,
        CMD_EVALUATE_CONTEXT,
        device.slot_id,
    )?;
    device.ep0_mps = mps;
    Ok(())
}

fn set_configuration(controller: &mut Controller, device: &mut Device, value: u8) -> Result<(), &'static str> {
    let setup = setup_packet(0x00, 9, value as u16, 0, 0);
    let _ = control_transfer(controller, device, setup, 0, false)?;
    Ok(())
}

fn set_boot_protocol(controller: &mut Controller, device: &mut Device, interface: u8) -> bool {
    let setup = setup_packet(0x21, 0x0b, 0, interface as u16, 0);
    control_transfer(controller, device, setup, 0, false).is_ok()
}

fn set_idle(controller: &mut Controller, device: &mut Device, interface: u8) {
    // HID SET_IDLE duration=0/report-id=0. Boot keyboards do not require this
    // for state changes, but several real devices only start cleanly after the
    // host has completed the class initialization sequence used by PC stacks.
    let setup = setup_packet(0x21, 0x0a, 0, interface as u16, 0);
    let _ = control_transfer(controller, device, setup, 0, false);
}

fn hid_item_value(bytes: &[u8]) -> u32 {
    let mut value = 0u32;
    for (shift, byte) in bytes.iter().take(4).enumerate() {
        value |= (*byte as u32) << (shift * 8);
    }
    value
}

fn classify_hid_report_descriptor(bytes: &[u8]) -> (u8, u8) {
    // Report-aware classifier. In addition to the canonical top-level Mouse /
    // Keyboard application usages, keep semantic hints seen in nested
    // collections. Several wireless receivers put Pointer/X/Y/Button usages in
    // a report collection whose top-level application is vendor-specific.
    let mut offset = 0usize;
    let mut usage_page = 0u32;
    let mut local_usage = 0u32;
    let mut kind = 0u8;
    let mut report_id = 0u8;
    let mut saw_pointer = false;
    let mut saw_x = false;
    let mut saw_y = false;
    let mut saw_buttons = false;
    let mut saw_keyboard_page = false;
    while offset < bytes.len() {
        let prefix = bytes[offset];
        offset += 1;
        if prefix == 0xfe {
            if offset + 2 > bytes.len() { break; }
            let size = bytes[offset] as usize;
            offset = offset.saturating_add(2).saturating_add(size);
            continue;
        }
        let size_code = (prefix & 0x03) as usize;
        let size = if size_code == 3 { 4 } else { size_code };
        if offset + size > bytes.len() { break; }
        let ty = (prefix >> 2) & 0x03;
        let tag = (prefix >> 4) & 0x0f;
        let value = hid_item_value(&bytes[offset..offset + size]);
        offset += size;
        match (ty, tag) {
            (1, 0) => {
                usage_page = value;
                if usage_page == 0x07 { saw_keyboard_page = true; }
                if usage_page == 0x09 { saw_buttons = true; }
            }
            (1, 8) => {
                if report_id == 0 { report_id = value as u8; }
            }
            (2, 0) => {
                local_usage = value;
                if usage_page == 0x01 {
                    if value == 0x01 || value == 0x02 { saw_pointer = true; }
                    if value == 0x30 { saw_x = true; }
                    if value == 0x31 { saw_y = true; }
                    if value == 0x06 { kind = 1; }
                }
            }
            (0, 10) => { // Collection
                if usage_page == 0x01 && local_usage == 0x02 {
                    kind = 2; // Generic Desktop / Mouse
                } else if usage_page == 0x01 && local_usage == 0x06 {
                    kind = 1; // Generic Desktop / Keyboard
                }
                local_usage = 0;
            }
            (0, _) => local_usage = 0,
            _ => {}
        }
    }
    if kind == 0 && saw_keyboard_page {
        kind = 1;
    }
    if kind == 0 && (saw_pointer || (saw_x && saw_y) || (saw_buttons && (saw_x || saw_y))) {
        kind = 2;
    }
    (kind, report_id)
}

fn get_hid_report_descriptor(
    controller: &mut Controller,
    device: &mut Device,
    interface: u8,
    length: usize,
) -> Result<usize, &'static str> {
    let length = length.min(4096);
    if length == 0 { return Err("hid-report-length-zero"); }
    let setup = setup_packet(
        0x81, // device-to-host, standard, interface
        6,    // GET_DESCRIPTOR
        (0x22u16) << 8,
        interface as u16,
        length as u16,
    );
    control_transfer(controller, device, setup, length, true)
}

fn enrich_hid_descriptor(
    controller: &mut Controller,
    device: &mut Device,
    descriptor: &mut HidDescriptor,
) {
    // Boot interfaces have an explicit protocol and are the most reliable path.
    descriptor.kind = match (descriptor.subclass, descriptor.protocol) {
        (1, 1) => 1,
        (1, 2) => 2,
        _ => 0,
    };

    // Report-only interfaces (subclass/protocol 0) are extremely common on
    // wireless receivers and gaming mice. Read their HID Report Descriptor and
    // classify the application collection instead of dropping them.
    if descriptor.kind == 0 && descriptor.report_len != 0 {
        match get_hid_report_descriptor(
            controller,
            device,
            descriptor.interface,
            descriptor.report_len as usize,
        ) {
            Ok(received) if received != 0 => {
                let bytes = unsafe {
                    core::slice::from_raw_parts(device.control_virt as *const u8, received)
                };
                let (kind, report_id) = classify_hid_report_descriptor(bytes);
                if descriptor.kind == 0 { descriptor.kind = kind; }
                descriptor.report_id = report_id;
                crate::serial_println!(
                    "BOUCHAUD_HID_REPORT_DESC slot={} if={} subclass={} protocol={} kind={} report_id={} bytes={}",
                    device.slot_id, descriptor.interface, descriptor.subclass,
                    descriptor.protocol, descriptor.kind, descriptor.report_id, received,
                );
            }
            _ => {
                crate::serial_println!(
                    "BOUCHAUD_HID_REPORT_DESC_FAIL slot={} if={} len={}",
                    device.slot_id, descriptor.interface, descriptor.report_len,
                );
            }
        }
    }
}

fn parse_hid_descriptors(bytes: &[u8], output: &mut [HidDescriptor]) -> (u8, usize) {
    if bytes.len() < 9 || bytes[1] != 2 {
        return (0, 0);
    }
    let configuration_value = bytes[5];
    let mut current_interface = 0u8;
    let mut current_subclass = 0u8;
    let mut current_protocol = 0u8;
    let mut current_hid = false;
    let mut current_report_len = 0u16;
    let mut count = 0usize;
    let mut offset = 0usize;

    while offset + 2 <= bytes.len() {
        let len = bytes[offset] as usize;
        let ty = bytes[offset + 1];
        if len < 2 || offset + len > bytes.len() {
            break;
        }
        match ty {
            4 if len >= 9 => {
                current_interface = bytes[offset + 2];
                let alternate = bytes[offset + 3];
                let class = bytes[offset + 5];
                current_subclass = bytes[offset + 6];
                current_protocol = bytes[offset + 7];
                current_hid = alternate == 0 && class == 3;
                current_report_len = 0;
                if current_hid {
                    crate::serial_println!(
                        "BOUCHAUD_HID_INTERFACE if={} subclass={} protocol={}",
                        current_interface, current_subclass, current_protocol,
                    );
                }
            }
            0x21 if current_hid && len >= 9 => {
                // HID descriptor: first subordinate descriptor is normally the
                // Report Descriptor (0x22), with its length in bytes 7..8.
                let count_desc = bytes[offset + 5] as usize;
                let mut pos = offset + 6;
                for _ in 0..count_desc {
                    if pos + 3 > offset + len { break; }
                    if bytes[pos] == 0x22 {
                        current_report_len = u16::from_le_bytes([bytes[pos + 1], bytes[pos + 2]]);
                        break;
                    }
                    pos += 3;
                }
            }
            5 if len >= 7 && current_hid && count < output.len() => {
                let address = bytes[offset + 2];
                let attributes = bytes[offset + 3] & 0x03;
                if address & 0x80 != 0 && attributes == 3 {
                    let kind = match (current_subclass, current_protocol) {
                        (1, 1) => 1,
                        (1, 2) => 2,
                        _ => 0,
                    };
                    output[count] = HidDescriptor {
                        interface: current_interface,
                        subclass: current_subclass,
                        protocol: current_protocol,
                        kind,
                        report_len: current_report_len,
                        report_id: 0,
                        endpoint_address: address,
                        max_packet: u16::from_le_bytes([
                            bytes[offset + 4],
                            bytes[offset + 5],
                        ]) & 0x07ff,
                        interval: bytes[offset + 6],
                    };
                    count += 1;
                }
            }
            _ => {}
        }
        offset += len;
    }
    (configuration_value, count)
}

fn xhci_interval(speed: u8, usb_interval: u8) -> u8 {
    if speed >= 3 {
        // High/Super: xHCI stores bInterval-1.
        return usb_interval.max(1).saturating_sub(1).min(15);
    }
    // Low/Full speed bInterval is expressed in 1ms frames. xHCI wants the
    // largest power-of-two microframe interval not exceeding that period:
    // floor(log2(8*bInterval)). This matches mature xHCI host stacks and fixes
    // the V3.2 ceil() off-by-one on non-power-of-two intervals (e.g. 10ms).
    let microframes = (usb_interval.max(1) as u32).saturating_mul(8);
    let log2 = (31u32.saturating_sub(microframes.leading_zeros())) as u8;
    log2.clamp(3, 15)
}

fn configure_hids(
    controller: &mut Controller,
    device: &mut Device,
    descriptors: &mut [HidDescriptor],
) -> Result<usize, &'static str> {
    if descriptors.is_empty() {
        return Ok(0);
    }
    for descriptor in descriptors.iter_mut() {
        enrich_hid_descriptor(controller, device, descriptor);
    }
    let available = MAX_HID_ENDPOINTS_PER_CONTROLLER.saturating_sub(controller.hid_count);
    let wanted = descriptors.len().min(available);
    if wanted == 0 {
        return Err("hid-endpoint-table-full");
    }

    clear_input(device);
    let slot_out = context_ptr(device.out_ctx_virt, controller.context_size, 0);
    let slot_in = context_ptr(device.in_ctx_virt, controller.context_size, 1);
    unsafe {
        copy_nonoverlapping(
            slot_out as *const u8,
            slot_in as *mut u8,
            controller.context_size,
        );
    }

    let mut add_flags = 1u32; // Slot Context
    let mut highest_dci = 1u8;
    let base_index = controller.hid_count;
    let mut installed = 0usize;

    for descriptor in descriptors.iter().take(wanted) {
        let mut kind = descriptor.kind;
        if kind == 0 && descriptor.max_packet >= 3 && descriptor.protocol != 1 {
            // Pragmatic physical fallback: an otherwise-unclassified HID IN
            // interface on this lab machine is treated as a mouse candidate.
            // This keeps vendor/report-protocol mice usable while the full HID
            // usage parser grows, without ever stealing a known keyboard.
            kind = 2;
            crate::serial_println!(
                "BOUCHAUD_HID_HEURISTIC_MOUSE if={} subclass={} protocol={} ep={:#04x}",
                descriptor.interface, descriptor.subclass, descriptor.protocol,
                descriptor.endpoint_address,
            );
        }
        if kind == 0 {
            crate::serial_println!(
                "BOUCHAUD_HID_UNSUPPORTED if={} subclass={} protocol={} ep={:#04x}",
                descriptor.interface, descriptor.subclass, descriptor.protocol,
                descriptor.endpoint_address,
            );
            continue;
        }
        let ep_number = descriptor.endpoint_address & 0x0f;
        if ep_number == 0 {
            continue;
        }
        let dci = ep_number.saturating_mul(2).saturating_add(1);
        if dci >= 32 || descriptor.max_packet == 0 {
            continue;
        }
        let Some(ring) = alloc_producer_ring() else {
            continue;
        };
        let Some((buffer_phys, buffer_virt)) = alloc_zeroed(4096) else {
            continue;
        };
        let buffer_len = (descriptor.max_packet as usize).clamp(3, 64);
        let ep_ctx = context_ptr(
            device.in_ctx_virt,
            controller.context_size,
            dci as usize + 1,
        );
        fill_endpoint_context(
            ep_ctx,
            7, // Interrupt IN
            descriptor.max_packet,
            xhci_interval(device.speed, descriptor.interval),
            &ring,
        );
        add_flags |= 1u32 << dci;
        highest_dci = highest_dci.max(dci);
        controller.hids[base_index + installed] = HidEndpoint {
            active: false,
            slot_id: device.slot_id,
            dci,
            interface: descriptor.interface,
            protocol: descriptor.protocol,
            kind,
            report_id: descriptor.report_id,
            max_packet: descriptor.max_packet,
            ring,
            buffer_phys,
            buffer_virt,
            buffer_len,
            last_modifiers: 0,
            last_keys: [0; 6],
        };
        installed += 1;
    }

    if installed == 0 {
        return Ok(0);
    }

    unsafe {
        write_volatile((device.in_ctx_virt + 4) as *mut u32, add_flags);
        let dw0 = ctx_r32(slot_in, 0);
        ctx_w32(
            slot_in,
            0,
            (dw0 & !(0x1f << 27)) | ((highest_dci as u32) << 27),
        );
    }

    command(
        controller,
        device.in_ctx_phys,
        CMD_CONFIGURE_ENDPOINT,
        device.slot_id,
    )?;

    for index in base_index..base_index + installed {
        let dci = controller.hids[index].dci;
        let out_ctx = context_ptr(device.out_ctx_virt, controller.context_size, dci as usize);
        let (state, dequeue) = unsafe {
            (ctx_r32(out_ctx, 0) & 0x7, read_volatile((out_ctx + 8) as *const u64))
        };
        crate::serial_println!(
            "BOUCHAUD_HID_ENDPOINT_STATE slot={} dci={} state={} deq={:#x} ring={:#x}",
            device.slot_id, dci, state, dequeue, controller.hids[index].ring.phys,
        );
    }

    for index in base_index..base_index + installed {
        let interface = controller.hids[index].interface;
        let protocol = controller.hids[index].protocol;
        if protocol == 1 || protocol == 2 {
            let boot = set_boot_protocol(controller, device, interface);
            crate::serial_println!(
                "BOUCHAUD_HID_SET_PROTOCOL slot={} if={} boot={}",
                device.slot_id, interface, boot as u8,
            );
        }
        set_idle(controller, device, interface);
    }
    wait_ms(10);

    for index in base_index..base_index + installed {
        // Arm only after every port on this controller has finished control
        // enumeration, so interrupt events cannot steal command/control events.
        controller.hids[index].active = true;
    }
    controller.hid_count += installed;
    Ok(installed)
}

fn arm_hid_endpoint(controller: &mut Controller, index: usize) {
    if index >= controller.hids.len() {
        return;
    }
    let (slot, dci) = {
        let endpoint = &mut controller.hids[index];
        unsafe { write_bytes(endpoint.buffer_virt as *mut u8, 0, endpoint.buffer_len) };
        ring_push(
            &mut endpoint.ring,
            Trb {
                parameter: endpoint.buffer_phys,
                status: endpoint.buffer_len as u32,
                // ISP matters for HID: many endpoints advertise 64-byte MPS
                // while keyboard/mouse reports are only 8/4 bytes. Ask for a
                // Transfer Event on that short packet as well as IOC.
                control: (TRB_NORMAL << 10) | TRB_IOC | TRB_ISP,
            },
        );
        (endpoint.slot_id, endpoint.dci)
    };
    fence(Ordering::SeqCst);
    ring_doorbell(controller, slot, dci);
    HID_REARMS.fetch_add(1, Ordering::Relaxed);
}

fn arm_all_hids(controller: &mut Controller) {
    for index in 0..controller.hid_count {
        if controller.hids[index].active {
            arm_hid_endpoint(controller, index);
        }
    }
}

fn enumerate_port(controller: &mut Controller, port_index: usize) {
    let port = (port_index + 1) as u8;
    let portsc = unsafe { r32(controller.op, 0x400 + port_index * 0x10) };
    if portsc & PORTSC_CCS == 0 { return; }
    if portsc & PORTSC_PED == 0 {
        crate::serial_println!(
            "BOUCHAUD_USB_ENUM_FAIL port={} phase=port-not-enabled portsc={:#010x}",
            port, portsc
        );
        return;
    }
    let speed = ((portsc >> PORTSC_SPEED_SHIFT) & 0x0f) as u8;
    let mut device = match address_device(controller, port, speed) {
        Ok(device) => device,
        Err(error) => {
            crate::serial_println!(
                "BOUCHAUD_USB_ENUM_FAIL port={} phase=address error={}",
                port,
                error
            );
            return;
        }
    };

    // Full-speed devices disclose their real EP0 MPS in the first 8 bytes.
    if device.speed <= 2 {
        match get_descriptor(controller, &mut device, 1, 0, 8) {
            Ok(received) if received >= 8 => {
                let encoded = unsafe { read_volatile((device.control_virt + 7) as *const u8) };
                let mps = if device.speed >= 4 {
                    1u16.checked_shl(encoded as u32).unwrap_or(512)
                } else {
                    encoded as u16
                };
                if matches!(mps, 8 | 16 | 32 | 64 | 512) {
                    if let Err(error) = evaluate_ep0_mps(controller, &mut device, mps) {
                        crate::serial_println!(
                            "BOUCHAUD_USB_ENUM_FAIL port={} phase=eval-mps error={}",
                            port,
                            error
                        );
                        disable_slot(controller, device.slot_id);
                        return;
                    }
                }
            }
            _ => {
                disable_slot(controller, device.slot_id);
                return;
            }
        }
    }

    let device_len = match get_descriptor(controller, &mut device, 1, 0, 18) {
        Ok(len) if len >= 18 => len,
        _ => {
            crate::serial_println!("BOUCHAUD_USB_ENUM_FAIL port={} phase=device-descriptor", port);
            disable_slot(controller, device.slot_id);
            return;
        }
    };
    let _ = device_len;
    let vendor = unsafe {
        u16::from_le_bytes([
            read_volatile((device.control_virt + 8) as *const u8),
            read_volatile((device.control_virt + 9) as *const u8),
        ])
    };
    let product = unsafe {
        u16::from_le_bytes([
            read_volatile((device.control_virt + 10) as *const u8),
            read_volatile((device.control_virt + 11) as *const u8),
        ])
    };

    let config_head = match get_descriptor(controller, &mut device, 2, 0, 9) {
        Ok(len) if len >= 9 => len,
        _ => {
            crate::serial_println!("BOUCHAUD_USB_ENUM_FAIL port={} phase=config-header", port);
            disable_slot(controller, device.slot_id);
            return;
        }
    };
    let _ = config_head;
    let total = unsafe {
        u16::from_le_bytes([
            read_volatile((device.control_virt + 2) as *const u8),
            read_volatile((device.control_virt + 3) as *const u8),
        ]) as usize
    }
    .clamp(9, MAX_CONFIG_DESCRIPTOR);

    let config_len = match get_descriptor(controller, &mut device, 2, 0, total) {
        Ok(len) if len >= 9 => len,
        _ => {
            crate::serial_println!("BOUCHAUD_USB_ENUM_FAIL port={} phase=config-descriptor", port);
            disable_slot(controller, device.slot_id);
            return;
        }
    };

    let mut descriptors = [EMPTY_HID_DESCRIPTOR; MAX_HID_ENDPOINTS_PER_CONTROLLER];
    let config_slice = unsafe {
        core::slice::from_raw_parts(device.control_virt as *const u8, config_len)
    };
    let (configuration_value, hid_count) = parse_hid_descriptors(config_slice, &mut descriptors);
    controller.usb_devices += 1;
    crate::serial_println!(
        "BOUCHAUD_USB_ENUM_OK port={} slot={} speed={} vid={:04x} pid={:04x} hid_in_endpoints={}",
        port,
        device.slot_id,
        speed,
        vendor,
        product,
        hid_count
    );

    if hid_count == 0 || configuration_value == 0 {
        // Device enumeration is green, but there is no HID endpoint to own.
        return;
    }

    if let Err(error) = set_configuration(controller, &mut device, configuration_value) {
        crate::serial_println!(
            "BOUCHAUD_HID_CONFIG_FAIL port={} phase=set-configuration error={}",
            port,
            error
        );
        return;
    }
    wait_ms(10);

    match configure_hids(controller, &mut device, &mut descriptors[..hid_count]) {
        Ok(installed) => {
            crate::serial_println!(
                "BOUCHAUD_HID_CONFIG_OK port={} endpoints={}",
                port,
                installed
            );
        }
        Err(error) => {
            crate::serial_println!(
                "BOUCHAUD_HID_CONFIG_FAIL port={} phase=configure-endpoint error={}",
                port,
                error
            );
        }
    }
    if (device.slot_id as usize) < controller.devices.len() {
        controller.devices[device.slot_id as usize] = Some(device);
    }
}

fn neutral_port_state(value: u32) -> u32 {
    // Do not replay RW1C/RW1S fields while changing power/reset bits.
    value & !(PORTSC_PED | PORTSC_PR | PORTSC_WPR | PORTSC_CHANGE_BITS)
}

fn wait_port_enabled(controller: &Controller, port_index: usize, timeout_ms: u64) -> bool {
    let off = 0x400 + port_index * 0x10;
    let start = crate::kernel::timer::monotonic_ns();
    let deadline = start.saturating_add(timeout_ms.saturating_mul(1_000_000));
    loop {
        let value = unsafe { r32(controller.op, off) };
        if value & PORTSC_CCS != 0 && value & PORTSC_PED != 0 { return true; }
        if value & PORTSC_CCS == 0 { return false; }
        if start != 0 && crate::kernel::timer::monotonic_ns() >= deadline { return false; }
        core::hint::spin_loop();
    }
}

fn reset_root_port(controller: &mut Controller, port_index: usize) -> bool {
    let off = 0x400 + port_index * 0x10;
    let before = unsafe { r32(controller.op, off) };
    if before & PORTSC_CCS == 0 { return false; }
    if before & PORTSC_PED != 0 { return true; }

    // USB2 ports use PR; USB3 ports use WPR. Choose from the Supported Protocol
    // capability instead of assuming a particular numeric Speed ID mapping.
    let major = controller.port_major[port_index];
    let reset_bit = if major >= 3 { PORTSC_WPR } else { PORTSC_PR };
    let power = before & PORTSC_PP;
    unsafe { w32(controller.op, off, neutral_port_state(before) | power | reset_bit) };
    if !wait_until(|| unsafe { r32(controller.op, off) } & reset_bit == 0) {
        crate::serial_println!(
            "BOUCHAUD_XHCI_PORT_RESET_FAIL port={} protocol={} reason=reset-bit-timeout before={:#010x}",
            port_index + 1, major, before
        );
        return false;
    }
    if wait_port_enabled(controller, port_index, 250) { return true; }
    let after = unsafe { r32(controller.op, off) };
    crate::serial_println!(
        "BOUCHAUD_XHCI_PORT_RESET_FAIL port={} protocol={} reason=ped-timeout after={:#010x}",
        port_index + 1, major, after
    );
    false
}

fn prepare_root_ports(controller: &mut Controller) {
    if controller.ppc {
        for port in 0..controller.max_ports.min(MAX_PORTS_PER_CONTROLLER as u8) as usize {
            let off = 0x400 + port * 0x10;
            let value = unsafe { r32(controller.op, off) };
            if value & PORTSC_PP == 0 {
                unsafe { w32(controller.op, off, neutral_port_state(value) | PORTSC_PP) };
            }
        }
        wait_ms(50);
    }
    // HCRST destroyed the firmware's host state; let attached devices settle.
    wait_ms(100);

    let mut connected = 0usize;
    let mut enabled = 0usize;
    for port in 0..controller.max_ports.min(MAX_PORTS_PER_CONTROLLER as u8) as usize {
        let off = 0x400 + port * 0x10;
        let before = unsafe { r32(controller.op, off) };
        if before & PORTSC_CCS == 0 {
            crate::serial_println!(
                "BOUCHAUD_XHCI_ROOT_PORT port={} protocol={} before={:#010x} connected=0",
                port + 1, controller.port_major[port], before
            );
            continue;
        }
        connected += 1;
        wait_ms(20);
        let reset_ok = reset_root_port(controller, port);
        let mut after = unsafe { r32(controller.op, off) };
        if reset_ok && after & PORTSC_PED != 0 { enabled += 1; }
        if after & PORTSC_CHANGE_BITS != 0 {
            unsafe {
                w32(controller.op, off,
                    neutral_port_state(after) | (after & PORTSC_PP) | (after & PORTSC_CHANGE_BITS));
            }
            after = unsafe { r32(controller.op, off) };
        }
        let speed = ((after >> PORTSC_SPEED_SHIFT) & 0x0f) as u8;
        crate::serial_println!(
            "BOUCHAUD_XHCI_ROOT_PORT port={} protocol={} connected=1 enabled={} reset_ok={} speed_id={} before={:#010x} after={:#010x}",
            port + 1, controller.port_major[port], (after & PORTSC_PED != 0) as u8,
            reset_ok as u8, speed, before, after
        );
    }
    controller.connected_ports = connected;
    controller.enabled_ports = enabled;
}

fn protocol_info(
    base: usize,
    hccparams1: u32,
    max_ports: u8,
) -> ([u8; MAX_PORTS_PER_CONTROLLER], [u8; MAX_PORTS_PER_CONTROLLER]) {
    let mut slot_types = [0u8; MAX_PORTS_PER_CONTROLLER];
    let mut major = [0u8; MAX_PORTS_PER_CONTROLLER];
    let mut off = (((hccparams1 >> 16) & 0xffff) as usize) * 4;
    for _ in 0..64 {
        if off == 0 { break; }
        let header = unsafe { r32(base, off) };
        let cap_id = (header & 0xff) as u8;
        let next = ((header >> 8) & 0xff) as usize;
        if cap_id == 2 {
            let protocol_major = ((header >> 24) & 0xff) as u8;
            let ports = unsafe { r32(base, off + 8) };
            let start = (ports & 0xff) as usize;
            let count = ((ports >> 8) & 0xff) as usize;
            let slot_type = (unsafe { r32(base, off + 12) } & 0x1f) as u8;
            for port in start..start.saturating_add(count) {
                if port != 0 && port <= max_ports as usize && port <= slot_types.len() {
                    slot_types[port - 1] = slot_type;
                    major[port - 1] = protocol_major;
                }
            }
        }
        if next == 0 { break; }
        off = off.saturating_add(next * 4);
    }
    (slot_types, major)
}

fn init_controller(dev: PciDevice) -> Result<(Controller, usize), &'static str> {
    let bar0 = pci::bar_decode(&dev, 0).adresse();
    if bar0 == 0 {
        return Err("bar0-absent");
    }
    pci::enable_bus_master(&dev);
    let base = memory::phys_to_virt(bar0) as usize;

    unsafe {
        let cap_len = r8(base, 0x00) as usize;
        let hci_version = r16(base, 0x02);
        let hcs1 = r32(base, 0x04);
        let hcs2 = r32(base, 0x08);
        let hccparams1 = r32(base, 0x10);
        let dboff = (r32(base, 0x14) & !0x3) as usize;
        let rtsoff = (r32(base, 0x18) & !0x1f) as usize;
        if !(0x20..=0x80).contains(&cap_len) {
            return Err("caplength-invalide");
        }
        let max_slots = (hcs1 & 0xff) as u8;
        let max_ports = ((hcs1 >> 24) & 0xff) as u8;
        if max_slots == 0 || max_ports == 0 {
            return Err("controller-capabilities-empty");
        }
        let scratchpads = max_scratchpads(hcs2);
        if scratchpads > 1024 {
            return Err("scratchpad-count-invalid");
        }
        let context_size = if hccparams1 & HCC_CSZ != 0 { 64 } else { 32 };
        let ppc = hccparams1 & HCC_PPC != 0;
        let (slot_types, port_major) = protocol_info(base, hccparams1, max_ports);
        let op = base + cap_len;

        let mut connected_before = 0usize;
        for port in 0..max_ports.min(MAX_PORTS_PER_CONTROLLER as u8) as usize {
            let value = r32(op, 0x400 + port * 0x10);
            if value & PORTSC_CCS != 0 {
                connected_before += 1;
            }
        }
        crate::serial_println!(
            "BOUCHAUD_XHCI_CONTROLLER_DISCOVERED bdf={:02x}:{:02x}.{} vid={:04x} did={:04x} bar0={:#x} version={:#06x} slots={} ports={} connected_before={} csz={} ppc={} scratchpads={}",
            dev.bus,
            dev.slot,
            dev.func,
            dev.vendor,
            dev.device,
            bar0,
            hci_version,
            max_slots,
            max_ports,
            connected_before,
            context_size,
            ppc as u8,
            scratchpads
        );

        if !legacy_handoff(base, hccparams1) {
            return Err("bios-ownership-timeout");
        }

        let mut cmd = r32(op, 0x00);
        cmd &= !USBCMD_RUN;
        w32(op, 0x00, cmd);
        if !wait_until(|| r32(op, 0x04) & USBSTS_HCH != 0) {
            return Err("halt-timeout");
        }

        w32(op, 0x00, r32(op, 0x00) | USBCMD_HCRST);
        if !wait_until(|| {
            let c = r32(op, 0x00);
            let s = r32(op, 0x04);
            c & USBCMD_HCRST == 0 && s & USBSTS_CNR == 0
        }) {
            return Err("reset-timeout");
        }
        if r32(op, 0x08) & 1 == 0 {
            return Err("page-4k-non-supportee");
        }

        let Some((dcbaa_phys, dcbaa_virt)) = alloc_zeroed(256 * 8) else {
            return Err("dma-dcbaa");
        };
        if scratchpads != 0 {
            let Some((array_phys, array_virt)) = alloc_zeroed(scratchpads * 8) else {
                return Err("dma-scratch-array");
            };
            for index in 0..scratchpads {
                let Some((page_phys, _)) = alloc_zeroed(4096) else {
                    return Err("dma-scratch-page");
                };
                write_volatile((array_virt + index * 8) as *mut u64, page_phys);
            }
            write_volatile(dcbaa_virt as *mut u64, array_phys);
        }

        let command = alloc_producer_ring().ok_or("dma-command-ring")?;
        let (event_phys, event_virt) = alloc_zeroed(RING_BYTES).ok_or("dma-event-ring")?;
        let events = EventRing {
            phys: event_phys,
            virt: event_virt,
            index: 0,
            cycle: 1,
        };
        let (erst_phys, erst_virt) = alloc_zeroed(64).ok_or("dma-erst")?;
        write_volatile(erst_virt as *mut u64, event_phys);
        write_volatile((erst_virt + 8) as *mut u32, TRBS_PER_RING as u32);
        write_volatile((erst_virt + 12) as *mut u32, 0);

        w64(op, 0x30, dcbaa_phys);
        w64(op, 0x18, command.phys | 1);
        w32(op, 0x38, max_slots as u32);

        let intr0 = base + rtsoff + 0x20;
        w32(intr0, 0x00, 1); // clear pending, keep IE off: V3 polls
        w32(intr0, 0x08, 1);
        w64(intr0, 0x10, erst_phys);
        w64(intr0, 0x18, event_phys | (1 << 3));

        let mut controller = Controller {
            dev,
            base,
            op,
            doorbells: base + dboff,
            intr0,
            hci_version,
            max_slots,
            max_ports,
            context_size,
            ppc,
            slot_types,
            port_major,
            command,
            events,
            dcbaa_phys,
            dcbaa_virt,
            hids: [EMPTY_HID_ENDPOINT; MAX_HID_ENDPOINTS_PER_CONTROLLER],
            hid_count: 0,
            devices: [None; MAX_RUNTIME_DEVICES],
            connected_ports: 0,
            enabled_ports: 0,
            usb_devices: 0,
        };
        let _ = controller.base;
        let _ = controller.dcbaa_phys;

        w32(op, 0x00, r32(op, 0x00) | USBCMD_RUN);
        if !wait_until(|| r32(op, 0x04) & USBSTS_HCH == 0) {
            return Err("run-timeout");
        }
        prepare_root_ports(&mut controller);

        crate::serial_println!(
            "BOUCHAUD_XHCI_CONTROLLER_ACTIVE_OK bdf={:02x}:{:02x}.{} connected={} enabled={}",
            controller.dev.bus,
            controller.dev.slot,
            controller.dev.func,
            controller.connected_ports,
            controller.enabled_ports
        );
        Ok((controller, scratchpads))
    }
}

fn report_payload<'a>(endpoint: &HidEndpoint, data: &'a [u8]) -> Option<&'a [u8]> {
    if endpoint.report_id == 0 {
        return Some(data);
    }
    if data.first().copied()? != endpoint.report_id {
        return None;
    }
    Some(&data[1..])
}

fn process_keyboard_report(endpoint: &mut HidEndpoint, data: &[u8]) -> bool {
    let Some(data) = report_payload(endpoint, data) else { return false; };
    if data.len() < 8 {
        return false;
    }
    let modifiers = data[0];
    let mut keys = [0u8; 6];
    keys.copy_from_slice(&data[2..8]);

    for bit in 0..8u8 {
        let mask = 1u8 << bit;
        let old = endpoint.last_modifiers & mask != 0;
        let new = modifiers & mask != 0;
        if old != new {
            if let Some((code, extended)) = modifier_ps2(bit) {
                push_ps2(code, extended, new);
            }
        }
    }

    for old in endpoint.last_keys.iter().copied() {
        if old >= 4 && !keys.contains(&old) {
            if let Some((code, extended)) = usage_ps2(old) {
                push_ps2(code, extended, false);
            }
        }
    }
    for key in keys.iter().copied() {
        if key >= 4 && !endpoint.last_keys.contains(&key) {
            if let Some((code, extended)) = usage_ps2(key) {
                push_ps2(code, extended, true);
            }
        }
    }

    endpoint.last_modifiers = modifiers;
    endpoint.last_keys = keys;
    true
}

fn process_mouse_report(endpoint: &HidEndpoint, data: &[u8]) -> bool {
    let Some(data) = report_payload(endpoint, data) else { return false; };
    if data.len() < 3 {
        return false;
    }
    // Boot layout and the overwhelming majority of report-only desktop mice:
    // buttons, X, Y, optional wheel. Report-ID wrappers are stripped above.
    let buttons = data[0] & 0x07;
    let dx = data[1] as i8;
    let dy = data[2] as i8;
    let wheel = if data.len() >= 4 { data[3] as i8 } else { 0 };
    crate::drivers::mouse::inject_usb_report(buttons, dx, dy, wheel);
    true
}

fn process_hid_event(controller: &mut Controller, event: Trb) {
    HID_TRANSFER_EVENTS.fetch_add(1, Ordering::Relaxed);
    let slot = event_slot(event);
    let dci = event_dci(event);
    let cc = completion_code(event.status);
    let Some(index) = (0..controller.hid_count).find(|&index| {
        let ep = &controller.hids[index];
        ep.active && ep.slot_id == slot && ep.dci == dci
    }) else {
        crate::serial_println!(
            "BOUCHAUD_HID_EVENT_UNMATCHED slot={} dci={} cc={} status={:#010x}",
            slot, dci, cc, event.status,
        );
        return;
    };

    let buffer_len = controller.hids[index].buffer_len;
    let residual = (event.status & 0x00ff_ffff) as usize;
    let actual = buffer_len.saturating_sub(residual.min(buffer_len));
    if (cc == CC_SUCCESS || cc == CC_SHORT_PACKET) && actual != 0 {
        let data = unsafe {
            core::slice::from_raw_parts(
                controller.hids[index].buffer_virt as *const u8,
                actual,
            )
        };
        HID_REPORTS.fetch_add(1, Ordering::Relaxed);
        let accepted = match controller.hids[index].kind {
            1 => {
                let ok = process_keyboard_report(&mut controller.hids[index], data);
                if ok {
                    let previous = HID_KEYBOARD_REPORTS.fetch_add(1, Ordering::Relaxed);
                    if previous == 0 {
                        crate::serial_println!("BOUCHAUD_HID_KEYBOARD_INPUT_GREEN slot={} dci={}", slot, dci);
                    }
                }
                ok
            }
            2 => {
                let ok = process_mouse_report(&controller.hids[index], data);
                if ok {
                    let previous = HID_MOUSE_REPORTS.fetch_add(1, Ordering::Relaxed);
                    if previous == 0 {
                        crate::serial_println!("BOUCHAUD_HID_MOUSE_INPUT_GREEN slot={} dci={}", slot, dci);
                    }
                }
                ok
            }
            _ => false,
        };
        if HID_REPORTS.load(Ordering::Relaxed) <= 16 {
            crate::serial_println!(
                "BOUCHAUD_HID_REPORT slot={} dci={} kind={} cc={} actual={} accepted={} b0={:#04x} b1={:#04x} b2={:#04x} b3={:#04x}",
                slot, dci, controller.hids[index].kind, cc, actual, accepted as u8,
                data.get(0).copied().unwrap_or(0), data.get(1).copied().unwrap_or(0),
                data.get(2).copied().unwrap_or(0), data.get(3).copied().unwrap_or(0),
            );
        }
    } else if cc != CC_SUCCESS && cc != CC_SHORT_PACKET {
        let n = HID_TRANSFER_ERRORS.fetch_add(1, Ordering::Relaxed) + 1;
        if n <= 16 {
            crate::serial_println!(
                "BOUCHAUD_HID_TRANSFER_FAIL slot={} dci={} cc={} residual={} status={:#010x}",
                slot, dci, cc, residual, event.status,
            );
        }
    }

    // A completed TD is consumed whatever its status. Keep one receive TD
    // outstanding so the input path survives shorts and transient transaction
    // errors without requiring an interrupt-driven producer yet.
    arm_hid_endpoint(controller, index);
}

fn control_get_report(controller: &mut Controller, endpoint_index: usize) -> bool {
    if endpoint_index >= controller.hid_count { return false; }
    let endpoint = controller.hids[endpoint_index];
    if !endpoint.active || endpoint.slot_id as usize >= controller.devices.len() { return false; }
    let Some(mut device) = controller.devices[endpoint.slot_id as usize] else { return false; };

    let report_id = endpoint.report_id;
    let length = endpoint.buffer_len.clamp(3, 64);
    let setup = setup_packet(
        0xa1, // device-to-host, class, interface
        0x01, // GET_REPORT
        ((1u16) << 8) | report_id as u16, // Input report + Report ID
        endpoint.interface as u16,
        length as u16,
    );
    HID_CONTROL_POLLS.fetch_add(1, Ordering::Relaxed);
    let result = control_transfer(controller, &mut device, setup, length, true);
    controller.devices[endpoint.slot_id as usize] = Some(device);
    let received = match result {
        Ok(received) if received != 0 => received.min(length),
        _ => {
            HID_CONTROL_FAILS.fetch_add(1, Ordering::Relaxed);
            return false;
        }
    };

    let data = unsafe {
        core::slice::from_raw_parts(device.control_virt as *const u8, received)
    };
    let accepted = match controller.hids[endpoint_index].kind {
        1 => process_keyboard_report(&mut controller.hids[endpoint_index], data),
        2 => process_mouse_report(&controller.hids[endpoint_index], data),
        _ => false,
    };
    if accepted {
        let previous = HID_CONTROL_REPORTS.fetch_add(1, Ordering::Relaxed);
        match controller.hids[endpoint_index].kind {
            1 => {
                HID_KEYBOARD_REPORTS.fetch_add(1, Ordering::Relaxed);
                if previous == 0 {
                    crate::serial_println!("BOUCHAUD_HID_CONTROL_FALLBACK_GREEN kind=keyboard");
                }
            }
            2 => {
                HID_MOUSE_REPORTS.fetch_add(1, Ordering::Relaxed);
                if previous == 0 {
                    crate::serial_println!("BOUCHAUD_HID_CONTROL_FALLBACK_GREEN kind=mouse");
                }
            }
            _ => {}
        }
    }
    accepted
}

fn poll_control_fallback(controller: &mut Controller, poll_no: usize) {
    // Interrupt-IN remains the preferred path. If the physical controller has
    // produced no endpoint Transfer Event after initial arming, use the HID
    // class GET_REPORT request over EP0 as a compatibility bridge. HID 1.11
    // requires GET_REPORT support on HID devices; this path is deliberately a
    // fallback, not the long-term periodic transport.
    if HID_TRANSFER_EVENTS.load(Ordering::Acquire) != 0 || poll_no < 32 || (poll_no & 1) != 0 {
        return;
    }
    for index in 0..controller.hid_count {
        let _ = control_get_report(controller, index);
    }
}

fn push_ps2(code: u8, extended: bool, pressed: bool) {
    if extended {
        crate::drivers::keyboard::push_scancode(0xe0);
    }
    crate::drivers::keyboard::push_scancode(if pressed { code } else { code | 0x80 });
}

fn modifier_ps2(bit: u8) -> Option<(u8, bool)> {
    Some(match bit {
        0 => (0x1d, false), // Left Ctrl
        1 => (0x2a, false), // Left Shift
        2 => (0x38, false), // Left Alt
        3 => (0x5b, true),  // Left GUI
        4 => (0x1d, true),  // Right Ctrl
        5 => (0x36, false), // Right Shift
        6 => (0x38, true),  // Right Alt / AltGr
        7 => (0x5c, true),  // Right GUI
        _ => return None,
    })
}

fn usage_ps2(usage: u8) -> Option<(u8, bool)> {
    let result = match usage {
        0x04 => (0x1e, false), 0x05 => (0x30, false), 0x06 => (0x2e, false),
        0x07 => (0x20, false), 0x08 => (0x12, false), 0x09 => (0x21, false),
        0x0a => (0x22, false), 0x0b => (0x23, false), 0x0c => (0x17, false),
        0x0d => (0x24, false), 0x0e => (0x25, false), 0x0f => (0x26, false),
        0x10 => (0x32, false), 0x11 => (0x31, false), 0x12 => (0x18, false),
        0x13 => (0x19, false), 0x14 => (0x10, false), 0x15 => (0x13, false),
        0x16 => (0x1f, false), 0x17 => (0x14, false), 0x18 => (0x16, false),
        0x19 => (0x2f, false), 0x1a => (0x11, false), 0x1b => (0x2d, false),
        0x1c => (0x15, false), 0x1d => (0x2c, false),
        0x1e => (0x02, false), 0x1f => (0x03, false), 0x20 => (0x04, false),
        0x21 => (0x05, false), 0x22 => (0x06, false), 0x23 => (0x07, false),
        0x24 => (0x08, false), 0x25 => (0x09, false), 0x26 => (0x0a, false),
        0x27 => (0x0b, false),
        0x28 => (0x1c, false), 0x29 => (0x01, false), 0x2a => (0x0e, false),
        0x2b => (0x0f, false), 0x2c => (0x39, false), 0x2d => (0x0c, false),
        0x2e => (0x0d, false), 0x2f => (0x1a, false), 0x30 => (0x1b, false),
        0x31 | 0x32 => (0x2b, false), 0x33 => (0x27, false), 0x34 => (0x28, false),
        0x35 => (0x29, false), 0x36 => (0x33, false), 0x37 => (0x34, false),
        0x38 => (0x35, false), 0x39 => (0x3a, false),
        0x3a => (0x3b, false), 0x3b => (0x3c, false), 0x3c => (0x3d, false),
        0x3d => (0x3e, false), 0x3e => (0x3f, false), 0x3f => (0x40, false),
        0x40 => (0x41, false), 0x41 => (0x42, false), 0x42 => (0x43, false),
        0x43 => (0x44, false), 0x44 => (0x57, false), 0x45 => (0x58, false),
        0x47 => (0x46, false),
        0x49 => (0x52, true), 0x4a => (0x47, true), 0x4b => (0x49, true),
        0x4c => (0x53, true), 0x4d => (0x4f, true), 0x4e => (0x51, true),
        0x4f => (0x4d, true), 0x50 => (0x4b, true), 0x51 => (0x50, true),
        0x52 => (0x48, true),
        0x53 => (0x45, false), 0x54 => (0x35, true), 0x55 => (0x37, false),
        0x56 => (0x4a, false), 0x57 => (0x4e, false), 0x58 => (0x1c, true),
        0x59 => (0x4f, false), 0x5a => (0x50, false), 0x5b => (0x51, false),
        0x5c => (0x4b, false), 0x5d => (0x4c, false), 0x5e => (0x4d, false),
        0x5f => (0x47, false), 0x60 => (0x48, false), 0x61 => (0x49, false),
        0x62 => (0x52, false), 0x63 => (0x53, false),
        _ => return None,
    };
    Some(result)
}

pub fn is_active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}

pub fn connected_ports() -> usize {
    CONNECTED.load(Ordering::Acquire)
}

pub fn enabled_ports() -> usize {
    ENABLED.load(Ordering::Acquire)
}

pub fn addressed_devices() -> usize {
    ADDRESS_OK.load(Ordering::Acquire)
}

pub fn control_transfers_ok() -> usize {
    CONTROL_OK.load(Ordering::Acquire)
}

pub fn usb_devices() -> usize {
    USB_DEVICES.load(Ordering::Acquire)
}

pub fn hid_keyboards() -> usize {
    HID_KEYBOARDS.load(Ordering::Acquire)
}

pub fn hid_mice() -> usize {
    HID_MICE.load(Ordering::Acquire)
}

pub fn hid_polling() -> bool {
    is_active() && hid_keyboards().saturating_add(hid_mice()) != 0
}

pub fn hid_ready() -> bool {
    hid_keyboards() != 0 && hid_mice() != 0
}

pub fn hid_control_fallback_stats() -> (usize, usize, usize) {
    (
        HID_CONTROL_POLLS.load(Ordering::Acquire),
        HID_CONTROL_REPORTS.load(Ordering::Acquire),
        HID_CONTROL_FAILS.load(Ordering::Acquire),
    )
}

pub fn hid_transport_stats() -> (usize, usize, usize, usize, usize, usize, usize, usize) {
    (
        HID_POLLS.load(Ordering::Acquire),
        HID_TRANSFER_EVENTS.load(Ordering::Acquire),
        HID_REPORTS.load(Ordering::Acquire),
        HID_KEYBOARD_REPORTS.load(Ordering::Acquire),
        HID_MOUSE_REPORTS.load(Ordering::Acquire),
        HID_TRANSFER_ERRORS.load(Ordering::Acquire),
        HID_REARMS.load(Ordering::Acquire),
        HID_KICKS.load(Ordering::Acquire),
    )
}

pub fn poll() {
    if !hid_polling() {
        return;
    }
    HID_POLLS.fetch_add(1, Ordering::Relaxed);
    if RUNTIME_BUSY
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return;
    }

    unsafe {
        if let Some(runtime) = RUNTIME.as_mut() {
            let poll_no = HID_POLLS.load(Ordering::Relaxed);
            for controller in runtime.controllers.iter_mut() {
                for _ in 0..MAX_EVENTS_PER_POLL {
                    let Some(event) = next_event(controller) else { break };
                    if trb_type(event.control) == EVT_TRANSFER {
                        process_hid_event(controller, event);
                    }
                }
                // Some physical controllers are conservative about periodic
                // scheduling after Configure Endpoint. Re-kicking an endpoint
                // with an already pending TD is harmless and gives us a robust
                // polling-only path without MSI-X. Roughly every 128 polls.
                if poll_no & 0x7f == 0 {
                    for index in 0..controller.hid_count {
                        let ep = controller.hids[index];
                        if ep.active {
                            fence(Ordering::SeqCst);
                            ring_doorbell(controller, ep.slot_id, ep.dci);
                            HID_KICKS.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
                poll_control_fallback(controller, poll_no);
            }
        }
    }
    RUNTIME_BUSY.store(false, Ordering::Release);
}

pub fn bring_up() -> ActiveSummary {
    ACTIVE.store(false, Ordering::Release);
    CONNECTED.store(0, Ordering::Release);
    ENABLED.store(0, Ordering::Release);
    ADDRESS_OK.store(0, Ordering::Release);
    CONTROL_OK.store(0, Ordering::Release);
    USB_DEVICES.store(0, Ordering::Release);
    HID_KEYBOARDS.store(0, Ordering::Release);
    HID_MICE.store(0, Ordering::Release);
    HID_POLLS.store(0, Ordering::Release);
    HID_TRANSFER_EVENTS.store(0, Ordering::Release);
    HID_REPORTS.store(0, Ordering::Release);
    HID_KEYBOARD_REPORTS.store(0, Ordering::Release);
    HID_MOUSE_REPORTS.store(0, Ordering::Release);
    HID_TRANSFER_ERRORS.store(0, Ordering::Release);
    HID_REARMS.store(0, Ordering::Release);
    HID_KICKS.store(0, Ordering::Release);
    HID_CONTROL_POLLS.store(0, Ordering::Release);
    HID_CONTROL_REPORTS.store(0, Ordering::Release);
    HID_CONTROL_FAILS.store(0, Ordering::Release);
    CONTROLLERS.store(0, Ordering::Release);
    unsafe { RUNTIME = None };

    let mut devices = Vec::<PciDevice>::new();
    pci::parcours(&mut |dev| {
        if dev.class == 0x0c && dev.subclass == 0x03 && dev.prog_if == 0x30 {
            devices.push(*dev);
        }
        true
    });
    let seen = devices.len();
    if seen == 0 {
        return ActiveSummary {
            present: false,
            active: false,
            controllers_seen: 0,
            controllers_active: 0,
            bar0: 0,
            hci_version: 0,
            max_slots: 0,
            max_ports: 0,
            scratchpads: 0,
            connected_ports: 0,
            enabled_ports: 0,
            usb_devices: 0,
            hid_keyboards: 0,
            hid_mice: 0,
            error: Some("xhci-absent"),
        };
    }

    let mut runtime = Runtime { controllers: Vec::new() };
    let mut first_bar = 0u64;
    let mut first_version = 0u16;
    let mut first_slots = 0u8;
    let mut first_ports = 0u8;
    let mut scratchpads_total = 0usize;
    let mut connected_total = 0usize;
    let mut enabled_total = 0usize;
    let mut usb_total = 0usize;
    let mut keyboard_total = 0usize;
    let mut mouse_total = 0usize;

    for dev in devices {
        match init_controller(dev) {
            Ok((mut controller, scratchpads)) => {
                if first_bar == 0 {
                    first_bar = pci::bar_decode(&controller.dev, 0).adresse();
                    first_version = controller.hci_version;
                    first_slots = controller.max_slots;
                    first_ports = controller.max_ports;
                }
                scratchpads_total = scratchpads_total.saturating_add(scratchpads);
                let ports = controller.max_ports.min(MAX_PORTS_PER_CONTROLLER as u8) as usize;
                for port in 0..ports {
                    enumerate_port(&mut controller, port);
                }
                arm_all_hids(&mut controller);
                connected_total = connected_total.saturating_add(controller.connected_ports);
                enabled_total = enabled_total.saturating_add(controller.enabled_ports);
                usb_total = usb_total.saturating_add(controller.usb_devices);
                for endpoint in controller.hids[..controller.hid_count].iter() {
                    match endpoint.kind {
                        1 => keyboard_total += 1,
                        2 => mouse_total += 1,
                        _ => {}
                    }
                }
                runtime.controllers.push(controller);
            }
            Err(error) => {
                crate::serial_println!(
                    "BOUCHAUD_XHCI_CONTROLLER_FAIL bdf={:02x}:{:02x}.{} error={}",
                    dev.bus,
                    dev.slot,
                    dev.func,
                    error
                );
            }
        }
    }

    let active_controllers = runtime.controllers.len();
    let active = active_controllers != 0;
    if active {
        unsafe { RUNTIME = Some(runtime) };
    }
    ACTIVE.store(active, Ordering::Release);
    CONNECTED.store(connected_total, Ordering::Release);
    ENABLED.store(enabled_total, Ordering::Release);
    USB_DEVICES.store(usb_total, Ordering::Release);
    HID_KEYBOARDS.store(keyboard_total, Ordering::Release);
    HID_MICE.store(mouse_total, Ordering::Release);
    CONTROLLERS.store(active_controllers, Ordering::Release);

    if connected_total != 0 {
        crate::serial_println!("BOUCHAUD_XHCI_ROOT_PORT_GREEN count={}", connected_total);
    }
    if usb_total != 0 {
        crate::serial_println!("BOUCHAUD_USB_ENUM_GREEN devices={}", usb_total);
    }
    if keyboard_total != 0 {
        crate::serial_println!("BOUCHAUD_HID_KEYBOARD_GREEN count={}", keyboard_total);
    }
    if mouse_total != 0 {
        crate::serial_println!("BOUCHAUD_HID_MOUSE_GREEN count={}", mouse_total);
    }
    crate::serial_println!(
        "BOUCHAUD_XHCI_V34_SUMMARY controllers={}/{} ports={} enabled={} addressed={} control_ok={} usb={} keyboards={} mice={} polling={}",
        active_controllers,
        seen,
        connected_total,
        enabled_total,
        ADDRESS_OK.load(Ordering::Acquire),
        CONTROL_OK.load(Ordering::Acquire),
        usb_total,
        keyboard_total,
        mouse_total,
        (keyboard_total + mouse_total != 0) as u8
    );

    ActiveSummary {
        present: true,
        active,
        controllers_seen: seen,
        controllers_active: active_controllers,
        bar0: first_bar,
        hci_version: first_version,
        max_slots: first_slots,
        max_ports: first_ports,
        scratchpads: scratchpads_total,
        connected_ports: connected_total,
        enabled_ports: enabled_total,
        usb_devices: usb_total,
        hid_keyboards: keyboard_total,
        hid_mice: mouse_total,
        error: if active { None } else { Some("all-xhci-init-failed") },
    }
}
