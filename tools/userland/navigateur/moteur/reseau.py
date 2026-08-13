"""Chargement des ressources : HTTP, HTTPS, fichiers locaux, pages internes.

Rien de special au moteur ici : on se sert de `http.client` et de `ssl`, qui
sont dans la bibliotheque standard de l'interprete embarque. C'est le noyau de
Bouchaud OS qui porte les sockets sous-jacentes.

## Ce qui rend le chargement rapide

Trois choses, et elles comptent plus ici qu'ailleurs parce que la pile TCP est
celle du noyau et que tout tourne sous emulation.

**Les connexions sont reutilisees.** Une page cite trente ressources sur le
meme hote ; sans reserve, c'est trente poignees de main TCP et trente poignees
de main TLS. La reserve les ramene a quelques-unes. C'est de loin le gain le
plus gros.

**Les corps sont compresses.** `gzip` divise une page HTML par cinq ou six. Sur
un lien lent — et le notre l'est — cela se voit directement.

**Les sous-ressources partent ensemble.** Voir `prechargement.py` : elles sont
demandees en parallele et deposees ici, pretes a etre reprises sans reseau.
"""

import gzip
import http.client
import os
import socket
import threading
import time
import ssl
import urllib.parse
import zlib

from . import securite
from . import stockage
from . import telemetrie
from . import transport as mod_transport

DELAI = 20.0
AGENT = "BouchaudOS/1.0 (navigateur natif; Qt/Python)"
REDIRECTIONS_MAX = 5

# --- Reserve de connexions ----------------------------------------------------

# Connexions gardees ouvertes, par `(securise, hote, port)`. Bornee : au-dela,
# on ferme plutot que de retenir des prises qu'on ne rouvrira jamais.
PRISES_PAR_HOTE = 4
_reserve = {}
_verrou = threading.Lock()


def _emprunte(cle):
    """Rend une connexion deja ouverte vers cet hote, ou `None`."""
    with _verrou:
        libres = _reserve.get(cle)
        while libres:
            connexion = libres.pop()
            if connexion.sock is not None:
                return connexion
            _ferme(connexion)
    return None


def _rend(cle, connexion):
    """Remet une connexion dans la reserve, ou la ferme si elle est de trop."""
    if connexion.sock is None:
        _ferme(connexion)
        return
    with _verrou:
        libres = _reserve.setdefault(cle, [])
        if len(libres) < PRISES_PAR_HOTE:
            libres.append(connexion)
            return
    _ferme(connexion)


def _ferme(connexion):
    try:
        connexion.close()
    except Exception:  # noqa: BLE001 — fermer ne doit jamais faire echouer
        pass


def ferme_connexions():
    """Ferme toute la reserve. Appele en quittant une page."""
    with _verrou:
        restantes = [c for libres in _reserve.values() for c in libres]
        _reserve.clear()
    for connexion in restantes:
        _ferme(connexion)


# --- Reponses prechargees -----------------------------------------------------

# Deposees par `prechargement`, reprises par `charge`. Chaque entree ne sert
# qu'une fois : le prechargement anticipe une demande, il ne remplace pas un
# cache — celui-ci viendra avec la persistance sur disque.
_precharges = {}


def depose(url, reponse):
    """Range une reponse obtenue d'avance, pour la prochaine demande."""
    with _verrou:
        _precharges[url] = reponse


def _reprend(url):
    with _verrou:
        return _precharges.pop(url, None)


def oublie_precharges():
    with _verrou:
        _precharges.clear()


class Reponse:
    __slots__ = ("url", "contenu", "type_mime", "code", "erreur", "entetes", "octets")

    def __init__(self, url, contenu="", type_mime="text/html", code=200, erreur=None,
                 entetes=None, octets=None):
        self.url = url
        self.contenu = contenu
        self.type_mime = type_mime
        self.code = code
        self.erreur = erreur
        self.entetes = entetes or {}
        # Le corps non decode. Une image, une police ou un script compresse ne
        # survivent pas au passage par `str` : ce qui doit rester binaire reste
        # ici.
        self.octets = octets if octets is not None else b""


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


