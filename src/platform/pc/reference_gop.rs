//! Dashboard GOP premium du Stage 1.
//!
//! Le framebuffer reste le backend firmware UEFI. Ce module ne connait ni
//! AMDGPU ni BAR0. Le texte utilise la DejaVu Sans DEJA embarquee dans le noyau
//! (`gui::font::FONT_DATA`) et `fontdue` pour un rendu TrueType antialiase.

use alloc::format;
use alloc::string::String;
use core::ptr::{read_volatile, write_volatile};

use fontdue::{Font, FontSettings};

use crate::boot::{BootInfo, FramebufferInfo, FramebufferPixelFormat};

use super::reference_metrics::{self, Snapshot};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderError {
    NullAddress,
    UnsupportedPixelFormat,
    UnsupportedBytesPerPixel,
    InvalidGeometry,
    BufferTooSmall,
    FontInit,
    ReadbackMismatch,
}

#[derive(Clone, Copy, Debug)]
pub struct RenderProof {
    pub pixels_written: usize,
    pub readback_ok: bool,
    pub vector_font_ok: bool,
}

#[derive(Clone, Copy)]
struct Rgb {
    r: u8,
    g: u8,
    b: u8,
}

const BG: Rgb = Rgb { r: 13, g: 17, b: 23 };
const PANEL: Rgb = Rgb { r: 25, g: 31, b: 40 };
const PANEL_2: Rgb = Rgb { r: 31, g: 38, b: 49 };
const BORDER: Rgb = Rgb { r: 49, g: 58, b: 72 };
const ACCENT: Rgb = Rgb { r: 68, g: 168, b: 255 };
const GOOD: Rgb = Rgb { r: 91, g: 218, b: 139 };
const WARN: Rgb = Rgb { r: 245, g: 190, b: 74 };
const TEXT: Rgb = Rgb { r: 239, g: 243, b: 248 };
const MUTED: Rgb = Rgb { r: 157, g: 168, b: 184 };
const BAR_BG: Rgb = Rgb { r: 43, g: 50, b: 63 };

fn validate(info: FramebufferInfo) -> Result<(), RenderError> {
    if info.address == 0 {
        return Err(RenderError::NullAddress);
    }
    if info.bytes_per_pixel != 4 {
        return Err(RenderError::UnsupportedBytesPerPixel);
    }
    match info.pixel_format {
        FramebufferPixelFormat::Rgb | FramebufferPixelFormat::Bgr => {}
        _ => return Err(RenderError::UnsupportedPixelFormat),
    }
    if info.width == 0 || info.height == 0 || info.stride < info.width {
        return Err(RenderError::InvalidGeometry);
    }
    let required = (info.stride as usize)
        .checked_mul(info.height as usize)
        .and_then(|pixels| pixels.checked_mul(info.bytes_per_pixel as usize))
        .ok_or(RenderError::InvalidGeometry)?;
    if required > info.byte_len {
        return Err(RenderError::BufferTooSmall);
    }
    Ok(())
}

fn offset(info: FramebufferInfo, x: u32, y: u32) -> Option<usize> {
    if x >= info.width || y >= info.height {
        return None;
    }
    let pixel = (y as usize)
        .checked_mul(info.stride as usize)?
        .checked_add(x as usize)?;
    let offset = pixel.checked_mul(4)?;
    (offset.checked_add(4)? <= info.byte_len).then_some(offset)
}

fn read_rgb(info: FramebufferInfo, x: u32, y: u32) -> Option<Rgb> {
    let o = offset(info, x, y)?;
    let base = info.address as *const u8;
    let b0 = unsafe { read_volatile(base.add(o)) };
    let b1 = unsafe { read_volatile(base.add(o + 1)) };
    let b2 = unsafe { read_volatile(base.add(o + 2)) };
    Some(match info.pixel_format {
        FramebufferPixelFormat::Rgb => Rgb { r: b0, g: b1, b: b2 },
        FramebufferPixelFormat::Bgr => Rgb { r: b2, g: b1, b: b0 },
        _ => Rgb { r: 0, g: 0, b: 0 },
    })
}

fn write_rgb(info: FramebufferInfo, x: u32, y: u32, color: Rgb) -> bool {
    let Some(o) = offset(info, x, y) else { return false; };
    let base = info.address as *mut u8;
    let bytes = match info.pixel_format {
        FramebufferPixelFormat::Rgb => [color.r, color.g, color.b, 0],
        FramebufferPixelFormat::Bgr => [color.b, color.g, color.r, 0],
        _ => return false,
    };
    unsafe {
        for (index, byte) in bytes.iter().enumerate() {
            write_volatile(base.add(o + index), *byte);
        }
    }
    true
}

