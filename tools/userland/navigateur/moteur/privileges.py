"""Ce qu'un renderer possede, et ce qu'on lui retire a la naissance.

## Le probleme que ce module existe pour resoudre

`superviseur.py` cree le renderer par `fork()` **sans `execve()`**. C'est ce qui
rend la creation rapide et portable — il n'y a pas de second binaire a lancer
sous Bouchaud OS. Mais un `fork` ne copie pas seulement la memoire : il copie
**la table des descripteurs**. Tout ce que le navigateur avait d'ouvert au
moment du `fork`, l'enfant l'a aussi, et il l'a pour de vrai — le meme fichier,
la meme prise TCP deja connectee, la meme base de temoins.

La liste n'est pas theorique. Au moment ou un navigateur cree un renderer, il
tient couramment :

* la connexion a l'affichage (X11, Wayland, ou le framebuffer sous Bouchaud OS) ;
* les peripheriques d'entree ;
* les prises TCP de sa reserve de connexions (`reseau._PRISES`) — deja ouvertes,
  deja authentifiees TLS pour certaines ;
* les fichiers du stockage persistant : temoins, `localStorage`, IndexedDB,
  cache HTTP ;
* les journaux ;
* les prises de controle des **autres** renderers ;
* les surfaces partagees des **autres** onglets.

Un renderer qui herite de tout cela n'est plus une frontiere de privileges. Il
reste une frontiere de crash — ce qui est deja beaucoup — mais une page qui
obtiendrait l'execution de code arbitraire dedans lirait les temoins de
n'importe quel site en ouvrant un descripteur qu'on lui a offert.

## Pourquoi pas `FD_CLOEXEC`

Parce que `FD_CLOEXEC` ne ferme rien a `fork` : il ferme a `exec`, et il n'y a
pas d'`exec`. C'est le piege exact de ce modele — le drapeau habituel donne
l'impression d'avoir traite la question, et ne fait rien du tout. Il n'y a qu'une
facon de fermer apres un `fork` sans `exec` : **enumerer et fermer**.

## La regle

L'enfant garde une liste blanche, pas une liste noire :

    0, 1, 2        la console serie — un renderer qu'on ne peut pas deboguer
                   n'est pas un progres, et c'est une concession explicite
    la prise de controle
    les descripteurs explicitement accordes

Tout le reste est ferme, par numero, sans passer par les objets Python qui les
detiennent. C'est volontaire : appeler `.close()` sur l'objet ferait tourner des
destructeurs qui, dans un enfant de `fork`, ecriraient dans des fichiers que le
parent croit posseder seul. Un `os.close(n)` ne touche que la table du
processus courant.

## Ce que ce module ne fait pas

Il ne remplace pas un bac a sable. Un renderer sans descripteur peut toujours
appeler `open()` : le systeme de fichiers reste atteignable par le nom. Fermer
les descripteurs herites supprime la fuite **passive** — celle qu'on offre sans
le vouloir. Refuser l'ouverture est un autre travail, et il demande un
mecanisme du noyau (`seccomp`, `landlock`, ou l'equivalent Bouchaud OS) qui
n'existe pas encore. `docs/RENDERER_PRIVILEGE_AUDIT.md` dit lequel des deux est
en place, capacite par capacite, sans arrondir.
"""

import os
import socket
import stat

# Ce que l'enfant garde, toujours. La console serie est une concession
# assumee : un renderer muet est un renderer qu'on ne sait pas deboguer sous
# emulation, et ce qu'elle donne a un attaquant — ecrire dans un journal — est
# tres au-dessous de ce que donnerait une prise TCP heritee.
GARDES_PAR_DEFAUT = (0, 1, 2)


def balayage_actif():
    """Faux si l'utilisateur a demande a garder les descripteurs herites.

    Une soupape, pas une option. Le renderer rasterise par `bo.rasterise`,
    c'est-a-dire par le Qt du navigateur : la fermeture emporte au passage la
    connexion a l'affichage, dont un `QPainter` sur une `QImage` n'a pas besoin —
    mais dont on ne peut pas jurer qu'aucune plateforme Qt n'aura besoin. Si un
    jour une plateforme le prouve, `BO_RENDERER_GARDE_FD=1` permet de le
    constater en une relance au lieu d'une bissection.
    """
    return os.environ.get("BO_RENDERER_GARDE_FD", "") in ("", "0")


