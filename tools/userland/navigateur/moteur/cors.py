"""CORS : ce qu'un script a le droit de **lire**, pas ce qu'il a le droit de demander.

## La confusion qu'il faut eviter

Le raccourci naturel — « si c'est cross-origine, on refuse » — est faux, et le
faire donnerait un navigateur qui casse la moitie du Web. Une page a toujours eu
le droit d'**envoyer** une requete a une autre origine : c'est ainsi que se
chargent les images d'un CDN, les polices, les scripts d'analyse, et c'est ainsi
que fonctionne un formulaire qui poste ailleurs. Ce que la politique controle
est autre chose : **ce que le script peut lire de la reponse**.

D'ou la distinction que ce module maintient partout :

* la **requete** part — sauf si un preflight l'a refusee ;
* la **reponse** n'est exposee au script que si le serveur y consent, par
  `Access-Control-Allow-Origin`.

Sans cette distinction, on aurait soit un navigateur qui refuse tout — donc
inutilisable —, soit un navigateur qui laisse un script lire n'importe quel site
auquel l'utilisateur est connecte, ce qui reviendrait a supprimer la SOP.

## Les trois cas

1. **Meme origine** — rien a verifier. La reponse est lisible.
2. **Cross-origine simple** — la requete part telle quelle, et l'on regarde
   `Access-Control-Allow-Origin` au retour. `*` autorise tout le monde ; une
   origine exacte n'autorise qu'elle. Avec `credentials`, `*` ne suffit plus :
   le serveur doit nommer l'origine et poser
   `Access-Control-Allow-Credentials: true`, sans quoi un site pourrait lire
   les donnees d'un autre en se servant des temoins de l'utilisateur.
3. **Cross-origine non simple** — une methode autre que `GET`/`HEAD`/`POST`, ou
   un en-tete que la liste sure ne contient pas : un `OPTIONS` part d'abord et
   demande l'autorisation. Si elle est refusee, **la vraie requete ne part
   pas** — c'est le seul cas ou CORS empeche l'envoi, et c'est voulu : une
   requete non simple peut modifier l'etat du serveur.

## Ce qui n'est pas fait

`Access-Control-Expose-Headers` n'est pas encore lu : la liste des en-tetes
lisibles est celle des en-tetes surs, sans extension. Pas de cache de preflight
(`Access-Control-Max-Age`), donc un `OPTIONS` par requete non simple. Ni l'un ni
l'autre ne change ce qui est autorise — seulement ce que cela coute.
"""

from . import origine as mod_origine
from . import telemetrie

# Les methodes qui ne declenchent pas de preflight.
METHODES_SIMPLES = frozenset(("GET", "HEAD", "POST"))

# Les en-tetes qu'une page peut poser sans demander la permission. La liste est
# celle de Fetch, et elle est courte pour une raison : chacun de ces en-tetes
# peut deja etre emis par un formulaire HTML ordinaire, donc les autoriser
# n'ouvre rien de nouveau.
ENTETES_SIMPLES = frozenset((
    "accept", "accept-language", "content-language", "content-type",
    "range", "dpr", "downlink", "save-data", "viewport-width", "width",
))

# Les valeurs de `Content-Type` qu'un formulaire sait deja produire.
TYPES_SIMPLES = ("application/x-www-form-urlencoded", "multipart/form-data",
                 "text/plain")

# Les en-tetes de reponse qu'un script peut lire sans autorisation explicite.
ENTETES_EXPOSES = frozenset((
    "cache-control", "content-language", "content-length", "content-type",
    "expires", "last-modified", "pragma",
))


class Refus(Exception):
    """La reponse existe, mais le script n'a pas le droit de la lire."""

    def __init__(self, raison):
        super().__init__(raison)
        self.raison = raison


def _entete(entetes, nom):
    """Lit un en-tete sans se soucier de la casse."""
    if not entetes:
        return None
    cible = nom.lower()
    for cle, valeur in entetes.items():
        if str(cle).lower() == cible:
            return valeur
    return None


def type_simple(valeur):
    if not valeur:
        return True
    base = str(valeur).split(";")[0].strip().lower()
    return base in TYPES_SIMPLES


def requete_simple(methode, entetes):
    """Cette requete part-elle sans demander la permission d'abord ?"""
    if (methode or "GET").upper() not in METHODES_SIMPLES:
        return False
    for nom, valeur in (entetes or {}).items():
        minuscule = str(nom).lower()
        if minuscule not in ENTETES_SIMPLES:
            return False
        if minuscule == "content-type" and not type_simple(valeur):
            return False
    return True


def entetes_non_simples(entetes):
    """Ceux qu'il faudra faire autoriser par le preflight."""
    return sorted(nom.lower() for nom in (entetes or {})
                  if nom.lower() not in ENTETES_SIMPLES)