def charge(url, methode="GET", corps=None, entetes=None, brut=False,
           document=None, destination=None):
    """Charge une URL et rend une [`Reponse`]. N'echoue jamais par exception.

    `methode`, `corps` et `entetes` servent aux requetes que le JavaScript de la
    page emet (`XMLHttpRequest`, `fetch`). `brut` demande a ne pas convertir le
    corps en HTML affichable : c'est ce qu'il faut pour une image ou un script,
    dont le contenu n'a pas a etre enrobe dans un `<pre>`.

    `document` est l'adresse du document qui demande la ressource. Le donner
    range la requete en provenance `DOCUMENT` : c'est la page qui demande, pas
    l'utilisateur, et la politique de `securite.py` s'applique. L'omettre decrit
    une navigation voulue par l'utilisateur, qui reste libre d'ouvrir un
    fichier local.

    ## Le seuil du courtier

    Quand un transport est installe (`transport.installe`), la requete lui est
    remise **au lieu** d'etre servie ici. C'est le seul endroit ou la bascule se
    fait, et c'est voulu : une vingtaine d'appelants passent par cette fonction —
    cascade, images, polices, prechargement, `fetch` — et un seul d'entre eux
    qui aurait garde un chemin direct aurait suffi a rendre l'isolation
    mensongere. Voir `transport.py`.
    """
    courtier = mod_transport.actuel()
    if courtier is not None:
        return courtier.charge(url, methode=methode, corps=corps,
                               entetes=entetes, brut=brut, document=document,
                               destination=destination)
    return charge_localement(url, methode=methode, corps=corps,
                             entetes=entetes, brut=brut, document=document,
                             destination=destination)


def charge_localement(url, methode="GET", corps=None, entetes=None, brut=False,
                      document=None, destination=None):
    """`charge` sans le seuil : va vraiment chercher la ressource.

    Le navigateur s'en sert pour honorer une requete que le renderer lui a
    courtee — c'est le bout de la chaine, celui qui a le droit d'ouvrir une
    prise. Personne d'autre ne devrait l'appeler directement.
    """
    if document is not None:
        requete = securite.requete_document(url, document,
                                            destination or "fetch")
        try:
            securite.verifie(requete)
        except securite.Refus as refus:
            telemetrie.note("securite_refus", requete.destination, url)
            return Reponse(url, "", "text/plain", 0, erreur=str(refus))
    try:
        if url.startswith("bo:"):
            return Reponse(url, _page_interne(url), "text/html")
        if url.startswith("file://"):
            return _charge_fichier(url, brut=brut)
        # Le prechargement a peut-etre deja rapporte cette ressource. Il ne sert
        # que les demandes qu'il a pu anticiper : une lecture simple, sans corps
        # ni en-tetes propres a l'appelant.
        if brut and methode == "GET" and corps is None and not entetes:
            avance = _reprend(url)
            if avance is not None:
                return avance
            # Puis le cache du disque : une ressource encore valable ne
            # justifie pas un aller-retour reseau.
            garde = stockage.cache().lit(url)
            if garde is not None:
                octets, entetes_gardes = garde
                type_mime = entetes_gardes.get("content-type", "").split(";")[0]
                return Reponse(url, octets.decode("utf-8", "replace"),
                               type_mime or "application/octet-stream", 200,
                               entetes=entetes_gardes, octets=octets)
        # YouTube n'est pas servi tel quel : sa page de lecture est une
        # application qu'on ne peut pas executer. On lui substitue une page qui
        # lit le flux — voir `lecteur_youtube`. `brut` court-circuite ce
        # detour : c'est le lecteur lui-meme qui redemande le media.
        if not brut and _detour_youtube is not None:
            substitut = _detour_youtube(url)
            if substitut is not None:
                return substitut
        return _charge_http(url, methode=methode, corps=corps,
                            entetes=entetes, brut=brut)
    except Exception as e:  # une page cassee ne doit pas tuer le navigateur
        telemetrie.ressource_echouee(url, 0, destination or "document")
        return Reponse(url, _page_erreur(url, e), "text/html", 0, str(e))


