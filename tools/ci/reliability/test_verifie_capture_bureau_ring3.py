"""BOUCHAUD_BUREAU_RING3_2A/2B : le juge de l'ecran compose par `composited`.

Le banc ne vaut que si le juge REFUSE ce qui doit l'etre : contenu non recopie
(la premiere version du compositeur perdait la trame complete du demarrage),
pixel decale d'une case, trace d'un autre compositeur, fenetre restee apres la
mort du client.
"""
import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

RACINE = Path(__file__).resolve().parents[3]
SCRIPT = RACINE / "tools/ci/verifie_capture_bureau_ring3.py"
spec = importlib.util.spec_from_file_location("vcb", SCRIPT)
vcb = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vcb)

ECRAN = (50, 50)
FEN = (4, 28, 10, 8)  # contenu ; cadre de (2, 2) a (16, 38)


def bureau(x, y, contenu):
    x0, y0, l, h = FEN
    if x0 <= x < x0 + l and y0 <= y < y0 + h:
        return contenu(x - x0, y - y0)
    if x0 - 2 <= x < x0 + l + 2 and y0 - 26 <= y < y0 + h + 2:
        return vcb.BARRE if (y0 - 24 <= y < y0 and x0 <= x < x0 + l) else vcb.BORD
    return vcb.FOND


def ppm(pixel):
    w, h = ECRAN
    corps = bytearray()
    for y in range(h):
        for x in range(w):
            v = pixel(x, y)
            corps += bytes(((v >> 16) & 0xFF, (v >> 8) & 0xFF, v & 0xFF))
    return f"P6\n# capture\n{w} {h}\n255\n".encode() + bytes(corps)


def lance(contenu, etat, fenetres=None):
    args = ["vide"] if etat == "mort" else [",".join(map(str, f)) for f in (fenetres or [FEN])]
    with tempfile.TemporaryDirectory() as d:
        f = Path(d, "c.ppm")
        f.write_bytes(contenu)
        r = subprocess.run([sys.executable, str(SCRIPT), str(f), *args],
                           capture_output=True, text=True, timeout=20)
        return r.returncode, r.stdout


# BOUCHAUD_BUREAU_RING3_2B : deux fenetres qui se recouvrent, ecran 60x80.
ECRAN2 = (60, 80)
BAS = (4, 28, 20, 20)    # cadre (2, 2)-(26, 50)
HAUT = (14, 52, 20, 20)  # cadre (12, 26)-(36, 74) : recouvre le bas de BAS


def pile(x, y, fenetres, deborde=False):
    """L'ecran attendu ; `deborde` : la fenetre du fond est peinte PAR-DESSUS."""
    ordre = list(enumerate(fenetres))
    if deborde:
        ordre = ordre[::-1]
    for k, (fx, fy, fl, fh) in reversed(ordre):
        if fx - 2 <= x < fx + fl + 2 and fy - 26 <= y < fy + fh + 2:
            if fx <= x < fx + fl and fy <= y < fy + fh:
                return vcb.motif(x - fx, y - fy)
            if fy - 24 <= y < fy and fx <= x < fx + fl:
                return vcb.BARRE if k == len(fenetres) - 1 else vcb.BARRE_INACTIVE
            return vcb.BORD
    return vcb.FOND


def ppm2(pixel):
    w, h = ECRAN2
    corps = bytearray()
    for y in range(h):
        for x in range(w):
            v = pixel(x, y)
            corps += bytes(((v >> 16) & 0xFF, (v >> 8) & 0xFF, v & 0xFF))
    return f"P6\n{w} {h}\n255\n".encode() + bytes(corps)


