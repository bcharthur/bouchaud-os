"""Le cote navigateur : celui qui tient la fenetre et survit a tout.

## Ce que ce module possede

La surface, la prise, le processus enfant, et la politique. Tout ce qui, s'il
disparaissait, ferait disparaitre la fenetre avec lui. Le renderer, lui, ne
possede que ce qu'on peut perdre.

## Le modele de creation : `fork`, sans `execve`

Un `fork` sans `exec` — le « zygote » de Chromium. L'enfant herite de
l'interprete deja demarre, des modules deja importes, des polices deja lues :
la creation coute quelques millisecondes au lieu du demarrage complet d'un
Python. C'est aussi ce qui rend le mecanisme portable sous Bouchaud OS, ou le
navigateur est un unique ELF statique et ou il n'y a pas de second binaire a
lancer.

L'isolation est entiere malgre l'absence d'`exec` : `fork` donne un espace
d'adressage separe. Un renderer qui deborde, qui plante ou qu'on tue n'emporte
rien d'autre que lui.

## Les trois isolations, et ce qui les etablit

* **crash** — l'enfant meurt, `wait4` le recolte, le navigateur fabrique
  lui-meme l'evenement. Attendre un message d'un processus mort serait attendre
  pour toujours ; c'est pourquoi le `CRASH` est **synthetise ici** et n'existe
  pas dans le protocole ;
* **memoire** — `RLIMIT_AS` est pose dans l'enfant, apres le `fork` et avant
  qu'il ne serve. Un renderer qui alloue sans fin echoue dans son propre bac a
  sable ;
* **processeur** — le navigateur se declare `Interactive`, l'enfant reste
  `Normale`. C'est la mesure de `ordonnanceur-probe` qui dit ce que cela vaut :
  sous Bouchaud OS, huit millisecondes de pire retard ramenees a une.

## Ce que le navigateur ne delegue jamais

Naviguer. Le renderer **demande** (`REQUEST_NAVIGATION`) ; c'est ici qu'on
decide. Un renderer compromis ne doit gagner que ce que sa page pouvait deja
faire, et une page ne peut pas se rendre elle-meme sur `file:///etc/shadow`.

Charger, non plus. Le renderer emet `FETCH_REQUEST` ; c'est ici que la politique
s'applique, que les temoins sont lus, et que la prise s'ouvre. Voir
`transport.py` pour ce que ce deplacement change, et pour ce qu'il ne change pas
encore.

## Ce dont l'enfant n'herite pas

Un `fork` copie la table des descripteurs : sans precaution, le renderer
naitrait avec les prises TCP de la reserve du navigateur, sa base de temoins,
son affichage et les surfaces des autres onglets. `privileges.ferme_herites`
les ferme un par un dans l'enfant, avant qu'il ne serve. `FD_CLOEXEC` ne peut
rien ici : il n'y a pas d'`exec`.
"""

import base64
import contextlib
import errno
import os
import resource
import signal
import socket
import threading
import time
from collections import deque

from . import (privileges, protocole, securite, surface as mod_surface,
               transport as mod_transport)

# Espace d'adressage qu'un renderer a le droit d'ajouter a celui dont il herite.
#
# **Un budget, pas un plafond**, et la distinction a coute une epreuve. `fork`
# donne a l'enfant l'espace d'adressage du parent : un `RLIMIT_AS` absolu se
# mesure donc contre ce que le parent occupait deja au moment du `fork`. Un
# navigateur qui a beaucoup travaille avant de creer son renderer lui donne un
# enfant qui nait au ras de son plafond — et dont le premier `mmap` echoue, sans
# que rien dans le code du renderer ne soit en cause.
#
# C'est arrive ici de la facon la plus nette : apres l'epreuve qui cree et
# detruit mille Workers, l'espace d'adressage du processus d'essai passait de
# 0,4 a 2,02 Gio — des arenes et des piles de fils que la libc garde reservees,
# virtuelles et non residentes. Le renderer forke juste apres heritait de ces
# 2,02 Gio et se voyait poser un plafond de 2. Toutes ses cartes echouaient.
#
# La lecon vaut au-dela de l'epreuve : **un zygote se forke tot**, avant que le
# parent n'ait rien accumule. Exprimer la limite en budget rend le mecanisme
# correct meme quand cette regle n'est pas tenue.
BUDGET_AS = 1 << 31          # 2 Gio de plus que ce dont il herite

