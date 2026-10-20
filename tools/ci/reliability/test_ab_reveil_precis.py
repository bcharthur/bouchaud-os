"""L'analyse de l'A/B du reveil precis (BOUCHAUD_REVEIL_PRECIS_V1) : verdicts
et defauts sur des journaux fabriques. Aucun verdict de noyau n'est tire d'ici."""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import analyse_ab_reveil_precis as a  # noqa: E402

P = "[22:10:30][ 97%:  0%:  2%][FPS: --] "


def bras(t, v, repos=(900, 1600, 2500, 4000), inter=(600, 1000, 1500, 3000), verdict="QUALIFIE_OK",
         armes=(0, 0), voulu=None):
    voulu = (1 if v == "on" else 0) if voulu is None else voulu
    return [
        P + f"+ echo AB_PRECIS tour={t} reveil={v}",
        P + f"AB_PRECIS tour={t} reveil={v}",
        P + f"REVEIL_PRECIS voulu={voulu} minuteur=tsc-deadline armes={armes[0]} retires_avant_service=0",
        P + "  au repos                 median %5d us  p95 %6d us  p99 %6d us  pire %6d us" % repos,
        P + "  au repos                 median %5d us  p95 %6d us  p99 %6d us  pire %6d us  [propres 300/300, regle=preuve-hote]" % repos,
        P + "  sous charge (interactif) median %5d us  p95 %6d us  p99 %6d us  pire %6d us" % inter,
        P + "  sous charge (interactif) median %5d us  p95 %6d us  p99 %6d us  pire %6d us  [propres 300/300, regle=preuve-hote]" % inter,
        P + f"VERDICT_SONDE : {verdict} tentatives=1",
        P + f"REVEIL_PRECIS voulu={voulu} minuteur=tsc-deadline armes={armes[1]} retires_avant_service=0",
        P + f"+ echo AB_PRECIS_BRAS_FIN tour={t} reveil={v}",
        P + f"AB_PRECIS_BRAS_FIN tour={t} reveil={v}",
    ]


def journal(tours=4, on=None, off=None, inverse=False):
    l, armes = [], 0
    for t in range(1, tours + 1):
        ordre = ["off", "on"] if (t % 2) != inverse else ["on", "off"]
        for v in ordre:
            k = dict((on if v == "on" else off) or {})
            if v == "on":
                k.setdefault("armes", (armes, armes + 500))
                armes += 500
            else:
                k.setdefault("armes", (armes, armes))
            k.setdefault("repos", (300, 500, 900, 2000) if v == "on" else (900, 1600, 2500, 4000))
            l += bras(t, v, **k)
    return "\n".join(l) + "\n"


