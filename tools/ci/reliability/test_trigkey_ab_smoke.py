"""BOUCHAUD_RELEASE_V1 (revue #306) : le smoke QEMU ne modifie JAMAIS l'image publiable.

Un faux `qemu-system-x86_64` (place en tete du PATH) fait ce que font le
micrologiciel et le shim : il ECRIT dans le disque qu'on lui donne, puis
emet un journal de sante valide. Le smoke doit :
  - lui donner une copie jetable, jamais `disque-ab.img` ;
  - constater ensuite que l'empreinte de l'original est intacte ;
  - echouer (`image_originale_modifiee`) si l'original a change quand meme.
"""
import hashlib
import json
import os
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path

RACINE = Path(__file__).resolve().parents[3]
SMOKE = RACINE / "tools/release/smoke_trigkey_ab_qemu.sh"

FAUX_QEMU = textwrap.dedent('''\
    #!/usr/bin/env python3
    import os, sys, time
    args = sys.argv[1:]
    disque = journal = None
    for i, a in enumerate(args):
        if a.startswith("if=none,id=disque"):
            disque = [p[5:] for p in a.split(",") if p.startswith("file=")][0]
        if a == "-serial":
            journal = args[i + 1][5:]
    open(os.environ["FAUX_QEMU_VU"], "w").write(disque)
    with open(disque, "r+b") as f:          # le shim ecrit sur l'ESP qu'il amorce
        f.seek(4096); f.write(b"BOUCHAUD-PREBOOT.TXT ecrit par l'essai")
    cible = os.environ.get("FAUX_QEMU_ECRIT_AUSSI")
    if cible:                                # temoin : quelqu'un touche l'original
        with open(cible, "r+b") as f:        # un octet GARANTI different : ecrire
            f.seek(8192); o = f.read(1)[0]   # b"X" sur une image aleatoire ne changeait
            f.seek(8192); f.write(bytes([o ^ 0xFF]))  # rien une fois sur 256
    with open(journal, "w") as j:
        j.write("BOUCHAUD_AB_BOOT slot=A\\nBOUCHAUD_HWPROBE_BEGIN\\n"
                "SERVICED_UPDATE_STATE rc=0\\nSERVICED_HEALTH_COMMIT result=already_good\\n")
    time.sleep(120)
''')


class SmokeNeModifiePasLImage(unittest.TestCase):
    def setUp(self):
        self.t = tempfile.TemporaryDirectory()
        d = Path(self.t.name)
        self.out = d / "out"
        self.out.mkdir()
        self.disque = self.out / "disque-ab.img"
        self.disque.write_bytes(os.urandom(64 * 1024))
        h = hashlib.sha256(self.disque.read_bytes()).hexdigest()
        (self.out / "disque-ab.img.sha256").write_text(f"{h}  disque-ab.img\n")
        (self.out / "manifest-ab.json").write_text(json.dumps(
            {"proof_level": "BUILD_ONLY", "physical": "NOT_RUN", "commit": "a" * 40}))
        self.empreinte = h
        ovmf = d / "ovmf"
        ovmf.mkdir()
        (ovmf / "OVMF_CODE_4M.fd").write_bytes(b"\0" * 16)
        (ovmf / "OVMF_VARS_4M.fd").write_bytes(b"\0" * 16)
        bin_ = d / "bin"
        bin_.mkdir()
        q = bin_ / "qemu-system-x86_64"
        q.write_text(FAUX_QEMU)
        q.chmod(0o755)
        self.vu = d / "vu.txt"
        self.env = dict(os.environ, PATH=f"{bin_}:{os.environ['PATH']}", BO_OVMF_DIR=str(ovmf),
                        FAUX_QEMU_VU=str(self.vu), BO_TRIGKEY_QEMU_S="30", BO_TRIGKEY_QEMU_GRACE_S="0")

    def tearDown(self):
        self.t.cleanup()

    def lance(self, **env):
        return subprocess.run(["bash", str(SMOKE), str(self.out)], capture_output=True, text=True,
                              env=dict(self.env, **env), timeout=120)

    def test_essai_sur_copie_original_intact(self):
        r = self.lance()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("TRIGKEY_QEMU_BOOT_OK", r.stdout)
        vu = Path(self.vu.read_text())
        self.assertNotEqual(vu.resolve(), self.disque.resolve(), "QEMU a recu l'image publiable")
        self.assertFalse(vu.exists(), "la copie jetable doit etre effacee")
        self.assertEqual(hashlib.sha256(self.disque.read_bytes()).hexdigest(), self.empreinte)

    def test_temoin_original_modifie_refuse(self):
        r = self.lance(FAUX_QEMU_ECRIT_AUSSI=str(self.disque))
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("image_originale_modifiee", r.stderr)
        self.assertNotIn("TRIGKEY_QEMU_BOOT_OK", r.stdout)

    def test_empreinte_initiale_fausse_refusee(self):
        (self.out / "disque-ab.img.sha256").write_text("0" * 64 + "  disque-ab.img\n")
        r = self.lance()
        self.assertEqual(r.returncode, 2)
        self.assertIn("empreinte_initiale_differente", r.stderr)
        self.assertFalse(self.vu.exists(), "QEMU ne doit pas etre lance")


if __name__ == "__main__":
    unittest.main()
