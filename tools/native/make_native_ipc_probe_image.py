#!/usr/bin/env python3
"""Build the smallest Bouchaud userland disk that autoruns the native IPC probe.

Works on Windows, Linux and macOS.  The image uses the same USTAR-at-start plus
128 MiB persistence-at-end layout as tools/userland/mkdisk.sh, but uses sparse
extension instead of physically writing 128 MiB of zeroes.
"""

from __future__ import annotations

import argparse
import io
import os
import tarfile
from pathlib import Path

SECTOR = 512
ZONE_SECTORS = 262_144
ZONE_BYTES = ZONE_SECTORS * SECTOR
SECTEUR_CONTENU = 1025
ARCHIVE_FLOOR = (SECTEUR_CONTENU + 1) * SECTOR

def add_bytes(tar: tarfile.TarFile, name: str, data: bytes, mode: int) -> None:
    info = tarfile.TarInfo(name)
    info.size = len(data)
    info.mode = mode
    info.mtime = 0
    info.uid = 0
    info.gid = 0
    info.uname = ""
    info.gname = ""
    tar.addfile(info, io.BytesIO(data))

def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--ring3-probe", required=True, type=Path)
    p.add_argument("--libc-probe", type=Path)
    p.add_argument("--image", required=True, type=Path)
    args = p.parse_args()

    if not args.ring3_probe.is_file():
        raise SystemExit(f"probe ring3 absent: {args.ring3_probe}")
    if args.libc_probe is not None and not args.libc_probe.is_file():
        raise SystemExit(f"probe libc absent: {args.libc_probe}")

    args.image.parent.mkdir(parents=True, exist_ok=True)
    autorun = [
        "echo NATIVE_IPC_AUTORUN_BEGIN",
        "/bin/native-ipc-ring3-probe",
    ]
    if args.libc_probe is not None:
        autorun.append("/bin/native-ipc-probe")
    autorun.append("echo NATIVE_IPC_AUTORUN_END")
    autorun_data = ("\n".join(autorun) + "\n").encode("ascii")

    with tarfile.open(args.image, "w", format=tarfile.USTAR_FORMAT) as tar:
        add_bytes(
            tar,
            "bin/native-ipc-ring3-probe",
            args.ring3_probe.read_bytes(),
            0o755,
        )
        if args.libc_probe is not None:
            add_bytes(
                tar,
                "bin/native-ipc-probe",
                args.libc_probe.read_bytes(),
                0o755,
            )
        add_bytes(tar, "autorun", autorun_data, 0o644)

    archive_size = args.image.stat().st_size
    padded_archive = max(
        ARCHIVE_FLOOR,
        ((archive_size + SECTOR - 1) // SECTOR) * SECTOR,
    )
    total = padded_archive + ZONE_BYTES

    # Sparse extension is enough: Bouchaud only needs the disk geometry and
    # reads zeroes from the unused persistence zone.
    with args.image.open("r+b") as f:
        f.truncate(total)

    print(f"NATIVE_IPC_IMAGE_OK image={args.image}")
    print(f"archive={padded_archive} bytes total={total} bytes")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
