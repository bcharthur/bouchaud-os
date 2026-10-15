"""L'A/B de la variante d'IPI ne compte que des mesures qualifiees
(BOUCHAUD_AB_REVEIL_IPI_V2).

Journaux synthetiques au format de la sonde : chaque cas negatif est un
CONVAINCANT que le banc aurait pu rendre sur des passes NON_QUALIFIANT, ou
avec trop peu de mesures.
"""
import importlib.util
import pathlib
import unittest

RACINE = pathlib.Path(__file__).resolve().parents[3]
_spec = importlib.util.spec_from_file_location("analyse_ab", RACINE / "tools/ci/analyse_ab_reveil_ipi.py")
analyse_ab = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(analyse_ab)


def passe(tour, variante, verdict, p99, pire, tours_int=100000, tours_norm=100000, ipi=0, preuve="kvm-steal",
          depart=None):
    depart = ipi if depart is None else depart
    return "\n".join([
        f"AB_BRAS tour={tour} variante={variante}",
        f"ORDO_VARIANTE reveil={variante} ipi_variante={depart} differes=0 cibles=0 immediats=0 ipi=0 preempt_noyau=0 grace_ms=50",
        f"  sous charge (normal)     median  1500 us  p95  2000 us  p99  {p99 + 500} us  pire  {pire + 500} us  [propres 300/300, regle=preuve-hote]",
        f"  {tours_norm} tours de calcul",
        f"  sous charge (interactif) median  1200 us  p95  1800 us  p99  {p99} us  pire  {pire} us  [propres 300/300, regle=preuve-hote]",
        f"  comptes sous charge (interactif) : preuve_hote={preuve} prouves=0 inexpliques=0 ecartes=0 externe_us=0"
        f" inexplique_us=0 inexplique_pire_us=0 masque_us=0 pret_us=0 incompletes=0 vol_hote_pour_mille=3",
        f"  {tours_int} tours de calcul",
        f"VERDICT_SONDE : {verdict} tentatives=1",
        f"ORDO_VARIANTE reveil={variante} ipi_variante={ipi} differes=0 cibles=0 immediats=0 ipi=0 preempt_noyau=0",
    ])


def journal(passes):
    return "\n".join(["AB_REVEIL_DEBUT"] + passes + ["AB_REVEIL_FIN"])


def scenario(tours, mesure):
    """`mesure(t, variante)` -> (verdict, p99, pire) ; ordre AB/BA, IPI cumules."""
    passes, ipi = [], 0
    for t in range(1, tours + 1):
        ordre = ["defaut", "ipi-interactive"] if t % 2 else ["ipi-interactive", "defaut"]
        for v in ordre:
            depart = ipi
            if v == "ipi-interactive":
                ipi += 10
            verdict, p99, pire = mesure(t, v)
            passes.append(passe(t, v, verdict, p99, pire, ipi=ipi, depart=depart))
    return journal(passes)