class CaptureBureauTests(unittest.TestCase):
    def test_fenetre_exacte_acceptee(self):
        code, sortie = lance(ppm(lambda x, y: bureau(x, y, vcb.motif)), "vivant")
        self.assertEqual(code, 0, sortie)
        self.assertIn("BUREAU_RING3_CAPTURE_OK", sortie)

    def test_carre_mobile_admis(self):
        def avec_carre(cx, cy):
            return vcb.BLANC if cx < 3 and cy < 3 else vcb.motif(cx, cy)
        code, sortie = lance(ppm(lambda x, y: bureau(x, y, avec_carre)), "vivant")
        self.assertEqual(code, 0, sortie)
        self.assertIn("carre=9", sortie)

    def test_contenu_non_recopie_refuse(self):
        # La premiere version perdait la trame complete : zone vide du cadre.
        def vide(cx, cy):
            return 0x202020 if cy >= 4 else vcb.motif(cx, cy)
        code, sortie = lance(ppm(lambda x, y: bureau(x, y, vide)), "vivant")
        self.assertEqual(code, 1, sortie)
        self.assertIn("BUREAU_RING3_CAPTURE_ECHEC", sortie)

    def test_pixel_decale_refuse(self):
        code, sortie = lance(ppm(lambda x, y: bureau(x, y, lambda cx, cy: vcb.motif(cx + 1, cy))), "vivant")
        self.assertEqual(code, 1, sortie)

    def test_trace_hors_fenetre_refusee(self):
        def trace(x, y):
            return 0x123456 if (x, y) == (45, 45) else bureau(x, y, vcb.motif)
        code, sortie = lance(ppm(trace), "vivant")
        self.assertEqual(code, 1, sortie)
        self.assertIn("(45,45)", sortie)

    def test_cadre_absent_refuse(self):
        def sans_cadre(x, y):
            x0, y0, l, h = FEN
            if x0 <= x < x0 + l and y0 <= y < y0 + h:
                return vcb.motif(x - x0, y - y0)
            return vcb.FOND
        code, sortie = lance(ppm(sans_cadre), "vivant")
        self.assertEqual(code, 1, sortie)

    def test_mort_exige_l_ecran_vide(self):
        code, sortie = lance(ppm(lambda x, y: vcb.FOND), "mort")
        self.assertEqual(code, 0, sortie)
        code, sortie = lance(ppm(lambda x, y: bureau(x, y, vcb.motif)), "mort")
        self.assertEqual(code, 1, sortie)

    def test_deux_fenetres_empilees_acceptees(self):
        code, sortie = lance(ppm2(lambda x, y: pile(x, y, [BAS, HAUT])), "vivant", [BAS, HAUT])
        self.assertEqual(code, 0, sortie)
        self.assertIn("fenetres=2", sortie)

    def test_fenetre_du_fond_qui_deborde_refusee(self):
        # La fenetre du fond peinte PAR-DESSUS celle du premier plan.
        code, sortie = lance(ppm2(lambda x, y: pile(x, y, [BAS, HAUT], deborde=True)), "vivant", [BAS, HAUT])
        self.assertEqual(code, 1, sortie)

    def test_ordre_inverse_annonce_refuse(self):
        # Ecran correct pour [BAS, HAUT], juge avec l'ordre inverse.
        code, sortie = lance(ppm2(lambda x, y: pile(x, y, [BAS, HAUT])), "vivant", [HAUT, BAS])
        self.assertEqual(code, 1, sortie)

    def test_barre_active_sur_la_mauvaise_fenetre_refusee(self):
        def barres_inversees(x, y):
            v = pile(x, y, [BAS, HAUT])
            return {vcb.BARRE: vcb.BARRE_INACTIVE, vcb.BARRE_INACTIVE: vcb.BARRE}.get(v, v)
        code, sortie = lance(ppm2(barres_inversees), "vivant", [BAS, HAUT])
        self.assertEqual(code, 1, sortie)

    def test_fenetre_opaque_peinte_acceptee(self):
        # DOOM/Ladybird : contenu quelconque, mais peint.
        def jeu(x, y):
            v = pile(x, y, [BAS, HAUT])
            fx, fy, fl, fh = HAUT
            return 0x7F3F1F if fx <= x < fx + fl and fy <= y < fy + fh else v
        code, sortie = lance(ppm2(jeu), "vivant", [BAS, HAUT + ("o",)])
        self.assertEqual(code, 0, sortie)

    def test_fenetre_opaque_non_peinte_refusee(self):
        def vide(x, y):
            v = pile(x, y, [BAS, HAUT])
            fx, fy, fl, fh = HAUT
            return vcb.FOND if fx <= x < fx + fl and fy <= y < fy + fh else v
        code, sortie = lance(ppm2(vide), "vivant", [BAS, HAUT + ("o",)])
        self.assertEqual(code, 1, sortie)
        self.assertIn("non-peinte", sortie)

    def test_ppm_tronque_echoue_sans_boucler(self):
        for brut in (b"", b"P6", b"P6\n# rien", b"P6 50 50 255\n\x00"):
            with self.subTest(brut=brut):
                code, _ = lance(brut, "mort")
                self.assertNotEqual(code, 0)


if __name__ == "__main__":
    unittest.main()
