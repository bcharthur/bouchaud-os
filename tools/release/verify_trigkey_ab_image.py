#!/usr/bin/env python3
"""Vérification statique fail-closed des quatre services A/B et de provenance.

Ne prouve pas qu'une machine physique démarre. Sans édition des META ni flash.
"""
from __future__ import annotations

import hashlib
import json
import sys
import tarfile
from pathlib import Path

SERVICES = ("serviced", "healthd", "updated", "logd")

def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()

def check_tar(path: Path) -> dict[str, str]:
    with tarfile.open(path, "r:") as archive:
        found = set(archive.getnames())
        expected = {f"bin/{name}" for name in SERVICES}
        if found != expected or len(archive.getmembers()) != len(SERVICES):
            raise ValueError(f"{path}: services invalides {sorted(found)}")
        digests: dict[str, str] = {}
        for name in SERVICES:
            member = archive.getmember(f"bin/{name}")
            if not member.isfile() or not member.mode & 0o111 or member.size < 1:
                raise ValueError(f"{path}: {name} absent, non exécutable ou vide")
            source = archive.extractfile(member)
            if source is None:
                raise ValueError(f"{path}: {name} illisible")
            digests[name] = hashlib.sha256(source.read()).hexdigest()
        return digests

def verify(out: Path, commit: str) -> dict:
    if len(commit) != 40 or any(c not in "0123456789abcdef" for c in commit):
        raise ValueError("SHA commit invalide")
    for name in ("disque-ab.img", "kernel.elf", "meta.img"):
        if not (out / name).is_file() or (out / name).stat().st_size == 0:
            raise ValueError(f"{name} absent ou vide")
    a = check_tar(out / "slot-a" / "ramdisk")
    b = check_tar(out / "slot-b" / "ramdisk")
    if a != b:
        raise ValueError("slots A/B : services différents")
    return {
        "schema": "trigkey-ab-build/1",
        "commit": commit,
        "proof_level": "BUILD_ONLY",
        "physical": "NOT_RUN",
        "update_key": "NOT_CONFIGURED",
        "boot_health": "NOT_TESTED",
        "image": {"sha256": sha256(out / "disque-ab.img"),
                  "bytes": (out / "disque-ab.img").stat().st_size},
        "kernel_elf": {"sha256": sha256(out / "kernel.elf")},
        "services_sha256": a,
    }

def main() -> int:
    if len(sys.argv) != 3:
        raise SystemExit("usage: verify_trigkey_ab_image.py OUT_DIR COMMIT_SHA")
    out, commit = Path(sys.argv[1]), sys.argv[2]
    try:
        manifest = verify(out, commit)
    except (OSError, ValueError, tarfile.TarError) as exc:
        print(f"TRIGKEY_AB_PRECHECK_FAIL {exc}", file=sys.stderr)
        return 2
    (out / "manifest-ab.json").write_text(
        json.dumps(manifest, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"TRIGKEY_AB_PRECHECK_OK commit={commit[:12]} services={len(SERVICES)} niveau=BUILD_ONLY")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
