"""BOUCHAUD_STATE_AB_V1 : le verdict du banc STATE refuse toute perte ou tout faux positif."""
import importlib.util
import sys
import unittest
from pathlib import Path

RACINE = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("sv", RACINE / "tools/ci/state_ab_verdict.py")
sv = importlib.util.module_from_spec(spec)
sys.modules["sv"] = sv
spec.loader.exec_module(sv)


def journal(n, slot, sync=0, monte=True, boot=None, etat=None):
    t = f"BOUCHAUD_AB_BOOT slot={boot or slot} raison=actif\n"
    if monte:
        t += "BOUCHAUD_STAGE2_PERSIST_NVME fichiers=1 (differe)\n"
        t += f"BOUCHAUD_PERSIST_MONTAGE etat={etat or ('neuf' if n == 1 else 'restaure')} fichiers=1\n"
    return t + f"\x1b[0mBOUCHAUD_PERSIST_DEMARRAGES n={n} slot={slot} sync={sync}\n"


BONS = [journal(1, "A"), journal(2, "A"), journal(3, "B")]


class Verdict(unittest.TestCase):
    def test_nominal(self):
        self.assertEqual(sv.verifie(BONS), [])

    def test_compteur_reparti_a_un_apres_changement_de_slot(self):
        e = sv.verifie([BONS[0], BONS[1], journal(1, "B")])
        self.assertTrue(any("donnees STATE perdues" in x for x in e), e)

    def test_compteur_jamais_persiste(self):
        self.assertTrue(sv.verifie([journal(1, "A"), journal(1, "A"), journal(1, "B")]))

    def test_slot_b_non_demarre(self):
        self.assertTrue(sv.verifie([BONS[0], BONS[1], journal(3, "A")]))
        self.assertTrue(sv.verifie([BONS[0], BONS[1], journal(3, "B", boot="A")]))

    def test_state_non_montee(self):
        self.assertTrue(sv.verifie([journal(1, "A", monte=False), BONS[1], BONS[2]]))

    def test_sync_en_echec(self):
        self.assertTrue(sv.verifie([journal(1, "A", sync=-1), BONS[1], BONS[2]]))
        self.assertTrue(sv.verifie([journal(1, "A", sync=-2), BONS[1], BONS[2]]))

    def test_compteur_absent_ou_double(self):
        self.assertTrue(sv.verifie([BONS[0].replace("PERSIST_DEMARRAGES", "X"), BONS[1], BONS[2]]))
        self.assertTrue(sv.verifie([BONS[0] + BONS[0], BONS[1], BONS[2]]))

    def test_montage_en_echec_ou_neuf_a_tort(self):
        self.assertTrue(sv.verifie([BONS[0], journal(2, "A", etat="echec"), BONS[2]]))
        # Un volume « neuf » au 3e demarrage = donnees perdues, meme si le compteur ment.
        self.assertTrue(sv.verifie([BONS[0], BONS[1], journal(3, "B", etat="neuf")]))

    def test_panique(self):
        self.assertTrue(sv.verifie([BONS[0], BONS[1] + "*** KERNEL PANIC ***\n", BONS[2]]))

    def test_nombre_de_journaux(self):
        self.assertTrue(sv.verifie(BONS[:2]))


if __name__ == "__main__":
    unittest.main()
