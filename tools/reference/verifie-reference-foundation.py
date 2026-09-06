#!/usr/bin/env python3
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]

def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")

def require(condition: bool, message: str) -> None:
    if not condition:
        print(f"ECHEC: {message}", file=sys.stderr)
        raise SystemExit(1)

cargo = read("Cargo.toml")
main = read("src/main.rs")
physical = read("src/kernel/memory/physical.rs")
boot_mod = read("src/boot/mod.rs")
legacy = read("src/boot/legacy_x86.rs")
bringup = read("src/platform/pc/bringup.rs")

require("reference-bringup = []" in cargo, "feature reference-bringup absente")
require(
    "fn legacy_boot_entry(legacy: &'static LegacyBootInfo)" in main,
    "frontiere bootloader 0.9 absente",
)
require(
    "fn kernel_main(boot_info: &'static boot::BootInfo)" in main,
    "kernel_main ne consomme pas le contrat Bouchaud",
)
require(
    "bootloader::BootInfo" not in physical,
    "memory/physical.rs depend encore du BootInfo du chargeur",
)
require(
    "pub use legacy_x86::from_bootloader_09;" in boot_mod,
    "adaptateur legacy non expose",
)
require(
    "memory_regions_complete" in legacy,
    "carte memoire tronquee non signalee",
)
require(
    "BOUCHAUD_REFERENCE_BRINGUP_LEGACY_OK" in bringup,
    "marqueur foundation absent",
)

barrier = main.index(
    "platform::pc::bringup::complete_legacy_foundation_and_halt(boot_info);"
)

for forbidden in (
    "drivers::keyboard::init();",
    "drivers::ata::probe();",
    "drivers::ata_bloc::installe();",
    "fs::persistance::monte();",
    "net::demarre();",
    "arch::x86_64::smp::enable_scheduler();",
):
    require(
        main.index(forbidden) > barrier,
        f"{forbidden} est passe avant la barriere de bring-up",
    )

window = main[main.index("arch::x86_64::init();"):barrier]
require(
    "if reference_bringup" in window
    and "arch::x86_64::smp::init_probe();" in window,
    "le probe SMP n'est pas protege par reference_bringup",
)

print("REFERENCE_FOUNDATION_GUARD_OK")
