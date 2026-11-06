"""Contrat pur de la gouvernance ; aucune requête GitHub lors du test."""
import importlib.util
from pathlib import Path
import unittest

source = Path(__file__).resolve().parents[2] / "github" / "audit_repository_state.py"
spec = importlib.util.spec_from_file_location("audit_repository_state", source)
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)

class TestGovernance(unittest.TestCase):
    A = "a" * 40
    B = "b" * 40

    def test_main_protegee_et_snapshot_exact(self):
        self.assertEqual(audit.conclusions(True, self.A, self.A), [])

    def test_main_sans_protection_refusee(self):
        problems = audit.conclusions(False, self.A, self.A)
        self.assertTrue(any("MAIN_UNPROTECTED" in x for x in problems))

    def test_snapshot_perime_refuse(self):
        problems = audit.conclusions(True, self.B, self.A)
        self.assertTrue(any("STATUS_SNAPSHOT_STALE" in x for x in problems))

    def test_sha_invalide_refuse(self):
        self.assertIn("STATUS_SNAPSHOT_INVALID", audit.conclusions(True, self.A, "main"))

if __name__ == "__main__":
    unittest.main()
