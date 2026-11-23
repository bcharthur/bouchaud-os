//! Live system metrics. The same snapshot is available in /proc/bouchaud/metrics.

use crate::gui::{framebuffer as fb, window::clip};
use crate::kernel::system_metrics::{current, value};
use alloc::format;

pub(crate) fn draw(bx: usize, by: usize, bw: usize, bh: usize) {
    let cols = bw / 7;
    let mut yy = by;
    let end = by.saturating_add(bh);
    let mut put = |text: &str, color: u32, bold: bool| {
        if yy.saturating_add(20) <= end {
            fb::draw_text_prop(bx, yy, clip(text, cols), color, 14.0, bold);
            yy += 20;
        }
    };
    let white = 0xeff3f8;
    put("Moniteur système", 0xf5be4a, true);
    let Some(s) = current() else {
        put("Mesure en cours", white, false);
        return;
    };
    const MIB: u64 = 1024 * 1024;
    let cpu = s.cpu_busy_pct.map_or_else(|| "N/A".into(), |p| format!("{p} %"));
    put(&format!("CPU   {} · {} cœur(s)", cpu, s.cpu_online), white, false);
    put(&format!("RAM  {}/{} Mio", s.ram_used_bytes / MIB, s.ram_total_bytes / MIB), white, false);
    put(&format!("ATA  {} Kio/s (lectures)", value(s.ata_read_bytes_per_sec.map(|b| b / 1024))), white, false);
    if s.nvme_present {
        put(&format!("NVMe  {} lectures / {} écr.", s.nvme_reads, s.nvme_writes), white, false);
    }
    let mode = if s.virgl_ready { "éligible" } else { "indisponible" };
    put(&format!("VIRGL {mode} · {} trames", s.virgl_frames), white, false);
    put(&format!("VirtIO  {} µs / présent.", value(s.virtio_present_us)), white, false);
    put("Utilisation GPU  N/A", white, false);
}
