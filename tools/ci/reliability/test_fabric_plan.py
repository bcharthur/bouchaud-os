"""Tests du plan et de l'intégrité de l'image mutualisée."""
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import fabric_bootimage
from fabric_plan import COMPAT, CONSUMERS, MIGRATED, plan


class FabricPlan(unittest.TestCase):
    def test_empreinte_historique(self):
        for domain, patterns in COMPAT.items():
            for pattern in patterns:
                with self.subTest(domain=domain, pattern=pattern):
                    result = plan([pattern.replace("**", "deep/fichier.rs")])
                    self.assertIn(domain, result["domains"])

    def test_transitif(self):
        self.assertTrue(MIGRATED <= set(plan(["src/kernel/scheduler/task.rs"])["domains"]))
        self.assertIn("ladybird", plan(["src/kernel/memory/vmm.rs"])["domains"])
        self.assertIn("security", plan(["src/drivers/network/e1000.rs"])["domains"])

    def test_documentation_et_inconnu(self):
        self.assertEqual(plan(["docs/current/CI_CD_FABRIC.md"])["domains"], [])
        for path in ["nouveau_build.py", ".github/actions/fabric-bootimage/action.yml", "tools/ci/fabric_plan.py"]:
            self.assertEqual(set(plan([path])["domains"]), set(CONSUMERS))
        self.assertEqual(set(plan(["README.md"], exhaustive=True)["domains"]), set(CONSUMERS))

    def test_artefact_mutants(self):
        with tempfile.TemporaryDirectory() as root, patch.object(fabric_bootimage, "identity", return_value={
            "source_sha": "abc",
            "source_tree": "tree",
        }), patch.object(fabric_bootimage.subprocess, "check_output", return_value="rustc"):
            root = Path(root)
            archive = root / "archive"
            boot = root / "local" / "bootimage.bin"
            boot.parent.mkdir()
            boot.write_bytes(b"bootimage")
            with patch.object(fabric_bootimage, "BOOT", boot):
                fabric_bootimage.publish(archive)
                boot.unlink()
                fabric_bootimage.install(archive)
                self.assertEqual(boot.read_bytes(), b"bootimage")
                manifest = archive / fabric_bootimage.MANIFEST
                original = json.loads(manifest.read_text())
                manifest.write_text(json.dumps({**original, "source_sha": "old"}))
                with self.assertRaisesRegex(ValueError, "source_sha"):
                    fabric_bootimage.install(archive)
                manifest.write_text(json.dumps(original))
                (archive / boot.name).write_bytes(b"corrupted")
                with self.assertRaisesRegex(ValueError, "empreinte"):
                    fabric_bootimage.install(archive)


if __name__ == "__main__":
    unittest.main()
