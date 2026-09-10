// BOUCHAUD_TRIGKEY_BLACKBOX_V2
//
// Extension interne de `xhci_active.rs`. Ce fichier est `include!` dans le
// meme module afin de reutiliser rings, contextes et wait_event xHCI prives.

// Le decodage du protocole est DECLARE PAR `xhci_active.rs`, qui inclut ce
// fichier : le declarer une seconde fois ici compilerait `stockage/decodage.rs`
// deux fois dans le meme module, avec deux jeux de constantes qui pourraient
// diverger sans que rien ne le dise. L'alias garde les appels inchanges.
use stockage as stockage_blackbox;

const BLACKBOX_PARTITION_NAME: &[u8] = b"BOUCHAUD-BLACKBOX";
const BLACKBOX_RECORD_BYTES: usize = 4096;
const BLACKBOX_HEADER_BYTES: usize = 64;
const BLACKBOX_PAYLOAD_MAX: usize = BLACKBOX_RECORD_BYTES - BLACKBOX_HEADER_BYTES;
const BLACKBOX_RECORD_MAGIC: &[u8; 8] = b"BOUBBX01";
const BLACKBOX_RECORD_VERSION: u16 = 1;
const BLACKBOX_DMA_BYTES: usize = 64 * 1024;
const BLACKBOX_SCAN_GPT_ENTRIES: usize = 16;
const BLACKBOX_SYNC_EVERY: u32 = 64;
const BLACKBOX_KIND_FATAL: u16 = 9;

static BLACKBOX_STORAGE_READY: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);
static BLACKBOX_WRITES: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
static BLACKBOX_FAILURES: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
static BLACKBOX_CONSECUTIVE_FAILURES: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
static BLACKBOX_LAST_ERROR: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
static BLACKBOX_BUSY_SKIPS: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
static BLACKBOX_LAST_OK_NS: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

#[derive(Clone, Copy, Default)]
struct BlackboxStorageDescriptor {
    interface: u8,
    bulk_in: u8,
    bulk_out: u8,
    mps_in: u16,
    mps_out: u16,
    burst_in: u8,
    burst_out: u8,
}

struct BlackboxStorage {
    slot_id: u8,
    interface: u8,
    dci_in: u8,
    dci_out: u8,
    ring_in: ProducerRing,
    ring_out: ProducerRing,
    buffer_phys: u64,
    buffer_virt: usize,
    buffer_len: usize,
    tag: u32,
    block_size: u32,
    blocks: u64,
    partition_first: u64,
    partition_blocks: u64,
    since_sync: u32,
    disabled: bool,
}

fn parse_blackbox_storage_descriptor(bytes: &[u8]) -> Option<BlackboxStorageDescriptor> {
    if bytes.len() < 9 || bytes[1] != 2 {
        return None;
    }
    let mut out = BlackboxStorageDescriptor::default();
    let mut storage_interface = false;
    let mut last_endpoint = 0u8;
    let mut offset = 0usize;

    while offset + 2 <= bytes.len() {
        let len = bytes[offset] as usize;
        let ty = bytes[offset + 1];
        if len < 2 || offset + len > bytes.len() {
            break;
        }
        match ty {
            4 if len >= 9 => {
                let alternate = bytes[offset + 3];
                let class = bytes[offset + 5];
                let subclass = bytes[offset + 6];
                let protocol = bytes[offset + 7];
                storage_interface =
                    alternate == 0 && class == 0x08 && subclass == 0x06 && protocol == 0x50;
                last_endpoint = 0;
                if storage_interface {
                    out.interface = bytes[offset + 2];
                }
            }
            5 if len >= 7 && storage_interface => {
                let address = bytes[offset + 2];
                let attributes = bytes[offset + 3] & 0x03;
                if attributes == 2 {
                    let mps = u16::from_le_bytes([bytes[offset + 4], bytes[offset + 5]]) & 0x07ff;
                    if address & 0x80 != 0 {
                        out.bulk_in = address;
                        out.mps_in = mps;
                        last_endpoint = 2;
                    } else {
                        out.bulk_out = address;
                        out.mps_out = mps;
                        last_endpoint = 1;
                    }
                }
            }
            0x30 if len >= 6 && storage_interface => {
                let burst = bytes[offset + 2];
                if last_endpoint == 1 {
                    out.burst_out = burst;
                } else if last_endpoint == 2 {
                    out.burst_in = burst;
                }
            }
            _ => {}
        }
        offset += len;
    }

    if out.bulk_in != 0 && out.bulk_out != 0 && out.mps_in != 0 && out.mps_out != 0 {
        Some(out)
    } else {
        None
    }
}

