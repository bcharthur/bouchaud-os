#!/usr/bin/env python3
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]

def fail(msg):
    print(f"[ECHEC] {msg}", file=sys.stderr)
    raise SystemExit(1)

def text(rel):
    p = ROOT / rel
    if not p.exists():
        fail(f"fichier absent: {rel}")
    return p.read_text(encoding="utf-8-sig")

expected = {
    "src/kernel/debug/blackbox.rs": [
        "KIND_TERMINAL", "terminal-blackbox-v1", "hid-running-zombie-v3", "fn hid_points_sample(",
        "terminal_resultat", "terminal_sortie",
    ],
    "src/drivers/display/vga_text.rs": ["TERMINAL_TRACE_DEPTH", "terminal_trace_begin", "terminal_sortie(args.clone())"],
    "src/shell/mod.rs": ["run_line_journalisee", 'run_line_journalisee("gui"', 'run_line_journalisee("tty"'],
    "src/gui/apps/mod.rs": ["journal_gui_special"],
    "src/drivers/usb/xhci_active.rs": [
        "SENTINELLE_APRES_NS: u64 = 100_000_000",
        "SENTINELLE_PERIODE_NS: u64 = 50_000_000",
        "SENTINELLE_CONFIRMATION_NS: u64 = 20_000_000",
        "PAUSE_REPLI_OISIF_TICKS: u64 = 25",
        "CMD_STOP_ENDPOINT",
        "BOUCHAUD_HID_SENTINEL_EP0",
        "BOUCHAUD_HID_SENTINEL_ACTIVITY",
        "BOUCHAUD_HID_STALL_CONFIRMED",
        "BOUCHAUD_HID_FAILOVER_EP0",
        "BOUCHAUD_HID_STOP_OK",
        "BOUCHAUD_HID_DEQUEUE_RESET",
        "BOUCHAUD_HID_REARMED",
        "BOUCHAUD_HID_INTERRUPT_BACK",
    ],
    "tools/ladybird/prepare-m11-page-registry.py": [
        "M11_TAB_STAGE 10 CALLBACK_ENTER",
        "M11_TAB_STAGE 20 HOST_REQUEST_BEGIN",
        "M11_TAB_STAGE 30 HOST_REPLY",
        "M11_TAB_STAGE 40 CREATE_PAGE_BEGIN",
        "M11_TAB_STAGE 50 CREATE_PAGE_OK",
        "M11_TAB_STAGE 60 TRAVERSABLE_BEGIN",
        "M11_TAB_STAGE 70 TRAVERSABLE_OK",
        "M11_TAB_STAGE 80 READY",
        "auto const source_page = page_id();",
    ],
    "tools/ladybird/prepare-full-browser-host.py": [
        "M11_HOST_TAB_STAGE 10 REQUEST_RECEIVED",
        "M11_HOST_TAB_STAGE 40 VIEW_REGISTERED",
        "M11_HOST_TAB_STAGE 60 REPLY",
    ],
    "tools/ladybird/verifie-chrome.sh": ["M11_TAB_STAGE 80 READY", "M11_HOST_TAB_STAGE 40 VIEW_REGISTERED"],
    "tools/reference/extract-blackbox.py": ['8:"terminal"', '"terminal.log"', "terminal_records=len(terminal)"],
}

for rel, needles in expected.items():
    data = text(rel)
    for needle in needles:
        if needle not in data:
            fail(f"preuve absente: {needle!r} dans {rel}")
    print(f"[OK] {rel}")

# La sentinelle doit pouvoir voir une interaction avant environ 250 ms.
xhci = text("src/drivers/usb/xhci_active.rs")
if "let repond = verdict.analyse();" not in xhci or "if verdict.entree()" not in xhci:
    fail("la sentinelle EP0 ne distingue pas Analyse et Entree")
if xhci.count("sentinelle_ep0: false,") != 2:
    fail(f"initialisations sentinelle attendues=2, trouvees={xhci.count('sentinelle_ep0: false,')}")
if xhci.count("derniere_reprise_ns: 0,") != 2:
    fail(f"initialisations derniere_reprise attendues=2, trouvees={xhci.count('derniere_reprise_ns: 0,')}")
if "if etait_casse" not in xhci or "ep.interrupt_in_casse = false;" not in xhci:
    fail("le premier Transfer Event ne coupe pas explicitement le fallback")

# Contrat absolu : ce lot ne doit modifier aucun fichier reseau.
allowed = {
    "src/kernel/debug/blackbox.rs",
    "src/drivers/display/vga_text.rs",
    "src/shell/mod.rs",
    "src/gui/apps/mod.rs",
    "src/drivers/usb/xhci_active.rs",
    "tools/reference/extract-blackbox.py",
    "tools/ladybird/prepare-m11-page-registry.py",
    "tools/ladybird/prepare-full-browser-host.py",
    "tools/ladybird/verifie-chrome.sh",
}
try:
    changed = subprocess.check_output(
        ["git", "diff", "--name-only", "--"], cwd=ROOT, text=True
    ).splitlines()
except Exception as exc:
    fail(f"git diff impossible: {exc}")

outside = [p for p in changed if p and p not in allowed]
if outside:
    fail("fichiers hors perimetre modifies: " + ", ".join(outside))

network_words = ("rtl8168", "dhcp", "dns", "tcp", "tls", "src/net/", "drivers/network/")
network = [p for p in changed if any(w.lower() in p.lower() for w in network_words)]
if network:
    fail("regression guard reseau viole: " + ", ".join(network))

print("[OK] aucun fichier RTL8168/DHCP/DNS/TCP/TLS modifie")
print("[OK] contrat HID : sentinelle EP0 + Stop/SetDequeue + retour Interrupt-IN present")
print("[OK] contrat onglets : stages 10..80 + VIEW_REGISTERED present")
print("STABILITY_OVERLAY_VERIFY_OK")
