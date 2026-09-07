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


def ordered(text: str, *needles: str) -> bool:
    pos = -1
    for needle in needles:
        pos = text.find(needle, pos + 1)
        if pos < 0:
            return False
    return True


main = read("src/main.rs")
pc_mod = read("src/platform/pc/mod.rs")
bringup = read("src/platform/pc/bringup.rs")
gop = read("src/platform/pc/reference_gop.rs")
runner = read("tools/reference/run-reference-gop.ps1")

# Wiring Stage 1.
require("pub mod reference_gop;" in pc_mod, "module reference_gop non cable")
require("complete_uefi_stage1_and_halt(boot_info)" in main,
        "main n'appelle pas la barriere UEFI Stage 1")
require("complete_uefi_bootinfo_and_halt(boot_info)" not in main,
        "ancien arret Lot 2A encore actif")
require("reference_gop::render_stage1(boot, framebuffer)" in bringup,
        "bringup n'appelle pas le renderer Lot 2C")

# La preuve serie n'est emise qu'apres retour avec succes du renderer.
require(ordered(
    bringup,
    "reference_gop::render_stage1(boot, framebuffer)",
    'serial_println!("BOUCHAUD_GOP_OK")',
    'serial_println!("BOUCHAUD_REFERENCE_BOOT_STAGE1_OK")',
), "ordre renderer -> GOP_OK -> STAGE1_OK invalide")
require('.expect("bringup UEFI: ecriture/readback GOP en echec")' in bringup,
        "echec renderer non fail-closed")

# Preuve d'ecriture/readback de l'implementation ACTUELLE. Ne pas rechercher
# une variable `expected` de l'ancien Lot 2B : le Lot 2C compare les canaux RGB.
require("write_volatile" in gop, "aucune ecriture volatile framebuffer")
require("read_volatile" in gop, "aucune relecture volatile framebuffer")
require("if !write_rgb(info, sx, sy, ACCENT)" in gop,
        "pixel sentinelle non ecrit")
require("let observed = read_rgb(info, sx, sy)" in gop,
        "pixel sentinelle non relu")
require(
    "observed.r != ACCENT.r" in gop
    and "observed.g != ACCENT.g" in gop
    and "observed.b != ACCENT.b" in gop,
    "readback RGB non compare au pixel sentinelle",
)
require(ordered(
    gop,
    "if !write_rgb(info, sx, sy, ACCENT)",
    "let observed = read_rgb(info, sx, sy)",
    "observed.r != ACCENT.r",
    "Ok(RenderProof",
), "ordre store -> readback -> comparaison -> preuve invalide")
require("ReadbackMismatch" in gop, "echec readback non represente")
require("readback_ok: true" in gop, "preuve readback succes absente")

# Contrat framebuffer fail-closed.
require("info.bytes_per_pixel != 4" in gop, "BPP 32 bits non exige")
require("FramebufferPixelFormat::Bgr" in gop and "FramebufferPixelFormat::Rgb" in gop,
        "formats RGB/BGR non geres")
require("UnsupportedPixelFormat" in gop, "format inconnu non refuse")
require("required > info.byte_len" in gop, "taille framebuffer non bornee")
require("offset.checked_add(4)? <= info.byte_len" in gop,
        "ecriture pixel non bornee par byte_len")

# Barriere avant les sous-systemes a effets de bord du boot produit.
barrier_call = "platform::pc::bringup::complete_uefi_stage1_and_halt(boot_info);"
require(barrier_call in main, "barriere Stage 1 absente")
barrier = main.index(barrier_call)
require("arch::x86_64::init();" in main, "arch init introuvable")
require(barrier < main.index("arch::x86_64::init();"),
        "barriere Stage 1 placee apres arch::init")
prefix = main[:barrier]
for forbidden in (
    "drivers::ata::probe();",
    "drivers::ata_bloc::installe();",
    "fs::persistance::monte();",
    "net::demarre();",
    "arch::x86_64::smp::init_probe();",
):
    require(forbidden not in prefix, f"effet de bord avant barriere: {forbidden}")

# Runner OVMF de preuve : 1 vCPU, aucun reseau, fenetre graphique visible.
require('"-display", "none"' not in runner, "runner masque l'affichage")
require('"-smp", "1"' in runner, "runner n'impose pas BSP/1 vCPU")
require('"-net", "none"' in runner, "runner n'isole pas le reseau")
require("BOUCHAUD_GOP_OK" in runner, "runner n'annonce pas GOP_OK")
require("BOUCHAUD_REFERENCE_BOOT_STAGE1_OK" in runner,
        "runner n'annonce pas STAGE1_OK")

print("REFERENCE_GOP_STAGE1_GUARD_OK")
