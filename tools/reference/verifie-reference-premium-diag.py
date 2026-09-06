#!/usr/bin/env python3
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def require(ok: bool, message: str) -> None:
    if not ok:
        print(f"ECHEC: {message}", file=sys.stderr)
        raise SystemExit(1)


pc_mod = read("src/platform/pc/mod.rs")
bringup = read("src/platform/pc/bringup.rs")
gop = read("src/platform/pc/reference_gop.rs")
metrics = read("src/platform/pc/reference_metrics.rs")
font = read("src/gui/font.rs")

# Police : reutilisation du meme TTF que le GUI, sans dupliquer un asset.
require("pub mod reference_metrics;" in pc_mod, "module telemetry non cable")
require('pub static FONT_DATA: &[u8] = include_bytes!("../assets/fonts/DejaVuSans.ttf");' in font,
        "DejaVuSans embarquee du GUI introuvable")
require("use fontdue::{Font, FontSettings};" in gop, "fontdue absent")
require("crate::gui::font::FONT_DATA" in gop, "FONT_DATA existante non reutilisee")
require("Font::from_bytes" in gop, "TrueType non initialise")
require("font.rasterize(ch, px)" in gop, "glyphes TrueType non rasterises")
require("blend_rgb" in gop and "alpha" in gop,
        "antialias par couverture alpha absent")
require("vector_font_ok: true" in gop, "preuve police vectorielle absente")
require("cursor.round()" not in gop and "baseline.round()" not in gop,
        "appel f32::round incompatible no_std encore present")

# Telemetrie réellement disponible au Stage 1.
require("reference_metrics::snapshot(boot, info)" in gop, "snapshot non rendu")
require("crate::kernel::heap::stats()" in metrics, "stats heap reelles absentes")
require("crate::kernel::vmm::frame_stats_relaxed()" in metrics, "stats VMM reelles absentes")
require("MemoryRegionKind::Usable" in metrics, "RAM UEFI utilisable non comptee")
require("__cpuid" in metrics, "CPUID absent")
require("cpu_brand()" in metrics and "cpu_vendor()" in metrics,
        "identite CPU non collectee")
require("crate::kernel::timer::tsc_hz()" in metrics, "frequence TSC non collectee")
require("crate::kernel::timer::monotonic_ms()" in metrics, "uptime non collecte")

# Les valeurs indisponibles ne doivent pas etre inventees.
require('storage_state: "Not probed - safe Stage 1"' in metrics,
        "stockage n'est plus explicitement non sonde")
require('storage_write_state: "OFF"' in metrics,
        "ecritures stockage non marquees OFF")
require('network_state: "OFF"' in metrics, "reseau non marque OFF")
require('smp_state: "BSP only"' in metrics, "BSP-only non expose")
require('cpu_load_state: "N/A before scheduler"' in metrics,
        "charge CPU Stage 1 devrait rester N/A")

# Aucun pilote de stockage/reseau ne doit etre introduit par le collecteur.
for forbidden in (
    "drivers::ata",
    "drivers::nvme",
    "drivers::network",
    "net::demarre",
    "pci::",
):
    require(forbidden not in metrics, f"sonde materielle interdite dans metrics: {forbidden}")

require("reference_gop::render_stage1(boot, framebuffer)" in bringup,
        "BootInfo non transmis au dashboard")
require("vector_font={}" in bringup, "preuve vector_font non journalisee")

print("REFERENCE_PREMIUM_DIAG_GUARD_OK")
