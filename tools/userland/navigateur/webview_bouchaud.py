"""Backend Bouchaud OS de pywebview.

Ce fichier est installe par `build-navigateur.sh` sous
`webview/platforms/bouchaud.py` dans la copie embarquee de pywebview. Il permet
au code d'un tutoriel pywebview de tourner **sans modification** sur Bouchaud
OS :

    import webview
    webview.create_window('Bonjour', 'https://example.com')
    webview.start()

pywebview est une bibliotheque a moteurs enfichables : `webview/platforms/`
contient un module par systeme d'affichage (Qt/QtWebEngine, GTK/WebKit, Cocoa,
EdgeChromium...), et `guilib.initialize()` choisit celui qui se charge. Bouchaud
OS en est un de plus, et le sien rend les pages avec le moteur natif de l'OS
(`moteur/`) sur la toile Qt de l'hote (`hote.cpp`).

## Ce que ce moteur n'a pas

Le moteur natif ne fait pas tourner JavaScript. `evaluate_js` et le pont
`window.pywebview.api` — qui reposent l'un et l'autre sur l'execution de code
dans la page — levent donc une exception explicite plutot que de renvoyer
silencieusement `None`. Tout le reste de l'interface pywebview fonctionne :
creation de fenetre, chargement d'URL et de HTML, navigation, titre, taille,
plein ecran, evenements de cycle de vie, boites de dialogue.

## Un seul fil pour Qt

pywebview execute la fonction passee a `webview.start()` dans un fil separe, et
cette fonction appelle `load_url`, `set_title`... Or Qt n'accepte d'etre touche
que depuis le fil qui tient sa boucle d'evenements : un `QFontMetrics` construit
ailleurs suffit a faire tomber le programme. Tout ce qui touche a l'affichage
passe donc par [`_sur_le_fil_principal`], qui met le travail en file et attend
son execution par le battement de l'hote.

## Une seule fenetre

L'affichage de l'OS est un framebuffer sans gestionnaire de fenetres : une
surface, plein ecran. Le backend accepte donc les fenetres suivantes mais les
affiche l'une apres l'autre dans la meme surface, la derniere creee devant. Les
API restent celles de pywebview ; c'est la contrainte materielle qui differe.
"""

from __future__ import annotations

import collections
import logging
import os
import sys
import threading

import bo

from moteur import Document, reseau

logger = logging.getLogger('pywebview')

# Nom rapporte par `webview.renderer` et par l'evenement `initialized`.
renderer = 'bouchaud'

# --- Etat du backend ---------------------------------------------------------

# uid -> Fenetre. pywebview identifie ses fenetres par une chaine.
_fenetres: dict[str, 'Fenetre'] = {}
_ordre: list[str] = []
_pret = threading.Event()

# File de travail a executer sur le fil principal, et identite de celui-ci.
_travaux: collections.deque = collections.deque()
_fil_principal: int | None = None


def _sur_le_fil_principal(fonction, *arguments, attendre=True):
    """Execute `fonction` sur le fil de la boucle Qt.

    Appelee depuis ce fil, elle appelle directement. Appelee d'ailleurs, elle
    met en file : c'est le battement de l'hote (`tic`) qui depile. Sans ce
    detour, un `load_url` lance depuis la fonction de `webview.start()`
    construirait des objets Qt sur le mauvais fil.

    `attendre=False` rend la main sans attendre l'execution. C'est ce qu'il faut
    pour un chargement : l'API de pywebview est asynchrone — `load_url` revient
    tout de suite et l'appelant guette `events.loaded`. Attendre ici bloquerait
    le fil appelant pendant tout le telechargement, verrou global compris, donc
    empecherait le fil principal de faire le travail qu'on vient de lui confier.
    """
    if _fil_principal is None or threading.get_ident() == _fil_principal:
        return fonction(*arguments)

    fait = threading.Event()
    boite: dict = {}

    def enveloppe():
        try:
            boite['valeur'] = fonction(*arguments)
        except BaseException as e:  # noqa: BLE001 — reportee a l'appelant
            boite['erreur'] = e
        finally:
            fait.set()

    # Aucun appel a Qt ici : `QWidget::update()` depuis un autre fil n'est pas
    # sur. Le battement de l'hote passe toutes les 16 ms, c'est assez.
    _travaux.append(enveloppe)
    if not attendre:
        return None
    if os.environ.get('BO_TRACE'):
        print('[trace] mis en file depuis le fil', threading.get_ident(),
              '(principal =', _fil_principal, ')', flush=True)
    if not fait.wait(60):
        raise TimeoutError('le fil principal n\'a pas traite la demande')
    if 'erreur' in boite:
        raise boite['erreur']
    return boite.get('valeur')


