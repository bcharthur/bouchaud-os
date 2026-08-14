"""IndexedDB : la memoire d'une application Web, cote base de donnees.

## Pourquoi celle-ci et pas `localStorage`

`localStorage` existe deja, et ne suffit pas. Il ne range que des chaines,
n'a ni transaction ni index, et surtout **il est synchrone** : une page qui y
ecrit un objet un peu gros bloque le fil de l'interface le temps de l'ecriture.
Une application qui garde son etat hors ligne — la moitie de ce qu'on appelle
aujourd'hui une WebApp — a besoin de l'autre.

## L'architecture, en quatre etages

    API JavaScript (IDBFactory, IDBDatabase, IDBTransaction, IDBObjectStore)
      ↓  chaque appel rend une IDBRequest, sans rien faire
    couche de requetes     (js.py : `_op_idb`)
      ↓  le travail part vers un fil, le resultat revient dans la file
    service par origine    (ce module : `Service`)
      ↓
    stockage persistant    (ce module : `_ecrit_atomique`)

Le point important est le second etage. Chaque appel de la page **rend
immediatement** une requete vide et rend la main ; le travail se fait ailleurs,
et son resultat n'est livre qu'au battement suivant de la boucle d'evenements.
C'est la meme discipline que le reseau, et pour la meme raison : une ecriture
qui bloque le fil de l'interface fige la page, et la promesse qu'on rend
n'y change rien.

## Le choix du stockage

Trois candidats ont ete pesés.

* **SQLite** — le choix d'un vrai navigateur. Ecarte : le CPython embarque n'a
  pas `sqlite3` construit, et l'ajouter voudrait dire porter une bibliotheque C
  entiere pour un sous-ensemble qui n'a besoin ni de SQL, ni d'index composes,
  ni de jointures.
* **Un B-tree maison** — necessaire si les bases devaient depasser la memoire.
  Elles ne le peuvent pas : la zone `/persist` se compte en mebioctets. Ecrire
  un arbre sur disque pour des donnees qui tiennent en RAM serait payer une
  complexite pour un probleme qu'on n'a pas.
* **Un fichier structure par base, ecrit d'un bloc** — retenu.

Ce que le troisieme donne, et qui etait le cahier des charges :

* **persistance** : sous `/persist`, avec `fsync`, comme les temoins et le
  cache. C'est la seule zone qui survit a l'extinction ;
* **atomicite minimale** : ecriture dans un fichier temporaire, `fsync`, puis
  `rename`. Un `rename` ne laisse jamais voir un etat intermediaire : soit
  l'ancien fichier, soit le nouveau. Une coupure de courant au milieu d'un
  commit rend la base telle qu'elle etait avant ;
* **pas de corruption triviale** : un fichier illisible est traite comme une
  base vide plutot que de faire tomber la page — et l'ancien est conserve sous
  `.corrompu` pour qu'on puisse regarder ;
* **isolation par origine** : le nom du fichier contient l'origine. Deux sites
  qui ouvrent `mabase` ouvrent deux bases.

## Les limites, dites franchement

L'unite d'ecriture est **la base entiere**. Une transaction en lecture-ecriture
accumule ses modifications en memoire et les ecrit d'un seul bloc au `commit` ;
si elle echoue avant, rien n'est ecrit. C'est une atomicite reelle mais
grossiere : deux transactions concurrentes sur la meme base se serialisent, et
une base de plusieurs mebioctets est reecrite en entier pour une cle modifiee.
Pour ce que le module doit porter — l'etat d'une application, quelques milliers
d'enregistrements — c'est le bon compromis. Ce ne le serait pas pour un journal
de plusieurs dizaines de mebioctets, et c'est a ce moment-la qu'il faudra un
vrai moteur.

Ne sont pas faits : les index secondaires (`createIndex`), les curseurs, les
cles composees, `autoIncrement` au-dela d'un compteur simple, et le versionnage
concurrent entre onglets. Les cles sont des chaines ou des nombres.
"""

import json
import os
import threading
import urllib.parse

from . import stockage, telemetrie

# Ou vivent les bases. Sous la meme racine que le reste de ce qui persiste.
DOSSIER = "indexeddb"

# Au-dela, on refuse d'ecrire : une page ne doit pas pouvoir remplir la zone
# persistante et empecher le navigateur de garder ses temoins.
BASE_MAX_OCTETS = 2 * 1024 * 1024


# Ce que rend une lecture qui ne trouve rien. Voir `execute`, verbe `get`.
ABSENT = {"__bo_absent": True}


class Erreur(Exception):
    """Ce que la page recevra comme `event.target.error`."""

    def __init__(self, nom, message):
        super().__init__(message)
        self.nom = nom


