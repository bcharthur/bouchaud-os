#!/usr/bin/env python3
"""Build a minimal Bouchaud userland disk for native SDK programs.

    make_native_image.py --image OUT.img --bin NAME=PATH [--bin ...] \
                         --autorun "line" [--autorun "line" ...]

Same layout as make_native_ipc_probe_image.py (USTAR at start, 128 MiB
persistence zone at the end, sparse), but generic: any set of /bin programs
and any autorun script. Deterministic: mtime/uid/gid are zero, entries are
written in the order given.
"""
from __future__ import annotations

import argparse
import io
import tarfile
from pathlib import Path

SECTOR = 512
ZONE_BYTES = 262_144 * SECTOR
ARCHIVE_FLOOR = (1025 + 1) * SECTOR


def add_bytes(tar: tarfile.TarFile, name: str, data: bytes, mode: int) -> None:
    info = tarfile.TarInfo(name)
    info.size = len(data)
    info.mode = mode
    info.mtime = 0
    info.uid = info.gid = 0
    info.uname = info.gname = ""
    tar.addfile(info, io.BytesIO(data))


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--image", required=True, type=Path)
    p.add_argument("--bin", action="append", default=[], metavar="NAME=PATH")
    p.add_argument("--autorun", action="append", default=[])
    args = p.parse_args()
    if not args.autorun:
        raise SystemExit("au moins une ligne --autorun est requise")

    args.image.parent.mkdir(parents=True, exist_ok=True)
    with tarfile.open(args.image, "w", format=tarfile.USTAR_FORMAT) as tar:
        for spec in args.bin:
            name, sep, path = spec.partition("=")
            if not sep or not name or "/" in name:
                raise SystemExit(f"--bin invalide: {spec}")
            source = Path(path)
            if not source.is_file():
                raise SystemExit(f"binaire absent: {source}")
            add_bytes(tar, f"bin/{name}", source.read_bytes(), 0o755)
        add_bytes(tar, "autorun", ("\n".join(args.autorun) + "\n").encode("ascii"), 0o644)

    size = args.image.stat().st_size
    padded = max(ARCHIVE_FLOOR, ((size + SECTOR - 1) // SECTOR) * SECTOR)
    with args.image.open("r+b") as f:
        f.truncate(padded + ZONE_BYTES)
    print(f"NATIVE_IMAGE_OK image={args.image} archive={padded}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
