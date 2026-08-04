"""Chargement des ressources : HTTP, HTTPS, fichiers locaux, pages internes.

Rien de special au moteur ici : on se sert de `http.client` et de `ssl`, qui
sont dans la bibliotheque standard de l'interprete embarque. C'est le noyau de
Bouchaud OS qui porte les sockets sous-jacentes.
"""

import http.client
import os
import socket
import time
import ssl
import urllib.parse

DELAI = 20.0
AGENT = "BouchaudOS/1.0 (navigateur natif; Qt/Python)"
REDIRECTIONS_MAX = 5


class Reponse:
    __slots__ = ("url", "contenu", "type_mime", "code", "erreur")

    def __init__(self, url, contenu="", type_mime="text/html", code=200, erreur=None):
        self.url = url
        self.contenu = contenu
        self.type_mime = type_mime
        self.code = code
        self.erreur = erreur


def normalise(saisie, base=None):
    """Transforme ce que l'utilisateur a tape en URL utilisable."""
    saisie = saisie.strip()
    if not saisie:
        return ""
    if saisie.startswith(("http://", "https://", "file://", "bo:")):
        return saisie
    if base:
        return urllib.parse.urljoin(base, saisie)
    if saisie.startswith("/"):
        return "file://" + saisie
    # Sans schema : un point suggere un domaine, sinon c'est une recherche.
    if " " not in saisie and "." in saisie:
        return "https://" + saisie
    return "https://duckduckgo.com/html/?q=" + urllib.parse.quote(saisie)


def charge(url):
    """Charge une URL et rend une [`Reponse`]. N'echoue jamais par exception."""
    try:
        if url.startswith("bo:"):
            return Reponse(url, _page_interne(url), "text/html")
        if url.startswith("file://"):
            return _charge_fichier(url)
        return _charge_http(url)
    except Exception as e:  # une page cassee ne doit pas tuer le navigateur
        return Reponse(url, _page_erreur(url, e), "text/html", 0, str(e))


def _charge_fichier(url):
    chemin = url[len("file://"):] or "/"
    if os.path.isdir(chemin):
        entrees = sorted(os.listdir(chemin))
        lignes = ["<h1>%s</h1><ul>" % _echappe(chemin)]
        for nom in entrees:
            complet = os.path.join(chemin, nom)
            suffixe = "/" if os.path.isdir(complet) else ""
            lignes.append('<li><a href="file://%s">%s%s</a></li>'
                          % (_echappe(complet), _echappe(nom), suffixe))
        lignes.append("</ul>")
        return Reponse(url, "".join(lignes), "text/html")
    with open(chemin, "rb") as f:
        brut = f.read()
    texte = brut.decode("utf-8", "replace")
    if chemin.endswith((".html", ".htm")):
        return Reponse(url, texte, "text/html")
    return Reponse(url, "<pre>%s</pre>" % _echappe(texte), "text/html")


# --- Resolution de noms ------------------------------------------------------
#
# `socket.getaddrinfo` ne fonctionne pas ici. La glibc y delegue a ses modules
# NSS, qui sont des bibliotheques partagees chargees par `dlopen` — impossible
# dans un binaire statique. C'est la limite dont l'editeur de liens previent a
# la construction, et elle est absolue : le navigateur doit resoudre lui-meme.
#
# Un client DNS minimal suffit : une requete A en UDP vers le serveur de
# `/etc/resolv.conf`, et la premiere adresse de la reponse.

_CACHE_DNS = {}


def _serveurs_dns():
    serveurs = []
    try:
        with open("/etc/resolv.conf") as f:
            for ligne in f:
                if ligne.startswith("nameserver"):
                    parties = ligne.split()
                    if len(parties) > 1:
                        serveurs.append(parties[1])
    except OSError:
        pass
    # Repli : la passerelle SLIRP de QEMU rend le service DNS a cette adresse.
    return serveurs or ["10.0.2.3"]


def _est_adresse(nom):
    parties = nom.split(".")
    return len(parties) == 4 and all(p.isdigit() and int(p) < 256 for p in parties)