def _cible(numero):
    """Ou pointe un descripteur : `/proc/self/fd/N`, ou un nom faute de mieux."""
    try:
        return os.readlink("/proc/self/fd/%d" % numero)
    except OSError:
        return "?"


def _genre(numero, cible):
    """Le genre d'un descripteur, du point de vue d'un audit.

    Le nom compte plus que le detail : ce qui interesse un audit, c'est
    « est-ce que l'enfant tient une prise reseau », pas le numero d'inode.
    """
    if cible.startswith("socket:"):
        # Une prise sans plus de precision ne dit pas grand-chose ; sa famille,
        # si. Une `AF_UNIX` est un canal interne, une `AF_INET` est une sortie
        # vers le monde.
        #
        # Le duplicata est **referme par `close()`**, pas par `detach()`. La
        # difference a coute l'epreuve : `detach()` rend le descripteur a
        # personne — l'objet le lache, le noyau le garde — et `fileno()` rend
        # ensuite `-1`, si bien que le `os.close` qui suivait ne fermait rien.
        # L'inventaire fuyait donc un descripteur par prise inspectee, et comme
        # c'est l'inventaire qui alimente le balayage, l'enfant se retrouvait
        # apres nettoyage avec un duplicata de chacune des prises qu'on venait
        # de lui fermer. Un audit qui cree ce qu'il mesure ne mesure rien.
        try:
            duplicata = os.dup(numero)
        except OSError:
            return "prise"
        prise = socket.socket(fileno=duplicata)
        try:
            famille = prise.family
        finally:
            prise.close()
        if famille == socket.AF_UNIX:
            return "prise_unix"
        if famille in (socket.AF_INET, socket.AF_INET6):
            return "prise_reseau"
        return "prise"
    if cible.startswith("pipe:"):
        return "tube"
    if cible.startswith("anon_inode:"):
        return "anonyme"
    if "memfd:" in cible:
        return "memfd"
    if cible.startswith("/dev/"):
        return "peripherique"
    if cible == "?":
        return "inconnu"
    try:
        mode = os.fstat(numero).st_mode
    except OSError:
        return "inconnu"
    if stat.S_ISDIR(mode):
        return "repertoire"
    if stat.S_ISCHR(mode) or stat.S_ISBLK(mode):
        return "peripherique"
    return "fichier"


def inventaire():
    """Tous les descripteurs ouverts de ce processus.

    Rend une liste de `{"fd", "genre", "cible"}`, triee. L'enumeration passe par
    `/proc/self/fd` plutot que par un balayage de 0 a `RLIMIT_NOFILE` : le
    balayage coute un million d'appels systeme quand la limite est haute, et il
    ne dit rien de ce que les descripteurs sont.
    """
    trouve = []
    try:
        noms = os.listdir("/proc/self/fd")
    except OSError:
        return trouve
    # `listdir` ouvre lui-meme un descripteur, qui apparait dans sa propre
    # liste et n'existe deja plus quand on le lit. On le laisse tomber
    # silencieusement plus bas, via le `fstat` qui echoue.
    for nom in noms:
        try:
            numero = int(nom)
        except ValueError:
            continue
        try:
            os.fstat(numero)
        except OSError:
            continue
        cible = _cible(numero)
        trouve.append({"fd": numero, "genre": _genre(numero, cible),
                       "cible": cible})
    trouve.sort(key=lambda entree: entree["fd"])
    return trouve


def resume(entrees):
    """Compte les descripteurs par genre. Ce qu'un test compare."""
    compte = {}
    for entree in entrees:
        compte[entree["genre"]] = compte.get(entree["genre"], 0) + 1
    return compte


