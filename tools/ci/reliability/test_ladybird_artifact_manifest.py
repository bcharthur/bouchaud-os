#!/usr/bin/env python3
import importlib.util
import json
import os
import struct
import tempfile
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location(
    "artifact_manifest", ROOT / "tools" / "ladybird" / "artifact_manifest.py"
)
m = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(m)


def elf64(interp=False):
    # ELF64 little endian minimal, avec une table PH valide.
    data = bytearray(64 + 56)
    data[:4] = b"\x7fELF"
    data[4] = 2
    data[5] = 1
    struct.pack_into("<Q", data, 32, 64)
    struct.pack_into("<H", data, 54, 56)
    struct.pack_into("<H", data, 56, 1)
    struct.pack_into("<I", data, 64, 3 if interp else 1)
    return bytes(data)


class ArtifactManifestTests(unittest.TestCase):
    def make_tree(self, root: Path, interp_name=None):
        for name in m.ELF_RUNTIME:
            (root / name).write_bytes(elf64(name == interp_name))
        (root / "webcontent-bootstrap").write_text("#!/bin/sh\n", encoding="utf-8")
        for name in ("M9_CAPABLE", "V16_UI_CAPABLE", "V19_UI_CAPABLE"):
            (root / name).write_text(name + "\n", encoding="utf-8")
        (root / "resources").mkdir()
        (root / "resources" / "a.txt").write_text("alpha\n", encoding="utf-8")

    def test_create_verify_and_corruption(self):
        with tempfile.TemporaryDirectory() as td:
            base = Path(td)
            root = base / "artifact"
            root.mkdir()
            self.make_tree(root)
            upstream = base / "UPSTREAM.md"
            upstream.write_text("    sha     " + "a" * 40 + "\n", encoding="utf-8")
            with mock.patch.object(m, "git_head", return_value="b" * 40):
                manifest = m.build_manifest(root, upstream)
                path = root / m.MANIFEST_NAME
                path.write_text(json.dumps(manifest), encoding="utf-8")
                m.verify_manifest(root, upstream, path, check_head=False)
                (root / "WebWorker").write_bytes(elf64() + b"corruption")
                with self.assertRaises(m.ManifestError):
                    m.verify_manifest(root, upstream, path, check_head=False)

    def test_interp_is_rejected(self):
        with tempfile.TemporaryDirectory() as td:
            base = Path(td)
            root = base / "artifact"
            root.mkdir()
            self.make_tree(root, interp_name="WebWorker")
            upstream = base / "UPSTREAM.md"
            upstream.write_text("sha " + "a" * 40 + "\n", encoding="utf-8")
            with mock.patch.object(m, "git_head", return_value="b" * 40):
                with self.assertRaises(m.ManifestError):
                    m.build_manifest(root, upstream)


if __name__ == "__main__":
    unittest.main(verbosity=2)