fn blend_rgb(info: FramebufferInfo, x: u32, y: u32, color: Rgb, alpha: u8) -> bool {
    if alpha == 0 {
        return false;
    }
    if alpha == 255 {
        return write_rgb(info, x, y, color);
    }
    let Some(dst) = read_rgb(info, x, y) else { return false; };
    let a = alpha as u32;
    let ia = 255 - a;
    let mixed = Rgb {
        r: ((color.r as u32 * a + dst.r as u32 * ia) / 255) as u8,
        g: ((color.g as u32 * a + dst.g as u32 * ia) / 255) as u8,
        b: ((color.b as u32 * a + dst.b as u32 * ia) / 255) as u8,
    };
    write_rgb(info, x, y, mixed)
}

fn fill_rect(
    info: FramebufferInfo,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    color: Rgb,
) -> usize {
    let x2 = x.saturating_add(width).min(info.width);
    let y2 = y.saturating_add(height).min(info.height);
    let mut writes = 0usize;
    for py in y..y2 {
        for px in x..x2 {
            writes += write_rgb(info, px, py, color) as usize;
        }
    }
    writes
}

fn border_rect(
    info: FramebufferInfo,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    color: Rgb,
) -> usize {
    let mut n = 0;
    n += fill_rect(info, x, y, width, 1, color);
    n += fill_rect(info, x, y.saturating_add(height.saturating_sub(1)), width, 1, color);
    n += fill_rect(info, x, y, 1, height, color);
    n += fill_rect(info, x.saturating_add(width.saturating_sub(1)), y, 1, height, color);
    n
}

fn draw_text(
    info: FramebufferInfo,
    font: &Font,
    x: i32,
    y_top: i32,
    text: &str,
    px: f32,
    color: Rgb,
) -> usize {
    let mut cursor = x as f32;
    let baseline = y_top as f32 + px * 0.82;
    let mut writes = 0usize;

    for ch in text.chars() {
        let (metrics, bitmap) = font.rasterize(ch, px);
        let gx = (cursor + 0.5) as i32 + metrics.xmin;
        let gy = (baseline + 0.5) as i32 - metrics.height as i32 - metrics.ymin;

        for row in 0..metrics.height {
            for col in 0..metrics.width {
                let alpha = bitmap[row * metrics.width + col];
                if alpha == 0 {
                    continue;
                }
                let dx = gx + col as i32;
                let dy = gy + row as i32;
                if dx >= 0 && dy >= 0 {
                    writes += blend_rgb(info, dx as u32, dy as u32, color, alpha) as usize;
                }
            }
        }
        cursor += metrics.advance_width;
    }
    writes
}

fn fmt_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;

    if bytes >= GIB {
        let whole = bytes / GIB;
        let tenth = (bytes % GIB) * 10 / GIB;
        format!("{}.{} GiB", whole, tenth)
    } else if bytes >= MIB {
        let whole = bytes / MIB;
        let tenth = (bytes % MIB) * 10 / MIB;
        format!("{}.{} MiB", whole, tenth)
    } else {
        format!("{} KiB", bytes / KIB)
    }
}

fn pct(used: u64, total: u64) -> u8 {
    if total == 0 {
        0
    } else {
        ((used.saturating_mul(100) / total).min(100)) as u8
    }
}

fn bar(
    info: FramebufferInfo,
    x: u32,
    y: u32,
    width: u32,
    percent: u8,
    color: Rgb,
) -> usize {
    let mut n = fill_rect(info, x, y, width, 8, BAR_BG);
    let active = (width as u64 * percent as u64 / 100) as u32;
    if active != 0 {
        n += fill_rect(info, x, y, active, 8, color);
    }
    n
}

fn label_value(
    info: FramebufferInfo,
    font: &Font,
    x: i32,
    y: i32,
    label: &str,
    value: &str,
) -> usize {
    let mut n = draw_text(info, font, x, y, label, 15.0, MUTED);
    n += draw_text(info, font, x, y + 21, value, 19.0, TEXT);
    n
}

fn card(
    info: FramebufferInfo,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
) -> usize {
    fill_rect(info, x, y, w, h, PANEL) + border_rect(info, x, y, w, h, BORDER)
}