def preflight_requis(origine_document, url, methode, entetes):
    """Faut-il un `OPTIONS` avant d'envoyer ?"""
    if mod_origine.Origine.de_url(url).meme_origine(origine_document):
        return False
    return not requete_simple(methode, entetes)


def entetes_preflight(origine_document, methode, entetes):
    """Les en-tetes du `OPTIONS` qui demande l'autorisation."""
    demandes = {
        "Origin": origine_document.serialise(),
        "Access-Control-Request-Method": (methode or "GET").upper(),
    }
    liste = entetes_non_simples(entetes)
    if liste:
        demandes["Access-Control-Request-Headers"] = ", ".join(liste)
    return demandes


def preflight_accorde(origine_document, methode, entetes, reponse_entetes,
                      code=200):
    """Le serveur autorise-t-il la vraie requete ?"""
    if code and (code < 200 or code >= 400):
        return False
    permise = _entete(reponse_entetes, "access-control-allow-origin")
    if permise is None:
        return False
    permise = permise.strip()
    if permise != "*" and not mod_origine.Origine.de_url(permise).meme_origine(
            origine_document):
        return False

    methodes = _entete(reponse_entetes, "access-control-allow-methods") or ""
    voulue = (methode or "GET").upper()
    autorisees = {m.strip().upper() for m in methodes.split(",") if m.strip()}
    if voulue not in METHODES_SIMPLES and "*" not in autorisees \
            and voulue not in autorisees:
        return False

    accordes = _entete(reponse_entetes, "access-control-allow-headers") or ""
    permis = {h.strip().lower() for h in accordes.split(",") if h.strip()}
    for nom in entetes_non_simples(entetes):
        if "*" not in permis and nom not in permis:
            return False
    return True


def reponse_lisible(origine_document, url_reponse, entetes, avec_temoins=False):
    """Le script peut-il lire cette reponse ? Rend `(oui, raison)`.

    C'est **la** decision de CORS. Elle se prend au retour, sur la reponse, et
    non au depart sur la requete — confondre les deux est l'erreur qui casse un
    navigateur ou qui l'ouvre.
    """
    cible = mod_origine.Origine.de_url(url_reponse)
    if cible.meme_origine(origine_document):
        return True, ""

    permise = _entete(entetes, "access-control-allow-origin")
    if permise is None:
        return False, ("aucun Access-Control-Allow-Origin : la reponse de %s "
                       "n'est pas exposee a %s"
                       % (cible.serialise(), origine_document.serialise()))
    permise = permise.strip()

    if permise == "*":
        if avec_temoins:
            # Avec des temoins, `*` ne suffit pas : le serveur doit nommer
            # l'origine. Sans cette regle, un site pourrait lire les donnees
            # privees d'un autre en se servant de la session de l'utilisateur —
            # c'est exactement l'attaque que CORS existe pour empecher.
            return False, ("Access-Control-Allow-Origin: * ne vaut pas pour une "
                           "requete avec temoins")
        return True, ""

    if not mod_origine.Origine.de_url(permise).meme_origine(origine_document):
        return False, ("Access-Control-Allow-Origin: %s ne correspond pas a %s"
                       % (permise, origine_document.serialise()))

    if avec_temoins:
        aveu = (_entete(entetes, "access-control-allow-credentials") or "").strip()
        if aveu.lower() != "true":
            return False, ("Access-Control-Allow-Credentials manquant pour une "
                           "requete avec temoins")
    return True, ""


def filtre_entetes(entetes, origine_document, url_reponse):
    """Ne laisse au script que les en-tetes qu'il a le droit de lire.

    Meme quand la reponse est lisible, tous ses en-tetes ne le sont pas : un
    `Set-Cookie` ou un en-tete applicatif d'un autre site n'ont pas a etre
    exposes. En meme origine, tout est lisible.
    """
    if mod_origine.Origine.de_url(url_reponse).meme_origine(origine_document):
        return dict(entetes or {})
    exposes = {}
    for cle, valeur in (entetes or {}).items():
        if str(cle).lower() in ENTETES_EXPOSES:
            exposes[cle] = valeur
    telemetrie.compte("cors.entetes_filtres",
                      max(0, len(entetes or {}) - len(exposes)))
    return exposes


def reponse_opaque(url):
    """Ce que voit un script dont la reponse ne lui est pas exposee.

    `status: 0`, corps vide, aucun en-tete : c'est ce que dit Fetch, et c'est
    ce qui distingue « refuse » de « erreur reseau » du point de vue du code —
    a savoir, rien. C'est voulu : reveler la difference apprendrait a un script
    si une ressource existe sur un site tiers.
    """
    return {"status": 0, "statusText": "", "text": "", "url": url,
            "headers": {}, "type": "opaque"}