fn fill_blackbox_bulk_context(
    ctx: usize,
    ep_type: u8,
    max_packet: u16,
    max_burst: u8,
    ring: &ProducerRing,
) {
    fill_endpoint_context(ctx, ep_type, max_packet, 0, ring);
    unsafe {
        let dw1 = ctx_r32(ctx, 1);
        ctx_w32(ctx, 1, (dw1 & !(0xff << 8)) | ((max_burst as u32) << 8));
    }
}

fn free_blackbox_storage(storage: &BlackboxStorage) {
    memory::free_dma(storage.ring_in.phys, RING_BYTES);
    memory::free_dma(storage.ring_out.phys, RING_BYTES);
    memory::free_dma(storage.buffer_phys, storage.buffer_len);
}

fn blackbox_bulk_transfer(
    controller: &mut Controller,
    storage: &mut BlackboxStorage,
    input: bool,
    len: usize,
) -> Result<usize, &'static str> {
    if len == 0 {
        return Ok(0);
    }
    if len > storage.buffer_len {
        return Err("blackbox-dma-buffer-too-small");
    }
    let (dci, ring) = if input {
        (storage.dci_in, &mut storage.ring_in)
    } else {
        (storage.dci_out, &mut storage.ring_out)
    };

    ring_push(
        ring,
        Trb {
            parameter: storage.buffer_phys,
            status: len as u32,
            control: (TRB_NORMAL << 10) | TRB_IOC | if input { TRB_ISP } else { 0 },
        },
    );
    ring_doorbell(controller, storage.slot_id, dci);

    let event = wait_event(controller, EVT_TRANSFER, Some(storage.slot_id), Some(dci))
        .ok_or("blackbox-bulk-timeout")?;
    let cc = completion_code(event.status);
    if cc != CC_SUCCESS && cc != CC_SHORT_PACKET {
        return Err("blackbox-bulk-status");
    }
    let residual = (event.status & 0x00ff_ffff) as usize;
    Ok(len.saturating_sub(residual.min(len)))
}

fn blackbox_bot(
    controller: &mut Controller,
    storage: &mut BlackboxStorage,
    cdb: &[u8],
    data: &mut [u8],
    data_in: bool,
) -> Result<usize, &'static str> {
    if data.len() > storage.buffer_len {
        return Err("blackbox-bot-data-too-large");
    }

    storage.tag = storage.tag.wrapping_add(1).max(1);
    let tag = storage.tag;
    let mut cbw = [0u8; stockage_blackbox::CBW_OCTETS];
    if !stockage_blackbox::encode_cbw(&mut cbw, tag, data.len() as u32, data_in, 0, cdb) {
        return Err("blackbox-cbw");
    }

    unsafe {
        copy_nonoverlapping(cbw.as_ptr(), storage.buffer_virt as *mut u8, cbw.len());
    }
    if blackbox_bulk_transfer(controller, storage, false, cbw.len())? != cbw.len() {
        return Err("blackbox-cbw-short");
    }

    let mut actual = 0usize;
    if !data.is_empty() {
        if !data_in {
            unsafe {
                copy_nonoverlapping(data.as_ptr(), storage.buffer_virt as *mut u8, data.len());
            }
        }
        actual = blackbox_bulk_transfer(controller, storage, data_in, data.len())?;
        if data_in && actual != 0 {
            unsafe {
                copy_nonoverlapping(
                    storage.buffer_virt as *const u8,
                    data.as_mut_ptr(),
                    actual.min(data.len()),
                );
            }
        }
    }

    unsafe {
        write_bytes(storage.buffer_virt as *mut u8, 0, stockage_blackbox::CSW_OCTETS);
    }
    let got = blackbox_bulk_transfer(controller, storage, true, stockage_blackbox::CSW_OCTETS)?;
    if got != stockage_blackbox::CSW_OCTETS {
        return Err("blackbox-csw-short");
    }
    let csw_bytes = unsafe {
        core::slice::from_raw_parts(storage.buffer_virt as *const u8, stockage_blackbox::CSW_OCTETS)
    };
    let csw = stockage_blackbox::decode_csw(csw_bytes).ok_or("blackbox-csw-invalid")?;
    if !stockage_blackbox::transfert_complet(&csw, tag) {
        return Err("blackbox-csw-failure");
    }
    Ok(if data.is_empty() { 0 } else { actual })
}

