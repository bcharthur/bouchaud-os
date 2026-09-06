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

cargo = read("Cargo.toml")
main = read("src/main.rs")
boot_mod = read("src/boot/mod.rs")
api = read("src/boot/api_x86.rs")
bringup = read("src/platform/pc/bringup.rs")
builder = read("tools/reference/uefi-image-builder/Cargo.toml")

require('default = ["legacy-boot"]' in cargo, "legacy-boot n'est pas le chemin par defaut")
require('legacy-boot = ["dep:bootloader"]' in cargo, "feature legacy-boot absente")
require('uefi-boot = ["dep:bootloader_api"]' in cargo, "feature uefi-boot absente")
require('bootloader_api = { version = "=0.11.15"' in cargo, "bootloader_api n'est pas epingle a 0.11.15")
require('optional = true' in cargo, "dependances de boot non optionnelles")
require('compile_error!("legacy-boot et uefi-boot sont mutuellement exclusifs")' in main,
        "garde features mutuellement exclusives absente")
require(
    re.search(
        r"bootloader_api::entry_point!\s*\(\s*uefi_boot_entry\s*,",
        main,
        re.MULTILINE,
    ) is not None,
    "vrai entry_point bootloader_api absent",
)
require('Mapping::Dynamic' in main, "mapping physique dynamique UEFI absent")
require('from_bootloader_api(api)' in main, "adaptateur UEFI non appele")
require('mod api_x86;' in boot_mod and 'pub use api_x86::from_bootloader_api;' in boot_mod,
        "adaptateur UEFI non cable")
require('framebuffer.buffer_mut()' in api, "framebuffer mappe non consomme")
require('BOUCHAUD_UEFI_ENTRY_OK' in main, "marqueur entree UEFI absent")
require('BOUCHAUD_UEFI_BOOTINFO_OK' in bringup, "marqueur BootInfo UEFI absent")
require('complete_uefi_bootinfo_and_halt' in main, "barriere UEFI precoce absente")
require('bootloader = { version = "=0.11.15"' in builder,
        "image builder non epingle a 0.11.15")
require('features = ["uefi"]' in builder, "image builder ne construit pas UEFI")

# La preuve UEFI doit s'arrêter avant l'initialisation arch/PIC/PCI.
barrier = main.index("platform::pc::bringup::complete_uefi_bootinfo_and_halt(boot_info);")
arch_init = main.index("arch::x86_64::init();")
require(barrier < arch_init, "barriere UEFI placee apres arch::init")

print("REFERENCE_UEFI_BOOTINFO_GUARD_OK")
