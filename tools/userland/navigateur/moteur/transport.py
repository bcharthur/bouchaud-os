"""Par ou une ressource arrive, et qui a le droit de la chercher.

## Le constat qui a rendu ce module necessaire

Le renderer tourne dans son propre processus, mais il chargeait ses ressources
lui-meme : `moteur.charge(url)` appelait `reseau.charge(url)`, qui resolvait un
nom, ouvrait une prise, presentait un certificat, lisait le magasin de temoins
et ecrivait dans le cache du disque. La feuille de style, la police, l'image, le
script et le prechargement passaient par le meme chemin.

Autrement dit : le processus qu'on isole pour qu'il ne puisse rien faire de mal
avait le reseau, les temoins et le stockage — pas parce qu'on le lui avait
accorde, mais parce que le code Python savait appeler les modules
correspondants. La frontiere existait pour les crashs et pas pour les
privileges.

## La forme de la solution

Un seul point de bascule, et deux implantations derriere :

    DirectTransport     le renderer va chercher lui-meme  (tests, mode debug)
    BrokeredTransport   le renderer demande, le navigateur applique et va chercher

`reseau.charge` consulte le transport actif. Rien d'autre du moteur ne change :
`Document`, la cascade, le prechargement, `fetch()` cote JavaScript continuent
d'appeler `reseau.charge` sans savoir lequel des deux repond. **Les pages ne
voient pas la difference**, et c'est la seule facon de pouvoir comparer les deux
modes sur les memes epreuves.

## Ce que le courtier deplace, concretement

Sous `BrokeredTransport`, le renderer n'a plus besoin de :

* resoudre un nom, ouvrir une prise, faire une poignee de main TLS ;
* lire ou ecrire le magasin de temoins ;
* lire ou ecrire le cache HTTP sur le disque ;
* ouvrir un `file://`.

Tout cela se passe dans le navigateur, qui applique **sa** politique avant
d'agir — `securite.verifie` sur chaque requete, avec l'origine du document qui
demande. Un renderer compromis qui fabrique une requete obtient exactement ce
que la politique accorde a sa page, ni plus.

## Ce que le courtier ne resout pas encore

Le navigateur sert la requete **synchroniquement**, dans son pompage
d'evenements. Une ressource lente gele donc le chrome le temps de son
aller-retour — exactement comme le faisait le chargement en-processus d'avant,
donc pas une regression, mais pas encore le benefice complet. Le rendre
asynchrone demande une file de requetes cote navigateur et une continuation
cote renderer ; c'est le goulot suivant, et il est nomme comme tel dans
`docs/BROWSER_ISOLATION.md`.

Le media (`reseau.tranche`, `reseau.taille_distante`) reste direct : il lit par
tranches de plusieurs mebioctets, en boucle, et le passer par des trames de
controle sans l'avoir d'abord rendu asynchrone transformerait une lecture video
en gel de l'interface. C'est ecrit ici pour que ce ne soit pas une surprise dans
un audit.
"""

import base64
import threading

# Le transport actif de **ce processus**. `None` veut dire « direct » : c'est
# le cas du navigateur lui-meme, et celui du moteur execute dans les epreuves.
#
# Une variable de module et pas un parametre : le seuil traverse une vingtaine
# d'appelants — cascade, images, polices, prechargement, `fetch` — et les faire
# tous porter un transport aurait donne vingt occasions d'en oublier un. Un
# appelant oublie serait reste direct **en silence**, ce qui est precisement le
# defaut que ce module existe pour rendre impossible.
_actuel = None
_verrou = threading.Lock()


def actuel():
    return _actuel


def installe(transport):
    """Pose le transport de ce processus. Rend le precedent."""
    global _actuel
    with _verrou:
        ancien, _actuel = _actuel, transport
    return ancien


class Direct:
    """Va chercher la ressource ici et maintenant.

    C'est le comportement d'origine, conserve tel quel : les epreuves du moteur
    et le mode `BO_BROWSER_INPROCESS=1` s'en servent, et c'est aussi ce que le
    **navigateur** utilise pour honorer une requete courtee.
    """

    nom = "direct"

    def charge(self, url, methode="GET", corps=None, entetes=None, brut=False,
               document=None, destination=None):
        from . import reseau
        return reseau.charge_localement(
            url, methode=methode, corps=corps, entetes=entetes, brut=brut,
            document=document, destination=destination)


class Courtier:
    """Demande la ressource au navigateur, et attend sa reponse.

    L'objet ne connait pas le protocole : il recoit une fonction `demande` qui
    sait envoyer la requete et rendre la reponse. C'est ce qui permet de
    l'eprouver sans forker quoi que ce soit — une fausse `demande` suffit — et
    ce qui garde `renderer.py` seul responsable du cadencement des trames.
    """

    nom = "courtier"

    def __init__(self, demande):
        self._demande = demande
        # Le protocole v1 utilise un seul lecteur sur le flux de controle.
        self._requete = threading.Lock()

    def charge(self, url, methode="GET", corps=None, entetes=None, brut=False,
               document=None, destination=None):
        from . import reseau
        requete = {
            "url": str(url), "methode": str(methode), "brut": bool(brut),
            "entetes": dict(entetes or {}),
            "document": document, "destination": destination,
        }
        if corps is not None:
            # Le corps d'un POST peut etre binaire : `json` ne le porterait pas.
            octets = corps if isinstance(corps, (bytes, bytearray)) \
                else str(corps).encode("utf-8")
            requete["corps"] = base64.b64encode(bytes(octets)).decode("ascii")
        # Le prechargement peut appeler charge() depuis plusieurs fils.
        # Une transaction complete a la fois evite qu'un fil lise la reponse
        # d'un autre. Le reseau, lui, est sorti du fil Qt cote navigateur.
        with self._requete:
            reponse = self._demande(requete)
        return reponse_depuis(reponse, url)


def reponse_vers(reponse):
    """Une `reseau.Reponse` en dictionnaire transportable, corps exclu.

    Le corps voyage a part, en morceaux : voir `protocole.MORCEAU_FETCH`.
    """
    return {
        "url": reponse.url, "type_mime": reponse.type_mime,
        "code": reponse.code, "erreur": reponse.erreur,
        "entetes": dict(reponse.entetes or {}),
        # `contenu` est le corps decode en texte, `octets` le corps brut. Les
        # deux ne sont pas redondants — une image n'a pas de `contenu` utile, et
        # une page d'erreur fabriquee n'a pas d'`octets`. On transporte les
        # octets, et le texte est reconstruit a l'arrivee **sauf** quand il ne
        # derive pas des octets, auquel cas il voyage aussi.
        "texte_propre": bool(reponse.contenu) and (
            reponse.contenu.encode("utf-8", "replace")
            != bytes(reponse.octets or b"")),
    }


def reponse_depuis(charge, url_demandee):
    """Refabrique une `reseau.Reponse` a partir de ce qui a traverse."""
    from . import reseau
    octets = charge.get("octets") or b""
    if isinstance(octets, str):
        octets = base64.b64decode(octets)
    contenu = charge.get("contenu")
    if contenu is None:
        contenu = bytes(octets).decode("utf-8", "replace")
    return reseau.Reponse(
        charge.get("url") or url_demandee, contenu,
        charge.get("type_mime") or "application/octet-stream",
        int(charge.get("code") or 0), charge.get("erreur"),
        entetes=charge.get("entetes") or {}, octets=bytes(octets))