def tranche(url, debut, longueur, entetes=None):
    """Telecharge un morceau d'une ressource, par une requete `Range`.

    C'est ce qui permet de lire une video sans la tenir entierement en memoire,
    et de commencer avant la fin du telechargement. Un serveur qui ignore
    `Range` rend tout le corps avec un code 200 : l'appelant doit donc regarder
    le code, pas seulement la taille.
    """
    demandes = dict(entetes or {})
    demandes["Range"] = "bytes=%d-%d" % (debut, debut + longueur - 1)
    # Pas de compression sur une tranche : les octets demandes sont ceux du
    # fichier, et un serveur qui compresserait la tranche rendrait les decalages
    # faux — or c'est sur eux que le lecteur video s'appuie.
    demandes["Accept-Encoding"] = "identity"
    return charge(url, entetes=demandes, brut=True)


def taille_distante(url, entetes=None):
    """Taille totale d'une ressource, par une tranche d'un octet.

    `HEAD` serait plus direct, mais beaucoup de serveurs de media le refusent ou
    repondent sans `Content-Length` ; une tranche d'un octet donne un
    `Content-Range` qui porte la taille exacte.
    """
    reponse = tranche(url, 0, 1, entetes)
    portee = reponse.entetes.get("content-range", "")
    if "/" in portee:
        try:
            return int(portee.rsplit("/", 1)[1])
        except ValueError:
            pass
    longueur = reponse.entetes.get("content-length")
    if longueur and reponse.code == 200:
        try:
            return int(longueur)
        except ValueError:
            pass
    return 0


# Rappel installe par le navigateur pour detourner les adresses YouTube. Il
# reste `None` quand le moteur tourne sans lecteur — dans les verifications hors
# ligne, par exemple.
_detour_youtube = None


def installe_detour_youtube(rappel):
    """Branche (ou debranche, avec `None`) la substitution des pages YouTube."""
    global _detour_youtube
    _detour_youtube = rappel


def _charge_fichier(url, brut=False):
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
        donnees = f.read()
    if brut:
        # Une image, un son, une video : le contenu doit rester binaire, et
        # l'enrober de HTML le rendrait indecodable.
        return Reponse(url, "", _type_mime_du_chemin(chemin), 200, octets=donnees)
    texte = donnees.decode("utf-8", "replace")
    if chemin.endswith((".html", ".htm")):
        return Reponse(url, texte, "text/html", octets=donnees)
    return Reponse(url, "<pre>%s</pre>" % _echappe(texte), "text/html",
                   octets=donnees)


def _type_mime_du_chemin(chemin):
    """Type deduit de l'extension : `file://` n'en annonce aucun."""
    minuscule = chemin.lower()
    for suffixe, type_mime in (
        (".avi", "video/x-msvideo"), (".mp4", "video/mp4"),
        (".webm", "video/webm"), (".mkv", "video/x-matroska"),
        (".wav", "audio/wav"), (".mp3", "audio/mpeg"),
        (".ogg", "audio/ogg"), (".opus", "audio/opus"),
        (".png", "image/png"), (".jpg", "image/jpeg"),
        (".jpeg", "image/jpeg"), (".gif", "image/gif"),
    ):
        if minuscule.endswith(suffixe):
            return type_mime
    return "application/octet-stream"


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


# Les noms qui designent la machine elle-meme. Ils ne se demandent a personne :
# aucun serveur de noms exterieur n'a a savoir qu'on parle a sa propre boucle
# locale, et beaucoup refusent de repondre pour ces noms-la. Les faire passer
# par le reseau, c'est rendre `localhost` injoignable — donc rendre intestable
# tout ce qui se sert d'un serveur local, `wss://` en tete.
NOMS_LOCAUX = {
    "localhost": "127.0.0.1",
    "localhost.localdomain": "127.0.0.1",
    "ip6-localhost": "127.0.0.1",
}


