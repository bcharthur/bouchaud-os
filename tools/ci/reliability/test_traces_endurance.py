"""Le contrat producteur -> consommateur des traces d'endurance.

# Ce que ce test empeche

Le workflow d'endurance cherchait `cycle-*/serial.log` ; `qemu_matrix.run_one`
ecrivait `smp{N}.log`. Personne ne verifiait que les deux noms coincidaient,
et l'etape des budgets n'a jamais lu une trace : elle tombait sur « aucune
trace serie produite », ce qui ressemble a une panne du noyau et n'en est pas
une.

Ici, le VRAI `soak.py` pilote le VRAI `run_one` avec un faux `qemu` -- un
script qui ecrit sur la sortie serie demandee (`-serial file:...`) ce qu'un
noyau ecrirait. Puis le VRAI `budgets_endurance.py` relit la campagne. Un
renommage de la trace d'un seul cote casse ce test, pas une campagne de six
heures.

Le faux `qemu` remplace l'EMULATEUR, pas une mesure : aucun verdict de noyau
n'est tire de ce test, seulement la tuyauterie entre les etapes.
"""

from __future__ import annotations

import json
import os
import stat
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

ICI = Path(__file__).resolve().parent
sys.path.insert(0, str(ICI))

import budgets_endurance  # noqa: E402
import qemu_matrix  # noqa: E402

TRACE_COMPLETE = "\n".join([
    "ENDURANCE_MEMOIRE_OK",
    "RESULTAT : 0 verification(s) en echec (4 passees)",
    "[SCHED-NG-PIRE] classe=normale attente_ns=1000 tid=1 pid=1 cpu=0 pret_depuis_ms=1 elu_a_ms=1 coherent=1",
    "ENDURANCE_CYCLE_FIN",
    "=== AUTORUN FIN === statut=0",
]) + "\n"


def faux_qemu(dossier: Path, contenu: str, rc: int = 0) -> Path:
    """Un `qemu-system-x86_64` qui ecrit `contenu` la ou `-serial file:` le demande."""
    charge = dossier / "trace.txt"
    charge.write_text(contenu, encoding="utf-8")
    script = dossier / "qemu-system-x86_64"
    script.write_text(textwrap.dedent(f"""\
        #!{sys.executable}
        import sys
        args = sys.argv[1:]
        cible = args[args.index("-serial") + 1]
        assert cible.startswith("file:"), cible
        with open(cible[5:], "w") as f:
            f.write(open({str(charge)!r}).read())
        sys.exit({rc})
    """), encoding="utf-8")
    script.chmod(script.stat().st_mode | stat.S_IEXEC)
    return script


def faux_check_budgets(dossier: Path, rc: int, sortie: str) -> Path:
    script = dossier / "check_budgets.py"
    script.write_text(textwrap.dedent(f"""\
        import sys
        args = sys.argv[1:]
        journal = args[args.index("--journal") + 1]
        open(journal).read()  # un journal introuvable doit lever, pas passer
        print({sortie!r})
        sys.exit({rc})
    """), encoding="utf-8")
    return script


