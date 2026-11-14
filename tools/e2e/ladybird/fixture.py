#!/usr/bin/env python3
"""BOUCHAUD_E2E_LADYBIRD_V1 -- site de REFERENCE des parcours E2E.

    fixture.py PORT JOURNAL [PORT_TLS]

Un site deterministe, servi par l'hote (l'invite le joint en
http://10.0.2.2:PORT par le reseau utilisateur de QEMU, sous Linux comme sous
Windows). Il NE pretend PAS etre Google : il reproduit les interactions d'un
moteur de recherche (formulaire, page de resultats, liens, retour) pour que
les controles bloquants ne dependent d'aucun site externe.

Chaque page porte un bloc de couleur UNIQUE (`COULEURS`) : le banc le cherche
dans la capture d'ecran. Un document charge n'est pas la preuve que ses
pixels sont visibles ; ce bloc l'est.

Chaque requete servie est journalisee (une ligne JSON par requete) dans
JOURNAL : chemin, parametres, heure hote (horloge MONOTONE de l'hote, jamais
melangee a celle de l'invite).
"""
import html
import json
import ssl
import sys
import threading
import time
from pathlib import Path
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

# Couleur pleine de chaque page (R, G, B). Choisies hors de la palette
# d'Atelier pour qu'un bloc trouve dans la capture vienne de la page.
COULEURS = {
    "accueil": (0xE0, 0x21, 0x8A),
    "recherche": (0x21, 0x8A, 0xE0),
    "resultats": (0x2E, 0xB8, 0x4C),
    "article-1": (0xF2, 0x9B, 0x11),
    "article-2": (0x7A, 0x2B, 0xD9),
    "onglet-b": (0x11, 0xC4, 0xC4),
    "onglet-c": (0xC4, 0x11, 0x11),
    "expire": (0x8A, 0xE0, 0x21),
}
# PORT_TLS : le MEME site, en HTTPS, avec un certificat EXPIRE signe par
# l'autorite de test (tls/). Le disque E2E fait confiance a cette autorite :
# un refus ne peut donc venir que de l'expiration. Chaque requete servie y
# est journalisee `tls=1` -- une page jamais servie prouve le refus.
TLS = Path(__file__).resolve().parent / "tls"
RESULTATS = [
    ("article-1", "Bouchaud OS : un systeme d'exploitation"),
    ("article-2", "Le navigateur Ladybird sous Bouchaud OS"),
]


def hexa(rgb):
    return "#%02x%02x%02x" % rgb


def page(nom, titre, corps, focus=None, lien_bloc=None):
    script = ""
    if focus:
        # Le foyer va a l'element voulu des le chargement : le banc tape au
        # clavier sans viser a la souris une page dont il ignore la mise en page.
        script = "<script>window.addEventListener('load',function(){var e=document.querySelector('%s');if(e){e.focus();}});</script>" % focus
    bloc = "<div id='bloc' style='width:100%%;height:260px;background:%s'></div>" % hexa(COULEURS[nom])
    if lien_bloc:
        # Le bloc entier est un lien : le banc clique au milieu de la page,
        # sans connaitre sa mise en page au pixel pres.
        bloc = "<a id='lien-bloc' href='%s' style='display:block'>%s</a>" % (lien_bloc, bloc)
    return ("<!doctype html><html><head><meta charset='utf-8'><title>%s</title>%s</head>"
            "<body style='margin:0;font-family:sans-serif'>%s"
            "<div style='padding:16px'>%s</div></body></html>") % (titre, script, bloc, corps)


class Gestionnaire(BaseHTTPRequestHandler):
    journal = None

    def log_message(self, *args):
        pass

    tls = False

    def note(self, chemin, q):
        ligne = {"t_hote_ms": round(time.monotonic() * 1000), "chemin": chemin, "q": q,
                 "agent": self.headers.get("User-Agent", ""), "tls": int(self.tls)}
        with open(self.journal, "a", encoding="utf-8") as f:
            f.write(json.dumps(ligne, ensure_ascii=False) + "\n")

    def envoie(self, code, html):
        corps = html.encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(corps)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(corps)

    def do_GET(self):
        u = urlparse(self.path)
        q = parse_qs(u.query).get("q", [""])[0]
        self.note(u.path, q)
        if u.path in ("/", "/accueil.html"):
            self.envoie(200, page("accueil", "Accueil E2E", "<h1>Accueil E2E Bouchaud</h1>"))
        elif u.path == "/recherche.html":
            self.envoie(200, page("recherche", "Recherche E2E",
                                  "<form action='/resultats' method='get'>"
                                  "<input id='q' name='q' size='40' autocomplete='off'>"
                                  "<button type='submit'>Rechercher</button></form>", focus="#q"))
        elif u.path == "/resultats":
            liens = "".join("<p><a id='r%d' href='/%s.html'>%s</a></p>" % (i + 1, nom, titre)
                            for i, (nom, titre) in enumerate(RESULTATS))
            qh = html.escape(q)
            self.envoie(200, page("resultats", "Resultats E2E : %s" % qh,
                                  "<h1>Resultats pour %s</h1>%s" % (qh, liens), focus="#r1",
                                  lien_bloc="/%s.html" % RESULTATS[0][0]))
        elif u.path.endswith(".html") and u.path[1:-5] in COULEURS:
            nom = u.path[1:-5]
            self.envoie(200, page(nom, "Page %s" % nom, "<h1>Page %s</h1>" % nom))
        else:
            self.envoie(404, "<!doctype html><title>404</title><h1>404</h1>")


class GestionnaireTls(Gestionnaire):
    tls = True


def serveur_tls(port):
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.load_cert_chain(str(TLS / "expire.pem"), str(TLS / "expire.key"))
    serveur = ThreadingHTTPServer(("127.0.0.1", port), GestionnaireTls)
    serveur.socket = ctx.wrap_socket(serveur.socket, server_side=True)
    return serveur


def main(argv):
    if len(argv) not in (3, 4):
        print(__doc__, file=sys.stderr)
        return 2
    Gestionnaire.journal = argv[2]
    open(argv[2], "w").close()
    serveur = ThreadingHTTPServer(("127.0.0.1", int(argv[1])), Gestionnaire)
    if len(argv) == 4:
        threading.Thread(target=serveur_tls(int(argv[3])).serve_forever, daemon=True).start()
    print("E2E_FIXTURE_PRETE port=%s tls=%s" % (argv[1], argv[3] if len(argv) == 4 else "-"), flush=True)
    serveur.serve_forever()
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
