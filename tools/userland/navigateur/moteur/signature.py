"""Dechiffrer les signatures de YouTube, en executant leur propre code.

Quand YouTube rend une adresse signee, elle porte un parametre `s` qu'il faut
transformer avant de s'en servir. La transformation est definie dans le script
du lecteur (`base.js`), sous une forme volontairement changeante : le nom de la
fonction, l'ordre de ses etapes et le nom de l'objet auxiliaire varient a chaque
version.

La reimplementer serait un travail sans fin — elle change plusieurs fois par
mois. La seule approche tenable est celle qu'emploient tous les extracteurs :
**retrouver la fonction dans `base.js` et l'executer**. C'est exactement ce que
le moteur JavaScript du navigateur permet depuis qu'il porte QuickJS.

## Ce qu'on execute, et dans quoi

Pas `base.js` en entier : c'est un megaoctet de code qui touche au DOM, au
reseau et a mille choses absentes ici. On en extrait les quelques lignes utiles
— la fonction et l'objet des trois operations elementaires (inverser, echanger,
tronquer) — et on les evalue dans un contexte QuickJS **neuf et vide**, sans
`document`, sans `fetch`, sans rien. Le bac a sable est donc total : ce code ne
peut qu'appliquer des permutations a une chaine de caracteres.

Un budget de temps court l'accompagne : une fonction qui bouclerait est
interrompue au lieu de figer le navigateur.
"""

import re

import bojs

from . import reseau

# Budget d'execution : la transformation est une poignee de permutations, elle
# ne prend pas dix millisecondes. Au-dela, c'est que quelque chose ne va pas.
BUDGET_MS = 2000

# `base.js` change souvent, mais rarement en meme temps que sa version : on
# garde les fonctions extraites par adresse de script.
_CACHE = {}


def adresse_du_lecteur(page_ou_reponse):
    """Trouve l'adresse de `base.js`, dans la page de lecture ou une reponse.

    Deux formes circulent : le chemin complet (`/s/player/…/base.js`) et
    l'identifiant seul dans `jsUrl`. Les deux sont acceptees.
    """
    if not page_ou_reponse:
        return None
    texte = page_ou_reponse
    if isinstance(texte, bytes):
        texte = texte.decode("utf-8", "replace")

    for motif in (r'"jsUrl"\s*:\s*"([^"]+base\.js)"',
                  r'"(/s/player/[A-Za-z0-9_/.-]+/base\.js)"',
                  r'src="([^"]+/base\.js)"'):
        trouve = re.search(motif, texte)
        if trouve:
            chemin = trouve.group(1).replace("\\/", "/")
            if chemin.startswith("//"):
                return "https:" + chemin
            if chemin.startswith("/"):
                return "https://www.youtube.com" + chemin
            return chemin
    return None


class Dechiffreur:
    """La fonction de transformation d'une version donnee du lecteur."""

    def __init__(self, source_base_js, journal=None):
        self.journal = journal or (lambda niveau, texte: None)
        self.source_signature = extrait_fonction_signature(source_base_js)
        self.source_n = extrait_fonction_n(source_base_js)
        self._contexte = None

    def utilisable(self):
        return self.source_signature is not None

    def _prepare(self):
        if self._contexte is not None:
            return True
        if not self.source_signature:
            return False
        # Contexte neuf et vide : ni DOM, ni reseau, ni minuteries. Le prelude
        # du navigateur n'y est pas charge, et c'est voulu.
        self._contexte = bojs.cree(BUDGET_MS)
        bojs.pont(self._contexte, lambda *arguments: None)
        try:
            bojs.evalue(self._contexte, self.source_signature, "base.js:signature")
            if self.source_n:
                bojs.evalue(self._contexte, self.source_n, "base.js:n")
        except bojs.Erreur as e:
            self.journal("warn", "youtube: fonction de signature illisible : %s" % e)
            bojs.detruit(self._contexte)
            self._contexte = None
            return False
        return True

    def signature(self, chaine):
        """Applique la transformation a `s`. Rend `None` en cas d'echec."""
        if not self._prepare():
            return None
        try:
            resultat = bojs.appelle(self._contexte, "__bo_signature", [chaine])
        except bojs.Erreur as e:
            self.journal("warn", "youtube: dechiffrement echoue : %s" % e)
            return None
        return resultat if isinstance(resultat, str) else None

    def parametre_n(self, chaine):
        """Applique la transformation du parametre `n` (limitation de debit).

        Sans elle, YouTube sert le flux a debit reduit — la lecture demarre puis
        s'essouffle. Son absence n'empeche pas de lire, elle ralentit.
        """
        if not self.source_n or not self._prepare():
            return None
        try:
            resultat = bojs.appelle(self._contexte, "__bo_n", [chaine])
        except bojs.Erreur:
            return None
        return resultat if isinstance(resultat, str) else None

    def ferme(self):
        if self._contexte is not None:
            bojs.detruit(self._contexte)
            self._contexte = None


