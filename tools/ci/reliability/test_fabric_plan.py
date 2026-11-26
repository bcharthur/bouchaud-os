"""Tests du plan et de l'intégrité de l'image mutualisée."""
import json
from pathlib import Path
import sys
import tempfile
import unittest
import yaml
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import fabric_bootimage
from fabric_verdict import verdict
from fabric_plan import COMPAT, CONSUMERS, MIGRATED, plan


class FabricPlan(unittest.TestCase):
    def test_les_contrats_pointent_vers_des_bancs_actifs(self):
        root = Path(__file__).resolve().parents[3]
        contracts = json.loads((root / "tools/ci/fabric_contracts.json").read_text())["contracts"]
        for identifier, record in contracts.items():
            with self.subTest(contract=identifier):
                document = yaml.safe_load((root / record["workflow"]).read_text())
                job = document["jobs"][record["job"]]
                self.assertFalse(job.get("continue-on-error"))
                by_name = {step.get("name"): step for step in job["steps"]}
                for name in record["steps"]:
                    self.assertIn("run", by_name[name])
                    self.assertFalse(by_name[name].get("continue-on-error"))
                    self.assertNotIn(by_name[name].get("if"), ("false", False, "${{ false }}"))
                self.assertTrue(record["evidence"])

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

    def test_verdict_fait_echouer_une_preuve_manquante(self):
        needs = {
            "plan": {"result": "success", "outputs": {
                "gpu": "true", "security": "false", "storage": "false", "p13": "false", "memory": "false", "ipc": "false", "any": "true",
            }},
            "kernel": {"result": "success"},
            "gpu-witness": {"result": "success"},
            "gpu": {"result": "skipped"},
            "security": {"result": "skipped"},
            "storage": {"result": "skipped"},
        }
        self.assertEqual(verdict(needs)["gpu"], "NOT_RUN")
        self.assertEqual(verdict(needs)["storage"], "SKIPPED_BY_PLAN")
        needs["gpu"]["result"] = "cancelled"
        self.assertEqual(verdict(needs)["gpu"], "CANCELLED")
        needs["plan"]["outputs"]["any"] = "false"
        self.assertEqual(verdict(needs)["plan"], "FAIL")
        self.assertEqual(verdict(needs)["kernel"], "BLOCKED")

    def test_configuration_du_temoin_refusee_comme_standard(self):
        with tempfile.TemporaryDirectory() as root, patch.object(fabric_bootimage, "identity",
                side_effect=lambda variant: {"source_sha": "abc", "features": variant}), patch.object(
                fabric_bootimage.subprocess, "check_output", return_value="rustc"):
            root = Path(root)
            image = root / "bootimage.bin"
            image.write_bytes(b"witness")
            archive = root / "archive"
            with patch.object(fabric_bootimage, "BOOT", image):
                fabric_bootimage.publish(archive, "banc-gpu-composition-panne")
                with self.assertRaisesRegex(ValueError, "features"):
                    fabric_bootimage.install(archive)

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
