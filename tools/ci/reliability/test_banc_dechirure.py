"""L'analyse du banc de dechirure (BOUCHAUD_GUI_SURFACE_DEUX_TAMPONS_V1) :
verdicts et defauts sur des journaux fabriques. Aucun verdict de noyau n'est
tire d'ici."""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import analyse_banc_dechirure as a  # noqa: E402

P = "[14:26:00][ 50%:  0%:  1%][FPS: --] "


def bras(t, v, mixtes=None, lues=120, tampons=None, defaut=0, liberations=None, trames=150, sur_face=0, duree=6000):
    tampons = (2 if v == "v2" else 1) if tampons is None else tampons
    mixtes = (0 if v == "v2" else 9) if mixtes is None else mixtes
    liberations = (trames - 1 if v == "v2" else 0) if liberations is None else liberations
    return [
        P + f"+ echo BANC_DECHIRURE tour={t} mode={v}",
        P + f"BANC_DECHIRURE tour={t} mode={v}",
        P + f"SURFACE_SONDE_DEBUT mode={v} offerts={tampons} tampons={tampons} surface=1278x626 duree_ms={duree}",
        P + f"SURFACE_SONDE mode={v} tampons={tampons} trames={trames} duree_ms={duree} trames_par_s=25 "
            f"liberations={liberations} liberations_etrangeres=0 attentes=10 attente_moy_us=10 attente_max_us=20 "
            f"ecritures_refusees=0 defaut={defaut}",
        P + f"GUI_SURFACE_BILAN pid=15 offerts={tampons} tampons={tampons} trames={trames} liberations={liberations} "
            f"liberations_retardees=0 hors_bornes=0 ecritures_sur_face={sur_face} ecritures_sans_liberation=0 "
            f"compositions_lues={lues} compositions_mixtes={mixtes}",
    ]


def journal(tours=4, v1=None, v2=None, inverse=False):
    l = []
    for t in range(1, tours + 1):
        ordre = ["v1", "v2"] if (t % 2) != inverse else ["v2", "v1"]
        for v in ordre:
            l += bras(t, v, **((v1 if v == "v1" else v2) or {}))
    return "\n".join(l) + "\n"


class BancDechirure(unittest.TestCase):
    def test_convaincant(self):
        _, v, c = a.analyse(journal(), 4, "2", "kvm")
        self.assertEqual((v, c), ("CONVAINCANT", 0))

    def test_une_composition_mixte_en_v2_est_un_defaut(self):
        _, v, c = a.analyse(journal(v2={"mixtes": 1}), 4, "2", "kvm")
        self.assertEqual((v, c), ("DEFAUT", 1))

    def test_sans_dechirure_en_v1_le_banc_ne_conclut_pas(self):
        _, v, c = a.analyse(journal(v1={"mixtes": 0}), 4, "2", "kvm")
        self.assertEqual((v, c), ("NON_CONCLUANT", 0))

    def test_debit_effondre_non_convaincant(self):
        _, v, _ = a.analyse(journal(v2={"lues": 60}, v1={"lues": 120}), 4, "2", "kvm")
        self.assertEqual(v, "NON_CONVAINCANT")

    def test_instrument_aveugle_est_un_defaut(self):
        _, v, c = a.analyse(journal(v1={"lues": 3}), 4, "2", "kvm")
        self.assertEqual((v, c), ("DEFAUT", 1))

    def test_contrat_v2_non_retenu_est_un_defaut(self):
        _, v, c = a.analyse(journal(v2={"tampons": 1}), 4, "2", "kvm")
        self.assertEqual((v, c), ("DEFAUT", 1))

    def test_liberations_manquantes_sont_un_defaut(self):
        _, v, c = a.analyse(journal(v2={"liberations": 10}), 4, "2", "kvm")
        self.assertEqual((v, c), ("DEFAUT", 1))

    def test_ecriture_sur_la_face_est_un_defaut(self):
        _, v, c = a.analyse(journal(v2={"sur_face": 1}), 4, "2", "kvm")
        self.assertEqual((v, c), ("DEFAUT", 1))

    def test_sonde_en_defaut_est_un_defaut(self):
        _, v, c = a.analyse(journal(v2={"defaut": 1}), 4, "2", "kvm")
        self.assertEqual((v, c), ("DEFAUT", 1))

    def test_bilan_absent_est_un_defaut(self):
        texte = journal().replace("GUI_SURFACE_BILAN", "GUI_SURFACE_AUTRE", 1)
        _, v, c = a.analyse(texte, 4, "2", "kvm")
        self.assertEqual((v, c), ("DEFAUT", 1))

    def test_ordre_non_alterne_est_un_defaut(self):
        _, v, c = a.analyse(journal(inverse=True), 4, "2", "kvm")
        self.assertEqual((v, c), ("DEFAUT", 1))

    def test_moins_de_quatre_tours_non_concluant(self):
        _, v, c = a.analyse(journal(tours=3), 4, "2", "kvm")
        self.assertEqual((v, c), ("NON_CONCLUANT", 0))

    def test_la_trace_du_shell_n_ouvre_pas_de_bras(self):
        texte = "\n".join(l for l in journal().splitlines() if "+ echo" in l or "BANC_DECHIRURE" not in l) + "\n"
        _, v, c = a.analyse(texte, 4, "2", "kvm")
        self.assertEqual((v, c), ("NON_CONCLUANT", 0))

    def test_panique_est_un_defaut(self):
        _, v, c = a.analyse(journal() + "panicked at x\n", 4, "2", "kvm")
        self.assertEqual((v, c), ("DEFAUT", 1))


if __name__ == "__main__":
    unittest.main()