# --- Extraction depuis base.js ------------------------------------------------

# Le nom de la fonction de dechiffrement, tel qu'il apparait a l'endroit ou le
# lecteur s'en sert. Plusieurs formes ont cours selon les versions ; on les
# essaie toutes, de la plus recente a la plus ancienne.
_MOTIFS_NOM = (
    r'\b([a-zA-Z0-9$_]+)\s*=\s*function\(\s*a\s*\)\s*\{\s*a\s*=\s*a\.split\(\s*""\s*\)',
    r'(?:\b|[^a-zA-Z0-9$])([a-zA-Z0-9$]{2,})\s*=\s*function\(\s*a\s*\)\s*\{\s*a\s*=\s*a\.split\(\s*""\s*\)',
    r'\bm=([a-zA-Z0-9$]{2,})\(decodeURIComponent\(h\.s\)\)',
    r'\bc&&\(c=([a-zA-Z0-9$]{2,})\(decodeURIComponent\(c\)\)',
    r'(?:\b|[^a-zA-Z0-9$])([a-zA-Z0-9$]{2,})\s*=\s*function\(\s*a\s*\)\s*\{\s*a\s*=\s*a\.split\(\s*""\s*\)\s*;\s*[a-zA-Z0-9$]{2}\.',
)


def extrait_fonction_signature(source):
    """Extrait la fonction de dechiffrement et son objet auxiliaire.

    Rend du JavaScript autonome qui definit `__bo_signature`, ou `None`.
    """
    if not source:
        return None

    nom = None
    for motif in _MOTIFS_NOM:
        trouve = re.search(motif, source)
        if trouve:
            nom = trouve.group(1)
            break
    if not nom:
        return None

    corps = _corps_de_fonction(source, nom)
    if corps is None:
        return None

    # La fonction appelle les methodes d'un objet auxiliaire, du genre
    # `Xy.ab(a,17)`. Il faut embarquer cet objet avec elle.
    auxiliaire = ""
    trouve = re.search(r";\s*([A-Za-z0-9$_]+)\s*\.\s*[A-Za-z0-9$_]+\s*\(", corps)
    if trouve:
        auxiliaire = _objet_auxiliaire(source, trouve.group(1)) or ""

    return "%s\nvar __bo_signature = %s;\n" % (auxiliaire, corps)


def extrait_fonction_n(source):
    """Extrait la fonction du parametre `n`, si on la retrouve."""
    if not source:
        return None
    for motif in (
        r'\b([a-zA-Z0-9$_]+)\s*=\s*function\(\s*a\s*\)\s*\{\s*var\s+b\s*=\s*a\.split\(',
        r'\.get\("n"\)\)&&\([a-zA-Z0-9$]+=([a-zA-Z0-9$]{2,})\(',
        r'\b([a-zA-Z0-9$_]+)\s*=\s*function\(\s*a\s*\)\s*\{\s*var\s+b\s*=\s*a\.split\(\s*""\s*\)',
    ):
        trouve = re.search(motif, source)
        if not trouve:
            continue
        corps = _corps_de_fonction(source, trouve.group(1))
        if corps:
            return "var __bo_n = %s;\n" % corps
    return None


