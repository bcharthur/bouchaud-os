#!/usr/bin/env python3
from pathlib import Path
import re, sys
ROOT=Path(__file__).resolve().parents[2]

def text(rel):
    p=ROOT/rel
    if not p.is_file(): raise SystemExit(f"ECHEC: fichier absent: {rel}")
    return p.read_text(encoding="utf-8-sig")
def req(cond,msg):
    if not cond: raise SystemExit("ECHEC: "+msg)

info=text("src/boot/info.rs")
api=text("src/boot/api_x86.rs")
legacy=text("src/boot/legacy_x86.rs")
backing=text("src/fs/backing.rs")
tar=text("src/fs/tar.rs")
drivers=text("src/drivers/mod.rs")
e1000=text("src/drivers/network/e1000.rs")
rtl=text("src/drivers/network/rtl8168.rs")
net=text("src/net/mod.rs")
pci=text("src/arch/x86_64/pci.rs")
stage2=text("src/platform/pc/stage2.rs")
builder=text("tools/reference/uefi-image-builder/src/main.rs")
builder_cargo=text("tools/reference/uefi-image-builder/Cargo.toml")
build=text("tools/reference/build-reference-stage2.ps1")

for tok in ("RamdiskInfo", "pub ramdisk: Option<RamdiskInfo>"):
    req(tok in info, f"BootInfo ramdisk incomplet: {tok}")
for tok in ("ramdisk_addr", "ramdisk_len", "ramdisk_info(api)"):
    req(tok in api, f"adaptateur UEFI ramdisk incomplet: {tok}")
req("ramdisk: None" in legacy, "legacy doit explicitement fournir ramdisk=None")
for tok in ("BackingSource::Memory", "register_memory", "is_memory_backed"):
    req(tok in backing, f"backing RAM UEFI incomplet: {tok}")
for tok in ("index_boot_ramdisk", "mount_boot_ramdisk", "register_memory"):
    req(tok in tar, f"USTAR ramdisk incomplet: {tok}")
req('network/rtl8168.rs' in drivers, "module RTL8168 non exporte")
for tok in ("0x10EC", "0x8168", "BOUCHAUD_TRIGKEY_RTL8168_DETECTED", "BOUCHAUD_TRIGKEY_RTL8168_DRIVER_OK", "DESC_OWN"):
    req(tok in rtl, f"driver RTL8168 incomplet: {tok}")
req("device.device == 0x100E" in e1000, "e1000 n'est pas limite au 82540EM QEMU")
req("rtl8168::init_with_device" in e1000, "dispatcher RTL8168 absent")
req("SansConfiguration" in net and "using_rtl8168" in net,
    "fallback QEMU encore utilisable sur NIC physique")
req("find_xhci" in pci, "inventaire xHCI absent")
for tok in ("boot.ramdisk", "mount_boot_ramdisk", "BOUCHAUD_TRIGKEY_LADYBIRD_RAMDISK_OK", "BOUCHAUD_TRIGKEY_XHCI_PRESENT"):
    req(tok in stage2, f"Stage2 physique incomplet: {tok}")
req("drivers::ata::probe" in stage2 and "mount_data_disk" in stage2,
    "fallback QEMU ATA supprime par erreur")
for tok in (
    "[ramdisk]",
    "minimum_framebuffer_width",
    "minimum_framebuffer_height",
    "BOUCHAUD_UEFI_FB_REQUEST",
    "BOUCHAUD_UEFI_RAMDISK_EMBED",
    "BOUCHAUD_UEFI_FAT_HEADROOM",
    "create_pxe_tftp_folder",
    "create_large_uefi_fat",
    "FatType::Fat32",
    "bytes_per_cluster(4096)",
    "FAT_MIN_HEADROOM",
):
    req(tok in builder, f"image builder GOP+ramdisk large FAT incomplet: {tok}")
req(".set_ramdisk(" not in builder,
    "le chemin physique utilise encore le FAT sous-dimensionne du builder officiel")
for tok in ('fatfs = { version = "=0.3.6"', 'gpt = "=3.1.0"'):
    req(tok in builder_cargo, f"dependance large FAT absente: {tok}")
for tok in ("[string]$Ramdisk", "[int]$MinWidth", "[int]$MinHeight", "BOUCHAUD_STAGE2_SINGLE_USB_RAMDISK_READY", "STAGE2_GOP_MIN_REQUEST"):
    req(tok in build, f"build-reference-stage2 GOP+ramdisk incomplet: {tok}")
# Invariant critique: ramdisk_addr est deja virtuel.
req("physical_memory_offset" not in re.search(r"fn ramdisk_info\(.*?\n}\n", api, re.S).group(0),
    "ramdisk_info ajoute physical_memory_offset a une adresse deja virtuelle")
print("BOUCHAUD_TRIGKEY_PHYSICAL_GUARD_V2_3_OK")
