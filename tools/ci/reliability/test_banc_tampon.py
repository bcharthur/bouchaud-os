"""L'A/B copie / composition directe (BOUCHAUD_VIRTIO_GPU_TAMPONS_V1) : la
surete d'abord (zero copie en direct, aucun acces pendant un transfert), puis
la mesure. Journaux synthetiques au format du banc."""
import importlib.util
import pathlib
import unittest

RACINE = pathlib.Path(__file__).resolve().parents[3]
_spec = importlib.util.spec_from_file_location("analyse_banc_tampon", RACINE / "tools/ci/analyse_banc_tampon.py")
a = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(a)

P = "[22:10:30][ 97%:  0%:  2%][FPS: --] "
N = {"plein": 40, "bande": 200, "tuile": 400}
TAILLE = {"plein": 3_686_400, "bande": 153_600, "tuile": 16_384}


def banc(t, v, p50, fin="640 0 1", copies=None, acces=0, n=None):
    l = [P + f"+ echo AB_TAMPON tour={t} tampon={v}", P + f"AB_TAMPON tour={t} tampon={v}"]
    for c in N:
        octets = TAILLE[c] * N[c]
        cp = (octets if v == "copie" else 0) if copies is None else copies
        dr = octets if v == "direct" else 0
        l.append(P + f"[BANC-PRESENT] ecran=virtio-gpu-2d categorie={c} rect=1x1 n={(n or N)[c]} moyen_us=1 "
                 f"p50_us={p50[c]} p95_us=1 max_us=1 mpix_s=1 contexte=tache attente=scrute cessions=0 sommeils=0 "
                 f"irq=0 irq_sans_progres=0 vus_par_relecture=0 echues=0 coeur=0 inactif_pour_mille=0 "
                 f"tampon={v} octets_copies={cp} octets_directs={dr} acces_en_transfert={acces}")
    ok, ech, actif = fin.split()
    l.append(P + f"[BANC-PRESENT] ecran=virtio-gpu-2d fin presentations_virtio_ok={ok} echecs_virtio={ech} actif={actif} "
             "contexte=tache attente=scrute")
    return l


COPIE = {"plein": 1000, "bande": 100, "tuile": 50}
DIRECT = {"plein": 600, "bande": 95, "tuile": 50}


def journal(tours=6, direct=None, copie=None, inverse=False, **kw):
    l = []
    for t in range(1, tours + 1):
        ordre = ["copie", "direct"] if (t % 2) != inverse else ["direct", "copie"]
        for v in ordre:
            extra = kw if v == "direct" else {}
            l += banc(t, v, (direct or DIRECT) if v == "direct" else (copie or COPIE), **extra)
    l += [P + "+ echo AB_TAMPON_FIN", P + "AB_TAMPON_FIN"]
    return "\n".join(l) + "\n"


class BancTampon(unittest.TestCase):
    def test_direct_plus_rapide_et_sur_est_convaincant(self):
        _, v, c = a.analyse(journal(), 6, "kvm")
        self.assertEqual((v, c), ("CONVAINCANT", 0))

    def test_un_plein_ecran_pas_assez_gagne_n_est_pas_convaincant(self):
        _, v, _ = a.analyse(journal(direct={"plein": 900, "bande": 95, "tuile": 50}), 6, "kvm")
        self.assertEqual(v, "NON_CONVAINCANT")

    def test_une_tuile_degradee_n_est_pas_convaincante(self):
        _, v, _ = a.analyse(journal(direct={"plein": 600, "bande": 95, "tuile": 70}), 6, "kvm")
        self.assertEqual(v, "NON_CONVAINCANT")

    def test_une_copie_en_composition_directe_est_un_defaut(self):
        _, v, c = a.analyse(journal(copies=4096), 6, "kvm")
        self.assertEqual((v, c), ("DEFAUT", 1))

    def test_un_acces_pendant_un_transfert_est_un_defaut(self):
        _, v, c = a.analyse(journal(acces=1), 6, "kvm")
        self.assertEqual((v, c), ("DEFAUT", 1))

    def test_un_echec_de_presentation_est_un_defaut(self):
        _, v, c = a.analyse(journal(fin="639 1 1"), 6, "kvm")
        self.assertEqual((v, c), ("DEFAUT", 1))

    def test_un_scanout_abandonne_est_un_defaut(self):
        _, v, c = a.analyse(journal(fin="640 0 0"), 6, "kvm")
        self.assertEqual((v, c), ("DEFAUT", 1))

    def test_l_ordre_ab_ba_est_exige(self):
        _, v, c = a.analyse(journal(inverse=True), 6, "kvm")
        self.assertEqual((v, c), ("DEFAUT", 1))

    def test_moins_de_quatre_tours_ne_conclut_rien(self):
        _, v, c = a.analyse(journal(tours=3), 3, "kvm")
        self.assertEqual((v, c), ("NON_CONCLUANT", 0))

    def test_la_trace_de_l_echo_ne_vaut_pas_fin(self):
        texte = journal().replace(P + "AB_TAMPON_FIN\n", "")
        _, v, c = a.analyse(texte, 6, "kvm")
        self.assertEqual((v, c), ("DEFAUT", 1))


if __name__ == "__main__":
    unittest.main()
