#![no_main]
#![no_std]
extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;

use uefi::boot::LoadImageSource;
use uefi::fs::FileSystem;
use uefi::proto::console::gop::{BltOp, BltPixel, BltRegion, GraphicsOutput};
use uefi::proto::console::pointer::Pointer;
use uefi::proto::console::text::Input;
use uefi::proto::device_path::LoadedImageDevicePath;
use uefi::proto::unsafe_protocol;
use uefi::{boot, cstr16, entry, Handle, Status};

#[repr(C)]
#[unsafe_protocol("bd8c1056-9f36-44ec-92a8-a6337f817986")]
struct EdidActive {
    size_of_edid: u32,
    edid: *const u8,
}

fn read_edid(handle: Handle) -> Vec<u8> {
    let Ok(proto) = boot::open_protocol_exclusive::<EdidActive>(handle) else { return Vec::new() };
    if proto.size_of_edid == 0 || proto.size_of_edid > 4096 || proto.edid.is_null() { return Vec::new(); }
    unsafe { core::slice::from_raw_parts(proto.edid, proto.size_of_edid as usize).to_vec() }
}

fn edid_native(edid: &[u8]) -> Option<(usize, usize)> {
    const MAGIC: [u8; 8] = [0x00,0xff,0xff,0xff,0xff,0xff,0xff,0x00];
    if edid.len() < 72 || edid[0..8] != MAGIC { return None; }
    let d = &edid[54..72];
    if d[0] == 0 && d[1] == 0 { return None; }
    let h = d[2] as usize | (((d[4] >> 4) as usize) << 8);
    let v = d[5] as usize | (((d[7] >> 4) as usize) << 8);
    (h != 0 && v != 0).then_some((h, v))
}

#[inline]
fn dans_ellipse(x: i32, y: i32, cx: i32, cy: i32, rx: i32, ry: i32) -> bool {
    let dx = (x - cx) as i64;
    let dy = (y - cy) as i64;
    dx * dx * ry as i64 * ry as i64 + dy * dy * rx as i64 * rx as i64
        <= rx as i64 * rx as i64 * ry as i64 * ry as i64
}

#[inline]
fn dans_arrondi(x: i32, y: i32, x0: i32, y0: i32, x1: i32, y1: i32, rayon: i32) -> bool {
    if x < x0 || y < y0 || x >= x1 || y >= y1 { return false; }
    let cx = x.clamp(x0 + rayon, x1 - rayon - 1);
    let cy = y.clamp(y0 + rayon, y1 - rayon - 1);
    let dx = x - cx;
    let dy = y - cy;
    dx * dx + dy * dy <= rayon * rayon
}

fn logo_alpha(px: usize, py: usize, taille: usize) -> u8 {
    let mut couverture = 0u16;
    for sy in 0..4usize {
        for sx in 0..4usize {
            let nx = ((px * 4 + sx) * 1024 / (taille * 4).max(1)) as i32;
            let ny = ((py * 4 + sy) * 1024 / (taille * 4).max(1)) as i32;
            let hampe = dans_arrondi(nx, ny, 145, 70, 345, 950, 72);
            let haut = nx >= 260
                && dans_ellipse(nx, ny, 455, 315, 345, 245)
                && !dans_ellipse(nx, ny, 465, 315, 155, 105);
            let bas = nx >= 260
                && dans_ellipse(nx, ny, 475, 710, 380, 275)
                && !dans_ellipse(nx, ny, 485, 710, 175, 125);
            if hampe || haut || bas { couverture += 1; }
        }
    }
    (couverture * 255 / 16) as u8
}

fn logo_lisse(taille: usize) -> Vec<BltPixel> {
    const FOND: (u8, u8, u8) = (13, 17, 23);
    const BLEU: (u8, u8, u8) = (68, 168, 255);
    let mut pixels = Vec::with_capacity(taille * taille);
    for y in 0..taille {
        for x in 0..taille {
            let a = logo_alpha(x, y, taille) as u16;
            let melange = |fond: u8, avant: u8| -> u8 {
                ((avant as u16 * a + fond as u16 * (255 - a) + 127) / 255) as u8
            };
            pixels.push(BltPixel::new(
                melange(FOND.0, BLEU.0),
                melange(FOND.1, BLEU.1),
                melange(FOND.2, BLEU.2),
            ));
        }
    }
    pixels
}

