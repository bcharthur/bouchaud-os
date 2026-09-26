#!/usr/bin/env python3
"""Manifest deterministe du runtime Ladybird natif Bouchaud OS.

Le but n'est pas de signer cryptographiquement un artefact, mais d'empecher
qu'un vieux repertoire "native-browser" soit pris pour le build du HEAD
courant. Le manifeste lie :
  * le HEAD Bouchaud qui a produit l'artefact ;
  * le SHA Ladybird epingle dans third_party/UPSTREAM.md ;
  * les SHA-256 des binaires et marqueurs de capacite ;
  * une empreinte deterministe de l'arbre resources/ ;
  * l'absence de PT_INTERP sur les executables Ladybird freestanding.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import struct
import subprocess
import sys
from pathlib import Path

SCHEMA = 1
KIND = "bouchaud-ladybird-native-browser"
ELF_RUNTIME = (
    "BouchaudBrowserHost",
    "WebContent",
    "RequestServer",
    "ImageDecoder",
    "WebWorker",
    "Compositor",
    "WebDriver",
)
REQUIRED_FILES = ELF_RUNTIME + (
    "webcontent-bootstrap",
    "M9_CAPABLE",
    "V16_UI_CAPABLE",
    "V19_UI_CAPABLE",
)
MANIFEST_NAME = "BOUCHAUD_ARTIFACT_MANIFEST.json"


class ManifestError(RuntimeError):
    pass


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def upstream_sha(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    m = re.search(r"(?m)^\s*sha\s+([0-9a-fA-F]{40})\s*$", text)
    if not m:
        raise ManifestError(f"SHA upstream Ladybird introuvable dans {path}")
    return m.group(1).lower()


def git_head(cwd: Path) -> str | None:
    try:
        out = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=cwd, stderr=subprocess.DEVNULL, text=True
        ).strip()
    except (OSError, subprocess.CalledProcessError):
        return None
    return out if re.fullmatch(r"[0-9a-f]{40}", out) else None


def elf_has_interp(path: Path) -> bool:
    data = path.read_bytes()
    if len(data) < 64 or data[:4] != b"\x7fELF":
        raise ManifestError(f"{path}: pas un ELF")
    elf_class = data[4]
    endian = data[5]
    if endian != 1:
        raise ManifestError(f"{path}: ELF big-endian non supporte")
    if elf_class == 2:  # ELF64
        if len(data) < 64:
            raise ManifestError(f"{path}: ELF64 tronque")
        phoff = struct.unpack_from("<Q", data, 32)[0]
        phentsize = struct.unpack_from("<H", data, 54)[0]
        phnum = struct.unpack_from("<H", data, 56)[0]
    elif elf_class == 1:  # ELF32
        if len(data) < 52:
            raise ManifestError(f"{path}: ELF32 tronque")
        phoff = struct.unpack_from("<I", data, 28)[0]
        phentsize = struct.unpack_from("<H", data, 42)[0]
        phnum = struct.unpack_from("<H", data, 44)[0]
    else:
        raise ManifestError(f"{path}: classe ELF inconnue {elf_class}")
    if phentsize < 4:
        raise ManifestError(f"{path}: table de programmes invalide")
    end = phoff + phentsize * phnum
    if end > len(data):
        raise ManifestError(f"{path}: table de programmes tronquee")
    for i in range(phnum):
        p_type = struct.unpack_from("<I", data, phoff + i * phentsize)[0]
        if p_type == 3:  # PT_INTERP
            return True
    return False


def resources_digest(root: Path) -> dict:
    resources = root / "resources"
    if not resources.is_dir():
        raise ManifestError(f"{resources}: repertoire absent")
    tree = hashlib.sha256()
    count = 0
    total = 0
    for path in sorted(resources.rglob("*"), key=lambda p: p.as_posix()):
        rel = path.relative_to(resources).as_posix()
        if path.is_symlink():
            payload = ("SYMLINK\0" + os.readlink(path)).encode("utf-8", "surrogateescape")
            digest = hashlib.sha256(payload).hexdigest()
            size = len(payload)
        elif path.is_dir():
            continue
        elif path.is_file():
            digest = sha256_file(path)
            size = path.stat().st_size
        else:
            raise ManifestError(f"resources contient un type non supporte: {path}")
        tree.update(rel.encode("utf-8", "surrogateescape"))
        tree.update(b"\0")
        tree.update(str(size).encode("ascii"))
        tree.update(b"\0")
        tree.update(digest.encode("ascii"))
        tree.update(b"\n")
        count += 1
        total += size
    if count == 0:
        raise ManifestError("resources/ est vide")
    return {"files": count, "bytes": total, "sha256_tree": tree.hexdigest()}


def build_manifest(root: Path, upstream: Path) -> dict:
    root = root.resolve()
    for name in REQUIRED_FILES:
        p = root / name
        if not p.is_file():
            raise ManifestError(f"artefact incomplet: {name} absent")
    files = []
    for name in REQUIRED_FILES:
        p = root / name
        entry = {"path": name, "bytes": p.stat().st_size, "sha256": sha256_file(p)}
        if name in ELF_RUNTIME:
            has_interp = elf_has_interp(p)
            if has_interp:
                raise ManifestError(f"{name}: PT_INTERP present, runtime non freestanding")
            entry["elf_static"] = True
        files.append(entry)
    repo = Path.cwd()
    head = git_head(repo)
    if head is None:
        raise ManifestError("git rev-parse HEAD impossible lors de la creation")
    return {
        "schema": SCHEMA,
        "kind": KIND,
        "producer": {"bouchaud_head": head},
        "ladybird_upstream": {"sha": upstream_sha(upstream)},
        "files": files,
        "resources": resources_digest(root),
    }


def verify_manifest(root: Path, upstream: Path, manifest_path: Path, check_head: bool = True) -> dict:
    root = root.resolve()
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ManifestError(f"manifeste illisible: {exc}") from exc
    if manifest.get("schema") != SCHEMA or manifest.get("kind") != KIND:
        raise ManifestError("schema/kind du manifeste inattendu")
    expected_upstream = upstream_sha(upstream)
    got_upstream = manifest.get("ladybird_upstream", {}).get("sha")
    if got_upstream != expected_upstream:
        raise ManifestError(
            f"artefact Ladybird perime: upstream manifeste={got_upstream} courant={expected_upstream}"
        )
    if check_head:
        current = git_head(Path.cwd())
        produced = manifest.get("producer", {}).get("bouchaud_head")
        if current and produced != current:
            raise ManifestError(f"artefact d'un autre HEAD: produit={produced} courant={current}")
    listed = manifest.get("files")
    if not isinstance(listed, list):
        raise ManifestError("liste files absente")
    by_path = {e.get("path"): e for e in listed if isinstance(e, dict)}
    if set(by_path) != set(REQUIRED_FILES):
        missing = sorted(set(REQUIRED_FILES) - set(by_path))
        extra = sorted(set(by_path) - set(REQUIRED_FILES))
        raise ManifestError(f"table artefact divergente missing={missing} extra={extra}")
    for name in REQUIRED_FILES:
        p = root / name
        if not p.is_file():
            raise ManifestError(f"{name}: absent a la verification")
        entry = by_path[name]
        got_size = p.stat().st_size
        got_hash = sha256_file(p)
        if entry.get("bytes") != got_size or entry.get("sha256") != got_hash:
            raise ManifestError(f"{name}: taille/SHA-256 divergent")
        if name in ELF_RUNTIME:
            if elf_has_interp(p):
                raise ManifestError(f"{name}: PT_INTERP present")
            if entry.get("elf_static") is not True:
                raise ManifestError(f"{name}: preuve elf_static absente")
    resources = resources_digest(root)
    if manifest.get("resources") != resources:
        raise ManifestError("resources/: empreinte divergente")
    return manifest


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__)
    sub = p.add_subparsers(dest="cmd", required=True)
    for name in ("create", "verify"):
        sp = sub.add_parser(name)
        sp.add_argument("--root", required=True, type=Path)
        sp.add_argument("--upstream", required=True, type=Path)
        sp.add_argument("--output" if name == "create" else "--manifest", required=True, type=Path)
    spv = sub.choices["verify"]
    spv.add_argument("--allow-head-mismatch", action="store_true")
    args = p.parse_args(argv)
    try:
        if args.cmd == "create":
            manifest = build_manifest(args.root, args.upstream)
            args.output.parent.mkdir(parents=True, exist_ok=True)
            tmp = args.output.with_suffix(args.output.suffix + ".tmp")
            tmp.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
            os.replace(tmp, args.output)
            print(
                f"LADYBIRD_ARTIFACT_MANIFEST_CREATED head={manifest['producer']['bouchaud_head']} "
                f"upstream={manifest['ladybird_upstream']['sha']} files={len(manifest['files'])} "
                f"resources={manifest['resources']['files']}"
            )
        else:
            manifest = verify_manifest(
                args.root, args.upstream, args.manifest, check_head=not args.allow_head_mismatch
            )
            print(
                f"LADYBIRD_ARTIFACT_MANIFEST_OK head={manifest['producer']['bouchaud_head']} "
                f"upstream={manifest['ladybird_upstream']['sha']}"
            )
    except ManifestError as exc:
        print(f"LADYBIRD_ARTIFACT_MANIFEST_FAIL {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
