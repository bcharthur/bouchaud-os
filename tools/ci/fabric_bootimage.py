#!/usr/bin/env python3
"""Publish and verify the default BIOS image within one Actions campaign."""

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess


BOOT = Path("target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin")
MANIFEST = "fabric-bootimage.json"
VARIANTS = ("default", "banc-gpu-composition-panne")


def git(revision):
    return subprocess.check_output(["git", "rev-parse", revision], text=True).strip()


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def identity(variant="default"):
    return {
        "schema": 1,
        "source_sha": git("HEAD"),
        "source_tree": git("HEAD^{tree}"),
        "toolchain_config_sha256": digest(Path("rust-toolchain.toml")),
        "cargo_lock_sha256": digest(Path("Cargo.lock")),
        "cargo_config_sha256": digest(Path(".cargo/config.toml")),
        "target_spec_sha256": digest(Path("targets/x86_64-bouchaud_os.json")),
        "target": "x86_64-bouchaud_os",
        "profile": "debug",
        "features": variant,
        "image": "bios-bootimage",
    }


def publish(directory, variant="default"):
    directory.mkdir(parents=True, exist_ok=True)
    image = directory / BOOT.name
    shutil.copyfile(BOOT, image)
    manifest = {
        **identity(variant),
        "compiler": subprocess.check_output(["rustc", "-vV"], text=True).strip(),
        "image_size": image.stat().st_size,
        "image_sha256": digest(image),
    }
    (directory / MANIFEST).write_text(json.dumps(manifest, indent=2) + "\n")


def install(directory, variant="default"):
    destination = BOOT if variant == "default" else Path("boot-panne.bin")
    manifest = json.loads((directory / MANIFEST).read_text())
    for key, expected in identity(variant).items():
        if manifest.get(key) != expected:
            raise ValueError(f"identité de compilation invalide : {key}")
    if not isinstance(manifest.get("compiler"), str) or not manifest["compiler"]:
        raise ValueError("identité du compilateur absente")
    image = directory / BOOT.name
    if not image.is_file() or image.is_symlink() or not image.stat().st_size:
        raise ValueError("bootimage absente, vide ou non régulière")
    if manifest.get("image_size") != image.stat().st_size or manifest.get("image_sha256") != digest(image):
        raise ValueError("empreinte de l’image invalide")
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(image, destination)
    print(f"FABRIC_BOOTIMAGE_OK sha={manifest['source_sha']} sha256={manifest['image_sha256']}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("publish", "install"))
    parser.add_argument("directory", type=Path)
    parser.add_argument("--variant", choices=VARIANTS, default="default")
    args = parser.parse_args()
    (publish if args.command == "publish" else install)(args.directory, args.variant)


if __name__ == "__main__":
    main()
