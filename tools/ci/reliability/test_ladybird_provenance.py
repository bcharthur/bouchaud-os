#!/usr/bin/env python3
"""BOUCHAUD_LADYBIRD_PROVENANCE_V1 : construire ou reutiliser l'artefact Ladybird."""
import importlib.util
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location("ladybird_provenance", ROOT / "tools" / "ci" / "ladybird_provenance.py")
lp = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(lp)

DEPOT = "bcharthur/bouchaud-os"
E = "e" * 64


def artefact(run_id, aid, name=lp.ARTEFACT, expired=False):
    return {"id": aid, "name": name, "expired": expired, "workflow_run": {"id": run_id}}


def run(rid, **kw):
    r = {"id": rid, "repository": {"full_name": DEPOT}, "head_repository": {"full_name": DEPOT},
         "path": lp.WORKFLOW, "event": "push", "head_sha": f"{rid:040d}"}
    r.update(kw)
    return r


class Api:
    def __init__(self, runs, jobs=None, artefacts=None, pannes=()):
        self.runs, self.jobs, self.artefacts, self.pannes = runs, jobs or {}, artefacts or {}, set(pannes)
        self.appels = []

    def __call__(self, chemin):
        self.appels.append(chemin)
        rid = int(chemin.split("/")[2].split("?")[0])
        if rid in self.pannes:
            raise RuntimeError("HTTP 502")
        if chemin.endswith(f"runs/{rid}"):
            return self.runs[rid]
        if "/jobs" in chemin:
            return self.jobs.get(rid, {"jobs": [{"name": lp.JOB_BUILD, "conclusion": "success"}]})
        return self.artefacts.get(rid, {"artifacts": [{"name": lp.ARTEFACT, "expired": False},
                                                      {"name": lp.CORPUS, "expired": False}]})


def decide(liste, api, empreintes, palier="developpement", courant=999):
    return lp.decide(palier, courant, E, liste, api, lambda sha: empreintes.get(sha, "f" * 64), DEPOT,
                     sortie=lambda *_: None)


class ProvenanceTests(unittest.TestCase):
    def test_qualification_construit_toujours(self):
        api = Api({1: run(1)})
        d = decide([artefact(1, 10)], api, {f"{1:040d}": E}, palier="qualification")
        self.assertEqual(d["mode"], "construit")
        self.assertEqual(api.appels, [])

    def test_palier_inconnu_refuse(self):
        with self.assertRaises(ValueError):
            decide([], Api({}), {}, palier="")

    def test_reutilise_le_plus_recent_aux_entrees_identiques(self):
        runs = {1: run(1), 2: run(2), 3: run(3)}
        liste = [artefact(1, 10), artefact(3, 30), artefact(2, 20)]
        d = decide(liste, Api(runs), {f"{1:040d}": E, f"{2:040d}": E, f"{3:040d}": "a" * 64})
        self.assertEqual((d["mode"], d["source_run"]), ("reutilise", "2"))

    def test_empreinte_differente_construit(self):
        d = decide([artefact(1, 10)], Api({1: run(1)}), {f"{1:040d}": "a" * 64})
        self.assertEqual(d["mode"], "construit")

    def test_sources_non_sures_ecartees(self):
        cas = {
            "fork": run(1, head_repository={"full_name": "pirate/bouchaud-os"}),
            "pull_request": run(1, event="pull_request"),
            "autre workflow": run(1, path=".github/workflows/nightly.yml"),
            "depot etranger": run(1, repository={"full_name": "pirate/x"}),
            "sans head": run(1, head_sha=None),
        }
        for nom, r in cas.items():
            with self.subTest(nom):
                d = decide([artefact(1, 10)], Api({1: r}), {f"{1:040d}": E, None: E})
                self.assertEqual(d["mode"], "construit")

    def test_build_non_reussi_ou_absent_ecarte(self):
        for jobs in ({"jobs": [{"name": lp.JOB_BUILD, "conclusion": "failure"}]},
                     {"jobs": [{"name": lp.JOB_BUILD, "conclusion": "skipped"}]},
                     {"jobs": [{"name": "ladybird / artefact reutilise (entrees identiques)", "conclusion": "success"}]},
                     {"jobs": []}):
            with self.subTest(jobs=jobs):
                d = decide([artefact(1, 10)], Api({1: run(1)}, jobs={1: jobs}), {f"{1:040d}": E})
                self.assertEqual(d["mode"], "construit")

    def test_artefacts_incomplets_ou_expires_ecartes(self):
        for arts in ({"artifacts": [{"name": lp.ARTEFACT, "expired": False}]},
                     {"artifacts": [{"name": lp.ARTEFACT, "expired": True}, {"name": lp.CORPUS, "expired": False}]},
                     {"artifacts": []}):
            with self.subTest(arts=arts):
                d = decide([artefact(1, 10)], Api({1: run(1)}, artefacts={1: arts}), {f"{1:040d}": E})
                self.assertEqual(d["mode"], "construit")

    def test_liste_filtre_nom_expiration_et_run_courant(self):
        liste = [artefact(1, 10, expired=True), artefact(2, 20, name="autre"), artefact(999, 99)]
        api = Api({1: run(1), 2: run(2), 999: run(999)})
        d = decide(liste, api, {f"{n:040d}": E for n in (1, 2, 999)})
        self.assertEqual(d["mode"], "construit")
        self.assertEqual(api.appels, [])

    def test_panne_api_sur_un_candidat_passe_au_suivant(self):
        runs = {1: run(1), 2: run(2)}
        d = decide([artefact(1, 10), artefact(2, 20)], Api(runs, pannes={2}), {f"{1:040d}": E, f"{2:040d}": E})
        self.assertEqual((d["mode"], d["source_run"]), ("reutilise", "1"))

    def test_limite_de_candidats(self):
        runs = {n: run(n) for n in range(1, 30)}
        liste = [artefact(n, n) for n in range(1, 30)]
        api = Api(runs)
        d = lp.decide("developpement", 999, E, liste, api, lambda sha: "a" * 64, DEPOT, limite=5,
                      sortie=lambda *_: None)
        self.assertEqual(d["mode"], "construit")
        self.assertEqual(len([a for a in api.appels if a.endswith("/jobs?filter=latest&per_page=100")]), 5)


if __name__ == "__main__":
    unittest.main()
