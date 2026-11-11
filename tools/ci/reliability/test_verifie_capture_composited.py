"""BOUCHAUD_COMPOSITORD_PRESENTATION_V1 : le juge de la capture au pixel.

Revue de la PR #286 : un en-tete PPM tronque faisait boucler `lit_ppm` sans
fin ; le job CI n'aurait jamais rendu COMPOSITED_CAPTURE_ECHEC.
"""
import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

RACINE = Path(__file__).resolve().parents[3]
SCRIPT = RACINE / "tools/ci/verifie_capture_composited.py"
spec = importlib.util.spec_from_file_location("vcc", SCRIPT)
vcc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vcc)


def ppm(w, h, pixel):
    corps = bytes(c for y in range(h) for x in range(w) for c in pixel(x, y))
    return f"P6\n# capture\n{w} {h}\n255\n".encode() + corps


def lance(contenu, *zone):
    with tempfile.TemporaryDirectory() as d:
        f = Path(d, "c.ppm")
        f.write_bytes(contenu)
        r = subprocess.run([sys.executable, str(SCRIPT), str(f), *map(str, zone)],
                           capture_output=True, text=True, timeout=20)
        return r.returncode, r.stdout


class CaptureTests(unittest.TestCase):
    def test_en_tetes_tronques_echouent_sans_boucler(self):
        for brut in (b"", b"P6", b"P6\n", b"P6 16", b"P6 16 16", b"P6\n# rien\n"):
            with self.subTest(brut=brut):
                with self.assertRaises((ValueError, IndexError)):
                    vcc.lit_ppm_octets(brut)
                code, sortie = lance(brut, 0, 0, 1, 1)
                self.assertEqual(code, 1, sortie)
                self.assertIn("COMPOSITED_CAPTURE_ECHEC", sortie)

    def test_mire_exacte_passe(self):
        img = ppm(40, 30, lambda x, y: vcc.attendu(x - 4, y - 3) if x >= 4 and y >= 3 else (0, 0, 0))
        code, sortie = lance(img, 4, 3, 20, 10)
        self.assertEqual(code, 0, sortie)
        self.assertIn("exacts=200 faux=0", sortie)

    def test_un_pixel_faux_echoue(self):
        def p(x, y):
            return (1, 2, 3) if (x, y) == (10, 5) else vcc.attendu(x, y)
        code, sortie = lance(ppm(20, 10, p), 0, 0, 20, 10)
        self.assertEqual(code, 1, sortie)
        self.assertIn("faux=1", sortie)


if __name__ == "__main__":
    unittest.main()