class ContratDesTraces(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = Path(self._tmp.name)
        self.image = self.tmp / "boot.bin"
        self.image.write_bytes(b"\0" * 512)
        self.disque = self.tmp / "scenario.img"
        self.disque.write_bytes(b"\0" * 512)
        self.marqueurs = self.tmp / "marqueurs.txt"
        self.marqueurs.write_text("ENDURANCE_CYCLE_FIN\n=== AUTORUN FIN === statut=0\n")

    def tearDown(self):
        self._tmp.cleanup()

    def soak(self, contenu: str, rc: int = 0, disque: bool = True) -> tuple[int, Path]:
        faux = faux_qemu(self.tmp, contenu, rc)
        sortie = self.tmp / "tranche"
        env = dict(os.environ, PATH=f"{faux.parent}{os.pathsep}{os.environ['PATH']}")
        commande = [
            sys.executable, str(ICI / "soak.py"), str(self.image),
            "--cpus", "2", "--duration-seconds", "3", "--cycle-seconds", "2",
            "--out-dir", str(sortie), "--require-fichier", str(self.marqueurs),
        ]
        if disque:
            commande += ["--disque", str(self.disque)]
        fini = subprocess.run(commande, env=env, cwd=self.tmp, capture_output=True, text=True)
        return fini.returncode, sortie

    # -- le nom de la trace ---------------------------------------------------

    def test_le_consommateur_lit_la_trace_que_le_producteur_ecrit(self):
        rc, sortie = self.soak(TRACE_COMPLETE)
        self.assertEqual(rc, 0)
        resume = json.loads((sortie / "summary.json").read_text())
        publie = Path(resume["cycles"][0]["log"])
        # Le nom publie est celui du proprietaire unique du nom.
        self.assertEqual(publie.name, qemu_matrix.journal_du_cycle(Path("x"), 2).name)
        self.assertTrue(publie.is_file(), f"trace publiee absente : {publie}")
        budgets = faux_check_budgets(self.tmp, 0, "ok  budgets tenus ; execution : 9/9 grandeur(s) mesuree(s)")
        rapport = budgets_endurance.juge(sortie, budgets)
        self.assertTrue(rapport["ok"], rapport["fautes"])
        fiche = rapport["cycles"][0]
        self.assertEqual(fiche["budgets"]["mesurees"], 9)
        self.assertEqual(fiche["noyau"]["autorun_statut"], 0)
        self.assertEqual(len(fiche["noyau"]["pire_attente"]), 1)

    def test_la_trace_se_retrouve_apres_deplacement_de_l_artefact(self):
        # Un artefact telecharge ailleurs : le chemin publie est perime, le
        # suffixe `cycle-NNNN/<nom>` sous le dossier du resume ne l'est pas.
        rc, sortie = self.soak(TRACE_COMPLETE)
        self.assertEqual(rc, 0)
        ailleurs = self.tmp / "telecharge"
        sortie.rename(ailleurs)
        budgets = faux_check_budgets(self.tmp, 0, "execution : 9/9 grandeur(s) mesuree(s)")
        rapport = budgets_endurance.juge(ailleurs, budgets)
        self.assertTrue(rapport["ok"], rapport["fautes"])

    def test_un_renommage_de_la_trace_rend_le_verdict_rouge(self):
        rc, sortie = self.soak(TRACE_COMPLETE)
        self.assertEqual(rc, 0)
        trace = next(sortie.glob("cycle-*/smp*.log"))
        trace.rename(trace.with_name("serial.log"))
        budgets = faux_check_budgets(self.tmp, 0, "ok")
        rapport = budgets_endurance.juge(sortie, budgets)
        self.assertFalse(rapport["ok"])
        self.assertTrue(any("introuvable" in f for f in rapport["fautes"]), rapport["fautes"])

    def test_le_disque_n_est_garde_que_pour_un_cycle_en_echec(self):
        rc, sortie = self.soak(TRACE_COMPLETE)
        self.assertEqual(rc, 0)
        cycle = json.loads((sortie / "summary.json").read_text())["cycles"][0]
        self.assertFalse(cycle["disque_conserve"])
        self.assertFalse(Path(cycle["disque"]).exists())

        rc, sortie = self.soak("ENDURANCE_MEMOIRE_OK\n")
        self.assertNotEqual(rc, 0)
        cycle = json.loads((sortie / "summary.json").read_text())["cycles"][0]
        self.assertTrue(cycle["disque_conserve"])
        self.assertTrue(Path(cycle["disque"]).is_file())

    # -- fail-closed ----------------------------------------------------------

    def test_sans_resume_la_campagne_est_rouge(self):
        rapport = budgets_endurance.juge(self.tmp / "rien", self.tmp / "inutile.py")
        self.assertFalse(rapport["ok"])

    def test_zero_cycle_est_rouge(self):
        dossier = self.tmp / "vide"
        dossier.mkdir()
        (dossier / "summary.json").write_text(json.dumps({"cycles": [], "disque": "x"}))
        rapport = budgets_endurance.juge(dossier, self.tmp / "inutile.py")
        self.assertFalse(rapport["ok"])

    def test_sans_disque_de_scenario_ce_n_est_pas_une_endurance(self):
        rc, sortie = self.soak(TRACE_COMPLETE, disque=False)
        self.assertEqual(rc, 0)
        budgets = faux_check_budgets(self.tmp, 0, "ok")
        rapport = budgets_endurance.juge(sortie, budgets)
        self.assertFalse(rapport["ok"])

    def test_un_marqueur_absent_rend_le_cycle_et_le_verdict_rouges(self):
        rc, sortie = self.soak("ENDURANCE_MEMOIRE_OK\n")
        self.assertNotEqual(rc, 0)
        budgets = faux_check_budgets(self.tmp, 0, "ok")
        rapport = budgets_endurance.juge(sortie, budgets)
        self.assertFalse(rapport["ok"])
        self.assertIn("ENDURANCE_CYCLE_FIN", rapport["cycles"][0]["marqueurs_absents"])

    def test_un_budget_depasse_rend_le_verdict_rouge(self):
        rc, sortie = self.soak(TRACE_COMPLETE)
        self.assertEqual(rc, 0)
        budgets = faux_check_budgets(
            self.tmp, 1, "  ready_latency_max_ms : REQUIS mais absent de x")
        rapport = budgets_endurance.juge(sortie, budgets)
        self.assertFalse(rapport["ok"])
        self.assertEqual(rapport["cycles"][0]["budgets"]["requis_absents"], ["ready_latency_max_ms"])

    def test_une_panique_rend_le_cycle_rouge(self):
        rc, sortie = self.soak(TRACE_COMPLETE + "panicked at src/kernel/test.rs:1:1\n")
        budgets = faux_check_budgets(self.tmp, 0, "ok")
        rapport = budgets_endurance.juge(sortie, budgets)
        self.assertNotEqual(rc, 0)
        self.assertFalse(rapport["ok"])

    def test_check_budgets_refuse_un_journal_introuvable(self):
        fini = subprocess.run(
            [sys.executable, str(ICI.parent / "check_budgets.py"),
             "--journal", str(self.tmp / "serial.log")],
            capture_output=True, text=True,
        )
        self.assertNotEqual(fini.returncode, 0, fini.stdout)


if __name__ == "__main__":
    unittest.main()
