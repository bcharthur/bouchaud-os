"""L'A/B de la cession au quantum (BOUCHAUD_CESSION_PAR_CLASSE_V1) : seules
des mesures qualifiees et appariees comptent, et chaque politique est tenue a
son compteur. Journaux synthetiques au format de la sonde."""
import importlib.util
import pathlib
import unittest

RACINE = pathlib.Path(__file__).resolve().parents[3]
_spec = importlib.util.spec_from_file_location("analyse_ab_cession", RACINE / "tools/ci/analyse_ab_cession.py")
a = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(a)

P = "[22:10:30][ 97%:  0%:  2%][FPS: --] "


def passe(t, v, verdict, p99, pire, avant, apres, tours_int=90000, tours_norm=100000, preuve="kvm-steal"):
    return [
        P + f"+ echo AB_CESSION tour={t} politique={v}",
        P + f"AB_CESSION tour={t} politique={v}",
        P + f"ORDO_CESSION politique={v} gardees={avant[0]} inversees={avant[1]}",
        P + f"  sous charge (normal)     median  1500 us  p95  2000 us  p99  {p99 + 500} us  pire  {pire + 500} us  [propres 300/300, regle=preuve-hote]",
        P + f"  comptes sous charge (normal) : preuve_hote={preuve} prouves=0 inexpliques=0",
        P + f"                           {tours_norm} tours de calcul",
        P + f"  sous charge (interactif) median   400 us  p95  1800 us  p99  {p99} us  pire  {pire} us  [propres 300/300, regle=preuve-hote]",
        P + f"  comptes sous charge (interactif) : preuve_hote={preuve} prouves=0 inexpliques=0",
        P + f"                           {tours_int} tours de calcul",
        P + f"VERDICT_SONDE : {verdict} tentatives=1",
        P + f"ORDO_CESSION politique={v} gardees={apres[0]} inversees={apres[1]}",
    ]


def journal(tours, mesure, fuite=None, inverse=False, fin=True, gardees=300):
    """`mesure(t, v)` -> (verdict, p99, pire[, tours_int]) ; compteurs cumules."""
    l, g, i = [], 0, 0
    for t in range(1, tours + 1):
        ordre = ["historique", "classe"] if (t % 2) != inverse else ["classe", "historique"]
        for v in ordre:
            avant = (g, i)
            if v == "classe":
                g += gardees
                if fuite == "inversion":
                    i += 2
            else:
                i += 900
                if fuite == "gardee":
                    g += 1
            m = mesure(t, v)
            l += passe(t, v, m[0], m[1], m[2], avant, (g, i), *(m[3:]))
    l += [P + f"ORDO_CESSION politique=classe gardees={g} inversees={i}"]
    if fin:
        l += [P + "+ echo AB_CESSION_FIN", P + "AB_CESSION_FIN"]
    return "\n".join(l) + "\n"


def mieux(t, v):
    return ("QUALIFIE_OK", 6000, 9000) if v == "historique" else ("QUALIFIE_OK", 2500, 4000)


class AbCession(unittest.TestCase):
    def test_classe_meilleure_et_qualifiee_est_convaincante(self):
        _, v, c = a.analyse(journal(6, mieux), 6, "4", "kvm")
        self.assertEqual((v, c), ("CONVAINCANT", 0))

    def test_une_queue_plus_longue_n_est_pas_convaincante(self):
        _, v, _ = a.analyse(journal(6, lambda t, v: ("QUALIFIE_OK", 6000, 9000) if v == "classe"
                                    else ("QUALIFIE_OK", 5000, 9000)), 6, "4", "kvm")
        self.assertEqual(v, "NON_CONVAINCANT")

    def test_un_calcul_affame_n_est_pas_convaincant(self):
        _, v, _ = a.analyse(journal(6, lambda t, v: mieux(t, v) + ((90000,) if v == "historique" else (60000,))),
                            6, "4", "kvm")
        self.assertEqual(v, "NON_CONVAINCANT")

    def test_un_echec_qualifie_sous_classe_n_est_pas_convaincant(self):
        _, v, _ = a.analyse(journal(6, lambda t, v: ("QUALIFIE_ECHEC", 2500, 4000) if (v, t) == ("classe", 2)
                                    else mieux(t, v)), 6, "4", "kvm")
        self.assertEqual(v, "NON_CONVAINCANT")

    def test_des_passes_non_qualifiantes_ne_comptent_pour_aucun_bras(self):
        _, v, c = a.analyse(journal(6, lambda t, v: ("NON_QUALIFIANT", 100, 200) if v == "classe"
                                    else mieux(t, v)), 6, "4", "kvm")
        self.assertEqual((v, c), ("NON_CONCLUANT", 0))

    def test_une_cession_gardee_sous_historique_est_un_defaut(self):
        _, v, c = a.analyse(journal(6, mieux, fuite="gardee"), 6, "4", "kvm")
        self.assertEqual((v, c), ("ECHEC", 1))

    def test_une_inversion_sous_classe_est_un_defaut(self):
        _, v, c = a.analyse(journal(6, mieux, fuite="inversion"), 6, "4", "kvm")
        self.assertEqual((v, c), ("ECHEC", 1))

    def test_l_ordre_ab_ba_est_exige(self):
        _, v, c = a.analyse(journal(6, mieux, inverse=True), 6, "4", "kvm")
        self.assertEqual((v, c), ("ECHEC", 1))

    def test_kvm_sans_preuve_de_l_hote_est_un_defaut(self):
        texte = journal(6, mieux).replace("preuve_hote=kvm-steal", "preuve_hote=absente")
        _, v, c = a.analyse(texte, 6, "4", "kvm")
        self.assertEqual((v, c), ("ECHEC", 1))

    def test_la_trace_de_l_echo_ne_vaut_pas_fin(self):
        texte = journal(6, mieux, fin=False) + P + "+ echo AB_CESSION_FIN\n"
        _, v, c = a.analyse(texte, 6, "4", "kvm")
        self.assertEqual((v, c), ("ECHEC", 1))

    def test_une_regle_qui_n_agit_jamais_ne_conclut_rien(self):
        sortie, v, c = a.analyse(journal(6, mieux, gardees=0), 6, "4", "kvm")
        self.assertEqual((v, c), ("NON_CONCLUANT", 0), "\n".join(sortie))


if __name__ == "__main__":
    unittest.main()
