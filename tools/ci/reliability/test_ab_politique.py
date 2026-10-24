"""Adopted P13 policy: complete evidence, negative mutations, no false greens."""
import unittest

from test_ab_reveil_precis import P, a, journal5


def journal(**kwargs):
    texte = journal5(**kwargs)
    for phase in a.PHASES[1:]:
        texte = texte.replace(P + "VERDICT_SONDE", P + f"  attribution {phase} : reveils=300\n" + P + "VERDICT_SONDE")
    return texte


def controle_negatif():
    texte = journal(off={"normal": (1000, 1700, 2500, 4000), "verdict": a.NQ})
    return texte.replace("NON_QUALIFIANT tentatives=1", "NON_QUALIFIANT tentatives=1 raison=charge sans effet mesurable : p95 sous charge 1700")


class PolitiqueAdoptee(unittest.TestCase):
    def analyse(self, texte):
        lignes, verdict, code = a.analyse(texte, 4, "4", "kvm")
        return "\n".join(lignes), verdict, code

    def test_candidat_et_controle_qualifies(self):
        self.assertEqual(self.analyse(journal())[1:], ("CONVAINCANT", 0))

    def test_controle_negatif_sans_effet_ne_devient_pas_qualifie_historique(self):
        texte, verdict, code = self.analyse(controle_negatif())
        self.assertEqual((verdict, code), ("CONVAINCANT", 0))
        self.assertIn("AB_REVEIL_PRECIS_HISTORIQUE : NON_CONCLUANT", texte)
        self.assertIn("role=controle-negatif verdict=QUALIFIE_OK P3=NON_QUALIFIANT historique=NON_QUALIFIANT", texte)
        self.assertIn("non bloquant, jamais OK", texte)

    def test_on_non_qualifie_reste_refuse(self):
        self.assertNotEqual(self.analyse(journal(on={"verdict": a.NQ, "normal": (300, 600, 900, 2000)}))[1], "CONVAINCANT")

    def test_controle_non_qualifie_inexplique_refuse(self):
        self.assertNotEqual(self.analyse(controle_negatif().replace("charge sans effet mesurable :", "horloge non verifiable :"))[1], "CONVAINCANT")

    def test_p3_frontiere_du_bruit_sans_constante(self):
        for normal, attendu in ((700, "CONVAINCANT"), (701, "NON_CONVAINCANT")):
            texte = journal(on={"normal": (300, normal, 900, 2000), "inter": (300, 700, 1000, 2100)})
            sortie, verdict, _ = self.analyse(texte)
            self.assertEqual(verdict, attendu, sortie)
            if normal == 700:
                self.assertIn("P3 NON_QUALIFIANT charge dans le bruit", sortie)

    def test_p3_ne_masque_pas_p4_ni_p5(self):
        for k in ({"inter": (300, 700, 1000, 5000)}, {"tours": (1000, 99)}):
            k["normal"] = (300, 700, 900, 2000)
            self.assertEqual(self.analyse(journal(on=k))[1], "NON_CONVAINCANT")

    def test_echec_historique_inexplique_jamais_efface(self):
        self.assertEqual(self.analyse(journal(off={"verdict": a.ECHEC, "normal": (1000, 1700, 2500, 4000)}))[1], "NON_CONVAINCANT")

    def test_echec_historique_p3_sous_bruit_est_trace(self):
        texte = journal(on={"verdict": a.ECHEC, "normal": (300, 700, 900, 2000)})
        texte = texte.replace("VERDICT_SONDE : QUALIFIE_ECHEC", "  PREUVE la priorite interactive raccourcit la queue de latence : chiffres -> QUALIFIE_ECHEC\n" + P + "VERDICT_SONDE : QUALIFIE_ECHEC")
        sortie, verdict, _ = self.analyse(texte)
        self.assertEqual(verdict, "CONVAINCANT", sortie)
        self.assertIn("historique=QUALIFIE_ECHEC", sortie)
        self.assertIn("P3=NON_QUALIFIANT", sortie)

    def test_mutations_preuves_ne_passent_jamais(self):
        valide = controle_negatif()
        mutations = {
            "contamination_5pct": valide.replace("300/300", "285/300", 1),
            "effectif_zero": valide.replace("300/300", "0/0", 1),
            "effectif_invente": valide.replace("300/300", "301/300", 1),
            "exclusion_inconnue": valide.replace("regle=preuve-hote", "regle=ignore-retards", 1),
            "fermeture": valide.replace("AB_PRECIS_BRAS_FIN tour=1 reveil=off", "fin", 1),
            "tour_hors_protocole": valide.replace("tour=4", "tour=5"),
            "panique": valide + "KERNEL PANIC\n",
        }
        for phase in a.PHASES:
            mutations[f"attribution_{phase}"] = valide.replace(f"attribution {phase} :", f"absente {phase} :", 1)
            mutations[f"mesure_{phase}"] = "\n".join(l for l in valide.splitlines() if not (phase in l and "median" in l))
            mutations[f"non_jugeable_{phase}"] = valide.replace("VERDICT_SONDE", f"  phase non jugeable -- {phase} : attribution incomplete\n" + P + "VERDICT_SONDE", 1)
        for nom, texte in mutations.items():
            with self.subTest(nom=nom):
                self.assertNotEqual(self.analyse(texte)[1], "CONVAINCANT")

    def test_precision_et_equite_reelles_non_assouplies(self):
        for change in ({"repos": (1000, 1700, 2500, 4000)},
                       {"inter": (600, 1200, 1500, 3000)},
                       {"inter": (600, 1000, 7000, 13000)},
                       {"tours": (1000, 0)}):
            self.assertEqual(self.analyse(journal(on=change))[1], "NON_CONVAINCANT")


if __name__ == "__main__":
    unittest.main()
