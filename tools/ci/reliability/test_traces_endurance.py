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
import etat_cycle  # noqa: E402
import logscan  # noqa: E402
import qemu_matrix  # noqa: E402

FIN_OK = "=== AUTORUN FIN === statut=0"

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


PREFIXE = "[22:10:30][ 97%:  0%:  2%][FPS: --] "
LIGNE_SONDE = "/bin/ordonnanceur-probe 8 && echo ENDURANCE_ORDONNANCEUR_OK"


def trace_sonde(verdict: str, statut: int, autres_ok: bool = True, emis: bool = False) -> str:
    """Un cycle ou la famille de la sonde est echoee (« + ... ») puis jugee."""
    lignes = [
        PREFIXE + "+ echo ENDURANCE_MEMOIRE_DEBUT && /bin/mmstress 4 512 4 && echo ENDURANCE_MEMOIRE_OK",
    ]
    if autres_ok:
        lignes.append(PREFIXE + "ENDURANCE_MEMOIRE_OK")
    lignes += [
        PREFIXE + "+ echo ENDURANCE_ORDONNANCEUR_DEBUT && " + LIGNE_SONDE,
        PREFIXE + "ENDURANCE_ORDONNANCEUR_DEBUT",
        PREFIXE + f"VERDICT_SONDE : {verdict} tentatives=3 raison=phase non jugeable (126 reveils)",
    ]
    if emis:
        lignes.append(PREFIXE + "ENDURANCE_ORDONNANCEUR_OK")
    lignes += [
        PREFIXE + "+ echo ENDURANCE_CYCLE_FIN",
        PREFIXE + "ENDURANCE_CYCLE_FIN",
        f"=== AUTORUN FIN === statut={statut}",
    ]
    return "\n".join(lignes) + "\n"


class MarqueursEmis(unittest.TestCase):
    """BOUCHAUD_MARQUEUR_HORS_ECHO_V1 : l'echo d'une commande n'est pas un marqueur."""

    def test_un_marqueur_seulement_echoe_est_absent(self):
        _, absents = logscan.scan_text(trace_sonde("NON_QUALIFIANT", 2),
                                       required_markers=["ENDURANCE_ORDONNANCEUR_OK"])
        self.assertEqual(absents, ("ENDURANCE_ORDONNANCEUR_OK",))

    def test_un_marqueur_emis_est_present(self):
        _, absents = logscan.scan_text(trace_sonde("QUALIFIE_OK", 0, emis=True),
                                       required_markers=["ENDURANCE_ORDONNANCEUR_OK", "ENDURANCE_MEMOIRE_OK"])
        self.assertEqual(absents, ())

    def test_l_echo_colore_est_reconnu(self):
        colore = ("\x1b[90m[\x1b[36m18:35:57\x1b[90m][\x1b[33m 50\x1b[90m%:\x1b[32m  0\x1b[90m%:"
                  "\x1b[32m  1\x1b[90m%][FPS:\x1b[90m --\x1b[90m] \x1b[0m+ " + LIGNE_SONDE)
        self.assertTrue(logscan.est_echo_de_commande(colore))
        self.assertTrue(logscan.est_echo_de_commande("+ " + LIGNE_SONDE))
        self.assertFalse(logscan.est_echo_de_commande(PREFIXE + "ENDURANCE_ORDONNANCEUR_OK"))
        self.assertFalse(logscan.est_echo_de_commande("=== AUTORUN FIN === statut=0"))