_tics = 0


def _tic():
    """Battement de l'hote : depile le travail destine au fil principal."""
    global _tics
    _tics += 1
    if os.environ.get('BO_TRACE') and _tics % 60 == 1:
        print('[trace] battement', _tics, '— file :', len(_travaux), flush=True)
    while _travaux:
        try:
            _travaux.popleft()()
        except Exception:
            logger.exception('travail de fil principal')

# Codes de touches Qt (Qt::Key_*) et modificateurs.
K_ECHAP = 0x01000000
K_HAUT, K_BAS = 0x01000013, 0x01000015
K_PAGE_HAUT, K_PAGE_BAS = 0x01000016, 0x01000017
K_DEBUT, K_FIN = 0x01000010, 0x01000011
K_F5 = 0x01000034
K_Q, K_W = 0x51, 0x57
MOD_CTRL = 0x04000000


class JavaScriptIndisponible(RuntimeError):
    """Levee par `evaluate_js` : le moteur natif n'execute pas JavaScript."""


class Fenetre:
    """Une fenetre pywebview, rendue par le moteur natif."""

    def __init__(self, modele):
        self.modele = modele
        self.uid = modele.uid
        self.titre = modele.title or 'pywebview'
        self.document: Document | None = None
        self.defilement = 0.0
        self.url = ''
        self.etat = ''
        self.survol = None
        self.largeur, self.hauteur = bo.taille()

    # --- Chargement ---------------------------------------------------------

    def charge_url(self, url):
        # Toujours appelee sur le fil principal (voir `_sur_le_fil_principal`).
        # On ne relance pas la file d'evenements de Qt ici : ce chargement peut
        # lui-meme venir d'un battement, et imbriquer les boucles d'evenements
        # est le meilleur moyen de se retrouver a en attendre une qui attend
        # l'autre.
        url = reseau.normalise(url, self.url or None)
        self.etat = 'Chargement de %s…' % url
        self._pose(Document(reseau.charge(url), self.largeur))

    def charge_html(self, contenu, base=''):
        # `load_html` recoit le document tel quel : pas de reseau, pas d'URL.
        self._pose(Document(reseau.Reponse(base or 'about:blank', contenu),
                            self.largeur))

    def _pose(self, document):
        self.document = document
        self.url = document.url
        self.defilement = 0.0
        self.titre = document.titre or self.titre
        self.etat = '' if not document.erreur else 'Echec : %s' % document.erreur
        bo.titre(self.titre)
        # pywebview attend cet evenement apres chaque chargement ; c'est lui que
        # guettent les programmes qui veulent agir une fois la page prete.
        self.modele.events.loaded.set()
        bo.redessiner()

    # --- Peinture -----------------------------------------------------------

    def peint(self, largeur, hauteur):
        if largeur != self.largeur and self.document:
            self.document.remet_en_page(largeur)
        self.largeur, self.hauteur = largeur, hauteur

        liste = [('rect', 0, 0, largeur, hauteur, 0xFFFFFFFF)]
        hauteur_vue = max(1, hauteur - 20)
        if self.document:
            liste.append(('clip', 0, 0, largeur, hauteur_vue))
            liste.extend(self.document.liste_affichage(self.defilement, largeur,
                                                       hauteur_vue))
            liste.append(('declip',))

        # Une barre d'etat discrete : sans chrome, c'est le seul endroit qui
        # rappelle l'adresse courante et signale un lien survole.
        liste.append(('rect', 0, hauteur - 20, largeur, 20, 0xFF10182B))
        message = self.survol or self.etat or self.url
        liste.append(('texte', 8, hauteur - 17, message[:200], 0xFF9FB3D0, 11,
                      False, False, False, False))
        return liste

    def defile(self, pixels):
        if not self.document:
            return
        maximum = max(0.0, self.document.hauteur - self.hauteur + 40)
        self.defilement = min(maximum, max(0.0, self.defilement + pixels))


