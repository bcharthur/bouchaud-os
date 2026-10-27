#!/usr/bin/env python3
"""BOUCHAUD_CI_PALIERS_V2 : un vert de developpement n'est jamais une convergence,
et une epreuve requise sautee, annulee ou en echec n'est jamais un vert."""
import importlib.util
import os
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / "tools" / "ci" / "verdict_convergence.py"
sys.path.insert(0, str(ROOT / "tools" / "ci"))
import contrat_paliers as contrat  # noqa: E402


def env(palier, mode, **kw):
    e = {}
    for ep in contrat.EPREUVES:
        e[ep.cle] = "success" if ep.exigence[palier] == contrat.REQUIS else "skipped"
    e.update(BOUCHAUD_PALIER=palier, PROVENANCE_MODE=mode,
             R_BUILD="success" if mode == "construit" else "skipped",
             R_REUTILISE="success" if mode == "reutilise" else "skipped")
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

    def test_run_40554668634_reste_rouge(self):
        # Ce qu'a vu le verdict du run : provenance et artefact verts, les
        # epreuves Ladybird toutes sautees par cablage.
        e = env("developpement", "reutilise")
        for ep in contrat.EPREUVES:
            if ep.job not in ("provenance", "artefact", "primitives"):
                e[ep.cle] = "skipped"
        code, sortie = lance(e)
        self.assertEqual(code, 1, sortie)
        self.assertEqual(sortie.count("=skipped"), 13, sortie)

    def test_chaque_epreuve_requise(self):
        """Toute epreuve requise, dans chaque palier, en echec/annulee/sautee/absente : rouge."""
        for palier, mode in (("qualification", "construit"), ("developpement", "reutilise")):
            for ep in contrat.EPREUVES:
                if ep.exigence[palier] != contrat.REQUIS:
                    continue
                for conclusion in ("failure", "cancelled", "skipped", None):
                    with self.subTest(palier=palier, job=ep.job, conclusion=conclusion):
                        e = env(palier, mode)
                        if conclusion is None:
                            del e[ep.cle]
                        else:
                            e[ep.cle] = conclusion
                        lignes, causes = contrat.juge(e)
                        self.assertTrue(any(c.startswith(ep.job + "=") for c in causes), causes)

    def test_hors_run_execute_ou_en_echec_est_rouge(self):
        for conclusion in ("failure", "cancelled", "success", None):
            with self.subTest(conclusion=conclusion):
                e = env("developpement", "reutilise")
                if conclusion is None:
                    del e["R_PRIMITIVES"]
                else:
                    e["R_PRIMITIVES"] = conclusion
                self.assertTrue(contrat.juge(e)[1])

    def test_rouges(self):
        cas = {
            # palier
            "palier absent": env("qualification", "construit", BOUCHAUD_PALIER=""),
            "palier inconnu": env("qualification", "construit", BOUCHAUD_PALIER="rapide"),
            "palier en majuscules": env("qualification", "construit", BOUCHAUD_PALIER="QUALIFICATION"),
            # provenance invalide
            "provenance absente": env("developpement", "reutilise", PROVENANCE_MODE=""),
            "provenance inconnue": env("developpement", "reutilise", PROVENANCE_MODE="telecharge"),
            "provenance en echec": env("developpement", "reutilise", R_PROVENANCE="failure"),
            "qualification sur artefact reutilise": env("qualification", "reutilise"),
            "deux producteurs": env("developpement", "reutilise", R_BUILD="success"),
            "decision contredite": env("developpement", "reutilise", R_BUILD="success", R_REUTILISE="skipped"),
            # build absent
            "qualification build saute": env("qualification", "construit", R_BUILD="skipped"),
            "qualification build absent": {k: v for k, v in env("qualification", "construit").items()
                                           if k != "R_BUILD"},
            "qualification build echoue": env("qualification", "construit", R_BUILD="failure"),
            "reutilisation echouee": env("developpement", "reutilise", R_REUTILISE="failure"),
            "artefact indisponible": env("developpement", "reutilise", R_ARTEFACT="skipped"),
            # epreuves
            "epreuve sautee": env("developpement", "reutilise", R_WPT="skipped"),
            "epreuve echouee": env("qualification", "construit", R_SITES="failure"),
            "primitives sautees en qualification": env("qualification", "construit", R_PRIMITIVES="skipped"),
        }
        for nom, e in cas.items():
            with self.subTest(nom):
                code, sortie = lance(e)
                self.assertEqual(code, 1, sortie)
                self.assertIn("BOUCHAUD_LADYBIRD_CONVERGENCE_FAIL", sortie)
                self.assertNotIn("_OK", sortie.replace("BOUCHAUD_TEST_HEAD", ""))


