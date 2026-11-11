"""BOUCHAUD_APT_REPLI_MIROIR_V1 : le repli de miroir APT ne retire QUE les miroirs injoignables
(nom non resolu, ou InRelease sans reponse).

Runs 43037630866 et suivants (11/11) : `azure.archive.ubuntu.com` ne se
resolvait plus sur certains runners, et trois essais rejouaient ce miroir.
Le repli doit retirer ce miroir-la et garder les autres ; ne rien retirer si
tous se resolvent ; ne jamais vider la liste.
"""
import functools
import http.server
import socket
import subprocess
import tempfile
import threading
import unittest
from pathlib import Path

RACINE = Path(__file__).resolve().parents[3]
SCRIPT = RACINE / "tools/ci/apt-repare-miroirs.sh"
MORT = "http://miroir-mort.invalid/ubuntu/\tpriority:1"


def port_libre():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Miroir:
    """Un vrai miroir HTTP local qui sert ubuntu/dists/noble/InRelease."""

    def __enter__(self):
        self.dossier = tempfile.TemporaryDirectory()
        racine = Path(self.dossier.name)
        (racine / "ubuntu/dists/noble").mkdir(parents=True)
        (racine / "ubuntu/dists/noble/InRelease").write_text("Origin: Ubuntu\n")
        gestion = functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(racine))
        gestion.log_message = lambda *a: None
        self.serveur = http.server.ThreadingHTTPServer(("127.0.0.1", 0), gestion)
        threading.Thread(target=self.serveur.serve_forever, daemon=True).start()
        self.ligne = f"http://localhost:{self.serveur.server_address[1]}/ubuntu/\tpriority:2"
        return self

    def __exit__(self, *a):
        self.serveur.shutdown()
        self.dossier.cleanup()


def lance(lignes):
    with tempfile.TemporaryDirectory() as d:
        liste = Path(d, "apt-mirrors.txt")
        liste.write_text("\n".join(lignes) + "\n")
        env = {"APT_LISTE_MIROIRS": str(liste), "APT_VERSION_UBUNTU": "noble", "PATH": "/usr/bin:/bin",
               "NO_PROXY": "localhost,127.0.0.1", "no_proxy": "localhost,127.0.0.1"}
        r = subprocess.run(["bash", str(SCRIPT)], env=env, capture_output=True, text=True, timeout=120)
        return r.returncode, r.stderr, liste.read_text().splitlines()


class ReplieMiroirTests(unittest.TestCase):
    def test_le_miroir_au_nom_non_resolu_est_retire_le_vivant_garde(self):
        with Miroir() as m:
            code, journal, apres = lance([MORT, m.ligne])
        self.assertEqual(code, 0, journal)
        self.assertEqual(apres, [m.ligne])
        self.assertIn("APT_REPLI_MIROIR retires=1 gardes=1", journal)
        self.assertIn("dns hote=miroir-mort.invalid resolu=0", journal)

    def test_le_miroir_resolu_mais_muet_est_retire(self):
        # Le cas des runners sans erreur DNS : nom resolu, InRelease sans reponse.
        muet = f"http://localhost:{port_libre()}/ubuntu/\tpriority:1"
        with Miroir() as m:
            code, journal, apres = lance([muet, m.ligne])
        self.assertEqual(apres, [m.ligne], journal)
        self.assertIn("resolu=1 repond=0", journal)

    def test_liste_jamais_videe(self):
        code, journal, apres = lance([MORT])
        self.assertEqual(code, 0, journal)
        self.assertEqual(apres, [MORT])
        self.assertIn("APT_REPLI_MIROIR impossible", journal)

    def test_rien_n_est_retire_si_tout_repond(self):
        with Miroir() as m:
            code, journal, apres = lance([m.ligne])
        self.assertEqual(apres, [m.ligne], journal)
        self.assertIn("APT_REPLI_MIROIR aucun", journal)


if __name__ == "__main__":
    unittest.main()
