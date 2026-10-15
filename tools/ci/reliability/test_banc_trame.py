"""L'A/B « une trame, une soumission » (BOUCHAUD_VIRTIO_GPU_TRAME_V1) : la
surete d'abord (aucune presentation perdue), puis les commandes et la latence.
Journaux synthetiques au format du banc."""
import importlib.util
import pathlib
import unittest

RACINE = pathlib.Path(__file__).resolve().parents[3]
_spec = importlib.util.spec_from_file_location("analyse_banc_trame", RACINE / "tools/ci/analyse_banc_trame.py")
a = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(a)

P = "[22:10:30][ 97%:  0%:  2%][FPS: --] "


def banc(t, v, p50, p95, ok=None, echecs=0, actif=1, cpt10=None, ecran="virtio-gpu-2d"):
    ok = (100 if v == "on" else 800) if ok is None else ok
    cpt10 = (50 if v == "on" else 160) if cpt10 is None else cpt10
    return [
        P + f"+ echo AB_TRAME tour={t} trame={v}",
        P + f"AB_TRAME tour={t} trame={v}",
        P + f"[BANC-TRAME] ecran={ecran} trame={v} n=100 regions=8 p50_us={p50} p95_us={p95} p99_us={p95} "
            f"max_us={p95} commandes={cpt10 * 10} commandes_par_trame_x10={cpt10} presentations_ok={ok} echecs={echecs} "
            f"pixels=1 actif={actif}",
    ]


def journal(tours=6, on=(400, 600), off=(600, 700), inverse=False, fin=True, **kw):
    l = []
    for t in range(1, tours + 1):
        ordre = ["off", "on"] if (t % 2) != inverse else ["on", "off"]
        for v in ordre:
            l += banc(t, v, *(on if v == "on" else off), **(kw if v == "on" else {}))
    l += [P + "+ echo AB_TRAME_FIN"] + ([P + "AB_TRAME_FIN"] if fin else [])
    return "\n".join(l) + "\n"


class BancTrame(unittest.TestCase):
    def test_moins_de_commandes_et_plus_rapide_est_convaincant(self):
        _, v, c = a.analyse(journal(), 6, "kvm")
        self.assertEqual((v, c), ("CONVAINCANT", 0))

    def test_un_gain_de_latence_insuffisant_n_est_pas_convaincant(self):
        _, v, _ = a.analyse(journal(on=(580, 600)), 6, "kvm")
        self.assertEqual(v, "NON_CONVAINCANT")

    def test_une_queue_degradee_n_est_pas_convaincante(self):
        _, v, _ = a.analyse(journal(on=(400, 900)), 6, "kvm")
        self.assertEqual(v, "NON_CONVAINCANT")

    def test_trop_de_commandes_n_est_pas_convaincant(self):
        _, v, _ = a.analyse(journal(cpt10=90), 6, "kvm")
        self.assertEqual(v, "NON_CONVAINCANT")

    def test_une_trame_perdue_est_un_defaut(self):
        _, v, c = a.analyse(journal(ok=99), 6, "kvm")
        self.assertEqual((v, c), ("DEFAUT", 1))

    def test_un_echec_est_un_defaut(self):
        _, v, c = a.analyse(journal(echecs=1), 6, "kvm")
        self.assertEqual((v, c), ("DEFAUT", 1))

    def test_un_scanout_abandonne_est_un_defaut(self):
        _, v, c = a.analyse(journal(actif=0), 6, "kvm")
        self.assertEqual((v, c), ("DEFAUT", 1))

    def test_un_autre_ecran_est_un_defaut(self):
        _, v, c = a.analyse(journal(ecran="autre"), 6, "kvm")
        self.assertEqual((v, c), ("DEFAUT", 1))

    def test_l_ordre_ab_ba_est_exige(self):
        _, v, c = a.analyse(journal(inverse=True), 6, "kvm")
        self.assertEqual((v, c), ("DEFAUT", 1))

    def test_la_trace_de_l_echo_ne_vaut_pas_fin(self):
        _, v, c = a.analyse(journal(fin=False), 6, "kvm")
        self.assertEqual((v, c), ("DEFAUT", 1))

    def test_moins_de_quatre_tours_ne_conclut_rien(self):
        _, v, c = a.analyse(journal(tours=3), 3, "kvm")
        self.assertEqual((v, c), ("NON_CONCLUANT", 0))


if __name__ == "__main__":
    unittest.main()
