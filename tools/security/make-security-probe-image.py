#!/usr/bin/env python3
from __future__ import annotations

import argparse
import io
import tarfile
from pathlib import Path

SECTOR = 512
ZONE_BYTES = 262_144 * SECTOR
ARCHIVE_FLOOR = (1025 + 1) * SECTOR

def add(tar, name, data, mode):
    info = tarfile.TarInfo(name)
    info.size = len(data)
    info.mode = mode
    info.mtime = 0
    info.uid = 0
    info.gid = 0
    info.uname = ""
    info.gname = ""
    tar.addfile(info, io.BytesIO(data))

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--probe", required=True, type=Path)
    parser.add_argument("--image", required=True, type=Path)
    args = parser.parse_args()

    if not args.probe.is_file():
        raise SystemExit(f"probe absent: {args.probe}")

    args.image.parent.mkdir(parents=True, exist_ok=True)
    autorun = (
        "echo SECURITY_AUTORUN_BEGIN\n"
        "/bin/security-ring3-probe\n"
        "echo SECURITY_AUTORUN_END\n"
    ).encode("ascii")

    with tarfile.open(args.image, "w", format=tarfile.USTAR_FORMAT) as tar:
        add(tar, "bin/security-ring3-probe", args.probe.read_bytes(), 0o755)
        add(tar, "autorun", autorun, 0o644)

    archive = args.image.stat().st_size
    padded = max(ARCHIVE_FLOOR, ((archive + SECTOR - 1) // SECTOR) * SECTOR)
    total = padded + ZONE_BYTES
    with args.image.open("r+b") as file:
        file.truncate(total)

    print(f"SECURITY_IMAGE_OK image={args.image}")
    print(f"archive={padded} bytes total={total} bytes")

if __name__ == "__main__":
    main()
