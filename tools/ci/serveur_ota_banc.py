#!/usr/bin/env python3
"""BOUCHAUD_OTA_V1 -- le serveur HTTPS des bancs OTA (run_ab_rollback.sh).

    serveur_ota_banc.py PORT RACINE CERT CLE [--reseau]

TLS 1.3 seulement, le certificat donne (signe par la CA de banc, ou par une
CA que le noyau ne connait pas). Chaque echec de poignee de main est journalise
(`HTTPS TLS_REFUS ...`), chaque requete aussi (`HTTPS TLSv1.3 "GET ..." 200`).

Comportements VOLONTAIRES, une fois chacun, journalises :
  - toujours : la premiere demande de la partie 5 est coupee sans reponse
    (COUPURE_VOLONTAIRE) -- l'agent doit reprendre CETTE partie ;
  - --reseau (campagne D) :
      /erreur/...      -> HTTP 500 (ERREUR_VOLONTAIRE) ;
      partie 1, 1re    -> 200, BONNE taille, un octet change (ALTERATION_VOLONTAIRE) :
                          le contenu, pas le transport -> refus, relance du cycle ;
      partie 2, 1re    -> 200, Content-Length complet, moitie du corps puis
                          fermeture (TRONCATURE_VOLONTAIRE) -> reprise de la partie.
"""
from __future__ import annotations

import functools
import http.server
import os
import ssl
import sys
import threading

port, racine, cert, cle = int(sys.argv[1]), sys.argv[2], sys.argv[3], sys.argv[4]
RESEAU = "--reseau" in sys.argv[5:]
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.minimum_version = ssl.TLSVersion.TLSv1_3
ctx.load_cert_chain(cert, cle)
FAIT: set[str] = set()
VERROU = threading.Lock()


def journal(texte: str) -> None:
    sys.stderr.write(f"HTTPS {texte}\n")
    sys.stderr.flush()


def premiere_fois(cle_: str) -> bool:
    with VERROU:
        if cle_ in FAIT:
            return False
        FAIT.add(cle_)
        return True


class H(http.server.SimpleHTTPRequestHandler):
    def do_GET(self):
        if self.path.endswith("/system-slot.img.part005") and premiere_fois("coupe5"):
            journal(f"COUPURE_VOLONTAIRE {self.path}")
            self.close_connection = True
            return
        if RESEAU and self.path.startswith("/erreur/"):
            journal(f"ERREUR_VOLONTAIRE {self.path} 500")
            self.send_error(500, "erreur volontaire du banc")
            return
        if RESEAU and self.path.endswith("/system-slot.img.part001") and premiere_fois("altere1"):
            corps = bytearray(self._lit())
            corps[len(corps) // 2] ^= 0x01
            journal(f"ALTERATION_VOLONTAIRE {self.path} octets={len(corps)}")
            self._envoie(bytes(corps), len(corps))
            return
        if RESEAU and self.path.endswith("/system-slot.img.part002") and premiere_fois("tronque2"):
            corps = self._lit()
            journal(f"TRONCATURE_VOLONTAIRE {self.path} envoyes={len(corps) // 2}/{len(corps)}")
            self._envoie(corps[: len(corps) // 2], len(corps))
            self.close_connection = True
            return
        return super().do_GET()

    def _lit(self) -> bytes:
        chemin = self.translate_path(self.path)
        with open(chemin, "rb") as f:
            return f.read()

    def _envoie(self, corps: bytes, annonce: int) -> None:
        self.send_response(200)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(annonce))
        self.end_headers()
        self.wfile.write(corps)
        self.wfile.flush()

    def log_message(self, fmt, *a):
        journal(f"{self.request.version()} {fmt % a}")


class Serveur(http.server.ThreadingHTTPServer):
    # La poignee de main se fait dans le FIL de la connexion, jamais dans la
    # boucle d'acceptation : une connexion figee (lien coupe pendant la
    # poignee) bloquait sinon TOUTES les suivantes (premier essai ota-reseau).
    # Chaque connexion a un delai : un client parti ne retient rien.
    daemon_threads = True

    def get_request(self):
        s, a = self.socket.accept()
        s.settimeout(60)
        return ctx.wrap_socket(s, server_side=True, do_handshake_on_connect=False), a

    def finish_request(self, request, client_address):
        try:
            request.do_handshake()
        except (ssl.SSLError, OSError) as e:
            journal(f"TLS_REFUS client={client_address[0]} raison={getattr(e, 'reason', None) or e}")
            return
        super().finish_request(request, client_address)

    def handle_error(self, request, client_address):
        journal(f"CONNEXION_ROMPUE client={client_address[0]} raison={sys.exc_info()[1]!r}")


os.chdir(racine)
srv = Serveur(("127.0.0.1", port), functools.partial(H, directory=racine))
journal(f"PRET port={port} reseau={int(RESEAU)}")
srv.serve_forever()
