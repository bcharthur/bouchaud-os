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
"""

import errno
import os
import resource
import signal
import socket
import time

from . import protocole, securite, surface as mod_surface

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
                 limite_as=None, journal=None):
        self.journal = journal or (lambda niveau, texte: None)
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
        self._demarre(self.limite_as)

    # --- Naissance et mort ----------------------------------------------------

    def _demarre(self, limite_as):
        parent, enfant = socket.socketpair(socket.AF_UNIX, socket.SOCK_STREAM)
        self.surface = mod_surface.Surface.alloue(self.largeur, self.hauteur)

        pid = os.fork()
        if pid == 0:
            # --- Cote enfant ------------------------------------------------
            #
            # Rien de ce qui appartient au parent ne doit survivre ici : ni sa
            # prise, ni son descripteur de surface. L'enfant recevra le sien par
            # `SCM_RIGHTS`, ce qui parait redondant apres un `fork` — mais ne
            # l'est pas : c'est le chemin qui servira le jour ou le renderer
            # sera lance par `execve`, et l'eprouver maintenant coute zero.
            code = 3
            try:
                parent.close()
                self.surface.ferme()
                self.surface = None
                if limite_as:
                    resource.setrlimit(resource.RLIMIT_AS,
                                       (limite_as, limite_as))
                # Le renderer reste `Normale` : c'est l'interface qui passe
                # devant, et un renderer qui se declarerait interactif
                # annulerait tout le benefice mesure. On le pose explicitement
                # plutot que de compter sur l'heritage — un navigateur qui se
                # serait deja declare interactif transmettrait sa classe a son
                # enfant, et les deux se retrouveraient a egalite.
                try:
                    os.setpriority(os.PRIO_PROCESS, 0, 0)
                except (OSError, AttributeError):
                    pass
                from . import renderer as mod_renderer
                code = mod_renderer.sers(enfant)
            except BaseException:  # noqa: BLE001 — on ne remonte jamais d'ici
                code = 4
            finally:
                try:
                    enfant.close()
                except OSError:
                    pass
                # `_exit` et non `exit` : l'enfant ne doit rejouer ni les
                # `atexit` du parent, ni vidanger ses tampons — il ecrirait
                # deux fois ce que le parent a deja ecrit une.
                os._exit(code)

        # --- Cote parent ---------------------------------------------------
        enfant.close()
        self.pid = pid
        self.prise = parent
        self.canal = protocole.Canal(parent)
        self.prise.settimeout(0.0)

        # La surface part par `SCM_RIGHTS`, avec ses dimensions dans le message
        # de controle : les deux dans le meme envoi, sinon il faudrait les
        # reapparier a l'arrivee.
        self.canal.envoie_avec_descripteur(
            protocole.SURFACE,
            {"largeur": self.largeur, "hauteur": self.hauteur},
            self.surface.descripteur)
        self.dis(protocole.CREATE_DOCUMENT,
                 {"contexte": self.contexte, "largeur": self.largeur,
                  "hauteur": self.hauteur})

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
        if self.pid is None:
            return None
        if self.mort is None:
            try:
                self.dis(protocole.CLOSE, {"contexte": self.contexte})
            except (OSError, protocole.Fin):
                pass
            limite = time.monotonic() + delai
            while time.monotonic() < limite:
                if not self.vivant():
                    break
                time.sleep(0.01)
            if self.vivant():
                # Un renderer qui n'obeit pas au `CLOSE` est exactement le cas
                # que la separation existe pour traiter : on le tue, et la
                # fenetre ne s'en apercoit pas.
                self.journal("warn", "renderer %d : arret force" % self.pid)
                try:
                    os.kill(self.pid, signal.SIGKILL)
                except OSError:
                    pass
                try:
                    os.waitpid(self.pid, 0)
                except ChildProcessError:
                    pass
                self._note_mort(-1, int(signal.SIGKILL))
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
        if self.prise is None or self.canal is None:
            raise protocole.Fin()
        try:
            self.canal.envoie(genre, charge)
        except BrokenPipeError:
            self.vivant()
            raise protocole.Fin()
        except OSError as e:
            if e.errno in (errno.EPIPE, errno.ECONNRESET):
                self.vivant()
                raise protocole.Fin()
            raise

    def navigue(self, url):
        self.dis(protocole.NAVIGATE, {"contexte": self.contexte,
                                      "url": str(url)})

    def redimensionne(self, largeur, hauteur):
        """Redimensionne la vue. La **surface reste celle du navigateur**.

        Reallouer ici plutot que de laisser le renderer le faire n'est pas une
        commodite : c'est ce qui borne sa consommation. Il ne peut pas demander
        mille surfaces, il peut seulement peindre dans celle qu'on lui donne.
        """
        self.largeur, self.hauteur = int(largeur), int(hauteur)
        ancienne, self.surface = self.surface, mod_surface.Surface.alloue(
            self.largeur, self.hauteur)
        self.canal.envoie_avec_descripteur(
            protocole.SURFACE,
            {"largeur": self.largeur, "hauteur": self.hauteur},
            self.surface.descripteur)
        if ancienne is not None:
            ancienne.ferme()
        self.dis(protocole.RESIZE, {"contexte": self.contexte,
                                    "largeur": self.largeur,
                                    "hauteur": self.hauteur})

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
        self.dis(protocole.TICK, {"horodatage": time.monotonic()})

    # --- Ecoute ---------------------------------------------------------------

    def recolte(self, secondes=0.0):
        """Lit ce que le renderer a dit, sans jamais bloquer indefiniment.

        Rend la liste des `(nom, charge)` accumules. Un renderer mort ajoute son
        `CRASH` a cette meme liste : du point de vue de l'appelant, une mort est
        un evenement comme un autre — ce qui est exactement ce qu'il faut pour
        que le chrome n'ait pas deux chemins de code.
        """
        limite = time.monotonic() + max(0.0, secondes)
        while True:
            if self.prise is None:
                break
            try:
                self.prise.settimeout(0.0)
                genre, charge = self.canal.lis(protocole.VERS_NAVIGATEUR)
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
                # Un renderer qui parle mal est un renderer dont on ne sait plus
                # rien. On le tue plutot que de continuer a le croire.
                self.journal("error", "renderer : %s" % e)
                self.evenements.append(("PROTOCOLE", {"detail": str(e)}))
                self.tue()
                break
            except OSError:
                self.vivant()
                break
            self._note(genre, charge or {})
        self.vivant()
        sortie, self.evenements = self.evenements, []
        return sortie

    def _note(self, genre, charge):
        nom = protocole.NOMS.get(genre, str(genre))
        if genre == protocole.TITLE_CHANGED:
            self.titre = charge.get("titre")
        elif genre == protocole.URL_CHANGED:
            self.url = charge.get("url")
        elif genre == protocole.CURSOR_CHANGED:
            self.curseur = charge.get("forme", "fleche")
        elif genre == protocole.FRAME_READY:
            self.derniere_trame = charge
        elif genre == protocole.REQUEST_NAVIGATION:
            # **La politique est ici.** Le renderer demande ; le navigateur
            # verifie le schema avant d'appliquer. C'est ce qui empeche une page
            # compromise d'obtenir par la porte de derriere ce que la politique
            # lui refuse par la porte de devant.
            url = str(charge.get("url", ""))
            try:
                securite.verifie(securite.requete_document(
                    url, charge.get("provenance") or self.url,
                    destination="document"))
            except securite.Refus as refus:
                self.journal("warn", "navigation refusee : %s" % refus.raison)
                nom = "NAVIGATION_REFUSEE"
            else:
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
