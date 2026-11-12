"""BOUCHAUD_RELEASE_V1 (revue #306) : rien n'est publie qui differe de ce qui a ete construit et essaye.

Une release complete est fabriquee (cles de fixture, jamais celles du depot),
puis chaque regle du verificateur est violee une fois : chaque temoin doit
etre refuse.
"""
import hashlib
import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

RACINE = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(RACINE / "tools/release"))
spec = importlib.util.spec_from_file_location("va", RACINE / "tools/release/verifie_artefacts_release.py")
va = importlib.util.module_from_spec(spec)
sys.modules["va"] = va
spec.loader.exec_module(va)
fr = va.fr
COMMIT = "c" * 40


def cle(d: Path, nom: str) -> Path:
    p = d / f"{nom}.pem"
    subprocess.run(["openssl", "genpkey", "-algorithm", "ed25519", "-out", str(p)], check=True, capture_output=True)
    return p


def sha(p: Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest()


class VerifieArtefacts(unittest.TestCase):
    def setUp(self):
        self.t = tempfile.TemporaryDirectory()
        d = self.d = Path(self.t.name)
        e = d / "entree"
        e.mkdir()
        (e / "system-slot.img").write_bytes(b"\x11" * 8192)
        disque = e / "disque-ab.img"
        disque.write_bytes(b"\x22" * 16384)
        (e / "disque-ab.img.sha256").write_text(f"{sha(disque)}  disque-ab.img\n")
        (e / "manifest-ab.json").write_text(json.dumps({"commit": COMMIT, "image": {"sha256": sha(disque)}}))
        (e / "qemu-boot-evidence.json").write_text(json.dumps(
            {"build_sha": COMMIT, "qemu_boot": "PASS", "physical": "NOT_RUN"}))
        self.prod = cle(d, "prod")
        self.cles = f'pub const CLES_PRODUCTION: &[(&str, &str)] = &[\n    ("release-test", "{fr.publique_de(self.prod)}"),\n];\n'
        rs = d / "cles.rs"
        rs.write_text(self.cles)
        self.reel, fr.CLES_PRODUCTION = fr.CLES_PRODUCTION, rs
        self.r = d / "release"
        rc = fr.main(["x", "--image", str(e / "system-slot.img"), "--version", "v1", "--sequence", "7",
                      "--canal", "testing", "--commit", COMMIT, "--sortie", str(self.r), "--cle", str(self.prod),
                      *sum((["--joint", str(e / n)] for n in ("disque-ab.img", "disque-ab.img.sha256",
                                                             "manifest-ab.json", "qemu-boot-evidence.json")), [])])
        assert rc == 0
        (self.r / "system-slot.img").write_bytes((e / "system-slot.img").read_bytes())

    def tearDown(self):
        fr.CLES_PRODUCTION = self.reel
        self.t.cleanup()

    def ok(self):
        return va.verifie(self.r, True, self.cles)

    def refuse(self, motif):
        with self.assertRaises(va.Refus) as c:
            self.ok()
        self.assertIn(motif, str(c.exception))

    def test_release_complete_acceptee(self):
        self.assertEqual(self.ok()["sequence"], 7)

    def test_disque_modifie_par_l_essai_refuse(self):
        # Ce que faisait le smoke avant la revue #306 : le shim ecrit sur l'ESP.
        with open(self.r / "disque-ab.img", "r+b") as f:
            f.seek(100)
            f.write(b"BOUCHAUD-PREBOOT.TXT")
        self.refuse("empreinte_fausse")

    def test_disque_modifie_et_sha256sums_recalcule_refuse(self):
        with open(self.r / "disque-ab.img", "r+b") as f:
            f.write(b"X")
        lignes = [l for l in (self.r / "SHA256SUMS").read_text().splitlines() if not l.endswith("  disque-ab.img")]
        lignes.append(f"{sha(self.r / 'disque-ab.img')}  disque-ab.img")
        (self.r / "SHA256SUMS").write_text("\n".join(lignes) + "\n")
        self.refuse("disque_modifie_depuis_la_construction")

    def test_fichier_hors_empreinte_refuse(self):
        (self.r / "vars-qemu.fd").write_bytes(b"x")
        self.refuse("fichiers_inattendus")

    def test_fichier_manquant_refuse(self):
        (self.r / "qemu-boot-evidence.json").unlink()
        self.refuse("fichiers_inattendus")

    def test_image_de_slot_remplacee_refusee(self):
        (self.r / "system-slot.img").write_bytes(b"\x99" * 8192)
        self.refuse("empreinte_fausse")

    def test_preuve_qemu_d_un_autre_commit_refusee(self):
        p = self.r / "qemu-boot-evidence.json"
        p.write_text(json.dumps({"build_sha": "d" * 40, "qemu_boot": "PASS", "physical": "NOT_RUN"}))
        texte = (self.r / "SHA256SUMS").read_text().replace(
            [l for l in (self.r / "SHA256SUMS").read_text().splitlines() if l.endswith("qemu-boot-evidence.json")][0],
            f"{sha(p)}  qemu-boot-evidence.json")
        (self.r / "SHA256SUMS").write_text(texte)
        self.refuse("commits_differents")

    def test_signature_d_une_cle_non_engagee_refusee(self):
        autre = cle(self.d, "autre")
        self.cles = f'pub const CLES_PRODUCTION: &[(&str, &str)] = &[\n    ("x", "{fr.publique_de(autre)}"),\n];\n'
        self.refuse("signature_ne_verifie_avec_aucune_cle_engagee")

    def test_aucune_cle_engagee_refusee(self):
        self.cles = "pub const CLES_PRODUCTION: &[(&str, &str)] = &[\n];\n"
        self.refuse("signature_ne_verifie_avec_aucune_cle_engagee")

    def test_release_de_developpement_refusee_en_production(self):
        rel = json.loads((self.r / "release.json").read_text())
        rel["signature"]["developpement"] = True
        (self.r / "release.json").write_text(json.dumps(rel))
        with self.assertRaises(va.Refus):
            self.ok()


if __name__ == "__main__":
    unittest.main()
