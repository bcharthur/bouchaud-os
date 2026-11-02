#!/usr/bin/env python3
"""C22 V22: lock-consistent, read-only physical page ownership evidence."""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[3]
GUI = ROOT / "src/gui/services.rs"
VIRTUAL = ROOT / "src/kernel/memory/virtual.rs"


class C22V22OwnerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.gui = GUI.read_text(encoding="utf-8")
        cls.virt = VIRTUAL.read_text(encoding="utf-8")

    def test_marker_and_exact_owner(self):
        self.assertIn("[C22:PAGE-OWNER-V22]", self.gui)
        self.assertIn("regions.iter().rev().find", self.gui)
        self.assertIn("owner.debut == TARGETS[i]", self.gui)
        self.assertIn("target_pages[i] += 1", self.gui)
        self.assertIn("ids[i] != 0", self.gui)

    def test_same_lock_and_rss_crosscheck(self):
        s = self.gui
        self.assertIn("let mm = process.mm.lock();", s)
        self.assertIn("(mm.promesses.clone(), mm.space.iter_user_pages(), mm.space.resident_stats())", s)
        self.assertIn("page_count == resident.total_pages", s)
        self.assertIn("resident.anonymous_pages", s)
        self.assertIn("chosen <= totals[0]", s)
        self.assertIn("other_anon = totals[0].saturating_sub(chosen)", s)

    def test_no_reclaim_and_narrow_logging(self):
        self.assertIn('base == "WebContent" && row.pid == 20', self.gui)
        self.assertIn("now.saturating_sub(last) >= 5_000", self.gui)
        self.assertIn("C22_PTE_LAST_MS.compare_exchange", self.gui)
        probe = self.gui.split("// BOUCHAUD_C22_PTE_OWNER_V22:", 1)[1].split(
            "                let detail = crate::kernel::task::fautes_par_categorie(row.pid);", 1
        )[0]
        self.assertNotIn("collect_garbage", probe)
        self.assertNotIn("madvise", probe)
        self.assertNotIn("unmap", probe)
        self.assertNotIn("MEMOIRE_BORNEE", probe)
        self.assertNotIn("1024", probe)

    def test_pte_classifier_same_as_rss(self):
        s = self.virt
        self.assertIn("BOUCHAUD_C22_PTE_OWNER_V22", s)
        self.assertIn("match ResidentKind::from_entry(entry)", s)
        for kind in ("Anonymous", "FilePrivate", "Shared", "Device", "Untracked"):
            self.assertIn("ResidentKind." + kind, s)


if __name__ == "__main__":
    unittest.main()
