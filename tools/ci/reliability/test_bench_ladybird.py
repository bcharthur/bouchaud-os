"""BOUCHAUD_LADYBIRD_BENCH_V1 : le banc A/B juge sur ce que les journaux
contiennent, y compris quand un smoke court ne produit qu'UN releve.

Regression du run 42842860256 (SHA 93d9421) : un releve unique par bras
etait ecarte, le banc concluait a tort `le bras GPU n'a pas compose`, et le
`| tee` sans pipefail rendait l'etape verte quand meme."""
import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

RACINE = Path(__file__).resolve().parents[3]
BANC = RACINE / "tools/ci/bench_ladybird.py"
spec = importlib.util.spec_from_file_location("bench_ladybird", BANC)
banc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(banc)


def releve(px_gpu, copies=310596):
    return ("[17:20:59] [GPU-COMPOSITION] disponible=1 periode_ms=30000 trames_bureau=112 "
            f"trames_clients=93 trames_gpu=111 blits=444 px_poses_gpu={px_gpu} "
            f"px_client_copies_cpu={copies} us_dessin_cpu_par_trame=900 us_presentation_par_trame=8000\n")


def lance(on, off):
    with tempfile.TemporaryDirectory() as d:
        a, b = Path(d, "on.log"), Path(d, "off.log")
        a.write_text(on)
        b.write_text(off)
        r = subprocess.run([sys.executable, str(BANC), str(a), str(b)], capture_output=True, text=True)
        return r.returncode, r.stdout


class TestBenchLadybird(unittest.TestCase):
    def test_releve_unique_retenu(self):
        m = banc.mesures(releve(69929112))
        self.assertEqual(m["releves_gpu_composition"], 1)
        self.assertEqual(m["px_poses_gpu_30s"], 69929112)

    def test_premier_releve_ecarte_s_il_y_en_a_d_autres(self):
        m = banc.mesures(releve(1) + releve(500) + releve(700))
        self.assertEqual(m["releves_gpu_composition"], 2)
        self.assertEqual(m["px_poses_gpu_30s"], 600)

    def test_ab_reel_vert(self):
        rc, out = lance(releve(69929112), releve(0))
        self.assertEqual(rc, 0, out)
        self.assertIn("LADYBIRD_BENCH_OK", out)

    def test_bras_gpu_sans_pose_rouge(self):
        rc, out = lance(releve(0), releve(0))
        self.assertEqual(rc, 1)
        self.assertIn("LADYBIRD_BENCH_ECHEC le bras GPU", out)

    def test_bras_cpu_qui_pose_par_le_gpu_rouge(self):
        rc, out = lance(releve(5), releve(5))
        self.assertEqual(rc, 1)
        self.assertIn("LADYBIRD_BENCH_ECHEC le bras CPU", out)

    def test_aucun_releve_rouge(self):
        rc, out = lance("rien\n", "rien\n")
        self.assertEqual(rc, 1)

    def test_etape_ci_propage_l_echec(self):
        w = (RACINE / ".github/workflows/gpu-composition.yml").read_text()
        i = w.index("bench_ladybird.py serie-browser-host.log")
        etape = w[w.rindex("- name:", 0, i):i]
        self.assertIn("set -o pipefail", etape)


if __name__ == "__main__":
    unittest.main()