def _corps_de_fonction(source, nom):
    """Rend le texte `function(a){…}` associe a ce nom.

    Le corps est delimite en comptant les accolades, pas par une expression
    rationnelle : il contient des accolades imbriquees et des chaines qui en
    contiennent aussi.
    """
    for motif in (
        r'\b%s\s*=\s*function\s*\(' % re.escape(nom),
        r'\bfunction\s+%s\s*\(' % re.escape(nom),
        r'\b%s\s*:\s*function\s*\(' % re.escape(nom),
    ):
        trouve = re.search(motif, source)
        if not trouve:
            continue
        debut_parenthese = source.index("(", trouve.start())
        fin_parenthese = _ferme(source, debut_parenthese, "(", ")")
        if fin_parenthese < 0:
            continue
        debut_accolade = source.find("{", fin_parenthese)
        if debut_accolade < 0:
            continue
        fin_accolade = _ferme(source, debut_accolade, "{", "}")
        if fin_accolade < 0:
            continue
        arguments = source[debut_parenthese:fin_parenthese + 1]
        return "function%s%s" % (arguments, source[debut_accolade:fin_accolade + 1])
    return None


def _objet_auxiliaire(source, nom):
    """Rend la declaration `var Xy = { … };` de l'objet des operations."""
    trouve = re.search(r'\bvar\s+%s\s*=\s*\{' % re.escape(nom), source)
    if not trouve:
        return None
    debut = source.index("{", trouve.start())
    fin = _ferme(source, debut, "{", "}")
    if fin < 0:
        return None
    return "var %s = %s;" % (nom, source[debut:fin + 1])


def _ferme(source, debut, ouvrant, fermant):
    """Position du delimiteur fermant correspondant, en ignorant les chaines."""
    niveau = 0
    dans_chaine = None
    echappe = False
    position = debut
    while position < len(source):
        caractere = source[position]
        if dans_chaine:
            if echappe:
                echappe = False
            elif caractere == "\\":
                echappe = True
            elif caractere == dans_chaine:
                dans_chaine = None
        elif caractere in "\"'":
            dans_chaine = caractere
        elif caractere == ouvrant:
            niveau += 1
        elif caractere == fermant:
            niveau -= 1
            if niveau == 0:
                return position
        position += 1
    return -1


# --- Application a une adresse ------------------------------------------------

def dechiffreur_pour(video, page=None, journal=None):
    """Recupere `base.js` et prepare le dechiffreur. Rend `None` si impossible."""
    journal = journal or (lambda niveau, texte: None)
    if page is None:
        reponse = reseau.charge(
            "https://www.youtube.com/watch?v=%s&hl=fr" % video, brut=True)
        if reponse.code != 200:
            return None
        page = reponse.octets

    adresse = adresse_du_lecteur(page)
    if not adresse:
        journal("warn", "youtube: adresse de base.js introuvable")
        return None
    if adresse in _CACHE:
        return _CACHE[adresse]

    reponse = reseau.charge(adresse, brut=True)
    if reponse.code != 200:
        journal("warn", "youtube: base.js indisponible (code %s)" % reponse.code)
        return None

    dechiffreur = Dechiffreur(reponse.octets.decode("utf-8", "replace"), journal)
    if not dechiffreur.utilisable():
        journal("warn", "youtube: fonction de signature introuvable dans base.js")
        return None
    _CACHE[adresse] = dechiffreur
    return dechiffreur


def resout(url, signee, dechiffreur):
    """Applique la signature a une adresse. Rend l'adresse utilisable, ou `None`."""
    if not signee:
        return url
    if dechiffreur is None:
        return None
    valeur = dechiffreur.signature(signee.get("s", ""))
    if not valeur:
        return None
    separateur = "&" if "?" in url else "?"
    return "%s%s%s=%s" % (url, separateur, signee.get("sp", "signature"),
                          urllib_quote(valeur))


def applique_n(url, dechiffreur):
    """Remplace le parametre `n` par sa forme transformee, si on sait le faire."""
    if dechiffreur is None or "n=" not in url:
        return url
    morceaux = urllib_split(url)
    ancienne = morceaux.get("n")
    if not ancienne:
        return url
    nouvelle = dechiffreur.parametre_n(ancienne)
    if not nouvelle or nouvelle == ancienne:
        return url
    return url.replace("n=" + urllib_quote(ancienne), "n=" + urllib_quote(nouvelle))


def urllib_quote(valeur):
    import urllib.parse
    return urllib.parse.quote(valeur, safe="")


def urllib_split(url):
    import urllib.parse
    morceaux = urllib.parse.urlsplit(url)
    return {c: v[0] for c, v in urllib.parse.parse_qs(morceaux.query).items()}
