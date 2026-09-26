#!/usr/bin/env python3
"""Verdict structure du smoke Ladybird Bouchaud OS.

Le shell de smoke garde ses diagnostics fins. Ce lecteur produit en plus un
verdict unique et machine-readable reliant startup/IPC, WebWorker, codecs,
JavaScript et surface presentee. Il ne transforme jamais un manque de preuve
en succes.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

REQUIRED = {
    "startup_host": "[ladybird-bouchaud] BROWSER_HOST_START",
    "startup_initialized": "[ladybird-bouchaud] BROWSER_HOST_INITIALIZED",
    "ipc_gui": "[ladybird-bouchaud] M11_GUI_HANDSHAKE_OK",
    "document": "[ladybird-bouchaud] M11_DOCUMENT_LOADED",
    "frame": "[ladybird-bouchaud] BROWSER_HOST_M11_FRAME_PRESENTED",
    "codecs": "HOST_IMAGES_OK codecs=11/11 fond=1 echelle=1 reutilise=1",
    "javascript": "HOST_JS_OK",
    "worker_http": "HOST_WORKER_HTTP_FUNCTIONAL OK",
    "worker_blob": "HOST_WORKER_BLOB_FUNCTIONAL OK",
    "worker_global": "HOST_WORKER_FUNCTIONAL_GLOBAL OK pong",
    "smoke": "HOST_SMOKE_OK canvas=1 worker=1 image=1 frame=1",
}
FATAL = re.compile(
    r"\*\*\* KERNEL PANIC \*\*\*|DOUBLE FAULT|TRIPLE FAULT|"
    r"SpinLock recursive acquisition|BKL(?:-FR)?.*VIOLATION|"
    r"VERIFICATION FAILED:|IMAGE_DECODER_ABSENT|instruction illegale dans le programme utilisateur",
    re.IGNORECASE,
)


def analyse(text: str, surface: str) -> dict:
    checks = {name: marker in text for name, marker in REQUIRED.items()}
    fatals = [line for line in text.splitlines() if FATAL.search(line)]
    lines = text.splitlines()
    worker_steps = [line.strip() for line in lines if "WORKER_ETAPE" in line]
    perf_lines = [
        line.strip() for line in lines
        if "HOST_WORKER_" in line and "_PERF" in line
    ]
    cache_lines = [
        line.strip() for line in lines
        if any(tag in line for tag in (
            "CACHE_BALAYAGE", "CLEAN_PAGE_CACHE_GLOBAL",
            "FAULT_FILE_BREAKDOWN", "FAULT_FILE_SNAPSHOT"
        ))
    ]
    ok = all(checks.values()) and surface == "ok" and not fatals
    return {
        "schema": 1,
        "ok": ok,
        "surface": surface,
        "checks": checks,
        "fatal_count": len(fatals),
        "fatal_tail": fatals[-8:],
        "worker_etape_count": len(worker_steps),
        "worker_etape_tail": worker_steps[-12:],
        "worker_perf_tail": perf_lines[-12:],
        "cache_fault_tail": cache_lines[-20:],
    }


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("log", type=Path)
    p.add_argument("--surface", required=True)
    p.add_argument("--json-out", type=Path)
    args = p.parse_args(argv)
    text = args.log.read_text(encoding="utf-8", errors="replace")
    report = analyse(text, args.surface)
    if args.json_out:
        args.json_out.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if report["ok"]:
        print(
            "LADYBIRD_CONVERGENCE_OK "
            "startup=1 ipc=1 worker_http=1 worker_blob=1 codecs=11/11 js=1 surface=1"
        )
        return 0
    missing = [k for k, ok in report["checks"].items() if not ok]
    print(
        "LADYBIRD_CONVERGENCE_FAIL "
        f"missing={','.join(missing) or '-'} surface={args.surface} fatals={report['fatal_count']}",
        file=sys.stderr,
    )
    for line in report["worker_etape_tail"]:
        print(f"  {line}", file=sys.stderr)
    for line in report["fatal_tail"]:
        print(f"  FATAL {line}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
