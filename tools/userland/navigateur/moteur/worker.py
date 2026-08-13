"""Un Worker : un second monde JavaScript, sur son propre fil.

## Ce qui rend un Worker different d'un contexte de plus

Un `iframe` est un autre document, mais il bat sur le meme fil : son JavaScript
et celui de la page se relaient, jamais ne se croisent. Un Worker, lui, doit
**calculer pendant** que la page continue. C'est sa seule raison d'exister — une
API `Worker` dont le travail se ferait sur le fil de l'interface serait une
facon compliquee de ne rien changer.

Trois choses en decoulent, et toutes les trois ont demande du travail ailleurs :

* **un `JSRuntime` par worker.** QuickJS n'est pas reentrant : deux fils dans le
  meme tas le corrompent. `bojs.cree` en fabriquait deja un par contexte, donc
  la separation etait acquise — il fallait la verifier, pas l'ecrire ;
* **le GIL relache pendant l'execution.** Sans cela, le fil du worker aurait
  tenu le verrou de Python pendant tout `JS_Eval`, et la page aurait gele comme
  si le calcul s'etait fait chez elle. C'est la modification de `bojs.cpp` :
  `Py_BEGIN_ALLOW_THREADS` autour de l'execution, `PyGILState_Ensure` dans le
  pont ;
* **un arret qui aboutit.** `terminate()` sur un `while (true) {}` ne peut pas
  attendre poliment. `bojs.interromps` leve un drapeau que le gardien
  d'interruption relit a chaque poignee d'instructions : c'est le seul point de
  rendez-vous sur entre deux fils qui partagent un runtime QuickJS — et encore,
  il ne partage qu'un `bool`.

## Ce que le worker ne fait pas lui-meme

Rien de ce qui engage le reseau ou le stockage. Une requete part vers **le
groupe de fils de l'hote**, traverse le meme `_recupere` — donc le meme CORS,
le meme `Origin`, la meme politique de schema — et revient dans la file du
worker. Une transaction IndexedDB passe par **le fil de stockage de l'hote**,
qui est unique et garantit l'ordre. Dupliquer l'un ou l'autre aurait donne deux
implementations a maintenir, et la seconde aurait fini par diverger sur un
detail de securite.

Le worker ne possede que ce qu'un fil doit posseder en propre : sa file
d'entree, ses minuteries, ses jetons de requete.

## La discipline de livraison

Rien n'entre dans le JavaScript du worker au milieu d'un script. Les messages,
les reponses et les echeances s'accumulent dans `_entrant` et ne sont livres
qu'entre deux tours de la boucle — la meme regle que la boucle d'evenements
d'un document, pour la meme raison : un script qui peut etre interrompe
n'importe ou n'a plus d'invariant.
"""

import json
import os
import threading
import time
import urllib.parse

import bojs

from . import origine as mod_origine, telemetrie, websocket

# Le fil dort au plus longtemps quand il n'a rien a faire : un message le
# reveille par l'evenement, une minuterie par son echeance. Sans borne, un
# worker inactif ne couterait rien du tout ; avec une borne courte, mille
# workers inactifs couteraient un fil qui se reveille sans cesse. Un quart de
# seconde est le compromis : imperceptible pour l'usager, negligeable pour la
# machine.
ATTENTE_MAX_S = 0.25

# Attente maximale d'un `terminate()`. Au-dela, le fil est declare recalcitrant
# et signale — ce qui n'est jamais arrive dans les epreuves, mais un
# `terminate()` qui bloque l'interface serait pire que le worker qu'il tue.
ARRET_MAX_S = 2.0

# Le budget d'une execution dans un worker, plus large que celui d'une page.
#
# Ce n'est pas une faveur : les deux budgets ne protegent pas de la meme chose.
# Celui d'une page existe parce qu'un script qui boucle **fige la fenetre**, et
# qu'il n'y a ni onglet a fermer ni gestionnaire de taches pour le rattraper.
# Un worker qui boucle ne fige rien — c'est tout l'interet de son fil —, et un
# calcul de trente secondes est un usage legitime de la chose. Le vrai garde-fou
# n'est plus le temps mais `terminate()`, qui aboutit toujours.
BUDGET_MS = 30000


def _chemin(nom):
    return os.path.join(os.path.dirname(os.path.abspath(__file__)), nom)