def origine_de(url):
    """La cle d'isolement : deux origines ne partagent jamais une base."""
    morceaux = urllib.parse.urlsplit(url or "")
    if not morceaux.hostname:
        return "about"
    return "%s://%s" % (morceaux.scheme or "http", morceaux.netloc)


def _nom_de_fichier(origine, base):
    """Un nom de fichier sur, derive de l'origine et du nom de base.

    Le quotage n'est pas cosmetique : un nom de base est choisi par la page, et
    `../../temoins.json` en est un nom valide. Sans echappement, une page
    pourrait ecrire par-dessus le bocal a temoins.
    """
    sur = urllib.parse.quote("%s|%s" % (origine, base), safe="")
    return os.path.join(DOSSIER, sur + ".json")


def _chemin(nom):
    return os.path.join(stockage.RACINE, nom)


def _lit_base(nom):
    """Lit une base, ou rend une base vide si elle n'existe pas.

    Un fichier illisible est mis de cote plutot que jete : la page repart d'une
    base vide — ce qu'elle sait gerer, c'est le cas du premier lancement — et
    ce qui restait est conserve pour diagnostic.
    """
    chemin = _chemin(nom)
    try:
        with open(chemin, "r", encoding="utf-8") as fichier:
            donnees = json.load(fichier)
    except FileNotFoundError:
        return {"version": 0, "magasins": {}, "compteurs": {}}
    except (OSError, ValueError):
        telemetrie.compte("idb.base_illisible")
        try:
            os.replace(chemin, chemin + ".corrompu")
        except OSError:
            pass
        return {"version": 0, "magasins": {}, "compteurs": {}}
    if not isinstance(donnees, dict) or "magasins" not in donnees:
        return {"version": 0, "magasins": {}, "compteurs": {}}
    return donnees


def _ecrit_atomique(nom, donnees):
    """Ecrit une base d'un seul bloc. Rend `True` si elle a atteint le disque.

    Temporaire, `fsync`, puis `rename` : c'est la sequence qui donne
    l'atomicite. Ecrire directement dans le fichier definitif laisserait, en
    cas de coupure, une base tronquee — c'est-a-dire perdue, puisqu'un JSON
    incomplet ne se relit pas. Avec le renommage, une coupure ne fait que
    conserver l'etat precedent.

    `fsync` **avant** le renommage, pas apres : renommer vers un contenu qui
    n'est pas encore sur le disque echangerait une base valide contre une base
    vide.
    """
    if not stockage.disponible():
        return False
    chemin = _chemin(nom)
    try:
        os.makedirs(os.path.dirname(chemin), exist_ok=True)
    except OSError:
        return False

    texte = json.dumps(donnees)
    if len(texte) > BASE_MAX_OCTETS:
        raise Erreur("QuotaExceededError",
                     "base de %d octets, maximum %d" % (len(texte), BASE_MAX_OCTETS))

    temporaire = chemin + ".tmp"
    try:
        with open(temporaire, "w", encoding="utf-8") as fichier:
            fichier.write(texte)
            fichier.flush()
            try:
                os.fsync(fichier.fileno())
            except OSError:
                pass
        os.replace(temporaire, chemin)
        return True
    except OSError:
        try:
            os.unlink(temporaire)
        except OSError:
            pass
        return False


def _cle_valide(cle):
    """Les cles admises : chaine ou nombre.

    Le sous-ensemble est documente et voulu. Une cle qui n'en est pas une doit
    lever plutot que d'etre convertie en silence — une page qui range sous
    `{a: 1}` et relit sous `"[object Object]"` a un bogue, et le lui cacher ne
    l'aide pas.
    """
    if isinstance(cle, bool):
        raise Erreur("DataError", "un booleen n'est pas une cle")
    if isinstance(cle, (int, float)):
        return "n:%r" % float(cle)
    if isinstance(cle, str):
        return "s:" + cle
    raise Erreur("DataError", "cle non prise en charge : %r" % (cle,))


def _cle_rendue(interne):
    """L'inverse de `_cle_valide` : ce que la page reverra."""
    if interne.startswith("n:"):
        valeur = float(interne[2:])
        return int(valeur) if valeur.is_integer() else valeur
    return interne[2:]


