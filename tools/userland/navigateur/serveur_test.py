#!/usr/bin/env python3
"""Un serveur HTTP local pour eprouver le navigateur sans Internet.

    ./serveur_test.py                 # sert sur 127.0.0.1:8731
    ./serveur_test.py --port 9000 --origine-b 9001

## Pourquoi il fallait le construire

L'audit precedent s'est heurte a un mur : le proxy de l'environnement refuse
tout hote hors liste, et le corpus reel s'est reduit a un seul domaine. Cela
n'a pas seulement limite la mesure — cela a rendu **intestable** tout ce qui
compte maintenant : une redirection, un 500, un POST de formulaire, un module
importe par un autre module, une reponse lente pendant qu'on verifie que
l'interface repond encore.

Aucune de ces choses n'a besoin d'Internet. Elles ont besoin d'un serveur qui
reponde exactement ce qu'on lui demande, a la milliseconde pres, deux fois de
suite pareil. C'est ce que fait ce fichier.

## Ce qu'il sert

| Route | Ce qu'elle eprouve |
|---|---|
| `/` | une page d'accueil qui liste tout le reste |
| `/json` | `Content-Type: application/json`, corps structure |
| `/redirect?to=…&n=…` | chaines de redirection, boucles bornees |
| `/status/404`, `/status/500` | ce que le moteur fait d'un echec |
| `/form/get`, `/form/post` | soumission reelle, et ce que le serveur a recu |
| `/delay/2` | reponse lente : l'interface doit rester vivante |
| `/module/main.js` et ses voisins | resolution relative au **module**, pas au document |
| `/echo` | renvoie la requete telle qu'elle est arrivee |
| `/cookie/pose`, `/cookie/lit` | aller-retour de temoins, `HttpOnly` compris |
| `/slow-script.js` | un script lent au milieu du document |
| `/404.js`, `/404.css` | un bundle et une feuille perdus |

## Deux origines

Le serveur peut ouvrir un second port. Deux ports sur `127.0.0.1`, ce sont deux
origines au sens de la norme : c'est le minimum pour que « cross-origin » veuille
dire quelque chose, et donc pour qu'un test de SOP ou de CORS ait un sens le jour
ou l'on s'y mettra. Aujourd'hui la seconde origine sert surtout a verifier que
les temoins et les en-tetes ne franchissent pas la frontiere.

## Deterministe

Aucune date, aucun aleatoire, aucun compteur global dans les corps servis. Deux
executions rendent octet pour octet la meme chose — sans quoi les references
visuelles et les rapports de compatibilite ne seraient pas comparables d'un
commit a l'autre.
"""

import argparse
import base64
import hashlib
import http.server
import json
import os
import socket
import socketserver
import struct
import sys
import threading
import time
import urllib.parse

RACINE = os.path.dirname(os.path.abspath(__file__))
PAGES = os.path.join(RACINE, "tests", "pages")

PORT_DEFAUT = 8731