class Worker:
    """Un `DedicatedWorkerGlobalScope` vivant, et le fil qui l'execute."""

    def __init__(self, hote, identifiant, url, journal=None, budget_ms=BUDGET_MS):
        self.hote = hote
        self.identifiant = int(identifiant)
        self.url = str(url)
        self.journal = journal or (lambda niveau, texte: None)
        self._budget = budget_ms

        self._contexte = None
        self._arret = threading.Event()
        self._reveil = threading.Event()
        self._verrou = threading.Lock()

        # Ce qui attend d'entrer dans le worker, et ce qui attend d'en sortir.
        # Les deux files sont les seuls objets que les deux fils touchent, et
        # elles ne portent que des valeurs Python — jamais une reference a un
        # objet QuickJS, qui appartient a un seul fil.
        self._entrant = []
        self._sortant = []

        self._minuteries = {}      # identifiant -> (echeance, delai, repete)
        self._requetes = set()     # requetes reseau en vol
        self._sockets = {}
        self._transactions = {}    # jeton IndexedDB -> Transaction
        self._prochaine_transaction = 1

        self._fil = threading.Thread(target=self._boucle, daemon=True,
                                     name="bo-worker-%d" % self.identifiant)
        self._fil.start()
        telemetrie.compte("worker.crees")

    # --- Vu depuis le document ------------------------------------------------

    def poste(self, donnees):
        """Un `worker.postMessage(...)` de la page."""
        return self._depose(("message", donnees))

    def recolte(self):
        """Ce que le worker a dit depuis le dernier battement.

        Vidange sous verrou puis livraison hors verrou : un rappel de la page
        peut reposter au worker, et tenir le verrou pendant son execution serait
        un blocage certain.
        """
        with self._verrou:
            sortant, self._sortant = self._sortant, []
        return sortant

    def termine(self):
        """`worker.terminate()`, et tout ce que la norme y attache.

        Le contexte QuickJS est detruit, les minuteries oubliees, les requetes
        en vol abandonnees, les connexions fermees, les transactions rendues,
        les messages en attente jetes. C'est le fil du worker qui fait le
        menage — il est le seul a avoir le droit de toucher au tas QuickJS —,
        et `terminate()` ne fait que le lui demander puis l'attendre.
        """
        if self._arret.is_set():
            return
        self._arret.set()
        self._reveil.set()
        contexte = self._contexte
        if contexte is not None:
            # Le point important : un worker en boucle infinie doit s'arreter.
            # Le drapeau est relu par le gardien d'interruption a chaque poignee
            # d'instructions, donc l'arret aboutit meme si le script ne rend
            # jamais la main de lui-meme.
            try:
                bojs.interromps(contexte)
            except Exception:  # noqa: BLE001
                pass
        if self._fil is not threading.current_thread():
            self._fil.join(timeout=ARRET_MAX_S)
            if self._fil.is_alive():
                self.journal("warn",
                             "worker %s : le fil ne s'arrete pas" % self.url)
                telemetrie.compte("worker.arrets_manques")
        telemetrie.compte("worker.termines")

    @property
    def vivant(self):
        return not self._arret.is_set() and self._fil.is_alive()

    # --- Le fil ---------------------------------------------------------------

    def _depose(self, entree):
        with self._verrou:
            if self._arret.is_set():
                return False
            self._entrant.append(entree)
        self._reveil.set()
        return True

    def _emet(self, *entree):
        with self._verrou:
            self._sortant.append(entree)

    def _boucle(self):
        try:
            self._amorce()
        except Exception as e:  # noqa: BLE001 — un worker casse ne tue personne
            # Un `terminate()` pendant l'amorce fait echouer l'amorce : c'est le
            # resultat voulu, pas une panne. Le signaler comme une erreur
            # remplissait le journal a chaque worker ferme tot.
            if not self._arret.is_set():
                self._emet("erreur", str(e), "")
                self.journal("error", "worker %s : %s" % (self.url, e))
        try:
            while not self._arret.is_set():
                actif = self._traite_entrants()
                actif = self._traite_minuteries() or actif
                actif = self._traite_sockets() or actif
                self._pompe()
                if actif or self._arret.is_set():
                    continue
                self._reveil.wait(self._attente())
                self._reveil.clear()
        finally:
            self._ferme()

    def _amorce(self):
        """Cree le contexte, installe la surface Worker, charge le script."""
        self._contexte = bojs.cree(self._budget)
        bojs.pont(self._contexte, self.appel)
        # Un `terminate()` arrive pendant la creation : le drapeau etait deja
        # leve, mais il n'y avait pas encore de contexte a interrompre. On
        # rattrape ici, sans quoi le script s'executerait une fois de trop.
        if self._arret.is_set():
            bojs.interromps(self._contexte)

        for nom in ("prelude_partage.js", "prelude_worker.js"):
            with open(_chemin(nom), "r", encoding="utf-8") as fichier:
                bojs.evalue(self._contexte, fichier.read(), nom)

        source = self.hote.telecharge_worker(self.url)
        if source is None:
            raise RuntimeError("script de worker introuvable : %s" % self.url)

        # `location` est pose depuis l'hote : le worker n'a pas a le deviner, et
        # une adresse posee par le script serait une adresse choisie par lui.
        morceaux = _decoupe_url(self.url)
        bojs.evalue(self._contexte,
                    "globalThis.location = Object.freeze(%s);" % json.dumps(morceaux),
                    "<amorce>")
        bojs.evalue(self._contexte, "globalThis.__bo_installe_partage(globalThis"
                                    ".__bo_worker_env);", "<amorce>")
        bojs.evalue(self._contexte, source, self.url)

    def _attente(self):
        """Combien de temps dormir : jusqu'a la prochaine echeance, au plus."""
        if not self._minuteries:
            return ATTENTE_MAX_S
        maintenant = time.monotonic() * 1000.0
        prochaine = min(entree[0] for entree in self._minuteries.values())
        return max(0.0, min(ATTENTE_MAX_S, (prochaine - maintenant) / 1000.0))

    def _traite_entrants(self):
        with self._verrou:
            entrees, self._entrant = self._entrant, []
        for entree in entrees:
            genre = entree[0]
            if genre == "message":
                self._appelle("__bo_worker_message", [entree[1]])
            elif genre == "reponse":
                self._requetes.discard(entree[1])
                self._appelle("__bo_worker_reponse", [entree[1], entree[2]])
            elif genre == "idb":
                self._appelle("__bo_idb", [entree[1], entree[2], entree[3]])
        return bool(entrees)

    def _traite_minuteries(self):
        maintenant = time.monotonic() * 1000.0
        fait = False
        for identifiant in sorted(self._minuteries):
            entree = self._minuteries.get(identifiant)
            if entree is None or maintenant < entree[0]:
                continue
            _echeance, delai, repete = entree
            if repete:
                self._minuteries[identifiant] = (maintenant + delai, delai, True)
            else:
                del self._minuteries[identifiant]
            fait = True
            encore = self._appelle("__bo_worker_minuterie", [identifiant])
            if not encore and repete:
                self._minuteries.pop(identifiant, None)
        return fait

    def _traite_sockets(self):
        fait = False
        for identifiant, connexion in list(self._sockets.items()):
            for type_, charge in connexion.evenements():
                fait = True
                self._appelle("__bo_socket", [identifiant, type_, charge])
                if type_ == "close":
                    self._sockets.pop(identifiant, None)
        return fait

    def _pompe(self):
        if self._contexte is None or self._arret.is_set():
            return
        try:
            bojs.pompe(self._contexte)
        except Exception:  # noqa: BLE001
            pass

    def _appelle(self, nom, arguments):
        if self._contexte is None or self._arret.is_set():
            return None
        try:
            return bojs.appelle(self._contexte, nom, arguments)
        except Exception as e:  # noqa: BLE001
            self._emet("erreur", str(e), "")
            return None

    def _ferme(self):
        """Le menage, sur le fil du worker et nulle part ailleurs."""
        self._arret.set()
        self._minuteries.clear()
        for connexion in list(self._sockets.values()):
            try:
                connexion.ferme(1001, "worker termine")
            except Exception:  # noqa: BLE001
                pass
        self._sockets.clear()
        for transaction in list(self._transactions.values()):
            try:
                transaction.abandonne()
            except Exception:  # noqa: BLE001
                pass
        self._transactions.clear()
        self._requetes.clear()
        with self._verrou:
            del self._entrant[:]
        if self._contexte is not None:
            contexte, self._contexte = self._contexte, None
            try:
                bojs.detruit(contexte)
            except Exception:  # noqa: BLE001
                pass
            telemetrie.compte("worker.contextes_detruits")

    # --- Le pont --------------------------------------------------------------
    #
    # Toujours appele depuis le fil du worker, jamais depuis celui de
    # l'interface : c'est QuickJS qui appelle, et QuickJS ne tourne qu'ici.

    def appel(self, operation, *arguments):
        methode = getattr(self, "_op_" + str(operation), None)
        if methode is None:
            self.journal("warn", "worker : operation inconnue %s" % operation)
            return None
        try:
            return methode(*arguments)
        except Exception as e:  # noqa: BLE001
            self.journal("warn", "worker %s : %s" % (operation, e))
            return None

    # -- Messages et cycle de vie --

    def _op_wkVersHote(self, donnees):
        self._emet("message", donnees)
        return True

    def _op_wkErreur(self, message, pile):
        self._emet("erreur", str(message), str(pile or ""))
        return True

    def _op_wkFerme(self):
        # `self.close()` depuis le worker : il demande sa propre fin. Le fil
        # sort de sa boucle au tour suivant et fait son menage lui-meme.
        self._arret.set()
        self._reveil.set()
        return True

    def _op_wkConsole(self, niveau, texte):
        self._emet("console", str(niveau), str(texte))
        return True

    def _op_wkTemps(self):
        return time.monotonic() * 1000.0

    # -- Minuteries --

    def _op_wkMinuterie(self, identifiant, delai, repete):
        maintenant = time.monotonic() * 1000.0
        self._minuteries[int(identifiant)] = (maintenant + float(delai),
                                              float(delai), bool(repete))
        return True

    def _op_wkAnnuleMinuterie(self, identifiant):
        self._minuteries.pop(int(identifiant), None)
        return True

    # -- Reseau --

    def _op_wkAnalyseUrl(self, adresse, base):
        socle = str(base) if base else self.url
        return _decoupe_url(urllib.parse.urljoin(socle, str(adresse)))

    def _op_wkRequete(self, identifiant, methode, url, corps, entetes, temoins):
        """`fetch` dans un worker.

        **La meme infrastructure que la fenetre**, litteralement : le groupe de
        fils est celui de l'hote, et la requete traverse le `_recupere` du
        document — donc le meme preflight, le meme `Origin`, la meme lecture de
        `Access-Control-Allow-Origin`. Seul le retour differe : la reponse entre
        dans la file du worker au lieu de celle de la page.
        """
        identifiant = int(identifiant)
        absolue = urllib.parse.urljoin(self.url, str(url))
        avec_temoins = str(temoins or "same-origin") == "include"
        self._requetes.add(identifiant)

        def travaille():
            reponse = self.hote.recupere_pour(str(methode), absolue, corps,
                                              entetes or {}, avec_temoins)
            # Un worker termine pendant le vol : la reponse est jetee, et
            # `_depose` s'en charge tout seul. C'est ce qu'attend la norme d'un
            # `terminate()`, et c'est ce qui evite de reveiller un fil mort.
            self._depose(("reponse", identifiant, reponse))

        self.hote.soumets_reseau(travaille)
        return True

    def _op_wkImporte(self, adresse):
        absolue = urllib.parse.urljoin(self.url, str(adresse))
        return self.hote.telecharge_worker(absolue)

    # -- WebSocket : les memes noms d'operation que la fenetre --

    def _op_ouvreSocket(self, identifiant, url, protocoles):
        absolue = urllib.parse.urljoin(self.url, str(url))
        if absolue.startswith("http"):
            absolue = "ws" + absolue[4:]
        identifiant = int(identifiant)

        def travaille():
            connexion = websocket.Connexion(absolue, protocoles or (),
                                            document=self.url)
            if self._arret.is_set():
                return
            self._sockets[identifiant] = connexion
            try:
                connexion.ouvre()
            except websocket.Erreur as e:
                connexion._pose("error", str(e))
                connexion._pose("close", {"code": 1006, "raison": str(e),
                                          "propre": False})
            self._reveil.set()

        self.hote.soumets_reseau(travaille)
        return True

    def _op_envoieSocket(self, identifiant, donnees):
        connexion = self._sockets.get(int(identifiant))
        return bool(connexion and connexion.envoie(donnees))

    def _op_fermeSocket(self, identifiant, code, raison):
        connexion = self._sockets.get(int(identifiant))
        return bool(connexion and connexion.ferme(int(code or 1000), raison or ""))

    def _op_etatSocket(self, identifiant):
        connexion = self._sockets.get(int(identifiant))
        return websocket.FERME if connexion is None else connexion.etat

    # -- IndexedDB : le service et le fil de l'hote, les jetons du worker --
    #
    # Le service est par origine, donc un worker et son document ouvrent
    # litteralement la meme base — c'est ce que dit la norme, et c'est ce qui
    # rend un worker de synchronisation possible. Les jetons de transaction,
    # eux, sont propres au worker : un entier devine ne doit pas donner acces a
    # la transaction de quelqu'un d'autre.

    def _idb_en_fond(self, identifiant, travail):
        from . import indexeddb

        def enrobe():
            try:
                self._depose(("idb", int(identifiant), travail(), None))
            except indexeddb.Erreur as e:
                self._depose(("idb", int(identifiant), None,
                              {"nom": e.nom, "message": str(e)}))
            except Exception as e:  # noqa: BLE001
                self._depose(("idb", int(identifiant), None,
                              {"nom": "UnknownError", "message": str(e)}))

        self.hote.soumets_stockage(enrobe)

    def _op_idbOuvre(self, identifiant, base, version):
        version = None if version in (None, "", 0) else int(version)

        def travail():
            actuelle, magasins, montee = self.hote.service_idb().ouvre(
                str(base), version)
            return {"version": actuelle, "magasins": magasins, "montee": montee,
                    "cible": version if version is not None else actuelle or 1}

        self._idb_en_fond(identifiant, travail)
        return True

    def _op_idbMonte(self, identifiant, base, version, creations, suppressions):
        listes = [(str(nom), options or {}) for nom, options in (creations or [])]
        retires = [str(nom) for nom in (suppressions or [])]

        def travail():
            magasins = self.hote.service_idb().applique_montee(
                str(base), int(version), listes, retires)
            return {"version": int(version), "magasins": magasins}

        self._idb_en_fond(identifiant, travail)
        return True

    def _op_idbSupprime(self, identifiant, base):
        def travail():
            return self.hote.service_idb().supprime(str(base))

        self._idb_en_fond(identifiant, travail)
        return True

    def _op_idbTransaction(self, identifiant, base, mode):
        jeton = self._prochaine_transaction
        self._prochaine_transaction += 1
        mode = "readwrite" if mode == "readwrite" else "readonly"

        def travail():
            self._transactions[jeton] = self.hote.service_idb().transaction(
                str(base), mode)
            return jeton

        self._idb_en_fond(identifiant, travail)
        return jeton

    def _op_idbOperation(self, identifiant, jeton, verbe, magasin, cle, valeur):
        from . import indexeddb

        def travail():
            transaction = self._transactions.get(int(jeton))
            if transaction is None:
                raise indexeddb.Erreur("TransactionInactiveError",
                                       "transaction inconnue")
            return indexeddb.execute(transaction, str(verbe), str(magasin),
                                     cle, valeur)

        self._idb_en_fond(identifiant, travail)
        return True

    def _op_idbTermine(self, identifiant, jeton, valide):
        def travail():
            transaction = self._transactions.pop(int(jeton), None)
            if transaction is None:
                return False
            if valide:
                return transaction.commit()
            transaction.abandonne()
            return False

        self._idb_en_fond(identifiant, travail)
        return True


def _decoupe_url(url):
    """Les morceaux d'une adresse, sous la forme que `URL` et `location` veulent."""
    morceaux = urllib.parse.urlsplit(url)
    port = "" if morceaux.port is None else str(morceaux.port)
    hote = morceaux.hostname or ""
    return {
        "href": url,
        "protocol": (morceaux.scheme + ":") if morceaux.scheme else "",
        "hostname": hote,
        "port": port,
        "host": hote + (":" + port if port else ""),
        "origin": mod_origine.Origine.de_url(url).serialise(),
        "pathname": morceaux.path or "/",
        "search": ("?" + morceaux.query) if morceaux.query else "",
        "hash": ("#" + morceaux.fragment) if morceaux.fragment else "",
    }
