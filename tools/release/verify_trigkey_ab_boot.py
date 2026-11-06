#!/usr/bin/env python3
"""Fail-closed boot evidence for a real QEMU/OVMF A/B image, never physical."""
from __future__ import annotations
import json
import re
import sys
from pathlib import Path

ANSI = re.compile(r"\\x1b\\[[0-9;]*m")
REQUIRED = (
    "BOUCHAUD_AB_BOOT slot=A",
    "BOUCHAUD_HWPROBE_BEGIN",
    "SERVICED_UPDATE_STATE",
    "result=already_good",
)
FORBIDDEN = (
    "*** KERNEL PANIC ***", "DOUBLE FAULT", "TRIPLE FAULT",
    "BOUCHAUD_AB_RECOVERY_REQUIRED", "BOUCHAUD_AB_BOOT slot=B",
    "SERVICED_UPDATE_STATE rc=-",
)
MAX_LOG_BYTES = 16 * 1024 * 1024


def inspect(log_text: str, manifest: dict) -> dict:
    if manifest.get("proof_level") != "BUILD_ONLY" or manifest.get("physical") != "NOT_RUN":
        raise ValueError("the build manifest must remain BUILD_ONLY / NOT_RUN")
    if not re.fullmatch(r"[0-9a-f]{40}", str(manifest.get("commit", ""))):
        raise ValueError("missing exact build commit")
    clean = ANSI.sub("", log_text).replace("\\r", "")
    missing = [marker for marker in REQUIRED if marker not in clean]
    bad = [marker for marker in FORBIDDEN if marker in clean]
    if missing or bad:
        raise ValueError("QEMU boot incomplete or unsafe: missing="
                         + ",".join(missing) + " forbidden=" + ",".join(bad))
    return {
        "schema": "trigkey-ab-qemu-smoke/1",
        "build_sha": manifest["commit"],
        "qemu_boot": "PASS",
        "transport": "QEMU_OVMF_NVME",
        "active_slot_observed": "A",
        "services_health_observed": True,
        "physical": "NOT_RUN",
        "release_approved": False,
    }


def verify(out: Path) -> dict:
    log = out / "boot-qemu.log"
    if not log.is_file() or log.is_symlink() or log.stat().st_size > MAX_LOG_BYTES:
        raise ValueError("invalid QEMU log file")
    manifest = json.loads((out / "manifest-ab.json").read_text(encoding="utf-8"))
    result = inspect(log.read_text(encoding="utf-8", errors="replace"), manifest)
    (out / "qemu-boot-evidence.json").write_text(
        json.dumps(result, indent=2, ensure_ascii=False) + "\\n", encoding="utf-8")
    return result


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: verify_trigkey_ab_boot.py OUT_DIR")
    try:
        result = verify(Path(sys.argv[1]))
    except (OSError, ValueError, TypeError, json.JSONDecodeError) as exc:
        print(f"TRIGKEY_QEMU_BOOT_FAIL {exc}", file=sys.stderr)
        return 2
    print(f"TRIGKEY_QEMU_BOOT_OK sha={result['build_sha'][:12]} PHYSICAL=NOT_RUN")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
