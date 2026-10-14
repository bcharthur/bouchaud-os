"""L'analyse de l'A/B scrutation / interruption virtio-gpu
(BOUCHAUD_VIRTIO_GPU_IRQ_V1) : verdicts et defauts, sur des journaux
fabriques. Aucun verdict de noyau n'est tire d'ici."""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import analyse_banc_irq_gpu as a  # noqa: E402

PREFIXE = "[22:10:30][ 97%:  0%:  2%][FPS: --] "
CAT = {"plein": ("1280x720", 40), "bande": ("1280x30", 200), "tuile": ("64x64", 400)}


def bras(tour, attente, p50=None, p95=None, inactif=None, cessions=None, sommeils=None, echues=0, relecture=0,
         acceptees=640, manque=None):
    p50 = p50 or {"plein": 1000, "bande": 200, "tuile": 100}
    p95 = p95 or {"plein": 1500, "bande": 300, "tuile": 150}
    inactif = inactif or ({"plein": 400, "bande": 300, "tuile": 200} if attente == "irq" else
                          {"plein": 0, "bande": 0, "tuile": 0})
    l = [PREFIXE + f"+ echo GPU_AB_BRAS tour={tour} attente={attente}", PREFIXE + f"GPU_AB_BRAS tour={tour} attente={attente}"]
    for c, (rect, n) in CAT.items():
        if c == manque:
            continue
        ces = cessions if cessions is not None else (0 if attente == "irq" else 500)
        som = sommeils if sommeils is not None else (n if attente == "irq" else 0)
        l.append(PREFIXE + f"[BANC-PRESENT] ecran=virtio-gpu-2d categorie={c} rect={rect} n={n} moyen_us={p50[c]} "
                 f"p50_us={p50[c]} p95_us={p95[c]} max_us={p95[c] * 2} mpix_s=10 contexte=tache attente={attente} "
                 f"cessions={ces} sommeils={som} irq={2 * n} irq_sans_progres=0 vus_par_relecture={relecture} "
                 f"echues={echues} coeur=0 inactif_pour_mille={inactif[c]}")
    l.append(PREFIXE + f"[BANC-PRESENT] ecran=virtio-gpu-2d fin presentations_virtio_ok={acceptees} echecs_virtio=0 "
             f"actif=1 contexte=tache attente={attente}")
    return l


def journal(tours=6, irq=None, scrute=None, msix=True, ordre_inverse=False):
    l = [PREFIXE + "VIRTIO_GPU_IRQ msix vecteur=0x50 apic=0 entrees=3 controlq=0 (achevement par interruption)"] if msix else []
    for t in range(1, tours + 1):
        ordre = ["scrute", "irq"] if (t % 2) != ordre_inverse else ["irq", "scrute"]
        for v in ordre:
            l += bras(t, v, **((irq or {}) if v == "irq" else (scrute or {})))
    return "\n".join(l) + "\n"


class AnalyseIrqGpu(unittest.TestCase):
    def test_convaincant(self):
        _, verdict, code = a.analyse(journal(), 6, "kvm")
        self.assertEqual((verdict, code), ("CONVAINCANT", 0))

    def test_latence_degradee_non_convaincant(self):
        _, verdict, code = a.analyse(journal(irq={"p95": {"plein": 1500, "bande": 600, "tuile": 150}}), 6, "kvm")
        self.assertEqual((verdict, code), ("NON_CONVAINCANT", 0))

    def test_sans_gain_processeur_non_convaincant(self):
        _, verdict, _ = a.analyse(journal(irq={"inactif": {"plein": 50, "bande": 300, "tuile": 200}}), 6, "kvm")
        self.assertEqual(verdict, "NON_CONVAINCANT")

    def test_interruptions_perdues_non_convaincant(self):
        _, verdict, _ = a.analyse(journal(irq={"relecture": 5}), 6, "kvm")
        self.assertEqual(verdict, "NON_CONVAINCANT")

    def test_moins_de_quatre_tours_non_concluant(self):
        _, verdict, code = a.analyse(journal(tours=3), 6, "kvm")
        self.assertEqual((verdict, code), ("NON_CONCLUANT", 0))

    def test_une_cession_sous_interruption_est_un_defaut(self):
        _, verdict, code = a.analyse(journal(irq={"cessions": 3}), 6, "kvm")
        self.assertEqual((verdict, code), ("DEFAUT", 1))

    def test_un_sommeil_sous_scrutation_est_un_defaut(self):
        _, verdict, code = a.analyse(journal(scrute={"sommeils": 1}), 6, "kvm")
        self.assertEqual((verdict, code), ("DEFAUT", 1))

    def test_une_attente_echue_est_un_defaut(self):
        _, verdict, code = a.analyse(journal(irq={"echues": 1}), 6, "kvm")
        self.assertEqual((verdict, code), ("DEFAUT", 1))

    def test_presentations_refusees_sont_un_defaut(self):
        _, verdict, code = a.analyse(journal(scrute={"acceptees": 639}), 6, "kvm")
        self.assertEqual((verdict, code), ("DEFAUT", 1))

    def test_categorie_absente_est_un_defaut(self):
        _, verdict, code = a.analyse(journal(irq={"manque": "tuile"}), 6, "kvm")
        self.assertEqual((verdict, code), ("DEFAUT", 1))

    def test_sans_msix_est_un_defaut(self):
        _, verdict, code = a.analyse(journal(msix=False), 6, "kvm")
        self.assertEqual((verdict, code), ("DEFAUT", 1))

    def test_ordre_non_alterne_est_un_defaut(self):
        _, verdict, code = a.analyse(journal(ordre_inverse=True), 6, "kvm")
        self.assertEqual((verdict, code), ("DEFAUT", 1))

    def test_panique_est_un_defaut(self):
        _, verdict, code = a.analyse(journal() + "panicked at src/x.rs:1:1\n", 6, "kvm")
        self.assertEqual((verdict, code), ("DEFAUT", 1))

    def test_l_echo_de_la_commande_n_ouvre_pas_de_bras(self):
        # Seule la SORTIE de `echo` ouvre un bras ; sinon chaque bras serait
        # compte deux fois.
        lignes, verdict, _ = a.analyse(journal(), 6, "kvm")
        self.assertIn("6/6 tours complets", lignes[0])
        self.assertEqual(verdict, "CONVAINCANT")


if __name__ == "__main__":
    unittest.main()
