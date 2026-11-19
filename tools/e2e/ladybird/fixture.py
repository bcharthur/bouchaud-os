#!/usr/bin/env python3
"""BOUCHAUD_E2E_LADYBIRD_V1 -- site de REFERENCE des parcours E2E.

    fixture.py PORT JOURNAL [PORT_TLS DOSSIER_TLS]

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
    "lourde": (0xD9, 0x6B, 0x2B),
    "video": (0x2B, 0x6B, 0xD9),
}
# PORT_TLS : le MEME site, en HTTPS, avec le certificat EXPIRE que
# tls/genere_tls.py fabrique au moment du test dans DOSSIER_TLS, signe par
# une autorite de test que le disque E2E croit : un refus ne peut donc venir
# que de l'expiration. Chaque requete servie y est journalisee `tls=1` --
# une page jamais servie prouve le refus.
RESULTATS = [
    ("article-1", "Bouchaud OS : un systeme d'exploitation"),
    ("article-2", "Le navigateur Ladybird sous Bouchaud OS"),
]


# BOUCHAUD_E2E_PAGE_LOURDE_V1 -- une page du GABARIT d'une page de resultats
# video : grille de cartes construite par JavaScript (comme un site moderne),
# une vignette PNG par carte (decodage d'images), du texte, des minuteries.
# Deterministe : aucun aleas, les memes octets a chaque requete. Ce n'est pas
# YouTube ; c'est une charge reproductible qui exerce les memes chemins (DOM
# construit par script, mise en page de grille, images, defilement).
def vignette_png(i, l=160, h=90):
    import struct as _s, zlib as _z
    r, g, b = (37 * i) % 200 + 40, (91 * i) % 200 + 40, (53 * i) % 200 + 40
    lignes = bytearray()
    for y in range(h):
        lignes.append(0)
        for x in range(l):
            bande = ((x + y + i) // 12) % 2
            lignes += bytes((r if bande else 255 - r, g, b if bande else 255 - b))
    def bloc(t, c):
        return _s.pack(">I", len(c)) + t + c + _s.pack(">I", _z.crc32(t + c) & 0xFFFFFFFF)
    return (b"\x89PNG\r\n\x1a\n" + bloc(b"IHDR", _s.pack(">IIBBBBB", l, h, 8, 2, 0, 0, 0))
            + bloc(b"IDAT", _z.compress(bytes(lignes), 6)) + bloc(b"IEND", b""))


def page_lourde(nom, titre, n, lien_bloc):
    """Grille de N cartes construite par script, sous un bloc de couleur."""
    script = """<script>
(function(){
  var n=%d, g=document.getElementById('grille'), f=document.createDocumentFragment();
  for(var i=1;i<=n;i++){
    var a=document.createElement('a'); a.href='/video.html?i='+i; a.className='carte';
    a.innerHTML="<img src='/vignette/"+(i%%48)+".png' width='160' height='90'>"
      +"<div class='t'>Video de reference numero "+i+" : un titre assez long pour passer a la ligne</div>"
      +"<div class='m'>Chaine "+(i%%17)+" &middot; "+(i*997%%100000)+" vues &middot; il y a "+(i%%30+1)+" jours</div>";
    f.appendChild(a);
  }
  g.appendChild(f);
  var c=0; setInterval(function(){document.getElementById('horloge').textContent='t='+(++c);},1000);
})();
</script>""" % n
    style = ("<style>#grille{display:grid;grid-template-columns:repeat(auto-fill,minmax(180px,1fr));gap:12px;padding:12px}"
             ".carte{display:block;color:#111;text-decoration:none;font-size:13px}"
             ".carte img{display:block;border-radius:8px}.t{font-weight:bold;margin-top:6px}.m{color:#555}</style>")
    bloc = ("<a id='lien-bloc' href='%s' style='display:block'><div id='bloc' style='width:100%%;height:260px;"
            "background:%s'></div></a>") % (lien_bloc, hexa(COULEURS[nom]))
    return ("<!doctype html><html><head><meta charset='utf-8'><title>%s</title>%s</head>"
            "<body style='margin:0;font-family:sans-serif'>%s<div id='horloge'>t=0</div>"
            "<div id='grille'></div>%s</body></html>") % (titre, style, bloc, script)


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


VIGNETTES = {}


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
        elif u.path == "/lourde.html":
            n = max(1, min(400, int(parse_qs(u.query).get("n", ["120"])[0] or 120)))
            self.envoie(200, page_lourde("lourde", "Page lourde E2E", n, "/video.html?i=1"))
        elif u.path == "/video.html":
            i = parse_qs(u.query).get("i", ["1"])[0]
            self.envoie(200, page_lourde("video", "Video E2E %s" % html.escape(i), 40, "/lourde.html"))
        elif u.path.startswith("/vignette/") and u.path.endswith(".png"):
            try:
                i = int(u.path[len("/vignette/"):-4])
            except ValueError:
                i = -1
            if not 0 <= i < 48:
                self.envoie(404, "<!doctype html><title>404</title><h1>404</h1>")
                return
            corps = VIGNETTES.setdefault(i, vignette_png(i))
            self.send_response(200)
            self.send_header("Content-Type", "image/png")
            self.send_header("Content-Length", str(len(corps)))
            self.send_header("Cache-Control", "max-age=3600")
            self.end_headers()
            self.wfile.write(corps)
        elif u.path.endswith(".html") and u.path[1:-5] in COULEURS:
            nom = u.path[1:-5]
            self.envoie(200, page(nom, "Page %s" % nom, "<h1>Page %s</h1>" % nom))
        else:
            self.envoie(404, "<!doctype html><title>404</title><h1>404</h1>")


class GestionnaireTls(Gestionnaire):
    tls = True


def serveur_tls(port, dossier):
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.load_cert_chain(str(Path(dossier) / "expire.pem"), str(Path(dossier) / "expire.key"))
    serveur = ThreadingHTTPServer(("127.0.0.1", port), GestionnaireTls)
    serveur.socket = ctx.wrap_socket(serveur.socket, server_side=True)
    return serveur


def main(argv):
    if len(argv) not in (3, 5):
        print(__doc__, file=sys.stderr)
        return 2
    Gestionnaire.journal = argv[2]
    open(argv[2], "w").close()
    serveur = ThreadingHTTPServer(("127.0.0.1", int(argv[1])), Gestionnaire)
    if len(argv) == 5:
        threading.Thread(target=serveur_tls(int(argv[3]), argv[4]).serve_forever, daemon=True).start()
    print("E2E_FIXTURE_PRETE port=%s tls=%s" % (argv[1], argv[3] if len(argv) == 5 else "-"), flush=True)
    serveur.serve_forever()
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