class Transaction:
    """Un lot de modifications, visible d'un coup ou pas du tout.

    En lecture seule, elle ne fait qu'observer l'instantane pris a l'ouverture.
    En lecture-ecriture, elle accumule ses modifications dans une copie et ne
    les publie qu'au `commit`. C'est ce qui garantit qu'une transaction
    interrompue en chemin ne laisse pas la moitie de ses ecritures visibles.
    """

    __slots__ = ("service", "base", "mode", "donnees", "vivante", "modifiee",
                 "touches")

    def __init__(self, service, base, mode):
        self.service = service
        self.base = base
        self.mode = mode
        # Une copie, pas une reference : ecrire dans l'original rendrait les
        # modifications visibles avant le commit, et un abandon ne saurait plus
        # les retirer.
        self.donnees = json.loads(json.dumps(service._donnees(base))) \
            if mode == "readwrite" else service._donnees(base)
        self.vivante = True
        self.modifiee = False
        # Les magasins que cette transaction a reellement ecrits. C'est ce qui
        # sera publie, et **rien d'autre** : voir `commit`.
        self.touches = set()

    def magasin(self, nom):
        magasins = self.donnees.setdefault("magasins", {})
        if nom not in magasins:
            raise Erreur("NotFoundError", "magasin inconnu : %s" % nom)
        return magasins[nom]

    def ecriture_permise(self):
        if self.mode != "readwrite":
            raise Erreur("ReadOnlyError",
                         "transaction en lecture seule")

    def commit(self):
        """Publie **les magasins touches**, fondus dans l'etat courant.

        Pas l'instantane entier, et la difference est une perte de donnees.
        Une transaction prend a sa naissance une copie de toute la base ; la
        publier telle quelle au commit remet donc en place la valeur qu'avaient
        **tous les autres magasins a cet instant-la**. Deux transactions qui se
        chevauchent sur des magasins differents s'effacent alors l'une l'autre,
        et c'est la derniere qui commite qui gagne.

        Ce n'est pas theorique : la page temoin ecrit son profil a la
        soumission du formulaire et ses messages depuis un gestionnaire
        WebSocket. Quand un message arrivait entre l'ouverture de la
        transaction du profil et son commit, la transaction des messages
        publiait un instantane pris avant le profil — et le profil disparaissait
        du disque. L'epreuve de persistance echouait alors une fois sur six,
        avec un fichier bien present et un profil absent.

        La norme n'autorise pas ce recouvrement : une transaction a une portee,
        et deux portees qui se croisent sont serialisees. Ne republier que ce
        qu'on a touche donne le meme resultat observable sans avoir a ordonner
        les transactions entre elles.
        """
        if not self.vivante:
            return False
        self.vivante = False
        if self.mode != "readwrite" or not self.modifiee:
            return True
        return self.service._publie_magasins(self.base, self.donnees,
                                             self.touches)

    def abandonne(self):
        """Jette tout ce que la transaction avait prepare."""
        self.vivante = False
        self.donnees = None
        telemetrie.compte("idb.transactions_abandonnees")


class Service:
    """Les bases d'une origine, et les transactions en cours.

    Un service par origine : c'est l'isolement, et il n'est pas facultatif.
    Sans lui, deux sites qui appellent leur base `donnees` liraient les memes
    enregistrements.
    """

    def __init__(self, origine):
        self.origine = origine
        self._bases = {}
        self._verrou = threading.RLock()

    # -- Acces au stockage ----------------------------------------------------

    def _donnees(self, base):
        with self._verrou:
            if base not in self._bases:
                self._bases[base] = _lit_base(_nom_de_fichier(self.origine, base))
            return self._bases[base]

    def _publie_magasins(self, base, donnees, touches):
        """Recopie les magasins nommes depuis `donnees` dans l'etat courant.

        L'etat courant est relu **sous le verrou**, juste avant l'ecriture :
        c'est ce qui fait que ce qu'une autre transaction a publie entre-temps
        survit.
        """
        with self._verrou:
            if not touches:
                return True
            courant = json.loads(json.dumps(self._donnees(base)))
            magasins = courant.setdefault("magasins", {})
            source = donnees.get("magasins", {})
            for nom in touches:
                if nom in source:
                    magasins[nom] = source[nom]
                else:
                    magasins.pop(nom, None)
            # Les compteurs de cles automatiques suivent leur magasin.
            compteurs_source = donnees.get("compteurs", {})
            compteurs = courant.setdefault("compteurs", {})
            for nom in touches:
                if nom in compteurs_source:
                    compteurs[nom] = compteurs_source[nom]
            return self._publie(base, courant)

    def _publie(self, base, donnees):
        with self._verrou:
            ecrit = _ecrit_atomique(_nom_de_fichier(self.origine, base), donnees)
            # La copie en memoire devient la verite meme si le disque n'a pas
            # pu suivre : sur une machine sans zone persistante, la base doit
            # rester utilisable pour la duree de la session.
            self._bases[base] = donnees
            telemetrie.compte("idb.commits")
            return ecrit

    # -- Ouverture ------------------------------------------------------------

    def ouvre(self, base, version):
        """Ouvre une base. Rend `(version, magasins, mise_a_jour_requise)`.

        `mise_a_jour_requise` dit a la page qu'elle doit recevoir
        `onupgradeneeded` : c'est le seul moment ou elle a le droit de creer
        des magasins, et c'est ce qui rend le schema d'une base previsible.
        """
        with self._verrou:
            donnees = self._donnees(base)
            actuelle = int(donnees.get("version", 0))
            if version is None:
                # `open(nom)` sans version : on prend ce qui existe, et 1 si la
                # base est neuve.
                version = actuelle or 1
            version = int(version)
            if version < actuelle:
                raise Erreur("VersionError",
                             "version %d demandee, %d deja en place"
                             % (version, actuelle))
            telemetrie.compte("idb.ouvertures")
            return actuelle, sorted(donnees.get("magasins", {})), version > actuelle

    def applique_montee(self, base, version, creations, suppressions):
        """Effectue la montee de version demandee par `onupgradeneeded`.

        Tout part en une seule ecriture : la nouvelle version et les magasins
        creees. Si l'ecriture echoue, la base garde son ancienne version — et
        la page recevra `onupgradeneeded` de nouveau au prochain lancement,
        ce qui est le comportement correct.
        """
        with self._verrou:
            donnees = json.loads(json.dumps(self._donnees(base)))
            magasins = donnees.setdefault("magasins", {})
            for nom in suppressions:
                magasins.pop(nom, None)
                donnees.get("compteurs", {}).pop(nom, None)
            for nom, options in creations:
                if nom not in magasins:
                    magasins[nom] = {}
                    donnees.setdefault("cles", {})[nom] = options or {}
            donnees["version"] = int(version)
            self._publie(base, donnees)
            return sorted(magasins)

    def supprime(self, base):
        with self._verrou:
            self._bases.pop(base, None)
            try:
                os.unlink(_chemin(_nom_de_fichier(self.origine, base)))
            except OSError:
                pass
            return True

    # -- Transactions ---------------------------------------------------------

    def transaction(self, base, mode):
        with self._verrou:
            self._donnees(base)   # charge la base si besoin
            return Transaction(self, base, mode)


