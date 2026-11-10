#!/usr/bin/env python3
"""BOUCHAUD_LADYBIRD_PROVENANCE_V2 : reutiliser, attendre, produire ou construire."""
import importlib.util
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location("ladybird_provenance", ROOT / "tools" / "ci" / "ladybird_provenance.py")
lp = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(lp)

DEPOT = "bcharthur/bouchaud-os"
E = "e" * 64
AUTRE = "a" * 64


def artefact(run_id, aid, name=lp.ARTEFACT, expired=False):
    return {"id": aid, "name": name, "expired": expired, "workflow_run": {"id": run_id}}


def sha(n):
    return f"{n:040d}"


def run(rid, **kw):
    r = {"id": rid, "repository": {"full_name": DEPOT}, "head_repository": {"full_name": DEPOT},
         "path": lp.WORKFLOW, "event": "push", "head_sha": sha(rid), "created_at": "2026-11-10T10:00:00Z"}
    r.update(kw)
    return r


def production(rid, statut="in_progress", **kw):
    champs = {"event": "workflow_dispatch", "status": statut, "display_title": f"{lp.TITRE_PRODUCTION}{E}"}
    champs.update(kw)
    return run(rid, **champs)


class Api:
    def __init__(self, runs, jobs=None, artefacts=None, pannes=(), en_cours=()):
        self.runs, self.jobs, self.artefacts, self.pannes = runs, jobs or {}, artefacts or {}, set(pannes)
        self.en_cours = list(en_cours)
        self.appels = []

    def __call__(self, chemin):
        self.appels.append(chemin)
        if chemin.startswith("actions/workflows/"):
            statut = chemin.split("status=")[1].split("&")[0]
            return {"workflow_runs": [r for r in self.en_cours if r.get("status") == statut]}
        rid = int(chemin.split("/")[2].split("?")[0])
        if rid in self.pannes:
            raise RuntimeError("HTTP 502")
        if chemin.endswith(f"runs/{rid}"):
            return self.runs[rid]
        if "/jobs" in chemin:
            return self.jobs.get(rid, {"jobs": [{"name": lp.JOB_BUILD, "conclusion": "success"}]})
        return self.artefacts.get(rid, {"artifacts": [{"name": lp.ARTEFACT, "expired": False},
                                                      {"name": lp.CORPUS, "expired": False}]})


class Declencheur:
    def __init__(self, panne=None):
        self.appels, self.panne = 0, panne

    def __call__(self):
        self.appels += 1
        if self.panne:
            raise self.panne


def decide(liste, api, empreintes, palier="developpement", courant=999, **kw):
    return lp.decide(palier, courant, E, liste, api, lambda s: empreintes.get(s, "f" * 64), DEPOT,
                     sortie=lambda *_: None, **kw)