def resout(nom):
    """Rend l'adresse IPv4 d'un nom. Leve `OSError` si elle est introuvable."""
    if _est_adresse(nom):
        return nom
    if nom in _CACHE_DNS:
        return _CACHE_DNS[nom]

    requete = _requete_dns(nom)
    derniere = "aucun serveur de noms"
    # Trois tentatives par serveur, comme le fait n'importe quel resolveur : UDP
    # perd des paquets, et la premiere emission peut echouer le temps que la
    # couche liaison apprenne l'adresse materielle de la passerelle. Abandonner
    # au premier essai ferait echouer toute la premiere navigation.
    for serveur in _serveurs_dns():
        for tentative in range(3):
            try:
                prise = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                try:
                    # L'emission se fait en mode bloquant : la premiere sortie
                    # vers un hote inconnu doit d'abord resoudre son adresse
                    # materielle, et une prise non bloquante ferait echouer
                    # l'envoi au lieu de laisser cette resolution aboutir.
                    prise.sendto(requete, (serveur, 53))
                    prise.settimeout(5.0)
                    reponse, _ = prise.recvfrom(2048)
                finally:
                    prise.close()
                adresse = _reponse_dns(reponse)
                if adresse:
                    _CACHE_DNS[nom] = adresse
                    return adresse
                derniere = "le serveur %s ne connait pas ce nom" % serveur
                break
            except OSError as e:
                derniere = "%s : %s" % (serveur, e)
                time.sleep(0.3 * (tentative + 1))
    raise OSError("resolution de %s impossible (%s)" % (nom, derniere))


def _requete_dns(nom):
    entete = b"\x12\x34\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00"
    question = b"".join(bytes([len(e)]) + e.encode("idna" if not e.isascii() else "ascii")
                        for e in nom.rstrip(".").split("."))
    return entete + question + b"\x00\x00\x01\x00\x01"


def _reponse_dns(paquet):
    if len(paquet) < 12:
        return None
    questions = int.from_bytes(paquet[4:6], "big")
    reponses = int.from_bytes(paquet[6:8], "big")
    position = 12
    for _ in range(questions):
        position = _saute_nom(paquet, position) + 4
    for _ in range(reponses):
        position = _saute_nom(paquet, position)
        if position + 10 > len(paquet):
            return None
        type_, longueur_donnees = (int.from_bytes(paquet[position:position + 2], "big"),
                                   int.from_bytes(paquet[position + 8:position + 10], "big"))
        position += 10
        if type_ == 1 and longueur_donnees == 4:
            return ".".join(str(o) for o in paquet[position:position + 4])
        position += longueur_donnees
    return None


def _saute_nom(paquet, position):
    while position < len(paquet):
        longueur = paquet[position]
        if longueur == 0:
            return position + 1
        if longueur & 0xC0 == 0xC0:  # pointeur de compression : deux octets
            return position + 2
        position += longueur + 1
    return position


def _charge_http(url, restantes=REDIRECTIONS_MAX):
    morceaux = urllib.parse.urlsplit(url)
    hote = morceaux.hostname
    if not hote:
        return Reponse(url, _page_erreur(url, "adresse incomprehensible"), "text/html", 0)
    chemin = morceaux.path or "/"
    if morceaux.query:
        chemin += "?" + morceaux.query

    adresse = resout(hote)
    securise = morceaux.scheme == "https"
    port = morceaux.port or (443 if securise else 80)

    prise = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    prise.settimeout(DELAI)
    prise.connect((adresse, port))

    if securise:
        contexte = ssl.create_default_context()
        # Sans magasin de racines sur la machine, toute connexion echouerait.
        # On verifie donc la chaine quand un magasin existe, et on s'en passe
        # sinon — la barre d'etat le dit.
        if not _magasin_disponible(contexte):
            contexte.check_hostname = False
            contexte.verify_mode = ssl.CERT_NONE
        prise = contexte.wrap_socket(prise, server_hostname=hote)

    # La prise est deja ouverte : `http.client` s'en sert telle quelle au lieu
    # d'appeler `connect()`, donc sans repasser par `getaddrinfo`.
    connexion = http.client.HTTPConnection(hote, port, timeout=DELAI)
    connexion.sock = prise

    try:
        connexion.request("GET", chemin, headers={
            "Host": morceaux.netloc,
            "User-Agent": AGENT,
            "Accept": "text/html,application/xhtml+xml,text/plain;q=0.9,*/*;q=0.8",
            "Accept-Encoding": "identity",
            "Connection": "close",
        })
        reponse = connexion.getresponse()
        entetes = {k.lower(): v for k, v in reponse.getheaders()}
        corps = reponse.read()
        code = reponse.status
    finally:
        connexion.close()

    if code in (301, 302, 303, 307, 308) and restantes > 0:
        cible = entetes.get("location")
        if cible:
            return _charge_http(urllib.parse.urljoin(url, cible), restantes - 1)

    type_mime = entetes.get("content-type", "text/html").split(";")[0].strip()
    jeu = "utf-8"
    complet = entetes.get("content-type", "")
    if "charset=" in complet:
        jeu = complet.split("charset=")[1].split(";")[0].strip() or "utf-8"
    try:
        texte = corps.decode(jeu, "replace")
    except LookupError:
        texte = corps.decode("utf-8", "replace")

    if type_mime not in ("text/html", "application/xhtml+xml"):
        if type_mime.startswith("text/"):
            texte = "<pre>%s</pre>" % _echappe(texte)
        else:
            texte = ("<h1>Type non affichable</h1><p>%s — %d octets.</p>"
                     % (_echappe(type_mime), len(corps)))
        type_mime = "text/html"

    return Reponse(url, texte, type_mime, code)


