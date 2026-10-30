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
    def test_partition_heaps_thread_affinity(self):
        # mi_heap_new() n'est pas partageable comme un tas global entre fils.
        partition_patcher = (SRC.parent / "prepare-c22-partitions.py").read_text(encoding="utf-8")
        ast.parse(partition_patcher)
        self.assertIn("BOUCHAUD_C22_PARTITIONS_PAR_FIL_V2", partition_patcher)
        self.assertIn("static thread_local mi_heap_t* s_bouchaud_partitions[4]", partition_patcher)
        self.assertIn("static thread_local mi_heap_t* {nom}", partition_patcher)
        self.assertIn("for (auto* heap : s_bouchaud_partitions)", partition_patcher)
        self.assertNotIn("pthread_equal", partition_patcher)
        self.assertNotIn("std::atomic<mi_heap_t*>", partition_patcher)

    def test_memory_fixture_keeps_native_swap_proof_authoritative(self):
        tools = SRC.parents[1]
        fixture = (tools / "health" / "browser_host_fixture.py").read_text(encoding="utf-8")
        self.assertIn('HOST_MEMOIRE_ONGLET phase=', fixture)
        self.assertIn('HOST_MEMOIRE_ECHEC phase=', fixture)
        self.assertIn('raison=ouverture_refusee', fixture)
        # A cross-process window.opener/postMessage is not a reliable
        # substitute for the native WebContent swap + cleanup proof.
        self.assertNotIn('HOST_MEMOIRE_ENFANT_PRET', fixture)
        self.assertNotIn('navigation_autre_site_non_prouvee', fixture)
        runner = (tools / "ci" / "run_ladybird_memoire.sh").read_text(encoding="utf-8")
        self.assertIn('verdict=enfant_non_prouve', runner)
        self.assertIn('verifie-preuve-swaps.py', runner)

    def test_c22_rss_and_perf_share_one_snapshot(self):
        repo = SRC.parents[2]
        services = (repo / "src" / "gui" / "services.rs").read_text(encoding="utf-8")
        self.assertIn("let c22_snapshot = if base ==", services)
        self.assertIn("let rss_mesure = c22_snapshot.as_ref()", services)
        self.assertIn("rss_mesure / 1024", services)
        self.assertIn("if let Some((process, usage, vma)) = c22_snapshot", services)

    def test_source_patch_has_valid_python_ast(self):
        self.assertTrue(any(isinstance(n, ast.FunctionDef) and n.name == "main" for n in ast.walk(self.tree)))

if __name__ == "__main__":
    unittest.main()
