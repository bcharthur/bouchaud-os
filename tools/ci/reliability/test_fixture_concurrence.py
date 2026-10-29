#!/usr/bin/env python3
"""P13 : une connexion HTTP incomplete ne doit pas bloquer les autres requetes."""
import importlib.util
import socket
import threading
import unittest
from concurrent.futures import ThreadPoolExecutor
from http.server import HTTPServer
from pathlib import Path
from urllib.request import urlopen


FICHIER = Path(__file__).resolve().parents[2] / "health" / "browser_host_fixture.py"


def _importe_fixture():
    spec = importlib.util.spec_from_file_location("bouchaud_fixture_concurrente", FICHIER)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class FixtureConcurrence(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.fixture = _importe_fixture()

    def _exerce(self, fabrique, doit_repondre):
        accepte = threading.Event()
        source = self.fixture.Handler

        class HandlerAvecAccuse(source):
            def setup(self):
                super().setup()
                accepte.set()

        serveur = fabrique(("127.0.0.1", 0), HandlerAvecAccuse)
        fil = threading.Thread(target=serveur.serve_forever, kwargs={"poll_interval": 0.01}, daemon=True)
        fil.start()
        adresse = serveur.server_address
        bloqueur = socket.create_connection(adresse, timeout=1)
        try:
            # Historique mono-thread: une connexion avec en-tetes incomplets
            # immobilise le service de toutes les autres requetes.
            bloqueur.sendall(b"GET /pixel.png HTTP/1.1\r\nHost: 127.0.0.1\r\n")
            self.assertTrue(accepte.wait(2), "connexion bloquante non acceptee")
            reponse = False
            try:
                with urlopen(f"http://127.0.0.1:{adresse[1]}/pixel.png", timeout=1.0) as page:
                    reponse = page.status == 200 and page.read().startswith(b"\x89PNG\r\n\x1a\n")
            except TimeoutError:
                pass
            self.assertEqual(reponse, doit_repondre)
        finally:
            bloqueur.close()
            serveur.shutdown()
            serveur.server_close()
            fil.join(timeout=2)

    def test_blocage_historique_et_correction(self):
        self._exerce(lambda addr, handler: HTTPServer(addr, handler), False)
        self._exerce(self.fixture.creer_serveur, True)

    def test_compteurs_exacts_sous_concurrence(self):
        serveur = self.fixture.creer_serveur(("127.0.0.1", 0))
        fil = threading.Thread(target=serveur.serve_forever, kwargs={"poll_interval": 0.01}, daemon=True)
        fil.start()
        port = serveur.server_address[1]
        avant = self.fixture._lit_compteur("worker.js")
        try:
            def charge(_):
                with urlopen(f"http://127.0.0.1:{port}/worker.js", timeout=2) as page:
                    self.assertEqual(page.status, 200)
                    self.assertTrue(page.read())
            with ThreadPoolExecutor(max_workers=8) as pool:
                list(pool.map(charge, range(24)))
            with urlopen(f"http://127.0.0.1:{port}/compteur?fichier=worker.js", timeout=2) as page:
                self.assertEqual(int(page.read()), avant + 24)
        finally:
            serveur.shutdown()
            serveur.server_close()
            fil.join(timeout=2)


if __name__ == "__main__":
    unittest.main()
