"""Contrats stricts : une mesure inqualifiable ne devient jamais un succes P13."""
import sys
import unittest
from pathlib import Path

CI = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(CI))

import qualifie_p13 as q  # noqa: E402
from test_ab_reveil_precis import P, journal  # noqa: E402


def attrib(verdicts=("QUALIFIE_OK", "QUALIFIE_OK"), preuves=6, fini=True):
    lignes = ["+ echo ORDO_ATTRIBUTION_FIN"]
    for v in verdicts:
        lignes.append(f"VERDICT_SONDE : {v} tentatives=1")
    lignes.extend("  comptes au repos : preuve_hote=kvm-steal prouves=0" for _ in range(preuves))
    if fini:
        lignes.append("ORDO_ATTRIBUTION_FIN")
    return "\n".join(lignes)


class AttributionStricte(unittest.TestCase):
    def test_vraiment_qualifie(self):
        self.assertEqual(q.inspecte_attribution(attrib(), 4, 2), [])

    def test_non_qualifiant_ne_passe_pas(self):
        self.assertTrue(q.inspecte_attribution(attrib(("QUALIFIE_OK", "NON_QUALIFIANT")), 2, 2))

    def test_preuve_hote_absente(self):
        self.assertTrue(q.inspecte_attribution(attrib(preuves=0), 1, 2))

    def test_echo_shell_n_est_pas_la_fin(self):
        self.assertTrue(q.inspecte_attribution(attrib(fini=False), 1, 2))

    def test_verdict_manquant(self):
        self.assertTrue(q.inspecte_attribution(attrib(("QUALIFIE_OK",)), 1, 2))


class ABEveilStrict(unittest.TestCase):
    def test_convaincant_et_qualifie(self):
        texte = journal() + P + "AB_PRECIS_FIN\n"
        self.assertEqual(q.inspecte_ab(texte, 1, 4), [])

    def test_non_qualifiant_ne_prouve_pas_de_gain(self):
        texte = journal(on={"verdict": "NON_QUALIFIANT"}) + P + "AB_PRECIS_FIN\n"
        self.assertTrue(q.inspecte_ab(texte, 1, 4))

    def test_p95_regresse_meme_avec_quorum(self):
        texte = journal(on={"repos": (300, 1700, 2000, 4000)}) + P + "AB_PRECIS_FIN\n"
        self.assertTrue(q.inspecte_ab(texte, 1, 4))

    def test_simple_trace_de_echo_ne_suffit_pas(self):
        texte = journal() + P + "+ echo AB_PRECIS_FIN\n"
        self.assertTrue(q.inspecte_ab(texte, 4, 4))

    def test_surete_pas_masquee(self):
        texte = journal(on={"verdict": "QUALIFIE_ECHEC"}) + P + "AB_PRECIS_FIN\n"
        self.assertTrue(q.inspecte_ab(texte, 4, 4))


if __name__ == "__main__":
    unittest.main()
