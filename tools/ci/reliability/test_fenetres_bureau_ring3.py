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

    def test_trois_fenetres_pile_republiee_entiere(self):
        # Run 43088438133 : seules 0 et 1 etaient republiees apres `remonte(0)` ;
        # la troisieme gardait son rang 2 perime, ex aequo avec la fenetre
        # remontee, et le juge l'empilait au-dessus. Le compositeur republie
        # desormais TOUTES les fenetres visibles, puis
        # COMPOSITED_BUREAU_SCENARIO_PILE_PUBLIEE.
        j = (bloc("COMPOSITED_BUREAU_FENETRE indice=0", 16, 40, 66, 0)
             + bloc("COMPOSITED_BUREAU_FENETRE indice=1", 17, 460, 256, 1)
             + bloc("COMPOSITED_BUREAU_FENETRE indice=2", 18, 638, 318, 2)
             + bloc("COMPOSITED_BUREAU_SCENARIO_FIN fenetre=0", 16, 200, 146, 2)
             + bloc("COMPOSITED_BUREAU_SCENARIO_FIN fenetre=1", 17, 460, 256, 0)
             + bloc("COMPOSITED_BUREAU_SCENARIO_FIN fenetre=2", 18, 638, 318, 1)
             + ["COMPOSITED_BUREAU_SCENARIO_PILE_PUBLIEE", "COMPOSITED_BUREAU_CLIENT_MORT pid=18"])
        self.assertEqual(fbr.etat(j, "COMPOSITED_BUREAU_SCENARIO_PILE_PUBLIEE"),
                         [(460, 256, 640, 400), (638, 318, 640, 400), (200, 146, 640, 400)])
        # Sans la republication de la troisieme, l'ordre etait faux : le temoin.
        perime = [l for l in j if l != "COMPOSITED_BUREAU_SCENARIO_FIN fenetre=2"]
        perime = perime[: perime.index("[COMPOSITED] pid=18", perime.index("COMPOSITED_BUREAU_SCENARIO_FIN fenetre=1"))] \
            + ["COMPOSITED_BUREAU_SCENARIO_PILE_PUBLIEE"]
        self.assertNotEqual(fbr.etat(perime, "COMPOSITED_BUREAU_SCENARIO_PILE_PUBLIEE"),
                            [(460, 256, 640, 400), (638, 318, 640, 400), (200, 146, 640, 400)])
        # Fin du journal : la mire morte n'est plus attendue.
        self.assertEqual(fbr.etat(j, None), [(460, 256, 640, 400), (200, 146, 640, 400)])

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