fn blackbox_read_blocks(
    controller: &mut Controller,
    storage: &mut BlackboxStorage,
    lba: u64,
    blocks: u16,
    out: &mut [u8],
) -> Result<(), &'static str> {
    if storage.block_size != 512 || lba > u32::MAX as u64 {
        return Err("blackbox-read-address");
    }
    let bytes = blocks as usize * storage.block_size as usize;
    if out.len() != bytes || bytes > storage.buffer_len {
        return Err("blackbox-read-size");
    }
    let cdb = stockage_blackbox::cdb_transfert_10(false, lba as u32, blocks);
    let got = blackbox_bot(controller, storage, &cdb, out, true)?;
    if got != bytes {
        return Err("blackbox-read-short");
    }
    Ok(())
}

fn blackbox_write_blocks(
    controller: &mut Controller,
    storage: &mut BlackboxStorage,
    lba: u64,
    blocks: u16,
    data: &mut [u8],
) -> Result<(), &'static str> {
    if storage.block_size != 512 || lba > u32::MAX as u64 {
        return Err("blackbox-write-address");
    }
    let bytes = blocks as usize * storage.block_size as usize;
    if data.len() != bytes || bytes > storage.buffer_len {
        return Err("blackbox-write-size");
    }
    let cdb = stockage_blackbox::cdb_transfert_10(true, lba as u32, blocks);
    let got = blackbox_bot(controller, storage, &cdb, data, false)?;
    if got != bytes {
        return Err("blackbox-write-short");
    }
    Ok(())
}

fn blackbox_sync_cache(controller: &mut Controller, storage: &mut BlackboxStorage) {
    let cdb = [0x35u8, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let mut empty = [0u8; 0];
    let _ = blackbox_bot(controller, storage, &cdb, &mut empty, false);
    storage.since_sync = 0;
}

fn le_u32(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
}

fn le_u64(data: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        data[off], data[off + 1], data[off + 2], data[off + 3],
        data[off + 4], data[off + 5], data[off + 6], data[off + 7],
    ])
}

fn blackbox_partition_name_matches(entry: &[u8]) -> bool {
    if entry.len() < 128 {
        return false;
    }
    for (i, expected) in BLACKBOX_PARTITION_NAME.iter().copied().enumerate() {
        let off = 56 + i * 2;
        if off + 1 >= entry.len() || entry[off] != expected || entry[off + 1] != 0 {
            return false;
        }
    }
    let end = 56 + BLACKBOX_PARTITION_NAME.len() * 2;
    end + 1 < entry.len() && entry[end] == 0 && entry[end + 1] == 0
}

fn blackbox_find_partition(
    controller: &mut Controller,
    storage: &mut BlackboxStorage,
) -> Result<bool, &'static str> {
    let mut sector = [0u8; 512];
    blackbox_read_blocks(controller, storage, 1, 1, &mut sector)?;
    if &sector[0..8] != b"EFI PART" {
        return Ok(false);
    }
    let entries_lba = le_u64(&sector, 72);
    let entries = le_u32(&sector, 80) as usize;
    let entry_size = le_u32(&sector, 84) as usize;
    if entry_size < 128 || entry_size > 512 || 512 % entry_size != 0 {
        return Ok(false);
    }
    let per_sector = 512 / entry_size;
    let to_scan = entries.min(BLACKBOX_SCAN_GPT_ENTRIES);
    let mut cached_sector = usize::MAX;

    for index in 0..to_scan {
        let sector_index = index / per_sector;
        if cached_sector != sector_index {
            blackbox_read_blocks(
                controller,
                storage,
                entries_lba.saturating_add(sector_index as u64),
                1,
                &mut sector,
            )?;
            cached_sector = sector_index;
        }
        let off = (index % per_sector) * entry_size;
        let entry = &sector[off..off + entry_size];
        if entry[..16].iter().all(|b| *b == 0) {
            continue;
        }
        if !blackbox_partition_name_matches(entry) {
            continue;
        }
        let first = le_u64(entry, 32);
        let last = le_u64(entry, 40);
        if first == 0 || last < first {
            return Ok(false);
        }
        let blocks = last - first + 1;
        let record_blocks = (BLACKBOX_RECORD_BYTES / 512) as u64;
        if blocks < record_blocks.saturating_mul(64) {
            return Ok(false);
        }
        storage.partition_first = first;
        storage.partition_blocks = blocks;
        return Ok(true);
    }
    Ok(false)
}

