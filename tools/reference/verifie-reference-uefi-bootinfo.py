#!/usr/bin/env python3
from pathlib import Path
import re
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


cargo = read("Cargo.toml")
main = read("src/main.rs")
boot_mod = read("src/boot/mod.rs")
api = read("src/boot/api_x86.rs")
bringup = read("src/platform/pc/bringup.rs")
builder = read("tools/reference/uefi-image-builder/Cargo.toml")

require('default = ["legacy-boot"]' in cargo, "legacy-boot n'est pas le chemin par defaut")
require('legacy-boot = ["dep:bootloader"]' in cargo, "feature legacy-boot absente")
require('uefi-boot = ["dep:bootloader_api"]' in cargo, "feature uefi-boot absente")
require('bootloader_api = { version = "=0.11.15"' in cargo,
        "bootloader_api n'est pas epingle a 0.11.15")
require(
    re.search(r"bootloader_api::entry_point!\s*\(\s*uefi_boot_entry\s*,", main, re.MULTILINE)
    is not None,
    "vrai entry_point bootloader_api absent",
)
require("Mapping::Dynamic" in main, "mapping physique dynamique UEFI absent")
require("from_bootloader_api(api)" in main, "adaptateur UEFI non appele")
require("mod api_x86;" in boot_mod and "pub use api_x86::from_bootloader_api;" in boot_mod,
        "adaptateur UEFI non cable")
require("framebuffer.buffer_mut()" in api, "framebuffer mappe non consomme")
require("BOUCHAUD_UEFI_ENTRY_OK" in main, "marqueur entree UEFI absent")
require("BOUCHAUD_UEFI_BOOTINFO_OK" in bringup, "marqueur BootInfo UEFI absent")
require("complete_uefi_stage1_and_halt" in main,
        "barriere UEFI Stage 1 actuelle absente")
require('bootloader = { version = "=0.11.15"' in builder,
        "image builder non epingle a 0.11.15")
require('features = ["uefi"]' in builder, "image builder ne construit pas UEFI")

barrier_call = "platform::pc::bringup::complete_uefi_stage1_and_halt(boot_info);"
require(barrier_call in main, "appel barriere UEFI Stage 1 absent")
barrier = main.index(barrier_call)
arch_init = main.index("arch::x86_64::init();")
require(barrier < arch_init, "barriere UEFI placee apres arch::init")

# Le marqueur BootInfo reste une preuve distincte et doit preceder le rendu GOP.
require(ordered(
    bringup,
    'serial_println!("BOUCHAUD_UEFI_BOOTINFO_OK")',
    "reference_gop::render_stage1(boot, framebuffer)",
    'serial_println!("BOUCHAUD_GOP_OK")',
), "ordre BootInfo -> rendu GOP -> GOP_OK invalide")

print("REFERENCE_UEFI_BOOTINFO_GUARD_OK")