# --- Instances par origine ----------------------------------------------------

_services = {}
_verrou_services = threading.Lock()


def service(url):
    """Le service de l'origine de cette adresse."""
    origine = origine_de(url)
    with _verrou_services:
        if origine not in _services:
            _services[origine] = Service(origine)
        return _services[origine]


def reinitialise():
    """Oublie tout ce qui est en memoire. Pour les epreuves."""
    with _verrou_services:
        _services.clear()


# --- Les operations que la page demande ---------------------------------------
#
# Une seule fonction par verbe, sans etat propre : c'est ce qui permet de les
# executer sur un fil de travail sans se demander ce qui est partage.


def execute(transaction, verbe, magasin, cle, valeur):
    """Effectue une operation dans une transaction. Rend ce que la page verra."""
    if not transaction.vivante:
        raise Erreur("TransactionInactiveError", "transaction terminee")
    telemetrie.compte("idb.operations")

    if verbe == "get":
        contenu = transaction.magasin(magasin)
        interne = _cle_valide(cle)
        if interne not in contenu:
            # Absent et « range a `null` » ne sont pas la meme chose, et la
            # norme les distingue : un `get` qui ne trouve rien rend
            # `undefined`. Python n'a qu'un `None`, qui traverse le pont en
            # `null` — d'ou ce marqueur, que le prelude retraduit. Sans lui, une
            # page qui teste `if (r.result === undefined)` pour savoir s'il faut
            # creer l'enregistrement ne le creerait jamais.
            return ABSENT
        return contenu[interne]

    if verbe == "getAll":
        contenu = transaction.magasin(magasin)
        return [contenu[k] for k in sorted(contenu)]

    if verbe == "getAllKeys":
        contenu = transaction.magasin(magasin)
        return [_cle_rendue(k) for k in sorted(contenu)]

    if verbe == "count":
        return len(transaction.magasin(magasin))

    if verbe in ("put", "add"):
        transaction.ecriture_permise()
        contenu = transaction.magasin(magasin)
        interne = _cle_valide(cle)
        if verbe == "add" and interne in contenu:
            raise Erreur("ConstraintError",
                         "cle deja presente : %r" % (cle,))
        contenu[interne] = valeur
        transaction.modifiee = True
        transaction.touches.add(magasin)
        return cle

    if verbe == "delete":
        transaction.ecriture_permise()
        transaction.magasin(magasin).pop(_cle_valide(cle), None)
        transaction.modifiee = True
        transaction.touches.add(magasin)
        return None

    if verbe == "clear":
        transaction.ecriture_permise()
        transaction.donnees["magasins"][magasin] = {}
        transaction.modifiee = True
        transaction.touches.add(magasin)
        return None

    raise Erreur("NotSupportedError", "operation inconnue : %s" % verbe)
