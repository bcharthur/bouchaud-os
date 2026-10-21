"""BOUCHAUD_CI_UN_BUILD_PAR_SHA_V2 : un run PR ne reutilise que ce qui est
PROUVE. Chaque etat du run de reference est essaye, y compris les erreurs
d'API, les reponses incompletes et les resultats arrivant apres la decision."""
import importlib.util
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
TETE = "c" * 40
ARBRE = "a" * 40
CHEMIN = ".github/workflows/ladybird-native-browser.yml"
VERDICT = "ladybird / verdict de convergence"


def run(statut="completed", conclusion="success", **kw):
    r = {"id": 7, "status": statut, "conclusion": conclusion, "event": "push", "head_sha": TETE,
         "path": CHEMIN, "run_attempt": 1}
    r.update(kw)
    return r


def decide(source=run(), **kw):
    args = dict(evenement="pull_request", action="synchronize", depot=DEPOT, depot_tete=DEPOT,
                arbre_tete=ARBRE, arbre_fusion=ARBRE, source=source)
    args.update(kw)
    return dd.decide(**args)[0]


class Decision(unittest.TestCase):
    def test_en_cours_reutilise_et_le_verdict_attendra(self):
        for statut in ("queued", "in_progress", "waiting", "requested", "pending"):
            self.assertTrue(decide(run(statut=statut, conclusion=None)), statut)

    def test_succes_ou_echec_se_refletent(self):
        self.assertTrue(decide(run(conclusion="success")))
        self.assertTrue(decide(run(conclusion="failure")))  # le verdict transmettra l'echec

    def test_ce_qui_n_est_pas_une_preuve_fait_tourner_les_tests(self):
        for conclusion in ("cancelled", "skipped", "timed_out", "neutral", "stale", "action_required",
                           "startup_failure", None):
            self.assertFalse(decide(run(conclusion=conclusion)), conclusion)
        self.assertFalse(decide(run(statut="bizarre", conclusion=None)))

    def test_conditions_d_arbre_et_d_origine(self):
        self.assertFalse(decide(source=None))
        self.assertFalse(decide(arbre_fusion="b" * 40))
        self.assertFalse(decide(arbre_tete=None))
        self.assertFalse(decide(depot_tete="autre/bouchaud-os"))
        self.assertFalse(decide(action="labeled"))
        for evt in ("push", "schedule", "workflow_dispatch", ""):
            self.assertFalse(decide(evenement=evt))


class ChoixDeLaSource(unittest.TestCase):
    def test_le_plus_recent_du_meme_workflow_meme_tete(self):
        runs = [run(id=5), run(id=9), run(id=12, path=".github/workflows/ci.yml"),
                run(id=13, head_sha="d" * 40), run(id=14, event="pull_request"), {"id": "x"}, None]
        self.assertEqual(dd.choisit_source(runs, TETE, CHEMIN)["id"], 9)
        self.assertIsNone(dd.choisit_source([], TETE, CHEMIN))


class FausseApi:
    """Rejoue une sequence de reponses ; une Exception dans la sequence est levee."""
    def __init__(self, runs, jobs=None):
        self.runs = list(runs)
        self.jobs = jobs if jobs is not None else {"jobs": [{"id": 1, "name": VERDICT, "status": "completed",
                                                             "conclusion": "success"}]}
        self.t = 0

    def __call__(self, chemin):
        if "/jobs" in chemin:
            j = self.jobs(chemin) if callable(self.jobs) else self.jobs
            if isinstance(j, Exception):
                raise j
            return j
        r = self.runs.pop(0) if len(self.runs) > 1 else self.runs[0]
        if isinstance(r, Exception):
            raise r
        return r

    def dormir(self, s):
        self.t += s

    def maintenant(self):
        return self.t


def attend(api, delai=600):
    return dd.attend(api, api.dormir, api.maintenant, 7, TETE, CHEMIN, VERDICT, delai, 60, sortie=lambda *_: None)