def ferme_herites(gardes, journal=None):
    """Ferme tout ce qui n'est pas dans `gardes`. **A appeler dans l'enfant.**

    Rend la liste des numeros fermes. Les erreurs sont avalees une par une :
    un descripteur qui disparait entre l'enumeration et la fermeture n'est pas
    un probleme, c'est une course normale avec `listdir`.

    `os.close` et non `objet.close()` : voir l'en-tete du module. Fermer par
    l'objet ferait tourner un destructeur dans un enfant de `fork`, qui
    ecrirait — vidange de tampon, `COMMIT` sqlite — dans un fichier que le
    parent croit tenir seul.
    """
    gardes = set(int(n) for n in gardes)
    fermes = []
    for entree in inventaire():
        numero = entree["fd"]
        if numero in gardes:
            continue
        try:
            os.close(numero)
        except OSError:
            continue
        fermes.append(numero)
    if journal is not None and fermes:
        journal("info", "renderer : %d descripteur(s) herite(s) fermes"
                % len(fermes))
    return fermes


# --- La batterie adversariale -------------------------------------------------
#
# Ce qui suit est joue **dans le renderer**, et seulement quand le navigateur lui
# a accorde la capacite d'audit. Chaque tentative est ce que ferait une page qui
# aurait obtenu l'execution de code dans ce processus : elle essaie pour de vrai,
# et rapporte si elle a reussi.
#
# Le but n'est pas d'obtenir des « non » partout — plusieurs de ces tentatives
# reussissent aujourd'hui, et il faut que ce soit ecrit. Une frontiere dont on
# connait la forme exacte vaut mieux qu'une frontiere qu'on croit etanche.

def _tente(nom, action):
    """Joue une tentative et rend `{"capacite", "obtenu", "detail"}`."""
    try:
        detail = action()
    except BaseException as e:  # noqa: BLE001 — un refus est un resultat
        return {"capacite": nom, "obtenu": False,
                "detail": "%s: %s" % (type(e).__name__, e)}
    return {"capacite": nom, "obtenu": True, "detail": str(detail)[:200]}


def _lit_fichier_arbitraire(chemin):
    with open(chemin, "rb") as fichier:
        return "%d octets lus de %s" % (len(fichier.read(64)), chemin)


def _ouvre_prise_directe(hote, port):
    prise = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    prise.settimeout(2.0)
    try:
        prise.connect((hote, int(port)))
        return "connecte a %s:%s" % (hote, port)
    finally:
        prise.close()


def _lit_descripteurs_herites(accordes):
    """Cherche un descripteur utilisable en dehors de ceux qu'on a accordes.

    `accordes` porte la prise de controle et la surface partagee : ce sont des
    capacites **donnees**, pas des fuites, et les compter comme un butin
    rendrait la mesure inutilisable — elle echouerait toujours, donc elle ne
    dirait plus rien.
    """
    suspects = [entree for entree in inventaire()
                if entree["fd"] not in accordes
                and entree["genre"] != "memfd"]
    if not suspects:
        raise OSError("aucun descripteur herite exploitable")
    return ", ".join("%(fd)d=%(genre)s (%(cible)s)" % entree
                     for entree in suspects)


def _lit_stockage_direct():
    from . import stockage
    temoins = stockage.temoins()
    lus = temoins.pour("https://exemple.test/")
    return "magasin de temoins ouvert (%d entree(s) pour exemple.test)" % len(lus)


def tentatives(cible_reseau=None, fichier=None, accordes=()):
    """Tout ce qu'un renderer compromis essaierait. Rend la liste des verdicts.

    `cible_reseau` est un `(hote, port)` que l'epreuve sait joignable — sans lui,
    un echec de connexion ne prouverait rien d'autre que l'absence de serveur.
    `accordes` liste les descripteurs qu'on lui a **volontairement** donnes.
    """
    accordes = set(GARDES_PAR_DEFAUT) | set(int(n) for n in accordes)
    resultats = [
        _tente("fichier_arbitraire",
               lambda: _lit_fichier_arbitraire(fichier or "/etc/hostname")),
        _tente("descripteur_herite",
               lambda: _lit_descripteurs_herites(accordes)),
        _tente("stockage_direct", _lit_stockage_direct),
    ]
    if cible_reseau:
        hote, port = cible_reseau
        resultats.append(_tente("prise_reseau_directe",
                                lambda: _ouvre_prise_directe(hote, port)))
    return resultats