class CycleNonQualifiant(unittest.TestCase):
    """BOUCHAUD_ENDURANCE_NON_QUALIFIANT_V1 : ni reussite, ni echec du noyau."""

    MARQUEURS = ["ENDURANCE_MEMOIRE_OK", "ENDURANCE_ORDONNANCEUR_OK", "ENDURANCE_CYCLE_FIN", FIN_OK]

    def resultat(self, texte: str, timed_out: bool = False, fatals: int = 0) -> dict:
        _, absents = logscan.scan_text(texte, required_markers=self.MARQUEURS)
        return {"ok": not absents and not fatals and not timed_out, "timed_out": timed_out,
                "qemu_exit_ok": True, "returncode": 0,
                "fatal_findings": [{"kind": "kernel_panic"}] * fatals,
                "required_markers_missing": list(absents)}

    def etat(self, texte: str, **k) -> str:
        return etat_cycle.etat_du_cycle(self.resultat(texte, **k), texte, "ENDURANCE_ORDONNANCEUR_OK")[0]

    def test_sonde_non_qualifiante_seule(self):
        texte = trace_sonde("NON_QUALIFIANT", 2)
        etat, raison = etat_cycle.etat_du_cycle(self.resultat(texte), texte, "ENDURANCE_ORDONNANCEUR_OK")
        self.assertEqual(etat, "NON_QUALIFIANT")
        self.assertIn("126 reveils", raison)

    def test_un_echec_qualifie_reste_un_echec(self):
        self.assertEqual(self.etat(trace_sonde("QUALIFIE_ECHEC", 1)), "ECHEC")

    def test_une_autre_famille_en_echec_reste_un_echec(self):
        self.assertEqual(self.etat(trace_sonde("NON_QUALIFIANT", 2, autres_ok=False)), "ECHEC")

    def test_une_panique_reste_un_echec(self):
        self.assertEqual(self.etat(trace_sonde("NON_QUALIFIANT", 2), fatals=1), "ECHEC")

    def test_un_cycle_coupe_reste_un_echec(self):
        self.assertEqual(self.etat(trace_sonde("NON_QUALIFIANT", 2), timed_out=True), "ECHEC")

    def test_un_statut_autre_que_2_reste_un_echec(self):
        self.assertEqual(self.etat(trace_sonde("NON_QUALIFIANT", 3)), "ECHEC")

    def test_sans_sonde_designee_tout_rate_est_un_echec(self):
        texte = trace_sonde("NON_QUALIFIANT", 2)
        self.assertEqual(etat_cycle.etat_du_cycle(self.resultat(texte), texte, None)[0], "ECHEC")

    def test_verdict_de_campagne(self):
        v = etat_cycle.verdict_campagne
        self.assertEqual(v(["OK", "OK"]), "OK")
        self.assertEqual(v(["OK", "NON_QUALIFIANT"]), "NON_QUALIFIANT")
        self.assertEqual(v(["NON_QUALIFIANT", "ECHEC"]), "ECHEC")
        self.assertEqual(v([]), "ECHEC")
        self.assertEqual([etat_cycle.code_de_sortie(x) for x in ("OK", "NON_QUALIFIANT", "ECHEC")], [0, 2, 1])


