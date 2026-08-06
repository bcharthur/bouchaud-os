"""Ce que le navigateur retient d'une session a l'autre.

Trois choses, qui n'ont en commun que de devoir survivre a l'extinction :

- les **temoins** (`Set-Cookie`), sans lesquels aucun site ne reconnait qui
  revient ;
- le **cache HTTP**, qui evite de retelecharger ce qui n'a pas change ;
- le **stockage local** (`localStorage`), ou une page range son propre etat.

## Ou cela vit

Sous `/persist`, la zone que le noyau deplie du disque au demarrage et y
reecrit sur `sync` (voir `src/fs/persistance.rs`). Le reste du systeme de
fichiers est en memoire et disparait a l'extinction ; c'est le seul endroit qui
ne disparait pas.

Quand cette zone n'existe pas — sur la machine de developpement, ou si le
disque est trop petit pour la porter — tout ce module fonctionne encore, en
memoire seule. Une session qui ne persiste pas vaut mieux qu'un navigateur qui
refuse de demarrer, et c'est ce qui permet aux verifications de tourner sans
l'OS.

## Ce qui n'est pas fait

Pas de chiffrement, pas de cloisonnement par site pour `localStorage` autre que
la cle d'origine, pas d'eviction du cache autre qu'une borne de taille. Un
navigateur de bureau fait plus ; ce qu'il fait de plus ne change pas ce qu'une
page voit.
"""

import json
import os
import time
import urllib.parse

# Racine des donnees persistantes. `/persist` est la zone du disque ; le reste
# du systeme de fichiers ne survit pas a l'extinction.
RACINE = os.environ.get("BO_PERSIST", "/persist/bo-navigateur")

# Au-dela, le cache se vide : mieux vaut retelecharger que remplir la zone.
CACHE_MAX_OCTETS = 4 * 1024 * 1024

# Ce qu'on refuse de mettre en cache, quelle que soit la reponse.
TAILLE_ENTREE_MAX = 512 * 1024


def disponible():
    """La zone persistante est-elle utilisable ?"""
    try:
        os.makedirs(RACINE, exist_ok=True)
        return os.path.isdir(RACINE)
    except OSError:
        return False


def _lit(nom):
    try:
        with open(os.path.join(RACINE, nom), "r", encoding="utf-8") as fichier:
            return json.load(fichier)
    except (OSError, ValueError):
        return None


def _ecrit(nom, valeur):
    """Ecrit un fichier de la zone. Rend `True` s'il a atteint le disque.

    `fsync` sur un fichier de `/persist` declenche l'ecriture reelle : sans lui
    le contenu ne serait que dans le RAMFS, et disparaitrait avec lui.
    """
    if not disponible():
        return False
    chemin = os.path.join(RACINE, nom)
    try:
        with open(chemin, "w", encoding="utf-8") as fichier:
            json.dump(valeur, fichier)
            fichier.flush()
            try:
                os.fsync(fichier.fileno())
            except OSError:
                pass
        return True
    except (OSError, TypeError, ValueError):
        return False


# --- Temoins ------------------------------------------------------------------

