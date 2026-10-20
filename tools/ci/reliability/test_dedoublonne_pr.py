"""BOUCHAUD_CI_UN_BUILD_PAR_SHA_V1 : le run PR ne se saute que s'il refait
exactement un run push vivant sur le meme arbre."""
import importlib.util
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

RACINE = Path(__file__).resolve().parents[3]
SCRIPT = RACINE / "tools/ci/dedoublonne_pr.py"
spec = importlib.util.spec_from_file_location("dedoublonne_pr", SCRIPT)
dd = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dd)

DEPOT = "bcharthur/bouchaud-os"
ARBRE = "a" * 40
PUSH_VIVANT = [{"conclusion": None, "html_url": "https://github.com/x/runs/1"}]


def decide(**kw):
    args = dict(evenement="pull_request", action="synchronize", depot=DEPOT, depot_tete=DEPOT,
                arbre_tete=ARBRE, arbre_fusion=ARBRE, runs_push=PUSH_VIVANT)
    args.update(kw)
    return dd.decide(**args)[0]


class Decision(unittest.TestCase):
    def test_doublon_exact(self):
        self.assertTrue(decide())

    def test_push_termine_compte(self):
        self.assertTrue(decide(runs_push=[{"conclusion": "failure"}]))
        self.assertTrue(decide(runs_push=[{"conclusion": "success"}]))

    # Les negatifs : chacun retire UNE condition et le run PR doit tourner.
    def test_base_avancee(self):
        self.assertFalse(decide(arbre_fusion="b" * 40))

    def test_sans_run_push(self):
        self.assertFalse(decide(runs_push=[]))

    def test_run_push_annule_ou_saute(self):
        self.assertFalse(decide(runs_push=[{"conclusion": "cancelled"}]))
        self.assertFalse(decide(runs_push=[{"conclusion": "skipped"}, {"conclusion": "cancelled"}]))

    def test_fork(self):
        self.assertFalse(decide(depot_tete="autre/bouchaud-os"))

    def test_libelle_explicite(self):
        self.assertFalse(decide(action="labeled"))

    def test_evenements_hors_pr(self):
        for evt in ("push", "schedule", "workflow_dispatch", ""):
            self.assertFalse(decide(evenement=evt))

    def test_arbres_inconnus(self):
        self.assertFalse(decide(arbre_tete=""))
        self.assertFalse(decide(arbre_fusion=""))

    def test_fichier_workflow(self):
        self.assertEqual(
            dd.fichier_workflow("bcharthur/bouchaud-os/.github/workflows/ci.yml@refs/pull/226/merge"),
            "ci.yml")


class Repli(unittest.TestCase):
    def test_api_absente_le_run_tourne(self):
        # `gh` introuvable : la decision echoue, la sortie dit `doublon=false`.
        with tempfile.TemporaryDirectory() as d:
            sortie = Path(d) / "out"
            env = {"PATH": d, "EVENEMENT": "pull_request", "DEPOT": DEPOT, "TETE": "1" * 40,
                   "FUSION": "2" * 40, "WORKFLOW_REF": "x/y/.github/workflows/ci.yml@r",
                   "GITHUB_OUTPUT": str(sortie)}
            r = subprocess.run([sys.executable, str(SCRIPT)], env=env, capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertIn("doublon=false", r.stdout)
            self.assertEqual(sortie.read_text().strip(), "doublon=false")


if __name__ == "__main__":
    unittest.main()