fn configure_blackbox_storage(
    controller: &mut Controller,
    device: &mut Device,
    descriptor: BlackboxStorageDescriptor,
) -> Result<bool, &'static str> {
    if controller.blackbox_storage.is_some() {
        return Ok(false);
    }

    let ep_out = descriptor.bulk_out & 0x0f;
    let ep_in = descriptor.bulk_in & 0x0f;
    if ep_out == 0 || ep_in == 0 {
        return Ok(false);
    }
    let dci_out = ep_out.saturating_mul(2);
    let dci_in = ep_in.saturating_mul(2).saturating_add(1);
    if dci_out >= 32 || dci_in >= 32 {
        return Ok(false);
    }

    let ring_out = alloc_producer_ring().ok_or("blackbox-ring-out")?;
    let ring_in = match alloc_producer_ring() {
        Some(r) => r,
        None => {
            memory::free_dma(ring_out.phys, RING_BYTES);
            return Err("blackbox-ring-in");
        }
    };
    let (buffer_phys, buffer_virt) = match alloc_zeroed(BLACKBOX_DMA_BYTES) {
        Some(v) => v,
        None => {
            memory::free_dma(ring_out.phys, RING_BYTES);
            memory::free_dma(ring_in.phys, RING_BYTES);
            return Err("blackbox-buffer");
        }
    };

    clear_input(device);
    let slot_out = context_ptr(device.out_ctx_virt, controller.context_size, 0);
    let slot_in = context_ptr(device.in_ctx_virt, controller.context_size, 1);
    unsafe {
        copy_nonoverlapping(slot_out as *const u8, slot_in as *mut u8, controller.context_size);
    }

    let out_ctx = context_ptr(device.in_ctx_virt, controller.context_size, dci_out as usize + 1);
    let in_ctx = context_ptr(device.in_ctx_virt, controller.context_size, dci_in as usize + 1);
    fill_blackbox_bulk_context(out_ctx, 2, descriptor.mps_out, descriptor.burst_out, &ring_out);
    fill_blackbox_bulk_context(in_ctx, 6, descriptor.mps_in, descriptor.burst_in, &ring_in);

    let add_flags = 1u32 | (1u32 << dci_out) | (1u32 << dci_in);
    let highest_dci = dci_out.max(dci_in);
    unsafe {
        write_volatile((device.in_ctx_virt + 4) as *mut u32, add_flags);
        let dw0 = ctx_r32(slot_in, 0);
        ctx_w32(slot_in, 0, (dw0 & !(0x1f << 27)) | ((highest_dci as u32) << 27));
    }

    if let Err(error) = command(controller, device.in_ctx_phys, CMD_CONFIGURE_ENDPOINT, device.slot_id) {
        memory::free_dma(ring_out.phys, RING_BYTES);
        memory::free_dma(ring_in.phys, RING_BYTES);
        memory::free_dma(buffer_phys, BLACKBOX_DMA_BYTES);
        return Err(error);
    }

    let mut storage = BlackboxStorage {
        slot_id: device.slot_id,
        interface: descriptor.interface,
        dci_in,
        dci_out,
        ring_in,
        ring_out,
        buffer_phys,
        buffer_virt,
        buffer_len: BLACKBOX_DMA_BYTES,
        tag: 0,
        block_size: 0,
        blocks: 0,
        partition_first: 0,
        partition_blocks: 0,
        since_sync: 0,
        disabled: false,
    };

    let mut empty = [0u8; 0];
    let tur = stockage_blackbox::cdb_test_unite_prete();
    let _ = blackbox_bot(controller, &mut storage, &tur, &mut empty, true);

    let mut inquiry = [0u8; 36];
    let inquiry_cdb = stockage_blackbox::cdb_interroge(inquiry.len() as u8);
    if blackbox_bot(controller, &mut storage, &inquiry_cdb, &mut inquiry, true).is_err() {
        free_blackbox_storage(&storage);
        return Ok(false);
    }
    let Some(info) = stockage_blackbox::decode_interrogation(&inquiry) else {
        free_blackbox_storage(&storage);
        return Ok(false);
    };
    if !stockage_blackbox::support_utilisable(&info) {
        free_blackbox_storage(&storage);
        return Ok(false);
    }

    let mut cap = [0u8; 8];
    let cap_cdb = stockage_blackbox::cdb_lit_capacite();
    if blackbox_bot(controller, &mut storage, &cap_cdb, &mut cap, true).is_err() {
        free_blackbox_storage(&storage);
        return Ok(false);
    }
    let Some(capacity) = stockage_blackbox::decode_capacite(&cap) else {
        free_blackbox_storage(&storage);
        return Ok(false);
    };
    storage.block_size = capacity.taille_bloc;
    storage.blocks = capacity.blocs();
    if storage.block_size != 512 {
        free_blackbox_storage(&storage);
        return Ok(false);
    }

    match blackbox_find_partition(controller, &mut storage) {
        Ok(true) => {
            crate::serial_println!(
                "BOUCHAUD_BLACKBOX_USB_READY slot={} if={} blocks={} partition_first={} partition_blocks={}",
                storage.slot_id,
                storage.interface,
                storage.blocks,
                storage.partition_first,
                storage.partition_blocks,
            );
            controller.blackbox_storage = Some(storage);
            BLACKBOX_STORAGE_READY.store(true, Ordering::Release);
            Ok(true)
        }
        _ => {
            free_blackbox_storage(&storage);
            Ok(false)
        }
    }
}

