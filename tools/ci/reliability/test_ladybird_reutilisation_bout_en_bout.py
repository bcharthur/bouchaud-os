#!/usr/bin/env python3
"""BOUCHAUD_LADYBIRD_PRODUCTION_V1 -- non-regression de bout en bout.

Un vrai depot git, la vraie empreinte (ladybird_empreinte.empreinte) et la
vraie decision (ladybird_provenance.decide) :

  * un commit purement NOYAU reutilise l'artefact du build precedent, en
    developpement comme en QUALIFICATION -- le navigateur n'est pas recompile ;
  * l'orchestration ajoutee par la refonte (palier artefact, concurrence par
    empreinte, run-name, job d'attente, conditions) ne change pas l'empreinte :
    les artefacts produits avant elle restent valides ;
  * un changement de SOURCE (SHA upstream), de PATCH, d'OUTIL (apt, chaine de
    compilation), de DEPENDANCE (vcpkg) ou d'ETAPE productrice du build
    invalide l'artefact : il n'est jamais reutilise.
"""
import importlib.util
import os
import unittest
from pathlib import Path

RACINE = Path(__file__).resolve().parents[3]


def charge(nom, chemin):
    spec = importlib.util.spec_from_file_location(nom, chemin)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


te = charge("test_ladybird_empreinte_base", RACINE / "tools/ci/reliability/test_ladybird_empreinte.py")
le = te.le
lp = charge("ladybird_provenance_e2e", RACINE / "tools/ci/ladybird_provenance.py")

DEPOT = "bcharthur/bouchaud-os"
WF = ".github/workflows/ladybird-native-browser.yml"


class Api:
    """Un seul run producteur (id 1) dont le build a reussi et publie."""

    def __init__(self, head):
        self.run = {"id": 1, "repository": {"full_name": DEPOT}, "head_repository": {"full_name": DEPOT},
                    "path": lp.WORKFLOW, "event": "push", "head_sha": head, "created_at": "2026-11-10T10:00:00Z"}

    def __call__(self, chemin):
        if chemin.startswith("actions/workflows/"):
            return {"workflow_runs": []}
        if chemin == "actions/runs/1":
            return self.run
        if "/jobs" in chemin:
            return {"jobs": [{"name": lp.JOB_BUILD, "conclusion": "success"}]}
        return {"artifacts": [{"name": lp.ARTEFACT, "expired": False}, {"name": lp.CORPUS, "expired": False}]}


class BoutEnBoutTests(unittest.TestCase):
    def setUp(self):
        self.depot = te.Depot()
        self.ancien = os.getcwd()
        os.chdir(self.depot.racine)
        self.producteur = self.depot.git("rev-parse", "HEAD")

    def tearDown(self):
        os.chdir(self.ancien)
        self.depot.td.cleanup()

    def decision(self, palier="qualification"):
        courant = le.empreinte("HEAD")["empreinte"]
        return lp.decide(palier, 999, courant, [{"id": 10, "name": lp.ARTEFACT, "expired": False,
                                                 "workflow_run": {"id": 1}}],
                         Api(self.producteur), lambda sha: le.empreinte(sha)["empreinte"], DEPOT,
                         declenche=None, sortie=lambda *_: None)

    def test_changement_noyau_reutilise_en_qualification_et_en_developpement(self):
        self.depot.remplace("src/kernel/noyau.rs", "fn main() {}", "fn main() { ordonnanceur(); }")
        self.depot.commit("noyau")
        for palier in ("qualification", "developpement"):
            with self.subTest(palier=palier):
                d = self.decision(palier)
                self.assertEqual((d["mode"], d["source_run"]), ("reutilise", "1"), d["raison"])

    def test_orchestration_de_la_refonte_ne_change_pas_l_empreinte(self):
        self.depot.remplace(WF, "jobs:\n", "run-name: ${{ inputs.palier == 'artefact' && 'p' || '' }}\n"
                            "concurrency:\n  group: ${{ inputs.palier }}\n  cancel-in-progress: false\n\njobs:\n"
                            "  attente:\n    needs: [provenance]\n    if: ${{ needs.provenance.outputs.mode == 'attendu' }}\n"
                            "    runs-on: ubuntu-latest\n    steps:\n      - run: python3 tools/ci/ladybird_provenance.py attends\n")
        self.depot.remplace(WF, "    runs-on: ubuntu-24.04\n    timeout-minutes: 180",
                            "    if: ${{ needs.provenance.outputs.mode != 'attendu' }}\n"
                            "    runs-on: ubuntu-24.04\n    timeout-minutes: 180")
        self.depot.commit("orchestration")
        d = self.decision()
        self.assertEqual(d["mode"], "reutilise", d["raison"])

    def test_chaque_entree_du_navigateur_invalide_l_artefact(self):
        cas = {
            "source (SHA upstream)": ("third_party/UPSTREAM.md", "a" * 40, "c" * 40),
            "outil (paquets apt)": ("tools/ci/apt-installe.sh", "apt-get install", "apt-get install -y"),
            "dependance (vcpkg)": ("tools/ladybird/browser-vcpkg.sh", "echo vcpkg", "echo vcpkg harfbuzz"),
            "recette (script de build)": ("tools/ladybird/browser-upstream.sh", "echo build", "echo build -O3"),
            "etape productrice du workflow": (WF, "tools/ci/apt-installe.sh clang", "tools/ci/apt-installe.sh clang lld"),
        }
        for nom, (chemin, avant, apres) in cas.items():
            with self.subTest(nom):
                self.depot.git("reset", "-q", "--hard", self.producteur)
                self.depot.remplace(chemin, avant, apres)
                self.depot.commit(nom)
                d = self.decision()
                self.assertEqual(d["mode"], "construit", f"{nom} : {d['raison']}")

    def test_nouveau_patch_invalide_l_artefact(self):
        self.depot.ecrit("tools/ladybird/patches/0001-correctif.patch", "+fix\n")
        self.depot.commit("patch")
        self.assertEqual(self.decision()["mode"], "construit")


if __name__ == "__main__":
    unittest.main()
