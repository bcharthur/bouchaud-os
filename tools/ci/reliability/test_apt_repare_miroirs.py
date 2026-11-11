"""BOUCHAUD_APT_REPLI_MIROIR_V1 : le repli de miroir APT ne retire QUE les miroirs injoignables.

Runs 43037630866 et suivants (11/11) : `azure.archive.ubuntu.com` ne se
resolvait plus sur certains runners, et trois essais rejouaient ce miroir.
Le repli doit retirer ce miroir-la et garder les autres ; ne rien retirer si
tous se resolvent ; ne jamais vider la liste.
"""
import subprocess
import tempfile
import unittest
from pathlib import Path

RACINE = Path(__file__).resolve().parents[3]
SCRIPT = RACINE / "tools/ci/apt-repare-miroirs.sh"
MORT = "http://miroir-mort.invalid/ubuntu/\tpriority:1"
VIVANT = "http://localhost/ubuntu/\tpriority:2"


def lance(lignes):
    with tempfile.TemporaryDirectory() as d:
        liste = Path(d, "apt-mirrors.txt")
        liste.write_text("\n".join(lignes) + "\n")
        r = subprocess.run(["bash", str(SCRIPT)], env={"APT_LISTE_MIROIRS": str(liste), "PATH": "/usr/bin:/bin"},
                           capture_output=True, text=True, timeout=60)
        return r.returncode, r.stderr, liste.read_text().splitlines()


class ReplieMiroirTests(unittest.TestCase):
    def test_le_miroir_injoignable_est_retire_les_autres_gardes(self):
        code, journal, apres = lance([MORT, VIVANT])
        self.assertEqual(code, 0, journal)
        self.assertEqual(apres, [VIVANT])
        self.assertIn("APT_REPLI_MIROIR retires=1 gardes=1", journal)
        self.assertIn("dns hote=miroir-mort.invalid resolu=0", journal)

    def test_liste_jamais_videe(self):
        code, journal, apres = lance([MORT])
        self.assertEqual(code, 0, journal)
        self.assertEqual(apres, [MORT])
        self.assertIn("APT_REPLI_MIROIR impossible", journal)

    def test_rien_n_est_retire_si_tout_se_resout(self):
        code, journal, apres = lance([VIVANT])
        self.assertEqual(apres, [VIVANT])
        self.assertIn("APT_REPLI_MIROIR aucun", journal)


if __name__ == "__main__":
    unittest.main()