class AbReveilPrecis(unittest.TestCase):
    def test_convaincant(self):
        _, v, c = a.analyse(journal(), 4, "1", "kvm")
        self.assertEqual((v, c), ("CONVAINCANT", 0))

    def test_queue_degradee_non_convaincant(self):
        _, v, _ = a.analyse(journal(on={"repos": (300, 500, 112000, 144000)}), 4, "1", "kvm")
        self.assertEqual(v, "NON_CONVAINCANT")

    def test_charge_degradee_non_convaincant(self):
        _, v, _ = a.analyse(journal(on={"inter": (600, 1500, 2000, 3000)}), 4, "1", "kvm")
        self.assertEqual(v, "NON_CONVAINCANT")

    def test_gain_insuffisant_non_convaincant(self):
        _, v, _ = a.analyse(journal(on={"repos": (800, 1500, 2400, 4000)}), 4, "1", "kvm")
        self.assertEqual(v, "NON_CONVAINCANT")

    def test_moins_de_quatre_tours_non_concluant(self):
        _, v, c = a.analyse(journal(tours=3), 4, "1", "kvm")
        self.assertEqual((v, c), ("NON_CONCLUANT", 0))

    def test_aucun_armement_sous_on_est_un_defaut(self):
        _, v, c = a.analyse(journal(on={"armes": (0, 0)}), 4, "1", "kvm")
        self.assertEqual((v, c), ("DEFAUT", 1))

    def test_armement_sous_off_est_un_defaut(self):
        _, v, c = a.analyse(journal(off={"armes": (0, 3)}), 4, "1", "kvm")
        self.assertEqual((v, c), ("DEFAUT", 1))

    def test_echec_qualifie_sous_on_est_un_defaut(self):
        _, v, c = a.analyse(journal(on={"verdict": "QUALIFIE_ECHEC"}), 4, "1", "kvm")
        self.assertEqual((v, c), ("DEFAUT", 1))

    def test_politique_non_basculee_est_un_defaut(self):
        _, v, c = a.analyse(journal(on={"voulu": 0}), 4, "1", "kvm")
        self.assertEqual((v, c), ("DEFAUT", 1))

    def test_ordre_non_alterne_est_un_defaut(self):
        _, v, c = a.analyse(journal(inverse=True), 4, "1", "kvm")
        self.assertEqual((v, c), ("DEFAUT", 1))

    def test_panique_est_un_defaut(self):
        _, v, c = a.analyse(journal() + "panicked at x\n", 4, "1", "kvm")
        self.assertEqual((v, c), ("DEFAUT", 1))


    def test_non_qualifiant_on_ne_prouve_pas_un_gain(self):
        lignes, v, c = a.analyse(journal(on={"verdict": "NON_QUALIFIANT"}), 4, "1", "kvm")
        self.assertEqual((v, c), ("NON_CONCLUANT", 0))
        self.assertTrue(any("apparies_qualifies=0/4" in ligne for ligne in lignes))

    def test_non_qualifiant_off_ne_prouve_pas_un_gain(self):
        _, v, c = a.analyse(journal(off={"verdict": "NON_QUALIFIANT"}), 4, "4", "kvm")
        self.assertEqual((v, c), ("NON_CONCLUANT", 0))

    def test_paires_sans_quorum_qualifie_non_concluantes(self):
        texte = journal().replace("VERDICT_SONDE : QUALIFIE_OK", "VERDICT_SONDE : NON_QUALIFIANT", 1)
        _, v, c = a.analyse(texte, 4, "4", "kvm")
        self.assertEqual((v, c), ("NON_CONCLUANT", 0))

class DelimitationDuBras(unittest.TestCase):
    def test_restauration_finale_ne_contamine_pas_le_dernier_off(self):
        texte = journal() + P + "+ reveil-precis on\n" + P + (
            "REVEIL_PRECIS voulu=1 minuteur=tsc-deadline armes=2000 retires_avant_service=0"
        ) + "\n" + P + "AB_PRECIS_FIN\n"
        _, verdict, code = a.analyse(texte, 4, "4", "kvm")
        self.assertEqual((verdict, code), ("CONVAINCANT", 0))

    def test_absence_fin_bras_est_un_defaut(self):
        fermeture = P + "AB_PRECIS_BRAS_FIN tour=4 reveil=off\n"
        texte = journal().replace(fermeture, "", 1)
        self.assertNotEqual(texte, journal())
        _, verdict, code = a.analyse(texte, 4, "4", "kvm")
        self.assertEqual((verdict, code), ("DEFAUT", 1))

    def test_fausse_fin_trace_shell_n_est_pas_acceptee(self):
        fermeture = P + "AB_PRECIS_BRAS_FIN tour=4 reveil=off\n"
        texte = journal().replace(fermeture, "", 1)
        # Le journal garde "+ echo ...", qui ne suffit pas.
        self.assertIn(P + "+ echo AB_PRECIS_BRAS_FIN", texte)
        _, verdict, code = a.analyse(texte, 4, "4", "kvm")
        self.assertEqual((verdict, code), ("DEFAUT", 1))


class LignesEntrelacees(unittest.TestCase):
    """Run 38704459647 : sous KVM le noyau ecrit sur la ligne de la sonde ;
    six bras sur huit etaient rendus « mesure incomplete »."""

    def test_une_ligne_interrompue_par_le_noyau_est_lue(self):
        texte = journal().replace("regle=preuve-hote]", "regle=preuve-hote[SCHED-TACHE] tid=103 slot=3 etat=Blocked")
        texte = texte.replace("  sous charge (interactif) median", "[kernel] RUN_NOYAU  sous charge (interactif) median")
        _, v, c = a.analyse(texte, 4, "1", "kvm")
        self.assertEqual((v, c), ("CONVAINCANT", 0))

    def test_sans_ligne_propre_la_mesure_reste_incomplete(self):
        texte = "\n".join(l for l in journal().splitlines() if "[propres" not in l) + "\n"
        _, v, c = a.analyse(texte, 4, "1", "kvm")
        self.assertEqual((v, c), ("DEFAUT", 1))


if __name__ == "__main__":
    unittest.main()
