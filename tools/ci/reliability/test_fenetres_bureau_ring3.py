"""BOUCHAUD_BUREAU_RING3_2B : l'etat des fenetres relu dans le journal de `composited`.

Le juge de capture ne vaut que si on lui donne les BONNES fenetres, dans le
bon ordre, a l'instant de la capture : apparition, scenario, mort d'un
client, et lignes du noyau intercalees dans un bloc.
"""
import importlib.util
import unittest
from pathlib import Path

RACINE = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("fbr", RACINE / "tools/ci/fenetres_bureau_ring3.py")
fbr = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fbr)


def bloc(tete, pid, x, y, rang):
    return [tete, f"[COMPOSITED] pid={pid}", f"[COMPOSITED] x={x}", f"[COMPOSITED] y={y}",
            "[COMPOSITED] l=640", "[COMPOSITED] h=400", f"[COMPOSITED] rang={rang}"]


JOURNAL = (
    ["\x1b[90m[12:00:00]\x1b[0m COMPOSITED_BUREAU_PRET fenetres=2"]
    + bloc("COMPOSITED_BUREAU_FENETRE indice=0", 16, 40, 66, 0)
    + bloc("COMPOSITED_BUREAU_FENETRE indice=1", 17, 460, 256, 1)
    + ["COMPOSITED_BUREAU_SCENARIO debut deplacement"]
    + bloc("COMPOSITED_BUREAU_SCENARIO_FIN fenetre=0", 16, 200, 146, 1)
    + bloc("COMPOSITED_BUREAU_SCENARIO_FIN fenetre=1", 17, 460, 256, 0)
    + ["COMPOSITED_BUREAU_CLIENT_MORT pid=16", "COMPOSITED_BUREAU_CLIENT_MORT pid=17", "COMPOSITED_BUREAU_OK"]
)


class EtatFenetresTests(unittest.TestCase):
    def test_apparition(self):
        self.assertEqual(fbr.etat(JOURNAL, "COMPOSITED_BUREAU_FENETRE indice=1"),
                         [(40, 66, 640, 400), (460, 256, 640, 400)])

    def test_apres_scenario_l_ordre_est_inverse(self):
        self.assertEqual(fbr.etat(JOURNAL, "COMPOSITED_BUREAU_SCENARIO_FIN fenetre=1"),
                         [(460, 256, 640, 400), (200, 146, 640, 400)])

    def test_mort_retire_la_fenetre(self):
        self.assertEqual(fbr.etat(JOURNAL, "COMPOSITED_BUREAU_CLIENT_MORT pid=16"), [(460, 256, 640, 400)])
        self.assertEqual(fbr.etat(JOURNAL, None), [])

    def test_minimise_puis_restaure(self):
        j = (bloc("COMPOSITED_BUREAU_FENETRE indice=0", 16, 40, 66, 0)
             + bloc("COMPOSITED_BUREAU_FENETRE indice=1", 17, 460, 256, 1)
             + ["COMPOSITED_BUREAU_MINIMISE fenetre=0"]
             + bloc("COMPOSITED_BUREAU_RESTAURE fenetre=0", 16, 40, 66, 0)
             + ["COMPOSITED_BUREAU_ETAPE fermeture fenetre=1"])
        self.assertEqual(fbr.etat(j, "COMPOSITED_BUREAU_MINIMISE fenetre=0"), [(460, 256, 640, 400)])
        self.assertEqual(fbr.etat(j, "COMPOSITED_BUREAU_RESTAURE fenetre=0"),
                         [(40, 66, 640, 400), (460, 256, 640, 400)])

    def test_ligne_du_noyau_intercalee_ne_coupe_pas_le_bloc(self):
        j = list(JOURNAL)
        i = j.index("[COMPOSITED] x=460")
        j.insert(i, "[SCHED-RESUME] coeurs=4 au_repos=3")
        self.assertEqual(fbr.etat(j, "COMPOSITED_BUREAU_FENETRE indice=1"),
                         [(40, 66, 640, 400), (460, 256, 640, 400)])


if __name__ == "__main__":
    unittest.main()