class Temoins:
    """Le bocal a temoins : ce que les sites deposent et redemandent.

    La correspondance suit ce que fait un navigateur sans etre exhaustive : le
    domaine se compare par suffixe, le chemin par prefixe, `Secure` reserve le
    temoin a HTTPS, et un temoin expire n'est ni rendu ni conserve. `SameSite`
    n'est pas interprete — il ne protege que contre des requetes croisees que
    le moteur n'emet pas.
    """

    FICHIER = "temoins.json"

    def __init__(self):
        self._temoins = _lit(self.FICHIER) or []
        self._nettoie()

    def _nettoie(self):
        maintenant = time.time()
        self._temoins = [t for t in self._temoins
                         if not t.get("expire") or t["expire"] > maintenant]

    def pour(self, url):
        """L'en-tete `Cookie` a envoyer pour cette adresse, ou `""`."""
        morceaux = urllib.parse.urlsplit(url)
        hote = (morceaux.hostname or "").lower()
        chemin = morceaux.path or "/"
        securise = morceaux.scheme == "https"
        if not hote:
            return ""

        self._nettoie()
        retenus = []
        for temoin in self._temoins:
            if temoin.get("secure") and not securise:
                continue
            if not _domaine_correspond(hote, temoin.get("domaine", "")):
                continue
            if not chemin.startswith(temoin.get("chemin", "/")):
                continue
            retenus.append("%s=%s" % (temoin["nom"], temoin["valeur"]))
        return "; ".join(retenus)

    def absorbe(self, url, entetes):
        """Retient les `Set-Cookie` d'une reponse. Rend le nombre retenu."""
        brut = entetes.get("set-cookie")
        if not brut:
            return 0
        morceaux = urllib.parse.urlsplit(url)
        hote = (morceaux.hostname or "").lower()
        if not hote:
            return 0

        retenus = 0
        for declaration in _separe_temoins(brut):
            temoin = _analyse_temoin(declaration, hote, morceaux.path or "/")
            if temoin is None:
                continue
            # Un temoin de meme nom, domaine et chemin remplace le precedent.
            self._temoins = [t for t in self._temoins
                             if not (t["nom"] == temoin["nom"]
                                     and t["domaine"] == temoin["domaine"]
                                     and t["chemin"] == temoin["chemin"])]
            if temoin.get("expire") and temoin["expire"] <= time.time():
                # `Set-Cookie` avec une date passee est la facon d'en effacer un.
                retenus += 1
                continue
            self._temoins.append(temoin)
            retenus += 1
        if retenus:
            _ecrit(self.FICHIER, self._temoins)
        return retenus

    def oublie_tout(self):
        self._temoins = []
        _ecrit(self.FICHIER, self._temoins)


def _domaine_correspond(hote, domaine):
    domaine = (domaine or "").lstrip(".").lower()
    if not domaine:
        return False
    return hote == domaine or hote.endswith("." + domaine)


def _separe_temoins(brut):
    """Separe plusieurs `Set-Cookie` reunis en une seule chaine.

    `http.client` joint les en-tetes repetes par des virgules, or une virgule
    apparait aussi dans une date `Expires`. On ne coupe donc que sur une virgule
    suivie de ce qui ressemble a un nouveau `nom=`.
    """
    morceaux = []
    courant = ""
    for partie in brut.split(","):
        if courant and "=" in partie.split(";")[0] \
                and not partie.strip()[:3].isdigit():
            morceaux.append(courant)
            courant = partie
        else:
            courant = (courant + "," + partie) if courant else partie
    if courant:
        morceaux.append(courant)
    return morceaux


def _analyse_temoin(declaration, hote, chemin_defaut):
    parties = [p.strip() for p in declaration.split(";") if p.strip()]
    if not parties or "=" not in parties[0]:
        return None
    nom, valeur = parties[0].split("=", 1)
    nom = nom.strip()
    if not nom:
        return None

    temoin = {
        "nom": nom,
        "valeur": valeur.strip(),
        "domaine": hote,
        "chemin": _chemin_parent(chemin_defaut),
        "secure": False,
        "expire": 0,
    }
    for attribut in parties[1:]:
        cle, _, contenu = attribut.partition("=")
        cle = cle.strip().lower()
        contenu = contenu.strip()
        if cle == "domain" and contenu:
            candidat = contenu.lstrip(".").lower()
            # Un site ne pose un temoin que sur son propre domaine ou un parent
            # dont il fait partie : sans cette regle, n'importe quel site
            # pourrait en poser un pour n'importe quel autre.
            if _domaine_correspond(hote, candidat):
                temoin["domaine"] = candidat
        elif cle == "path" and contenu.startswith("/"):
            temoin["chemin"] = contenu
        elif cle == "secure":
            temoin["secure"] = True
        elif cle == "max-age":
            try:
                temoin["expire"] = time.time() + float(contenu)
            except ValueError:
                pass
        elif cle == "expires" and contenu and not temoin["expire"]:
            date = _date_http(contenu)
            if date:
                temoin["expire"] = date
    return temoin


