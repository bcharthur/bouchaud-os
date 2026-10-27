#!/usr/bin/env python3
"""BOUCHAUD_CI_PALIERS_V1 : un vert de developpement n'est jamais une convergence."""
import importlib.util
import os
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / "tools" / "ci" / "verdict_convergence.py"
SPEC = importlib.util.spec_from_file_location("verdict_convergence", SCRIPT)
vc = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(vc)


def env(palier, mode, **kw):
    e = {cle: "success" for cle, _ in vc.EPREUVES}
    e.update(BOUCHAUD_PALIER=palier, PROVENANCE_MODE=mode)
    if mode == "construit":
        e["R_REUTILISE"] = "skipped"
    else:
        e.update(R_BUILD="skipped", R_REUTILISE="success")
    if palier == "developpement":
        e["R_PRIMITIVES"] = "skipped"
    e.update(kw)
    return e


def lance(e):
    complet = {k: v for k, v in os.environ.items() if not k.startswith(("R_", "BOUCHAUD_", "PROVENANCE_"))}
    complet.update(e)
    r = subprocess.run([sys.executable, str(SCRIPT)], cwd=ROOT, env=complet, capture_output=True, text=True)
    return r.returncode, r.stdout


class PaliersTests(unittest.TestCase):
    def test_qualification_complete(self):
        code, sortie = lance(env("qualification", "construit"))
        self.assertEqual(code, 0, sortie)
        self.assertIn("BOUCHAUD_LADYBIRD_CONVERGENCE_OK", sortie)

    def test_developpement_vert_sans_convergence(self):
        for mode in ("reutilise", "construit"):
            with self.subTest(mode=mode):
                code, sortie = lance(env("developpement", mode))
                self.assertEqual(code, 0, sortie)
                self.assertIn("BOUCHAUD_LADYBIRD_INTEGRATION_OK palier=developpement", sortie)
                self.assertIn("BOUCHAUD_LADYBIRD_CONVERGENCE_NON_QUALIFIANTE", sortie)
                self.assertNotIn("BOUCHAUD_LADYBIRD_CONVERGENCE_OK", sortie)

    def test_rouges(self):
        cas = {
            "palier absent": env("qualification", "construit", BOUCHAUD_PALIER=""),
            "palier inconnu": env("qualification", "construit", BOUCHAUD_PALIER="rapide"),
            "qualification sur artefact reutilise": env("qualification", "reutilise"),
            "qualification sans primitives": env("qualification", "construit", R_PRIMITIVES="skipped"),
            "qualification build echoue": env("qualification", "construit", R_BUILD="failure"),
            "provenance absente": env("developpement", "reutilise", PROVENANCE_MODE=""),
            "deux producteurs": env("developpement", "reutilise", R_BUILD="success"),
            "decision contredite": env("developpement", "reutilise", R_BUILD="success", R_REUTILISE="skipped"),
            "reutilisation echouee": env("developpement", "reutilise", R_REUTILISE="failure"),
            "primitives en echec en developpement": env("developpement", "reutilise", R_PRIMITIVES="failure"),
            "primitives annulees en developpement": env("developpement", "reutilise", R_PRIMITIVES="cancelled"),
            "epreuve sautee": env("developpement", "reutilise", R_WPT="skipped"),
            "epreuve absente": {k: v for k, v in env("developpement", "reutilise").items() if k != "R_SITES"},
        }
        for nom, e in cas.items():
            with self.subTest(nom):
                code, sortie = lance(e)
                self.assertEqual(code, 1, sortie)
                self.assertIn("BOUCHAUD_LADYBIRD_CONVERGENCE_FAIL", sortie)
                self.assertNotIn("_OK", sortie.replace("BOUCHAUD_TEST_HEAD", ""))


if __name__ == "__main__":
    unittest.main()
