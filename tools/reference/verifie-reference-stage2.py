#!/usr/bin/env python3
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[2]

def read(rel: str) -> str:
    p = ROOT / rel
    if not p.is_file():
        print(f"ECHEC: fichier absent: {rel}", file=sys.stderr)
        raise SystemExit(1)
    return p.read_text(encoding="utf-8-sig").replace("\r\n", "\n")

def req(ok: bool, msg: str) -> None:
    if not ok:
        print(f"ECHEC: {msg}", file=sys.stderr)
        raise SystemExit(1)

def strip_rust_noncode(src: str) -> str:
    out = []
    i = 0
    n = len(src)
    state = "code"
    block_depth = 0
    raw_hashes = 0

    while i < n:
        if state == "code":
            if src.startswith("//", i):
                out.extend("  ")
                i += 2
                state = "line"
            elif src.startswith("/*", i):
                out.extend("  ")
                i += 2
                block_depth = 1
                state = "block"
            elif src[i] == '"':
                out.append(" ")
                i += 1
                state = "string"
            elif src[i] == "r":
                j = i + 1
                while j < n and src[j] == "#":
                    j += 1
                if j < n and src[j] == '"':
                    raw_hashes = j - (i + 1)
                    out.extend(" " * (j - i + 1))
                    i = j + 1
                    state = "raw"
                else:
                    out.append(src[i])
                    i += 1
            else:
                out.append(src[i])
                i += 1

        elif state == "line":
            if src[i] == "\n":
                out.append("\n")
                state = "code"
            else:
                out.append(" ")
            i += 1

        elif state == "block":
            if src.startswith("/*", i):
                out.extend("  ")
                i += 2
                block_depth += 1
            elif src.startswith("*/", i):
                out.extend("  ")
                i += 2
                block_depth -= 1
                if block_depth == 0:
                    state = "code"
            else:
                out.append("\n" if src[i] == "\n" else " ")
                i += 1

        elif state == "string":
            if src[i] == "\\" and i + 1 < n:
                out.extend("  ")
                i += 2
            elif src[i] == '"':
                out.append(" ")
                i += 1
                state = "code"
            else:
                out.append("\n" if src[i] == "\n" else " ")
                i += 1

        elif state == "raw":
            end = '"' + ("#" * raw_hashes)
            if src.startswith(end, i):
                out.extend(" " * len(end))
                i += len(end)
                state = "code"
            else:
                out.append("\n" if src[i] == "\n" else " ")
                i += 1

    return "".join(out)

def rust_call_present(code: str, path: str) -> bool:
    return re.search(r"(?<![A-Za-z0-9_])" + re.escape(path) + r"\s*\(", code) is not None

def function_body(src: str, signature: str) -> str:
    start = src.find(signature)
    if start < 0:
        return ""
    brace = src.find("{", start)
    if brace < 0:
        return ""
    depth = 0
    for i in range(brace, len(src)):
        if src[i] == "{":
            depth += 1
        elif src[i] == "}":
            depth -= 1
            if depth == 0:
                return src[brace + 1:i]
    return ""

# Autotest exact du bug V3.
fake_safe = """
// sans pci::init().
let texte = "pci::init()";
/* net::demarre(); */
"""
fake_bad = """
pci::init();
"""
safe_code = strip_rust_noncode(fake_safe)
bad_code = strip_rust_noncode(fake_bad)
req(not rust_call_present(safe_code, "pci::init"),
    "autotest scanner: commentaire/chaine pris pour un appel")
req(rust_call_present(bad_code, "pci::init"),
    "autotest scanner: vrai appel PCI non detecte")
print("BOUCHAUD_STAGE2_GUARD_SCANNER_SELFTEST_OK")

cargo = read("Cargo.toml")
main = read("src/main.rs")
bringup = read("src/platform/pc/bringup.rs")
pcmod = read("src/platform/pc/mod.rs")
gfx = read("src/drivers/display/bochs.rs")
stage2 = read("src/platform/pc/stage2.rs")
wm = read("src/gui/window_manager.rs")
build = read("tools/reference/build-reference-stage2.ps1")
window = read("src/gui/window.rs")
widgets = read("src/gui/widgets.rs")
widgets_v15 = read("src/gui/widgets_v15.rs")
gui_font = read("src/gui/font.rs")
stage2_preview = read("src/gui/stage2_preview.rs")
mouse_facade = read("src/drivers/input/ps2_mouse.rs")
mouse_state = read("src/drivers/input/mouse/etat.rs")
mouse_ps2 = read("src/drivers/input/mouse/ps2.rs")
mouse_packet = read("src/drivers/input/mouse/paquet.rs")
commands = read("src/shell/commands.rs")
shell = read("src/shell/mod.rs")
ladybird_prepare = read("tools/reference/prepare-reference-ladybird.ps1")
ladybird_make = read("tools/reference/make-reference-ladybird-image.py")
ladybird_verify = read("tools/reference/verify-reference-ladybird-image.py")
ladybird_run = read("tools/reference/run-reference-stage2-ladybird.ps1")