# Ancien nom, conserve pour les appelants qui posaient un plafond absolu.
LIMITE_AS = BUDGET_AS


def taille_adressage():
    """L'espace d'adressage deja reserve par ce processus, en octets.

    Rend `0` quand la mesure n'est pas disponible : l'appelant pose alors son
    budget tel quel, ce qui est le comportement d'avant et reste sur.
    """
    try:
        with open("/proc/self/statm") as fichier:
            pages = int(fichier.read().split()[0])
        return pages * os.sysconf("SC_PAGE_SIZE")
    except (OSError, ValueError, IndexError, AttributeError):
        return 0

# Au-dela, le renderer est considere mort meme s'il respire encore.
ATTENTE_ARRET_S = 3.0

# Borne d'un envoi bloquant vers le renderer. Voir `_envoi_bloquant`.
ATTENTE_ENVOI_S = 30.0

# DNS/TCP/TLS hors du fil Qt. Quatre suffit sous QEMU.
FETCH_WORKERS = 4


class Crash(Exception):
    """Le renderer est mort. Porte de quoi le dire a l'utilisateur."""

    def __init__(self, code, signal_recu, raison):
        super().__init__(raison)
        self.code = code
        self.signal = signal_recu
        self.raison = raison


class Renderer:
    """Un processus de rendu, vu du navigateur."""

    def __init__(self, largeur=800, hauteur=600, budget_as=BUDGET_AS,
                 limite_as=None, journal=None, courtage=True, audit=False,
                 ferme_herites=True):
        self.journal = journal or (lambda niveau, texte: None)
        # Les capacites du renderer, decidees **ici** et posees dans l'enfant.
        # Un renderer ne se les accorde pas : il nait avec celles qu'on lui a
        # donnees, et il ne peut pas en demander d'autres.
        self.courtage = bool(courtage)
        self.audit = bool(audit)
        self.ferme_herites = bool(ferme_herites)
        self.descripteurs_fermes = 0
        self.requetes_servies = 0
        # Quand un chrome tient l'historique, il applique lui-meme les
        # navigations autorisees. Voir `_note`.
        self.auto_navigation = True
        self.foyer = False
        # La limite effective : ce dont l'enfant heritera, plus son budget. Un
        # appelant peut imposer la valeur absolue s'il sait ce qu'il fait —
        # c'est ce que font les epreuves d'isolation.
        self.limite_as = (int(limite_as) if limite_as is not None
                          else taille_adressage() + int(budget_as))
        self.largeur = int(largeur)
        self.hauteur = int(hauteur)
        self.contexte = 1
        self.pid = None
        self.prise = None
        self.canal = None
        self.surface = None
        self.mort = None            # `Crash` une fois recolte
        self.evenements = []
        self.titre = None
        self.url = None
        self.curseur = "fleche"
        self.derniere_trame = None
        self._fetch_lock = threading.Lock()
        self._fetch_attente = deque()
        self._fetch_terminees = deque()
        self._fetch_actifs = 0
        self._fetch_ferme = False
        self._demarre(self.limite_as)

    # --- Naissance et mort ----------------------------------------------------

    def _demarre(self, limite_as):
            parent, enfant = socket.socketpair(
                socket.AF_UNIX,
                socket.SOCK_STREAM
            )

            self.surface = mod_surface.Surface.alloue(
                self.largeur,
                self.hauteur
            )

            pid = os.fork()

            if pid == 0:
                # --- Cote enfant ---------------------------------------------
                code = 3

                try:
                    parent.close()

                    self.surface.ferme()
                    self.surface = None

                    if self.ferme_herites and privileges.balayage_actif():
                        privileges.ferme_herites(
                            set(privileges.GARDES_PAR_DEFAUT)
                            | {enfant.fileno()}
                        )

                    if limite_as:
                        resource.setrlimit(
                            resource.RLIMIT_AS,
                            (limite_as, limite_as)
                        )

                    try:
                        os.setpriority(
                            os.PRIO_PROCESS,
                            0,
                            0
                        )
                    except (OSError, AttributeError):
                        pass

                    from . import renderer as mod_renderer

                    code = mod_renderer.sers(
                        enfant,
                        courtage=self.courtage,
                        audit=self.audit
                    )

                except BaseException:
                    code = 4

                finally:
                    try:
                        enfant.close()
                    except OSError:
                        pass

                    os._exit(code)

            # --- Cote parent -------------------------------------------------
            enfant.close()

            self.pid = pid
            self.prise = parent
            self.canal = protocole.Canal(parent)

            # Bootstrap volontairement bloquant :
            # SURFACE transporte un fd SCM_RIGHTS.
            self.prise.settimeout(ATTENTE_ENVOI_S)

            self.canal.envoie_avec_descripteur(
                protocole.SURFACE,
                {
                    "largeur": self.largeur,
                    "hauteur": self.hauteur,
                },
                self.surface.descripteur,
            )

            self.canal.envoie(
                protocole.CREATE_DOCUMENT,
                {
                    "contexte": self.contexte,
                    "largeur": self.largeur,
                    "hauteur": self.hauteur,
                },
            )

            # Fonctionnement normal non bloquant.
            self.prise.settimeout(0.0)

    def vivant(self):
        """Le renderer respire-t-il ? Recolte au passage s'il vient de mourir."""
        if self.pid is None:
            return False
        if self.mort is not None:
            return False
        try:
            recolte, statut = os.waitpid(self.pid, os.WNOHANG)
        except ChildProcessError:
            self._note_mort(-1, 0)
            return False
        if recolte == 0:
            return True
        self._note_mort(os.WEXITSTATUS(statut) if os.WIFEXITED(statut) else -1,
                        os.WTERMSIG(statut) if os.WIFSIGNALED(statut) else 0)
        return False

    def _note_mort(self, code, signal_recu):
        if self.mort is not None:
            return
        if signal_recu:
            raison = "tue par le signal %d (%s)" % (
                signal_recu, signal.Signals(signal_recu).name
                if signal_recu in signal.Signals.__members__.values() else "?")
        elif code == 0:
            raison = "termine normalement"
        else:
            raison = "sorti avec le code %d" % code
        self.mort = Crash(code, signal_recu, raison)
        # Le `CRASH` n'est pas un message du protocole : un processus mort ne
        # parle pas. Il est fabrique ici, a partir de ce que `wait4` a rendu.
        self.evenements.append(("CRASH", {"contexte": self.contexte,
                                          "code": code, "signal": signal_recu,
                                          "raison": raison}))
        self.journal("warn", "renderer %d : %s" % (self.pid, raison))

    def ferme(self, delai=ATTENTE_ARRET_S):
        """Demande l'arret, puis insiste. Rend le code de sortie."""

        with self._fetch_lock:
            self._fetch_ferme = True
            self._fetch_attente.clear()

        if self.pid is None:
            return None

        if self.mort is None:
            try:
                self.dis(
                    protocole.CLOSE,
                    {"contexte": self.contexte},
                )
            except (OSError, protocole.Fin):
                pass

            limite = time.monotonic() + delai

            while time.monotonic() < limite:
                if not self.vivant():
                    break

                # CLOSE a pu etre mis en file pendant une saturation.
                # On continue donc a le pousser pendant l'attente.
                if self.canal is not None:
                    try:
                        self.canal.vidange()
                    except (
                        OSError,
                        protocole.Fin,
                    ):
                        pass

                time.sleep(0.01)

            if self.vivant():
                # Un renderer qui n'obeit pas au CLOSE est exactement le cas
                # que la separation existe pour traiter : on le tue, et la
                # fenetre ne s'en apercoit pas.
                self.journal(
                    "warn",
                    "renderer %d : arret force" % self.pid,
                )

                try:
                    os.kill(
                        self.pid,
                        signal.SIGKILL,
                    )
                except OSError:
                    pass

                try:
                    os.waitpid(
                        self.pid,
                        0,
                    )
                except ChildProcessError:
                    pass

                self._note_mort(
                    -1,
                    int(signal.SIGKILL),
                )

        if self.prise is not None:
            try:
                self.prise.close()
            except OSError:
                pass

            self.prise = None
            self.canal = None

        if self.surface is not None:
            self.surface.ferme()
            self.surface = None

        return None if self.mort is None else self.mort.code

    def tue(self, signal_envoye=signal.SIGKILL):
        """Tue le renderer sans ceremonie. Sert aux epreuves d'isolation."""
        if self.pid is None or self.mort is not None:
            return False
        try:
            os.kill(self.pid, signal_envoye)
        except OSError:
            return False
        limite = time.monotonic() + ATTENTE_ARRET_S
        while time.monotonic() < limite and self.vivant():
            time.sleep(0.005)
        return not self.vivant()

    # --- Parole ---------------------------------------------------------------

    def dis(self, genre, charge=None):
        """Place un message dans le canal vers le renderer.

        Sur la prise non bloquante, EAGAIN n'est plus une exception :
        `Canal.envoie()` conserve les octets restants et rend False.

        Retour :
            True  -> la file a ete entierement videe
            False -> le message est correctement mis en file mais attend
        """

        if self.prise is None or self.canal is None:
            raise protocole.Fin()

        try:
            return self.canal.envoie(
                genre,
                charge,
            )

        except BrokenPipeError:
            self.vivant()
            raise protocole.Fin()

        except OSError as e:
            if e.errno in (
                errno.EPIPE,
                errno.ECONNRESET,
            ):
                self.vivant()
                raise protocole.Fin()

            # EAGAIN ne devrait normalement plus sortir de Canal.envoie(),
            # puisqu'il est absorbe par la file de sortie.
            #
            # On garde toutefois cette ceinture de securite pour une libc
            # ou une implementation de socket qui le remonterait autrement.
            if e.errno in (
                errno.EAGAIN,
                errno.EWOULDBLOCK,
            ):
                return False

            raise

    def navigue(self, url):
        self.dis(protocole.NAVIGATE, {"contexte": self.contexte,
                                      "url": str(url)})

    def redimensionne(self, largeur, hauteur):
        """Redimensionne la vue et transmet la nouvelle surface.

        SURFACE transporte un fd SCM_RIGHTS.

        Contrairement aux messages ordinaires, cette operation est envoyee
        dans une courte section bloquante bornee, afin que le descripteur et
        sa trame restent atomiquement associes.
        """

        self.largeur = int(largeur)
        self.hauteur = int(hauteur)

        ancienne = self.surface

        nouvelle = mod_surface.Surface.alloue(
            self.largeur,
            self.hauteur,
        )

        self.surface = nouvelle

        try:
            with self._envoi_bloquant():
                # Conserver l'ordre :
                # tout ce qui precedait SURFACE doit partir avant le fd.
                if self.canal is not None:
                    self.canal.vidange()

                    self.canal.envoie_avec_descripteur(
                        protocole.SURFACE,
                        {
                            "largeur": self.largeur,
                            "hauteur": self.hauteur,
                        },
                        nouvelle.descripteur,
                    )

                    self.canal.envoie(
                        protocole.RESIZE,
                        {
                            "contexte": self.contexte,
                            "largeur": self.largeur,
                            "hauteur": self.hauteur,
                        },
                    )

                    # Socket bloquante dans ce contexte :
                    # on exige que RESIZE soit parti avant de rendre la main.
                    self.canal.vidange()

        except Exception:
            # Si la transmission echoue, ne garder aucune nouvelle surface
            # que le renderer ne connait pas.
            nouvelle.ferme()
            self.surface = ancienne
            raise

        if ancienne is not None:
            ancienne.ferme()

    def souris(self, x, y):
        self.dis(protocole.INPUT_EVENT, {"genre": "souris", "x": x, "y": y})

    def clic(self, x, y):
        self.dis(protocole.INPUT_EVENT, {"genre": "clic", "x": x, "y": y})

    def touche(self, touche, texte="", maj=False, ctrl=False):
        self.dis(protocole.INPUT_EVENT,
                 {"genre": "touche", "touche": touche, "texte": texte,
                  "maj": bool(maj), "ctrl": bool(ctrl)})

    def frappe(self, texte):
        for lettre in texte:
            self.touche(lettre, lettre)

    def defile(self, position):
        self.dis(protocole.INPUT_EVENT,
                 {"genre": "defilement", "position": position})

    def bat(self):
        """Fait avancer le renderer sans accumuler les battements.

        Un TICK est un signal de cadence, pas une commande qui doit etre
        rejouee cent fois.

        Si le renderer est tellement en retard que des octets attendent deja
        dans le canal, le prochain TICK apportera l'heure actuelle.

        On peut donc fusionner les battements sans perdre d'etat observable.
        """

        if self.prise is None or self.canal is None:
            raise protocole.Fin()

        try:
            # Le renderer a peut-etre libere de la place depuis le passage
            # precedent.
            self.canal.vidange()

        except BrokenPipeError:
            self.vivant()
            raise protocole.Fin()

        except OSError as e:
            if e.errno in (
                errno.EPIPE,
                errno.ECONNRESET,
            ):
                self.vivant()
                raise protocole.Fin()

            if e.errno not in (
                errno.EAGAIN,
                errno.EWOULDBLOCK,
            ):
                raise

        # Quelque chose d'important attend deja :
        #
        # NAVIGATE / INPUT / RESIZE / ancien TICK / etc.
        #
        # Ajouter encore TICK ne ferait qu'accroitre le retard.
        if self.canal.a_envoyer():
            return False

        return self.dis(
            protocole.TICK,
            {
                "horodatage": time.monotonic(),
            },
        )

    def demande_audit(self, quoi="descripteurs", **details):
        """Demande au renderer ce qu'il possede. Ne repond que s'il a la capacite."""
        charge = {"quoi": quoi}
        charge.update(details)
        self.dis(protocole.AUDIT, charge)

    # --- Courtage des ressources ----------------------------------------------

    @contextlib.contextmanager
    def _envoi_bloquant(self, delai=ATTENTE_ENVOI_S):
        """Rend la prise bloquante le temps d'un gros envoi, puis la rend.

        Le delai borne le cas ou le pair meurt en cours d'envoi : sans lui, le
        navigateur attendrait pour toujours qu'un processus mort le lise, ce qui
        est exactement le gel que la separation existe pour eviter.
        """
        if self.prise is None:
            yield
            return
        self.prise.settimeout(delai)
        try:
            yield
        finally:
            try:
                self.prise.settimeout(0.0)
            except OSError:
                pass

    def _charge_requete(self, charge):
        """Execute la partie potentiellement lente d'un FETCH, sans toucher Canal."""
        identifiant = charge.get("id")
        url = str(charge.get("url", ""))
        corps = charge.get("corps")
        if corps is not None:
            try:
                corps = base64.b64decode(corps)
            except (ValueError, TypeError):
                corps = None
        from . import reseau
        debut = time.monotonic()
        try:
            reponse = reseau.charge_localement(
                url, methode=str(charge.get("methode") or "GET"), corps=corps,
                entetes=charge.get("entetes") or None,
                brut=bool(charge.get("brut")),
                document=charge.get("document"),
                destination=charge.get("destination"))
        except Exception as e:  # noqa: BLE001
            reponse = reseau.Reponse(url, "", "text/plain", 0, erreur=str(e))
        duree_ms = int((time.monotonic() - debut) * 1000)
        return identifiant, url, reponse, duree_ms

    def _envoie_reponse_fetch(self, identifiant, reponse):
        """Reinjecte un resultat deja charge ; seul le fil UI touche au canal."""
        meta = mod_transport.reponse_vers(reponse)
        meta["id"] = identifiant
        octets = bytes(reponse.octets or b"")
        if meta.pop("texte_propre", False):
            meta["texte_propre"] = True
            meta["contenu"] = reponse.contenu
        meta["fin"] = not octets
        with self._envoi_bloquant():
            self.dis(protocole.FETCH_RESPONSE, meta)
            pas = protocole.MORCEAU_FETCH
            for debut in range(0, len(octets), pas):
                morceau = octets[debut:debut + pas]
                self.dis(protocole.FETCH_DATA,
                         {"id": identifiant,
                          "b64": base64.b64encode(morceau).decode("ascii"),
                          "fin": debut + pas >= len(octets)})
        self.requetes_servies += 1

    def _sert_requete(self, charge):
        identifiant, url, reponse, duree_ms = self._charge_requete(charge)
        self._envoie_reponse_fetch(identifiant, reponse)
        self.journal("debug", "fetch #%s local %s (%d ms)" %
                     (identifiant, url, duree_ms))

    def _travaille_requete(self, charge):
        resultat = self._charge_requete(charge)
        with self._fetch_lock:
            if not self._fetch_ferme:
                self._fetch_terminees.append(resultat)
            self._fetch_actifs = max(0, self._fetch_actifs - 1)
        self._lance_requetes()

    def _lance_requetes(self):
        while True:
            with self._fetch_lock:
                if (self._fetch_ferme or self._fetch_actifs >= FETCH_WORKERS
                        or not self._fetch_attente):
                    return
                charge = self._fetch_attente.popleft()
                self._fetch_actifs += 1
            identifiant = charge.get("id")
            try:
                fil = threading.Thread(
                    target=self._travaille_requete,
                    args=(charge,), name="bo-fetch-%s" % identifiant,
                    daemon=True)
                fil.start()
            except Exception as e:  # noqa: BLE001
                with self._fetch_lock:
                    self._fetch_actifs = max(0, self._fetch_actifs - 1)
                self.journal("warn", "worker fetch indisponible : %s" % e)
                resultat = self._charge_requete(charge)
                with self._fetch_lock:
                    if not self._fetch_ferme:
                        self._fetch_terminees.append(resultat)

    def _programme_requete(self, charge):
        url = str(charge.get("url", ""))
        if url.startswith(("bo:", "file://")):
            self._sert_requete(charge)
            return
        with self._fetch_lock:
            if self._fetch_ferme:
                return
            self._fetch_attente.append(dict(charge))
        self._lance_requetes()

    def _vidange_requetes_terminees(self, maximum=1):
        for _ in range(max(0, int(maximum))):
            with self._fetch_lock:
                if self._fetch_ferme or not self._fetch_terminees:
                    return
                identifiant, url, reponse, duree_ms = self._fetch_terminees.popleft()
            try:
                self._envoie_reponse_fetch(identifiant, reponse)
            except (OSError, protocole.Fin):
                return
            self.journal("debug", "fetch #%s termine %s (%d ms)" %
                         (identifiant, url, duree_ms))

    # --- Ecoute ---------------------------------------------------------------

    def recolte(self, secondes=0.0):
        """Lit les messages du renderer sans bloquer indefiniment.

        Avant chaque lecture, on profite du fait que le renderer ait peut-etre
        avance pour tenter de vider la file navigateur -> renderer.

        Ainsi les deux directions progressent ensemble et aucune ne peut etre
        abandonnee simplement parce que l'autre applique de la contre-pression.
        """

        limite = time.monotonic() + max(
            0.0,
            secondes,
        )

        self._vidange_requetes_terminees(maximum=1)

        while True:
            if self.prise is None or self.canal is None:
                break

            # Essayer d'abord de faire progresser ce qui attend vers le
            # renderer.
            try:
                self.canal.vidange()

            except BrokenPipeError:
                self.vivant()
                break

            except protocole.Fin:
                self.vivant()
                break

            except OSError as e:
                if e.errno not in (
                    errno.EAGAIN,
                    errno.EWOULDBLOCK,
                ):
                    self.vivant()
                    break

            # Puis lire ce qui vient du renderer.
            try:
                self.prise.settimeout(0.0)

                genre, charge = self.canal.lis(
                    protocole.VERS_NAVIGATEUR
                )

            except (BlockingIOError, socket.timeout):
                if time.monotonic() >= limite:
                    break

                if not self.vivant():
                    break

                time.sleep(0.002)
                continue

            except protocole.Fin:
                self.vivant()
                break

            except protocole.Erreur as e:
                # Un flux devenu invalide n'est plus fiable.
                self.journal(
                    "error",
                    "renderer : %s" % e,
                )

                self.evenements.append(
                    (
                        "PROTOCOLE",
                        {
                            "detail": str(e),
                        },
                    )
                )

                self.tue()
                break

            except OSError:
                self.vivant()
                break

            self._note(
                genre,
                charge or {},
            )

        self._vidange_requetes_terminees(maximum=1)
        self.vivant()

        sortie = self.evenements
        self.evenements = []

        return sortie

    def _note(self, genre, charge):
        nom = protocole.NOMS.get(genre, str(genre))
        if genre == protocole.FETCH_REQUEST:
            # La politique reste dans le navigateur, mais le reseau ne bloque
            # plus le fil Qt.
            self._programme_requete(charge)
            return
        if genre == protocole.TITLE_CHANGED:
            self.titre = charge.get("titre")
        elif genre == protocole.URL_CHANGED:
            self.url = charge.get("url")
        elif genre == protocole.CURSOR_CHANGED:
            self.curseur = charge.get("forme", "fleche")
        elif genre == protocole.FRAME_READY:
            self.derniere_trame = charge
        elif genre == protocole.FOCUS_CHANGED:
            self.foyer = bool(charge.get("foyer"))
        elif genre == protocole.REQUEST_NAVIGATION:
            # **La politique est ici.** Le renderer demande ; le navigateur
            # verifie le schema avant d'appliquer. C'est ce qui empeche une page
            # compromise d'obtenir par la porte de derriere ce que la politique
            # lui refuse par la porte de devant.
            if charge.get("pas") is not None:
                # `history.go(-1)` : le renderer n'a pas l'historique, il ne
                # peut donc que le demander. Seul le chrome sait ou cela mene.
                nom = "NAVIGATION_HISTORIQUE"
                self.evenements.append((nom, charge))
                return
            url = str(charge.get("url", ""))
            try:
                securite.verifie(securite.requete_document(
                    url, charge.get("provenance") or self.url,
                    destination="document"))
            except securite.Refus as refus:
                self.journal("warn", "navigation refusee : %s" % refus.raison)
                nom = "NAVIGATION_REFUSEE"
            else:
                nom = "NAVIGATION_AUTORISEE"
                # Le chrome tient l'historique ; quand il en tient un, c'est lui
                # qui applique, parce qu'une navigation appliquee ici serait une
                # page de plus a l'ecran et une entree de moins dans son
                # historique. Sans chrome — les epreuves du protocole — on
                # applique soi-meme, sinon le mecanisme ne serait pas eprouvable
                # tout seul.
                if self.auto_navigation:
                    self.navigue(url)
        self.evenements.append((nom, charge))

    def attends(self, nom, secondes=15.0):
        """Bat jusqu'a voir passer un evenement, ou renonce.

        Rend `(vu, evenements)`. Le battement est ici et pas dans le renderer :
        c'est le navigateur qui cadence, et une epreuve qui n'aurait pas la main
        sur le rythme ne serait pas reproductible.
        """
        vus = []
        limite = time.monotonic() + secondes
        while time.monotonic() < limite:
            if not self.vivant():
                vus.extend(self.recolte())
                return False, vus
            try:
                self.bat()
            except protocole.Fin:
                vus.extend(self.recolte())
                return False, vus
            lot = self.recolte(0.02)
            vus.extend(lot)
            if any(entree[0] == nom for entree in lot):
                return True, vus
        return False, vus

    # --- Pixels ---------------------------------------------------------------

    def trame(self):
        """La derniere trame publiee, ou `None` si le renderer n'a rien peint."""
        if self.surface is None or self.derniere_trame is None:
            return None
        return self.surface.lis(self.derniere_trame.get("tampon"))

    def pixels_non_vides(self):
        """Combien de pixels ne sont pas entierement noirs et transparents.

        C'est la question la plus simple qui distingue « le renderer a peint »
        de « le renderer a publie une surface vide », et elle ne depend d'aucune
        police ni d'aucune couleur.
        """
        trame = self.trame()
        if not trame:
            return 0
        return sum(1 for i in range(0, len(trame), 4)
                   if trame[i] or trame[i + 1] or trame[i + 2] or trame[i + 3])