def _chemin_parent(chemin):
    if not chemin.startswith("/"):
        return "/"
    coupe = chemin.rfind("/")
    return chemin[:coupe] or "/" if coupe > 0 else "/"


_MOIS = {"jan": 1, "feb": 2, "mar": 3, "apr": 4, "may": 5, "jun": 6,
         "jul": 7, "aug": 8, "sep": 9, "oct": 10, "nov": 11, "dec": 12}


def _date_http(texte):
    """`Wed, 21 Oct 2026 07:28:00 GMT` en secondes depuis l'epoque. `0` sinon.

    `email.utils` ferait cela, mais l'importer tire tout le module `email` dans
    un interprete embarque ou chaque module compte.
    """
    try:
        morceaux = texte.replace(",", " ").replace("-", " ").split()
        jour = int(morceaux[1])
        mois = _MOIS[morceaux[2][:3].lower()]
        annee = int(morceaux[3])
        heure, minute, seconde = (int(v) for v in morceaux[4].split(":"))
        return _epoque(annee, mois, jour, heure, minute, seconde)
    except (IndexError, KeyError, ValueError):
        return 0


def _epoque(annee, mois, jour, heure, minute, seconde):
    """Secondes depuis 1970, en temps universel. Calendrier gregorien."""
    jours = 0
    for an in range(1970, annee):
        jours += 366 if _bissextile(an) else 365
    longueurs = [31, 29 if _bissextile(annee) else 28, 31, 30, 31, 30,
                 31, 31, 30, 31, 30, 31]
    jours += sum(longueurs[:mois - 1]) + (jour - 1)
    return jours * 86400 + heure * 3600 + minute * 60 + seconde


def _bissextile(annee):
    return annee % 4 == 0 and (annee % 100 != 0 or annee % 400 == 0)


# --- Cache HTTP ---------------------------------------------------------------

class Cache:
    """Ce qui a deja ete telecharge et peut resservir.

    Le cache est volontairement simple : une entree ne resssert que si elle n'a
    pas expire, et l'expiration vient de `Cache-Control: max-age` ou de
    `Expires`. Pas de revalidation par `ETag` — elle demanderait un aller-retour
    reseau, ce que le cache est justement la pour eviter ; ce qui a expire est
    simplement redemande.
    """

    INDEX = "cache-index.json"

    def __init__(self):
        self._index = _lit(self.INDEX) or {}
        self._octets = sum(e.get("taille", 0) for e in self._index.values())

    def _fichier(self, url):
        # Un nom de fichier stable et sans separateur, tire de l'adresse.
        empreinte = 0
        for octet in url.encode("utf-8"):
            empreinte = (empreinte * 131 + octet) & 0xFFFFFFFFFFFF
        return "cache/%012x" % empreinte

    def lit(self, url):
        """Le corps en cache pour cette adresse, ou `None`."""
        entree = self._index.get(url)
        if not entree:
            return None
        if entree.get("expire", 0) <= time.time():
            self._retire(url)
            return None
        try:
            with open(os.path.join(RACINE, entree["fichier"]), "rb") as fichier:
                return (fichier.read(), entree.get("entetes", {}))
        except OSError:
            self._retire(url)
            return None

    def depose(self, url, octets, entetes):
        """Retient une reponse si elle est cachable. Rend `True` alors."""
        if not disponible() or not octets:
            return False
        if len(octets) > TAILLE_ENTREE_MAX:
            return False

        duree = _duree_de_vie(entetes)
        if duree <= 0:
            return False

        # Le cache se vide plutot que de grandir sans fin : une eviction fine
        # coûterait un suivi d'usage pour un gain que la zone ne justifie pas.
        if self._octets + len(octets) > CACHE_MAX_OCTETS:
            self.vide()

        nom = self._fichier(url)
        try:
            os.makedirs(os.path.join(RACINE, "cache"), exist_ok=True)
            with open(os.path.join(RACINE, nom), "wb") as fichier:
                fichier.write(octets)
                fichier.flush()
                try:
                    os.fsync(fichier.fileno())
                except OSError:
                    pass
        except OSError:
            return False

        self._index[url] = {
            "fichier": nom,
            "taille": len(octets),
            "expire": time.time() + duree,
            "entetes": {c: v for c, v in entetes.items()
                        if c in ("content-type", "content-length")},
        }
        self._octets += len(octets)
        _ecrit(self.INDEX, self._index)
        return True

    def _retire(self, url):
        entree = self._index.pop(url, None)
        if not entree:
            return
        self._octets = max(0, self._octets - entree.get("taille", 0))
        try:
            os.remove(os.path.join(RACINE, entree["fichier"]))
        except OSError:
            pass
        _ecrit(self.INDEX, self._index)

    def vide(self):
        for url in list(self._index):
            self._retire(url)
        self._index = {}
        self._octets = 0
        _ecrit(self.INDEX, self._index)