fn retire_blackbox_storage(controller: &mut Controller, slot: u8) {
    let matches = controller.blackbox_storage.as_ref().map(|s| s.slot_id == slot).unwrap_or(false);
    if matches {
        if let Some(storage) = controller.blackbox_storage.take() {
            free_blackbox_storage(&storage);
        }
        BLACKBOX_STORAGE_READY.store(false, Ordering::Release);
    }
}

fn blackbox_error_code(error: &'static str) -> u64 {
    match error {
        "blackbox-bulk-timeout" => 1,
        "blackbox-bulk-status" => 2,
        "blackbox-cbw-short" => 3,
        "blackbox-csw-short" => 4,
        "blackbox-csw-invalid" => 5,
        "blackbox-csw-failure" => 6,
        "blackbox-write-short" => 7,
        "blackbox-write-address" => 8,
        "blackbox-write-size" => 9,
        "blackbox-bot-data-too-large" => 10,
        "blackbox-cbw" => 11,
        "blackbox-disabled-or-payload" => 12,
        "blackbox-no-slots" => 13,
        _ => 255,
    }
}

fn blackbox_crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn put_u16(out: &mut [u8], off: usize, value: u16) {
    out[off..off + 2].copy_from_slice(&value.to_le_bytes());
}
fn put_u32(out: &mut [u8], off: usize, value: u32) {
    out[off..off + 4].copy_from_slice(&value.to_le_bytes());
}
fn put_u64(out: &mut [u8], off: usize, value: u64) {
    out[off..off + 8].copy_from_slice(&value.to_le_bytes());
}

fn blackbox_write_record(
    controller: &mut Controller,
    storage: &mut BlackboxStorage,
    kind: u16,
    boot_id: u64,
    sequence: u64,
    ts_ns: u64,
    trace_end: u64,
    payload: &[u8],
) -> Result<(), &'static str> {
    if storage.disabled || payload.len() > BLACKBOX_PAYLOAD_MAX {
        return Err("blackbox-disabled-or-payload");
    }
    let record_blocks = (BLACKBOX_RECORD_BYTES / 512) as u64;
    let slots = storage.partition_blocks / record_blocks;
    if slots == 0 {
        return Err("blackbox-no-slots");
    }

    let mut record = [0u8; BLACKBOX_RECORD_BYTES];
    record[0..8].copy_from_slice(BLACKBOX_RECORD_MAGIC);
    put_u16(&mut record, 8, BLACKBOX_RECORD_VERSION);
    put_u16(&mut record, 10, kind);
    put_u32(&mut record, 12, BLACKBOX_HEADER_BYTES as u32);
    put_u64(&mut record, 16, boot_id);
    put_u64(&mut record, 24, sequence);
    put_u64(&mut record, 32, ts_ns);
    put_u32(&mut record, 40, payload.len() as u32);
    put_u32(&mut record, 44, blackbox_crc32(payload));
    put_u64(&mut record, 48, trace_end);
    put_u32(&mut record, 56, 0);
    put_u32(&mut record, 60, 0);
    record[BLACKBOX_HEADER_BYTES..BLACKBOX_HEADER_BYTES + payload.len()].copy_from_slice(payload);

    let slot = sequence % slots;
    let lba = storage.partition_first.saturating_add(slot.saturating_mul(record_blocks));
    blackbox_write_blocks(controller, storage, lba, record_blocks as u16, &mut record)?;

    BLACKBOX_WRITES.fetch_add(1, Ordering::Relaxed);
    storage.since_sync = storage.since_sync.saturating_add(1);
    if kind == BLACKBOX_KIND_FATAL || storage.since_sync >= BLACKBOX_SYNC_EVERY {
        blackbox_sync_cache(controller, storage);
    }
    Ok(())
}