def _magasin_disponible(contexte):
    try:
        return bool(contexte.get_ca_certs())
    except Exception:
        return False


def _echappe(texte):
    return (str(texte).replace("&", "&amp;").replace("<", "&lt;")
            .replace(">", "&gt;"))


def _page_erreur(url, raison):
    return """
    <body style="font-family: sans-serif; margin: 40px">
      <h1 style="color:#b3261e">Chargement impossible</h1>
      <p><code>%s</code></p>
      <p style="color:#5f6368">%s</p>
      <p><a href="bo:accueil">Revenir a l'accueil</a></p>
    </body>
    """ % (_echappe(url), _echappe(raison))


PAGE_ACCUEIL = """
<body style="font-family: sans-serif; background-color:#f7f8fa; margin:0">
  <div style="background-color:#12203c; color:#ffffff; padding:36px 40px">
    <h1 style="margin:0; font-size:34px">Navigateur de Bouchaud OS</h1>
    <p style="margin:8px 0 0; color:#a9c4ee">
      Moteur natif ecrit en Python, affichage par Qt sur /dev/fb0, en ring 3.
    </p>
  </div>
  <div style="padding:28px 40px">
    <h2>Essayer</h2>
    <ul>
      <li><a href="http://example.com">example.com</a> — page de reference, en clair</li>
      <li><a href="https://www.wikipedia.org">wikipedia.org</a> — HTTPS reel</li>
      <li><a href="file:///">file:///</a> — parcourir le systeme de fichiers</li>
      <li><a href="bo:apropos">bo:apropos</a> — ce que le moteur sait faire</li>
    </ul>
    <h2>Raccourcis</h2>
    <ul>
      <li><b>Ctrl-L</b> : barre d'adresse &nbsp; <b>Entree</b> : charger</li>
      <li><b>Alt-Gauche</b> / <b>Alt-Droite</b> : precedent / suivant</li>
      <li><b>F5</b> : recharger &nbsp; <b>Molette</b> ou <b>Page haut/bas</b> : defiler</li>
      <li><b>Ctrl-Q</b> : quitter</li>
    </ul>
  </div>
</body>
"""

PAGE_APROPOS = """
<body style="font-family: sans-serif; margin:0 auto; padding:32px 40px">
  <h1>Ce que sait faire ce moteur</h1>
  <p>Il est ecrit pour Bouchaud OS, en Python, et n'emprunte son rendu a
     personne : Qt ne lui fournit que des rectangles et du texte.</p>
  <h2>Implemente</h2>
  <ul>
    <li>Analyse HTML tolerante : balises non fermees, imbrications interdites,
        attributs sans guillemets, commentaires, entites.</li>
    <li>CSS : selecteurs de balise, de classe, d'identifiant et de descendance ;
        specificite, cascade, heritage ; feuille de l'agent utilisateur.</li>
    <li>Modele de boite : marges, bordures, remplissage, largeurs et hauteurs.</li>
    <li>Mise en page bloc et en ligne, avec retour a la ligne mesure sur la
        vraie fonte.</li>
    <li>Listes a puces et numerotees, tableaux ramenes a des blocs, texte
        preformate.</li>
    <li>Reseau : HTTP et HTTPS, redirections, jeux de caracteres, plus
        <code>file://</code> pour le disque local.</li>
    <li>Navigation : historique avant/arriere, liens cliquables, defilement.</li>
  </ul>
  <h2>Pas implemente</h2>
  <ul>
    <li><b>JavaScript</b> — les pages qui se construisent elles-memes
        s'afficheront vides.</li>
    <li><b>Images</b> — remplacees par leur texte de remplacement.</li>
    <li><b>Flexbox et grid</b> — ramenes a un empilement vertical.</li>
    <li><b>Magasin de certificats</b> — HTTPS fonctionne, mais la chaine n'est
        verifiee que si le systeme fournit des racines.</li>
  </ul>
  <p><a href="bo:accueil">Retour</a></p>
</body>
"""


def _page_interne(url):
    nom = url[3:]
    if nom in ("", "accueil", "home"):
        return PAGE_ACCUEIL
    if nom == "apropos":
        return PAGE_APROPOS
    return "<body><h1>Page interne inconnue</h1><p><code>%s</code></p></body>" % _echappe(url)
