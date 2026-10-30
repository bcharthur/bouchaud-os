#!/usr/bin/env python3
"""Garanties structurelles du correctif C22 apres fermeture, avant rebuild C++."""
from pathlib import Path
import ast
import unittest

SRC = Path(__file__).resolve().parents[2] / "ladybird" / "prepare-echange-processus.py"
# tests reside under tools/ci/reliability; tools/ is parents[2]

class C22ClosureTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.code = SRC.read_text(encoding="utf-8")
        cls.tree = ast.parse(cls.code)
    def test_collect_after_page_discard(self):
        s=self.code
        self.assertIn("BOUCHAUD_C22_GC_APRES_FERMETURE_V1",s)
        self.assertIn("PAGE_DISCARD_END navigable=",s)
        self.assertIn("deferred_invoke(GC::create_function",s)
        self.assertIn("collect_garbage();",s)
        self.assertIn("ak_kmalloc_collect();",s)
    def test_no_reference_to_closed_page_in_delayed_gc(self):
        s=self.code
        self.assertIn("[owning_heap]",s)
        self.assertNotIn("[this, owning_heap]",s)
        self.assertIn("BOUCHAUD_LB_C22_TEMOIN_HISTORIQUE",s)
    def test_no_threshold_change(self):
        s=self.code
        self.assertNotIn("MEMOIRE_BORNEE=1",s)
        self.assertNotIn("g2 > 4096",s)
    def test_source_patch_has_valid_python_ast(self):
        self.assertTrue(any(isinstance(n, ast.FunctionDef) and n.name == "main" for n in ast.walk(self.tree)))

if __name__ == "__main__":
    unittest.main()