def resout(nom):
    """Rend l'adresse IPv4 d'un nom. Leve `OSError` si elle est introuvable."""
    if _est_adresse(nom):
        return nom
    locale = NOMS_LOCAUX.get((nom or "").lower().rstrip("."))
    if locale is not None:
        return locale
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


def _meme_hote(depart, arrivee):
    """Vrai si les deux adresses partagent schema, hote et port."""
    return securite.meme_origine(depart, arrivee)


def _charge_http(url, restantes=REDIRECTIONS_MAX, methode="GET", corps=None,
                 entetes=None, brut=False):
    morceaux = urllib.parse.urlsplit(url)
    hote = morceaux.hostname
    if not hote:
        return Reponse(url, _page_erreur(url, "adresse incomprehensible"), "text/html", 0)
    chemin = morceaux.path or "/"
    if morceaux.query:
        chemin += "?" + morceaux.query

    securise = morceaux.scheme == "https"
    port = morceaux.port or (443 if securise else 80)
    cle = (securise, hote, port)

    entetes_envoyes = {
        "Host": morceaux.netloc,
        "User-Agent": AGENT,
        "Accept": "text/html,application/xhtml+xml,text/plain;q=0.9,*/*;q=0.8",
        # Un corps compresse arrive plusieurs fois plus vite. `http.client` ne
        # decompresse pas de lui-meme : c'est `_decompresse` qui s'en charge.
        "Accept-Encoding": "gzip, deflate",
        "Connection": "keep-alive",
    }
    # Les temoins du site, s'il en a depose : c'est ce qui fait qu'une
    # session ouverte le reste d'une page a l'autre, et d'un demarrage au
    # suivant.
    biscuits = stockage.temoins().pour(url)
    if biscuits:
        entetes_envoyes["Cookie"] = biscuits
    # Ce que la page a demande passe devant : un `fetch` qui pose son
    # `Content-Type` ou son `Authorization` doit etre obei.
    for nom, valeur in (entetes or {}).items():
        entetes_envoyes[str(nom)] = str(valeur)
    if corps is not None and "Content-Type" not in entetes_envoyes:
        entetes_envoyes["Content-Type"] = "text/plain;charset=UTF-8"

    charge_utile = corps.encode("utf-8") if isinstance(corps, str) else corps

    # Une connexion tiree de la reserve peut avoir ete fermee par le serveur
    # entre-temps, et rien ne le dit avant qu'on ecrive dedans. On reessaie donc
    # une fois sur une connexion neuve : c'est la reprise que doit avoir tout
    # client persistant, et sans elle la reserve serait moins fiable que pas de
    # reserve du tout.
    connexion = _emprunte(cle)
    for tentative in (0, 1):
        recyclee = connexion is not None
        if not recyclee:
            connexion = _ouvre(hote, port, securise)
        try:
            connexion.request(methode, chemin, body=charge_utile,
                              headers=entetes_envoyes)
            reponse = connexion.getresponse()
            entetes = {k.lower(): v for k, v in reponse.getheaders()}
            reponse_corps = reponse.read()
            code = reponse.status
            break
        except Exception:  # noqa: BLE001
            _ferme(connexion)
            connexion = None
            if not recyclee or tentative == 1:
                raise

    stockage.temoins().absorbe(url, entetes)

    detendu = _decompresse(reponse_corps, entetes.get("content-encoding", ""))
    if detendu is not reponse_corps:
        # Les en-tetes decrivent maintenant ce que l'appelant tient vraiment :
        # laisser `content-length` a la taille compressee ferait mentir tout
        # code qui s'y fie, `XMLHttpRequest` le premier.
        reponse_corps = detendu
        entetes.pop("content-encoding", None)
        entetes["content-length"] = str(len(reponse_corps))

    # C'est le serveur qui decide s'il garde la connexion, et `http.client` a
    # deja tranche : `will_close` tient compte de la version du protocole, de
    # l'en-tete `Connection` et du mode de delimitation du corps. Le refaire a
    # la main ne ferait que diverger.
    if reponse.will_close:
        _ferme(connexion)
    else:
        _rend(cle, connexion)

    if code in (301, 302, 303, 307, 308) and restantes > 0:
        cible = entetes.get("location")
        if cible:
            # Une redirection apres POST se poursuit en GET, comme le veut la
            # norme pour 301/302/303.
            suite = "GET" if code in (301, 302, 303) else methode
            suivante = urllib.parse.urljoin(url, cible)
            # Les en-tetes propres a la connexion precedente ne suivent pas :
            # reemettre le `Host` de l'ancien hote vers un autre domaine fait
            # servir la mauvaise page, ou aucune.
            abandonnes = {"host", "connection", "accept-encoding",
                          "content-length",
                          # `Cookie` est recalcule pour l'adresse d'arrivee par
                          # `temoins().pour()`. Le reprendre tel quel enverrait
                          # les temoins de l'hote de depart a celui d'arrivee.
                          "cookie"}
            if not _meme_hote(url, suivante):
                # Un `Authorization` est destine a l'hote qui l'a demande. Une
                # redirection vers un autre domaine ne lui donne pas le droit
                # de le lire : c'est ainsi qu'un jeton s'exfiltre.
                abandonnes.add("authorization")
                abandonnes.add("proxy-authorization")
            reprises = {n: v for n, v in entetes_envoyes.items()
                        if n.lower() not in abandonnes}
            return _charge_http(suivante, restantes - 1,
                                methode=suite,
                                corps=None if suite == "GET" else corps,
                                entetes=reprises, brut=brut)

    type_mime = entetes.get("content-type", "text/html").split(";")[0].strip()
    jeu = "utf-8"
    complet = entetes.get("content-type", "")
    if "charset=" in complet:
        jeu = complet.split("charset=")[1].split(";")[0].strip() or "utf-8"
    donnees = reponse_corps
    try:
        texte = donnees.decode(jeu, "replace")
    except LookupError:
        texte = donnees.decode("utf-8", "replace")

    if brut:
        # Image, script, feuille de style : l'appelant sait quoi en faire, et
        # l'enrober de HTML le rendrait inutilisable. C'est aussi ce qui merite
        # d'etre garde : une page change, une icone non.
        if code == 200 and methode == "GET":
            stockage.cache().depose(url, donnees, entetes)
        return Reponse(url, texte, type_mime, code, entetes=entetes, octets=donnees)

    if type_mime not in ("text/html", "application/xhtml+xml"):
        if type_mime.startswith("text/"):
            texte = "<pre>%s</pre>" % _echappe(texte)
        else:
            texte = ("<h1>Type non affichable</h1><p>%s — %d octets.</p>"
                     % (_echappe(type_mime), len(donnees)))
        type_mime = "text/html"

    return Reponse(url, texte, type_mime, code, entetes=entetes, octets=donnees)