# --- Rappels de l'hote -------------------------------------------------------

def _active() -> Fenetre | None:
    return _fenetres.get(_ordre[-1]) if _ordre else None


def _peindre(largeur, hauteur):
    fenetre = _active()
    if fenetre is None:
        return [('rect', 0, 0, largeur, hauteur, 0xFFFFFFFF)]
    try:
        return fenetre.peint(largeur, hauteur)
    except Exception:
        logger.exception('peinture')
        return [('rect', 0, 0, largeur, hauteur, 0xFFFFFFFF),
                ('texte', 20, 20, 'Erreur de peinture — voir la console',
                 0xFFB3261E, 16, True, False, False, False)]


def _touche(code, texte, modificateurs=0):
    fenetre = _active()
    if fenetre is None:
        return
    ctrl = bool(modificateurs & MOD_CTRL)
    if ctrl and code in (K_Q, K_W):
        destroy_window(fenetre.uid)
    elif code == K_BAS:
        fenetre.defile(60)
    elif code == K_HAUT:
        fenetre.defile(-60)
    elif code == K_PAGE_BAS:
        fenetre.defile(fenetre.hauteur * 0.9)
    elif code == K_PAGE_HAUT:
        fenetre.defile(-fenetre.hauteur * 0.9)
    elif code == K_DEBUT:
        fenetre.defilement = 0.0
    elif code == K_FIN:
        fenetre.defile(1e9)
    elif code == K_F5 and fenetre.url:
        fenetre.charge_url(fenetre.url)


def _clic(x, y):
    fenetre = _active()
    if fenetre is None or fenetre.document is None:
        return
    lien = fenetre.document.lien_a(x, y + fenetre.defilement)
    if lien:
        try:
            fenetre.charge_url(lien)
        except Exception:
            logger.exception('navigation')


def _survol(x, y):
    fenetre = _active()
    if fenetre is None or fenetre.document is None:
        return
    lien = fenetre.document.lien_a(x, y + fenetre.defilement)
    nouveau = ('→ ' + lien) if lien else None
    if nouveau != fenetre.survol:
        fenetre.survol = nouveau
        bo.redessiner()


def _molette(cran):
    fenetre = _active()
    if fenetre is not None:
        fenetre.defile(-cran / 120.0 * 90.0)


def _fermeture():
    for uid in list(_ordre):
        fenetre = _fenetres.get(uid)
        if fenetre:
            fenetre.modele.events.closed.set()


# --- Interface attendue par pywebview ----------------------------------------

def setup_app():
    """Appelee avant toute creation de fenetre. Qt est deja demarre par l'hote."""
    global _fil_principal
    _fil_principal = threading.get_ident()
    bo.enregistrer({
        'peindre': _peindre,
        'touche': _touche,
        'clic': _clic,
        'survol': _survol,
        'molette': _molette,
        'fermeture': _fermeture,
        'tic': _tic,
    })