stage2_code = strip_rust_noncode(stage2)
wm_code = strip_rust_noncode(wm)

req('reference-desktop = ["reference-bringup"]' in cargo,
    "feature reference-desktop absente")
req('feature = "reference-desktop"' in main,
    "route reference-desktop absente")
req("stage2::run(boot_info)" in main,
    "kernel_main ne route pas vers Stage 2")
req("pub fn validate_uefi_stage1" in bringup,
    "validation UEFI partagee absente")
req("pub mod stage2;" in pcmod,
    "module Stage 2 absent")

for token in (
    "BOUCHAUD_STAGE2_GOP_BACKEND",
    "install_firmware_framebuffer",
    "firmware_resolution",
    "firmware_scale_milli",
    "FIRMWARE_FB",
    "present_firmware_full",
):
    req(token in gfx, f"backend GOP incomplet: {token}")

req("FramebufferPixelFormat::Rgb" in gfx and "FramebufferPixelFormat::Bgr" in gfx,
    "backend GOP ne gere pas explicitement RGB/BGR")
req("core::ptr::write_volatile" in gfx,
    "ecriture volatile GOP absente")
req("core::ptr::read_volatile" in gfx,
    "readback GOP absent")

req(
    stage2.find("kernel::process::init()") < stage2.find("gui::desktop::run()"),
    "process::init doit preceder le lancement du desktop",
)

for token in (
    "BOUCHAUD_STAGE2_WM_RUNTIME_READY",
    "cpu_local::register_bsp()",
    "cpu_local::mark_online",
    "gdt::init()",
    "idt::init()",
    "interrupts::init()",
    "usermode::init()",
    "users::init()",
    "ramfs::fs().init()",
    "kernel::process::init()",
    "gui::desktop::run()",
):
    req(token in stage2, f"runtime Stage 2 incomplet: {token}")

required_ladybird_calls = (
    "arch::x86_64::pci::init",
    "drivers::ata::probe",
    "drivers::ata_bloc::installe",
    "fs::tar::mount_data_disk",
    "fs::persistance::monte",
    "kernel::sysroot::install",
    "net::demarre",
)
for call in required_ladybird_calls:
    req(rust_call_present(stage2_code, call),
        f"Stage 2 FINAL V2 n'initialise pas Ladybird: {call}()")

forbidden_calls = (
    "arch::x86_64::init",
    "smp::init_probe",
    "smp::enable_scheduler",
)
for call in forbidden_calls:
    req(not rust_call_present(stage2_code, call),
        f"Stage 2 FINAL V2 appelle un sous-systeme hors contrat: {call}()")
print("BOUCHAUD_STAGE2_LADYBIRD_CALL_SCAN_OK")

req("stage2_preview::interactive_loop" not in stage2_code,
    "Stage 2 appelle encore la preview interactive")
req("stage2_preview::render_once" not in stage2_code,
    "Stage 2 appelle encore la preview statique")

run_body = function_body(wm, "pub fn run()")
req(run_body, "window_manager::run introuvable")
req('task::run_noyau(fil_bureau, "desktop");' in run_body,
    "window_manager::run doit lancer un vrai fil noyau")
req("boucle();" not in run_body,
    "window_manager::run ne doit jamais executer boucle() depuis le fil de boot")

fil_body = function_body(wm, "fn fil_bureau()")
req(fil_body, "fil_bureau introuvable")
req("BOUCHAUD_STAGE2_DESKTOP_TASK_ENTER" in fil_body,
    "preuve d'entree dans le fil desktop absente")
req("boucle();" in fil_body,
    "fil_bureau ne lance pas la boucle du WM")
req("task::exit_current(0)" in fil_body,
    "fil_bureau ne termine pas via task::exit_current")

for token in (
    "BOUCHAUD_STAGE2_WINDOW_MANAGER_READY",
    "BOUCHAUD_STAGE2_INPUT_READY",
    "BOUCHAUD_STAGE2_CLICK_DISPATCHED",
    "BOUCHAUD_STAGE2_LADYBIRD_LAUNCHED",
    "handle_click(mx, my",
    "mouse::init();",
    "Client::lance(",
):
    req(token in wm, f"window manager interactif incomplet: {token}")

req("Ladybird differe" not in wm,
    "Ladybird est encore bloque dans reference-desktop")
req("handle_click(mx, my" in wm_code,
    "appel reel a handle_click absent")

req('"uefi-boot,reference-bringup,reference-desktop"' in build,
    "feature set du builder Stage 2 incorrect")
req("bouchaud-reference-stage2.img" in build,
    "image Stage 2 dediee absente")
req("verifie-reference-uefi-elf.py" in build,
    "verification ET_DYN absente")

# --- Contrat de cloture Stage 2 : viewport natif -----------------------------
for token in (
    "pub fn width() -> usize",
    "pub fn height() -> usize",
    "set_canvas_size(native_width, native_height)",
    "present_firmware_rect",
    "GOP firmware natif actif",
):
    req(token in gfx, f"viewport natif incomplet: {token}")