def _ouvre(hote, port, securise):
    """Ouvre une connexion neuve, TLS compris s'il le faut."""
    adresse = resout(hote)
    prise = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    prise.settimeout(DELAI)
    prise.connect((adresse, port))

    if securise:
        contexte = contexte_tls()
        prise = contexte.wrap_socket(prise, server_hostname=hote)

    # La prise est deja ouverte : `http.client` s'en sert telle quelle au lieu
    # d'appeler `connect()`, donc sans repasser par `getaddrinfo`.
    connexion = http.client.HTTPConnection(hote, port, timeout=DELAI)
    connexion.sock = prise
    return connexion


def _decompresse(donnees, encodage):
    """Rend le corps en clair. Un encodage inconnu laisse les octets tels quels.

    Un corps annonce compresse mais illisible est rendu tel quel plutot que
    remplace par du vide : mieux vaut une page mal affichee qu'une page vide, et
    certains serveurs annoncent `gzip` sur des reponses qui ne le sont pas — les
    pages d'erreur surtout.
    """
    encodage = (encodage or "").strip().lower()
    if not donnees or not encodage or encodage == "identity":
        return donnees
    try:
        if encodage == "gzip":
            return gzip.decompress(donnees)
        if encodage == "deflate":
            try:
                return zlib.decompress(donnees)
            except zlib.error:
                # Certains serveurs emettent du deflate brut, sans en-tete zlib.
                return zlib.decompress(donnees, -zlib.MAX_WBITS)
    except Exception:  # noqa: BLE001
        return donnees
    return donnees