def create_window(window):
    """Cree une fenetre. Pour la premiere, entre dans la boucle d'evenements.

    C'est le contrat de pywebview : `create_window` de la fenetre maitresse ne
    rend la main qu'a la fin du programme. Les fenetres suivantes sont creees
    depuis un autre fil et doivent revenir tout de suite.
    """
    fenetre = Fenetre(window)
    _fenetres[window.uid] = fenetre
    _ordre.append(window.uid)

    if window.uid == 'master':
        setup_app()
        bo.ouvrir(fenetre.titre)
        _charge_initial(fenetre, window)
        window.events.shown.set()
        _pret.set()
        bo.boucle()
    else:
        _pret.wait(10)
        _sur_le_fil_principal(_charge_initial, fenetre, window)
        window.events.shown.set()
        bo.redessiner()


def _charge_initial(fenetre, window):
    try:
        if getattr(window, 'html', None):
            fenetre.charge_html(window.html)
        elif window.real_url:
            fenetre.charge_url(window.real_url)
        else:
            fenetre.charge_html(
                '<body style="font-family:sans-serif;padding:40px">'
                '<h1>pywebview sur Bouchaud OS</h1>'
                '<p>Fenetre sans contenu initial.</p></body>')
    except Exception:
        logger.exception('chargement initial')
        fenetre.etat = 'Echec du chargement initial'


def set_title(title, uid):
    fenetre = _fenetres.get(uid)
    if fenetre:
        fenetre.titre = title
        _sur_le_fil_principal(bo.titre, title, attendre=False)


def get_current_url(uid):
    fenetre = _fenetres.get(uid)
    return fenetre.url if fenetre else None


def load_url(url, uid):
    fenetre = _fenetres.get(uid)
    if fenetre:
        _sur_le_fil_principal(fenetre.charge_url, url, attendre=False)


def load_html(content, base_uri, uid):
    fenetre = _fenetres.get(uid)
    if fenetre:
        _sur_le_fil_principal(fenetre.charge_html, content, base_uri, attendre=False)


def destroy_window(uid):
    fenetre = _fenetres.pop(uid, None)
    if uid in _ordre:
        _ordre.remove(uid)
    if fenetre:
        fenetre.modele.events.closed.set()
    if not _fenetres:
        bo.quitter()
    else:
        bo.redessiner()


def get_active_window():
    fenetre = _active()
    return fenetre.modele if fenetre else None


def evaluate_js(script, uid, parse_json=True):
    raise JavaScriptIndisponible(
        "le moteur natif de Bouchaud OS n'execute pas JavaScript : "
        "evaluate_js et window.pywebview.api ne sont pas disponibles")


def get_position(uid):
    return 0, 0


def get_size(uid):
    fenetre = _fenetres.get(uid)
    return (fenetre.largeur, fenetre.hauteur) if fenetre else bo.taille()


def get_title(uid):
    fenetre = _fenetres.get(uid)
    return fenetre.titre if fenetre else ''


def get_screens():
    from webview.screen import Screen
    largeur, hauteur = bo.taille()
    return [Screen(0, 0, largeur, hauteur)]


# L'affichage est un framebuffer plein ecran, sans gestionnaire de fenetres :
# ces operations n'ont pas d'equivalent materiel. Elles reussissent sans rien
# faire plutot que d'echouer, pour ne pas casser un programme qui les appelle
# par habitude.
def hide(uid): pass
def show(uid): bo.redessiner()
def maximize(uid): pass
def minimize(uid): pass
def restore(uid): pass
def toggle_fullscreen(uid): pass
def set_on_top(uid, top): pass
def resize(width, height, uid, fix_point): pass
def move(x, y, uid): pass
def add_tls_cert(certfile): pass
def clear_cookies(uid): pass
def get_cookies(uid): return []


def create_confirmation_dialog(title, message, uid):
    """Sans clavier modal, la confirmation est affichee et acceptee."""
    print('[pywebview] %s : %s' % (title, message))
    return True


def create_file_dialog(dialog_type, directory, allow_multiple, save_filename,
                       file_types, uid):
    """Pas de selecteur de fichiers : on rend le repertoire propose."""
    if dialog_type == 20:  # SAVE_DIALOG
        return os.path.join(directory or '/', save_filename or 'sans-nom')
    return None


def create_menu(app_menu_list, menubar=None):
    pass
