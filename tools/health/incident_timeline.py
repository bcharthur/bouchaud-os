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


# Per-boot grouping is deliberate: two QEMU boots on the same A/B disk
# MUST NOT be collapsed into a single inferred causal chain.
BOOT_LOG_NAME = re.compile(r"boot-([0-9]+)\.log\Z")
MAX_BOOT_LOGS = 64
CRITICAL_KINDS = frozenset({
    "scheduler_panic", "scheduler_handoff_fault", "usb_io_failure",
    "usb_bot_recovery_failed", "update_state_error", "health_commit_refused",
})


def decode_boot_directory(folder: Path, max_events: int = 20000) -> dict:
    """Aggregate each boot-N.log independently, never concatenate raw input."""
    if not folder.is_dir():
        raise ValueError("not a boot log directory")
    matched = []
    for candidate in folder.iterdir():
        name = BOOT_LOG_NAME.fullmatch(candidate.name)
        if name:
            if candidate.is_symlink() or not candidate.is_file():
                raise ValueError("boot log must be a regular file")
            matched.append((int(name.group(1)), candidate))
    if not matched or len(matched) > MAX_BOOT_LOGS:
        raise ValueError("expected 1..64 regular boot-N.log files")
    matched.sort(key=lambda entry: (entry[0], entry[1].name))
    if len({number for number, _ in matched}) != len(matched):
        raise ValueError("ambiguous boot sequence numbering")
    aggregate: Counter[str] = Counter()
    boots = []
    for number, path in matched:
        with path.open("r", encoding="utf-8", errors="replace") as stream:
            report = decode_lines(stream, max_events=max_events)
        aggregate.update(report["event_counts"])
        critical = [event for event in report["events"]
                    if event["kind"] in CRITICAL_KINDS]
        boots.append({
            "boot_number": number,
            "source": path.name,
            "boot_id": report["boot_id_last_seen"],
            "lines_scanned": report["lines_scanned"],
            "events_kept": report["events_kept"],
            "events_truncated": report["events_truncated"],
            "logd_declared_losses": report["logd_declared_losses"],
            "event_counts": report["event_counts"],
            # Event counts include records omitted from the bounded JSON.
            # Never report 'no critical event' merely because its line
            # fell beyond max_events (the summary remains fail-closed).
            "critical_observed": any(report["event_counts"].get(kind, 0)
                                     for kind in CRITICAL_KINDS),
            "first_critical_line": critical[0]["line"] if critical else None,
            "events": report["events"],
        })
    return {
        "schema": "bouchaud-ab-boot-timeline/v1",
        "boot_count": len(boots),
        "boot_numbers": [b["boot_number"] for b in boots],
        "event_counts": dict(sorted(aggregate.items())),
        "any_critical_observed": any(b["critical_observed"] for b in boots),
        "evidence_truncated": any(b["events_truncated"] for b in boots),
        "boots": boots,
        "causality": "not-inferred: per-boot chronology only; no cross-boot causal edge",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--max-events", type=int, default=20000)
    args = parser.parse_args()
    if not (1 <= args.max_events <= 100000):
        parser.error("--max-events must be between 1 and 100000")
    if args.log.is_dir():
        report = decode_boot_directory(args.log, args.max_events)
        summary = (f"BOUCHAUD_AB_TIMELINE boots={report['boot_count']} "
                   f"critical={int(report['any_critical_observed'])} "
                   f"truncated={int(report['evidence_truncated'])}")
    else:
        with args.log.open("r", encoding="utf-8", errors="replace") as file:
            report = decode_lines(file, args.max_events)
        summary = (f"BOUCHAUD_TIMELINE events={report['events_kept']} "
                   f"truncated={int(report['events_truncated'])} "
                   f"logd_dropped={report['logd_declared_losses']}")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n",
                           encoding="utf-8")
    print(summary)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