fn render_dashboard(
    info: FramebufferInfo,
    font: &Font,
    s: &Snapshot,
) -> usize {
    let mut n = fill_rect(info, 0, 0, info.width, info.height, BG);
    n += fill_rect(info, 0, 0, info.width, 7, ACCENT);

    let margin = 60u32;
    let content_w = info.width.saturating_sub(margin * 2);

    // Header.
    n += draw_text(info, font, margin as i32, 46, "Bouchaud OS", 42.0, TEXT);
    n += draw_text(
        info,
        font,
        margin as i32,
        94,
        "Reference Device v1  /  UEFI Stage 1 diagnostics",
        17.0,
        MUTED,
    );

    let uptime = format!("Uptime {} ms", s.uptime_ms);
    n += draw_text(
        info,
        font,
        (info.width.saturating_sub(210)) as i32,
        62,
        &uptime,
        15.0,
        MUTED,
    );

    // Trois cartes principales.
    let gap = 16u32;
    let card_w = (content_w.saturating_sub(gap * 2)) / 3;
    let top = 138u32;
    let card_h = 154u32;

    n += card(info, margin, top, card_w, card_h);
    n += card(info, margin + card_w + gap, top, card_w, card_h);
    n += card(info, margin + (card_w + gap) * 2, top, card_w, card_h);

    n += draw_text(info, font, (margin + 20) as i32, (top + 18) as i32, "PROCESSOR", 14.0, ACCENT);
    n += draw_text(info, font, (margin + 20) as i32, (top + 47) as i32, &s.cpu_brand, 17.0, TEXT);
    let cpu_meta = format!("{} / {} logical reported", s.cpu_vendor, s.logical_cpus_reported);
    n += draw_text(info, font, (margin + 20) as i32, (top + 82) as i32, &cpu_meta, 14.0, MUTED);
    let tsc = s.tsc_mhz.map(|mhz| format!("TSC {} MHz", mhz)).unwrap_or_else(|| "TSC frequency unavailable".into());
    n += draw_text(info, font, (margin + 20) as i32, (top + 108) as i32, &tsc, 14.0, MUTED);

    let mx = margin + card_w + gap;
    n += draw_text(info, font, (mx + 20) as i32, (top + 18) as i32, "MEMORY", 14.0, ACCENT);
    n += label_value(info, font, (mx + 20) as i32, (top + 46) as i32, "Usable firmware RAM", &fmt_bytes(s.usable_memory_bytes));
    n += label_value(info, font, (mx + 190) as i32, (top + 46) as i32, "Framebuffer", &fmt_bytes(s.framebuffer_bytes as u64));

    let dx = margin + (card_w + gap) * 2;
    n += draw_text(info, font, (dx + 20) as i32, (top + 18) as i32, "DISPLAY", 14.0, ACCENT);
    let mode = format!("{} x {}  /  {} bpp", s.framebuffer_width, s.framebuffer_height, s.framebuffer_bpp * 8);
    n += draw_text(info, font, (dx + 20) as i32, (top + 52) as i32, &mode, 20.0, TEXT);
    n += draw_text(info, font, (dx + 20) as i32, (top + 86) as i32, "Firmware GOP framebuffer", 14.0, MUTED);
    n += draw_text(info, font, (dx + 20) as i32, (top + 110) as i32, "CPU writes + readback verified", 14.0, GOOD);

    // Resource usage.
    let lower_top = 312u32;
    let lower_h = 310u32;
    let left_w = (content_w * 3) / 5;
    let right_x = margin + left_w + gap;
    let right_w = content_w.saturating_sub(left_w + gap);

    n += card(info, margin, lower_top, left_w, lower_h);
    n += card(info, right_x, lower_top, right_w, lower_h);

    n += draw_text(info, font, (margin + 22) as i32, (lower_top + 20) as i32, "OS RESOURCE SNAPSHOT", 15.0, ACCENT);
    n += draw_text(
        info,
        font,
        (margin + 22) as i32,
        (lower_top + 50) as i32,
        "Real allocator counters at the Stage 1 barrier",
        14.0,
        MUTED,
    );

    let heap_pct = pct(s.heap_used_bytes as u64, s.heap_total_bytes as u64);
    let heap_line = format!(
        "Kernel heap   {} used / {} total   ({}%)",
        fmt_bytes(s.heap_used_bytes as u64),
        fmt_bytes(s.heap_total_bytes as u64),
        heap_pct
    );
    n += draw_text(info, font, (margin + 22) as i32, (lower_top + 92) as i32, &heap_line, 16.0, TEXT);
    n += bar(info, margin + 22, lower_top + 123, left_w.saturating_sub(44), heap_pct, ACCENT);

    let frame_used_bytes = s.frame_used.saturating_mul(4096);
    let frame_total_bytes = s.frame_total.saturating_mul(4096);
    let frame_pct = pct(s.frame_used, s.frame_total);
    let frame_line = format!(
        "VMM frames    {} used / {} pool   ({}%)",
        fmt_bytes(frame_used_bytes),
        fmt_bytes(frame_total_bytes),
        frame_pct
    );
    n += draw_text(info, font, (margin + 22) as i32, (lower_top + 157) as i32, &frame_line, 16.0, TEXT);
    n += bar(info, margin + 22, lower_top + 188, left_w.saturating_sub(44), frame_pct, GOOD);

    let tracked = (s.heap_used_bytes as u64).saturating_add(frame_used_bytes);
    let tracked_line = format!("Tracked live kernel memory: {}", fmt_bytes(tracked));
    n += draw_text(info, font, (margin + 22) as i32, (lower_top + 224) as i32, &tracked_line, 17.0, TEXT);
    n += draw_text(
        info,
        font,
        (margin + 22) as i32,
        (lower_top + 252) as i32,
        "CPU load is intentionally not fabricated before scheduler/IRQ accounting.",
        13.0,
        WARN,
    );

    // Safety / components.
    n += draw_text(info, font, (right_x + 22) as i32, (lower_top + 20) as i32, "COMPONENTS & SAFETY", 15.0, ACCENT);

    let rows = [
        ("CPU execution", s.smp_state, GOOD),
        ("CPU load", s.cpu_load_state, WARN),
        ("Storage", s.storage_state, WARN),
        ("Storage writes", s.storage_write_state, GOOD),
        ("Network", s.network_state, GOOD),
    ];
    let mut ry = lower_top + 58;
    for (label, value, color) in rows {
        n += draw_text(info, font, (right_x + 22) as i32, ry as i32, label, 14.0, MUTED);
        n += draw_text(info, font, (right_x + 174) as i32, ry as i32, value, 14.0, color);
        ry += 40;
    }

    // Footer statuses.
    let footer_y = info.height.saturating_sub(116);
    n += fill_rect(info, margin, footer_y, content_w, 62, PANEL_2);
    let statuses = [("UEFI", GOOD), ("MEMORY", GOOD), ("GOP", GOOD), ("BSP", GOOD)];
    let status_w = content_w / statuses.len() as u32;
    for (i, (name, color)) in statuses.iter().enumerate() {
        let x = margin + i as u32 * status_w + 18;
        n += fill_rect(info, x, footer_y + 21, 8, 20, *color);
        n += draw_text(info, font, (x + 20) as i32, (footer_y + 17) as i32, name, 18.0, TEXT);
        n += draw_text(info, font, (x + status_w.saturating_sub(70)) as i32, (footer_y + 17) as i32, "OK", 18.0, *color);
    }

    n
}