req("BOUCHAUD_STAGE2_NATIVE_VIEWPORT_OK" in stage2,
    "marqueur viewport natif absent")
req("BOUCHAUD_STAGE2_LADYBIRD_RUNTIME_OK" in stage2,
    "marqueur runtime Ladybird absent")
for token in (
    'set_exported_for_boot("BOUCHAUD_M9", "1")',
    'set_exported_for_boot("BOUCHAUD_M9_URL", "https://example.com/")',
    'set_exported_for_boot("BOUCHAUD_M11", "1")',
    'set_exported_for_boot("BOUCHAUD_BROWSER_HOST", "1")',
):
    req(token in stage2, f"environnement Ladybird incomplet: {token}")
req("pub fn set_exported_for_boot" in shell,
    "API boot -> environnement shell absente")
req("viewport GOP non natif" in stage2,
    "invariant physical==logical absent")

for rel, src in (
    ("src/gui/window_manager.rs", wm),
    ("src/gui/window.rs", window),
    ("src/gui/widgets.rs", widgets),
    ("src/gui/widgets_v15.rs", widgets_v15),
    ("src/gui/font.rs", gui_font),
    ("src/gui/stage2_preview.rs", stage2_preview),
):
    req("fb::WIDTH" not in src and "fb::HEIGHT" not in src,
        f"{rel}: depend encore des dimensions compile-time")

req("use crate::gui::framebuffer::{HEIGHT, WIDTH};" not in window,
    "window.rs depend encore de WIDTH/HEIGHT compile-time")
req("const fn rect_maximise" not in window and "const fn zone_maximale" not in window,
    "geometrie maximisee encore compile-time")

req("crate::drivers::gfx::{WIDTH, HEIGHT}" not in mouse_facade,
    "souris encore bornee par constantes compile-time")
req("crate::drivers::gfx::width()" in mouse_ps2
    and "crate::drivers::gfx::height()" in mouse_ps2,
    "centrage souris runtime absent")
req("crate::drivers::gfx::width()" in mouse_packet
    and "crate::drivers::gfx::height()" in mouse_packet,
    "bornes souris runtime absentes")

# Le chemin GOP ne doit plus projeter le canvas 800x600 vers le scanout.
gfx_code = strip_rust_noncode(gfx)
req("dy.saturating_mul(height()) / fb.height" not in gfx_code,
    "projection verticale legacy encore presente")
req("dx.saturating_mul(width()) / fb.width" not in gfx_code,
    "projection horizontale legacy encore presente")

# `present_rect` GOP doit alimenter les compteurs du dernier maillon.
req("PRESENTS_COPIES.fetch_add" in gfx
    and "PIXELS_COPIES_LFB.fetch_add" in gfx
    and "DERNIER_PRESENT_RECT.store" in gfx,
    "accounting GOP damage incomplet")

# --- Contrat de cloture Stage 2 : shell graphique robuste --------------------
req("BOUCHAUD_STAGE2_TREE_LOCK_SAFE" in commands,
    "correctif tree/VFS absent")
tree_body = function_body(commands, "pub fn tree(")
tree_rec_body = function_body(commands, "fn tree_rec(")
req(tree_body and tree_rec_body, "tree/tree_rec introuvable")
req("let idx = {" in tree_body and "tree_rec(idx, 0);" in tree_body,
    "tree ne relache pas explicitement le VFS avant recursion")
req("Vec<(usize, bool, String)>" in tree_rec_body,
    "tree_rec ne snapshotte pas les enfants avant recursion")
req("tree_rec(child, depth + 1);" in tree_rec_body,
    "recursion tree absente")


# --- Contrat Stage 2 FINAL V2 : image et runner Ladybird ---------------------
for token in (
    "BouchaudBrowserHost",
    "WebContent",
    "RequestServer",
    "ImageDecoder",
    "Compositor",
    "WebWorker",
    "WebDriver",
    "V16_UI_CAPABLE",
    "V19_UI_CAPABLE",
):
    req(token in ladybird_prepare, f"preparation Ladybird incomplete: {token}")

for token in (
    "tarfile.USTAR_FORMAT",
    "128 * 1024 * 1024",
    "usr/libexec/ladybird/BouchaudBrowserHost",
):
    req(token in ladybird_make, f"image builder Ladybird incomplet: {token}")

for token in (
    "BOUCHAUD_REFERENCE_LADYBIRD_IMAGE_OK",
    "ca-certificates.crt",
    "fontconfig/fonts.conf",
):
    req(token in ladybird_verify, f"validation image Ladybird incomplete: {token}")

for token in (
    '-machine", "pc"',
    "if=ide,index=0",
    "if=ide,index=1",
    "e1000,netdev=net0",
    "12288",
):
    req(token in ladybird_run, f"runner Ladybird incomplet: {token}")

print("BOUCHAUD_STAGE2_GUARD_FINAL_V2_OK")
