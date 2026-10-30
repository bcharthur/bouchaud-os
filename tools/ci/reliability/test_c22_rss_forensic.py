import importlib.util
from pathlib import Path
import unittest

f = Path(__file__).resolve().parents[1] / "c22_rss_forensic.py"
spec = importlib.util.spec_from_file_location("c22_forensic", f)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

def ligne(t, rss, vma=88):
    return (
        f"[00:01:00] [PERF-PROC] t={t} pid=20 image=WebContent rss_kio={rss} vss_kio=123 taches=1\n"
        f"[00:01:00] [C22:RSS] t={t} pid=20 rss_kio={rss} anon_kio={rss-58424} fichier_kio=58144 partage_kio=280 device_kio=0 sans_type_kio=0 vma={vma}\n"
    )

def journal(m2=83216):
    return (ligne(1,67868) + "HOST_MEMOIRE_REPERE m=0 n=10\n"
            + ligne(2,80428) + "HOST_MEMOIRE_REPERE m=1 n=10\n"
            + ligne(3,m2) + "HOST_MEMOIRE_REPERE m=2 n=10\n"
            + ligne(4,m2+12,89))

class TestC22(unittest.TestCase):
    def test_rouge_pas_masque_par_echantillon_apres_m2(self):
        mesure = module.analyser(journal())
        self.assertEqual(mesure[2][0]-mesure[1][0],2788)
        self.assertEqual(mesure[2][1]-mesure[1][1],2788)
        self.assertEqual(mesure[2][6],88)
    def test_vert_et_valeur_apres_repere_ignoree(self):
        mesure = module.analyser(journal(79428))
        self.assertEqual(mesure[2][0]-mesure[1][0],-1000)
    def test_sans_categories_echec(self):
        txt = "\n".join(x for x in journal().splitlines() if "[C22:RSS]" not in x)
        with self.assertRaises(ValueError):
            module.analyser(txt)
    def test_desaccord_perf_echec(self):
        txt = journal().replace("t=3 pid=20 rss_kio=83216", "t=3 pid=20 rss_kio=84216",1)
        with self.assertRaises(ValueError):
            module.analyser(txt)
    def test_reperes_manquants_echec(self):
        with self.assertRaises(ValueError):
            module.analyser(journal().replace("REPERE m=1","ABSENT m=1"))

if __name__ == "__main__":
    unittest.main()
