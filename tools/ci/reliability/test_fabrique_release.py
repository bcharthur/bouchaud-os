"""BOUCHAUD_RELEASE_V1 : la fabrique de release refuse tout ce qu'elle doit refuser.

Cles generees au test (openssl), jamais stockees. La forme canonique produite
est la meme que celle que le noyau accepte : le banc A/B (run_ab_rollback.sh
mise-a-jour) installe un manifeste produit par CE script.
"""
import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

RACINE = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("fr", RACINE / "tools/release/fabrique_release.py")
fr = importlib.util.module_from_spec(spec)
sys.modules["fr"] = fr
spec.loader.exec_module(fr)


def cle(d: Path, nom: str) -> Path:
    p = d / f"{nom}.pem"
    subprocess.run(["openssl", "genpkey", "-algorithm", "ed25519", "-out", str(p)], check=True, capture_output=True)
    return p


class FabriqueReleaseTests(unittest.TestCase):
    def setUp(self):
        self.t = tempfile.TemporaryDirectory()
        self.d = Path(self.t.name)
        self.image = self.d / "system.img"
        self.image.write_bytes(b"\x55" * 8192)
        self.commit = "a" * 40

    def tearDown(self):
        self.t.cleanup()

    def lance(self, *args):
        return fr.main(["x", "--image", str(self.image), "--version", "2026.11.12-1", "--commit", self.commit,
                        "--canal", "testing", *args])

    def test_manifeste_canonique_v2(self):
        t = fr.manifeste_v2("v1.0", 7, "x86_64-uefi", "0" * 64, 4096, 3)
        self.assertEqual(t, b"bouchaud-manifeste-v2\nversion=v1.0\nsequence=7\nmateriel=x86_64-uefi\n"
                            b"image_sha256=" + b"0" * 64 + b"\nimage_octets=4096\nessais=3\n")
        for args in [("v 1", 7, "x86_64-uefi", "0" * 64, 1, 3), ("v", 0, "a", "0" * 64, 1, 3),
                     ("v", 1, "X86", "0" * 64, 1, 3), ("v", 1, "a", "0" * 63, 1, 3),
                     ("v", 1, "a", "0" * 64, 0, 3), ("v", 1, "a", "0" * 64, 1, 10)]:
            with self.assertRaises(fr.Refus, msg=str(args)):
                fr.manifeste_v2(*args)

    def test_release_dev_signee_et_verifiable(self):
        k = cle(self.d, "dev")
        s = self.d / "r"
        self.assertEqual(self.lance("--sequence", "5", "--sortie", str(s), "--cle-dev", str(k)), 0)
        d = json.loads((s / "release.json").read_text())
        self.assertTrue(d["signature"]["developpement"])
        self.assertEqual(d["sequence"], 5)
        self.assertEqual(d["image"]["octets"], 8192)
        self.assertTrue(fr.verifie(fr.publique_de(k), (s / "manifest").read_bytes(), (s / "manifest.sig").read_bytes()))
        sommes = (s / "SHA256SUMS").read_text()
        for f in ("manifest", "manifest.sig", "release.json", "system.img"):
            self.assertIn(f"  {f}\n", sommes)

    def test_sequence_non_croissante_refusee(self):
        k = cle(self.d, "dev")
        for prec in ("5", "6"):
            self.assertEqual(self.lance("--sequence", "5", "--sequence-precedente", prec,
                                        "--sortie", str(self.d / prec), "--cle-dev", str(k)), 2)
        self.assertEqual(self.lance("--sequence", "5", "--sequence-precedente", "4",
                                    "--sortie", str(self.d / "ok"), "--cle-dev", str(k)), 0)

    def test_sortie_jamais_reecrite(self):
        k = cle(self.d, "dev")
        s = self.d / "r"
        self.assertEqual(self.lance("--sequence", "5", "--sortie", str(s), "--cle-dev", str(k)), 0)
        avant = (s / "manifest.sig").read_bytes()
        self.assertEqual(self.lance("--sequence", "6", "--sortie", str(s), "--cle-dev", str(k)), 2)
        self.assertEqual((s / "manifest.sig").read_bytes(), avant)

    def test_sans_cle_refusee(self):
        self.assertEqual(self.lance("--sequence", "5", "--sortie", str(self.d / "r")), 2)

    def test_production_sans_cle_engagee_refusee(self):
        # Le depot n'engage aujourd'hui AUCUNE cle de production : refus.
        self.assertEqual(fr.cles_production(fr.CLES_PRODUCTION.read_text()), [])
        k = cle(self.d, "prod")
        self.assertEqual(self.lance("--sequence", "5", "--sortie", str(self.d / "r"), "--cle", str(k)), 2)
        self.assertFalse((self.d / "r").exists())

    def test_production_mauvais_secret_refuse(self):
        engagee = cle(self.d, "engagee")
        autre = cle(self.d, "autre")
        texte = f'pub const CLES_PRODUCTION: &[(&str, &str)] = &[\n    ("release-test", "{fr.publique_de(engagee)}"),\n];\n'
        rs = self.d / "cles.rs"
        rs.write_text(texte)
        ancien = fr.CLES_PRODUCTION
        fr.CLES_PRODUCTION = rs
        try:
            self.assertEqual(self.lance("--sequence", "5", "--sortie", str(self.d / "ko"), "--cle", str(autre)), 2)
            self.assertEqual(self.lance("--sequence", "5", "--sortie", str(self.d / "ok"), "--cle", str(engagee)), 0)
            d = json.loads((self.d / "ok" / "release.json").read_text())
            self.assertEqual(d["signature"]["cle"], "release-test")
            self.assertFalse(d["signature"]["developpement"])
        finally:
            fr.CLES_PRODUCTION = ancien

    def test_cles_commentees_ignorees(self):
        self.assertEqual(fr.cles_production('// ("x", "' + "a" * 64 + '"),\n'), [])


if __name__ == "__main__":
    unittest.main()
