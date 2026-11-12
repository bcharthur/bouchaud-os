"""BOUCHAUD_INSTALL_SSD_V1 : l'installateur ne choisit rien, ne devine rien, et refuse tout doute.

Aucun peripherique reel : la cible est un FICHIER, /sys et /proc sont des
arborescences factices (BO_INSTALL_RACINE_TEST). La GPT de l'image est
fabriquee ici, independamment de l'outil.
"""
import hashlib
import importlib.util
import io
import os
import struct
import sys
import tempfile
import unittest
import uuid
import zlib
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest import mock

RACINE = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("ins", RACINE / "tools/install/installe_ssd.py")
ins = importlib.util.module_from_spec(spec)
sys.modules["ins"] = ins
spec.loader.exec_module(ins)

S = 512
MIB = 1 << 20
ESP = uuid.UUID("C12A7328-F81F-11D2-BA4B-00A0C93EC93B").bytes_le
META = uuid.UUID("B0DC8A5D-0002-4B0D-9E21-424F55434844").bytes_le
STATE = uuid.UUID("B0DC8A5D-0001-4B0D-9E21-424F55434844").bytes_le


def image_gpt(chemin: Path, mio: int, parts) -> None:
    """GPT minimale : MBR protecteur, 128 entrees de 128 octets, secours en fin."""
    n = mio * MIB // S
    d = bytearray(mio * MIB)
    d[446 + 4] = 0xEE
    struct.pack_into("<II", d, 446 + 8, 1, n - 1)
    d[510:512] = b"\x55\xaa"
    ent = bytearray(128 * 128)
    for i, (ty, prem, dern) in enumerate(parts):
        o = i * 128
        ent[o:o + 16] = ty
        ent[o + 16:o + 32] = uuid.uuid4().bytes_le
        struct.pack_into("<QQ", ent, o + 32, prem, dern)
        # contenu reconnaissable dans chaque partition
        d[prem * S:prem * S + 16] = bytes([0x40 + i]) * 16
    crc_e = zlib.crc32(ent) & 0xFFFFFFFF

    def entete(cur, sec, lba_e):
        h = bytearray(92)
        h[0:8] = b"EFI PART"
        struct.pack_into("<IIIIQQQQ", h, 8, 0x10000, 92, 0, 0, cur, sec, 34, n - 34)
        h[56:72] = uuid.uuid4().bytes_le
        struct.pack_into("<QIII", h, 72, lba_e, 128, 128, crc_e)
        struct.pack_into("<I", h, 16, zlib.crc32(h) & 0xFFFFFFFF)
        return bytes(h)
    d[2 * S:2 * S + len(ent)] = ent
    d[S:S + 92] = entete(1, n - 1, 2)
    d[(n - 33) * S:(n - 33) * S + len(ent)] = ent
    d[(n - 1) * S:(n - 1) * S + 92] = entete(n - 1, 1, n - 33)
    chemin.write_bytes(bytes(d))
    chemin.with_name(chemin.name + ".sha256").write_text(hashlib.sha256(bytes(d)).hexdigest() + f"  {chemin.name}\n")