# Emplacements ou l'image de Bouchaud OS peut deposer ses racines. Le premier
# qui existe gagne. `BO_CA_BUNDLE` passe devant tout : c'est ce que la
# construction de l'image renseigne.
MAGASINS = (
    "/etc/ssl/certs/ca-certificates.crt",
    "/etc/pki/tls/certs/ca-bundle.crt",
    "/etc/ssl/cert.pem",
    "/usr/share/bouchaud/ca-bundle.crt",
)


class MagasinAbsent(Exception):
    """Aucune racine de confiance : HTTPS ne peut pas etre verifie."""


def contexte_tls():
    """Un contexte TLS qui verifie la chaine et le nom, ou leve.

    Le navigateur est **fail-closed**. Il fut un temps ou cette fonction
    retombait sur `CERT_NONE` quand la machine n'avait pas de magasin de
    racines : la page s'affichait, la barre d'etat le mentionnait, et
    n'importe quel intermediaire pouvait lire et reecrire le trafic sans que
    rien ne casse. Une securite qui s'efface toute seule quand elle gene ne
    protege personne — c'est meme pire qu'aucune securite, parce qu'on croit
    l'avoir.

    Une image sans racines doit donc echouer bruyamment, et la reparation est
    d'embarquer un magasin, pas d'assouplir le controle.
    """
    contexte = ssl.create_default_context()
    contexte.check_hostname = True
    contexte.verify_mode = ssl.CERT_REQUIRED

    # Un magasin nomme explicitement s'**ajoute** au magasin du systeme, il ne
    # le remplace pas et n'est pas un simple repli.
    #
    # C'etait le cas avant, et c'etait surprenant : `BO_CA_BUNDLE` n'avait
    # d'effet que sur une machine depourvue de racines. Poser la variable et
    # constater qu'elle ne change rien est le genre de comportement qui fait
    # perdre une heure — et c'est exactement ce qui est arrive en branchant une
    # autorite d'epreuve pour eprouver `wss://`.
    nomme = os.environ.get("BO_CA_BUNDLE")
    if nomme and os.path.exists(nomme):
        try:
            if os.path.isdir(nomme):
                contexte.load_verify_locations(capath=nomme)
            else:
                contexte.load_verify_locations(cafile=nomme)
        except Exception:  # noqa: BLE001 — magasin illisible : on continue
            pass

    if _magasin_charge(contexte):
        return contexte

    for chemin in _magasins_candidats():
        if not chemin or not os.path.exists(chemin):
            continue
        try:
            if os.path.isdir(chemin):
                contexte.load_verify_locations(capath=chemin)
            else:
                contexte.load_verify_locations(cafile=chemin)
        except Exception:  # noqa: BLE001 — magasin illisible : on essaie le suivant
            continue
        if _magasin_charge(contexte):
            return contexte

    raise MagasinAbsent(
        "aucun magasin de racines de confiance : HTTPS refuse. Deposez un "
        "faisceau de certificats a l'un des emplacements attendus ou nommez-le "
        "dans BO_CA_BUNDLE.")


def _magasins_candidats():
    yield os.environ.get("BO_CA_BUNDLE")
    yield os.environ.get("SSL_CERT_FILE")
    yield os.environ.get("SSL_CERT_DIR")
    for chemin in MAGASINS:
        yield chemin


def _magasin_charge(contexte):
    try:
        return bool(contexte.get_ca_certs())
    except Exception:  # noqa: BLE001
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
