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


class DiagnosticNoyau(unittest.TestCase):
    """Les compteurs du noyau (queue au repos) : differences par bras, sans
    effet sur le verdict."""

    def test_differences_publiees_et_verdict_inchange(self):
        base = journal()
        texte = []
        n = 0
        for l in base.splitlines():
            if "REVEIL_PRECIS voulu=" in l:
                n += 1
                l += (f" periodique=0 materiel=0 hyperviseur=1 tirs={n * 10} bornes_propres={n * 7} "
                      f"ipi_pendant_arme=0 armes_dus={n * 2} refus_quantum_avant={n * 3} refus_sans_borne=0 "
                      f"refus_autres=0 tardifs_soi={n} tardifs_autre={n * 4} tardifs_irq=0 tardifs_inactif=0 "
                      f"installes_ipi={n * 5} installes_eveilles=0 serie_longues={n} serie_pire_us=4321")
            texte.append(l)
        sortie, v, c = a.analyse("\n".join(texte) + "\n", 4, "1", "kvm")
        self.assertEqual((v, c), ("CONVAINCANT", 0))
        lignes = [x for x in sortie if x.startswith("  noyau tour 1 off")]
        self.assertEqual(len(lignes), 1)
        self.assertIn("tirs=+10", lignes[0])
        self.assertIn("tardifs_autre=+4", lignes[0])
        self.assertIn("bornes_propres=+7", lignes[0])
        self.assertIn("installes_ipi=+5", lignes[0])
        self.assertIn("serie_longues=+1", lignes[0])
        self.assertIn("serie_pire_us=4321", lignes[0])

    def test_ligne_d_avant_les_compteurs_d_installation(self):
        # Un noyau qui ne publie pas encore les champs optionnels : zero.
        base = journal()
        texte = []
        for l in base.splitlines():
            if "REVEIL_PRECIS voulu=" in l:
                l += (" tirs=1 tirs_avant_cible=0 avance_max_ns=0 ipi_pendant_arme=0 armes_dus=0 "
                      "refus_quantum_avant=0 refus_sans_borne=0 refus_autres=0 tardifs_soi=0 tardifs_autre=0 "
                      "tardifs_irq=0 tardifs_inactif=0")
            texte.append(l)
        sortie, v, c = a.analyse("\n".join(texte) + "\n", 4, "1", "kvm")
        self.assertEqual((v, c), ("CONVAINCANT", 0))
        self.assertTrue(any("installes_ipi=+0" in x for x in sortie))


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


class RaisonDesBrasNonQualifies(unittest.TestCase):
    def test_raison_recopiee_verdict_inchange(self):
        texte = journal(off={"verdict": "NON_QUALIFIANT"}).replace(
            "VERDICT_SONDE : NON_QUALIFIANT tentatives=1",
            "VERDICT_SONDE : NON_QUALIFIANT tentatives=1 raison=charge sans effet mesurable : p95 sous charge 212")
        sortie, v, c = a.analyse(texte, 4, "1", "kvm")
        self.assertEqual((v, c), ("NON_CONCLUANT", 0))
        self.assertTrue(any(x.startswith("  raison tour 1 off : NON_QUALIFIANT charge sans effet") for x in sortie))

class PreuveDesEchecsQualifies(unittest.TestCase):
    def test_critere_et_charge_recopies_verdict_inchange(self):
        texte = journal(on={"verdict": "QUALIFIE_ECHEC"})
        texte = texte.replace(
            "VERDICT_SONDE : QUALIFIE_ECHEC tentatives=1",
            "  sous charge (normal)     median   500 us  p95    900 us  p99   1200 us  pire   1500 us"
            "  [propres 300/300, regle=preuve-hote]\n"
            + P + "  PREUVE et n'aggrave pas le pire cas au-dela du bruit au repos : tout[pire avec 3900"
            " <= sans 1500 + bruit au repos 2000] = faux ; propre[pire avec 3900 <= sans 1500 + bruit au"
            " repos 2000] = faux ; contamines ecartes repos=0 normal=0 interactif=0 sur 300 (preuve-hote)"
            " -> QUALIFIE_ECHEC\n" + P + "VERDICT_SONDE : QUALIFIE_ECHEC tentatives=1", 1)
        sortie, v, c = a.analyse(texte, 4, "4", "kvm")
        self.assertEqual((v, c), ("DEFAUT", 1))
        self.assertTrue(any(x.startswith("  preuve tour 1 on : et n'aggrave pas le pire cas") and
                            x.endswith("-> QUALIFIE_ECHEC") for x in sortie), sortie)
        self.assertIn("  charge tour 1 on : normal p50 500 p95 900 p99 1200 pire 1500 ; "
                      "interactif p50 600 p95 1000 p99 1500 pire 3000", sortie)

    def test_rien_pour_un_bras_qualifie_ok(self):
        sortie, _, _ = a.analyse(journal(), 4, "4", "kvm")
        self.assertFalse(any(x.startswith(("  preuve tour", "  charge tour")) for x in sortie))


class OccupantDuPireCas(unittest.TestCase):
    def test_occupant_recopie_verdict_inchange(self):
        texte = journal().replace(
            "retires_avant_service=0",
            "retires_avant_service=0 tirs=1 attentes_noyau=1 attente_noyau_pire_us=5096 "
            "occupant_noyau=7:usb-hid occupant_utilisateur=-")
        sortie, v, c = a.analyse(texte, 4, "4", "kvm")
        self.assertEqual((v, c), ("CONVAINCANT", 0))
        self.assertTrue(any(x.startswith("  noyau tour 1 on : ") and x.endswith(
            "occupant_noyau=7:usb-hid occupant_utilisateur=-") for x in sortie), sortie)

    def test_classement_des_attentes_recopie(self):
        # BOUCHAUD_ATTENTE_INTERACTIVE_CLASSEE_V1 : les cumuls par classe et par
        # raison de renoncement sont des differences ; l'occupant garde ses
        # battements, l'intervalle, la part masquee, le vol et la raison.
        n = [0]
        def releve(m):
            n[0] += 1
            return (f"retires_avant_service=0 tirs={n[0]} longues_b2={3 * n[0]} longues_b1_retard={n[0]} "
                    f"renonce_differee_appel={100 * n[0]} occupant_noyau=- "
                    f"occupant_utilisateur=140:/bin/ordonnanceur-probe@cpu0:b2:int1941:masq25:vol?:renonce_differee_appel")
        import re
        texte = re.sub("retires_avant_service=0", releve, journal())
        sortie, v, c = a.analyse(texte, 4, "4", "kvm")
        self.assertEqual((v, c), ("CONVAINCANT", 0))
        ligne = next(x for x in sortie if x.startswith("  noyau tour 1 off : "))
        self.assertIn("longues_b2=+3", ligne)
        self.assertIn("longues_b1_retard=+1", ligne)
        self.assertIn("renonce_differee_appel=+100", ligne)
        self.assertTrue(ligne.endswith(
            "occupant_utilisateur=140:/bin/ordonnanceur-probe@cpu0:b2:int1941:masq25:vol?:renonce_differee_appel"), ligne)


if __name__ == "__main__":
    unittest.main()