class Ab(unittest.TestCase):
    def verdict(self, texte, tours=6, nom="kvm"):
        sortie, verdict, code = analyse_ab.analyse(texte, tours, "4", nom)
        return verdict, code, "\n".join(sortie)

    def test_la_variante_meilleure_sur_assez_de_tours_apparies_est_convaincante(self):
        texte = scenario(6, lambda t, v: ("QUALIFIE_OK", 2000, 4000) if v == "defaut" else ("QUALIFIE_OK", 1400, 3500))
        verdict, code, _ = self.verdict(texte)
        self.assertEqual((verdict, code), ("CONVAINCANT", 0))

    def test_un_bras_non_qualifiant_ne_compte_jamais(self):
        # La variante parait spectaculaire... sur des passes NON_QUALIFIANT.
        texte = scenario(6, lambda t, v: ("QUALIFIE_OK", 2000, 4000) if v == "defaut" else ("NON_QUALIFIANT", 300, 600))
        verdict, code, sortie = self.verdict(texte)
        self.assertEqual((verdict, code), ("NON_CONCLUANT", 0))
        self.assertIn("variante 0/6", sortie)

    def test_trop_peu_de_tours_apparies_n_est_pas_un_verdict(self):
        # Trois tours sur six ou les DEUX bras sont qualifies : quorum 4.
        def mesure(t, v):
            if t > 3 and v == "ipi-interactive":
                return ("NON_QUALIFIANT", 300, 600)
            return ("QUALIFIE_OK", 2000, 4000) if v == "defaut" else ("QUALIFIE_OK", 1000, 2000)
        verdict, _, sortie = self.verdict(scenario(6, mesure))
        self.assertEqual(verdict, "NON_CONCLUANT")
        self.assertIn("tours apparies 3 (quorum 4)", sortie)

    def test_seuls_les_tours_apparies_portent_les_chiffres(self):
        # Le defaut est qualifie seul, tres lent, aux tours 5 et 6 (variante
        # NON_QUALIFIANT) : ces mesures isolees ne doivent pas faire gagner
        # la variante.
        def mesure(t, v):
            if t >= 5:
                return ("QUALIFIE_OK", 9000, 20000) if v == "defaut" else ("NON_QUALIFIANT", 100, 100)
            return ("QUALIFIE_OK", 2000, 4000) if v == "defaut" else ("QUALIFIE_OK", 1900, 3900)
        verdict, _, sortie = self.verdict(scenario(6, mesure))
        self.assertEqual(verdict, "NON_CONVAINCANT")
        self.assertIn("p99 median 2000.0 -> 1900.0 us", sortie)

    def test_un_echec_qualifie_sous_la_variante_n_est_jamais_convaincant(self):
        texte = scenario(6, lambda t, v: ("QUALIFIE_OK", 2000, 4000) if v == "defaut"
                         else (("QUALIFIE_ECHEC" if t == 2 else "QUALIFIE_OK"), 1000, 2000))
        verdict, _, _ = self.verdict(texte)
        self.assertEqual(verdict, "NON_CONVAINCANT")

    def test_l_ordre_alterne_est_lu_et_les_positions_imprimees(self):
        texte = scenario(6, lambda t, v: ("QUALIFIE_OK", 2000, 4000) if v == "defaut" else ("QUALIFIE_OK", 1400, 3500))
        _, _, sortie = self.verdict(texte)
        for pos in (0, 1):
            self.assertIn(f"position {pos} defaut", sortie)
            self.assertIn(f"position {pos} ipi-interactive", sortie)

    def test_un_ipi_de_variante_sous_defaut_est_un_defaut_de_surete(self):
        texte = scenario(6, lambda t, v: ("QUALIFIE_OK", 2000, 4000))
        # Le compteur avance pendant une passe `defaut`.
        texte = texte.replace("AB_BRAS tour=3 variante=defaut", "AB_BRAS tour=3 variante=defaut", 1)
        passes = texte.split("AB_BRAS ")
        texte = texte.replace("reveil=defaut ipi_variante=40", "reveil=defaut ipi_variante=41", 1)
        verdict, code, _ = self.verdict(texte)
        self.assertEqual((verdict, code), ("ECHEC", 1))

    def test_une_ipi_avant_la_bascule_n_est_pas_imputee_au_bras_defaut(self):
        """Run 38645396730 : deux IPI sous la variante, entre la fin de son
        bras et la bascule, etaient comptees au bras `defaut` suivant. Le
        depart du bras est desormais son propre releve, apres la bascule."""
        texte = scenario(6, lambda t, v: ("QUALIFIE_OK", 2000, 4000))
        # Tour 4 (BA) : variante (10 -> 40) puis defaut ; deux IPI avant la
        # bascule : le releve de la bascule vaut 42, la fin 42.
        texte = texte.replace(
            "AB_BRAS tour=4 variante=defaut\nORDO_VARIANTE reveil=defaut ipi_variante=40 ",
            "AB_BRAS tour=4 variante=defaut\nORDO_VARIANTE reveil=defaut ipi_variante=42 ", 1)
        i = texte.index("AB_BRAS tour=4 variante=defaut")
        j = texte.index("VERDICT_SONDE", i)
        texte = texte[:j] + texte[j:].replace("reveil=defaut ipi_variante=40 ", "reveil=defaut ipi_variante=42 ", 1)
        verdict, code, sortie = self.verdict(texte)
        self.assertEqual(code, 0, sortie)

    def test_une_ipi_pendant_le_bras_defaut_reste_un_defaut_de_surete(self):
        texte = scenario(6, lambda t, v: ("QUALIFIE_OK", 2000, 4000))
        i = texte.index("AB_BRAS tour=4 variante=defaut")
        j = texte.index("VERDICT_SONDE", i)
        texte = texte[:j] + texte[j:].replace("reveil=defaut ipi_variante=40 ", "reveil=defaut ipi_variante=41 ", 1)
        verdict, code, sortie = self.verdict(texte)
        self.assertEqual((verdict, code), ("ECHEC", 1))
        self.assertIn("1 IPI de variante sous defaut (tour 4)", sortie)

    def test_une_bascule_non_relevee_est_un_defaut_d_instrument(self):
        texte = scenario(6, lambda t, v: ("QUALIFIE_OK", 2000, 4000))
        texte = texte.replace("AB_BRAS tour=2 variante=defaut\nORDO_VARIANTE reveil=defaut",
                              "AB_BRAS tour=2 variante=defaut\nORDO_VARIANTE reveil=ipi-interactive", 1)
        verdict, code, sortie = self.verdict(texte)
        self.assertEqual((verdict, code), ("ECHEC", 1))
        self.assertIn("bascule non relevee", sortie)

    def test_sous_kvm_une_passe_sans_preuve_de_l_hote_est_un_defaut_d_instrument(self):
        texte = scenario(6, lambda t, v: ("QUALIFIE_OK", 2000, 4000)).replace("preuve_hote=kvm-steal", "preuve_hote=absente", 1)
        verdict, code, sortie = self.verdict(texte)
        self.assertEqual((verdict, code), ("ECHEC", 1))
        self.assertIn("sans preuve de l'hote", sortie)
        # Sous TCG, l'absence de preuve est attendue.
        verdict, code, _ = self.verdict(texte, nom="tcg")
        self.assertEqual(code, 0)


if __name__ == "__main__":
    unittest.main()
