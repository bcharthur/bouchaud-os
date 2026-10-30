#!/usr/bin/env python3
"""Contrat C22 : politique mimalloc du navigateur PRODUIT, pas un flag de banc."""
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SOURCE = ROOT / "src/gui/client.rs"


class BrowserPurgeRuntimeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.source = SOURCE.read_text(encoding="utf-8")

    def test_product_launcher_injects_proven_profile(self):
        s = self.source
        start = s.index("BOUCHAUD_C22_PURGE_PRODUIT_V1")
        end = s.index("let pid =", start)
        section = s[start:end]
        self.assertIn("MIMALLOC_PURGE_DELAY=0", section)
        self.assertIn("MIMALLOC_PURGE_DECOMMITS=1", section)
        self.assertIn("chemin == CHEMIN_NAVIGATEUR", section)
        self.assertIn("BOUCHAUD_LB_C22_TEMOIN_HISTORIQUE=1", section)
        self.assertIn("let mut base = crate::kernel::exec::shell_environment()", section)
        self.assertIn("lance_detache", s[end:])
        self.assertEqual(section.count("base.push("), 2)

    def test_explicit_user_configuration_wins(self):
        s = self.source
        self.assertIn('entry.starts_with("MIMALLOC_PURGE_DELAY=")', s)
        self.assertIn('entry.starts_with("MIMALLOC_PURGE_DECOMMITS=")', s)

    def test_memory_threshold_is_not_modified(self):
        analyzer = (ROOT / "tools/ci/analyse_memoire.py").read_text(encoding="utf-8")
        self.assertNotIn("BOUCHAUD_C22_PURGE_PRODUIT_V1", analyzer)
        bench = (ROOT / "tools/ci/run_ladybird_memoire.sh").read_text(encoding="utf-8")
        self.assertIn('BO_C22_TEMOIN', bench)
        self.assertIn('C22_TEMOIN_GUEST_EXPORT', bench)


if __name__ == "__main__":
    unittest.main()