class Fixtures(http.server.BaseHTTPRequestHandler):
    """Les routes. Une methode par famille, pas de cadre applicatif."""

    protocol_version = "HTTP/1.1"
    server_version = "BouchaudFixtures/1.0"

    # -- Utilitaires -----------------------------------------------------------

    def log_message(self, format, *args):  # noqa: A002 — signature imposee
        if self.server.bavard:
            sys.stderr.write("  %s %s\n" % (self.command, self.path))

    def repond(self, code, corps, type_mime="text/html; charset=utf-8",
               entetes=None):
        if isinstance(corps, str):
            corps = corps.encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", type_mime)
        self.send_header("Content-Length", str(len(corps)))
        # Sans cela, une mesure repetee lirait le cache du moteur et non le
        # serveur : deux executions ne mesureraient plus la meme chose.
        self.send_header("Cache-Control", "no-store")
        for nom, valeur in (entetes or []):
            self.send_header(nom, valeur)
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(corps)

    def json(self, code, charge, entetes=None):
        self.repond(code, json.dumps(charge, sort_keys=True, indent=2),
                    "application/json; charset=utf-8", entetes)

    def corps_recu(self):
        longueur = int(self.headers.get("Content-Length") or 0)
        return self.rfile.read(longueur).decode("utf-8", "replace") if longueur else ""

    def chemin_et_requete(self):
        morceaux = urllib.parse.urlsplit(self.path)
        return morceaux.path, urllib.parse.parse_qs(morceaux.query)

    # -- Repartition -----------------------------------------------------------

    def do_GET(self):  # noqa: N802 — nom impose par la bibliotheque
        self.sert("GET")

    def do_HEAD(self):  # noqa: N802
        self.sert("GET")

    def do_POST(self):  # noqa: N802
        self.sert("POST")

    def sert(self, methode):
        chemin, requete = self.chemin_et_requete()
        try:
            self.route(methode, chemin, requete)
        except BrokenPipeError:
            pass  # le navigateur a ferme : ce n'est pas une erreur du serveur
        except Exception as e:  # noqa: BLE001
            self.repond(500, "<h1>500</h1><pre>%s</pre>" % e)

    def route(self, methode, chemin, requete):
        if chemin == "/":
            return self.repond(200, self.accueil())
        if chemin == "/json":
            return self.json(200, {"ok": True, "nom": "bouchaud",
                                   "liste": [1, 2, 3],
                                   "imbrique": {"a": "b"}})
        if chemin == "/echo":
            return self.json(200, {
                "methode": methode,
                "chemin": chemin,
                "requete": requete,
                "entetes": {k.lower(): v for k, v in self.headers.items()},
                "corps": self.corps_recu() if methode == "POST" else "",
            })
        if chemin == "/redirect":
            return self.redirection(requete)
        if chemin.startswith("/status/"):
            return self.statut(chemin)
        if chemin.startswith("/delay/"):
            return self.lenteur(chemin)
        if chemin.startswith("/form/"):
            return self.formulaire(methode, chemin, requete)
        if chemin.startswith("/module/"):
            return self.module(chemin)
        if chemin.startswith("/cookie/"):
            return self.temoin(chemin)
        if chemin == "/ws" or chemin.startswith("/ws/"):
            return self.websocket(chemin)
        if chemin == "/slow-script.js":
            time.sleep(0.4)
            return self.repond(200, "globalThis.__lent = true;\n",
                               "application/javascript")
        if chemin.endswith(".js") and chemin.startswith("/404"):
            return self.repond(404, "not found", "text/plain")
        if chemin.endswith(".css") and chemin.startswith("/404"):
            return self.repond(404, "not found", "text/plain")
        if chemin.startswith("/page/"):
            return self.page(chemin)
        return self.repond(404, "<h1>404</h1><p>%s</p>" % chemin)

    # -- WebSocket -------------------------------------------------------------
    #
    # Un serveur d'echo minimal mais conforme : poignee de main RFC 6455,
    # trames texte, ping/pong, fermeture propre. Il n'y a pas de raison de
    # dependre d'un service exterieur pour eprouver un protocole dont tout
    # l'interet est le temps reel — et un service exterieur rendrait le test
    # non deterministe.

    MAGIE = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"

    def websocket(self, chemin):
        cle = self.headers.get("Sec-WebSocket-Key")
        if not cle or (self.headers.get("Upgrade") or "").lower() != "websocket":
            return self.repond(400, "attendu : une poignee de main WebSocket")

        accepte = base64.b64encode(
            hashlib.sha1((cle + self.MAGIE).encode("ascii")).digest()).decode("ascii")
        self.send_response(101)
        self.send_header("Upgrade", "websocket")
        self.send_header("Connection", "Upgrade")
        self.send_header("Sec-WebSocket-Accept", accepte)
        self.end_headers()
        self.wfile.flush()

        # `/ws/ferme` ferme des l'ouverture : c'est le cas qu'un client doit
        # savoir distinguer d'une panne.
        if chemin == "/ws/ferme":
            self._envoie_trame(0x8, struct.pack(">H", 1000) + b"au revoir")
            return
        self._boucle_echo(chemin)

    def _boucle_echo(self, chemin):
        salue = chemin == "/ws/salut"
        if salue:
            self._envoie_trame(0x1, b"bonjour")
        while True:
            trame = self._lis_trame()
            if trame is None:
                return
            code, charge = trame
            if code == 0x8:  # fermeture demandee par le client
                self._envoie_trame(0x8, charge[:2] or struct.pack(">H", 1000))
                return
            if code == 0x9:  # ping
                self._envoie_trame(0xA, charge)
                continue
            if code == 0xA:
                continue
            texte = charge.decode("utf-8", "replace")
            if texte == "__ferme__":
                self._envoie_trame(0x8, struct.pack(">H", 1000) + b"demande")
                return
            if texte == "__trois__":
                # Trois messages d'affilee : de quoi verifier que l'ordre est
                # conserve et qu'aucun n'est perdu.
                for i in (1, 2, 3):
                    self._envoie_trame(0x1, ("message %d" % i).encode("utf-8"))
                continue
            self._envoie_trame(code, charge)

    def _lis_trame(self):
        """Rend `(code, charge)`, ou `None` si la connexion se ferme."""
        entete = self._lis_exactement(2)
        if entete is None:
            return None
        premier, second = entete[0], entete[1]
        code = premier & 0x0F
        masque = bool(second & 0x80)
        longueur = second & 0x7F
        if longueur == 126:
            suite = self._lis_exactement(2)
            if suite is None:
                return None
            longueur = struct.unpack(">H", suite)[0]
        elif longueur == 127:
            suite = self._lis_exactement(8)
            if suite is None:
                return None
            longueur = struct.unpack(">Q", suite)[0]
        cles = self._lis_exactement(4) if masque else b"\0\0\0\0"
        if cles is None:
            return None
        charge = self._lis_exactement(longueur) if longueur else b""
        if charge is None:
            return None
        if masque:
            charge = bytes(o ^ cles[i % 4] for i, o in enumerate(charge))
        return code, charge

    def _lis_exactement(self, combien):
        morceaux = b""
        while len(morceaux) < combien:
            try:
                bloc = self.rfile.read(combien - len(morceaux))
            except (OSError, ValueError):
                return None
            if not bloc:
                return None
            morceaux += bloc
        return morceaux

    def _envoie_trame(self, code, charge=b""):
        entete = bytes([0x80 | code])
        taille = len(charge)
        if taille < 126:
            entete += bytes([taille])
        elif taille < 65536:
            entete += bytes([126]) + struct.pack(">H", taille)
        else:
            entete += bytes([127]) + struct.pack(">Q", taille)
        try:
            self.wfile.write(entete + charge)
            self.wfile.flush()
        except (OSError, ValueError):
            pass

    # -- Routes ----------------------------------------------------------------

    def accueil(self):
        liens = ["/json", "/echo", "/ws", "/redirect?to=/json&n=2", "/status/404",
                 "/status/500", "/form/get", "/form/post", "/delay/1",
                 "/module/main.js", "/cookie/pose", "/cookie/lit",
                 "/page/login-form.html", "/page/spa-navigation.html",
                 "/page/event-loop.html", "/page/reseau-lent.html"]
        return ("<!doctype html><html lang=fr><head><meta charset=utf-8>"
                "<title>Fixtures Bouchaud</title></head><body>"
                "<h1>Fixtures Bouchaud</h1><ul>"
                + "".join('<li><a href="%s">%s</a></li>' % (u, u) for u in liens)
                + "</ul></body></html>")

    def redirection(self, requete):
        """`/redirect?to=/json&n=3` : trois bonds avant d'arriver."""
        cible = (requete.get("to") or ["/json"])[0]
        restants = int((requete.get("n") or ["1"])[0])
        code = int((requete.get("code") or ["302"])[0])
        if restants > 1:
            suivant = "/redirect?to=%s&n=%d&code=%d" % (
                urllib.parse.quote(cible, safe=""), restants - 1, code)
            return self.repond(code, "", "text/plain", [("Location", suivant)])
        return self.repond(code, "", "text/plain", [("Location", cible)])

    def statut(self, chemin):
        try:
            code = int(chemin.rsplit("/", 1)[1])
        except ValueError:
            code = 400
        return self.repond(code, "<h1>%d</h1>" % code)

    def lenteur(self, chemin):
        """`/delay/2` : deux secondes avant la premiere ligne de reponse.

        Le sommeil est *avant* l'en-tete, volontairement : c'est le cas qui fait
        mal, celui ou un `fetch` synchrone tient le fil de l'interface sans
        qu'aucun octet ne soit encore arrive.
        """
        try:
            secondes = min(10.0, float(chemin.rsplit("/", 1)[1]))
        except ValueError:
            secondes = 1.0
        time.sleep(secondes)
        return self.json(200, {"attendu": secondes})

    def formulaire(self, methode, chemin, requete):
        if chemin == "/form/get" and methode == "GET":
            if requete:
                return self.repond(200, self.recu(requete, "GET"))
            return self.repond(200, self.page_formulaire("get"))
        if chemin == "/form/post":
            if methode == "POST":
                corps = self.corps_recu()
                recu = urllib.parse.parse_qs(corps)
                return self.repond(200, self.recu(recu, "POST", corps))
            return self.repond(200, self.page_formulaire("post"))
        return self.repond(404, "<h1>404</h1>")

    def recu(self, champs, methode, brut=""):
        lignes = "".join("<li>%s = %s</li>" % (nom, ", ".join(valeurs))
                         for nom, valeurs in sorted(champs.items()))
        return ("<!doctype html><meta charset=utf-8><title>recu</title>"
                "<h1 id=resultat>recu</h1><p id=methode>%s</p><ul id=champs>%s</ul>"
                "<pre id=brut>%s</pre>" % (methode, lignes, brut))

    def page_formulaire(self, methode):
        action = "/form/%s" % methode
        return ("<!doctype html><meta charset=utf-8><title>formulaire %s</title>"
                "<form id=f action=%s method=%s>"
                "<label for=nom>Nom</label><input id=nom name=nom value=''>"
                "<label for=mdp>Mot de passe</label>"
                "<input id=mdp name=mdp type=password value=''>"
                "<input id=coche name=coche type=checkbox value=oui>"
                "<button id=envoi type=submit>Envoyer</button></form>"
                % (methode, action, methode))

    def module(self, chemin):
        """Les modules. `main.js` importe deux voisins, dont un sous-dossier.

        Le point qu'ils eprouvent : un `import "./util.js"` ecrit dans
        `/module/lib/composant.js` doit se resoudre en `/module/lib/util.js`, et
        non en `/module/util.js`. La resolution se fait par rapport au module
        importeur, pas au document — c'est l'erreur classique, et elle ne se voit
        que si l'arborescence a plus d'un niveau.
        """
        modules = {
            "/module/main.js":
                'import { salut } from "./util.js";\n'
                'import composant from "./lib/composant.js";\n'
                'globalThis.__modules = salut() + "|" + composant();\n',
            "/module/util.js":
                'export function salut() { return "util-racine"; }\n',
            "/module/lib/composant.js":
                'import { aide } from "./util.js";\n'
                'export default function () { return "composant+" + aide(); }\n',
            "/module/lib/util.js":
                'export function aide() { return "util-voisin"; }\n',
            "/module/cycle-a.js":
                'import { b } from "./cycle-b.js";\n'
                'export const a = "a";\n'
                'globalThis.__cycle = String(b) + a;\n',
            "/module/cycle-b.js":
                'import { a } from "./cycle-a.js";\n'
                'export const b = "b";\n',
        }
        corps = modules.get(chemin)
        if corps is None:
            return self.repond(404, "not found", "text/plain")
        return self.repond(200, corps, "application/javascript; charset=utf-8")

    def temoin(self, chemin):
        if chemin == "/cookie/pose":
            return self.repond(200, "<p id=pose>pose</p>", "text/html", [
                ("Set-Cookie", "session=secret; Path=/; HttpOnly"),
                ("Set-Cookie", "theme=sombre; Path=/"),
            ])
        recu = self.headers.get("Cookie") or ""
        return self.repond(200, "<pre id=recu>%s</pre>" % recu)

    def page(self, chemin):
        """Sert une page temoin du depot, telle quelle."""
        nom = os.path.basename(chemin)
        fichier = os.path.join(PAGES, nom)
        if not nom.endswith(".html") or not os.path.exists(fichier):
            return self.repond(404, "<h1>404</h1><p>%s</p>" % nom)
        with open(fichier, "rb") as f:
            return self.repond(200, f.read())