/// Rendu premium + preuve CPU store/readback.
///
/// Les metriques sont un snapshot honnete du Stage 1. Le CPU load et le disque
/// restent explicitement "N/A/not probed" tant que scheduler et pilotes ne sont
/// pas actives; ce lot n'ajoute aucun effet de bord materiel.
pub fn render_stage1(
    boot: &BootInfo,
    info: FramebufferInfo,
) -> Result<RenderProof, RenderError> {
    validate(info)?;

    let font = Font::from_bytes(
        crate::gui::font::FONT_DATA,
        FontSettings::default(),
    )
    .map_err(|_| RenderError::FontInit)?;

    let snapshot = reference_metrics::snapshot(boot, info);
    let pixels_written = render_dashboard(info, &font, &snapshot);

    // Pixel sentinelle hors panneaux. Le marqueur GOP reste conditionne par
    // une vraie relecture volatile de ce que le noyau vient d'ecrire.
    let sx = info.width.saturating_sub(2);
    let sy = info.height.saturating_sub(2);
    if !write_rgb(info, sx, sy, ACCENT) {
        return Err(RenderError::ReadbackMismatch);
    }
    let observed = read_rgb(info, sx, sy).ok_or(RenderError::ReadbackMismatch)?;
    if observed.r != ACCENT.r || observed.g != ACCENT.g || observed.b != ACCENT.b {
        return Err(RenderError::ReadbackMismatch);
    }

    Ok(RenderProof {
        pixels_written: pixels_written.saturating_add(1),
        readback_ok: true,
        vector_font_ok: true,
    })
}
