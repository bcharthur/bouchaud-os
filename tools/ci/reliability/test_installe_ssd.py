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

    # --- P1 revue #307 : dependances de stockage indirectes --------------------

    def partition(self, disque, part):
        d = self.r / "sys/block" / disque / part
        (d / "holders").mkdir(parents=True, exist_ok=True)
        (d / "partition").write_text("1")
        return d

    def mappe(self, dm, nom, esclaves, prefixe="dm"):
        d = self.r / "sys/block" / dm
        (d / "slaves").mkdir(parents=True, exist_ok=True)
        (d / "holders").mkdir(exist_ok=True)
        if prefixe == "dm":
            (d / "dm").mkdir(exist_ok=True)
            (d / "dm/name").write_text(nom + "\n")
        (d / "size").write_text("1000")
        for e in esclaves:
            (d / "slaves" / e).mkdir()
            disque = e.rstrip("0123456789") if not e.startswith(("dm-", "md")) else e
            h = (self.r / "sys/block" / disque / e / "holders") if disque != e else (self.r / "sys/block" / e / "holders")
            h.mkdir(parents=True, exist_ok=True)
            (h / dm).mkdir(exist_ok=True)

    def monte(self, *lignes):
        (self.r / "proc/mounts").write_text("/dev/sda2 / ext4 rw 0 0\n" + "".join(l + "\n" for l in lignes))

    def test_dm_crypt_monte_sur_le_disque_refuse(self):
        # Le cas de la revue : /dev/mapper/cryptroot (dm-0 sur sdz1) monte sur /.
        self.partition("sdz", "sdz1")
        self.mappe("dm-0", "cryptroot", ["sdz1"])
        self.monte("/dev/mapper/cryptroot /data ext4 rw 0 0")
        self.refuse("partition_montee /dev/mapper/cryptroot sur /data")

    def test_lvm_sur_dm_crypt_chaine_profonde_refuse(self):
        self.partition("sdz", "sdz2")
        self.mappe("dm-0", "luks-x", ["sdz2"])
        self.mappe("dm-1", "vg-home", ["dm-0"])
        self.monte("/dev/dm-1 /home ext4 rw 0 0")
        self.refuse("partition_montee /dev/dm-1 sur /home")

    def test_raid_md_refuse(self):
        self.partition("sdz", "sdz1")
        self.mappe("md0", "", ["sdz1"], prefixe="md")
        self.monte("/dev/md0 /srv ext4 rw 0 0")
        self.refuse("partition_montee /dev/md0 sur /srv")

    def test_swap_sur_device_mapper_refuse(self):
        self.partition("sdz", "sdz3")
        self.mappe("dm-2", "swapcrypt", ["sdz3"])
        (self.r / "proc/swaps").write_text("Filename Type Size Used Priority\n/dev/mapper/swapcrypt partition 1 0 -2\n")
        self.refuse("swap_actif /dev/mapper/swapcrypt")

    def test_pile_ouverte_non_montee_refusee(self):
        # LVM actif ou dm-crypt ouvert, rien de monte : un holder suffit.
        self.partition("sdz", "sdz1")
        self.mappe("dm-0", "vg-inactif", ["sdz1"])
        self.refuse("detenu_par dm-0 (sdz1)")

    def test_source_non_resolue_refusee(self):
        self.monte("/dev/mapper/inconnu /x ext4 rw 0 0")
        self.refuse("source_non_resolue /dev/mapper/inconnu")
        self.monte("/dev/root /y ext4 rw 0 0")
        self.refuse("source_non_resolue /dev/root")

    def test_membre_btrfs_refuse(self):
        self.partition("sdz", "sdz1")
        (self.r / "sys/fs/btrfs/uuid-1/devices/sdz1").mkdir(parents=True)
        self.refuse("membre_btrfs sdz1")

    def test_piles_sur_un_AUTRE_disque_acceptees(self):
        # Positif : le systeme (dm-crypt + LVM + swap chiffre) vit sur sda ;
        # le SSD dedie sdz est libre -> l'installation passe.
        self.mappe("dm-0", "cryptroot", ["sda2"])
        self.mappe("dm-1", "vg-root", ["dm-0"])
        self.mappe("dm-2", "swapcrypt", ["sda1"])
        self.monte("/dev/mapper/vg-root /home ext4 rw 0 0")
        (self.r / "proc/swaps").write_text("Filename Type Size Used Priority\n/dev/dm-2 partition 1 0 -2\n")
        rc, out, err = self.installe()
        self.assertEqual(rc, 0, err)
        self.assertIn("INSTALL_OK", out)

    # --- P1 revue #307 : zone persistante finale apres extension --------------

    def test_ancienne_zone_persistante_effacee_apres_extension(self):
        # Un SSD de 256 Mio qui a deja porte une installation : un en-tete de
        # persistance et un fichier a l'endroit ou le NOUVEAU STATE etendu aura
        # sa zone (ses 128 derniers Mio), et des octets au debut de STATE.
        octets = 256 * MIB
        with open(self.ssd, "wb") as f:
            f.truncate(octets)
        n = octets // S
        (self.r / "sys/block/sdz/size").write_text(str(n))
        ident = f"SSD Dedie|S3RIE42|{octets}"
        ancien = b"BOPERSI1" + b"\x01" * 504
        with open(self.ssd, "r+b") as f:
            for lba in range(n - 34 - ins.SECTEURS_ZONE - 4096, n - 34, 7919):
                f.seek(lba * S)
                f.write(ancien)
        rc, out, err = self.installe(identite=ident)
        self.assertEqual(rc, 0, err)
        self.assertIn("zone_persistante=initialisee", out)
        with open(self.ssd, "rb") as f:
            e, ent = ins.lit_gpt(f)
            etat = [p for p in ins.partitions(e, ent) if p[1] == STATE][0]
            prem, dern = etat[2], etat[3]
            f.seek((dern + 1 - ins.SECTEURS_ZONE) * S)
            zone = f.read(ins.SECTEURS_ZONE * S)
            f.seek(prem * S)
            debut = f.read(MIB)
        self.assertEqual(zone.count(0), len(zone), "une ancienne zone persistante a survecu")
        self.assertNotIn(b"BOPERSI1", zone)
        self.assertEqual(debut.count(0), len(debut))

    def test_sans_initialisation_l_ancienne_zone_survivrait(self):
        # Temoin du temoin : l'extension SEULE laisse l'ancienne zone en place
        # -- c'est exactement le defaut de la revue.
        octets = 256 * MIB
        with open(self.ssd, "wb") as f:
            f.truncate(octets)
        (self.r / "sys/block/sdz/size").write_text(str(octets // S))
        with open(self.ssd, "r+b") as f:
            f.write(self.image.read_bytes())
            f.seek(octets - 64 * MIB)
            f.write(b"BOPERSI1-ancienne")
            avant, apres, prem, dern = ins.etend_state(f, octets)
            f.seek(octets - 64 * MIB)
            self.assertEqual(f.read(17), b"BOPERSI1-ancienne")
            ins.initialise_state(f, prem, dern)
            f.seek(octets - 64 * MIB)
            self.assertEqual(f.read(17), b"\0" * 17)

    def test_secteurs_zone_identique_au_noyau(self):
        src = (RACINE / "src/fs/persistance.rs").read_text()
        import re
        m = re.search(r"const SECTEURS_ZONE: u64 = (\d+);", src)
        self.assertIsNotNone(m)
        self.assertEqual(int(m.group(1)), ins.SECTEURS_ZONE)

    def test_liste_ne_choisit_rien(self):
        rc, out, _ = self.lance("liste")
        self.assertEqual(rc, 0)
        self.assertIn(f"identite='SSD Dedie|S3RIE42|{32 * MIB}'", out)
        self.assertIn("Aucun disque n'est choisi automatiquement", out)
        self.assertTrue(self.intact())


if __name__ == "__main__":
    unittest.main()