class CampagneNonQualifiante(unittest.TestCase):
    """Le vrai `soak.py` puis le vrai `budgets_endurance.py` sur des cycles NON_QUALIFIANT."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = Path(self._tmp.name)
        self.image = self.tmp / "boot.bin"
        self.image.write_bytes(b"\0" * 512)
        self.disque = self.tmp / "scenario.img"
        self.disque.write_bytes(b"\0" * 512)
        self.marqueurs = self.tmp / "marqueurs.txt"
        self.marqueurs.write_text("\n".join(CycleNonQualifiant.MARQUEURS) + "\n")

    def tearDown(self):
        self._tmp.cleanup()

    def soak_sonde(self, contenu: str) -> tuple[int, Path, dict]:
        faux = faux_qemu(self.tmp, contenu, 0)
        sortie = self.tmp / "tranche"
        env = dict(os.environ, PATH=f"{faux.parent}{os.pathsep}{os.environ['PATH']}")
        fini = subprocess.run([
            sys.executable, str(ICI / "soak.py"), str(self.image),
            "--cpus", "8", "--duration-seconds", "5", "--cycle-seconds", "2",
            "--out-dir", str(sortie), "--require-fichier", str(self.marqueurs),
            "--disque", str(self.disque), "--marqueur-sonde", "ENDURANCE_ORDONNANCEUR_OK",
        ], env=env, cwd=self.tmp, capture_output=True, text=True)
        return fini.returncode, sortie, json.loads((sortie / "summary.json").read_text())

    def test_la_campagne_continue_et_rend_non_qualifiant(self):
        rc, sortie, resume = self.soak_sonde(trace_sonde("NON_QUALIFIANT", 2))
        self.assertEqual(rc, 2)
        # Le faux qemu rend la main tout de suite : bien plus de deux cycles.
        self.assertGreaterEqual(len(resume["cycles"]), 2, "la campagne s'est arretee au premier cycle")
        self.assertEqual(resume["non_qualifiants"], len(resume["cycles"]))
        self.assertEqual(resume["verdict"], "NON_QUALIFIANT")
        self.assertFalse(resume["ok"])
        budgets = faux_check_budgets(self.tmp, 0, "execution : 9/9 grandeur(s) mesuree(s)")
        rapport = budgets_endurance.juge(sortie, budgets)
        self.assertEqual(rapport["verdict"], "NON_QUALIFIANT")
        self.assertFalse(rapport["ok"])
        self.assertEqual(rapport["fautes"], [])
        self.assertTrue(all("NON_QUALIFIANT" in f for f in rapport["non_qualifies"]))
        self.assertNotIn("noyau", " ".join(rapport["non_qualifies"]).split("--")[0])

    def test_un_budget_depasse_dans_un_cycle_non_qualifiant_est_un_echec(self):
        _, sortie, _ = self.soak_sonde(trace_sonde("NON_QUALIFIANT", 2))
        budgets = faux_check_budgets(self.tmp, 1, "  ready_latency_max_ms : REQUIS mais absent de x")
        rapport = budgets_endurance.juge(sortie, budgets)
        self.assertEqual(rapport["verdict"], "ECHEC")

    def test_un_echec_qualifie_arrete_la_campagne(self):
        rc, _, resume = self.soak_sonde(trace_sonde("QUALIFIE_ECHEC", 1))
        self.assertEqual(rc, 1)
        self.assertEqual(len(resume["cycles"]), 1)
        self.assertEqual(resume["verdict"], "ECHEC")


class DetailsSondes(unittest.TestCase):
    """BOUCHAUD_ATTRIBUTION_REVEIL_V1 : un echec de latence garde l'attribution."""

    def test_l_attribution_accompagne_les_latences(self):
        prefixe = "[14:18:44][ 34%:  0%:  1%][FPS: --] "
        journal = "\n".join([
            prefixe + "  sous charge (interactif) median  1062 us  p95   1569 us  p99   1855 us  pire   9333 us",
            prefixe + "  attribution sous charge (interactif) : reveils=300 pire_total_us=9301 tardif_us=9026 "
                      "attente_us=237 reveille_par=services-metrics:pid=11:tid=107:noyau=1:irq=0",
            prefixe + "  - et n'aggrave pas le pire cas au-dela du bruit au repos : pire sans 5284 / avec 9333 us",
            prefixe + "RESULTAT : 1 verification(s) en echec (3 passees)",
        ])
        blocs = budgets_endurance.details_sondes(journal)
        self.assertEqual(len(blocs), 1)
        self.assertEqual(len(blocs[0]), 3)
        self.assertTrue(blocs[0][1].startswith("attribution sous charge (interactif) : reveils=300"))
        self.assertIn("reveille_par=services-metrics", blocs[0][1])

    def test_une_sonde_non_qualifiante_n_est_pas_une_reussite(self):
        """BOUCHAUD_SONDE_ORDO_QUALIFICATION_V1 : le temoin contamine du run
        38569175760 ne donne plus « 0 echec » mais un NON_QUALIFIANT, avec ses
        preuves."""
        journal = "\n".join([
            "  au repos                 median  1323 us  p95   1935 us  p99  10920 us  pire  24465 us",
            "  ventilation au repos : seuil_us=1000 ventiles=12 contamines=6 incompletes=0 "
            "livraison_externe=6:21000:3900 liste=1000016:3900",
            "  PREUVE la charge degrade la latence (porte) : tout[p95 sous charge 1127 > p95 repos 1935"
            " + plancher 2] = faux ; propre[...] = faux ; contamines repos=6 normal=0 interactif=1 sur 300"
            " -> QUALIFIE_ECHEC",
            "  tentative 3/3 : NON_QUALIFIANT : charge sans effet mesurable",
            "VERDICT_SONDE : NON_QUALIFIANT tentatives=3 raison=charge sans effet mesurable",
            "RESULTAT : NON_QUALIFIANT 0 verification(s) en echec, 2 non qualifiante(s) (2 passees)",
        ])
        resultats = budgets_endurance.RESULTAT.findall(journal)
        self.assertEqual(resultats, [("NON_QUALIFIANT ", "0", "2", "2")])
        blocs = budgets_endurance.details_sondes(journal)
        self.assertEqual(len(blocs), 1)
        self.assertTrue(any(l.startswith("PREUVE ") for l in blocs[0]))
        self.assertTrue(any(l.startswith("ventilation au repos") for l in blocs[0]))
        self.assertTrue(any(l.startswith("VERDICT_SONDE : NON_QUALIFIANT") for l in blocs[0]))

    def test_l_ancienne_forme_reste_lue(self):
        journal = "RESULTAT : 0 verification(s) en echec (4 passees)"
        self.assertEqual(budgets_endurance.RESULTAT.findall(journal), [("", "0", "", "4")])

    def test_une_ligne_quelconque_n_est_pas_une_attribution(self):
        journal = "  attribution manquante\nRESULTAT : 0 verification(s) en echec (4 passees)"
        self.assertEqual(budgets_endurance.details_sondes(journal), [[]])



if __name__ == "__main__":
    unittest.main()