class Installateur(unittest.TestCase):
    def setUp(self):
        self.t = tempfile.TemporaryDirectory()
        r = self.r = Path(self.t.name)
        self.image = r / "img" / "disque-ab.img"
        self.image.parent.mkdir()
        # ESP 1..2 Mio, META 2..3 Mio, STATE 3..6 Mio (derniere).
        image_gpt(self.image, 8, [(ESP, 2048, 4095), (META, 4096, 6143), (STATE, 6144, 12287)])
        self.ssd = r / "sdz"
        self.ssd.write_bytes(b"\xAA" * (32 * MIB))
        self.sysfs("sdz", "SSD Dedie", "S3RIE42", 32 * MIB)
        self.sysfs("sda", "Disque Systeme", "SYS0001", 64 * MIB, parts=["sda1", "sda2"])
        (r / "proc").mkdir()
        (r / "proc/mounts").write_text("/dev/sda2 / ext4 rw 0 0\n/dev/sda1 /boot/efi vfat rw 0 0\n")
        (r / "proc/swaps").write_text("Filename Type Size Used Priority\n")
        self.env = mock.patch.dict(os.environ, {"BO_INSTALL_RACINE_TEST": str(r)})
        self.env.start()
        self.id = f"SSD Dedie|S3RIE42|{32 * MIB}"

    def tearDown(self):
        self.env.stop()
        self.t.cleanup()

    def sysfs(self, nom, modele, serie, octets, parts=()):
        d = self.r / "sys/block" / nom
        (d / "device").mkdir(parents=True)
        (d / "device/model").write_text(modele + "\n")
        (d / "device/serial").write_text(serie + "\n")
        (d / "size").write_text(str(octets // S))
        (d / "removable").write_text("0")
        for p in parts:
            (d / p).mkdir()
            (d / p / "partition").write_text("1")

    def lance(self, *args, entree=""):
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err), mock.patch("sys.stdin", io.StringIO(entree)):
            rc = ins.main(["x", *args])
        return rc, out.getvalue(), err.getvalue()

    def installe(self, cible=None, identite=None, execute=True, entree="EFFACER S3RIE42\n"):
        a = ["installe", "--image", str(self.image), "--cible", str(cible or self.ssd), "--identite", identite or self.id]
        return self.lance(*a, *(["--execute"] if execute else []), entree=entree)

    def intact(self):
        return self.ssd.read_bytes() == b"\xAA" * (32 * MIB)

    # --- ce qui ecrit -------------------------------------------------------

    def test_installation_complete_puis_state_etendue(self):
        rc, out, err = self.installe()
        self.assertEqual(rc, 0, err)
        self.assertIn("INSTALL_OK", out)
        self.assertIn("state_mio=3->", out)
        d = self.ssd.read_bytes()
        img = self.image.read_bytes()
        # Les partitions ESP et META sont octet pour octet celles de l'image.
        self.assertEqual(d[2048 * S:6144 * S], img[2048 * S:6144 * S])
        with open(self.ssd, "rb") as f:
            e, ent = ins.lit_gpt(f)
            parts = ins.partitions(e, ent)
        etat = [p for p in parts if p[1] == STATE][0]
        n = 32 * MIB // S
        self.assertGreater(etat[3], 12287)
        self.assertEqual((etat[3] + 1) % 2048, 0, "fin alignee sur 1 Mio")
        self.assertLessEqual(etat[3], n - 34)
        self.assertEqual(e["dernier"], n - 34)
        # Le secours est en fin de disque et valide.
        with open(self.ssd, "rb") as f:
            f.seek((n - 1) * S)
            s = ins._entete(f.read(S))
        self.assertEqual((s["courant"], s["secours"]), (n - 1, 1))
        # MBR protecteur a la taille du disque.
        self.assertEqual(struct.unpack_from("<I", d, 446 + 12)[0], n - 1)

    def test_dry_run_par_defaut_n_ecrit_rien(self):
        rc, out, _ = self.installe(execute=False)
        self.assertEqual(rc, 0)
        self.assertIn("INSTALL_DRY_RUN", out)
        self.assertTrue(self.intact())

    # --- temoins de refus : RIEN n'est ecrit ----------------------------------

    def refuse(self, motif, **kw):
        rc, _, err = self.installe(**kw)
        self.assertEqual(rc, 2, err)
        self.assertIn(motif, err)
        self.assertTrue(self.intact(), "un refus a ecrit sur la cible")

    def test_confirmation_absente_ou_fausse(self):
        self.refuse("confirmation", entree="")
        self.refuse("confirmation", entree="oui\n")
        self.refuse("confirmation", entree="EFFACER SYS0001\n")

    def test_identite_differente(self):
        self.refuse("identite_differente", identite=f"SSD Dedie|AUTRE|{32 * MIB}")
        self.refuse("identite_differente", identite=f"SSD Dedie|S3RIE42|{16 * MIB}")
        self.refuse("identite_differente", identite="SSD Dedie|S3RIE42")

    def test_disque_systeme_refuse(self):
        sda = self.r / "sda"
        sda.write_bytes(b"\0" * MIB)
        rc, _, err = self.lance("installe", "--image", str(self.image), "--cible", str(sda),
                                "--identite", f"Disque Systeme|SYS0001|{64 * MIB}", "--execute",
                                entree="EFFACER SYS0001\n")
        self.assertEqual(rc, 2)
        self.assertIn("partition_montee /dev/sda2 sur / (disque systeme)", err)

    def test_swap_refuse(self):
        (self.r / "proc/swaps").write_text("Filename Type Size Used Priority\n/dev/sdz partition 1 0 -2\n")
        self.refuse("swap_actif")

    def test_partition_montee_refusee(self):
        (self.r / "sys/block/sdz/sdz1").mkdir()
        (self.r / "sys/block/sdz/sdz1/partition").write_text("1")
        (self.r / "proc/mounts").write_text("/dev/sdz1 /media/x vfat rw 0 0\n")
        self.refuse("partition_montee /dev/sdz1")

    def test_serie_illisible_refusee(self):
        (self.r / "sys/block/sdz/device/serial").unlink()
        self.refuse("numero_de_serie_illisible", identite=f"SSD Dedie|?|{32 * MIB}")

    def test_image_alteree_refusee(self):
        with open(self.image, "r+b") as f:
            f.seek(5000)
            f.write(b"X")
        self.refuse("empreinte_image_differente")

    def test_image_plus_grande_que_le_disque(self):
        (self.r / "sys/block/sdz/size").write_text(str(4 * MIB // S))
        self.refuse("image_plus_grande_que_le_disque", identite=f"SSD Dedie|S3RIE42|{4 * MIB}")

    def test_disque_inconnu_et_vrai_peripherique_en_mode_test(self):
        autre = self.r / "sdq"
        autre.write_bytes(b"\0" * MIB)
        rc, _, err = self.installe(cible=autre)
        self.assertEqual(rc, 2)
        self.assertIn("disque_inconnu", err)
        rc, _, err = self.installe(cible="/dev/sda")
        self.assertEqual(rc, 2)
        self.assertIn("mode_test_sur_un_vrai_peripherique", err)

    def test_derniere_partition_pas_state(self):
        image_gpt(self.image, 8, [(ESP, 2048, 4095), (STATE, 4096, 6143), (META, 6144, 12287)])
        self.refuse("derniere_partition_n_est_pas_state")

    def test_liste_ne_choisit_rien(self):
        rc, out, _ = self.lance("liste")
        self.assertEqual(rc, 0)
        self.assertIn(f"identite='SSD Dedie|S3RIE42|{32 * MIB}'", out)
        self.assertIn("Aucun disque n'est choisi automatiquement", out)
        self.assertTrue(self.intact())


if __name__ == "__main__":
    unittest.main()