fn run() -> uefi::Result {
    uefi::helpers::init()?;
    let gop_handle = boot::get_handle_for_protocol::<GraphicsOutput>()?;
    let edid = read_edid(gop_handle.clone());
    let native = edid_native(&edid);

    let mut gop = boot::open_protocol_exclusive::<GraphicsOutput>(gop_handle)?;
    let modes: Vec<_> = gop.modes().collect();

    let mut report = String::new();
    let _ = writeln!(report, "BOUCHAUD UEFI PREBOOT PROBE V2");
    let _ = writeln!(report, "edid.bytes={}", edid.len());
    let _ = writeln!(report, "edid.native={}", native.map(|(w,h)| alloc::format!("{}x{}", w, h)).unwrap_or_else(|| String::from("unavailable")));

    for (index, mode) in modes.iter().enumerate() {
        let info = mode.info();
        let (w, h) = info.resolution();
        let _ = writeln!(report, "gop.mode.{}={}x{} stride={} format={:?}", index, w, h, info.stride(), info.pixel_format());
    }

    let selected = native
        .and_then(|wanted| modes.iter().copied().find(|m| m.info().resolution() == wanted))
        .or_else(|| modes.iter().copied().filter(|m| { let (w,h)=m.info().resolution(); h != 0 && w.saturating_mul(9) == h.saturating_mul(16) }).max_by_key(|m| { let (w,h)=m.info().resolution(); w.saturating_mul(h) }))
        .or_else(|| modes.iter().copied().max_by_key(|m| { let (w,h)=m.info().resolution(); w.saturating_mul(h) }));

    if let Some(mode) = selected {
        let (w, h) = mode.info().resolution();
        if gop.set_mode(&mode).is_ok() { let _ = writeln!(report, "gop.selected={}x{}", w, h); }
        else { let _ = writeln!(report, "gop.selected=SET_MODE_FAILED"); }
    }

    let current = gop.current_mode_info();
    let (cw, ch) = current.resolution();
    let _ = writeln!(report, "gop.current={}x{} stride={} format={:?}", cw, ch, current.stride(), current.pixel_format());

    let pointer_count = boot::find_handles::<Pointer>().map(|v| v.len()).unwrap_or(0);
    let keyboard_count = boot::find_handles::<Input>().map(|v| v.len()).unwrap_or(0);
    let _ = writeln!(report, "firmware.pointer_handles={}", pointer_count);
    let _ = writeln!(report, "firmware.keyboard_handles={}", keyboard_count);
    let _ = writeln!(report, "note=written before ExitBootServices; kernel xHCI report follows after handoff");
    // Keep a quiet brand screen while the loader reads the kernel/ramdisk.
    let _ = gop.blt(BltOp::VideoFill { color: BltPixel::new(13,17,23), dest: (0,0), dims: (cw,ch) });
    if cw >= 160 && ch >= 160 {
        let taille = 128usize;
        let logo = logo_lisse(taille);
        let _ = gop.blt(BltOp::BufferToVideo {
            buffer: &logo,
            src: BltRegion::Full,
            dest: ((cw - taille) / 2, (ch - taille) / 2),
            dims: (taille, taille),
        });
    }
    drop(gop);

    let fs_proto = boot::get_image_file_system(boot::image_handle())?;
    let mut fs = FileSystem::new(fs_proto);
    let _ = fs.write(cstr16!("\\BOUCHAUD-PREBOOT.TXT"), report.as_bytes());

    let loader = match fs.read(cstr16!("\\EFI\\BOOT\\BOUCHAUD-LOADER.EFI")) {
        Ok(bytes) => bytes,
        Err(_) => return Err(Status::NOT_FOUND.into()),
    };
    drop(fs);

    // Preserve a full device path for the child image. The bootloader then
    // keeps a valid DeviceHandle and can reopen the same FAT to load
    // kernel-x86_64, boot.json and /ramdisk.
    let loaded_path = boot::open_protocol_exclusive::<LoadedImageDevicePath>(
        boot::image_handle(),
    )?;
    let child = boot::load_image(
        boot::image_handle(),
        LoadImageSource::FromBuffer {
            buffer: &loader,
            file_path: Some(&*loaded_path),
        },
    )?;
    drop(loaded_path);
    boot::start_image(child)
}

#[entry]
fn main() -> Status {
    match run() { Ok(()) => Status::SUCCESS, Err(error) => error.status() }
}
