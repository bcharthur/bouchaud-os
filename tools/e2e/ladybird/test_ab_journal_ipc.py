import io
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from ab_journal_ipc import analyse, prepare_image, profil


class BancJournal(unittest.TestCase):
    def test_profil_conserve_les_commandes_et_ne_la_regle_qu_une_fois(self):
        lignes = ["console-async on", "profil-rip on", "desktop"]
        self.assertEqual(profil(lignes, "off"), ["console-async on", "profil-rip off", "desktop"])
        with self.assertRaises(ValueError):
            profil(["profil-rip on"], "off")

    def test_disques_ne_different_que_sur_autorun(self):
        with tempfile.TemporaryDirectory() as dossier:
            racine = Path(dossier)
            source = racine / "source.img"
            with tarfile.open(source, "w") as t:
                for chemin, contenu in (("autorun", b"profil-rip off\ndesktop\n"), ("bin/Compositor", b"pinned")):
                    entree = tarfile.TarInfo(chemin)
                    entree.size = len(contenu)
                    t.addfile(entree, io.BytesIO(contenu))
            for mode in ("on", "off"):
                dest = racine / (mode + ".img")
                with patch("ab_journal_ipc.ajoute_zone_persistante") as persistance:
                    prepare_image(source, dest, mode)
                    persistance.assert_called_once_with(dest)
                with tarfile.open(dest) as t:
                    self.assertEqual(t.extractfile("bin/Compositor").read(), b"pinned")
                    self.assertEqual(t.extractfile("autorun").read(), f"profil-rip {mode}\ndesktop\n".encode())

    def test_intervalles_excluent_trous_et_changements_d_onglet(self):
        trames = "\n".join(f"[LB:FRAME] onglet={o} seq={s} t={t} copie_px=100 complet=1 present_us=3200"
                            for o, s, t in ((1, 1, 0), (1, 2, 16), (1, 4, 200), (2, 5, 800), (2, 6, 820)))
        resultat = analyse(trames)
        self.assertEqual(resultat["intervalle_invite_ms"], {"n": 2, "p50": 16, "p95": 20, "p99": 20})
        self.assertEqual(resultat["copies_pixels"], 500)
        self.assertIsNone(resultat["contre_pression"])


if __name__ == "__main__":
    unittest.main()