def _duree_de_vie(entetes):
    """Duree pendant laquelle une reponse peut resservir, en secondes."""
    controle = (entetes.get("cache-control") or "").lower()
    if "no-store" in controle or "no-cache" in controle or "private" in controle:
        return 0
    for directive in controle.split(","):
        directive = directive.strip()
        if directive.startswith("max-age="):
            try:
                return max(0.0, float(directive[8:]))
            except ValueError:
                return 0
    expire = entetes.get("expires")
    if expire:
        date = _date_http(expire)
        if date:
            return max(0.0, date - time.time())
    return 0


# --- Stockage local -----------------------------------------------------------

class StockageLocal:
    """`localStorage` : ce qu'une page range pour elle-meme, par origine.

    Les origines sont cloisonnees, comme le veut la norme : un site ne lit pas
    ce qu'un autre a range. C'est la seule garantie de securite que ce module
    apporte, et elle n'est pas facultative — sans elle, une page pourrait lire
    le jeton de session qu'une autre a stocke.
    """

    FICHIER = "stockage-local.json"

    def __init__(self):
        self._tout = _lit(self.FICHIER) or {}

    def _origine(self, url):
        morceaux = urllib.parse.urlsplit(url or "")
        if not morceaux.hostname:
            return "about"
        return "%s://%s" % (morceaux.scheme or "http", morceaux.netloc)

    def lit(self, url, cle):
        return self._tout.get(self._origine(url), {}).get(str(cle))

    def ecrit(self, url, cle, valeur):
        origine = self._origine(url)
        self._tout.setdefault(origine, {})[str(cle)] = str(valeur)
        _ecrit(self.FICHIER, self._tout)

    def retire(self, url, cle):
        self._tout.get(self._origine(url), {}).pop(str(cle), None)
        _ecrit(self.FICHIER, self._tout)

    def cles(self, url):
        return list(self._tout.get(self._origine(url), {}).keys())

    def efface(self, url):
        self._tout[self._origine(url)] = {}
        _ecrit(self.FICHIER, self._tout)


# --- Instances partagees ------------------------------------------------------
#
# Un seul bocal, un seul cache, un seul stockage pour tout le navigateur : deux
# documents du meme site doivent voir les memes temoins, c'est le comportement
# attendu et c'est ce qui rend une session utilisable.

_temoins = None
_cache = None
_local = None


def temoins():
    global _temoins
    if _temoins is None:
        _temoins = Temoins()
    return _temoins


def cache():
    global _cache
    if _cache is None:
        _cache = Cache()
    return _cache


def local():
    global _local
    if _local is None:
        _local = StockageLocal()
    return _local


def reinitialise():
    """Oublie tout ce qui est en memoire. Sert aux verifications."""
    global _temoins, _cache, _local
    _temoins = None
    _cache = None
    _local = None
