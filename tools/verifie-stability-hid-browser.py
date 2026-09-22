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

# LE RESEAU PHYSIQUE NE SE TOUCHE PAS DANS UN LOT QUI NE LE VISE PAS.
#
# La chaine RTL8168 -> DHCP -> DNS -> TCP -> TLS est la seule partie du port
# qui fonctionne bout en bout sur la machine physique. Un lot HID ou navigateur
# n'a aucune raison d'y toucher, et une modification fortuite s'y verrait au
# pire moment : dans une archive de boite noire, une semaine plus tard.
#
# LA LISTE BLANCHE DE FICHIERS AUTORISES A ETE RETIREE, ET CE N'EST PAS UN
# RELACHEMENT.
#
# Elle enumerait les neuf fichiers du lot du 19 septembre et refusait tout le
# reste. C'etait juste le jour de la livraison et faux le lendemain : la garde
# vit dans `tools/verifie-*.py`, donc elle est DECOUVERTE et rejouee a chaque
# changement du depot. Toute modification ulterieure de n'importe quel autre
# fichier -- y compris la correction des deux gardes HID que ce meme lot avait
# rendues fausses -- echouait avec « fichiers hors perimetre modifies ». Une
# garde permanente ne peut pas encoder le perimetre d'un commit ; elle encode
# un invariant. L'invariant, ici, c'est le reseau.
#
# La comparaison porte sur `git diff HEAD` : l'index ET l'arbre de travail. La
# version precedente lisait `git diff` seul, donc un fichier reseau deja ajoute
# a l'index lui echappait entierement.
try:
    changed = subprocess.check_output(
        ["git", "diff", "HEAD", "--name-only", "--"], cwd=ROOT, text=True
    ).splitlines()
except Exception as exc:
    fail(f"git diff impossible: {exc}")

network_words = ("rtl8168", "dhcp", "dns", "tcp", "tls", "src/net/", "drivers/network/")
network = [p for p in changed if any(w.lower() in p.lower() for w in network_words)]
if network:
    fail("regression guard reseau viole: " + ", ".join(network))

print("[OK] aucun fichier RTL8168/DHCP/DNS/TCP/TLS modifie")
print("[OK] contrat HID : sentinelle EP0 + Stop/SetDequeue + retour Interrupt-IN present")
print("[OK] contrat onglets : stages 10..80 + VIEW_REGISTERED present")
print("STABILITY_OVERLAY_VERIFY_OK")