class SimulationTests(unittest.TestCase):
    """Le simulateur rejoue la semantique `needs`/`if` de GitHub sur le vrai workflow."""

    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location("simule_paliers", ROOT / "tools/ci/simule_paliers.py")
        cls.sp = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.sp)
        cls.texte = (ROOT / ".github/workflows/ladybird-native-browser.yml").read_text(encoding="utf-8")

    def test_workflow_courant_conforme(self):
        self.assertEqual(self.sp.controle(self.texte), [])

    def test_cablage_du_run_40554668634_reproduit(self):
        # Consommateurs ramenes a `needs: artefact` sans condition : c'est le
        # cablage de 14f9bcfa. GitHub a saute ces 15 jobs ; le simulateur aussi.
        casse = self.texte.replace(
            "    needs: [dedoublonne, artefact]\n", "    needs: artefact\n")
        casse = "\n".join(l for l in casse.split("\n")
                          if l != "    if: ${{ !cancelled() && needs.dedoublonne.outputs.reutilise != 'true' && needs.artefact.result == 'success' }}")
        sc = self.sp.Scenario("dev", "push", mode="reutilise")
        r = self.sp.simule(casse, sc)
        sautes = sorted(j for j, res in r["resultats"].items() if res == "skipped")
        self.assertEqual(r["resultats"]["artefact"], "success")
        self.assertEqual(len([j for j in sautes if j not in ("build", "primitives", "performance")]), 14, sautes)
        self.assertFalse(r["verdict_ok"])
        self.assertTrue(self.sp.controle(casse))

    def test_semantique_success_implicite_transitive(self):
        mini = """jobs:
  a:
    runs-on: x
  b:
    needs: a
    if: ${{ false }}
  c:
    needs: [a, b]
    if: ${{ always() }}
  d:
    needs: c
  e:
    needs: c
    if: ${{ !cancelled() && needs.c.result == 'success' }}
  convergence:
    needs: [d, e]
    if: always()
    name: v
    steps:
      - name: Verdict
        env:
          BOUCHAUD_PALIER: qualification
        run: x
"""
        r = self.sp.simule(mini, self.sp.Scenario("s", "push"))
        self.assertEqual(r["resultats"], {"a": "success", "b": "skipped", "c": "success", "d": "skipped",
                                          "e": "success", "convergence": "success"})

    def test_lecture_d_un_besoin_indirect_refusee(self):
        mini = """jobs:
  a:
    runs-on: x
  b:
    needs: a
  c:
    needs: b
    if: ${{ needs.a.result == 'success' }}
"""
        with self.assertRaises(self.sp.SimulationErreur):
            self.sp.simule(mini, self.sp.Scenario("s", "push"))

    def test_evaluateur(self):
        ev = self.sp.Evaluateur(lambda i: {"x": "a", "y": ""}[i], {"always": lambda: True})
        self.assertEqual(ev.evalue("${{ x == 'A' && 'oui' || 'non' }}"), "oui")
        self.assertEqual(ev.evalue("y && 'oui' || 'non'"), "non")
        self.assertTrue(ev.evalue("!(y) && always()"))
        self.assertEqual(ev.evalue("'l''apostrophe'"), "l'apostrophe")
        with self.assertRaises(self.sp.SimulationErreur):
            ev.evalue("contains(x, 'a')")


if __name__ == "__main__":
    unittest.main()