class Reflet(unittest.TestCase):
    def test_attend_la_fin_puis_reflete_le_succes(self):
        ok, code, _ = attend(FausseApi([run("queued", None), run("in_progress", None), run()]))
        self.assertTrue(ok)
        self.assertEqual(code, "SOURCE_PROUVEE")

    def test_un_echec_du_push_bloque(self):
        ok, code, _ = attend(FausseApi([run("in_progress", None), run(conclusion="failure")]))
        self.assertEqual((ok, code), (False, "SOURCE_ECHEC"))

    def test_annule_saute_expire_neutre_ne_prouvent_rien(self):
        for conclusion, attendu in (("cancelled", "SOURCE_ANNULEE"), ("skipped", "SOURCE_SAUTEE"),
                                    ("timed_out", "SOURCE_EXPIREE"), ("neutral", "SOURCE_NON_PROUVEE"),
                                    ("stale", "SOURCE_NON_PROUVEE")):
            ok, code, _ = attend(FausseApi([run(conclusion=conclusion)]))
            self.assertEqual((ok, code), (False, attendu), conclusion)

    def test_succes_decide_puis_relance_en_echec(self):
        # Resultat arrivant APRES la decision : le run reussi est relance et echoue.
        ok, code, _ = attend(FausseApi([run("in_progress", None, run_attempt=2), run(conclusion="failure", run_attempt=2)]))
        self.assertEqual((ok, code), (False, "SOURCE_ECHEC"))

    def test_job_de_verdict_saute_absent_ou_rouge(self):
        for jobs, attendu in (({"jobs": [{"id": 1, "name": VERDICT, "status": "completed", "conclusion": "skipped"}]},
                               "VERDICT_NON_PROUVE"),
                              ({"jobs": [{"id": 1, "name": VERDICT, "status": "completed", "conclusion": "failure"}]},
                               "VERDICT_NON_PROUVE"),
                              ({"jobs": [{"id": 1, "name": "autre", "status": "completed", "conclusion": "success"}]},
                               "VERDICT_ABSENT")):
            ok, code, _ = attend(FausseApi([run()], jobs=jobs))
            self.assertEqual((ok, code), (False, attendu))

    def test_source_incoherente(self):
        for mauvais in (run(head_sha="d" * 40), run(event="pull_request"), run(path=".github/workflows/ci.yml")):
            ok, code, _ = attend(FausseApi([mauvais]))
            self.assertEqual((ok, code), (False, "SOURCE_INCOHERENTE"))

    def test_erreurs_d_api_passageres_puis_succes(self):
        ok, code, _ = attend(FausseApi([OSError("reseau"), ValueError("json"), {"id": 7}, run()]))
        self.assertTrue(ok, code)

    def test_api_indisponible_jusqu_au_bout(self):
        ok, code, _ = attend(FausseApi([OSError("reseau")]), delai=300)
        self.assertEqual((ok, code), (False, "API_INDISPONIBLE"))

    def test_jamais_termine(self):
        ok, code, _ = attend(FausseApi([run("in_progress", None)]), delai=300)
        self.assertEqual((ok, code), (False, "DELAI"))

    def test_pagination_sans_fin_ne_boucle_pas(self):
        pleine = {"jobs": [{"id": i, "name": f"j{i}", "status": "completed", "conclusion": "success"} for i in range(100)]}
        ok, code, _ = attend(FausseApi([run()], jobs=pleine), delai=300)
        self.assertEqual((ok, code), (False, "API_INDISPONIBLE"))

    def test_reponse_jobs_incomplete_puis_complete(self):
        etat = {"n": 0}

        def jobs(_):
            etat["n"] += 1
            return {"total_count": 1} if etat["n"] == 1 else {
                "jobs": [{"id": 1, "name": VERDICT, "status": "completed", "conclusion": "success"}]}
        ok, _, _ = attend(FausseApi([run()], jobs=jobs))
        self.assertTrue(ok)

    def test_verdict_en_seconde_page(self):
        def jobs(chemin):
            if chemin.endswith("&page=1"):
                return {"jobs": [{"id": i, "name": f"j{i}", "status": "completed", "conclusion": "success"}
                                 for i in range(100)]}
            return {"jobs": [{"id": 500, "name": VERDICT, "status": "completed", "conclusion": "success"}]}
        ok, _, _ = attend(FausseApi([run()], jobs=jobs))
        self.assertTrue(ok)


class Repli(unittest.TestCase):
    def _lance(self, args, env):
        with tempfile.TemporaryDirectory() as d:
            sortie = Path(d) / "out"
            env = dict(env, PATH=d, GITHUB_OUTPUT=str(sortie))
            r = subprocess.run([sys.executable, str(SCRIPT)] + args, env=env, capture_output=True, text=True)
            return r, (sortie.read_text() if sortie.exists() else "")

    def test_decision_sans_api_execute_les_tests(self):
        r, out = self._lance([], {"EVENEMENT": "pull_request", "DEPOT": DEPOT, "TETE": TETE, "FUSION": "f" * 40,
                                  "WORKFLOW_REF": f"{DEPOT}/{CHEMIN}@refs/pull/226/merge"})
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("reutilise=false", out)

    def test_reflet_sans_parametres_est_rouge(self):
        r, _ = self._lance(["attend", "--run", "", "--job", VERDICT], {"DEPOT": DEPOT})
        self.assertEqual(r.returncode, 1)

    def test_chemin_du_workflow(self):
        self.assertEqual(dd.chemin_du_workflow(f"{DEPOT}/{CHEMIN}@refs/pull/226/merge"), CHEMIN)


if __name__ == "__main__":
    unittest.main()