class ReutilisationTests(unittest.TestCase):
    def test_qualification_reutilise_un_build_reel_de_meme_empreinte(self):
        # BOUCHAUD_LADYBIRD_PRODUCTION_V1 : un changement noyau (meme empreinte)
        # ne recompile plus le navigateur, meme en qualification.
        d = decide([artefact(1, 10)], Api({1: run(1)}), {sha(1): E}, palier="qualification")
        self.assertEqual((d["mode"], d["source_run"]), ("reutilise", "1"))

    def test_palier_inconnu_refuse(self):
        with self.assertRaises(ValueError):
            decide([], Api({}), {}, palier="")

    def test_reutilise_le_plus_recent_aux_entrees_identiques(self):
        runs = {1: run(1), 2: run(2), 3: run(3)}
        liste = [artefact(1, 10), artefact(3, 30), artefact(2, 20)]
        d = decide(liste, Api(runs), {sha(1): E, sha(2): E, sha(3): AUTRE})
        self.assertEqual((d["mode"], d["source_run"]), ("reutilise", "2"))

    def test_empreinte_differente_jamais_reutilisee(self):
        # Un changement de source, patch, outil ou dependance du navigateur
        # change l'empreinte : l'artefact anterieur est invalide.
        for palier in ("developpement", "qualification"):
            with self.subTest(palier=palier):
                d = decide([artefact(1, 10)], Api({1: run(1)}), {sha(1): AUTRE}, palier=palier)
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
                d = decide([artefact(1, 10)], Api({1: r}), {sha(1): E, None: E})
                self.assertEqual(d["mode"], "construit")

    def test_build_non_reussi_ou_absent_ecarte(self):
        for jobs in ({"jobs": [{"name": lp.JOB_BUILD, "conclusion": "failure"}]},
                     {"jobs": [{"name": lp.JOB_BUILD, "conclusion": "skipped"}]},
                     {"jobs": [{"name": "ladybird / artefact reutilise (entrees identiques)", "conclusion": "success"}]},
                     {"jobs": []}):
            with self.subTest(jobs=jobs):
                d = decide([artefact(1, 10)], Api({1: run(1)}, jobs={1: jobs}), {sha(1): E})
                self.assertEqual(d["mode"], "construit")

    def test_artefacts_incomplets_ou_expires_ecartes(self):
        for arts in ({"artifacts": [{"name": lp.ARTEFACT, "expired": False}]},
                     {"artifacts": [{"name": lp.ARTEFACT, "expired": True}, {"name": lp.CORPUS, "expired": False}]},
                     {"artifacts": []}):
            with self.subTest(arts=arts):
                d = decide([artefact(1, 10)], Api({1: run(1)}, artefacts={1: arts}), {sha(1): E})
                self.assertEqual(d["mode"], "construit")

    def test_liste_filtre_nom_expiration_et_run_courant(self):
        liste = [artefact(1, 10, expired=True), artefact(2, 20, name="autre"), artefact(999, 99)]
        api = Api({1: run(1), 2: run(2), 999: run(999)})
        d = decide(liste, api, {sha(n): E for n in (1, 2, 999)})
        self.assertEqual(d["mode"], "construit")
        self.assertEqual([a for a in api.appels if a.startswith("actions/runs/")], [])

    def test_panne_api_sur_un_candidat_passe_au_suivant(self):
        runs = {1: run(1), 2: run(2)}
        d = decide([artefact(1, 10), artefact(2, 20)], Api(runs, pannes={2}), {sha(1): E, sha(2): E})
        self.assertEqual((d["mode"], d["source_run"]), ("reutilise", "1"))

    def test_empreinte_comparee_avant_jobs_et_artefacts(self):
        runs = {n: run(n) for n in range(1, 30)}
        api = Api(runs)
        lp.cherche_artefact(999, E, [artefact(n, n) for n in range(1, 30)], api, lambda s: AUTRE, DEPOT,
                            limite=5, sortie=lambda *_: None)
        self.assertEqual(len([a for a in api.appels if a.startswith("actions/runs/")]), 5)
        self.assertEqual([a for a in api.appels if "/jobs" in a or "/artifacts" in a], [])

    def test_empreinte_calculee_une_fois_par_commit(self):
        vus = []
        runs = {n: run(n, head_sha=sha(1)) for n in range(1, 6)}
        lp.decide("developpement", 999, E, [artefact(n, n) for n in range(1, 6)], Api(runs),
                  lambda s: vus.append(s) or AUTRE, DEPOT, sortie=lambda *_: None)
        self.assertEqual(vus, [sha(1)])


class ProductionTests(unittest.TestCase):
    def test_production_en_cours_de_meme_empreinte_attendue_sans_redeclencher(self):
        dec = Declencheur()
        api = Api({}, en_cours=[production(50)])
        d = decide([], api, {sha(50): E}, palier="qualification", declenche=dec)
        self.assertEqual(d["mode"], "attendu")
        self.assertEqual(dec.appels, 0)

    def test_production_d_une_autre_empreinte_ignoree(self):
        dec = Declencheur()
        d = decide([], Api({}, en_cours=[production(50)]), {sha(50): AUTRE}, declenche=dec)
        self.assertEqual((d["mode"], dec.appels), ("attendu", 1))

    def test_titre_de_production_jamais_cru(self):
        # Titre de production, mais run d'un fork ou d'une pull_request : ignore.
        for faux in (production(50, event="pull_request"),
                     production(50, head_repository={"full_name": "pirate/bouchaud-os"})):
            with self.subTest(faux=faux.get("event")):
                dec = Declencheur()
                d = decide([], Api({}, en_cours=[faux]), {sha(50): E}, declenche=dec)
                self.assertEqual(dec.appels, 1)
                self.assertEqual(d["mode"], "attendu")

    def test_aucune_production_declenche_et_attend(self):
        dec = Declencheur()
        d = decide([], Api({}), {}, palier="qualification", declenche=dec)
        self.assertEqual((d["mode"], dec.appels), ("attendu", 1))

    def test_declenchement_impossible_repli_build_dans_le_run(self):
        for dec in (Declencheur(RuntimeError("HTTP 403")), None):
            with self.subTest(dec=dec):
                d = decide([], Api({}), {}, declenche=dec)
                self.assertEqual(d["mode"], "construit")

    def test_reconstruire_ignore_tout_artefact_et_toute_production_anterieure(self):
        dec = Declencheur()
        api = Api({1: run(1)}, en_cours=[production(50)])
        d = decide([artefact(1, 10)], api, {sha(1): E, sha(50): E}, palier="qualification",
                   reconstruire=True, declenche=dec)
        self.assertEqual((d["mode"], dec.appels), ("attendu", 1))
        self.assertEqual([a for a in api.appels if a.startswith("actions/runs/")], [])

    def test_palier_artefact(self):
        api = Api({1: run(1)})
        self.assertEqual(decide([artefact(1, 10)], api, {sha(1): E}, palier="artefact")["mode"], "existe")
        self.assertEqual(decide([], Api({}), {}, palier="artefact")["mode"], "construit")
        self.assertEqual(decide([artefact(1, 10)], Api({1: run(1)}), {sha(1): E}, palier="artefact",
                                reconstruire=True)["mode"], "construit")

    def test_production_ne_redeclenche_jamais(self):
        dec = Declencheur()
        decide([], Api({}), {}, palier="artefact", declenche=dec)
        self.assertEqual(dec.appels, 0)


