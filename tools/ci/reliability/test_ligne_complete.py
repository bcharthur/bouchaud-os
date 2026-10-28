"""P10_TELEMETRIE_COMPLETE_V1 : le banc scanout ne juge qu'une telemetrie
entierement ecrite.

Temoin negatif : le fragment exact du run 40715886448 (77665751, direct KVM),
QEMU tue au milieu de la ligne. Le motif est lu dans le banc lui-meme.
"""
import importlib.util
import re
import unittest
from pathlib import Path

RACINE = Path(__file__).resolve().parents[3]
BANC = RACINE / "tools/ci/run_virtio_gpu_scanout.sh"

spec = importlib.util.spec_from_file_location("ligne_complete", RACINE / "tools/ci/ligne_complete.py")
lc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lc)

# Le releve tronque du run 40715886448, tel qu'imprime par le job.
TRONQUE = (b"\x1b[32m[GUI-VIRTIO-GPU] presents_ok=28 presents_echec=0 pixels_transferes=1958400 "
           b"ns_total=7006759 us_par_present=250 ac")
# Une ligne entiere du meme run (scanout direct, TCG).
ENTIERE = (b"[GUI-VIRTIO-GPU] presents_ok=26 presents_echec=0 pixels_transferes=1881600 ns_total=7398100 "
           b"us_par_present=284 actif=1 sautes=0 cedes=0 non_cedables=13 irq_armee=1 irq_voulue=1 irq=70 "
           b"irq_sans_progres=0 dormantes=57 sommeils=0 reveils_sans_progres=0 vus_par_relecture=0 echues=0 "
           b"tampon=direct octets_copies=0 octets_directs=7526400 acces_en_transfert=0 trame=off trames=0 "
           b"regions_differees=0")


def motif_du_banc() -> str:
    texte = BANC.read_text(encoding="utf-8")
    m = re.search(r"^TELEMETRIE='([^']+)'$", texte, re.M)
    if not m:
        raise AssertionError("run_virtio_gpu_scanout.sh : TELEMETRIE absent")
    return m.group(1)


class LigneComplete(unittest.TestCase):
    def test_le_fragment_du_run_40715886448_est_refuse(self):
        # Temoin negatif : fichier coupe au milieu de la ligne.
        self.assertIsNone(lc.derniere(b"boot\r\n" + TRONQUE, motif_du_banc()))

    def test_meme_termine_le_fragment_ne_correspond_pas_au_motif(self):
        # Une ligne courte mais terminee (ecriture interrompue puis saut de
        # ligne) : le motif exige le dernier champ.
        self.assertIsNone(lc.derniere(TRONQUE + b"\r\n", motif_du_banc()))

    def test_une_ligne_entiere_est_retenue(self):
        ligne = lc.derniere(b"boot\r\n" + ENTIERE + b"\r\n", motif_du_banc())
        self.assertIsNotNone(ligne)
        self.assertTrue(ligne.endswith("regions_differees=0"))

    def test_la_derniere_entiere_passe_avant_un_fragment_final(self):
        # Deux releves, QEMU tue pendant le second : le premier est juge.
        ligne = lc.derniere(ENTIERE + b"\r\n" + TRONQUE, motif_du_banc())
        self.assertIsNotNone(ligne)
        self.assertIn("actif=1 sautes=0", ligne)

    def test_couleurs_et_retour_chariot_ne_genent_pas(self):
        ligne = lc.derniere(b"\x1b[36m" + ENTIERE + b"\x1b[0m\r\n", motif_du_banc())
        self.assertIsNotNone(ligne)

    def test_le_motif_nomme_le_dernier_champ_emis(self):
        # Le motif doit finir sur le dernier champ de la ligne du compositeur.
        source = (RACINE / "src/gui/window_manager.rs").read_text(encoding="utf-8")
        debut = source.index('"[GUI-VIRTIO-GPU] ')
        format_ = source[debut:source.index('",', debut)]
        dernier = format_.split()[-1].split("=")[0]
        self.assertTrue(motif_du_banc().endswith(dernier + "=[0-9]+$"), dernier)


if __name__ == "__main__":
    unittest.main()
