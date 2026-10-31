#!/usr/bin/env python3
"""Garde-fou source de la convergence Ladybird P13.

Il ne declare aucune capacite runtime sans execution. Il garantit seulement
que les preuves qui existaient deja restent cablees ensemble : P4/P18,
startup/IPC, cache, GPU, WebWorker, codecs/JS et surface du smoke.
"""
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[2]
errors = []


def read(rel):
    p = ROOT / rel
    try:
        return p.read_text(encoding="utf-8")
    except OSError as exc:
        errors.append(f"{rel}: {exc}")
        return ""


def need(rel, *needles):
    text = read(rel)
    for needle in needles:
        if needle not in text:
            errors.append(f"{rel}: marqueur absent: {needle}")
    return text

# Fast/Reliability : la table du faux serveur doit connaitre tout le BRDP courant.
remote_test = need(
    "tools/remote/test_bouchaud_lab.py",
    '"services-page": ("services page", 92)',
    '"serial-status": ("serial status", 90)',
    '"internet-start": ("internet proof start", 19)',
    '"system-reboot": ("system reboot", 100)',
    '"browser-restart": ("browser restart", 112)',
    '"process-kill-tree": ("process kill-tree", 121)',
)

# WebWorker + codecs/JS + surface + startup/IPC : le smoke doit les exiger SEPAREMENT.
smoke = need(
    "tools/ci/run_ladybird_browser_host.sh",
    "[ladybird-bouchaud] BROWSER_HOST_START",
    "[ladybird-bouchaud] BROWSER_HOST_INITIALIZED",
    "[ladybird-bouchaud] M11_GUI_HANDSHAKE_OK",
    "HOST_IMAGES_OK codecs=11/11 fond=1 echelle=1 reutilise=1",
    "HOST_JS_OK",
    "HOST_WORKER_HTTP_FUNCTIONAL OK",
    "HOST_WORKER_BLOB_FUNCTIONAL OK",
    "HOST_WORKER_FUNCTIONAL_GLOBAL OK pong",
    "ladybird_runtime_verdict.py",
)
need("tools/ci/ladybird_runtime_verdict.py", "LADYBIRD_CONVERGENCE_OK", "WORKER_ETAPE")

# Le vrai artefact doit etre lie au HEAD + upstream et re-verifie apres download.
workflow = need(
    ".github/workflows/ladybird-native-browser.yml",
    "branches: [main, 'claude/**', 'feat/**']",
    "artifact_manifest.py create",
    "artifact_manifest.py verify",
    "BOUCHAUD_ARTIFACT_MANIFEST.json",
)
need("tools/ladybird/artifact_manifest.py", "PT_INTERP", "sha256_tree", "bouchaud_head")

# P4 : sampler hors xHCI + recyclage des instances historiques.
xhci = need("src/drivers/usb/xhci_active.rs", "BOUCHAUD_V13_SERVICES_SAMPLER_DEDIE")
for forbidden in ("releve_processus(", "publie_processus(", "snapshot_processus("):
    if forbidden in xhci:
        errors.append(f"src/drivers/usb/xhci_active.rs: sampler processus revenu dans xHCI: {forbidden}")
need(
    "src/kernel/services/registre.rs",
    "BOUCHAUD_V13_RECYCLAGE_PID_HELPER",
    "BOUCHAUD_V13_RECYCLAGE_PID_SATURATION",
    '"sys.memory.page_cache"',
    '"sys.graphics.gpu"',
)
need("tools/services/test_registre.rs", "les_pid_navigateur_morts_se_recyclent_quand_le_registre_est_plein")

# P18 : proc dynamique au moment de open(), pas fichiers RAMFS figes au boot.
need(
    "src/kernel/object/fd.rs",
    'path == "/proc/stat"',
    'path == "/proc/self/stat"',
    "proc_pid_stat_depuis_chemin",
    "proc_cpu_cumul()",
    "proc_processus_cumul(pid)",
)

# Cache : le temoin utilise depuis exit_current doit rester lock-free.
cache = need("src/kernel/memory/page_cache.rs", "pub fn balayage_temoins()", "BALAYAGE_EVITES")
m = re.search(r"pub fn balayage_temoins\(\).*?\n\}", cache, flags=re.S)
if not m:
    errors.append("src/kernel/memory/page_cache.rs: corps balayage_temoins introuvable")
elif "CACHE.lock()" in m.group(0):
    errors.append("src/kernel/memory/page_cache.rs: balayage_temoins reprend CACHE.lock()")

# Startup/IPC natif : bornes explicites et preuve ring3 executee.
need("src/kernel/native/ipc/channel.rs", "MAX_QUEUED_MESSAGES", "MAX_QUEUED_BYTES", "Error::QueueFull")
need(
    "tools/ci/run_native_ipc_probe.sh",
    "[NATIVE-IPC-RING3] OK",
    "[NATIVE-IPC] OK",
    "COMPOSITED_SLICE_OK",
)

# Rendu/GPU : contrat explicite, mais aucune fausse promesse 3D.
need(
    "src/drivers/api/gpu.rs",
    "pub struct GpuStats",
    "pub fn note_present",
    "CPU raster + linear scanout",
)
need(
    "src/kernel/services/mod.rs",
    '"sys.memory.page_cache"',
    '"sys.graphics.gpu"',
    "crate::kernel::clean_page_cache::stats()",
    "crate::drivers::gpu::stats()",
)

if errors:
    print("LADYBIRD_CONVERGENCE_SOURCE_FAIL", file=sys.stderr)
    for e in errors:
        print("  " + e, file=sys.stderr)
    raise SystemExit(1)
print("LADYBIRD_CONVERGENCE_SOURCE_OK P4=1 P18=1 IPC=1 CACHE=1 GPU=1 SMOKE=1 MANIFEST=1")