class ApiProduction(Api):
    """Une production (id 7) qui se termine apres `tours_restants` lectures de son etat."""

    def __init__(self, tours, conclusion="success", pannes_transitoires=0, **kw):
        prod = production(7)
        super().__init__({7: prod}, en_cours=[prod], **kw)
        self.prod, self.tours, self.conclusion = prod, tours, conclusion
        self.pannes_transitoires = pannes_transitoires

    def __call__(self, chemin):
        if self.pannes_transitoires and chemin == "actions/runs/7":
            self.pannes_transitoires -= 1
            self.appels.append(chemin)
            raise RuntimeError("HTTP 403 rate limit")
        if chemin == "actions/runs/7" and self.prod["status"] != "completed":
            self.tours -= 1
            if self.tours <= 0:
                self.prod.update(status="completed", conclusion=self.conclusion)
        return super().__call__(chemin)


class AttenteTests(unittest.TestCase):
    def horloge(self):
        t = [0]

        def h():
            return t[0]

        def dort(s):
            t[0] += s
        return h, dort

    def attends(self, api, liste=lambda: [], **kw):
        h, dort = self.horloge()
        return lp.attends(999, E, api, liste, lambda s: E, DEPOT, horloge=h, dort=dort,
                          sortie=lambda *_: None, **kw)

    def test_artefact_deja_present_retourne_sans_attendre(self):
        api = Api({1: run(1)})
        self.assertEqual(self.attends(api, lambda: [artefact(1, 10)])[0], 1)

    def test_retourne_quand_la_production_suivie_publie(self):
        api = ApiProduction(tours=5)
        rid, raison = self.attends(api)
        self.assertEqual(rid, 7, raison)

    def test_budget_d_api_borne_pendant_un_build_long(self):
        # Revue de la PR #282 : 46 requetes par tour epuisaient 1 000 req/h.
        # Build de 150 min, un tour par minute : ~1 requete par tour, la liste
        # des productions relue un tour sur dix (5 requetes).
        api = ApiProduction(tours=150)
        self.assertEqual(self.attends(api)[0], 7)
        par_heure = len(api.appels) / 2.5
        self.assertLess(par_heure, 200, f"{len(api.appels)} appels en 150 min")

    def test_production_echouee_puis_plus_rien_echoue(self):
        api = ApiProduction(tours=3, conclusion="failure")
        with self.assertRaises(RuntimeError):
            self.attends(api, grace=0)

    def test_panne_d_api_transitoire_toleree(self):
        api = ApiProduction(tours=5, pannes_transitoires=4)
        self.assertEqual(self.attends(api)[0], 7)

    def test_production_reussie_mais_ineligible_ecartee(self):
        # Termine en succes, mais sans artefacts publies : jamais retenue.
        api = ApiProduction(tours=2, artefacts={7: {"artifacts": []}})
        with self.assertRaises(RuntimeError):
            self.attends(api, grace=0)

    def test_aucune_production_echoue_apres_la_grace(self):
        with self.assertRaises(RuntimeError):
            self.attends(Api({}), grace=120)

    def test_reconstruire_refuse_une_production_anterieure_au_run(self):
        api = ApiProduction(tours=2)
        api.prod["created_at"] = "2026-11-10T09:00:00Z"
        with self.assertRaises(RuntimeError):
            self.attends(api, depuis="2026-11-10T11:00:00Z", grace=0)
        api2 = ApiProduction(tours=2)
        api2.prod["created_at"] = "2026-11-10T12:00:00Z"
        self.assertEqual(self.attends(api2, depuis="2026-11-10T11:00:00Z")[0], 7)

    def test_delai_borne(self):
        api = ApiProduction(tours=10 ** 6)
        with self.assertRaises(RuntimeError):
            self.attends(api, delai=600, pas=60)


class VerdictProductionTests(unittest.TestCase):
    def v(self, **kw):
        e = {"P_PROVENANCE": "success", "P_BUILD": "success", "P_ARTEFACT": "skipped", "P_MODE": "construit"}
        e.update(kw)
        return lp.verdict_production(e)[0]

    def test_verts(self):
        self.assertTrue(self.v())
        self.assertTrue(self.v(P_MODE="existe", P_BUILD="skipped"))

    def test_rouges(self):
        for kw in ({"P_PROVENANCE": "failure"}, {"P_BUILD": "failure"}, {"P_BUILD": "cancelled"},
                   {"P_BUILD": "skipped"}, {"P_MODE": "existe"}, {"P_ARTEFACT": "success"},
                   {"P_MODE": "reutilise", "P_BUILD": "skipped"}, {"P_MODE": ""}, {"P_PROVENANCE": ""}):
            with self.subTest(kw=kw):
                self.assertFalse(self.v(**kw))


if __name__ == "__main__":
    unittest.main()