class Serveur(socketserver.ThreadingTCPServer):
    """Un fil par requete : sans cela `/delay/2` bloquerait tout le serveur.

    C'est justement ce qu'on veut mesurer cote navigateur, pas cote serveur —
    un serveur qui se bloque lui-meme rendrait le test inutile.
    """

    daemon_threads = True
    allow_reuse_address = True

    def __init__(self, adresse, bavard=False):
        self.bavard = bavard
        super().__init__(adresse, Fixtures)


def demarre(port=PORT_DEFAUT, bavard=False):
    """Lance le serveur dans un fil et rend `(serveur, base)`.

    C'est la forme dont les verifications ont besoin : elles demarrent le
    serveur, mesurent, puis appellent `arrete()`. Le port 0 laisse le systeme
    en choisir un libre, ce qui evite qu'une execution en parallele echoue sur
    un port deja pris.
    """
    serveur = Serveur(("127.0.0.1", port), bavard)
    fil = threading.Thread(target=serveur.serve_forever, daemon=True)
    fil.start()
    reel = serveur.server_address[1]
    return serveur, "http://127.0.0.1:%d" % reel


def arrete(serveur):
    serveur.shutdown()
    serveur.server_close()


class atelier:
    """Gestionnaire de contexte : `with atelier() as base:`."""

    def __init__(self, port=0, origines=1, bavard=False):
        self.port = port
        self.origines = origines
        self.bavard = bavard
        self.serveurs = []
        self.bases = []

    def __enter__(self):
        for i in range(self.origines):
            serveur, base = demarre(self.port if i == 0 else 0, self.bavard)
            self.serveurs.append(serveur)
            self.bases.append(base)
        return self.bases[0] if self.origines == 1 else self.bases

    def __exit__(self, *_):
        for serveur in self.serveurs:
            arrete(serveur)
        return False


def principal(argv=None):
    analyseur = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    analyseur.add_argument("--port", type=int, default=PORT_DEFAUT)
    analyseur.add_argument("--origine-b", type=int, default=0,
                           help="ouvre une seconde origine sur ce port")
    analyseur.add_argument("--bavard", action="store_true")
    arguments = analyseur.parse_args(argv)

    serveurs = []
    serveur, base = demarre(arguments.port, arguments.bavard)
    serveurs.append(serveur)
    print("origine A : %s" % base)
    if arguments.origine_b:
        second, base_b = demarre(arguments.origine_b, arguments.bavard)
        serveurs.append(second)
        print("origine B : %s" % base_b)
    print("Ctrl-C pour arreter.")
    try:
        while True:
            time.sleep(3600)
    except KeyboardInterrupt:
        pass
    finally:
        for s in serveurs:
            arrete(s)
    return 0


if __name__ == "__main__":
    sys.exit(principal())
