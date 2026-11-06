"""Régressions P0 : protection du commit A/B et d'une session Atelier vide.

Exécute par run_host_tests.sh. Ces gardes sont indépendantes des campagnes QEMU :
elles détectent une suppression involontaire du code même si le banc saute.
"""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[3]


class TestCriticalRegressionGuards(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.ab = (ROOT / "tools/ci/run_ab_rollback.sh").read_text(encoding="utf-8")
        cls.wm = (ROOT / "src/gui/window_manager.rs").read_text(encoding="utf-8")
        cls.atelier = (ROOT / "src/gui/atelier/mod.rs").read_text(encoding="utf-8")

    def test_ab_failure_logs_preserved(self):
        self.assertIn("diagnostic_echec() {", self.ab)
        self.assertIn("diagnostic_echec; rm -rf", self.ab)
        self.assertIn("BOUCHAUD_USB_STOCKAGE_IO_ECHEC", self.ab)
        self.assertIn("AB_DIAG", self.ab)

    def test_commit_waits_for_postcommit_readback(self):
        self.assertIn("attend_etat_apres_commit() {", self.ab)
        self.assertIn("SERVICED_UPDATE_STATE", self.ab)
        start = self.ab.index('if [ -n "$slot" ] && grep -aq "result=committed"')
        stop = self.ab.index('if [ -n "$slot" ] && grep -aq "result=already_good"', start)
        branch = self.ab[start:stop]
        self.assertIn("attend_etat_apres_commit || true", branch)
        self.assertLess(branch.index("attend_etat_apres_commit || true"), branch.index('verdict="$slot+"'))

    def test_atelier_escape_never_quits_without_window(self):
        self.assertIn("pub mod session;", self.atelier)
        self.assertIn("action_echap(prise, !wins.is_empty())", self.wm)
        self.assertIn("EchapSansMenu::Ignorer", self.wm)
        start = self.wm.index("match action_echap(prise, !wins.is_empty())")
        end = self.wm.index("\n                continue;", start)
        self.assertNotIn("quit = true", self.wm[start:end])
        self.assertTrue((ROOT / "tools/gui/test_session.rs").is_file())


if __name__ == "__main__":
    unittest.main()