pub fn blackbox_storage_ready() -> bool {
    BLACKBOX_STORAGE_READY.load(Ordering::Acquire)
}

pub fn blackbox_storage_counters() -> (u64, u64) {
    (
        BLACKBOX_WRITES.load(Ordering::Relaxed),
        BLACKBOX_FAILURES.load(Ordering::Relaxed),
    )
}

pub fn blackbox_storage_extended_counters() -> (u64, u64, u64, u64, u64, u64) {
    (
        BLACKBOX_WRITES.load(Ordering::Relaxed),
        BLACKBOX_FAILURES.load(Ordering::Relaxed),
        BLACKBOX_CONSECUTIVE_FAILURES.load(Ordering::Relaxed),
        BLACKBOX_LAST_ERROR.load(Ordering::Relaxed),
        BLACKBOX_BUSY_SKIPS.load(Ordering::Relaxed),
        BLACKBOX_LAST_OK_NS.load(Ordering::Relaxed),
    )
}

pub fn blackbox_append_record(
    kind: u16,
    boot_id: u64,
    sequence: u64,
    ts_ns: u64,
    trace_end: u64,
    payload: &[u8],
) -> bool {
    if payload.len() > BLACKBOX_PAYLOAD_MAX || !BLACKBOX_STORAGE_READY.load(Ordering::Acquire) {
        return false;
    }
    if RUNTIME_BUSY.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        BLACKBOX_BUSY_SKIPS.fetch_add(1, Ordering::Relaxed);
        return false;
    }

    let mut ok = false;
    unsafe {
        if let Some(runtime) = RUNTIME.as_mut() {
            for controller in runtime.controllers.iter_mut() {
                let Some(mut storage) = controller.blackbox_storage.take() else { continue; };
                // `disabled` est reserve a un futur retrait materiel explicite.
                // Une erreur de transport transitoire ne desactive plus le recorder.
                let result = blackbox_write_record(
                    controller, &mut storage, kind, boot_id, sequence, ts_ns, trace_end, payload,
                );
                match result {
                    Ok(()) => {
                        ok = true;
                        BLACKBOX_CONSECUTIVE_FAILURES.store(0, Ordering::Relaxed);
                        BLACKBOX_LAST_ERROR.store(0, Ordering::Relaxed);
                        BLACKBOX_LAST_OK_NS.store(ts_ns, Ordering::Relaxed);
                    }
                    Err(error) => {
                        let total = BLACKBOX_FAILURES.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
                        let consecutive = BLACKBOX_CONSECUTIVE_FAILURES
                            .fetch_add(1, Ordering::Relaxed)
                            .wrapping_add(1);
                        let code = blackbox_error_code(error);
                        BLACKBOX_LAST_ERROR.store(code, Ordering::Relaxed);

                        // V1 coupait definitivement le recorder apres trois erreurs,
                        // ce qui rendait precisement la panne du recorder invisible.
                        // V2 reste fail-open et retente au prochain poll. Les logs
                        // persistants pourront reprendre des que le transport revient.
                        if total <= 4 || total.is_power_of_two() {
                            crate::serial_println!(
                                "BOUCHAUD_BLACKBOX_WRITE_FAIL total={} consecutive={} code={} error={}",
                                total,
                                consecutive,
                                code,
                                error,
                            );
                        }
                    }
                }
                controller.blackbox_storage = Some(storage);
                if ok { break; }
            }
        }
    }
    RUNTIME_BUSY.store(false, Ordering::Release);
    ok
}

pub fn blackbox_force_sync() {
    if !BLACKBOX_STORAGE_READY.load(Ordering::Acquire) {
        return;
    }
    if RUNTIME_BUSY.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        return;
    }
    unsafe {
        if let Some(runtime) = RUNTIME.as_mut() {
            for controller in runtime.controllers.iter_mut() {
                let Some(mut storage) = controller.blackbox_storage.take() else { continue; };
                blackbox_sync_cache(controller, &mut storage);
                controller.blackbox_storage = Some(storage);
            }
        }
    }
    RUNTIME_BUSY.store(false, Ordering::Release);
}
