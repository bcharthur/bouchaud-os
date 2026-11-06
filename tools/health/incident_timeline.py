#!/usr/bin/env python3
"""Bounded, privacy-minimizing causal-timeline *input* from Bouchaud serial logs.

This tool reports sequence/co-occurrence only, never invents a causality edge.
It recognizes kernel and userland health markers and deliberately does not
copy arbitrary command lines, tokens, cookie values, or unrecognized output.
"""
from __future__ import annotations

import argparse
from collections import Counter
import json
from pathlib import Path
import re

MAX_LINE = 4096
BOOT = re.compile(r"BOUCHAUD_BOOT_RECORD\s+boot_id=([0-9a-fx]+)", re.I)
KV = re.compile(r"(?<!\S)([A-Za-z_][A-Za-z_0-9]*)=([^\s]+)")
PATTERNS = (
    ("scheduler_panic", "*** KERNEL PANIC ***", "kernel"),
    ("scheduler_handoff_fault", "passation pending sans switching_out", "scheduler"),
    ("usb_io_failure", "BOUCHAUD_USB_STOCKAGE_IO_ECHEC", "usb"),
    ("usb_stale_completion", "BOUCHAUD_USB_BULK_TRB_PERIME", "usb"),
    ("usb_bot_recovery", "BOUCHAUD_USB_BOT_REPRISE_OK", "usb"),
    ("usb_bot_recovery_failed", "BOUCHAUD_USB_BOT_REPRISE_ECHEC", "usb"),
    ("update_state", "SERVICED_UPDATE_STATE", "updated"),
    ("health_commit", "SERVICED_HEALTH_COMMIT", "serviced"),
    ("ab_boot", "BOUCHAUD_AB_BOOT", "loader"),
    ("service_event", "LOGD ", "logd"),
)
SAFE_FIELDS = frozenset({
    "slot", "raison", "result", "rc", "cpu", "tid", "pid", "svc",
    "type", "sev", "dropped", "boot", "ts_ns", "dci", "index", "etat",
    "active_slot", "state_b", "state_a", "cc", "status",
})


def decode_lines(lines, max_events: int = 20000) -> dict:
    boot_id = "unknown"
    events = []
    counts: Counter[str] = Counter()
    dropped_events = 0
    lines_seen = 0
    truncated = False
    for n, raw in enumerate(lines, start=1):
        lines_seen = n
        text = raw[:MAX_LINE]
        match = BOOT.search(text)
        if match:
            boot_id = match.group(1)
        found = next(((kind, owner) for kind, marker, owner in PATTERNS
                      if marker in text), None)
        if found is None:
            continue
        kind, owner = found
        pairs = dict(KV.findall(text))
        clean = {k: v[:64] for k, v in pairs.items() if k in SAFE_FIELDS}
        if kind == "service_event":
            owner = clean.get("svc", owner)
            try:
                dropped_events += int(clean.get("dropped", "0"))
            except ValueError:
                pass
        if kind == "update_state" and clean.get("rc") not in ("0", None):
            kind = "update_state_error"
        if kind == "health_commit" and clean.get("result") == "refused":
            kind = "health_commit_refused"
        if len(events) >= max_events:
            truncated = True
            counts[kind] += 1
            continue
        events.append({
            "seq": len(events) + 1,
            "line": n,
            "boot_id": boot_id,
            "kind": kind,
            "service": owner,
            "fields": clean,
        })
        counts[kind] += 1
    return {
        "schema": "bouchaud-incident-timeline/v1",
        "boot_id_last_seen": boot_id,
        "lines_scanned": lines_seen,
        "events_kept": len(events),
        "events_truncated": truncated,
        "logd_declared_losses": dropped_events,
        "event_counts": dict(sorted(counts.items())),
        "events": events,
        "causality": "not-inferred: order and co-occurrence only without explicit trace_id",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--max-events", type=int, default=20000)
    args = parser.parse_args()
    if not (1 <= args.max_events <= 100000):
        parser.error("--max-events must be between 1 and 100000")
    with args.log.open("r", encoding="utf-8", errors="replace") as file:
        report = decode_lines(file, args.max_events)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n",
                           encoding="utf-8")
    print(f"BOUCHAUD_TIMELINE events={report['events_kept']} "
          f"truncated={int(report['events_truncated'])} "
          f"logd_dropped={report['logd_declared_losses']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
