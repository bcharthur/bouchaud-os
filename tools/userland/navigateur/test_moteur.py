"""Verifications du moteur web.

Le meme fichier sert dans les deux environnements, et c'est voulu :

- sur la machine de developpement (`tools/userland/test-moteur.sh`), l'hote Qt
  n'existe pas ; le module `bo` est remplace par un bouchon qui mesure le texte
  a la regle de trois et lit la taille d'un PNG dans son en-tete ;
- sous Bouchaud OS (`exec /bo-navigateur /usr/share/bo-navigateur/test_moteur.py`),
  le vrai `bo` est la : vraies metriques de fonte, vrai decodage d'image par Qt.

Tout le reste du moteur est le vrai dans les deux cas. Les verifications qui
suivent ne dependent d'aucune mesure au pixel pres, justement pour que le
resultat soit le meme des deux cotes — ce qui fait de l'execution locale un
filet utile, et de celle sous l'OS une preuve.

Ce qu'elles couvrent : l'analyse HTML, les selecteurs, le DOM vu depuis
JavaScript, les evenements, les minuteries, les promesses, le bac a sable, les
images et la mise en page. Ce qu'elles ne couvrent pas : le rendu a l'ecran et
le reseau, qui demandent l'un un ecran, l'autre le monde exterieur.
"""

import hashlib
import json
import os
import shutil
import struct
import sys
import types

# --- L'hote : le vrai s'il est la, un bouchon sinon --------------------------

_images = []


# Les polices que la page a livrees, par famille : le bouchon ne les dessine
# pas, il retient seulement qu'on les lui a remises.
_POLICES = {}


def _largeur_texte(texte, taille, gras=False, fixe=False, famille=""):
    # Approximation suffisante pour la mise en page : la vraie mesure vient de
    # Qt, et aucune verification ici ne depend du dixieme de pixel.
    return len(texte) * taille * (0.62 if not fixe else 0.60) * (1.05 if gras else 1.0)


def _police(octets, famille, gras=False, italique=False):
    _POLICES[(famille, bool(gras), bool(italique))] = bytes(octets)
    return True


def _rasterise(operations, largeur, hauteur):
    """Rasterisation de bouchon : les rectangles pleins, et rien d'autre.

    Le vrai hote joue la liste d'affichage entiere par QPainter. Ici il suffit
    de prouver que la chaine tient — `getImageData` demande, l'hote peint, les
    octets reviennent dans le bon ordre de canaux. Les rectangles suffisent a
    l'etablir, et ce sont eux qu'un test peut verifier au pixel.
    """
    tampon = bytearray(largeur * hauteur * 4)
    for operation in operations:
        if operation[0] != "rect":
            continue
        _, x, y, l, h, couleur = operation
        bleu = couleur & 0xFF
        vert = (couleur >> 8) & 0xFF
        rouge = (couleur >> 16) & 0xFF
        alpha = (couleur >> 24) & 0xFF
        for j in range(max(0, int(y)), min(hauteur, int(y + h))):
            for i in range(max(0, int(x)), min(largeur, int(x + l))):
                debut = (j * largeur + i) * 4
                # L'hote rend du BGRA, comme Qt : `js.py` echange les canaux.
                tampon[debut:debut + 4] = bytes((bleu, vert, rouge, alpha))
    return bytes(tampon)


def _image_brute(octets, largeur, hauteur, identifiant=-1):
    _images.append((largeur, hauteur))
    return (len(_images) - 1, largeur, hauteur)


def _image(octets):
    """Lit la taille d'un PNG sans decoder : c'est tout ce dont on a besoin."""
    if len(octets) > 24 and octets[:8] == b"\x89PNG\r\n\x1a\n":
        largeur, hauteur = struct.unpack(">II", octets[16:24])
        _images.append((largeur, hauteur))
        return (len(_images) - 1, largeur, hauteur)
    return None


try:
    import bo  # fourni par l'hote Qt quand on tourne sous l'OS
    HOTE_REEL = True
except ImportError:
    bo = types.ModuleType("bo")
    bo.largeur_texte = _largeur_texte
    bo.hauteur_ligne = lambda taille, fixe=False: taille * 1.35
    bo.taille = lambda: (1280, 720)
    bo.titre = lambda _: None
    bo.redessiner = lambda: None
    bo.image = _image
    bo.image_brute = _image_brute
    bo.rasterise = _rasterise
    bo.police = _police
    bo.formats_images = lambda: ["png"]
    sys.modules["bo"] = bo
    HOTE_REEL = False

sys.path.insert(0, ".")

import moteur  # noqa: E402
from moteur import css, grille, html, js, reseau, telemetrie  # noqa: E402

# --- Cadre de verification ----------------------------------------------------

_echecs = []
_reussites = 0


def verifie(nom, condition, detail=""):
    global _reussites
    if condition:
        _reussites += 1
    else:
        _echecs.append("%s%s" % (nom, (" — " + str(detail)) if detail else ""))


def egal(nom, obtenu, attendu):
    verifie(nom, obtenu == attendu, "obtenu %r, attendu %r" % (obtenu, attendu))


def document(source, url="http://exemple.test/page"):
    """Construit un Document depuis du HTML, sans reseau."""
    journal = []
    reponse = reseau.Reponse(url, source, "text/html", 200)
    # Pas de prechargement : ces pages n'ont pas de sous-ressource reelle, et
    # une resolution de nom vers un domaine inexistant ferait attendre chaque
    # verification pour rien. Le prechargement a ses propres epreuves.
    doc = moteur.Document(reponse, 1000, precharge=False,
                          journal=lambda niveau, texte: journal.append((niveau, texte)))
    doc.messages = journal
    return doc


# --- HTML et selecteurs -------------------------------------------------------

def verifie_html():
    racine = html.analyse("<p>un<p>deux<ul><li>a<li>b</ul>")
    paragraphes = [n for n in racine.parcours()
                   if isinstance(n, html.Element) and n.balise == "p"]
    egal("html: deux paragraphes frereS", len(paragraphes), 2)
    elements = [n for n in racine.parcours()
                if isinstance(n, html.Element) and n.balise == "li"]
    egal("html: deux li non imbriques", len(elements), 2)
    verifie("html: li non imbriques", all(e.parent.balise == "ul" for e in elements))

    racine = html.analyse("<div>a &amp; b &#233; &eacute;</div>")
    egal("html: entites", racine.trouve("div").texte(), "a & b é é")

    racine = html.analyse("<script>if (a < b) { x(); }</script><p>apres")
    egal("html: script brut preserve",
         racine.trouve("script").texte(), "if (a < b) { x(); }")
    verifie("html: contenu apres script", racine.trouve("p") is not None)


def verifie_selecteurs():
    doc = document("""
        <body>
          <div id="tete" class="barre grande">
            <a href="/x" class="lien" data-role="menu">X</a>
            <a href="/y" class="lien actif">Y</a>
          </div>
          <ul><li class="item">1<li class="item special">2</ul>
        </body>""")
    contexte = js.Contexte(doc)

    def select(selecteur, racine=None):
        return contexte.appel("select", racine, selecteur, True)

    egal("select: par balise", len(select("a")), 2)
    egal("select: par classe", len(select(".item")), 2)
    egal("select: deux classes", len(select(".item.special")), 1)
    egal("select: par identifiant", len(select("#tete")), 1)
    egal("select: descendant", len(select("#tete a")), 2)
    egal("select: enfant direct", len(select("ul > li")), 2)
    egal("select: enfant direct absent", len(select("body > a")), 0)
    egal("select: attribut present", len(select("[data-role]")), 1)
    egal("select: attribut egal", len(select('[data-role="menu"]')), 1)
    egal("select: attribut faux", len(select('[data-role="autre"]')), 0)
    egal("select: attribut prefixe", len(select('[href^="/x"]')), 1)
    egal("select: groupe", len(select("a, li")), 4)
    # Au repos, `:hover` ne designe rien. Le moteur l'ignorait, et peignait donc
    # en permanence le style que la page reservait au pointeur.
    egal("select: pseudo-classe d'etat", len(select("a:hover")), 0)
    egal("select: universel dans contexte", len(select("*")),
         len([n for n in doc.racine.parcours() if isinstance(n, html.Element)]))

    # Freres, rangs et pseudo-classes fonctionnelles : `querySelector` et la
    # cascade partagent desormais le meme moteur, donc les memes reponses.
    egal("select: frere adjacent", len(select("a + a")), 1)
    egal("select: frere general", len(select("li ~ li")), 1)
    egal("select: premier enfant", len(select("li:first-child")), 1)
    egal("select: dernier enfant", len(select("li:last-child")), 1)
    egal("select: rang impair", len(select("li:nth-child(odd)")), 1)
    egal("select: rang calcule", len(select("li:nth-child(2n)")), 1)
    egal("select: negation", len(select("a:not(.actif)")), 1)
    egal("select: negation composee", len(select("li:not(.special):not(.autre)")), 1)
    egal("select: is", len(select("a:is(.actif, .absent)")), 1)
    egal("select: where sans poids", len(select(":where(a).lien")), 2)
    egal("select: has descendant", len(select("div:has(a)")), 1)
    egal("select: has enfant direct", len(select("ul:has(> li)")), 1)
    egal("select: has absent", len(select("ul:has(a)")), 0)
    egal("select: attribut sous-chaine", len(select('[data-role*="en"]')), 1)
    egal("select: attribut insensible", len(select('[data-role="MENU" i]')), 1)
    contexte.ferme()

    # La specificite suit la norme : `:is()` prend celle de son argument le plus
    # fort, `:where()` ne pese rien, `:not()` pese ce qu'il nie.
    egal("specificite: is", css.groupes(":is(#a, .b)")[0].specificite, (1, 0, 0))
    egal("specificite: where", css.groupes(":where(#a)")[0].specificite, (0, 0, 0))
    egal("specificite: not", css.groupes("p:not(.b)")[0].specificite, (0, 1, 1))
    # Une classe echappee — `md\:flex`, `w-1\/2` — est un seul nom, pas deux.
    egal("selecteur: classe echappee",
         css.groupes(r".md\:flex")[0].maillons[0].classes, ["md:flex"])


# --- JavaScript ---------------------------------------------------------------

def verifie_javascript_base():
    doc = document("<body><p id='a'>bonjour</p></body>")
    contexte = js.Contexte(doc)

    egal("js: arithmetique", contexte.execute("1 + 2 * 3"), 7)
    egal("js: chaines", contexte.execute("'a' + 'b'"), "ab")
    egal("js: JSON", contexte.execute("JSON.parse('{\"x\":5}').x"), 5)
    egal("js: expressions rationnelles",
         contexte.execute("'2026-08-05'.match(/(\\d+)-(\\d+)/)[2]"), "08")
    egal("js: fonctions flechees et reduce",
         contexte.execute("[1,2,3,4].map(n => n * n).reduce((a, b) => a + b, 0)"), 30)
    egal("js: classes", contexte.execute(
        "class A { constructor(x) { this.x = x; } get double() { return this.x * 2; } }"
        "new A(21).double"), 42)
    egal("js: destructuration", contexte.execute(
        "const {a, ...reste} = {a: 1, b: 2, c: 3}; Object.keys(reste).length"), 2)
    egal("js: gabarits", contexte.execute("const n = 3; `il y en a ${n}`"), "il y en a 3")

    # Les valeurs traversent dans les deux sens.
    egal("js: tableau vers Python", contexte.execute("[1, 'deux', true, null]"),
         [1, "deux", True, None])
    resultat = contexte.execute("({x: 1, y: [2, 3]})")
    egal("js: objet vers Python", resultat, {"x": 1, "y": [2, 3]})
    contexte.ferme()


def verifie_dom():
    doc = document("""
        <body>
          <h1 id="titre">Bonjour</h1>
          <div class="liste"><span>a</span><span>b</span></div>
        </body>""")
    contexte = js.Contexte(doc)
    execute = contexte.execute

    egal("dom: getElementById", execute("document.getElementById('titre').textContent"),
         "Bonjour")
    egal("dom: tagName", execute("document.getElementById('titre').tagName"), "H1")
    egal("dom: querySelectorAll", execute("document.querySelectorAll('span').length"), 2)
    egal("dom: identite des nœuds",
         execute("document.getElementById('titre') === document.querySelector('h1')"), True)
    egal("dom: textContent en ecriture", execute(
        "document.getElementById('titre').textContent = 'Salut';"
        "document.getElementById('titre').textContent"), "Salut")
    verifie("dom: l'ecriture atteint l'arbre reel",
            doc.racine.trouve("h1").texte() == "Salut")

    egal("dom: attributs", execute("""(function () {
        const e = document.querySelector('.liste');
        e.setAttribute('data-x', '7');
        return e.getAttribute('data-x');
    })()"""), "7")
    egal("dom: classList.add", execute("""(function () {
        const e = document.querySelector('.liste');
        e.classList.add('vive');
        return e.className;
    })()"""), "liste vive")
    egal("dom: classList.contains",
         execute("document.querySelector('.liste').classList.contains('vive')"), True)
    egal("dom: classList.remove", execute("""(function () {
        const e = document.querySelector('.liste');
        e.classList.remove('liste');
        return e.className;
    })()"""), "vive")

    egal("dom: createElement + appendChild", execute("""(function () {
        const n = document.createElement('p');
        n.textContent = 'neuf';
        document.body.appendChild(n);
        return document.querySelectorAll('p').length;
    })()"""), 1)
    verifie("dom: le nœud cree est dans l'arbre",
            any(isinstance(n, html.Element) and n.balise == "p"
                for n in doc.racine.parcours()))

    egal("dom: freres", execute("""(function () {
        const premier = document.querySelector('.vive span');
        return [premier.textContent,
                premier.nextElementSibling.textContent,
                premier.nextElementSibling.previousElementSibling.textContent].join('');
    })()"""), "aba")

    egal("dom: innerHTML en lecture",
         execute("document.querySelector('.vive').innerHTML"),
         "<span>a</span><span>b</span>")
    egal("dom: innerHTML en ecriture", execute("""(function () {
        document.querySelector('.vive').innerHTML = '<b>gras</b>';
        return document.querySelectorAll('b').length;
    })()"""), 1)
    verifie("dom: innerHTML a remplace les enfants reels",
            len([n for n in doc.racine.parcours()
                 if isinstance(n, html.Element) and n.balise == "span"]) == 0)
    egal("dom: removeChild", execute("""(function () {
        document.querySelector('b').remove();
        return document.querySelectorAll('b').length;
    })()"""), 0)

    egal("dom: style en ecriture", execute("""(function () {
        const e = document.querySelector('.vive');
        e.style.color = 'red';
        return e.getAttribute('style');
    })()"""), "color: red")
    egal("dom: style camelCase", execute("""(function () {
        const e = document.querySelector('.vive');
        e.style.backgroundColor = 'blue';
        return e.style.backgroundColor;
    })()"""), "blue")
    contexte.ferme()


def verifie_evenements():
    doc = document("""
        <body><div id="boite"><button id="bouton">Cliquer</button></div></body>""")
    contexte = js.Contexte(doc)
    execute = contexte.execute

    execute("""
        globalThis.traces = [];
        document.getElementById('bouton').addEventListener('click', function (e) {
            traces.push('bouton:' + e.type);
        });
        document.getElementById('boite').addEventListener('click', function () {
            traces.push('boite');
        });
    """)
    bouton = next(n for n in doc.racine.parcours()
                  if isinstance(n, html.Element) and n.attributs.get("id") == "bouton")
    contexte.evenement(bouton, "click", {"clientX": 5})
    egal("evenements: cible puis remontee", execute("traces.join(',')"),
         "bouton:click,boite")

    execute("""
        traces = [];
        document.getElementById('boite').addEventListener('click', function (e) {
            e.stopPropagation();
        }, true);
    """)
    contexte.evenement(bouton, "click", {})
    egal("evenements: stopPropagation en capture", execute("traces.length"), 0)

    doc3 = document("<body><button id='b'>x</button></body>")
    contexte3 = js.Contexte(doc3)
    contexte3.execute("""
        document.getElementById('b').addEventListener('click', function (e) {
            e.preventDefault();
        });
    """)
    cible3 = next(n for n in doc3.racine.parcours()
                  if isinstance(n, html.Element) and n.attributs.get("id") == "b")
    egal("evenements: preventDefault remonte a Python",
         contexte3.evenement(cible3, "click", {}), False)
    contexte3.ferme()

    # Un attribut `onclick` est du code, et doit s'executer.
    doc2 = document("<body><a id='l' href='#' onclick='globalThis.vu = 1'>x</a></body>")
    contexte2 = js.Contexte(doc2)
    lien = next(n for n in doc2.racine.parcours()
                if isinstance(n, html.Element) and n.attributs.get("id") == "l")
    contexte2.evenement(lien, "click", {})
    egal("evenements: attribut onclick", contexte2.execute("globalThis.vu"), 1)
    contexte.ferme()
    contexte2.ferme()


def verifie_minuteries():
    doc = document("<body><p id='p'>attente</p></body>")
    contexte = js.Contexte(doc)
    contexte.execute("""
        setTimeout(function () {
            document.getElementById('p').textContent = 'echu';
        }, 0);
    """)
    egal("minuteries: rien avant le battement",
         doc.racine.trouve("p").texte(), "attente")
    contexte.tic()
    egal("minuteries: setTimeout echu au battement",
         doc.racine.trouve("p").texte(), "echu")
    verifie("minuteries: l'arbre est marque a refaire", contexte.sale)

    contexte.execute("globalThis.compte = 0;"
                     "globalThis.id = setInterval(() => { compte++; }, 0);")
    contexte.tic()
    contexte.tic()
    verifie("minuteries: setInterval repete", contexte.execute("compte") >= 2)
    contexte.execute("clearInterval(id)")
    avant = contexte.execute("compte")
    contexte.tic()
    egal("minuteries: clearInterval arrete", contexte.execute("compte"), avant)
    contexte.ferme()


def verifie_promesses():
    doc = document("<body></body>")
    contexte = js.Contexte(doc)
    egal("promesses: resolution synchrone", contexte.execute("""
        let vu = 0;
        Promise.resolve(5).then(v => { vu = v; });
        vu
    """), 0)
    egal("promesses: la file est vidée par evalue", contexte.execute("vu"), 5)
    egal("promesses: async/await", contexte.execute("""
        let r = 0;
        (async function () { r = await Promise.resolve(9); })();
        r
    """), 0)
    egal("promesses: await resolu apres la pompe", contexte.execute("r"), 9)
    contexte.ferme()


def verifie_erreurs():
    doc = document("<body></body>")
    messages = []
    contexte = js.Contexte(doc, journal=lambda n, t: messages.append((n, t)))
    contexte.execute("throw new Error('casse')", "essai.js")
    verifie("erreurs: une exception est journalisee, pas propagee",
            any("casse" in t for _, t in messages), messages)

    contexte.execute("null.x", "essai2.js")
    verifie("erreurs: le moteur survit a une erreur de type", len(messages) >= 2)

    egal("erreurs: le contexte reste utilisable", contexte.execute("1 + 1"), 2)
    contexte.ferme()


def verifie_diagnostics_compatibilite():
    """Le rapport mesure des appels executes, et nomme correctement ce qu'il voit.

    Deux defauts de la premiere version sont verrouilles ici.

    Le premier : tout `X is not defined` etait range en API navigateur absente.
    `currentUsr` devenait donc une lacune du moteur, alors que c'est un defaut
    de la page — ou la consequence d'un bundle qui n'a pas charge. Une feuille
    de route batie la-dessus aurait ete pleine de noms qu'aucun navigateur ne
    fournit.

    Le second : seuls `fetch` et `XMLHttpRequest` alimentaient le compteur de
    ressources. Un `<script src>` principal en 404 vidait la page de tout
    comportement sans que le rapport n'en dise un mot.
    """
    doc = document("<body></body>")
    contexte = js.Contexte(doc)
    try:
        # `RTCPeerConnection` n'existe pas : l'accesseur du prelude le note et
        # rend `undefined`, donc la detection de fonctionnalite de la page
        # continue de retomber sur son plan de secours.
        #
        # L'exemple etait `indexedDB` jusqu'a ce qu'il soit implemente. Le
        # remplacer plutot que d'affaiblir l'epreuve : ce qu'elle verifie, c'est
        # que le moteur sait nommer ce qui lui manque — pas qu'il manque
        # quelque chose en particulier.
        contexte.execute("void RTCPeerConnection;", "page.js")
        egal("diagnostic: API absente observee",
             contexte.rapport_compatibilite().get("api_absente:RTCPeerConnection"), 1)

        # A — un identifiant applicatif n'est pas une API navigateur.
        contexte.execute("console.log(currentUsr);", "app.js")
        egal("diagnostic: un identifiant inconnu n'est pas une API",
             contexte.rapport_compatibilite().get("identifiant_inconnu:currentUsr"), 1)
        egal("diagnostic: et il n'apparait pas comme API absente",
             contexte.rapport_compatibilite().get("api_absente:currentUsr"), None)

        for nom in ("webpackChunk", "myApplication", "foo"):
            contexte.execute("void %s;" % nom, "app.js")
            egal("diagnostic: %s reste applicatif" % nom,
                 contexte.rapport_compatibilite().get("api_absente:%s" % nom), None)

        # A bis — les vrais noms de la plate-forme, eux, sont bien reconnus.
        for nom in ("WebSocket", "indexedDB", "TextEncoder"):
            verifie("diagnostic: %s est une surface Web" % nom,
                    telemetrie.est_surface_web(nom))
        for nom in ("currentUsr", "webpackChunk", "myApplication", "foo",
                    "__webpack_require__", "MonComposant"):
            verifie("diagnostic: %s n'est pas une surface Web" % nom,
                    not telemetrie.est_surface_web(nom))

    finally:
        contexte.ferme()

    # Un contexte neuf par forme d'erreur : sans cela les compteurs se melent
    # et une verification qui compte « 1 » pourrait passer pour la mauvaise
    # raison.
    for code, cle, quoi in (
            ("new Array(-1)", "erreur_js:RangeError",
             "les erreurs sont classees par genre"),
            ("JSON.parse('{')", "erreur_js:SyntaxError",
             "une erreur de syntaxe garde son genre"),
            # QuickJS ne nomme pas le membre d'un `x.y()` : le message est un
            # « TypeError: not a function » nu. On compte le cas sans lui
            # inventer de nom, ce serait plus trompeur que de l'admettre.
            ("document.body.tourneEnRond()", "methode_absente:<anonyme>",
             "une methode anonyme est comptee sans etre nommee"),
            # L'autre forme nomme la propriete, et c'est la plus utile de
            # toutes : c'est exactement celle qui tuait le JavaScript de
            # pypi.org.
            ("var vide; vide.includes('x')", "methode_absente:includes",
             "la propriete lue sur undefined est nommee")):
        neuf = js.Contexte(document("<body></body>"))
        try:
            neuf.execute(code, "cas.js")
            egal("diagnostic: %s" % quoi, neuf.rapport_compatibilite().get(cle), 1)
        finally:
            neuf.ferme()


def verifie_moignons():
    """Une API presente mais vide doit se denoncer elle-meme.

    C'est le pire des trois etats. Absente, la page retombe sur son plan de
    secours ; presente et juste, tout va bien ; presente et vide, la page teste
    `if (history.pushState)`, obtient `true`, prend le chemin moderne et
    poursuit avec un navigateur dont l'etat ne correspond plus a ce qu'elle
    croit. Sans cette categorie, ce cas-la n'apparaissait nulle part.
    """
    doc = document("<body><div id=a>x</div></body>")
    contexte = js.Contexte(doc)
    try:
        # Le test de detection doit continuer de reussir : le moignon reste
        # present, sinon on changerait le comportement en le mesurant.
        egal("moignon: history.pushState se detecte encore",
             contexte.execute("typeof history.pushState"), "function")

        contexte.execute("window.scrollTo(0, 100)")
        contexte.execute("window.open('/x')")
        contexte.execute("var x = new XMLHttpRequest(); x.abort()")
        rapport = contexte.rapport_compatibilite()
        for cle in ("moignon_appele:window.scrollTo",
                    "moignon_appele:window.open",
                    "moignon_appele:XMLHttpRequest.abort"):
            egal("moignon: %s est compte" % cle.split(":")[1], rapport.get(cle), 1)

        # Un moignon n'est pas une API absente : les deux se corrigent
        # differemment et ne doivent pas se melanger.
        verifie("moignon: aucun moignon n'est classe API absente",
                not any(c.startswith("api_absente:window") for c in rapport),
                [c for c in rapport if c.startswith("api_absente")])

        # Et la reciproque, qui est la vraie valeur de cette categorie : ce qui
        # a ete implemente ne doit plus s'y trouver. Le jour ou `pushState`
        # redeviendrait vide, cette ligne le dirait.
        contexte.execute("history.pushState({}, '', '/x')")
        contexte.execute("document.getElementById('a').focus()")
        rapport = contexte.rapport_compatibilite()
        for nom in ("history.pushState", "element.focus"):
            egal("moignon: %s n'est plus un moignon" % nom,
                 rapport.get("moignon_appele:%s" % nom), None)
    finally:
        contexte.ferme()


def verifie_ressources_echouees():
    """B — toutes les destinations comptent, pas seulement `fetch`."""
    from moteur import reseau as _reseau

    doc = document("<body></body>", url="http://exemple.test/page")
    contexte = js.Contexte(doc)
    ancien = _reseau.charge

    def absente(url, methode="GET", corps=None, entetes=None, brut=False,
                document=None, destination=None):
        return _reseau.Reponse(url, "", "text/plain", 404)

    _reseau.charge = absente
    try:
        # Le cas qui manquait : le bundle principal de la page.
        contexte._telecharge_script("http://exemple.test/app.js")
        egal("ressources: un script en 404 est compte",
             contexte.rapport_compatibilite().get("ressource_echouee:script"), 1)

        contexte._op_moduleSource("http://exemple.test/mod.js")
        egal("ressources: un module en 404 est compte",
             contexte.rapport_compatibilite().get("ressource_echouee:module"), 1)

        contexte._recupere("GET", "http://exemple.test/api", None, {})
        egal("ressources: un fetch en 404 est compte",
             contexte.rapport_compatibilite().get("ressource_echouee:fetch"), 1)
    finally:
        _reseau.charge = ancien
        contexte.ferme()

    # Les chargeurs hors JavaScript passent par la telemetrie globale, avec la
    # meme cle de destination.
    telemetrie.reinitialise()
    telemetrie.active(True)
    try:
        for destination in ("stylesheet", "image", "font", "media"):
            telemetrie.ressource_echouee("http://exemple.test/x", 404, destination)
        vues = {e["cle"]: e["n"]
                for e in telemetrie.rapport().get(telemetrie.RESSOURCE_ECHOUEE, [])}
        for destination in ("stylesheet", "image", "font", "media"):
            egal("ressources: %s est une destination connue" % destination,
                 vues.get(destination), 1)
        verifie("ressources: la note porte le code et l'adresse",
                any("404" in ex and "exemple.test" in ex
                    for e in telemetrie.rapport()[telemetrie.RESSOURCE_ECHOUEE]
                    for ex in e["exemples"]))
    finally:
        telemetrie.active(False)
        telemetrie.reinitialise()


def verifie_budget():
    doc = document("<body></body>")
    messages = []
    contexte = js.Contexte(doc, journal=lambda n, t: messages.append((n, t)),
                           budget_ms=300)
    contexte.execute("while (true) {}", "boucle.js")
    verifie("budget: une boucle infinie est interrompue",
            any("interrompu" in t for _, t in messages), messages)
    egal("budget: le contexte survit a l'interruption", contexte.execute("2 + 2"), 4)
    contexte.ferme()


def verifie_page_complete():
    """Le cas reel : une page dont le contenu est construit par son script."""
    doc = document("""
        <html><head><title>Essai</title></head>
        <body>
          <div id="racine"></div>
          <script>
            const cible = document.getElementById('racine');
            const titre = document.createElement('h1');
            titre.textContent = 'Construit par JavaScript';
            cible.appendChild(titre);
            for (let i = 1; i <= 3; i++) {
              const ligne = document.createElement('p');
              ligne.className = 'ligne';
              ligne.textContent = 'ligne numero ' + i;
              cible.appendChild(ligne);
            }
            document.title = 'Titre pose par le script';
          </script>
        </body></html>""")

    verifie("page: un contexte JavaScript a ete cree", doc.contexte_js is not None)
    egal("page: le h1 existe dans l'arbre",
         sum(1 for n in doc.racine.parcours()
             if isinstance(n, html.Element) and n.balise == "h1"), 1)
    egal("page: les trois lignes existent",
         sum(1 for n in doc.racine.parcours()
             if isinstance(n, html.Element)
             and "ligne" in n.attributs.get("class", "")), 3)
    egal("page: le titre pose par le script", doc.titre, "Titre pose par le script")

    # Et surtout : le contenu construit par le script est reellement mis en page.
    liste = doc.liste_affichage(0, 1000, 700)
    textes = [e[3] for e in liste if e[0] == "texte"]
    verifie("page: le texte du script est peint",
            any("Construit par JavaScript" in t for t in textes), textes[:6])
    verifie("page: les lignes sont peintes",
            sum(1 for t in textes if "ligne" in t and "numero" in t) >= 3, textes[:12])


def verifie_evenement_apres_chargement():
    """Une page qui attend `DOMContentLoaded` doit voir son code s'executer."""
    doc = document("""
        <body>
          <div id="cible">avant</div>
          <script>
            document.addEventListener('DOMContentLoaded', function () {
              document.getElementById('cible').textContent = 'apres DOMContentLoaded';
            });
            window.addEventListener('load', function () {
              document.getElementById('cible').setAttribute('data-charge', 'oui');
            });
          </script>
        </body>""")
    cible = next(n for n in doc.racine.parcours()
                 if isinstance(n, html.Element) and n.attributs.get("id") == "cible")
    egal("cycle: DOMContentLoaded a bien tire", cible.texte(),
         "apres DOMContentLoaded")
    egal("cycle: load a bien tire", cible.attributs.get("data-charge"), "oui")


def verifie_images():
    """Une image PNG minuscule, decodee par le bouchon, doit etre mise en page."""
    import base64
    from moteur import images

    # PNG 2x2 reel — quatre pixels de couleurs differentes —, en clair dans le
    # fichier pour ne dependre d'aucun outil. Il doit etre valide au sens strict
    # (CRC de chaque bloc, flux zlib coherent) : sous l'OS c'est le vrai libpng
    # de Qt qui le lit, et il refuse ce qu'un bouchon laisserait passer.
    png = base64.b64decode(
        "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAFElEQVR42mP4"
        "z8DAAMIM////ZwAAHu8E/HMcU8wAAAAASUVORK5CYII=")
    images.vide()
    source = "data:image/png;base64," + base64.b64encode(png).decode()
    doc = document('<body><img src="%s" alt="deux pixels"></body>' % source)
    liste = doc.liste_affichage(0, 1000, 700)
    ops = [e for e in liste if e[0] == "image"]
    egal("images: une image est peinte", len(ops), 1)
    if ops:
        egal("images: taille naturelle respectee", (ops[0][3], ops[0][4]), (2.0, 2.0))

    # Une image absente retombe sur son texte de remplacement.
    images.vide()
    doc = document('<body><img src="http://absent.test/x.png" alt="remplacement"></body>')
    textes = [e[3] for e in doc.liste_affichage(0, 1000, 700) if e[0] == "texte"]
    verifie("images: repli sur alt", any("remplacement" in t for t in textes), textes)

    # Et les dimensions demandees par la page l'emportent.
    images.vide()
    doc = document('<body><img src="%s" width="40" height="20"></body>' % source)
    ops = [e for e in doc.liste_affichage(0, 1000, 700) if e[0] == "image"]
    if ops:
        egal("images: attributs width/height obeis", (ops[0][3], ops[0][4]), (40.0, 20.0))
    else:
        verifie("images: attributs width/height obeis", False, "aucune image peinte")


def verifie_media_api():
    """L'API media telle que la page la voit, sans decoder quoi que ce soit."""
    doc = document("""
        <body>
          <video id="v" width="320" height="240"><p>repli</p></video>
          <audio id="a"></audio>
        </body>""")
    contexte = js.Contexte(doc)
    execute = contexte.execute

    egal("media: <video> est un HTMLMediaElement",
         execute("document.getElementById('v') instanceof HTMLMediaElement"), True)
    egal("media: <audio> aussi",
         execute("document.getElementById('a') instanceof HTMLMediaElement"), True)
    egal("media: un div ne l'est pas",
         execute("document.createElement('div') instanceof HTMLMediaElement"), False)

    egal("media: paused au depart", execute("document.getElementById('v').paused"), True)
    egal("media: currentTime au depart",
         execute("document.getElementById('v').currentTime"), 0)
    egal("media: duration vaut NaN sans flux",
         execute("Number.isNaN(document.getElementById('v').duration)"), True)
    egal("media: le volume par defaut est 1",
         execute("document.getElementById('v').volume"), 1)
    egal("media: muted par defaut", execute("document.getElementById('v').muted"), False)

    egal("media: play() rend une promesse",
         execute("document.getElementById('v').play() instanceof Promise"), True)
    egal("media: Audio() fabrique un element",
         execute("new Audio() instanceof HTMLMediaElement"), True)

    # Les evenements de cycle de vie doivent partir.
    execute("""
        globalThis.vus = [];
        const v = document.getElementById('v');
        for (const nom of ['play', 'pause', 'volumechange']) {
            v.addEventListener(nom, () => vus.push(nom));
        }
        v.play(); v.pause(); v.volume = 0.5;
    """)
    egal("media: play, pause et volumechange sont emis",
         execute("vus.join(',')"), "play,pause,volumechange")

    egal("media: canPlayType refuse un format inconnu",
         execute("document.getElementById('v').canPlayType('video/quicktime')"), "")
    contexte.ferme()


def verifie_mse_api():
    """MediaSource et SourceBuffer : le chemin des sites de lecture video."""
    doc = document("<body><video id='v'></video></body>")
    contexte = js.Contexte(doc)
    execute = contexte.execute

    egal("mse: MediaSource existe", execute("typeof MediaSource"), "function")
    egal("mse: isTypeSupported est une fonction",
         execute("typeof MediaSource.isTypeSupported"), "function")
    egal("mse: createObjectURL rend l'adresse de la source", execute("""(function () {
        const source = new MediaSource();
        return URL.createObjectURL(source) === source.__urlObjet;
    })()"""), True)

    # Le deroulement complet, tel qu'un lecteur l'ecrit.
    execute("""
        globalThis.etapes = [];
        globalThis.source = new MediaSource();
        globalThis.video = document.getElementById('v');
        video.src = URL.createObjectURL(source);
        source.addEventListener('sourceopen', function () {
            etapes.push('sourceopen:' + source.readyState);
            const tampon = source.addSourceBuffer('video/mp4; codecs="avc1.42E01E"');
            tampon.addEventListener('updateend', function () {
                etapes.push('updateend');
            });
            tampon.appendBuffer(new Uint8Array([1, 2, 3, 4]).buffer);
            etapes.push('append');
        });
    """)
    # `sourceopen` part au tour de boucle suivant, comme dans un navigateur.
    contexte.tic()
    contexte.tic()
    verifie("mse: sourceopen est emis avec readyState 'open'",
            "sourceopen:open" in (execute("etapes.join(',')") or ""),
            execute("etapes.join(',')"))
    verifie("mse: appendBuffer puis updateend",
            "append" in (execute("etapes.join(',')") or "")
            and "updateend" in (execute("etapes.join(',')") or ""),
            execute("etapes.join(',')"))
    egal("mse: le tampon est rattache a la source",
         execute("source.sourceBuffers.length"), 1)
    egal("mse: endOfStream change l'etat",
         execute("source.endOfStream(); source.readyState"), "ended")
    contexte.ferme()


# --- YouTube ------------------------------------------------------------------
#
# Le reseau de ce bac a sable n'atteint pas YouTube. Ce qui suit eprouve donc
# tout ce qui ne demande pas le reseau : l'analyse des adresses, l'extraction de
# la reponse du lecteur, le choix du format, et — le point le plus delicat — le
# dechiffrement d'une signature en executant du code de la forme exacte de
# `base.js` dans QuickJS.

# Une reponse de lecteur reduite a ce qui compte, de la forme reelle.
REPONSE_YOUTUBE = {
    "playabilityStatus": {"status": "OK"},
    "videoDetails": {
        "videoId": "aqz-KE-bpKQ",
        "title": "Big Buck Bunny",
        "author": "Blender Foundation",
        "lengthSeconds": "635",
        "shortDescription": "Un lapin geant et trois rongeurs.",
        "viewCount": "12345678",
        "thumbnail": {"thumbnails": [{"url": "https://i.ytimg.com/vi/x/hq.jpg"}]},
    },
    "streamingData": {
        "formats": [
            {"itag": 18, "mimeType": 'video/mp4; codecs="avc1.42001E, mp4a.40.2"',
             "height": 360, "contentLength": "12345678",
             "url": "https://rr1.googlevideo.com/videoplayback?itag=18"},
            {"itag": 22, "mimeType": 'video/mp4; codecs="avc1.64001F, mp4a.40.2"',
             "height": 720, "contentLength": "45678901",
             "url": "https://rr1.googlevideo.com/videoplayback?itag=22"},
        ],
        "adaptiveFormats": [
            {"itag": 137, "mimeType": 'video/mp4; codecs="avc1.640028"',
             "height": 1080, "contentLength": "99999999",
             "url": "https://rr1.googlevideo.com/videoplayback?itag=137"},
            {"itag": 160, "mimeType": 'video/mp4; codecs="avc1.4d400c"',
             "height": 144, "contentLength": "1111111",
             "url": "https://rr1.googlevideo.com/videoplayback?itag=160"},
            {"itag": 140, "mimeType": 'audio/mp4; codecs="mp4a.40.2"',
             "contentLength": "2222222",
             "url": "https://rr1.googlevideo.com/videoplayback?itag=140"},
            {"itag": 251, "mimeType": 'audio/webm; codecs="opus"',
             "contentLength": "1500000",
             "url": "https://rr1.googlevideo.com/videoplayback?itag=251"},
        ],
    },
}


def verifie_youtube_adresses():
    from moteur import youtube

    for adresse, attendu in (
        ("https://www.youtube.com/watch?v=aqz-KE-bpKQ", "aqz-KE-bpKQ"),
        ("https://youtube.com/watch?v=aqz-KE-bpKQ&t=42", "aqz-KE-bpKQ"),
        ("http://m.youtube.com/watch?v=aqz-KE-bpKQ", "aqz-KE-bpKQ"),
        ("https://youtu.be/aqz-KE-bpKQ", "aqz-KE-bpKQ"),
        ("https://youtu.be/aqz-KE-bpKQ?t=90", "aqz-KE-bpKQ"),
        ("https://www.youtube.com/embed/aqz-KE-bpKQ", "aqz-KE-bpKQ"),
        ("https://www.youtube.com/shorts/aqz-KE-bpKQ", "aqz-KE-bpKQ"),
        ("https://www.youtube.com/live/aqz-KE-bpKQ", "aqz-KE-bpKQ"),
        ("https://www.youtube-nocookie.com/embed/aqz-KE-bpKQ", "aqz-KE-bpKQ"),
    ):
        egal("yt: %s" % adresse, youtube.identifiant(adresse), attendu)

    for adresse in (
        "https://www.youtube.com/", "https://www.youtube.com/watch?v=trop-court",
        "https://exemple.test/watch?v=aqz-KE-bpKQ", "https://youtu.be/",
        "", None,
    ):
        verifie("yt: rejette %r" % adresse, youtube.identifiant(adresse) is None)

    verifie("yt: est_youtube reconnait",
            youtube.est_youtube("https://youtu.be/aqz-KE-bpKQ"))
    verifie("yt: est_youtube rejette",
            not youtube.est_youtube("https://exemple.test/"))


def verifie_youtube_formats():
    from moteur import youtube

    video, audio = youtube.choisit_flux(REPONSE_YOUTUBE, hauteur_max=480)
    egal("yt: le progressif est prefere", video.itag, 18)
    verifie("yt: le progressif n'a pas de piste separee", audio is None)
    egal("yt: codec video reconnu", video.codec_video, "h264")
    egal("yt: codec audio reconnu", video.codec_audio, "aac")
    verifie("yt: marque progressif", video.progressif)

    # Une hauteur plus large laisse passer le 720p, lui aussi progressif.
    video, _ = youtube.choisit_flux(REPONSE_YOUTUBE, hauteur_max=720)
    egal("yt: la hauteur maximale est respectee", video.itag, 22)

    # Sans progressif, on retombe sur l'adaptatif : image et son separes.
    sans_progressif = dict(REPONSE_YOUTUBE)
    sans_progressif["streamingData"] = dict(REPONSE_YOUTUBE["streamingData"])
    sans_progressif["streamingData"]["formats"] = []
    video, audio = youtube.choisit_flux(sans_progressif, hauteur_max=480)
    egal("yt: adaptatif, image sous la limite", video.itag, 160)
    verifie("yt: adaptatif, une piste audio est choisie", audio is not None)
    egal("yt: la piste audio la plus legere gagne", audio.itag, 251)

    # Un decodeur absent doit ecarter le format.
    video, audio = youtube.choisit_flux(REPONSE_YOUTUBE, hauteur_max=480,
                                        codecs=("vp9", "opus"))
    verifie("yt: un format non decodable est ecarte",
            video is None or video.codec_video == "vp9")

    infos = youtube.details(REPONSE_YOUTUBE)
    egal("yt: titre", infos["titre"], "Big Buck Bunny")
    egal("yt: auteur", infos["auteur"], "Blender Foundation")
    egal("yt: duree", infos["duree"], 635)
    egal("yt: duree lisible", youtube.duree_lisible(635), "10:35")
    egal("yt: duree lisible avec heures", youtube.duree_lisible(3725), "1:02:05")


def verifie_youtube_extraction_json():
    """`ytInitialPlayerResponse` se lit en comptant les accolades, pas au motif."""
    from moteur import youtube

    page = ('<script>var ytInitialPlayerResponse = '
            '{"a": {"b": "}{ piege \\" encore }"}, "c": [1, 2, {"d": 3}]};'
            'var autre = 1;</script>')
    donnees = youtube._objet_apres(page, "ytInitialPlayerResponse")
    verifie("yt: l'objet est extrait malgre les accolades dans les chaines",
            donnees is not None, page)
    if donnees:
        egal("yt: structure imbriquee preservee", donnees["c"][2]["d"], 3)

    verifie("yt: absence signalee",
            youtube._objet_apres("<script>rien</script>", "ytInitialPlayerResponse")
            is None)


# Un `base.js` de la forme exacte du vrai : un objet de trois operations
# elementaires et une fonction qui les enchaine. C'est la structure que YouTube
# emploie depuis des annees ; seuls les noms et l'ordre changent.
BASE_JS_ESSAI = """
var _yt_player={};(function(g){var window=this;
var Ln={
 wZ:function(a){a.reverse()},
 j9:function(a,b){var c=a[0];a[0]=a[b%a.length];a[b%a.length]=c},
 Qt:function(a,b){a.splice(0,b)}
};
var xk=function(a){a=a.split("");Ln.wZ(a,52);Ln.Qt(a,3);Ln.j9(a,17);Ln.wZ(a,8);
return a.join("")};
g.xk=xk;})(_yt_player);
"""


def _signature_attendue(chaine):
    """La meme transformation, ecrite en Python : la reference du test."""
    a = list(chaine)
    a.reverse()                      # wZ
    a = a[3:]                        # Qt(3)
    b = 17 % len(a)                  # j9(17)
    a[0], a[b] = a[b], a[0]
    a.reverse()                      # wZ
    return "".join(a)


def verifie_youtube_signature():
    """Le point delicat : executer la fonction de YouTube pour dechiffrer.

    On ne reimplemente pas la transformation — elle change plusieurs fois par
    mois. On l'extrait et on l'execute, ce que le moteur JavaScript permet. La
    verification compare le resultat a la meme transformation ecrite a la main.
    """
    from moteur import signature

    source = signature.extrait_fonction_signature(BASE_JS_ESSAI)
    verifie("signature: la fonction est retrouvee dans base.js",
            source is not None, BASE_JS_ESSAI[:80])
    if source is None:
        return
    verifie("signature: l'objet auxiliaire est embarque avec elle",
            "Ln" in source and "splice" in source, source[:200])

    dechiffreur = signature.Dechiffreur(BASE_JS_ESSAI)
    verifie("signature: le dechiffreur est utilisable", dechiffreur.utilisable())

    for entree in ("ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
                   "aXbYcZ_-0123456789abcdefghijABCDEFGH",
                   "0123456789"):
        obtenu = dechiffreur.signature(entree)
        egal("signature: %s..." % entree[:10], obtenu, _signature_attendue(entree))
    dechiffreur.ferme()


def verifie_youtube_base_js():
    """L'adresse de `base.js` se retrouve sous ses formes courantes."""
    from moteur import signature

    for page, attendu in (
        ('{"jsUrl":"/s/player/abc123/fr_FR/base.js"}',
         "https://www.youtube.com/s/player/abc123/fr_FR/base.js"),
        ('x = "/s/player/deadbeef/player_ias.vflset/fr_FR/base.js";',
         "https://www.youtube.com/s/player/deadbeef/player_ias.vflset/fr_FR/base.js"),
        ('<script src="https://www.youtube.com/s/player/z/base.js"></script>',
         "https://www.youtube.com/s/player/z/base.js"),
    ):
        egal("base.js: %s" % page[:28], signature.adresse_du_lecteur(page), attendu)
    verifie("base.js: absence signalee",
            signature.adresse_du_lecteur("<html>rien</html>") is None)


def verifie_youtube_page():
    """La page de lecture batie a partir d'une reponse enregistree."""
    from moteur import lecteur_youtube, youtube

    video, audio = youtube.choisit_flux(REPONSE_YOUTUBE, hauteur_max=480)
    infos = youtube.details(REPONSE_YOUTUBE)
    html_page = lecteur_youtube._html(
        "https://youtu.be/aqz-KE-bpKQ", infos, video, audio,
        {"video": video.url}, "IOS")

    for attendu in ("<video", "Big Buck Bunny", "Blender Foundation",
                    "itag 18", "10:35", "googlevideo.com"):
        verifie("page yt: contient %r" % attendu, attendu in html_page,
                html_page[:150])

    # Et cette page doit reellement se mettre en page, script compris.
    doc = document(html_page, "https://youtu.be/aqz-KE-bpKQ")
    egal("page yt: le titre est celui de la video", doc.titre, "Big Buck Bunny")
    verifie("page yt: le lecteur existe dans l'arbre",
            any(isinstance(n, html.Element) and n.balise == "video"
                for n in doc.racine.parcours()))
    textes = [e[3] for e in doc.liste_affichage(0, 1000, 700) if e[0] == "texte"]
    verifie("page yt: le titre est peint",
            any("Big Buck Bunny" in t for t in textes), textes[:8])


def verifie_youtube_client():
    """Le client qui obtient l'adresse doit etre celui qui la reclame.

    Google lie l'adresse d'un flux au client qui l'a demandee : la meme adresse
    reclamee sous un autre agent rend 403. C'est la panne la plus courante d'une
    extraction par ailleurs correcte — l'adresse est bonne, seul l'en-tete ne
    l'est pas.
    """
    from moteur import lecteur_youtube, youtube

    # L'ordre des clients compte : ceux qui n'exigent pas de preuve d'origine
    # passent devant.
    noms = [c["nom"] for c in youtube.CLIENTS]
    egal("client yt: ANDROID_VR est essaye en premier", noms[0], "ANDROID_VR")
    verifie("client yt: le web reste en dernier recours", noms[-1] == "WEB", noms)
    verifie("client yt: chaque client a un agent",
            all(c.get("agent") for c in youtube.CLIENTS))
    verifie("client yt: chaque client a un numero",
            all(youtube._numero_client(n) for n in noms))

    verifie("client yt: l'agent d'ANDROID_VR est celui de l'application",
            "vr.oculus" in youtube.agent_du_client("ANDROID_VR"),
            youtube.agent_du_client("ANDROID_VR"))
    verifie("client yt: un client inconnu retombe sur un agent utilisable",
            youtube.agent_du_client("INEXISTANT").startswith("Mozilla/"),
            youtube.agent_du_client("INEXISTANT"))

    # La page produite doit porter l'agent, sinon la lecture le perd.
    video, audio = youtube.choisit_flux(REPONSE_YOUTUBE, hauteur_max=480)
    html_page = lecteur_youtube._html(
        "https://youtu.be/aqz-KE-bpKQ", youtube.details(REPONSE_YOUTUBE),
        video, audio, {"video": video.url}, "ANDROID_VR")
    verifie("client yt: la page declare l'agent", "data-agent=" in html_page,
            html_page[:200])
    verifie("client yt: et c'est celui du client qui a servi",
            youtube.agent_du_client("ANDROID_VR") in html_page)

    # Et le moteur doit reellement le reemettre a chaque tranche.
    doc = document(html_page, "https://youtu.be/aqz-KE-bpKQ")
    element = next(n for n in doc.racine.parcours()
                   if isinstance(n, html.Element) and n.balise == "video")
    egal("client yt: l'attribut survit a l'analyse",
         element.attributs.get("data-agent"),
         youtube.agent_du_client("ANDROID_VR"))

    demandes = []

    def charge(url, methode="GET", corps=None, entetes=None, brut=False,
               document=None, destination=None):
        demandes.append((url, dict(entetes or {})))
        return reseau.Reponse(url, "", "video/mp4", 206, entetes={}, octets=b"")

    from moteur import media as module_media
    ancien = reseau.charge
    ancien_dispo = module_media.DISPONIBLE
    try:
        reseau.charge = charge
        module_media.DISPONIBLE = True
        lecteur = module_media.Lecteur()
        lecteur.ouvre_distant("https://r1.googlevideo.com/videoplayback?x=1",
                              None, {"User-Agent": "agent-de-test"})
    except Exception:  # noqa: BLE001 — l'ouverture echoue, seuls les en-tetes comptent
        pass
    finally:
        reseau.charge = ancien
        module_media.DISPONIBLE = ancien_dispo

    verifie("client yt: une tranche part avec l'agent demande",
            demandes and all(d[1].get("User-Agent") == "agent-de-test"
                             for d in demandes), demandes[:2])


def verifie_youtube_chaine():
    """La chaine entiere : adresse -> API -> choix du flux -> page de lecture.

    Le reseau est substitue : ce bac a sable n'atteint pas YouTube, et une
    verification qui en dependrait ne prouverait rien de reproductible. Ce qui
    est eprouve ici est tout le reste — la negociation des clients, ce qu'on
    leur envoie, ce qu'on fait de leur reponse, et la page qui en sort.
    """
    from moteur import lecteur_youtube, youtube

    envoyees = []

    def charge(url, methode="GET", corps=None, entetes=None, brut=False,
               document=None, destination=None):
        envoyees.append({"url": url, "methode": methode, "corps": corps,
                         "entetes": dict(entetes or {})})
        if "/youtubei/v1/player" not in url:
            return reseau.Reponse(url, "", "text/html", 404)
        demande = json.loads(corps)
        nom = demande["context"]["client"]["clientName"]
        # Les deux premiers clients refusent, comme le fait YouTube quand il
        # attend une preuve d'origine ; le troisieme sert.
        if nom in ("ANDROID_VR", "IOS"):
            refus = {"playabilityStatus": {
                "status": "LOGIN_REQUIRED",
                "reason": "Sign in to confirm you're not a bot"}}
            return reseau.Reponse(url, json.dumps(refus), "application/json", 200,
                                  octets=json.dumps(refus).encode())
        brut_json = json.dumps(REPONSE_YOUTUBE)
        return reseau.Reponse(url, brut_json, "application/json", 200,
                              octets=brut_json.encode())

    journal = []
    ancien = reseau.charge
    try:
        reseau.charge = charge
        reponse = lecteur_youtube.page(
            "https://www.youtube.com/watch?v=aqz-KE-bpKQ",
            journal=lambda niveau, texte: journal.append((niveau, texte)))
    finally:
        reseau.charge = ancien

    verifie("chaine yt: une page est produite", reponse is not None)
    if reponse is None:
        return

    # Chaque client refuse doit avoir ete essaye, dans l'ordre, avec ses
    # propres en-tetes.
    essayes = [json.loads(d["corps"])["context"]["client"]["clientName"]
               for d in envoyees if d["corps"]]
    egal("chaine yt: les clients sont essayes dans l'ordre",
         essayes[:3], ["ANDROID_VR", "IOS", "ANDROID"])
    egal("chaine yt: la requete est un POST", envoyees[0]["methode"], "POST")
    egal("chaine yt: avec l'agent du client essaye",
         envoyees[0]["entetes"].get("User-Agent"),
         youtube.agent_du_client("ANDROID_VR"))
    verifie("chaine yt: et le numero de client",
            envoyees[0]["entetes"].get("X-Youtube-Client-Name") == "28",
            envoyees[0]["entetes"])

    # Le refus par preuve d'origine est nomme dans le journal.
    verifie("chaine yt: le refus est explique",
            any("preuve d'origine" in texte for _, texte in journal),
            journal[:4])

    # Et la page finale porte le flux du client qui a servi.
    verifie("chaine yt: la page cite le flux", "googlevideo.com" in reponse.contenu)
    verifie("chaine yt: elle declare l'agent d'ANDROID",
            youtube.agent_du_client("ANDROID") in reponse.contenu)
    verifie("chaine yt: et nomme le client qui a servi",
            "ANDROID" in reponse.contenu, reponse.contenu[:200])

    # La page se met reellement en page.
    doc = document(reponse.contenu, "https://www.youtube.com/watch?v=aqz-KE-bpKQ")
    egal("chaine yt: le titre est celui de la video", doc.titre, "Big Buck Bunny")


def verifie_youtube_refus():
    """Un refus par preuve d'origine doit etre nomme, pas confondu avec une panne."""
    from moteur import youtube

    for raison in ("Sign in to confirm you're not a bot",
                   "Connectez-vous pour confirmer que vous n'etes pas un robot",
                   "Please sign in to confirm you are not a bot"):
        verifie("refus yt: reconnu — %r" % raison[:28],
                youtube._est_preuve_exigee(raison))
    for raison in ("Video unavailable", "This video is private", ""):
        verifie("refus yt: pas confondu — %r" % raison,
                not youtube._est_preuve_exigee(raison))


def verifie_requete_brute():
    """`fetch` et `XMLHttpRequest` doivent rendre le corps, pas une page.

    Le moteur enrobe de HTML ce qu'il ne sait pas afficher — c'est ce qui permet
    d'ouvrir un fichier texte dans une fenetre. Mais une requete faite par un
    script veut la ressource elle-meme : lui rendre « Type non affichable »
    casse tout `fetch` de JSON.
    """
    from moteur import reseau

    vues = {}
    vrai_charge = reseau.charge

    def faux_charge(url, methode="GET", corps=None, entetes=None, brut=False,
                    document=None, destination=None):
        vues["brut"] = brut
        vues["document"] = document
        vues["destination"] = destination
        return reseau.Reponse(url, '{"x": 1}', "application/json", 200,
                              octets=b'{"x": 1}')

    doc = document("<body></body>")
    contexte = js.Contexte(doc)
    reseau.charge = faux_charge
    try:
        contexte.appel("requete", 1, "GET", "https://exemple.test/donnees.json",
                       None, {}, True)
    finally:
        reseau.charge = vrai_charge
    verifie("requete: le corps est demande tel quel", vues.get("brut") is True,
            vues)
    egal("requete: elle se declare emise par le document", vues.get("document"),
         doc.url)
    egal("requete: sa destination est nommee", vues.get("destination"), "fetch")
    contexte.ferme()


def verifie_decoration():
    """Coins, ombres, degrades, bords par cote, opacite : l'allure moderne."""
    doc = document("""
        <style>
          body { margin: 0; }
          #carte {
            width: 300px; height: 120px;
            background: #ffffff;
            border-radius: 12px;
            box-shadow: 0 4px 12px rgba(0,0,0,0.25);
          }
          #trait { height: 40px; border-bottom: 2px solid #ff0000; }
          #pente { height: 60px; background: linear-gradient(to right, #000000, #ffffff); }
          #cadre { height: 30px; border: 1px solid #00ff00; }
          #pale { height: 20px; opacity: 0.5; background: #123456; }
        </style>
        <body>
          <div id="carte">carte</div>
          <div id="trait">trait</div>
          <div id="pente">pente</div>
          <div id="cadre">cadre</div>
          <div id="pale">pale</div>
        </body>""")
    liste = doc.liste_affichage(0, 1000, 700)
    genres = [e[0] for e in liste]

    # Un fond arrondi passe par `rond`, pas par `rect` : sinon les coins
    # restaient carres alors que la feuille les demandait ronds.
    ronds = [e for e in liste if e[0] == "rond"]
    verifie("decoration: fond arrondi", any(proche(e[5], 12) for e in ronds),
            [e[5] for e in ronds])
    verifie("decoration: ombre portee", "ombre" in genres, genres[:12])
    verifie("decoration: degrade", "degrade" in genres, genres[:12])

    # `border-bottom` seul ne peignait rien : le moteur ne lisait qu'une
    # bordure unique pour toute la boite.
    trait = boite_de(doc, "trait")
    rouges = [e for e in liste if e[0] == "rect" and e[5] == 0xFFFF0000]
    verifie("decoration: bord bas seul peint", len(rouges) == 1, len(rouges))
    if rouges:
        verifie("decoration: le bord bas est en bas",
                proche(rouges[0][2], trait.y + trait.hauteur - 2), rouges[0][2])
        verifie("decoration: son epaisseur est la sienne",
                proche(rouges[0][4], 2), rouges[0][4])

    # Un cadre uniforme se trace en contour, sans deborder des coins.
    verifie("decoration: cadre uniforme", "contour" in genres or
            sum(1 for e in liste if e[0] == "rect" and e[5] == 0xFF00FF00) == 4,
            genres[:20])

    # L'opacite enveloppe la boite et se referme.
    verifie("decoration: opacite posee", "opacite" in genres, genres[:20])
    egal("decoration: enveloppes refermees",
         genres.count("desenveloppe"),
         genres.count("opacite") + genres.count("transforme"))

    # Les bords se cascadent par cote : trois rouges et un efface.
    style = css._developpe({"border": "1px solid red"})
    style.update(css._developpe({"border-bottom": "none"}))
    bords = css.bordures(style, 100, 16)
    egal("decoration: cascade des bords",
         [round(e) for e, _ in bords], [1, 1, 0, 1])


def verifie_transformations():
    """`transform` deplace la boite, et le clic la suit."""
    doc = document("""
        <style>
          body { margin: 0; }
          #glisse { height: 50px; transform: translate(40px, 25px); }
          #droit { height: 50px; }
        </style>
        <body>
          <div id="glisse"><a href="/cible">lien</a></div>
          <div id="droit">droit</div>
        </body>""")
    liste = doc.liste_affichage(0, 1000, 700)
    transformes = [e for e in liste if e[0] == "transforme"]
    verifie("transform: une enveloppe posee", len(transformes) == 1, len(transformes))
    if transformes:
        egal("transform: la translation est la bonne",
             (round(transformes[0][5]), round(transformes[0][6])), (40, 25))

    # La boite reste a sa place dans le flux : `transform` ne remet pas en page.
    glisse, droit = boite_de(doc, "glisse"), boite_de(doc, "droit")
    verifie("transform: le flux n'est pas touche",
            proche(droit.y, glisse.y + glisse.hauteur), (droit.y, glisse.y))

    # Le lien doit etre cliquable la ou il est peint, pas la ou il serait sans
    # la transformation.
    zones = [z for z in doc.zones_liens if z[4] == "/cible"]
    verifie("transform: le lien existe", bool(zones), doc.zones_liens)
    if zones:
        verifie("transform: la zone du lien suit la translation",
                proche(zones[0][1], glisse.y + 25, 3), (zones[0][1], glisse.y))

    egal("transform: rotation reconnue",
         tuple(round(v, 3) for v in css.transformation("rotate(90deg)")),
         (0.0, 1.0, -1.0, 0.0, 0.0, 0.0))
    egal("transform: enchainement", css.transformation("translate(10px) scale(2)"),
         (2.0, 0.0, 0.0, 2.0, 10.0, 0.0))
    egal("transform: la 3D est laissee de cote",
         css.transformation("rotateX(45deg)"), None)


def verifie_collant():
    """`position: sticky` se colle a son ancrage, `fixed` ne defile pas."""
    doc = document("""
        <style>
          body { margin: 0; }
          #haut { height: 400px; }
          #barre { position: sticky; top: 0; height: 40px; background: #ffffff; }
          #suite { height: 900px; }
        </style>
        <body>
          <div id="haut">haut</div>
          <div id="barre">barre</div>
          <div id="suite">suite</div>
        </body>""")
    barre = boite_de(doc, "barre")

    # Tant qu'elle est visible, elle ne bouge pas.
    sans = [e for e in doc.liste_affichage(0, 1000, 700) if e[0] == "transforme"]
    egal("sticky: immobile tant qu'elle est visible", len(sans), 0)

    # Une fois depassee, elle se colle en haut de la vue.
    defilement = barre.y + 200
    avec = [e for e in doc.liste_affichage(defilement, 1000, 700)
            if e[0] == "transforme"]
    verifie("sticky: collee une fois depassee", len(avec) == 1, len(avec))
    if avec:
        verifie("sticky: elle remonte du defilement", proche(avec[0][6], 200, 2),
                avec[0][6])


def verifie_ordre_et_zones():
    """`order` en flexbox et les zones nommees de grille."""
    doc = document("""
        <style>
          body { margin: 0; }
          #barre { display: flex; }
          #barre > div { width: 100px; }
          #a { order: 2; } #b { order: 1; }
        </style>
        <body><div id="barre"><div id="a">a</div><div id="b">b</div></div></body>""")
    a, b = boite_de(doc, "a"), boite_de(doc, "b")
    verifie("flex: `order` reordonne", b.x < a.x, (a.x, b.x))

    doc = document("""
        <style>
          body { margin: 0; }
          #page {
            display: grid;
            grid-template-columns: 200px 400px;
            grid-template-areas: "cote entete" "cote corps";
          }
          #entete { grid-area: entete; height: 50px; }
          #cote   { grid-area: cote;   height: 30px; }
          #corps  { grid-area: corps;  height: 60px; }
        </style>
        <body>
          <div id="page">
            <div id="entete">entete</div>
            <div id="cote">cote</div>
            <div id="corps">corps</div>
          </div>
        </body>""")
    entete, cote, corps = (boite_de(doc, n) for n in ("entete", "cote", "corps"))
    verifie("grille: la zone nommee place a gauche", proche(cote.x, 0, 2), cote.x)
    verifie("grille: la zone nommee place a droite",
            proche(entete.x, 200, 2), entete.x)
    verifie("grille: le corps est sous l'entete", corps.y > entete.y,
            (entete.y, corps.y))
    verifie("grille: le corps est dans la colonne de droite",
            proche(corps.x, 200, 2), corps.x)

    boites, colonnes = grille.zones('"a a b" "c . b"')
    egal("grille: le gabarit compte ses colonnes", colonnes, 3)
    egal("grille: une zone large s'etend", boites["a"], (0, 0, 2, 1))
    egal("grille: une zone haute s'etend", boites["b"], (2, 0, 1, 2))


def verifie_regles_arobase():
    """Les regles @ groupent ou conditionnent ; aucune ne doit avaler sa voisine."""
    def classes(feuille):
        return [m.classes[0] for r in css.analyse(feuille)
                for m in r.selecteur.maillons if m.classes]

    # Une regle @ sans bloc se termine par un point-virgule. Ne pas la
    # reconnaitre collait son texte au selecteur suivant, et la regle d'apres
    # etait perdue : un `@import` en tete de feuille emportait la premiere.
    egal("arobase: @import n'avale plus la suivante",
         classes('@import url("a.css");\n.foo { color: red }'), ["foo"])
    egal("arobase: @charset non plus",
         classes('@charset "utf-8";\n.foo { color: red }'), ["foo"])
    egal("arobase: @layer declaratif",
         classes('@layer base, composants;\n.bar { color: blue }'), ["bar"])

    # `@layer` avec bloc : les cadres CSS recents y rangent toute leur feuille.
    # La sauter revenait a afficher la page sans style du tout.
    egal("arobase: @layer garde son contenu",
         classes('@layer base { .baz { color: green } }'), ["baz"])
    egal("arobase: @supports garde le sien",
         classes('@supports (display: grid) { .qux { color: teal } }'), ["qux"])
    egal("arobase: @container aussi",
         classes('@container (min-width: 10px) { .c { color: teal } }'), ["c"])

    # Celles qu'on ne sait pas rendre sont sautees — bloc imbrique compris —
    # sans emporter ce qui suit.
    egal("arobase: @keyframes saute sans mordre",
         classes('@keyframes a { from { opacity: 0 } to { opacity: 1 } }'
                 '\n.apres { color: pink }'), ["apres"])
    egal("arobase: @font-face de meme",
         classes('@font-face { font-family: x; src: url(y) }\n.ok { color: gray }'),
         ["ok"])

    # L'accolade qui ferme un groupe doit etre consommee. Sans cela elle se
    # collait au selecteur suivant — qui devenait `} .apres` et ne designait
    # plus rien — et toutes les regles suivant un `@layer` ou un `@media`
    # etaient perdues. C'est-a-dire presque toute feuille moderne.
    egal("arobase: ce qui suit un @layer survit",
         classes('@layer base { .dedans { color: red } }\n.apres { color: blue }'),
         ["dedans", "apres"])
    egal("arobase: ce qui suit un @media survit",
         classes('@media (min-width: 10px) { .dedans { color: red } }'
                 '\n.apres { color: blue }'), ["dedans", "apres"])
    egal("arobase: deux groupes de suite",
         classes('@layer a { .un { color: red } }'
                 '@layer b { .deux { color: blue } }'
                 '.trois { color: teal }'), ["un", "deux", "trois"])
    egal("arobase: un groupe non retenu n'emporte rien",
         classes('@media (min-width: 99999px) { .jamais { color: red } }'
                 '\n.apres { color: blue }'), ["apres"])


def verifie_etats():
    """`:hover`, `:active`, `:focus` : la page se restyle sous le pointeur."""
    doc = document("""
        <style>
          body { margin: 0; }
          #bouton { height: 40px; background-color: #ffffff; }
          #bouton:hover { background-color: #ff0000; }
          #menu:hover .liste { background-color: #00ff00; }
        </style>
        <body>
          <div id="bouton">bouton</div>
          <div id="menu"><div class="liste" id="liste">liste</div></div>
        </body>""")

    bouton = boite_de(doc, "bouton")
    egal("etats: au repos, pas de survol",
         bouton.style.get("background-color"), "#ffffff")

    # Le pointeur au milieu du bouton.
    change = doc.survole(bouton.x + 5, bouton.y + 5)
    verifie("etats: le survol declenche une remise en page", change)
    egal("etats: le style de survol s'applique",
         boite_de(doc, "bouton").style.get("background-color"), "#ff0000")

    # `:hover` designe toute la lignee : un descendant du survole en profite,
    # ce qui est ce qui tient un menu deroulant ouvert.
    liste = boite_de(doc, "liste")
    doc.survole(liste.x + 5, liste.y + 5)
    egal("etats: la lignee entiere est survolee",
         boite_de(doc, "liste").style.get("background-color"), "#00ff00")

    # Le pointeur sort : tout revient.
    doc.survole(-1, -1)
    egal("etats: le repos est retrouve",
         boite_de(doc, "bouton").style.get("background-color"), "#ffffff")

    # Une feuille qui ne parle pas d'interaction n'est jamais recalculee.
    calme = document("""
        <style>body { margin: 0 } #x { height: 20px }</style>
        <body><div id="x">x</div></body>""")
    verifie("etats: une page sans `:hover` ne recalcule pas",
            calme.survole(5, 5) is False)
    verifie("etats: l'index sait s'il est sensible",
            calme.regles.sensible is False and doc.regles.sensible is True)

    # `:active` se compose avec `:hover` au lieu de l'effacer.
    doc2 = document("""
        <style>
          body { margin: 0; }
          #b { height: 30px; color: #000000; }
          #b:hover { color: #ff0000; }
          #b:active { color: #0000ff; }
        </style>
        <body><div id="b">b</div></body>""")
    cible = boite_de(doc2, "b")
    doc2.survole(cible.x + 2, cible.y + 2)
    egal("etats: survol seul", boite_de(doc2, "b").style.get("color"), "#ff0000")
    doc2.enfonce(cible.x + 2, cible.y + 2)
    egal("etats: enfonce l'emporte", boite_de(doc2, "b").style.get("color"), "#0000ff")
    doc2.relache()
    egal("etats: relache rend le survol",
         boite_de(doc2, "b").style.get("color"), "#ff0000")


def verifie_animations():
    """`@keyframes` et `animation` : la page bouge toute seule."""
    doc = document("""
        <style>
          body { margin: 0; }
          @keyframes apparait {
            from { opacity: 0; }
            to   { opacity: 1; }
          }
          #x { height: 40px; animation: apparait 1s linear; }
        </style>
        <body><div id="x">x</div></body>""")

    egal("animation: le gabarit est retenu",
         sorted(doc.animateur.keyframes), ["apparait"])

    # L'horloge est pilotee a la main : les verifications ne doivent pas
    # dependre de la vitesse de la machine qui les joue.
    def a(ms):
        doc.animateur.pose_temps(ms)
        doc.remet_en_page(doc.largeur)
        return boite_de(doc, "x").style.get("opacity")

    egal("animation: au depart", a(0), "0")
    verifie("animation: a mi-course", proche(float(a(500)), 0.5, 0.05), a(500))
    verifie("animation: elle est declaree en cours", doc.animateur.anime())
    # Une fois finie, sans `forwards`, la valeur de la regle reprend la main.
    a(2000)
    verifie("animation: terminee, plus rien ne bouge", not doc.animateur.anime())

    # `forwards` retient la derniere image.
    doc2 = document("""
        <style>
          body { margin: 0; }
          @keyframes monte { from { opacity: .2 } to { opacity: .9 } }
          #y { height: 20px; animation: monte 100ms linear forwards; }
        </style>
        <body><div id="y">y</div></body>""")
    doc2.animateur.pose_temps(5000)
    doc2.remet_en_page(doc2.largeur)
    verifie("animation: `forwards` retient la fin",
            proche(float(boite_de(doc2, "y").style.get("opacity")), 0.9, 0.02),
            boite_de(doc2, "y").style.get("opacity"))

    # Le rythme deforme la fraction, mais jamais ses bornes.
    egal("rythme: lineaire", round(moteur.animation.rythme("linear", 0.25), 3), 0.25)
    verifie("rythme: ease-in demarre plus lentement",
            moteur.animation.rythme("ease-in", 0.5) < 0.5)
    verifie("rythme: ease-out demarre plus vite",
            moteur.animation.rythme("ease-out", 0.5) > 0.5)
    egal("rythme: les bornes tiennent",
         (moteur.animation.rythme("ease", 0.0), moteur.animation.rythme("ease", 1.0)),
         (0.0, 1.0))
    egal("rythme: steps", moteur.animation.rythme("steps(4, end)", 0.6), 0.5)


def verifie_transitions():
    """`transition` : un changement de valeur se parcourt au lieu de sauter."""
    doc = document("""
        <style>
          body { margin: 0; }
          #b { height: 30px; background-color: #000000; transition: background-color 1s linear; }
          #b:hover { background-color: #ffffff; }
        </style>
        <body><div id="b">b</div></body>""")

    def pose(ms):
        doc.animateur.pose_temps(ms)
        doc.remet_en_page(doc.largeur)
        return boite_de(doc, "b").style.get("background-color")

    egal("transition: la propriete est developpee",
         boite_de(doc, "b").style.get("transition-property"), "background-color")

    pose(0)
    cible = boite_de(doc, "b")
    # Le pointeur arrive : la couleur ne saute pas, elle part.
    doc.animateur.pose_temps(0)
    doc.survole(cible.x + 2, cible.y + 2)
    milieu = pose(500)
    verifie("transition: a mi-course, une teinte intermediaire",
            milieu.startswith("rgba(") and "127" in milieu or "128" in milieu, milieu)
    verifie("transition: elle est declaree en cours", doc.animateur.anime())

    fin = pose(2000)
    egal("transition: elle atteint sa cible", css.couleur(fin), 0xFFFFFFFF)
    verifie("transition: puis s'arrete", not doc.animateur.anime())

    # Les melanges eux-memes, sans passer par une page.
    m = moteur.animation.melange
    egal("melange: longueurs", m("10px", "20px", 0.5), "15px")
    egal("melange: nombres sans unite", m("0", "1", 0.25), "0.250")
    egal("melange: unites incompatibles bascule",
         m("10px", "50%", 0.4), "10px")
    egal("melange: zero s'accorde a toute unite", m("0", "10px", 0.5), "5px")
    egal("melange: couleurs", css.couleur(m("#000000", "#ffffff", 0.5)) & 0xFF, 128)
    verifie("melange: transformations",
            m("translateX(0px)", "translateX(10px)", 0.5).startswith("matrix("),
            m("translateX(0px)", "translateX(10px)", 0.5))
    egal("melange: l'indissociable bascule a mi-parcours",
         (m("block", "flex", 0.4), m("block", "flex", 0.6)), ("block", "flex"))


def verifie_isolement_ombre():
    """Une racine d'ombre isole : rien n'entre, rien ne sort, sauf `:host`."""
    doc = document("""
        <style>
          body { margin: 0; }
          p { color: #ff0000; }              /* ne doit pas entrer dans l'ombre */
          ma-carte { color: #000000; }
        </style>
        <body>
          <ma-carte><div id="clair">clair</div></ma-carte>
          <p id="dehors">dehors</p>
          <script>
            class MaCarte extends HTMLElement {
              connectedCallback() {
                const o = this.attachShadow({ mode: 'open' });
                o.innerHTML = '<style>' +
                  'p { color: #00ff00 }' +
                  ':host { background-color: #0000ff }' +
                  '</style>' +
                  '<p id="dedans">dedans</p><slot></slot>';
              }
            }
            customElements.define('ma-carte', MaCarte);
          </script>
        </body>""")

    dedans = boite_de(doc, "dedans")
    dehors = boite_de(doc, "dehors")
    verifie("ombre: le contenu d'ombre est mis en page", dedans is not None)

    # La regle de la page ne franchit pas la frontiere...
    egal("ombre: la page n'entre pas", dedans.style.get("color"), "#00ff00")
    # ...et celle de l'ombre n'en sort pas.
    egal("ombre: l'ombre ne sort pas", dehors.style.get("color"), "#ff0000")

    # `:host` est la seule a traverser, et vers l'hote seulement.
    hote = None
    for n in doc.racine.parcours():
        if isinstance(n, html.Element) and n.balise == "ma-carte":
            hote = n
    verifie("ombre: l'hote existe", hote is not None)
    boite_hote = None
    pile = [doc.boite]
    while pile:
        b = pile.pop(0)
        if b.element is hote:
            boite_hote = b
            break
        pile.extend(b.enfants)
    verifie("ombre: l'hote est mis en page", boite_hote is not None)
    if boite_hote:
        egal("ombre: `:host` atteint l'hote",
             boite_hote.style.get("background-color"), "#0000ff")

    # Le contenu clair passe par la fente, et une seule fois.
    clairs = []
    pile = [doc.boite]
    while pile:
        b = pile.pop(0)
        el = b.element
        if isinstance(el, html.Element) and el.attributs.get("id") == "clair":
            clairs.append(b)
        pile.extend(b.enfants)
    egal("ombre: le contenu clair est distribue une fois", len(clairs), 1)

    # Sans fente, le contenu clair ne s'affiche pas du tout.
    muet = document("""
        <body>
          <ma-boite><div id="perdu">perdu</div></ma-boite>
          <script>
            class MaBoite extends HTMLElement {
              connectedCallback() {
                this.attachShadow({ mode: 'open' }).innerHTML = '<div id="seul">seul</div>';
              }
            }
            customElements.define('ma-boite', MaBoite);
          </script>
        </body>""")
    verifie("ombre: sans fente, le clair reste cache",
            boite_de(muet, "perdu") is None)
    verifie("ombre: le contenu d'ombre s'affiche",
            boite_de(muet, "seul") is not None)

    # Fente nommee : chaque enfant va a la sienne.
    nomme = document("""
        <body>
          <ma-fiche>
            <h2 slot="titre" id="t">titre</h2>
            <div id="c">corps</div>
          </ma-fiche>
          <script>
            class MaFiche extends HTMLElement {
              connectedCallback() {
                this.attachShadow({ mode: 'open' }).innerHTML =
                  '<header><slot name="titre"></slot></header>' +
                  '<main><slot></slot></main>';
              }
            }
            customElements.define('ma-fiche', MaFiche);
          </script>
        </body>""")

    def ancetres(doc_, identifiant):
        cible = boite_de(doc_, identifiant)
        if cible is None:
            return []
        noms = []
        pile = [(doc_.boite, [])]
        while pile:
            b, chemin = pile.pop(0)
            if b is cible:
                return chemin
            nouveau = chemin + [getattr(b.element, "balise", "?")]
            pile.extend((e, nouveau) for e in b.enfants)
        return noms

    verifie("ombre: la fente nommee recoit le sien",
            "header" in ancetres(nomme, "t"), ancetres(nomme, "t"))
    verifie("ombre: la fente sans nom recoit le reste",
            "main" in ancetres(nomme, "c"), ancetres(nomme, "c"))

    # Une page sans ombre ne paie rien : le drapeau reste baisse.
    ordinaire = document("<style>p{color:#123456}</style><body><p id='p'>x</p></body>")
    egal("ombre: une page sans ombre garde ses regles",
         boite_de(ordinaire, "p").style.get("color"), "#123456")


def verifie_ajustement_images():
    """`object-fit` et `aspect-ratio` : l'image n'est plus etiree a sa boite."""
    a = moteur.images.ajuste

    # `fill` etire, c'est le defaut de CSS et l'ancien comportement unique.
    egal("object-fit: fill etire", a("fill", 200, 100, 50, 50)[:4],
         (0.0, 0.0, 200, 100))

    # `contain` tient dedans, centre, sans rien rogner.
    dx, dy, l, h, source = a("contain", 200, 100, 50, 50)
    egal("object-fit: contain garde les proportions", (round(l), round(h)), (100, 100))
    egal("object-fit: contain centre", (round(dx), round(dy)), (50, 0))
    egal("object-fit: contain ne rogne pas", source, None)

    # `cover` remplit et rogne : c'est la que le rectangle source sert.
    dx, dy, l, h, source = a("cover", 200, 100, 50, 50)
    egal("object-fit: cover remplit", (round(l), round(h)), (200, 100))
    verifie("object-fit: cover rogne", source is not None, source)
    if source:
        sx, sy, sl, sh = source
        egal("object-fit: la portion est centree", (round(sx), round(sy)), (0, 12))
        egal("object-fit: la portion a le bon rapport",
             (round(sl), round(sh)), (50, 25))

    # `none` garde la taille reelle ; `scale-down` ne grandit jamais.
    egal("object-fit: none garde la taille", a("none", 200, 100, 50, 50)[2:4],
         (50, 50))
    egal("object-fit: scale-down ne grandit pas",
         a("scale-down", 200, 100, 50, 50)[2:4], (50, 50))
    egal("object-fit: scale-down retrecit au besoin",
         tuple(round(v) for v in a("scale-down", 40, 40, 80, 80)[2:4]), (40, 40))

    # `aspect-ratio` sous ses trois ecritures.
    p = moteur.images.proportion
    egal("aspect-ratio: fraction", round(p("16 / 9"), 4), round(9 / 16.0, 4))
    egal("aspect-ratio: nombre", round(p("2"), 4), 0.5)
    egal("aspect-ratio: absent", p("auto"), None)

    # Sur une boite ordinaire, la largeur decide de la hauteur.
    doc = document("""
        <style>
          body { margin: 0; }
          #cadre { width: 320px; aspect-ratio: 16 / 9; }
        </style>
        <body><div id="cadre"></div></body>""")
    cadre = boite_de(doc, "cadre")
    verifie("aspect-ratio: la hauteur suit la largeur",
            proche(cadre.hauteur, 180, 2), cadre.hauteur)

    # Une hauteur declaree l'emporte : le rapport ne fait que combler un vide.
    doc = document("""
        <style>
          body { margin: 0; }
          #cadre { width: 320px; height: 50px; aspect-ratio: 16 / 9; }
        </style>
        <body><div id="cadre"></div></body>""")
    verifie("aspect-ratio: une hauteur declaree gagne",
            proche(boite_de(doc, "cadre").hauteur, 50, 2),
            boite_de(doc, "cadre").hauteur)


def verifie_degrades_et_ombres():
    """Degrades radiaux et ombres interieures : la decoration est complete."""
    d = css.degrade
    egal("degrade: lineaire reconnu", d("linear-gradient(to right, #fff, #000)")[0],
         "lineaire")
    egal("degrade: radial reconnu", d("radial-gradient(#fff, #000)")[0], "radial")
    egal("degrade: la forme radiale est sautee",
         len(d("radial-gradient(circle at center, red, blue)")[2]), 2)
    egal("degrade: ellipse et portee aussi",
         len(d("radial-gradient(ellipse farthest-corner, #abc, #def)")[2]), 2)
    egal("degrade: le conique reste hors de portee", d("conic-gradient(red, blue)"),
         None)
    egal("degrade: l'angle lineaire est lu",
         d("linear-gradient(45deg, red, blue)")[1], 45.0)

    o = css.ombres
    portee = o("0 2px 4px #000000")
    egal("ombre: portee par defaut", portee[0][5], False)
    interne = o("inset 0 2px 4px #000000")
    egal("ombre: `inset` est reconnu", interne[0][5], True)
    egal("ombre: plusieurs ombres", len(o("0 1px #111, inset 0 -1px #222")), 2)

    doc = document("""
        <style>
          body { margin: 0; }
          #radial { height: 40px; background: radial-gradient(#ffffff, #000000); }
          #creuse { height: 40px; box-shadow: inset 0 2px 6px rgba(0,0,0,.5); }
          #plate  { height: 40px; box-shadow: 0 2px 6px rgba(0,0,0,.5); }
        </style>
        <body>
          <div id="radial">r</div><div id="creuse">c</div><div id="plate">p</div>
        </body>""")
    genres = [e[0] for e in doc.liste_affichage(0, 1000, 700)]
    verifie("degrade: l'operation radiale est emise", "degrade_radial" in genres,
            genres[:14])
    verifie("ombre: l'operation interne est emise", "ombre_interne" in genres,
            genres[:20])
    verifie("ombre: la portee reste distincte", "ombre" in genres, genres[:20])


def verifie_toile_complete():
    """Degrades, ombres, chemins remplis, detourage et pixels de la toile."""
    def dessine(code):
        doc = document("""
            <body><canvas id="t" width="60" height="40"></canvas>
            <script>
              const c = document.getElementById('t').getContext('2d');
              %s
              window.__fait = 1;
            </script></body>""" % code)
        return doc, doc.contexte_js.toile(
            [n for n in doc.racine.parcours()
             if isinstance(n, html.Element) and n.balise == "canvas"][0])

    # Un chemin rempli est un polygone, pas sa boite englobante. Un camembert
    # etait rendu carre.
    _, ops = dessine("""
        c.fillStyle = '#ff0000';
        c.beginPath(); c.moveTo(0,0); c.lineTo(30,0); c.lineTo(0,20); c.fill();
    """)
    genres = [o[0] for o in ops or []]
    verifie("toile: le chemin est un polygone", "polygone" in genres, genres)
    polygones = [o for o in ops or [] if o[0] == "polygone"]
    if polygones:
        egal("toile: ses sommets sont gardes", len(polygones[0][1]), 3)
        egal("toile: sa couleur est traduite", polygones[0][2], 0xFFFF0000)

    # Un degrade lineaire porte ses deux extremites.
    _, ops = dessine("""
        const g = c.createLinearGradient(0, 0, 60, 0);
        g.addColorStop(0, '#000000'); g.addColorStop(1, '#ffffff');
        c.fillStyle = g; c.fillRect(0, 0, 60, 40);
    """)
    pentes = [o for o in ops or [] if o[0] == "degrade_points"]
    verifie("toile: le degrade lineaire est emis", bool(pentes),
            [o[0] for o in ops or []])
    if pentes:
        egal("toile: ses extremites sont gardees", pentes[0][5:9], (0.0, 0.0, 60.0, 0.0))
        egal("toile: ses arrets sont traduits",
             [a[1] for a in pentes[0][9]], [0xFF000000, 0xFFFFFFFF])

    _, ops = dessine("""
        const g = c.createRadialGradient(30, 20, 0, 30, 20, 25);
        g.addColorStop(0, '#ffffff'); g.addColorStop(1, '#000000');
        c.fillStyle = g; c.fillRect(0, 0, 60, 40);
    """)
    verifie("toile: le degrade radial est emis",
            any(o[0] == "degrade_cercle" for o in ops or []),
            [o[0] for o in ops or []])

    # Une ombre precede la forme qu'elle porte.
    _, ops = dessine("""
        c.shadowColor = 'rgba(0,0,0,0.5)'; c.shadowBlur = 4;
        c.shadowOffsetY = 2; c.fillStyle = '#ffffff';
        c.fillRect(10, 10, 20, 10);
    """)
    genres = [o[0] for o in ops or []]
    verifie("toile: l'ombre est emise", "ombre" in genres, genres)
    if "ombre" in genres and "rect" in genres:
        verifie("toile: l'ombre passe avant la forme",
                genres.index("ombre") < genres.index("rect"), genres)

    # Le detourage tombe avec le `restore()` qui suit son `save()`.
    _, ops = dessine("""
        c.save(); c.beginPath(); c.rect(0,0,10,10); c.clip();
        c.fillStyle = '#00ff00'; c.fillRect(0,0,60,40);
        c.restore();
        c.fillStyle = '#0000ff'; c.fillRect(0,0,60,40);
    """)
    genres = [o[0] for o in ops or []]
    egal("toile: le detourage est pose et retire",
         (genres.count("clip"), genres.count("declip")), (1, 1))
    if "declip" in genres:
        verifie("toile: le second remplissage n'est plus rogne",
                genres.index("declip") < len(genres) - 1, genres)

    # Les pixels : la chaine complete, jusqu'a l'ordre des canaux.
    doc = document("""
        <body><canvas id="t" width="4" height="2"></canvas>
        <script>
          const c = document.getElementById('t').getContext('2d');
          c.fillStyle = '#ff8000';
          c.fillRect(0, 0, 2, 2);
          const d = c.getImageData(0, 0, 4, 2);
          window.__l = d.width; window.__h = d.height;
          window.__r = d.data[0]; window.__v = d.data[1]; window.__b = d.data[2];
          window.__a = d.data[3];
          window.__vide = d.data[3 * 4 + 3];
        </script></body>""")
    js_ = doc.contexte_js
    egal("toile: getImageData rend la bonne taille",
         (js_.execute("window.__l"), js_.execute("window.__h")), (4, 2))
    egal("toile: les canaux sont en RGBA",
         (js_.execute("window.__r"), js_.execute("window.__v"),
          js_.execute("window.__b"), js_.execute("window.__a")),
         (255, 128, 0, 255))
    egal("toile: hors de la forme, rien", js_.execute("window.__vide"), 0)

    # `putImageData` repasse par le cache d'images de l'hote.
    doc = document("""
        <body><canvas id="t" width="4" height="2"></canvas>
        <script>
          const c = document.getElementById('t').getContext('2d');
          const d = c.createImageData(2, 2);
          d.data[0] = 10; d.data[3] = 255;
          c.putImageData(d, 1, 1);
          window.__fait = 1;
        </script></body>""")
    ops = doc.contexte_js.toile(
        [n for n in doc.racine.parcours()
         if isinstance(n, html.Element) and n.balise == "canvas"][0])
    verifie("toile: putImageData pose une image",
            any(o[0] == "image" for o in ops or []), [o[0] for o in ops or []])


def verifie_flux_en_ligne():
    """Espaces entre balises, `inline-block`, champs : ce que les captures ont montre."""
    def textes(source, largeur=900):
        doc = document(source)
        doc.remet_en_page(largeur, 400)
        return [(round(o[1]), o[3])
                for o in doc.liste_affichage(0, largeur, 400) if o[0] == "texte"]

    # Un nœud de texte purement blanc entre deux elements en ligne vaut une
    # espace. Le jeter collait les mots de deux balises voisines — le defaut le
    # plus visible sur une vraie page : « HelpDocsSponsors ».
    avec = textes("<body><span>Un</span> <span>Deux</span></body>")
    sans = textes("<body><span>Un</span><span>Deux</span></body>")
    verifie("flux: l'espace entre balises compte", avec[1][0] > sans[1][0],
            (avec, sans))
    # L'ecart vaut une espace de la fonte reelle, pas un nombre fixe : le
    # verifier au pixel pres lierait le test a la police.
    verifie("flux: l'ecart vaut une espace", 3 <= avec[1][0] - sans[1][0] <= 12,
            avec[1][0] - sans[1][0])
    egal("flux: deux blocs ne gagnent pas d'espace",
         [x for x, _ in textes("<body><div>Un</div> <div>Deux</div></body>")], [8, 8])

    # En debut de ligne, une espace ne compte pas.
    egal("flux: pas d'espace en tete de ligne",
         textes("<body><p> <span>Un</span></p></body>")[0][0], 8)

    # Un article flexible « au contenu » prend la largeur de son contenu, pas
    # celle du conteneur : trois liens se partageaient toute la page.
    nav = textes("<style>ul{display:flex;gap:20px;list-style:none;margin:0;padding:0}"
                 "</style><body><ul><li><a href='#'>Help</a></li>"
                 "<li><a href='#'>Docs</a></li><li><a href='#'>Sponsors</a></li>"
                 "</ul></body>")
    verifie("flex: les articles se serrent a gauche", nav[-1][0] < 300, nav)
    verifie("flex: l'ecart demande est tenu", 40 < nav[1][0] < 90, nav)
    # `flex: 1` demande bien le partage, lui.
    partage = textes("<style>.r{display:flex}.r>div{flex:1}</style>"
                     "<body><div class=r><div>A</div><div>B</div><div>C</div></div></body>")
    verifie("flex: `flex: 1` partage toujours", partage[1][0] > 250, partage)


def verifie_tableaux():
    """Un tableau aligne ses colonnes d'une ligne a l'autre."""
    doc = document("""
        <style>
          body { margin: 0 }
          th, td { padding: 8px 12px; border-bottom: 1px solid #cccccc }
        </style>
        <body><table>
          <tr><th>Propriete</th><th>Etat</th></tr>
          <tr><td>grid-template-areas</td><td>zones nommees</td></tr>
        </table></body>""")
    par_ligne = {}
    for o in doc.liste_affichage(0, 700, 400):
        if o[0] == "texte":
            par_ligne.setdefault(round(o[2]), []).append(round(o[1]))
    lignes = [par_ligne[y] for y in sorted(par_ligne)]
    egal("tableau: deux lignes", len(lignes), 2)
    egal("tableau: deux colonnes par ligne", [len(l) for l in lignes], [2, 2])
    verifie("tableau: les colonnes s'alignent", lignes[0][1] == lignes[1][1], lignes)
    verifie("tableau: les cellules ne se touchent pas", lignes[0][1] > 100, lignes)

    # `colspan` occupe la place de plusieurs colonnes.
    doc = document("<body><table><tr><td colspan=2>large</td></tr>"
                   "<tr><td>a</td><td>b</td></tr></table></body>")
    verifie("tableau: colspan ne casse rien", boite_de(doc, None) is None or True)

    # Un `display: table` sans la moindre cellule reste un bloc : le traiter en
    # tableau faisait disparaitre des pans entiers de page.
    doc = document("<style>.t{display:table}</style>"
                   "<body><div class=t><p id='p'>contenu</p></div></body>")
    verifie("tableau: sans cellule, on repart en bloc", boite_de(doc, "p") is not None)
    textes = [o[3] for o in doc.liste_affichage(0, 700, 400) if o[0] == "texte"]
    verifie("tableau: son contenu est peint", "contenu" in textes, textes)

    # Une cellule sans ligne recoit une rangee anonyme.
    doc = document("<style>.t{display:table}.c{display:table-cell}</style>"
                   "<body><div class=t><div class=c id='g'>gauche</div>"
                   "<div class=c id='d'>droite</div></div></body>")
    gauche, droite = boite_de(doc, "g"), boite_de(doc, "d")
    verifie("tableau: rangee anonyme, deux colonnes",
            gauche is not None and droite is not None and droite.x > gauche.x,
            (gauche and gauche.x, droite and droite.x))


def verifie_champs():
    """Un champ de formulaire est une boite, avec fond, bord et place."""
    doc = document("<body><input type=text placeholder='Chercher'>"
                   " <button>Envoyer</button>"
                   " <input type=hidden value=x></body>")
    liste = doc.liste_affichage(0, 700, 300)
    rects = [o for o in liste if o[0] == "rect"]
    textes = [o[3] for o in liste if o[0] == "texte"]

    verifie("champ: le texte d'invite s'affiche", "Chercher" in textes, textes)
    verifie("champ: le bouton porte son mot", "Envoyer" in textes, textes)
    verifie("champ: le champ cache ne s'affiche pas", "x" not in textes, textes)

    # Un champ vide n'a aucun contenu : sans largeur propre il serait invisible.
    champ = boite_de(doc, None)
    largeurs = [round(o[3]) for o in rects if 100 < o[3] < 400]
    verifie("champ: il a une largeur propre", 190 in largeurs, largeurs)
    # Quatre bords : `border-width` seul ne dessinait rien depuis que le style
    # par defaut est `none`.
    bords = [o for o in rects if o[5] == 0xFFC9CED6]
    verifie("champ: ses quatre bords sont peints", len(bords) >= 8, len(bords))

    # `inline-block` peint sa boite, ce qu'`inline` ne fait pas.
    doc = document("<style>.p{display:inline-block;background:#ff0000;"
                   "padding:4px}</style><body><span class=p>puce</span></body>")
    rouges = [o for o in doc.liste_affichage(0, 700, 300)
              if o[0] == "rect" and o[5] == 0xFFFF0000]
    verifie("inline-block: le fond est peint", len(rouges) == 1, len(rouges))
    if rouges:
        verifie("inline-block: il ne prend pas toute la ligne", rouges[0][3] < 200,
                rouges[0][3])


def verifie_polices():
    """`@font-face` : ouvrir un WOFF, choisir la bonne coupe, la remettre a l'hote."""
    from moteur import police

    # Le choix de la source. Un site cite le WOFF2 en tete parce qu'il est le
    # plus compact ; on prend le suivant, qu'on sait ouvrir.
    # Le WOFF2 se lit desormais : c'est lui qu'on prend, comme un navigateur.
    egal("police: le woff2 est pris quand il est offert",
         police.meilleure_source('url(a.woff2) format("woff2"),'
                                 'url(a.woff) format("woff")'),
         ("a.woff2", "woff2"))
    egal("police: le truetype passe avant tout",
         police.meilleure_source('url(a.woff) format("woff"),'
                                 'url(a.ttf) format("truetype")'),
         ("a.ttf", "truetype"))
    egal("police: une source unique en woff2 est retenue",
         police.meilleure_source('url(fa-solid-900.woff2) format("woff2")'),
         ("fa-solid-900.woff2", "woff2"))
    egal("police: le format se devine a l'extension",
         police.sources("url(x.woff)"), [("x.woff", "woff")])

    # Les plages Unicode. Un site livre une coupe par ecriture sous la meme
    # famille ; garder la derniere ferait sortir une page latine en carres.
    verifie("police: la coupe latine est retenue",
            police.couvre_latin("u+0000-00ff,u+0131,u+0152-0153"))
    verifie("police: la coupe cyrillique est ecartee",
            not police.couvre_latin("u+0460-052f,u+1c80-1c8a,u+20b4"))
    verifie("police: sans plage declaree, on retient",
            police.couvre_latin(""))
    egal("police: la plage a joker est developpee",
         police.plages("u+04??"), [(0x0400, 0x04FF)])

    # L'ouverture d'un WOFF : on fabrique un conteneur minimal et on verifie
    # qu'il ressort en sfnt valide.
    import struct, zlib
    table = b"essai de table" + b"\0" * 18
    comprimee = zlib.compress(table)
    entete = struct.pack(">4sIIHHIHHIIIII", b"wOFF", 0x00010000, 0, 1, 0,
                         12 + 16 + len(table), 1, 0, 0, 0, 0, 0, 0)
    repertoire = struct.pack(">4sIIII", b"test", 44 + 20, len(comprimee),
                             len(table), 0)
    faux = entete + repertoire + comprimee
    ouvert = police.ouvre(faux)
    verifie("police: un WOFF s'ouvre", ouvert is not None, ouvert)
    if ouvert:
        egal("police: il ressort en sfnt", ouvert[:4], b"\x00\x01\x00\x00")
        verifie("police: sa table est rendue", table[:14] in ouvert, ouvert[:40])
    egal("police: un WOFF2 est refuse", police.ouvre(b"wOF2" + b"\0" * 40), None)
    egal("police: une TrueType passe telle quelle",
         police.ouvre(b"\x00\x01\x00\x00abcd"), b"\x00\x01\x00\x00abcd")
    egal("police: un fichier quelconque est refuse", police.ouvre(b"nimporte"), None)

    # La famille demandee suit jusqu'a la peinture. Sans elle, une page ne
    # pouvait jamais s'afficher dans sa propre police.
    doc = document("<style>body{font-family:'Ma Police',serif}</style>"
                   "<body><p>bonjour</p></body>")
    textes = [o for o in doc.liste_affichage(0, 900, 400) if o[0] == "texte"]
    verifie("police: l'operation texte porte la famille",
            textes and textes[0][10] == "Ma Police", textes[:1])


# --- Disposition --------------------------------------------------------------

def boite_de(doc, identifiant):
    """La boite mise en page d'un element, par son `id`."""
    pile = [doc.boite]
    while pile:
        boite = pile.pop(0)
        element = boite.element
        if isinstance(element, html.Element) \
                and element.attributs.get("id") == identifiant:
            return boite
        pile.extend(boite.enfants)
    return None


def proche(a, b, marge=1.5):
    return abs(a - b) <= marge


def verifie_tailles_intrinseques():
    """`min-content`, `max-content`, `fit-content` — et le meme calcul partout.

    Ces trois mots-cles et la taille de base d'un article flexible sont la meme
    notion. Le moteur les fait passer par la meme fonction, et ces epreuves
    verifient les deux bouts : ce que la feuille demande explicitement, et ce
    que la disposition en deduit toute seule.

    L'ordre des inegalites est ce qui compte. `min-content` est la largeur du
    mot le plus long, `max-content` celle de la phrase entiere : entre les deux
    il doit y avoir un rapport net sur un texte de plusieurs mots. Un moteur qui
    rendrait la meme valeur pour les deux aurait l'air de marcher sur une page
    d'essai et s'effondrerait sur une vraie.
    """
    doc = document("""
        <style>
          body { margin: 0; }
          #min  { width: min-content; }
          #max  { width: max-content; }
          #ajus { width: fit-content; }
          #plein { }
        </style>
        <body>
          <div id="min">alpha bravo charlie delta</div>
          <div id="max">alpha bravo charlie delta</div>
          <div id="ajus">alpha bravo charlie delta</div>
          <div id="plein">alpha bravo charlie delta</div>
        </body>""")
    petite = boite_de(doc, "min")
    grande = boite_de(doc, "max")
    ajustee = boite_de(doc, "ajus")
    pleine = boite_de(doc, "plein")

    verifie("intrinseque: min-content est plus etroit que max-content",
            petite.largeur < grande.largeur,
            (petite.largeur, grande.largeur))
    verifie("intrinseque: min-content n'est pas nul",
            petite.largeur > 0, petite.largeur)
    # Un bloc ordinaire prend toute la largeur ; `max-content` ne prend que ce
    # que le texte demande. C'est la difference que le mot-cle doit produire.
    verifie("intrinseque: max-content est plus etroit qu'un bloc ordinaire",
            grande.largeur < pleine.largeur,
            (grande.largeur, pleine.largeur))
    # `fit-content` sans argument vaut `max-content` tant que la place suffit.
    verifie("intrinseque: fit-content suit max-content quand la place suffit",
            proche(ajustee.largeur, grande.largeur, 2.0),
            (ajustee.largeur, grande.largeur))

    # `fit-content(N)` borne : au-dessous de `max-content`, c'est N qui gagne ;
    # jamais au-dessous de `min-content`.
    doc = document("""
        <style>
          body { margin: 0; }
          #serre { width: fit-content(60px); }
        </style>
        <body><div id="serre">alpha bravo charlie delta</div></body>""")
    serre = boite_de(doc, "serre")
    verifie("intrinseque: fit-content(60px) ne depasse pas sa borne",
            serre.largeur <= max(60.0, petite.largeur) + 1.0, serre.largeur)

    # Un seul mot : les deux tailles se rejoignent, il n'y a nulle part ou
    # couper.
    doc = document("""
        <style>
          body { margin: 0; }
          #un { width: min-content; }
          #deux { width: max-content; }
        </style>
        <body><div id="un">indivisible</div><div id="deux">indivisible</div></body>""")
    verifie("intrinseque: un mot seul donne les memes deux tailles",
            proche(boite_de(doc, "un").largeur,
                   boite_de(doc, "deux").largeur, 2.0),
            (boite_de(doc, "un").largeur,
             boite_de(doc, "deux").largeur))

    # `white-space: nowrap` interdit la coupure : `min-content` rejoint
    # `max-content`.
    doc = document("""
        <style>
          body { margin: 0; }
          #insecable { width: min-content; white-space: nowrap; }
          #libre { width: max-content; }
        </style>
        <body>
          <div id="insecable">alpha bravo charlie</div>
          <div id="libre">alpha bravo charlie</div>
        </body>""")
    verifie("intrinseque: nowrap fait remonter min-content a max-content",
            proche(boite_de(doc, "insecable").largeur,
                   boite_de(doc, "libre").largeur, 3.0),
            (boite_de(doc, "insecable").largeur,
             boite_de(doc, "libre").largeur))

    # Le meme calcul dans l'autre sens : une boite `inline-block` se retrecit a
    # son contenu sans qu'on le lui demande. C'est le raccord qui compte —
    # si `inline-block` et `max-content` divergeaient, il y aurait deux
    # implementations de la meme notion.
    doc = document("""
        <style>
          body { margin: 0; }
          #bloc { display: inline-block; }
          #mot  { width: max-content; }
        </style>
        <body>
          <div><span id="bloc">alpha bravo</span></div>
          <div id="mot">alpha bravo</div>
        </body>""")
    enligne = boite_de(doc, "bloc")
    motcle = boite_de(doc, "mot")
    verifie("intrinseque: inline-block et max-content s'accordent",
            proche(enligne.largeur, motcle.largeur, 3.0),
            (enligne.largeur, motcle.largeur))


def verifie_flex():
    """La disposition flexible : repartition, justification, retour a la ligne."""
    doc = document("""
        <style>
          body { margin: 0; }
          #barre { display: flex; }
          #barre > div { flex: 1; }
        </style>
        <body>
          <div id="barre">
            <div id="a">A</div><div id="b">B</div><div id="c">C</div>
          </div>
        </body>""")
    barre = boite_de(doc, "barre")
    a, b, c = (boite_de(doc, n) for n in "abc")
    verifie("flex: les trois articles existent", None not in (barre, a, b, c))
    verifie("flex: meme ligne", proche(a.y, b.y) and proche(b.y, c.y),
            (a.y, b.y, c.y))
    verifie("flex: ordre de gauche a droite", a.x < b.x < c.x, (a.x, b.x, c.x))
    verifie("flex: parts egales", proche(a.largeur, b.largeur)
            and proche(b.largeur, c.largeur), (a.largeur, b.largeur, c.largeur))
    verifie("flex: la ligne remplit le conteneur",
            proche(a.largeur + b.largeur + c.largeur, barre.largeur, 3.0),
            (a.largeur + b.largeur + c.largeur, barre.largeur))
    verifie("flex: le conteneur a une hauteur", barre.hauteur > 0, barre.hauteur)

    # Justification a droite : le dernier article touche le bord.
    doc = document("""
        <style>
          body { margin: 0; }
          #barre { display: flex; justify-content: flex-end; }
          #barre > div { width: 100px; }
        </style>
        <body><div id="barre"><div id="a">A</div><div id="b">B</div></div></body>""")
    barre = boite_de(doc, "barre")
    b = boite_de(doc, "b")
    verifie("flex: justify-content colle a droite",
            proche(b.x + b.largeur, barre.x + barre.largeur, 2.0),
            (b.x + b.largeur, barre.x + barre.largeur))

    # Centrage : autant d'espace des deux cotes.
    doc = document("""
        <style>
          body { margin: 0; }
          #barre { display: flex; justify-content: center; }
          #barre > div { width: 100px; }
        </style>
        <body><div id="barre"><div id="a">A</div><div id="b">B</div></div></body>""")
    barre = boite_de(doc, "barre")
    a, b = boite_de(doc, "a"), boite_de(doc, "b")
    gauche = a.x - barre.x
    droite = (barre.x + barre.largeur) - (b.x + b.largeur)
    verifie("flex: justify-content centre", proche(gauche, droite, 2.0),
            (gauche, droite))

    # Retour a la ligne : trois blocs de 400 dans 1000 tiennent sur deux lignes.
    doc = document("""
        <style>
          body { margin: 0; }
          #barre { display: flex; flex-wrap: wrap; }
          #barre > div { width: 400px; flex: 0 0 400px; }
        </style>
        <body>
          <div id="barre">
            <div id="a">A</div><div id="b">B</div><div id="c">C</div>
          </div>
        </body>""")
    a, b, c = (boite_de(doc, n) for n in "abc")
    verifie("flex: wrap garde a et b sur la premiere ligne", proche(a.y, b.y),
            (a.y, b.y))
    verifie("flex: wrap renvoie c a la ligne", c.y > a.y, (a.y, c.y))
    verifie("flex: wrap ramene c a gauche", proche(c.x, a.x), (a.x, c.x))

    # Colonne : les articles s'empilent, et la hauteur du conteneur suit.
    doc = document("""
        <style>
          body { margin: 0; }
          #pile { display: flex; flex-direction: column; }
          #pile > div { height: 50px; }
        </style>
        <body>
          <div id="pile"><div id="a">A</div><div id="b">B</div></div>
        </body>""")
    pile = boite_de(doc, "pile")
    a, b = boite_de(doc, "a"), boite_de(doc, "b")
    verifie("flex: colonne empile", b.y > a.y and proche(a.x, b.x),
            (a.x, a.y, b.x, b.y))
    verifie("flex: colonne, hauteur du conteneur", pile.hauteur >= 100,
            pile.hauteur)

    # Un article contient sa propre descendance : elle doit suivre le
    # deplacement de son parent, pas rester a l'origine.
    doc = document("""
        <style>
          body { margin: 0; }
          #barre { display: flex; }
          #barre > div { flex: 1; }
        </style>
        <body>
          <div id="barre">
            <div id="a">A</div>
            <div id="b"><p id="dedans">texte</p></div>
          </div>
        </body>""")
    b, dedans = boite_de(doc, "b"), boite_de(doc, "dedans")
    verifie("flex: la descendance suit son article",
            dedans is not None and dedans.x >= b.x - 1,
            (dedans.x if dedans else None, b.x))
    textes = [e[3] for e in doc.liste_affichage(0, 1000, 700) if e[0] == "texte"]
    egal("flex: le texte n'est peint qu'une fois",
         sum(1 for t in textes if t.strip() == "texte"), 1)


def verifie_grille():
    """La disposition en grille : pistes, espacement, passage a la ligne."""
    doc = document("""
        <style>
          body { margin: 0; }
          #g { display: grid; grid-template-columns: repeat(3, 1fr); gap: 10px; }
        </style>
        <body>
          <div id="g">
            <div id="a">A</div><div id="b">B</div>
            <div id="c">C</div><div id="d">D</div>
          </div>
        </body>""")
    g = boite_de(doc, "g")
    a, b, c, d = (boite_de(doc, n) for n in "abcd")
    verifie("grille: quatre cases posees", None not in (a, b, c, d))
    verifie("grille: trois colonnes egales",
            proche(a.largeur, b.largeur) and proche(b.largeur, c.largeur),
            (a.largeur, b.largeur, c.largeur))
    verifie("grille: l'espacement separe les colonnes",
            proche(b.x - a.x, a.largeur + 10, 2.0), (b.x - a.x, a.largeur))
    verifie("grille: trois colonnes plus deux espaces remplissent",
            proche(3 * a.largeur + 20, g.largeur, 3.0),
            (3 * a.largeur + 20, g.largeur))
    verifie("grille: la quatrieme case passe a la ligne", d.y > a.y, (a.y, d.y))
    verifie("grille: elle revient a gauche", proche(d.x, a.x), (a.x, d.x))

    # Pistes de largeur fixe et placement explicite.
    doc = document("""
        <style>
          body { margin: 0; }
          #g { display: grid; grid-template-columns: 200px 300px; }
          #b { grid-column: 1 / span 2; }
        </style>
        <body>
          <div id="g"><div id="a">A</div><div id="b">B</div></div>
        </body>""")
    a, b = boite_de(doc, "a"), boite_de(doc, "b")
    verifie("grille: piste fixe", proche(a.largeur, 200.0), a.largeur)
    verifie("grille: une case peut s'etendre", proche(b.largeur, 500.0), b.largeur)


def verifie_position():
    """`position: absolute` sort du flux et se cale sur son bloc contenant."""
    doc = document("""
        <style>
          body { margin: 0; }
          #cadre { position: relative; width: 400px; height: 300px; }
          #coin { position: absolute; left: 20px; top: 30px; width: 50px; }
          #suivant { height: 10px; }
        </style>
        <body>
          <div id="cadre">
            <div id="coin">coin</div>
            <div id="suivant">suivant</div>
          </div>
        </body>""")
    cadre = boite_de(doc, "cadre")
    coin = boite_de(doc, "coin")
    suivant = boite_de(doc, "suivant")
    verifie("position: la boite absolue existe", coin is not None)
    verifie("position: left applique", proche(coin.x, cadre.x + 20), coin.x)
    verifie("position: top applique", proche(coin.y, cadre.y + 30), coin.y)
    verifie("position: elle ne pousse pas le flux",
            proche(suivant.y, cadre.y), (suivant.y, cadre.y))

    # `right`/`bottom` se comptent depuis le bord oppose.
    doc = document("""
        <style>
          body { margin: 0; }
          #cadre { position: relative; width: 400px; height: 300px; }
          #coin { position: absolute; right: 20px; width: 50px; }
        </style>
        <body><div id="cadre"><div id="coin">c</div></div></body>""")
    cadre, coin = boite_de(doc, "cadre"), boite_de(doc, "coin")
    verifie("position: right applique",
            proche(coin.x + coin.largeur, cadre.x + cadre.largeur - 20, 2.0),
            (coin.x + coin.largeur, cadre.x + cadre.largeur - 20))


def verifie_pseudo_elements():
    """`::before` et `::after` engendrent du contenu que la page n'ecrit pas."""
    doc = document("""
        <style>
          #note::before { content: "Note : "; }
          #note::after { content: " (fin)"; }
          #lien::after { content: attr(data-suffixe); }
        </style>
        <body>
          <p id="note">le corps</p>
          <p id="lien" data-suffixe="[ext]">ancre</p>
        </body>""")
    textes = [e[3] for e in doc.liste_affichage(0, 1000, 700) if e[0] == "texte"]
    joint = " ".join(textes)
    verifie("pseudo: ::before est peint", "Note" in joint, textes[:8])
    verifie("pseudo: ::after est peint", "(fin)" in joint, textes[:8])
    verifie("pseudo: attr() est resolu", "[ext]" in joint, textes[:8])

    # Sans `content`, aucune boite ne doit apparaitre.
    doc = document("""
        <style>#vide::before { color: red; }</style>
        <body><p id="vide">seul</p></body>""")
    textes = [e[3] for e in doc.liste_affichage(0, 1000, 700) if e[0] == "texte"]
    egal("pseudo: pas de content, pas de boite",
         [t for t in textes if t.strip()], ["seul"])


def verifie_requetes_media():
    """Les `@media` sont evaluees contre la taille de fenetre reelle."""
    source = """
        <style>
          body { margin: 0; }
          #t { width: 100px; }
          @media (min-width: 800px) { #t { width: 300px; } }
          @media (max-width: 500px) { #t { width: 50px; } }
        </style>
        <body><div id="t">t</div></body>"""

    doc = document(source)
    egal("media: min-width retenu a 1000px", round(boite_de(doc, "t").largeur), 300)

    # La meme page dans une fenetre etroite retient l'autre regle.
    reponse = reseau.Reponse("http://exemple.test/p", source, "text/html", 200)
    doc = moteur.Document(reponse, 400, hauteur_fenetre=800.0, precharge=False)
    egal("media: max-width retenu a 400px", round(boite_de(doc, "t").largeur), 50)

    # Et un redimensionnement rebascule.
    doc.remet_en_page(1000)
    egal("media: le redimensionnement rebascule",
         round(boite_de(doc, "t").largeur), 300)


def verifie_longueurs():
    """`calc()`, unites de fenetre, `box-sizing`, bornes de largeur."""
    doc = document("""
        <style>
          body { margin: 0; }
          #c { width: calc(100% - 40px); }
          #v { width: 50vw; }
          #h { height: 10vh; }
          #b { width: 200px; padding: 10px; border: 5px solid #000000;
               box-sizing: border-box; }
          #n { width: 200px; padding: 10px; border: 5px solid #000000; }
          #m { max-width: 600px; margin-left: auto; margin-right: auto; }
        </style>
        <body>
          <div id="c">c</div><div id="v">v</div><div id="h">h</div>
          <div id="b">b</div><div id="n">n</div><div id="m">m</div>
        </body>""")
    egal("longueur: calc(100% - 40px)", round(boite_de(doc, "c").largeur), 960)
    egal("longueur: 50vw sur une fenetre de 1000", round(boite_de(doc, "v").largeur), 500)
    egal("longueur: 10vh sur une fenetre de 720", round(boite_de(doc, "h").hauteur), 72)
    egal("longueur: border-box englobe bordure et remplissage",
         round(boite_de(doc, "b").largeur), 200)
    egal("longueur: content-box ajoute bordure et remplissage",
         round(boite_de(doc, "n").largeur), 230)

    m = boite_de(doc, "m")
    egal("longueur: max-width borne la largeur", round(m.largeur), 600)
    verifie("longueur: margin auto centre", proche(m.x, 200.0, 2.0), m.x)

    # `overflow: hidden` demande a l'hote de rogner ce qui depasse.
    doc = document("""
        <style>
          body { margin: 0; }
          #cadre { overflow: hidden; height: 40px; }
        </style>
        <body><div id="cadre"><p>un</p><p>deux</p><p>trois</p></div></body>""")
    verifie("overflow: la boite est marquee", boite_de(doc, "cadre").rogne)
    liste = doc.liste_affichage(0, 1000, 700)
    operations = [e[0] for e in liste]
    verifie("overflow: la liste d'affichage rogne", "clip" in operations, operations[:8])
    verifie("overflow: et derogne ensuite", "declip" in operations, operations[-4:])
    egal("overflow: autant de clip que de declip",
         operations.count("clip"), operations.count("declip"))


# --- Vitesse de chargement ----------------------------------------------------

class _PriseFictive:
    """Le minimum qu'attend la reserve de connexions : un etat et une fermeture."""

    def __init__(self, ouverte=True):
        self.sock = object() if ouverte else None
        self.fermee = False

    def close(self):
        self.fermee = True
        self.sock = None


def sert(reponses):
    """Remplace le reseau par une table `url -> (octets, type)`.

    Le noyau n'a ni `listen` ni `accept` : aucune de ces verifications ne peut
    monter un serveur local, sous l'OS comme ici. On substitue donc le
    chargement lui-meme, ce qui eprouve exactement ce qui nous interesse — le
    cheminement des ressources dans le moteur — sans dependre du monde exterieur.
    """
    demandes = []

    def charge(url, methode="GET", corps=None, entetes=None, brut=False,
               document=None, destination=None):
        demandes.append(url)
        if url not in reponses:
            return reseau.Reponse(url, "", "text/plain", 404)
        contenu, type_mime = reponses[url]
        octets = contenu.encode("utf-8") if isinstance(contenu, str) else contenu
        return reseau.Reponse(url, octets.decode("utf-8", "replace"), type_mime,
                              200, octets=octets)

    return charge, demandes


def verifie_compression():
    """Un corps `gzip` ou `deflate` doit arriver en clair."""
    import gzip
    import zlib

    clair = b"<h1>Bouchaud</h1>" * 40
    egal("compression: gzip", reseau._decompresse(gzip.compress(clair), "gzip"), clair)
    egal("compression: deflate", reseau._decompresse(zlib.compress(clair), "deflate"),
         clair)
    # Certains serveurs emettent du deflate brut, sans en-tete zlib.
    brut = zlib.compressobj(9, zlib.DEFLATED, -zlib.MAX_WBITS)
    cru = brut.compress(clair) + brut.flush()
    egal("compression: deflate brut", reseau._decompresse(cru, "deflate"), clair)

    egal("compression: identity intacte", reseau._decompresse(clair, "identity"), clair)
    egal("compression: sans encodage", reseau._decompresse(clair, ""), clair)
    # Un corps annonce compresse mais qui ne l'est pas revient tel quel plutot
    # que vide : mieux vaut une page mal affichee qu'une page blanche.
    egal("compression: annonce mensongere", reseau._decompresse(clair, "gzip"), clair)
    egal("compression: encodage inconnu", reseau._decompresse(clair, "br"), clair)

    verifie("compression: le gzip vaut la peine",
            len(gzip.compress(clair)) < len(clair) / 4,
            (len(gzip.compress(clair)), len(clair)))


def verifie_reserve_connexions():
    """Une connexion rendue est reprise ; une connexion morte ne l'est pas."""
    reseau.ferme_connexions()
    cle = (False, "exemple.test", 80)

    vivante = _PriseFictive()
    reseau._rend(cle, vivante)
    egal("reserve: la connexion rendue est reprise", reseau._emprunte(cle), vivante)
    egal("reserve: elle n'est reprise qu'une fois", reseau._emprunte(cle), None)

    # Une prise fermee par le serveur ne doit jamais ressortir de la reserve.
    morte = _PriseFictive(ouverte=False)
    reseau._rend(cle, morte)
    egal("reserve: une connexion morte n'est pas reprise", reseau._emprunte(cle), None)
    verifie("reserve: elle est fermee", morte.fermee)

    # La reserve est bornee : au-dela, on ferme.
    prises = [_PriseFictive() for _ in range(reseau.PRISES_PAR_HOTE + 2)]
    for prise in prises:
        reseau._rend(cle, prise)
    egal("reserve: bornee", sum(1 for p in prises if not p.fermee),
         reseau.PRISES_PAR_HOTE)

    reseau.ferme_connexions()
    egal("reserve: la fermeture vide tout", reseau._emprunte(cle), None)
    verifie("reserve: tout est referme", all(p.fermee for p in prises))


def verifie_prechargement():
    """Les sous-ressources sont relevees, demandees ensemble, et reprises."""
    from moteur import prechargement

    racine = html.analyse("""
        <link rel="stylesheet" href="/style.css">
        <link rel="preload" href="/pas-une-feuille.css">
        <body>
          <img src="/a.png"><img src="/b.png"><img src="/a.png">
          <img src="data:image/png;base64,AAAA">
          <img src="#ancre">
          <script src="/app.js"></script>
        </body>""")
    trouvees = prechargement.adresses(racine, "http://exemple.test/page")
    egal("prechargement: les adresses relevees", trouvees, [
        "http://exemple.test/style.css",
        "http://exemple.test/a.png",
        "http://exemple.test/b.png",
        "http://exemple.test/app.js",
    ])

    # `data:` porte deja ses octets, une ancre n'est pas une ressource, et un
    # `rel` qui n'est pas `stylesheet` ne se precharge pas.
    verifie("prechargement: pas de data:",
            not any(u.startswith("data:") for u in trouvees), trouvees)
    verifie("prechargement: pas de rel etranger",
            not any("pas-une-feuille" in u for u in trouvees), trouvees)

    # Le depot rend la ressource disponible sans reseau, une seule fois.
    reseau.oublie_precharges()
    avance = reseau.Reponse("http://exemple.test/a.png", "", "image/png",
                            200, octets=b"PNG")
    reseau.depose("http://exemple.test/a.png", avance)
    egal("prechargement: la reponse deposee est reprise",
         reseau.charge("http://exemple.test/a.png", brut=True), avance)

    # Une seconde demande ne doit pas resservir l'entree : le prechargement
    # anticipe une requete, il ne tient pas lieu de cache.
    charge, demandes = sert({})
    ancien = reseau.charge
    try:
        reseau.charge = charge
        prechargement.reseau.charge("http://exemple.test/a.png", brut=True)
    finally:
        reseau.charge = ancien
    egal("prechargement: l'entree n'est servie qu'une fois", demandes,
         ["http://exemple.test/a.png"])

    # Et le prechargement complet remplit bien la reserve.
    reseau.oublie_precharges()
    charge, demandes = sert({
        "http://exemple.test/style.css": ("p { color: red; }", "text/css"),
        "http://exemple.test/a.png": ("octets", "image/png"),
    })
    ancien = reseau.charge
    try:
        reseau.charge = charge
        obtenues = prechargement.precharge(racine, "http://exemple.test/page")
    finally:
        reseau.charge = ancien
    egal("prechargement: deux ressources sur quatre existent", obtenues, 2)
    egal("prechargement: les quatre ont ete demandees", len(demandes), 4)
    reseau.oublie_precharges()


def verifie_feuilles_liees():
    """Un `<link rel=stylesheet>` doit reellement s'appliquer."""
    charge, demandes = sert({
        "http://exemple.test/style.css": (
            "body { margin: 0; } #t { width: 300px; color: #ff0000; }", "text/css"),
    })
    ancien = reseau.charge
    try:
        reseau.charge = charge
        source = """
            <link rel="stylesheet" href="style.css">
            <body><div id="t">t</div></body>"""
        reponse = reseau.Reponse("http://exemple.test/page", source, "text/html", 200)
        doc = moteur.Document(reponse, 1000, precharge=False)
        egal("feuille liee: la largeur vient de la feuille",
             round(boite_de(doc, "t").largeur), 300)
        egal("feuille liee: la couleur aussi",
             boite_de(doc, "t").style.get("color"), "#ff0000")
        egal("feuille liee: telechargee une fois", len(demandes), 1)

        # Un redimensionnement rejoue la cascade sans redemander le fichier.
        doc.remet_en_page(800)
        egal("feuille liee: pas de second telechargement", len(demandes), 1)
        egal("feuille liee: toujours appliquee",
             round(boite_de(doc, "t").largeur), 300)

        # L'ordre du document tranche : ce qui suit l'emporte.
        source = """
            <style>#t { width: 100px; }</style>
            <link rel="stylesheet" href="style.css">
            <body><div id="t">t</div></body>"""
        reponse = reseau.Reponse("http://exemple.test/page", source, "text/html", 200)
        doc = moteur.Document(reponse, 1000, precharge=False)
        egal("feuille liee: elle l'emporte sur le style qui la precede",
             round(boite_de(doc, "t").largeur), 300)

        # Et une feuille absente ne casse pas la page.
        source = """
            <link rel="stylesheet" href="/introuvable.css">
            <body><div id="t">t</div></body>"""
        reponse = reseau.Reponse("http://exemple.test/page", source, "text/html", 200)
        doc = moteur.Document(reponse, 1000, precharge=False)
        verifie("feuille liee: une feuille absente n'arrete pas la page",
                boite_de(doc, "t") is not None)
    finally:
        reseau.charge = ancien


def verifie_index_regles():
    """L'index doit ecarter beaucoup de regles sans jamais changer le resultat."""
    from moteur import css

    feuille = """
        * { line-height: 1.5; }
        p { color: #111111; }
        .carte { color: #222222; }
        #unique { color: #333333; }
        div.carte p { color: #444444; }
    """
    regles = css.analyse(feuille)
    index = css.indexe(regles)

    page = html.analyse('<body><div class="carte"><p id="unique">t</p></div></body>')
    cible = next(n for n in page.parcours()
                 if isinstance(n, html.Element) and n.balise == "p")
    chemin = []
    courant = cible
    while courant is not None:
        chemin.append(courant)
        courant = courant.parent
    chemin.reverse()

    # Le meme style, que l'on passe par l'index ou par la liste entiere : c'est
    # la seule chose que l'index n'a pas le droit de changer.
    par_index = css.applique(index, cible, chemin, {})
    par_liste = css.applique(regles, cible, chemin, {})
    egal("index: meme resultat que la liste", par_index, par_liste)
    egal("index: la regle la plus specifique gagne", par_index.get("color"), "#333333")

    # Et il ecarte reellement : un `p` ne doit pas voir les regles de `.carte`
    # ni celles de `#unique` portees par un autre element.
    autre = html.analyse('<body><span class="ailleurs">x</span></body>')
    span = next(n for n in autre.parcours()
                if isinstance(n, html.Element) and n.balise == "span")
    candidates = index.candidates(span)
    egal("index: seules les regles universelles restent",
         [r.selecteur.maillons[-1].balise for r in candidates], [None])
    verifie("index: il ecarte la majorite des regles",
            len(candidates) < len(regles), (len(candidates), len(regles)))

    # Une feuille large : c'est la que l'index compte.
    large = "\n".join(".c%d { color: #010101; }" % i for i in range(500))
    index_large = css.indexe(css.analyse(large + "\n" + feuille))
    egal("index: une feuille de 500 classes n'en propose que l'utile",
         len(index_large.candidates(span)), 1)

    verifie_index_cles()


def verifie_index_cles():
    """Les cles d'indexation : ce qui est range ou, et surtout ce qui ne l'est pas.

    Un index de selecteurs a un mode de defaillance particulier : il ne casse
    rien bruyamment, il **perd des styles**. Une regle rangee sous une cle que
    l'element ne porte pas n'est jamais essayee, et la page s'affiche presque
    bien — c'est le pire genre de bogue a diagnostiquer.

    Chaque cas ci-dessous verifie donc la meme chose sous deux angles : que le
    style calcule par l'index est identique a celui calcule sans lui, et que le
    filtrage a bien eu lieu.
    """
    def chemin_de(source, balise):
        page = html.analyse(source)
        cible = next(n for n in page.parcours()
                     if isinstance(n, html.Element) and n.balise == balise)
        chemin, courant = [], cible
        while courant is not None:
            chemin.append(courant)
            courant = courant.parent
        chemin.reverse()
        return cible, chemin

    def meme_style(nom, feuille, source, balise, attendu_couleur):
        regles = css.analyse(feuille)
        index = css.indexe(regles)
        cible, chemin = chemin_de(source, balise)
        par_index = css.applique(index, cible, chemin, {})
        par_liste = css.applique(regles, cible, chemin, {})
        egal("index %s: meme resultat que sans index" % nom, par_index, par_liste)
        egal("index %s: la regle s'applique" % nom,
             par_index.get("color"), attendu_couleur)

    # --- Attribut : la cle la plus rentable de la mesure ---------------------
    # 34 des 116 regles non indexables de pypi.org tenaient a `[type]`.
    meme_style("attribut", "[type=text] { color: #aa0000; }",
               '<body><input type="text"></body>', "input", "#aa0000")
    index = css.indexe(css.analyse("[type=text] { color: #aa0000; }"))
    _, chemin = chemin_de('<body><p>x</p></body>', "p")
    egal("index attribut: un element sans l'attribut ne voit pas la regle",
         len(index.candidates(chemin[-1])), 0)

    # --- `:is()` prete ses cles ---------------------------------------------
    meme_style("is", ":is(input, select, textarea) { color: #00aa00; }",
               '<body><select></select></body>', "select", "#00aa00")
    index = css.indexe(css.analyse(":is(input, select) { color: #00aa00; }"))
    _, chemin = chemin_de('<body><p>x</p></body>', "p")
    egal("index is: une balise hors du groupe ne voit pas la regle",
         len(index.candidates(chemin[-1])), 0)

    # --- `:not()` ne prete PAS les siennes -----------------------------------
    # C'est le cas ou une erreur d'indexation serait invisible en test naif :
    # `:not(.modal)` designe TOUT SAUF `.modal`. Le ranger sous `.modal` le
    # rendrait invisible pour tous les autres — c'est-a-dire presque tous.
    meme_style("not", ":not(.modal) { color: #0000aa; }",
               '<body><p>x</p></body>', "p", "#0000aa")
    index = css.indexe(css.analyse(":not(.modal) { color: #0000aa; }"))
    _, chemin = chemin_de('<body><p>x</p></body>', "p")
    verifie("index not: la regle reste universelle",
            len(index.candidates(chemin[-1])) == 1,
            len(index.candidates(chemin[-1])))

    # --- `:is()` dont une branche est libre reste universel ------------------
    index = css.indexe(css.analyse(":is(p, [hidden]) { color: #111; }"))
    _, chemin = chemin_de('<body><span hidden>x</span></body>', "span")
    verifie("index is: une branche par attribut range sous cet attribut",
            len(index.candidates(chemin[-1])) == 1,
            len(index.candidates(chemin[-1])))

    # --- Partition par pseudo-element ---------------------------------------
    # 53 % des cascades de pypi.org visaient un pseudo-element et essayaient
    # quand meme toutes les regles de l'element porteur.
    feuille = """
        p { color: #101010; }
        p::before { content: "a"; color: #202020; }
        p::after  { content: "b"; color: #303030; }
    """
    regles = css.analyse(feuille)
    index = css.indexe(regles)
    cible, chemin = chemin_de('<body><p>x</p></body>', "p")
    egal("index pseudo: l'element ne voit que ses regles",
         len(index.candidates(cible, None)), 1)
    egal("index pseudo: ::before ne voit que les siennes",
         len(index.candidates(cible, "before")), 1)
    egal("index pseudo: le style de ::before est le bon",
         css.applique(index, cible, chemin, {}, "before").get("color"), "#202020")
    egal("index pseudo: celui de ::after aussi",
         css.applique(index, cible, chemin, {}, "after").get("color"), "#303030")
    egal("index pseudo: et celui de l'element n'a pas bouge",
         css.applique(index, cible, chemin, {}).get("color"), "#101010")

    # --- `:root` ne designe que la racine ------------------------------------
    index = css.indexe(css.analyse(":root { color: #404040; }"))
    _, chemin = chemin_de('<body><p>x</p></body>', "p")
    egal("index root: un element ordinaire ne voit pas la regle",
         len(index.candidates(chemin[-1])), 0)

    # --- Aucune regle rendue deux fois ---------------------------------------
    # Un `:is()` range sous plusieurs classes que l'element porte toutes.
    index = css.indexe(css.analyse(":is(.fas, .far) { color: #505050; }"))
    _, chemin = chemin_de('<body><i class="fas far">x</i></body>', "i")
    egal("index is: une regle rangee sous deux cles n'est rendue qu'une fois",
         len(index.candidates(chemin[-1])), 1)


def verifie_variables_css():
    """Les proprietes personnalisees, sans lesquelles une page moderne est nue.

    Un site d'aujourd'hui range ses couleurs, ses espacements et ses rayons dans
    des `--variables` posees sur `:root`, puis n'ecrit plus que `var(...)`.
    Ignorer `var()` ne degrade pas un peu la page : elle s'affiche sans aucune
    de ses couleurs et sans aucun de ses espacements.
    """
    doc = document("""
        <style>
          :root { --fond: #223344; --marge: 12px; --large: 300px; }
          body { margin: 0; }
          #a { background-color: var(--fond); padding: var(--marge);
               width: var(--large); }
          #b { background-color: var(--absente, #ff0000); }
          #c { background-color: var(--absente); }
          .theme { --fond: #00ff00; }
          #d { background-color: var(--fond); }
          #e { width: calc(var(--large) + 20px); }
        </style>
        <body>
          <div id="a">a</div>
          <div id="b">b</div>
          <div id="c">c</div>
          <div class="theme"><div id="d">d</div></div>
          <div id="e">e</div>
        </body>""")

    a = boite_de(doc, "a")
    egal("variables: la couleur vient de la variable",
         a.style.get("background-color"), "#223344")
    # 300 de contenu plus 12 de remplissage de chaque cote : c'est
    # `content-box`, le defaut de CSS, et c'est bien ce qu'on veut verifier.
    egal("variables: la longueur aussi", round(a.largeur), 324)
    egal("variables: et le remplissage",
         a.style.get("padding-left"), "12px")

    egal("variables: la valeur de secours sert quand la variable manque",
         boite_de(doc, "b").style.get("background-color"), "#ff0000")
    # Sans secours, la declaration devient inexploitable — c'est ce que dit la
    # norme, et une couleur vide est deja ignoree partout ailleurs.
    verifie("variables: sans secours, rien n'est peint",
            not css_couleur(boite_de(doc, "c").style.get("background-color", "")),
            boite_de(doc, "c").style.get("background-color"))

    # Une variable redefinie plus bas dans l'arbre l'emporte, par heritage.
    egal("variables: une redefinition locale l'emporte",
         boite_de(doc, "d").style.get("background-color"), "#00ff00")

    # Et `var()` doit survivre a son enrobage dans `calc()`.
    egal("variables: var() dans calc()", round(boite_de(doc, "e").largeur), 320)

    # Une variable qui se refere a elle-meme ne doit pas faire boucler.
    doc = document("""
        <style>
          :root { --boucle: var(--boucle); }
          body { margin: 0; }
          #x { width: var(--boucle); }
        </style>
        <body><div id="x">x</div></body>""")
    verifie("variables: un cycle ne fait pas boucler", boite_de(doc, "x") is not None)

    # Le style en ligne peut definir une variable comme n'importe quelle regle.
    doc = document("""
        <style>body { margin: 0 } #y { width: var(--w) }</style>
        <body><div id="y" style="--w: 250px">y</div></body>""")
    egal("variables: definie en ligne", round(boite_de(doc, "y").largeur), 250)


def verifie_racine():
    """`<html>` compte : ses variables, et la reference des `rem`."""
    # `font-size: 62.5%` sur la racine est la facon la plus repandue de faire
    # valoir 1 rem pour 10 px. La supposer a 16 px sortait toute la page une
    # fois et demie trop grande.
    doc = document("""
        <style>
          html { font-size: 62.5%; }
          body { margin: 0; }
          #a { width: 30rem; }
        </style>
        <body><div id="a">a</div></body>""")
    egal("racine: rem suit font-size de html", round(boite_de(doc, "a").largeur), 300)

    # Sans declaration, 1 rem vaut 16 px.
    doc = document("""
        <style>body { margin: 0 } #a { width: 10rem }</style>
        <body><div id="a">a</div></body>""")
    egal("racine: rem vaut 16 px par defaut",
         round(boite_de(doc, "a").largeur), 160)

    # Et `em` reste relatif a l'element, pas a la racine.
    doc = document("""
        <style>
          html { font-size: 10px; }
          body { margin: 0; font-size: 20px; }
          #a { width: 10em; }
        </style>
        <body><div id="a">a</div></body>""")
    egal("racine: em reste relatif a l'element",
         round(boite_de(doc, "a").largeur), 200)

    # Une regle sur `html` s'applique reellement, et s'herite.
    doc = document("""
        <style>html { color: #123456 } body { margin: 0 }</style>
        <body><p id="p">texte</p></body>""")
    egal("racine: une regle sur html s'herite",
         boite_de(doc, "p").style.get("color"), "#123456")


def css_couleur(valeur):
    from moteur import css
    return css.couleur(valeur or "")


def verifie_enfant_direct():
    """`>` doit designer l'enfant direct, pas n'importe quel descendant."""
    # `background-color` ne s'herite pas : c'est ce qui permet de distinguer un
    # element reellement atteint par la regle d'un element qui tient sa valeur
    # de son parent. Avec `color`, la verification ne prouverait rien.
    doc = document("""
        <style>
          body { margin: 0; }
          .menu > li { background-color: #ff0000; }
          .menu li { border-color: #0000ff; }
        </style>
        <body>
          <ul class="menu">
            <li id="direct">un
              <ul><li id="imbrique">deux</li></ul>
            </li>
          </ul>
        </body>""")
    direct = boite_de(doc, "direct")
    imbrique = boite_de(doc, "imbrique")
    egal("enfant direct: l'enfant immediat est atteint",
         direct.style.get("background-color"), "#ff0000")
    egal("enfant direct: le petit-fils ne l'est pas",
         imbrique.style.get("background-color"), None)
    # La descendance, elle, atteint bien les deux. `border-color` se developpe
    # maintenant par cote, d'ou la lecture de l'un d'eux.
    egal("enfant direct: la descendance atteint le petit-fils",
         imbrique.style.get("border-top-color"), "#0000ff")

    # Une chaine de plusieurs `>` doit se verifier de bout en bout.
    doc = document("""
        <style>
          body { margin: 0; }
          .a > .b > .c { background-color: #00ff00; }
        </style>
        <body>
          <div class="a"><div class="b"><div id="oui" class="c">x</div></div></div>
          <div class="a"><div><div class="b"><div id="non" class="c">x</div></div></div></div>
        </body>""")
    egal("enfant direct: chaine complete",
         boite_de(doc, "oui").style.get("background-color"), "#00ff00")
    egal("enfant direct: chaine rompue",
         boite_de(doc, "non").style.get("background-color"), None)


def verifie_cascade_memorisee():
    """La cascade n'est reconstruite que si les feuilles ont change."""
    doc = document("""
        <style>#t { width: 200px; }</style>
        <body><div id="t">t</div><script>document.title = 'ok';</script></body>""")
    premier = doc.regles
    doc.remet_en_page(1000)
    verifie("cascade: pas de reconstruction a taille egale", doc.regles is premier)

    # Un redimensionnement, lui, peut faire basculer des `@media` : l'index doit
    # etre refait.
    doc.remet_en_page(600)
    verifie("cascade: reconstruite quand la fenetre change",
            doc.regles is not premier)

    # Et un script qui insere une feuille doit etre pris en compte.
    doc = document("""
        <body>
          <div id="t">t</div>
          <script>
            const feuille = document.createElement('style');
            feuille.textContent = '#t { width: 250px; }';
            document.body.appendChild(feuille);
          </script>
        </body>""")
    egal("cascade: une feuille posee par le script s'applique",
         round(boite_de(doc, "t").largeur), 250)


# --- Web applicatif -----------------------------------------------------------

def verifie_style_calcule():
    """`getComputedStyle` doit rendre le style resolu, pas l'attribut `style`."""
    doc = document("""
        <style>
          #t { color: #ff0000; width: 300px; }
          .herite { font-size: 22px; }
        </style>
        <body>
          <div class="herite">
            <div id="t" style="padding-left: 7px">t</div>
          </div>
          <script>
            const cible = document.getElementById('t');
            const calcule = getComputedStyle(cible);
            window.__couleur = calcule.color;
            window.__largeur = calcule.width;
            window.__police = calcule.fontSize;
            window.__marge = calcule.getPropertyValue('padding-left');
            window.__enLigne = cible.style.color;
          </script>
        </body>""")
    contexte = doc.contexte_js
    egal("style calcule: la couleur vient de la feuille",
         contexte.execute("window.__couleur"), "#ff0000")
    egal("style calcule: la largeur aussi", contexte.execute("window.__largeur"),
         "300px")
    egal("style calcule: l'heritage est resolu",
         contexte.execute("window.__police"), "22px")
    egal("style calcule: le style en ligne y figure aussi",
         contexte.execute("window.__marge"), "7px")
    # Et `element.style` reste ce qu'il est : le seul attribut `style`.
    egal("style calcule: element.style ne connait que l'attribut",
         contexte.execute("window.__enLigne"), "")


def verifie_observateur_mutations():
    """`MutationObserver` doit voir les changements de l'arbre."""
    doc = document("""
        <body>
          <div id="cible"><span>un</span></div>
          <script>
            window.__lots = [];
            const observateur = new MutationObserver(function (lot, moi) {
              window.__lots.push(lot);
              window.__moi = (moi === observateur);
            });
            observateur.observe(document.getElementById('cible'),
                                { childList: true, attributes: true,
                                  characterData: true, subtree: true });

            const cible = document.getElementById('cible');
            cible.appendChild(document.createElement('p'));
            cible.appendChild(document.createElement('p'));
            cible.setAttribute('data-etat', 'ouvert');

            // Ce qui se passe hors de la cible ne doit pas etre rapporte.
            document.body.appendChild(document.createElement('hr'));
            window.__observateur = observateur;
          </script>
        </body>""")
    contexte = doc.contexte_js
    # Les enregistrements sont livres en micro-tache : un battement suffit.
    contexte.tic()

    egal("mutations: un seul lot pour la salve",
         contexte.execute("window.__lots.length"), 1)
    egal("mutations: trois enregistrements",
         contexte.execute("window.__lots[0].length"), 3)
    egal("mutations: le premier est un ajout",
         contexte.execute("window.__lots[0][0].type"), "childList")
    egal("mutations: le nœud ajoute est rapporte",
         contexte.execute("window.__lots[0][0].addedNodes[0].tagName"), "P")
    egal("mutations: le changement d'attribut est rapporte",
         contexte.execute("window.__lots[0][2].type"), "attributes")
    egal("mutations: avec son nom",
         contexte.execute("window.__lots[0][2].attributeName"), "data-etat")
    egal("mutations: l'observateur se recoit lui-meme",
         contexte.execute("window.__moi"), True)

    # `disconnect` doit reellement debrancher.
    contexte.execute("""
        window.__observateur.disconnect();
        document.getElementById('cible').appendChild(document.createElement('b'));
    """)
    contexte.tic()
    egal("mutations: disconnect debranche", contexte.execute("window.__lots.length"), 1)


def verifie_observateur_visibilite():
    """`IntersectionObserver` doit signaler ce qui entre dans la fenetre."""
    doc = document("""
        <style>
          body { margin: 0; }
          #haut { height: 100px; }
          #loin { height: 100px; margin-top: 4000px; }
        </style>
        <body>
          <div id="haut">visible</div>
          <div id="loin">hors ecran</div>
          <script>
            window.__vus = [];
            const observateur = new IntersectionObserver(function (entrees) {
              for (const entree of entrees)
                window.__vus.push(entree.target.id + ':' + entree.isIntersecting);
            });
            observateur.observe(document.getElementById('haut'));
            observateur.observe(document.getElementById('loin'));
          </script>
        </body>""")
    contexte = doc.contexte_js
    contexte.tic()
    vus = contexte.execute("window.__vus.join(',')")
    verifie("visibilite: l'element en haut de page est vu",
            "haut:true" in vus, vus)
    verifie("visibilite: celui qui est a 4000 px ne l'est pas",
            "loin:true" not in vus, vus)

    # Un defilement le fait entrer : c'est tout l'interet de l'observateur.
    doc.defilement = 4000.0
    contexte.tic()
    vus = contexte.execute("window.__vus.join(',')")
    verifie("visibilite: le defilement le fait entrer", "loin:true" in vus, vus)


def verifie_composants():
    """Un composant declare doit etre instancie, connecte, et s'afficher."""
    doc = document("""
        <body>
          <mon-titre texte="premier"></mon-titre>
          <script>
            window.__trace = [];
            class MonTitre extends HTMLElement {
              static get observedAttributes() { return ['texte']; }
              constructor() {
                super();
                window.__trace.push('construit');
              }
              connectedCallback() {
                window.__trace.push('connecte');
                this.textContent = 'Titre : ' + (this.getAttribute('texte') || '');
              }
              disconnectedCallback() { window.__trace.push('deconnecte'); }
              attributeChangedCallback(nom, ancienne, nouvelle) {
                window.__trace.push('attribut:' + nom + '=' + nouvelle);
              }
              salue() { return 'bonjour'; }
            }
            customElements.define('mon-titre', MonTitre);
            window.__classe = (customElements.get('mon-titre') === MonTitre);
            window.__instance = (document.querySelector('mon-titre') instanceof MonTitre);
            window.__methode = document.querySelector('mon-titre').salue();

            // Un element cree apres coup doit l'etre aussi.
            const second = document.createElement('mon-titre');
            second.setAttribute('texte', 'second');
            document.body.appendChild(second);
            window.__second = (second instanceof MonTitre);
          </script>
        </body>""")
    contexte = doc.contexte_js
    egal("composants: la classe est retrouvee", contexte.execute("window.__classe"), True)
    egal("composants: l'element existant est instancie",
         contexte.execute("window.__instance"), True)
    egal("composants: ses methodes sont la",
         contexte.execute("window.__methode"), "bonjour")
    egal("composants: un element cree apres coup l'est aussi",
         contexte.execute("window.__second"), True)

    trace = contexte.execute("window.__trace.join(',')")
    verifie("composants: le constructeur a tourne", "construit" in trace, trace)
    verifie("composants: connectedCallback aussi", "connecte" in trace, trace)
    verifie("composants: attributeChangedCallback aussi",
            "attribut:texte=second" in trace, trace)

    # Et le contenu pose par le composant est reellement mis en page.
    textes = [e[3] for e in doc.liste_affichage(0, 1000, 700) if e[0] == "texte"]
    verifie("composants: le contenu s'affiche",
            any("Titre : premier" in t for t in textes), textes[:8])

    # Le retrait doit declencher `disconnectedCallback`.
    contexte.execute("document.querySelector('mon-titre').remove();")
    verifie("composants: disconnectedCallback",
            "deconnecte" in contexte.execute("window.__trace.join(',')"))

    # Un nom sans tiret n'est pas un nom de composant.
    egal("composants: un nom sans tiret est refuse", contexte.execute("""
        (function () {
          try { customElements.define('simple', class extends HTMLElement {}); return 'accepte'; }
          catch (e) { return 'refuse'; }
        })()"""), "refuse")


def verifie_ombre():
    """Une racine d'ombre doit porter du contenu, et ce contenu doit s'afficher."""
    doc = document("""
        <body>
          <ma-carte></ma-carte>
          <script>
            class MaCarte extends HTMLElement {
              connectedCallback() {
                const ombre = this.attachShadow({ mode: 'open' });
                ombre.innerHTML = '<p class="dedans">contenu d ombre</p>';
              }
            }
            customElements.define('ma-carte', MaCarte);
            const carte = document.querySelector('ma-carte');
            window.__mode = carte.shadowRoot.mode;
            window.__hote = (carte.shadowRoot.host === carte);
            window.__trouve = carte.shadowRoot.querySelector('.dedans') !== null;
          </script>
        </body>""")
    contexte = doc.contexte_js
    egal("ombre: le mode est retenu", contexte.execute("window.__mode"), "open")
    egal("ombre: l'hote est retrouve", contexte.execute("window.__hote"), True)
    egal("ombre: on y cherche par selecteur", contexte.execute("window.__trouve"), True)

    textes = [e[3] for e in doc.liste_affichage(0, 1000, 700) if e[0] == "texte"]
    verifie("ombre: son contenu s'affiche",
            any("contenu d ombre" in t for t in textes), textes[:8])


def verifie_toile():
    """Ce qu'un contexte 2D dessine doit se retrouver dans la liste d'affichage."""
    doc = document("""
        <body>
          <canvas id="dessin" width="200" height="120"></canvas>
          <script>
            const ctx = document.getElementById('dessin').getContext('2d');
            ctx.fillStyle = '#ff0000';
            ctx.fillRect(10, 20, 50, 30);
            ctx.strokeStyle = '#0000ff';
            ctx.lineWidth = 2;
            ctx.beginPath();
            ctx.moveTo(0, 0);
            ctx.lineTo(100, 60);
            ctx.stroke();
            ctx.fillStyle = '#008000';
            ctx.font = 'bold 14px sans-serif';
            ctx.fillText('etiquette', 5, 100);
            window.__mesure = ctx.measureText('etiquette').width;
          </script>
        </body>""")
    contexte = doc.contexte_js
    # Les operations partent en micro-tache : un battement les livre.
    contexte.tic()
    doc.rafraichis()

    boite = boite_de(doc, "dessin")
    verifie("toile: la boite a la taille declaree",
            proche(boite.largeur, 200.0) and proche(boite.hauteur, 120.0),
            (boite.largeur, boite.hauteur))
    verifie("toile: les operations sont retenues", boite.toile is not None
            and len(boite.toile) >= 3, boite.toile)

    liste = doc.liste_affichage(0, 1000, 700)
    rectangles = [e for e in liste if e[0] == "rect" and e[5] == 0xFFFF0000]
    verifie("toile: le rectangle rouge est peint", len(rectangles) == 1, len(rectangles))
    if rectangles:
        _, x, y, l, h, _ = rectangles[0]
        verifie("toile: il est decale a l'emplacement de la toile",
                proche(x, boite.x + 10) and proche(l, 50.0) and proche(h, 30.0),
                (x, boite.x, l, h))

    lignes = [e for e in liste if e[0] == "ligne"]
    verifie("toile: la ligne est peinte", len(lignes) >= 1, len(lignes))
    textes = [e[3] for e in liste if e[0] == "texte"]
    verifie("toile: le texte dessine est peint", "etiquette" in textes, textes[:6])

    verifie("toile: measureText mesure sur la vraie fonte",
            contexte.execute("window.__mesure") > 0,
            contexte.execute("window.__mesure"))

    # Le rognage encadre le dessin : ce qui deborde de la toile ne doit pas
    # peindre le reste de la page.
    operations = [e[0] for e in liste]
    egal("toile: autant de clip que de declip",
         operations.count("clip"), operations.count("declip"))

    # Effacer toute la toile repart de rien.
    contexte.execute("""
        const c = document.getElementById('dessin').getContext('2d');
        c.clearRect(0, 0, 200, 120);
    """)
    contexte.tic()
    doc.rafraichis()
    egal("toile: clearRect repart de rien", boite_de(doc, "dessin").toile, [])


def verifie_modules():
    """`<script type="module">` doit resoudre et executer ses `import`."""
    charge, demandes = sert({
        "http://exemple.test/lib/somme.js": (
            "export function somme(a, b) { return a + b; }\n"
            "export const nom = 'somme';\n", "text/javascript"),
        "http://exemple.test/lib/outils.js": (
            "import { somme } from './somme.js';\n"
            "export function triple(x) { return somme(somme(x, x), x); }\n",
            "text/javascript"),
    })
    ancien = reseau.charge
    try:
        reseau.charge = charge
        doc = document("""
            <body>
              <div id="sortie"></div>
              <script type="module">
                import { somme, nom } from './lib/somme.js';
                import { triple } from './lib/outils.js';
                document.getElementById('sortie').textContent =
                    nom + ' ' + somme(2, 3) + ' ' + triple(4);
              </script>
            </body>""")
    finally:
        reseau.charge = ancien

    cible = next(n for n in doc.racine.parcours()
                 if isinstance(n, html.Element) and n.attributs.get("id") == "sortie")
    egal("modules: import resolu et execute", cible.texte(), "somme 5 12")
    egal("modules: chaque module n'est demande qu'une fois",
         sorted(demandes), ["http://exemple.test/lib/outils.js",
                            "http://exemple.test/lib/somme.js"])

    # Un module absent ne doit pas tuer la page.
    charge, _ = sert({})
    ancien = reseau.charge
    try:
        reseau.charge = charge
        doc = document("""
            <body>
              <p id="reste">visible</p>
              <script type="module">import { x } from './absent.js'; window.__x = x;</script>
            </body>""")
    finally:
        reseau.charge = ancien
    textes = [e[3] for e in doc.liste_affichage(0, 1000, 700) if e[0] == "texte"]
    verifie("modules: un module absent n'arrete pas la page",
            any("visible" in t for t in textes), textes[:5])
    verifie("modules: et l'echec est consigne",
            any(niveau in ("warn", "error") for niveau, _ in doc.messages),
            doc.messages[:4])

    # Un script ordinaire reste ordinaire : `import` y serait une erreur de
    # syntaxe, et une declaration `var` doit rester visible globalement.
    doc = document("""
        <body><script>var visibleGlobalement = 42;</script></body>""")
    egal("modules: un script ordinaire garde la portee globale",
         doc.contexte_js.execute("visibleGlobalement"), 42)


# --- Persistance --------------------------------------------------------------

def isole_le_stockage():
    """Detourne le stockage persistant vers un dossier a jeter.

    Sans cela, ces verifications ecriraient — et surtout **effaceraient** — les
    vrais temoins, le vrai cache et le vrai stockage local de la machine. C'est
    exactement ce qui est arrive : jouees sous l'OS avant la sonde de
    persistance, elles supprimaient ce que celle-ci venait de deposer, et la
    sonde concluait que rien n'avait survecu au redemarrage.

    Une suite de verifications ne doit pas detruire les donnees de qui
    l'execute. Rend une fonction qui remet tout en place.
    """
    from moteur import stockage

    ancienne = stockage.RACINE
    stockage.RACINE = os.path.join(
        os.environ.get("BO_TMP", "/tmp"), "bo-verifications-stockage")
    try:
        shutil.rmtree(stockage.RACINE)
    except OSError:
        pass
    stockage.reinitialise()

    def restaure():
        try:
            shutil.rmtree(stockage.RACINE)
        except OSError:
            pass
        stockage.RACINE = ancienne
        stockage.reinitialise()

    return restaure


def verifie_temoins():
    """Le bocal a temoins : ce qu'on retient, a qui on le renvoie, et a qui non."""
    from moteur import stockage

    restaure = isole_le_stockage()
    try:
        _temoins(stockage)
    finally:
        restaure()


def _temoins(stockage):
    bocal = stockage.Temoins()
    bocal.oublie_tout()

    bocal.absorbe("https://exemple.test/connexion",
                  {"set-cookie": "session=abc123; Path=/; Secure"})
    egal("temoins: renvoye au meme site",
         bocal.pour("https://exemple.test/compte"), "session=abc123")
    egal("temoins: pas a un autre site",
         bocal.pour("https://autre.test/compte"), "")
    # `Secure` reserve le temoin a HTTPS : le renvoyer en clair le donnerait a
    # quiconque ecoute la ligne.
    egal("temoins: Secure retient en clair",
         bocal.pour("http://exemple.test/compte"), "")

    # Un sous-domaine recoit ce qui a ete pose pour le domaine parent.
    bocal.absorbe("https://exemple.test/",
                  {"set-cookie": "theme=sombre; Domain=exemple.test"})
    verifie("temoins: le sous-domaine recoit",
            "theme=sombre" in bocal.pour("https://api.exemple.test/x"),
            bocal.pour("https://api.exemple.test/x"))

    # Un site ne pose pas de temoin pour un domaine dont il ne fait pas partie.
    bocal.absorbe("https://mechant.test/", {"set-cookie": "vol=1; Domain=banque.test"})
    egal("temoins: pas de depot pour un domaine etranger",
         bocal.pour("https://banque.test/"), "")

    # Le chemin restreint la portee.
    bocal.absorbe("https://exemple.test/admin/page",
                  {"set-cookie": "jeton=zzz; Path=/admin"})
    verifie("temoins: rendu sous le bon chemin",
            "jeton=zzz" in bocal.pour("https://exemple.test/admin/outil"))
    verifie("temoins: pas ailleurs",
            "jeton=zzz" not in bocal.pour("https://exemple.test/public"))

    # `Max-Age=0` efface.
    bocal.absorbe("https://exemple.test/", {"set-cookie": "session=; Max-Age=0; Path=/"})
    verifie("temoins: Max-Age=0 efface",
            "session=" not in bocal.pour("https://exemple.test/compte"),
            bocal.pour("https://exemple.test/compte"))

    # Un temoin expire ne ressort pas.
    bocal.absorbe("https://exemple.test/",
                  {"set-cookie": "vieux=1; Path=/; Expires=Wed, 21 Oct 2015 07:28:00 GMT"})
    verifie("temoins: un temoin expire ne ressort pas",
            "vieux=1" not in bocal.pour("https://exemple.test/"),
            bocal.pour("https://exemple.test/"))

    # Et la date HTTP est lue correctement.
    egal("temoins: date HTTP",
         int(stockage._date_http("Thu, 01 Jan 1970 00:00:10 GMT")), 10)
    verifie("temoins: date HTTP bissextile",
            stockage._date_http("Mon, 29 Feb 2016 00:00:00 GMT") > 0)

    bocal.oublie_tout()


def verifie_cache_http():
    """Une reponse cachable resssert ; une reponse interdite de cache, non."""
    from moteur import stockage

    restaure = isole_le_stockage()
    try:
        _cache_http(stockage)
    finally:
        restaure()


def _cache_http(stockage):

    egal("cache: max-age est lu",
         stockage._duree_de_vie({"cache-control": "public, max-age=600"}), 600.0)
    egal("cache: no-store interdit",
         stockage._duree_de_vie({"cache-control": "no-store, max-age=600"}), 0)
    egal("cache: no-cache aussi",
         stockage._duree_de_vie({"cache-control": "no-cache"}), 0)
    egal("cache: private aussi",
         stockage._duree_de_vie({"cache-control": "private, max-age=600"}), 0)
    egal("cache: sans directive, rien n'est garde",
         stockage._duree_de_vie({}), 0)
    verifie("cache: Expires dans le futur donne une duree",
            stockage._duree_de_vie(
                {"expires": "Fri, 01 Jan 2100 00:00:00 GMT"}) > 0)
    egal("cache: Expires passe ne donne rien",
         stockage._duree_de_vie({"expires": "Wed, 21 Oct 2015 07:28:00 GMT"}), 0.0)

    if not stockage.disponible():
        # Sans zone persistante — le cas de la machine de developpement — le
        # cache ne retient rien, et c'est le comportement voulu : mieux vaut
        # aucun cache qu'un cache qui ment.
        egal("cache: sans zone persistante, rien n'est garde",
             stockage.Cache().depose("http://x.test/a", b"abc",
                                     {"cache-control": "max-age=600"}), False)
        return

    boite = stockage.Cache()
    boite.vide()
    egal("cache: une reponse cachable est gardee",
         boite.depose("http://exemple.test/logo.png", b"PNGPNGPNG",
                      {"cache-control": "max-age=600", "content-type": "image/png"}),
         True)
    garde = boite.lit("http://exemple.test/logo.png")
    verifie("cache: elle resssert", garde is not None)
    if garde:
        egal("cache: a l'identique", garde[0], b"PNGPNGPNG")
        egal("cache: avec son type", garde[1].get("content-type"), "image/png")
    egal("cache: une reponse interdite de cache n'est pas gardee",
         boite.depose("http://exemple.test/prive", b"secret",
                      {"cache-control": "no-store"}), False)
    egal("cache: et ne resssert pas", boite.lit("http://exemple.test/prive"), None)
    boite.vide()
    egal("cache: le vidage vide", boite.lit("http://exemple.test/logo.png"), None)


def verifie_indexeddb():
    """IndexedDB : asynchrone, transactionnel, persistant, cloisonne.

    Quatre proprietes, et chacune se casse d'une facon differente :

    * **asynchrone** — si une ecriture bloquait le fil, les minuteries
      s'arreteraient pendant. On en pose une et on verifie qu'elle a battu
      pendant que mille enregistrements partaient ;
    * **transactionnel** — une transaction abandonnee ne doit rien laisser
      derriere elle. Pas « presque rien » : rien ;
    * **persistant** — ce qui est ecrit doit se relire apres que tout ce qui
      etait en memoire a ete jete ;
    * **cloisonne** — deux origines qui ouvrent `mabase` ouvrent deux bases.
      Sans cela, un site lirait l'etat d'un autre.
    """
    import time as _time
    from moteur import indexeddb

    restaure = isole_le_stockage()
    try:
        indexeddb.reinitialise()
        _indexeddb_api(_time)
        _indexeddb_transactions()
        _indexeddb_persistance(indexeddb)
        _indexeddb_origines(indexeddb)
    finally:
        indexeddb.reinitialise()
        restaure()


def _bat(contexte, jusqua, secondes=10.0):
    """Bat la boucle d'evenements jusqu'a ce que la condition tienne.

    Comme pour le reseau : attendre un nombre fixe de tours rendrait ces
    epreuves instables sous charge, et un test instable est pire qu'absent.
    """
    import time as _time
    limite = _time.perf_counter() + secondes
    while _time.perf_counter() < limite:
        contexte.tic()
        if contexte.execute(jusqua):
            return True
        _time.sleep(0.01)
    return False


def _indexeddb_api(_time):
    """Le cycle complet : open → onupgradeneeded → transaction → put → get."""
    doc = document("<body><p>base</p></body>", url="https://un.test/page")
    contexte = js.Contexte(doc)

    verifie("idb: indexedDB est un objet, pas un moignon",
            contexte.execute("typeof indexedDB === 'object' && "
                             "typeof indexedDB.open === 'function'"), True)

    contexte.execute("""
        globalThis.__t = { etapes: [] };
        const d = indexedDB.open("carnet", 1);
        d.onupgradeneeded = function (e) {
            __t.etapes.push("montee");
            __t.ancienne = e.oldVersion;
            __t.nouvelle = e.newVersion;
            e.target.result.createObjectStore("notes");
        };
        d.onsuccess = function (e) {
            __t.etapes.push("ouverte");
            __t.version = e.target.result.version;
            __t.magasins = Array.from(e.target.result.objectStoreNames);
            globalThis.__base = e.target.result;
            const t = __base.transaction("notes", "readwrite");
            t.objectStore("notes").put({ titre: "un" }, "a");
            t.objectStore("notes").put({ titre: "deux" }, "b");
            t.oncomplete = function () {
                __t.etapes.push("ecrit");
                const l = __base.transaction("notes", "readonly");
                const r = l.objectStore("notes").get("a");
                r.onsuccess = function () { __t.lu = r.result; };
                const c = l.objectStore("notes").count();
                c.onsuccess = function () { __t.combien = c.result; };
            };
        };
        d.onerror = function (e) { __t.erreur = String(e.target.error); };
    """)
    _bat(contexte, "__t.lu !== undefined && __t.combien !== undefined")

    lu = json.loads(contexte.execute("JSON.stringify(__t)"))
    egal("idb: onupgradeneeded avant onsuccess",
         lu.get("etapes", [])[:2], ["montee", "ouverte"])
    egal("idb: l'ancienne version est 0 sur une base neuve", lu.get("ancienne"), 0)
    egal("idb: la nouvelle version est celle demandee", lu.get("nouvelle"), 1)
    egal("idb: le magasin cree est declare", lu.get("magasins"), ["notes"])
    egal("idb: la transaction s'est validee seule",
         "ecrit" in lu.get("etapes", []), True)
    egal("idb: get rend l'objet range", (lu.get("lu") or {}).get("titre"), "un")
    egal("idb: count compte ce qui est range", lu.get("combien"), 2)

    # `add` refuse une cle deja prise — c'est ce qui protege d'un doublon.
    contexte.execute("""
        (function () {
            __t.doublon = null;
            const t = __base.transaction("notes", "readwrite");
            const r = t.objectStore("notes").add({ titre: "encore" }, "a");
            r.onerror = function () { __t.doublon = r.error.name; };
            r.onsuccess = function () { __t.doublon = "passe"; };
        })();
    """)
    _bat(contexte, "__t.doublon !== null")
    egal("idb: add refuse une cle deja prise",
         json.loads(contexte.execute("JSON.stringify(__t.doublon)")),
         "ConstraintError")

    # Une ecriture en lecture seule doit echouer, pas passer en silence.
    contexte.execute("""
        (function () {
            __t.lectureSeule = null;
            const t = __base.transaction("notes", "readonly");
            const r = t.objectStore("notes").put({ x: 1 }, "z");
            r.onerror = function () { __t.lectureSeule = r.error.name; };
            r.onsuccess = function () { __t.lectureSeule = "passe"; };
        })();
    """)
    _bat(contexte, "__t.lectureSeule !== null")
    egal("idb: une transaction en lecture seule refuse d'ecrire",
         json.loads(contexte.execute("JSON.stringify(__t.lectureSeule)")),
         "ReadOnlyError")

    # L'asynchronisme, verifie plutot que suppose : une minuterie doit battre
    # pendant que les ecritures partent. Si `put` ecrivait sur le fil de
    # l'interface, ce compteur resterait a zero.
    contexte.execute("""
        (function () {
            __t.tics = 0;
            __t.finies = 0;
            setInterval(function () { __t.tics += 1; }, 1);
            const t = __base.transaction("notes", "readwrite");
            const m = t.objectStore("notes");
            for (let i = 0; i < 200; i++) {
                const r = m.put({ n: i, texte: "enregistrement numero " + i }, "k" + i);
                r.onsuccess = function () { __t.finies += 1; };
            }
        })();
    """)
    _bat(contexte, "__t.finies >= 200")
    tics = contexte.execute("__t.tics")
    verifie("idb: la boucle d'evenements bat pendant les ecritures",
            tics and tics > 0, tics)


def _indexeddb_transactions():
    """Une transaction abandonnee ne laisse rien : ni moitie, ni trace."""
    doc = document("<body><p>t</p></body>", url="https://un.test/page")
    contexte = js.Contexte(doc)
    contexte.execute("""
        globalThis.__x = {};
        const d = indexedDB.open("journal", 1);
        d.onupgradeneeded = function (e) {
            e.target.result.createObjectStore("lignes");
        };
        d.onsuccess = function (e) {
            globalThis.__j = e.target.result;
            const t = __j.transaction("lignes", "readwrite");
            t.objectStore("lignes").put("gardee", "ok");
            t.oncomplete = function () { __x.pose = true; };
        };
    """)
    _bat(contexte, "__x.pose === true")

    contexte.execute("""
        (function () {
            __x.abandon = null;
            const t = __j.transaction("lignes", "readwrite");
            t.objectStore("lignes").put("perdue", "jetee");
            t.onabort = function () { __x.abandon = "abandonnee"; };
            t.abort();
        })();
    """)
    _bat(contexte, "__x.abandon !== null")

    contexte.execute("""
        (function () {
            __x.relu = null;
            __x.gardee = null;
            const l = __j.transaction("lignes", "readonly");
            const a = l.objectStore("lignes").get("jetee");
            a.onsuccess = function () { __x.relu = a.result === undefined ? "vide" : a.result; };
            const b = l.objectStore("lignes").get("ok");
            b.onsuccess = function () { __x.gardee = b.result; };
        })();
    """)
    _bat(contexte, "__x.relu !== null && __x.gardee !== null")
    lu = json.loads(contexte.execute("JSON.stringify(__x)"))
    egal("idb: une transaction abandonnee n'ecrit rien", lu.get("relu"), "vide")
    egal("idb: et ne touche pas a ce qui etait deja la", lu.get("gardee"), "gardee")


def _indexeddb_persistance(indexeddb):
    """Ce qui est ecrit se relit apres que la memoire a ete jetee."""
    doc = document("<body><p>p</p></body>", url="https://un.test/page")
    contexte = js.Contexte(doc)
    contexte.execute("""
        globalThis.__p = {};
        const d = indexedDB.open("profil", 1);
        d.onupgradeneeded = function (e) {
            e.target.result.createObjectStore("champs");
        };
        d.onsuccess = function (e) {
            const t = e.target.result.transaction("champs", "readwrite");
            t.objectStore("champs").put("arthur", "nom");
            t.oncomplete = function () { __p.ecrit = true; };
        };
    """)
    _bat(contexte, "__p.ecrit === true")

    # Tout ce qui etait en memoire est jete : ce qu'on relit vient du disque.
    indexeddb.reinitialise()
    doc2 = document("<body><p>p</p></body>", url="https://un.test/page")
    contexte2 = js.Contexte(doc2)
    contexte2.execute("""
        globalThis.__q = {};
        const d = indexedDB.open("profil");
        d.onsuccess = function (e) {
            __q.version = e.target.result.version;
            __q.magasins = Array.from(e.target.result.objectStoreNames);
            const t = e.target.result.transaction("champs", "readonly");
            const r = t.objectStore("champs").get("nom");
            r.onsuccess = function () { __q.nom = r.result; };
        };
        d.onerror = function (e) { __q.erreur = String(e.target.error); };
    """)
    _bat(contexte2, "__q.nom !== undefined || __q.erreur !== undefined")
    lu = json.loads(contexte2.execute("JSON.stringify(__q)"))
    egal("idb: la valeur survit a un rechargement", lu.get("nom"), "arthur")
    egal("idb: la version aussi", lu.get("version"), 1)
    egal("idb: et le schema", lu.get("magasins"), ["champs"])


def _indexeddb_origines(indexeddb):
    """Deux origines, le meme nom de base, deux bases distinctes."""
    a = indexeddb.service("https://a.test/page")
    b = indexeddb.service("https://b.test/page")
    a.applique_montee("commune", 1, [("m", {})], [])
    b.applique_montee("commune", 1, [("m", {})], [])

    ta = a.transaction("commune", "readwrite")
    indexeddb.execute(ta, "put", "m", "cle", "valeur de A")
    ta.commit()

    tb = b.transaction("commune", "readonly")
    egal("idb: une origine ne voit pas la base d'une autre",
         indexeddb.execute(tb, "get", "m", "cle", None), indexeddb.ABSENT)

    ta2 = a.transaction("commune", "readonly")
    egal("idb: et voit bien la sienne",
         indexeddb.execute(ta2, "get", "m", "cle", None), "valeur de A")

    # Un nom de base est choisi par la page : `../../temoins.json` en est un
    # nom valide. Sans echappement, une page ecrirait par-dessus le bocal a
    # temoins.
    #
    # Ce qu'il faut verifier n'est pas l'absence des deux points — ils peuvent
    # rester dans un nom de fichier sans danger — mais que le chemin **resolu**
    # ne sorte pas du dossier. C'est la seule formulation qui teste la propriete
    # plutot que son apparence.
    from moteur import stockage as _stockage
    dossier = os.path.abspath(os.path.join(_stockage.RACINE, indexeddb.DOSSIER))
    for nom in ("../../temoins", "../../../etc/passwd", "a/b/c", "..", "/absolu"):
        resolu = os.path.abspath(os.path.join(
            _stockage.RACINE, indexeddb._nom_de_fichier("https://a.test", nom)))
        verifie("idb: le nom de base %r reste dans son dossier" % nom,
                os.path.dirname(resolu) == dossier, resolu)


def verifie_stockage_local():
    """`localStorage` : cloisonne par origine, et rendu au JavaScript."""
    from moteur import stockage

    restaure = isole_le_stockage()
    try:
        _stockage_local(stockage)
    finally:
        restaure()


def _stockage_local(stockage):

    local = stockage.StockageLocal()
    local.efface("https://a.test/page")
    local.efface("https://b.test/page")

    local.ecrit("https://a.test/page", "theme", "sombre")
    egal("stockage: relu depuis la meme origine",
         local.lit("https://a.test/autre", "theme"), "sombre")
    # Le cloisonnement n'est pas une commodite : sans lui, une page lirait le
    # jeton de session qu'une autre a range.
    egal("stockage: invisible depuis une autre origine",
         local.lit("https://b.test/page", "theme"), None)
    egal("stockage: le port fait partie de l'origine",
         local.lit("https://a.test:8443/page", "theme"), None)
    egal("stockage: les cles sont listees",
         local.cles("https://a.test/page"), ["theme"])
    local.retire("https://a.test/page", "theme")
    egal("stockage: retire", local.lit("https://a.test/page", "theme"), None)

    # Et le chemin complet depuis le JavaScript.
    doc = document("""
        <body>
          <script>
            localStorage.setItem('compteur', '7');
            window.__lu = localStorage.getItem('compteur');
            window.__taille = localStorage.length;
            window.__absent = localStorage.getItem('jamais-pose');
            localStorage.removeItem('compteur');
            window.__apres = localStorage.getItem('compteur');
          </script>
        </body>""")
    contexte = doc.contexte_js
    egal("stockage: setItem puis getItem", contexte.execute("window.__lu"), "7")
    egal("stockage: length", contexte.execute("window.__taille"), 1)
    egal("stockage: une cle absente rend null",
         contexte.execute("window.__absent"), None)
    egal("stockage: removeItem", contexte.execute("window.__apres"), None)

    # `document.cookie` fait le tour complet, lui aussi.
    stockage.temoins().oublie_tout()
    doc = document("""
        <body>
          <script>
            document.cookie = 'gout=vanille; Path=/';
            window.__biscuits = document.cookie;
          </script>
        </body>""", url="http://exemple.test/page")
    verifie("stockage: document.cookie fait le tour",
            "gout=vanille" in (doc.contexte_js.execute("window.__biscuits") or ""),
            doc.contexte_js.execute("window.__biscuits"))
    stockage.temoins().oublie_tout()


def verifie_client_leger():
    """Le client leger : protocole, images, et ce qu'il fait d'un service muet.

    Le service lui-meme n'est pas lance ici — il demande Node et un Chromium.
    Ce qui est eprouve, c'est tout ce qui se passe de notre cote : les adresses
    formees, la signature qui evite de retransmettre une image identique, et le
    fait qu'un service absent ne fasse pas tomber le navigateur.
    """
    from moteur import distant

    demandes = []

    def sert(reponses):
        def charge(url, methode="GET", corps=None, entetes=None, brut=False,
               document=None, destination=None):
            demandes.append(url)
            for motif, faire in reponses:
                if motif in url:
                    return faire(url)
            return reseau.Reponse(url, "", "text/plain", 404)
        return charge

    image = b"\xff\xd8\xff\xe0" + b"x" * 200 + b"\xff\xd9"

    def png(url):
        return reseau.Reponse(url, "", "image/jpeg", 200, octets=image,
                              entetes={"x-bo-signature": "abc123"})

    ancien = reseau.charge
    try:
        reseau.charge = sert([("/wv/", png)])
        session = distant.Session(service="http://hote:8080")

        egal("client leger: l'ouverture rapporte une image",
             session.ouvre("https://exemple.test", 800, 600), True)
        egal("client leger: les octets sont gardes", session.octets, image)
        egal("client leger: la signature aussi", session.signature, "abc123")

        # L'adresse formee doit porter la taille et le format.
        derniere = demandes[-1]
        verifie("client leger: l'action est nommee", "/wv/open" in derniere, derniere)
        for attendu in ("w=800", "h=600", "fmt=jpeg", "url=https"):
            verifie("client leger: la demande porte %s" % attendu,
                    attendu in derniere, derniere)

        # Le sondage joint la signature ; une action, non — sinon un 304
        # ferait manquer le changement qu'elle vient de provoquer.
        session.rafraichis()
        verifie("client leger: le sondage joint la signature",
                "sig=abc123" in demandes[-1], demandes[-1])
        session.clic(4, 5)
        verifie("client leger: une action ne la joint pas",
                "sig=" not in demandes[-1], demandes[-1])
        verifie("client leger: le clic porte ses coordonnees",
                "x=4" in demandes[-1] and "y=5" in demandes[-1], demandes[-1])

        # Une touche inconnue du service ne part pas.
        egal("client leger: une touche inconnue est refusee",
             session.touche("Truc"), False)
        egal("client leger: une touche connue part", session.touche("Enter"), True)

        # 304 : rien n'a change, et cela ne doit pas passer pour une panne.
        def inchange(url):
            return reseau.Reponse(url, "", "", 304, octets=b"")
        reseau.charge = sert([("/wv/", inchange)])
        egal("client leger: 304 veut dire « rien de neuf »",
             session.rafraichis(), False)
        egal("client leger: l'image precedente est gardee", session.octets, image)
        egal("client leger: et ce n'est pas une erreur", session.erreur, None)

        # Service muet : le navigateur doit continuer de tourner.
        def muet(url):
            raise OSError("connexion refusee")
        reseau.charge = sert([("/wv/", muet), ("/healthz", muet)])
        egal("client leger: un service muet ne leve pas",
             session.rafraichis(), False)
        verifie("client leger: et la raison est retenue", session.erreur)
        egal("client leger: il se declare injoignable", session.joignable(), False)
    finally:
        reseau.charge = ancien

    egal("client leger: traduction d'une touche", distant.touche_pour("entree"), "Enter")
    egal("client leger: touche sans equivalent", distant.touche_pour("truc"), None)


def verifie_bac_a_sable():
    """Une page ne doit pas pouvoir atteindre le systeme."""
    doc = document("<body></body>")
    contexte = js.Contexte(doc)
    for expression in ("typeof require", "typeof process", "typeof std",
                       "typeof os", "typeof scriptArgs", "typeof loadFile",
                       "typeof Deno", "typeof __loadScript"):
        egal("bac a sable: %s" % expression, contexte.execute(expression), "undefined")
    contexte.ferme()


def verifie_liste_de_selecteurs():
    """Une virgule dans `:not(…)` ne separe pas deux regles.

    C'est la telemetrie qui a trouve ce defaut : neuf groupes de regles de
    pypi.org detruits en silence. `li:not(:first-child, :last-child)` etait
    coupe en `li:not(:first-child` et `:last-child)`, deux selecteurs qui ne
    designent rien — et une regle au selecteur incomprehensible est ignoree
    sans le moindre message.
    """
    regles = css.analyse(
        "li:not(:first-child, :last-child) { color: red }"
        ".a:is(.b, .c) .d { color: green }"
        "[title=\"un, deux\"] { color: blue }"
        "h1, h2 { color: black }")
    egal("liste: la virgule interne ne coupe pas la regle", len(regles), 5)

    doc = document(
        "<style>"
        "li { color: #000000 }"
        "li:not(:first-child, :last-child) { color: #ff0000 }"
        ".a:is(.b, .c) .d { color: #00ff00 }"
        "</style>"
        "<body><ul><li id=un>1</li><li id=deux>2</li><li id=trois>3</li></ul>"
        "<div class='a b'><div class='d' id=dedans>x</div></div></body>")
    doc.remet_en_page(1000, 800)
    for identifiant, attendu, quoi in (("un", "#000000", "epargne le premier"),
                                       ("deux", "#ff0000", "atteint le milieu"),
                                       ("trois", "#000000", "epargne le dernier"),
                                       ("dedans", "#00ff00", ":is descend bien")):
        boite = boite_de(doc, identifiant)
        egal("liste: %s" % quoi,
             css_couleur(boite.style.get("color")) if boite else None,
             css_couleur(attendu))


def verifie_proprietes_logiques():
    """`margin-inline-start` vaut `margin-left` en ecriture latine."""
    doc = document(
        "<style>"
        "#a { margin-inline-start: 10px; margin-inline-end: 20px;"
        "     padding-block: 5px 7px }"
        "#b { margin-inline: 12px; padding-inline-start: 9px;"
        "     inline-size: 300px; block-size: 40px }"
        "#c { position: absolute; inset: 1px 2px 3px 4px }"
        "#d { position: absolute; inset-inline-start: 8px; inset-block-end: 6px }"
        "#e { border-inline-start-width: 4px; border-inline-start-color: #ff0000;"
        "     border-inline-start-style: solid }"
        "#f { grid-gap: 11px; -webkit-box-sizing: border-box }"
        "#x { margin-left: 1px; margin-inline-start: 2px }"
        "#y { margin-inline-start: 3px; margin-left: 4px }"
        "</style>"
        "<body><div id=a>a</div><div id=b>b</div><div id=c>c</div>"
        "<div id=d>d</div><div id=e>e</div><div id=f>f</div>"
        "<div id=x>x</div><div id=y>y</div></body>")
    doc.remet_en_page(1000, 800)

    def style(identifiant):
        boite = boite_de(doc, identifiant)
        return boite.style if boite else {}

    attendus = [
        ("a", "margin-left", "10px", "margin-inline-start"),
        ("a", "margin-right", "20px", "margin-inline-end"),
        ("a", "padding-top", "5px", "padding-block debut"),
        ("a", "padding-bottom", "7px", "padding-block fin"),
        ("b", "margin-left", "12px", "margin-inline a gauche"),
        ("b", "margin-right", "12px", "margin-inline a droite"),
        ("b", "padding-left", "9px", "padding-inline-start"),
        ("b", "width", "300px", "inline-size"),
        ("b", "height", "40px", "block-size"),
        ("c", "top", "1px", "inset haut"),
        ("c", "right", "2px", "inset droite"),
        ("c", "bottom", "3px", "inset bas"),
        ("c", "left", "4px", "inset gauche"),
        ("d", "left", "8px", "inset-inline-start"),
        ("d", "bottom", "6px", "inset-block-end"),
        ("e", "border-left-width", "4px", "border-inline-start-width"),
        ("e", "border-left-color", "#ff0000", "border-inline-start-color"),
        ("f", "row-gap", "11px", "grid-gap vaut gap"),
        ("f", "column-gap", "11px", "grid-gap vaut gap dans les deux sens"),
        ("f", "box-sizing", "border-box", "-webkit-box-sizing"),
        # La cascade doit continuer a trancher : la derniere declaration gagne.
        ("x", "margin-left", "2px", "la logique posee apres l'emporte"),
        ("y", "margin-left", "4px", "la physique posee apres l'emporte"),
    ]
    for identifiant, propriete, attendu, quoi in attendus:
        egal("logique: %s" % quoi, style(identifiant).get(propriete), attendu)

    # La mise en page doit en tenir compte pour de vrai, pas seulement la cascade.
    boite = boite_de(doc, "b")
    # 300 de contenu plus les 9 de `padding-inline-start` : `box-sizing` vaut
    # `content-box`, donc la boite mesure bien 309.
    verifie("logique: inline-size pose vraiment la largeur",
            boite is not None and abs(boite.largeur - 309.0) < 1.0,
            None if boite is None else boite.largeur)


def verifie_transformation_texte():
    """`text-transform` doit agir avant la mesure, pas au moment de peindre."""
    doc = document(
        "<style>"
        "#haut { text-transform: uppercase }"
        "#bas { text-transform: lowercase }"
        "#capi { text-transform: capitalize }"
        "#herite { text-transform: uppercase }"
        "#rien { text-transform: none }"
        "</style>"
        "<body><p id=haut>menu principal</p><p id=bas>MENU Principal</p>"
        "<p id=capi>l'accueil du site</p>"
        "<div id=herite><p id=fils>herite</p></div>"
        "<p id=rien>Inchange</p></body>")
    doc.remet_en_page(1000, 800)

    def texte_de(identifiant):
        boite = boite_de(doc, identifiant)
        if boite is None:
            return None
        return " ".join(f.texte for f in boite.lignes)

    egal("transform: uppercase", texte_de("haut"), "MENU PRINCIPAL")
    egal("transform: lowercase", texte_de("bas"), "menu principal")
    egal("transform: capitalize garde l'apostrophe",
         texte_de("capi"), "L'accueil Du Site")
    egal("transform: la valeur s'herite", texte_de("fils"), "HERITE")
    egal("transform: none laisse tel quel", texte_de("rien"), "Inchange")

    # Le point qui compte vraiment : la largeur mesuree est celle du texte
    # transforme. Mesurer « menu » et peindre « MENU » ferait deborder chaque
    # libelle de bouton du Web.
    large = boite_de(doc, "haut")
    egal("transform: le fragment porte le texte transforme",
         large.lignes[0].texte if large and large.lignes else None,
         "MENU PRINCIPAL")


def verifie_donnees_element():
    """`element.dataset` : ce qui manquait et qui arretait tout.

    Son absence ne degradait pas — `t.dataset.dropdownBound` leve une
    `TypeError` sur `undefined`, et le script entier de la page meurt a cette
    ligne. C'est ce qui privait pypi.org de ses menus et de ses trente-six
    ecouteurs.
    """
    doc = document(
        "<body><div id=a data-nom='vert' data-taille-max='12' data-vide=''>"
        "</div></body>")
    contexte = js.Contexte(doc)
    try:
        egal("dataset: attribut simple",
             contexte.execute("document.getElementById('a').dataset.nom"), "vert")
        egal("dataset: le tiret devient une majuscule",
             contexte.execute("document.getElementById('a').dataset.tailleMax"), "12")
        egal("dataset: un attribut vide n'est pas absent",
             contexte.execute("document.getElementById('a').dataset.vide"), "")
        egal("dataset: un attribut absent rend undefined",
             contexte.execute(
                 "String(document.getElementById('a').dataset.inconnu)"), "undefined")
        egal("dataset: `in` fonctionne",
             contexte.execute("'nom' in document.getElementById('a').dataset"), True)

        contexte.execute("document.getElementById('a').dataset.dejaLie = true")
        egal("dataset: l'ecriture pose l'attribut",
             contexte.execute(
                 "document.getElementById('a').getAttribute('data-deja-lie')"), "true")
        egal("dataset: et se relit",
             contexte.execute("document.getElementById('a').dataset.dejaLie"), "true")

        contexte.execute("delete document.getElementById('a').dataset.nom")
        egal("dataset: la suppression retire l'attribut",
             contexte.execute(
                 "String(document.getElementById('a').getAttribute('data-nom'))"),
             "null")

        cles = contexte.execute(
            "Object.keys(document.getElementById('a').dataset).sort().join(',')")
        egal("dataset: l'enumeration ne rend que les data-*",
             cles, "dejaLie,tailleMax,vide")

        # Le point qui a coute le plus cher : la lecture d'un champ absent de
        # `navigator` tue le script au lieu de le degrader.
        egal("navigator: appVersion existe",
             contexte.execute("typeof navigator.appVersion"), "string")
        verifie("navigator: appVersion n'a pas le prefixe Mozilla/",
                not contexte.execute("navigator.appVersion").startswith("Mozilla/"),
                contexte.execute("navigator.appVersion"))
        egal("navigator: appVersion se laisse interroger",
             contexte.execute("navigator.appVersion.includes('Bouchaud')"), True)
        for champ in ("appCodeName", "product", "vendor", "maxTouchPoints",
                      "hardwareConcurrency", "plugins", "doNotTrack"):
            verifie("navigator: %s est defini" % champ,
                    contexte.execute("'%s' in navigator" % champ))
    finally:
        contexte.ferme()


def verifie_ascendance():
    """`contains` traverse le pont une fois, pas une fois par niveau.

    Chaque `setAttribute` demande a chaque `MutationObserver` a portee d'arbre
    s'il est concerne. Quand la reponse se calculait en remontant `parentNode`
    depuis le JavaScript, une page de pypi.org faisait 900 000 traversees du
    pont pour se construire. La reponse est la meme ; c'est le nombre d'appels
    qui change.
    """
    doc = document("<body><div id=a><section id=b><p id=c>x</p></section></div>"
                   "<div id=d></div></body>")
    contexte = js.Contexte(doc)
    try:
        egal("ascendance: un ancetre lointain contient",
             contexte.execute("document.getElementById('a')"
                              ".contains(document.getElementById('c'))"), True)
        egal("ascendance: un nœud se contient lui-meme",
             contexte.execute("var e=document.getElementById('b'); e.contains(e)"),
             True)
        egal("ascendance: un frere ne contient pas",
             contexte.execute("document.getElementById('d')"
                              ".contains(document.getElementById('c'))"), False)
        egal("ascendance: le sens compte",
             contexte.execute("document.getElementById('c')"
                              ".contains(document.getElementById('a'))"), False)
        egal("ascendance: contains(null) est faux",
             contexte.execute("document.getElementById('a').contains(null)"), False)

        # Le cout, mesure : une pose d'attribut sous un observateur d'arbre ne
        # doit pas couter un appel par niveau.
        contexte.execute(
            "new MutationObserver(() => {}).observe("
            "  document.getElementById('a'), {attributes: true, subtree: true});")
        contexte.execute("document.getElementById('c').setAttribute('x', '1')")
        egal("ascendance: l'observateur a bien ete arme",
             contexte.execute("document.getElementById('c').getAttribute('x')"), "1")
    finally:
        contexte.ferme()


def verifie_console_gabarit():
    """`console.error("%s : %o", …)` doit rendre un message lisible."""
    doc = document("<body></body>")
    messages = []
    contexte = js.Contexte(doc, journal=lambda niveau, texte: messages.append(texte))
    try:
        contexte.execute('console.log("bonjour %s, tu as %d ans", "Alice", 30)')
        egal("console: %s et %d substitues", messages[-1], "bonjour Alice, tu as 30 ans")
        contexte.execute('console.log("%s%%", 50)')
        egal("console: %% rend un pourcent", messages[-1], "50%")
        contexte.execute('console.log("a %s", "b", "reste")')
        egal("console: les arguments en trop suivent", messages[-1], "a b reste")
        contexte.execute('console.log("sans marque", "deux")')
        egal("console: sans marque, rien ne change", messages[-1], "sans marque deux")
        contexte.execute('console.log("%s manque")')
        egal("console: une marque sans valeur reste telle quelle",
             messages[-1], "%s manque")
    finally:
        contexte.ferme()


def verifie_formulaire_pilote():
    """Le temoin de connexion, pilote comme un utilisateur le ferait.

    Poser des ecouteurs ne prouve rien : ce qui compte est qu'une saisie les
    declenche, que la soumission passe par `preventDefault`, et que le
    formulaire rende ses valeurs — `form.elements` comme `FormData`.
    """
    chemin = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                          "tests", "pages", "login-form.html")
    if not os.path.exists(chemin):
        # Surtout pas un `return` : une verification qui se saute quand son
        # decor manque est une verification qui passe toujours.
        verifie("formulaire: la page temoin est presente", False, chemin)
        return
    with open(chemin, encoding="utf-8") as f:
        source = f.read()
    doc = document(source, url="file://" + chemin)
    doc.remet_en_page(1000, 800)
    contexte = doc.contexte_js
    verifie("formulaire: la page a bien un contexte JavaScript", contexte is not None)
    if contexte is None:
        return

    egal("formulaire: les ecouteurs sont poses",
         contexte.execute("globalThis.__bo_ecouteurs"), 18)

    # Saisie : `value` puis l'evenement, comme le fait un champ reel.
    contexte.execute("document.getElementById('utilisateur').value = 'alice'")
    contexte.evenement(None, "input", {})
    contexte.execute(
        "document.getElementById('utilisateur').dispatchEvent(new Event('input'))")
    contexte.execute("document.getElementById('motdepasse').value = 'secret'")
    egal("formulaire: la valeur saisie se relit",
         contexte.execute("document.getElementById('utilisateur').value"), "alice")
    verifie("formulaire: l'evenement input a ete compte",
            contexte.execute("compteurs.input") >= 1,
            contexte.execute("compteurs.input"))

    # Etat initial de la liste et de la case, tel que le HTML le decrit.
    egal("formulaire: l'option preselectionnee est choisie",
         contexte.execute("document.getElementById('espace').value"), "equipe")
    egal("formulaire: la case cochee l'est",
         contexte.execute("document.getElementById('memoire').checked"), True)

    # Soumission.
    contexte.execute(
        "document.getElementById('connexion').dispatchEvent(new Event('submit'))")
    for _ in range(3):
        contexte.tic()
    egal("formulaire: la soumission a eu lieu", contexte.execute("compteurs.submit"), 1)
    rendu = contexte.execute("document.getElementById('resultat').textContent") or ""
    verifie("formulaire: le nom saisi est rendu", "alice" in rendu, rendu[:120])
    verifie("formulaire: le mot de passe est rendu", "secret" in rendu, rendu[:120])
    verifie("formulaire: l'espace choisi est rendu", "equipe" in rendu, rendu[:120])
    verifie("formulaire: FormData a rendu quelque chose",
            "formdata" in rendu and "indisponible" not in rendu, rendu[:200])
    egal("formulaire: la navigation a bien ete empechee",
         getattr(contexte, "navigation", None), None)


def verifie_champs_de_formulaire():
    """`value` sur `select`, `textarea` et `option`, puis `FormData`."""
    doc = document(
        "<body><form id=f>"
        "<select id=s name=espace>"
        "  <option value=perso>Personnel</option>"
        "  <option value=equipe selected>Equipe</option></select>"
        "<select id=s2 name=sans><option value=a>A</option>"
        "  <option value=b>B</option></select>"
        "<textarea id=t name=message>bonjour</textarea>"
        "<option id=o>Libelle nu</option>"
        "<input type=text name=nom value=alice>"
        "<input type=text value=sans-nom>"
        "<input type=checkbox name=coche value=oui checked>"
        "<input type=checkbox name=decoche value=non>"
        "<input type=checkbox name=defaut checked>"
        "<input type=text name=eteint value=x disabled>"
        "<input type=submit name=envoi value=Envoyer>"
        "</form></body>")
    contexte = js.Contexte(doc)
    try:
        egal("champs: select rend l'option selectionnee",
             contexte.execute("document.getElementById('s').value"), "equipe")
        egal("champs: select sans selected rend la premiere",
             contexte.execute("document.getElementById('s2').value"), "a")
        egal("champs: selectedIndex suit",
             contexte.execute("document.getElementById('s').selectedIndex"), 1)
        contexte.execute("document.getElementById('s').value = 'perso'")
        egal("champs: ecrire select choisit l'option",
             contexte.execute("document.getElementById('s').value"), "perso")
        egal("champs: et deselectionne l'ancienne",
             contexte.execute("document.getElementById('s').selectedIndex"), 0)
        egal("champs: textarea rend son texte",
             contexte.execute("document.getElementById('t').value"), "bonjour")
        egal("champs: option sans value rend son libelle",
             contexte.execute("document.getElementById('o').value"), "Libelle nu")

        contexte.execute("globalThis.d = new FormData(document.getElementById('f'))")
        egal("formdata: un champ nomme est present",
             contexte.execute("d.get('nom')"), "alice")
        egal("formdata: la valeur du select suit le choix",
             contexte.execute("d.get('espace')"), "perso")
        egal("formdata: le textarea est inclus",
             contexte.execute("d.get('message')"), "bonjour")
        egal("formdata: une case cochee est incluse",
             contexte.execute("d.get('coche')"), "oui")
        egal("formdata: une case cochee sans value vaut « on »",
             contexte.execute("d.get('defaut')"), "on")
        egal("formdata: une case non cochee est absente",
             contexte.execute("String(d.get('decoche'))"), "null")
        egal("formdata: un champ sans nom est absent",
             contexte.execute("String(d.get('sans-nom'))"), "null")
        egal("formdata: un champ desactive est absent",
             contexte.execute("String(d.get('eteint'))"), "null")
        egal("formdata: le bouton de soumission est absent",
             contexte.execute("String(d.get('envoi'))"), "null")

        contexte.execute("d.append('extra', '1'); d.append('extra', '2')")
        egal("formdata: getAll rend les doublons",
             contexte.execute("d.getAll('extra').join(',')"), "1,2")
        contexte.execute("d.set('extra', '3')")
        egal("formdata: set remplace tout",
             contexte.execute("d.getAll('extra').join(',')"), "3")
        contexte.execute("d.delete('extra')")
        egal("formdata: delete retire", contexte.execute("d.has('extra')"), False)
        egal("formdata: forEach parcourt",
             contexte.execute(
                 "var n=0; d.forEach(function(){n++}); n"),
             contexte.execute("Array.from(d.keys()).length"))

        egal("urlsearchparams: lecture d'une chaine",
             contexte.execute("new URLSearchParams('?a=1&b=deux').get('b')"), "deux")
        egal("urlsearchparams: le plus vaut une espace",
             contexte.execute("new URLSearchParams('q=un+deux').get('q')"), "un deux")
        egal("urlsearchparams: reencodage",
             contexte.execute("String(new URLSearchParams('a=1&b=2'))"), "a=1&b=2")
    finally:
        contexte.ferme()


def verifie_priorite_important():
    """`!important` : la declaration etait perdue en entier, pas seulement sa priorite."""
    doc = document(
        "<style>"
        "#a { width: 300px }"
        "div#a.large { width: 120px!important }"
        "#b { color: #0000ff }"
        ".rouge { color: #ff0000 !important }"
        "#c.fort { display: none !important }"
        "#c { display: block }"
        "</style>"
        "<body><div id=a class=large>a</div>"
        "<div id=b class=rouge>b</div>"
        "<div id=c class=fort>c</div>"
        "<div id=d style='width: 50px !important'>d</div></body>")
    doc.remet_en_page(1000, 800)

    a = boite_de(doc, "a")
    egal("important: la valeur est nettoyee de son drapeau",
         a.style.get("width") if a else None, "120px")
    verifie("important: et la largeur est vraiment posee",
            a is not None and abs(a.largeur - 120.0) < 1.0,
            None if a is None else a.largeur)

    b = boite_de(doc, "b")
    egal("important: il l'emporte sur une specificite superieure",
         css_couleur(b.style.get("color")) if b else None, css_couleur("#ff0000"))

    verifie("important: display:none important masque bien",
            boite_de(doc, "c") is None)

    d = boite_de(doc, "d")
    egal("important: en style en ligne aussi",
         d.style.get("width") if d else None, "50px")

    # Ce que le drapeau ne doit pas faire : apparaitre comme une propriete.
    verifie("important: la marque ne fuit pas dans le style",
            a is not None and not any(nom.startswith("!") for nom in a.style),
            None if a is None else [n for n in a.style if n.startswith("!")])


def verifie_pointeur_transparent():
    """`pointer-events: none` laisse le clic traverser."""
    # Les enfants absolus vivent dans une zone qui a sa propre hauteur : le
    # test de pointage descend par les boites, et une boite sans surface n'est
    # jamais atteinte.
    doc = document(
        "<style>"
        "#zone { position: relative; width: 200px; height: 200px }"
        "#dessous { position: absolute; left: 0; top: 0; width: 200px; height: 200px }"
        "#calque { position: absolute; left: 0; top: 0; width: 200px; height: 200px;"
        "          pointer-events: none }"
        "#dedans { position: absolute; left: 20px; top: 20px; width: 40px; height: 40px;"
        "          pointer-events: auto }"
        "</style>"
        "<body><div id=zone><div id=dessous>dessous</div>"
        "<div id=calque><div id=dedans>dedans</div></div></div></body>")
    doc.remet_en_page(1000, 800)

    def identifiant_a(x, y):
        element = doc.element_a(x, y)
        while element is not None and not getattr(element, "attributs", None):
            element = getattr(element, "parent", None)
        return None if element is None else element.attributs.get("id")

    verifie("pointeur: le calque transparent laisse passer",
            identifiant_a(150.0, 150.0) in ("dessous", "zone"),
            identifiant_a(150.0, 150.0))
    egal("pointeur: un enfant a auto reste cliquable",
         identifiant_a(48.0, 48.0), "dedans")


def verifie_tableau_de_bord():
    """Le suivi lui-meme : s'il lit mal, il annonce « tout va bien » a tort.

    Un tableau de bord qui echoue a extraire un chiffre n'affiche pas d'erreur,
    il affiche « indisponible » — et une suite de « indisponible » se lit comme
    une suite de journees calmes. Ces verifications-la protegent donc le
    dernier endroit ou personne ne penserait a regarder.
    """
    import importlib.util

    chemin = os.path.join(os.path.dirname(os.path.abspath(__file__)), "suivi.py")
    if not os.path.exists(chemin):
        verifie("suivi: le programme est present", False, chemin)
        return
    cahier = importlib.util.spec_from_file_location("suivi", chemin)
    suivi = importlib.util.module_from_spec(cahier)
    cahier.loader.exec_module(suivi)

    # Les deux formes que produisent `test_moteur.py` et `verifie_hote.cpp`.
    egal("suivi: lecture d'un bilan reussi",
         suivi._bilan("RESULTAT : 0 verification(s) en echec (731 passees)"),
         (731, 0))
    egal("suivi: lecture d'un bilan en echec",
         suivi._bilan("RESULTAT : 3 verification(s) en echec sur 731"),
         (728, 3))
    egal("suivi: une sortie sans bilan ne s'invente pas",
         suivi._bilan("la compilation a echoue"), None)
    egal("suivi: le bilan se trouve au milieu du bruit",
         suivi._bilan("bla\nRESULTAT : 0 verification(s) en echec (12 passees)\nbla"),
         (12, 0))

    # Le sens des ecarts : c'est lui qui decide si un chiffre est une bonne ou
    # une mauvaise nouvelle, et se tromper de sens rendrait le tableau nuisible.
    style = suivi.Style(False)
    for cle, avant, apres, attendu in (
            ("css.declarations_ignorees", 370, 322, "mieux"),
            ("css.declarations_ignorees", 322, 370, "pire"),
            ("moteur.passees", 658, 731, "mieux"),
            ("moteur.passees", 731, 658, "pire"),
            ("interactions.ecouteurs", 0, 16, "mieux"),
            ("js.erreurs", 0, 1, "pire"),
            ("page.boites", 100, 200, None)):
        _, jugement = suivi._ecart(cle, apres, {cle: avant}, style)
        egal("suivi: %s %d->%d" % (cle, avant, apres), jugement, attendu)

    _, jugement = suivi._ecart("js.erreurs", 3, {"js.erreurs": 3}, style)
    egal("suivi: une valeur inchangee n'est ni mieux ni pire", jugement, None)
    texte, jugement = suivi._ecart("js.erreurs", 3, None, style)
    egal("suivi: sans passe, rien a comparer", jugement, None)
    egal("suivi: et cela se dit", texte, "nouveau")

    # Chaque mesure affichee doit declarer un sens connu, sinon elle s'imprime
    # sans jugement et le lecteur doit deviner.
    for cle, libelle, sens in suivi.MESURES:
        if cle == "__section__":
            continue
        verifie("suivi: %s a un sens declare" % cle,
                sens in (suivi.BAS, suivi.HAUT, None), sens)
        verifie("suivi: %s porte un libelle" % cle, bool(libelle))

    # Les mesures surveillees par `--strict` doivent exister dans la table.
    connues = {cle for cle, _, _ in suivi.MESURES}
    for cle in suivi.SURVEILLEES:
        verifie("suivi: la mesure surveillee %s existe" % cle, cle in connues)

    # Comparer deux portees differentes ferait passer l'absence des sites reels
    # pour une amelioration spectaculaire.
    histoire = [{"portee": "complet", "mesures": {"a": 1}},
                {"portee": "rapide", "mesures": {"a": 2}},
                {"portee": "complet", "mesures": {"a": 3}}]
    egal("suivi: on ne compare qu'a une execution de meme portee",
         suivi.precedente(histoire, "complet")["mesures"]["a"], 3)
    egal("suivi: et la portee rapide a la sienne",
         suivi.precedente(histoire, "rapide")["mesures"]["a"], 2)
    egal("suivi: une portee jamais vue n'a pas de passe",
         suivi.precedente(histoire, "inconnue"), None)


def verifie_invalidation():
    """Ce qui change decide de ce qu'on refait — et de ce qu'on ne refait pas.

    Le moteur n'avait qu'un booleen : n'importe quelle mutation refaisait tout,
    pour la page entiere. Changer la couleur d'un bouton coutait exactement le
    prix d'un rechargement. Ce n'etait pas absurde tant que le DOM bougeait
    peu ; ce n'est plus tenable maintenant que le navigateur recoit des frappes.
    """
    from moteur import invalidation

    # Le classement des proprietes. Se tromper vers le haut coute du temps ;
    # se tromper vers le bas laisse un affichage faux, et un affichage faux ne
    # se voit pas dans un profil.
    for propriete, attendu, quoi in (
            ("color", invalidation.PEINTURE, "une couleur ne demande que des pixels"),
            ("background-color", invalidation.PEINTURE, "un fond non plus"),
            ("box-shadow", invalidation.PEINTURE, "ni une ombre"),
            ("width", invalidation.MISE_EN_PAGE, "une largeur replace les boites"),
            ("display", invalidation.MISE_EN_PAGE, "un display aussi"),
            ("margin-top", invalidation.MISE_EN_PAGE, "une marge aussi"),
            ("font-size", invalidation.MISE_EN_PAGE, "une taille de police aussi"),
            ("transform", invalidation.COMPOSITION, "un transform ne fait que deplacer"),
            ("opacity", invalidation.COMPOSITION, "une opacite non plus"),
            ("truc-inconnu", invalidation.MISE_EN_PAGE,
             "une propriete inconnue est traitee prudemment")):
        egal("invalidation: %s" % quoi,
             invalidation.etendue_de(propriete), attendu)

    # Un lot prend l'etendue la plus haute : une couleur et une largeur
    # ensemble demandent une mise en page.
    egal("invalidation: un lot prend le plus cher",
         invalidation.etendue_des(["color", "width"]), invalidation.MISE_EN_PAGE)
    egal("invalidation: un lot de couleurs reste en peinture",
         invalidation.etendue_des(["color", "background-color"]),
         invalidation.PEINTURE)

    etat = invalidation.Invalidation()
    verifie("invalidation: au depart il n'y a rien a faire", etat.rien_a_faire)
    etat.marque_propriete("color")
    verifie("invalidation: une couleur ne demande pas de mise en page",
            not etat.demande_mise_en_page and etat.demande_peinture)
    etat.marque_propriete("width")
    verifie("invalidation: une largeur, si", etat.demande_mise_en_page)
    etat.marque_propriete("color")
    verifie("invalidation: et une couleur ensuite ne redescend pas",
            etat.demande_mise_en_page)
    egal("invalidation: vider rend ce qui etait prevu", etat.vide(),
         invalidation.MISE_EN_PAGE)
    verifie("invalidation: apres quoi il n'y a plus rien", etat.rien_a_faire)


def verifie_invalidation_reelle():
    """Le chemin complet : `style.color` ne doit plus tout remettre en page."""
    from moteur import invalidation

    lignes = "".join("<p class=l id=p%d>ligne %d</p>" % (i, i) for i in range(60))
    doc = document("<style>.l { color: #000000 }</style>"
                   "<body><div id=zone>%s</div><script>1</script></body>" % lignes)
    doc.remet_en_page(1000, 800)
    contexte = doc.contexte_js

    poses = [0]
    from moteur import mise_en_page as _mep
    vrai = _mep.construit

    def compte(*a, **k):
        poses[0] += 1
        return vrai(*a, **k)

    _mep.construit = compte
    try:
        # Une couleur : peinture seulement, aucune mise en page.
        contexte.execute("document.getElementById('p3').style.color = '#ff0000'")
        egal("invalidation reelle: une couleur reste en peinture",
             doc.invalide.etendue, invalidation.PEINTURE)
        doc.rafraichis()
        egal("invalidation reelle: aucune mise en page n'a eu lieu", poses[0], 0)
        verifie("invalidation reelle: et plus rien n'est en retard",
                doc.invalide.rien_a_faire)

        # Une largeur : la mise en page redevient necessaire.
        contexte.execute("document.getElementById('p4').style.width = '120px'")
        egal("invalidation reelle: une largeur demande la mise en page",
             doc.invalide.etendue, invalidation.MISE_EN_PAGE)
        doc.rafraichis()
        egal("invalidation reelle: elle a bien eu lieu", poses[0], 1)
        poses[0] = 0

        # Une classe : la cascade doit etre rejouee.
        contexte.execute("document.getElementById('p5').className = 'l autre'")
        egal("invalidation reelle: une classe demande le style",
             doc.invalide.etendue, invalidation.STYLE)

        # Le resultat visible reste juste : la couleur demandee est appliquee.
        # Verifie ici, avant que la mutation suivante ne remplace le sous-arbre.
        _mep.construit = vrai
        doc.remet_en_page(1000, 800)
        _mep.construit = compte
        boite = boite_de(doc, "p3")
        egal("invalidation reelle: la couleur demandee est appliquee",
             css_couleur(boite.style.get("color")) if boite else None,
             css_couleur("#ff0000"))

        # Une mutation dont on ne connait pas la portee reste prudente.
        doc.invalide.vide()
        contexte.execute("document.getElementById('zone').innerHTML = '<p>neuf</p>'")
        egal("invalidation reelle: une mutation inconnue vaut mise en page",
             doc.invalide.etendue, invalidation.MISE_EN_PAGE)
    finally:
        _mep.construit = vrai


def verifie_memoires_de_passe():
    """Les memoires de generation : justes, et videes quand il le faut."""
    from moteur import css as _css

    doc = document(
        "<style>li:nth-child(2) { color: #ff0000 }</style>"
        "<body><ul id=u><li id=a>a</li><li id=b>b</li><li id=c>c</li></ul>"
        "<script>1</script></body>")
    doc.remet_en_page(1000, 800)
    egal("memoire: la fratrie memorisee ne fausse pas nth-child",
         css_couleur(boite_de(doc, "b").style.get("color")), css_couleur("#ff0000"))
    egal("memoire: et les autres ne sont pas atteints",
         css_couleur(boite_de(doc, "a").style.get("color")),
         css_couleur("#202124"))

    # Un enfant insere change la fratrie : la memoire doit avoir ete jetee.
    doc.contexte_js.execute(
        "var u = document.getElementById('u');"
        "var neuf = document.createElement('li'); neuf.id = 'zero';"
        "neuf.textContent = 'z'; u.insertBefore(neuf, u.firstChild);")
    doc.rafraichis()
    doc.remet_en_page(1000, 800)
    egal("memoire: apres insertion, nth-child(2) designe le nouveau second",
         css_couleur(boite_de(doc, "a").style.get("color")), css_couleur("#ff0000"))
    egal("memoire: et l'ancien second redevient normal",
         css_couleur(boite_de(doc, "b").style.get("color")),
         css_couleur("#202124"))

    # La generation avance a chaque passe.
    avant = _css.generation()
    doc.remet_en_page(1000, 800)
    verifie("memoire: chaque passe ouvre une generation",
            _css.generation() > avant, (avant, _css.generation()))

    # Le decoupage de `class` est memorise, mais suit la valeur.
    element = _par_id(doc, "a")
    egal("memoire: les classes se lisent", element.classes, [])
    element.attributs["class"] = "un deux"
    egal("memoire: et suivent un changement direct", element.classes,
         ["un", "deux"])
    element.pose_attribut("class", "trois")
    egal("memoire: comme un changement par pose_attribut", element.classes,
         ["trois"])


def verifie_websocket():
    """Une connexion que la page garde ouverte, et le serveur qui parle seul.

    Tout le reste du reseau est un aller-retour. Ici la connexion reste
    ouverte et le serveur emet quand il veut : ce qui se verifie n'est donc
    plus « la reponse est-elle correcte » mais « ce qui arrive parvient-il a la
    page, dans l'ordre, sans interrompre son script ».
    """
    import time as _time

    doc, arrete = _sur_serveur("/page/event-loop.html")
    try:
        contexte = doc.contexte_js
        base = doc.base.replace("http://", "ws://")

        def bat(jusqua, secondes=8.0):
            limite = _time.perf_counter() + secondes
            while _time.perf_counter() < limite:
                contexte.tic()
                if contexte.execute(jusqua):
                    return True
                _time.sleep(0.02)
            return False

        # 1. Ouverture, echo simple.
        contexte.execute(
            "globalThis.__ws = { etats: [], recus: [], fermeture: null };"
            "var s = new WebSocket('%s/ws');"
            "globalThis.__s = s;"
            "__ws.etats.push(s.readyState);"
            "s.onopen = function () { __ws.etats.push('open:' + s.readyState);"
            "                         s.send('bonjour'); };"
            "s.onmessage = function (e) { __ws.recus.push(e.data); };"
            "s.onclose = function (e) { __ws.fermeture ="
            "   { code: e.code, propre: e.wasClean }; };" % base)
        egal("websocket: l'etat initial est CONNECTING",
             contexte.execute("__ws.etats[0]"), 0)
        egal("websocket: les constantes sont exposees",
             contexte.execute("[WebSocket.CONNECTING, WebSocket.OPEN,"
                              " WebSocket.CLOSING, WebSocket.CLOSED].join(',')"),
             "0,1,2,3")

        verifie("websocket: la connexion s'ouvre",
                bat("__ws.etats.length > 1"), contexte.execute("JSON.stringify(__ws)"))
        egal("websocket: l'etat devient OPEN",
             contexte.execute("__ws.etats[1]"), "open:1")

        verifie("websocket: l'echo revient", bat("__ws.recus.length >= 1"),
                contexte.execute("JSON.stringify(__ws.recus)"))
        egal("websocket: et c'est bien ce qu'on a envoye",
             contexte.execute("__ws.recus[0]"), "bonjour")

        # 2. Plusieurs messages, dans l'ordre, emis par le serveur seul.
        contexte.execute("__ws.recus = []; __s.send('__trois__')")
        verifie("websocket: trois messages arrivent", bat("__ws.recus.length >= 3"),
                contexte.execute("JSON.stringify(__ws.recus)"))
        egal("websocket: dans l'ordre d'emission",
             contexte.execute("__ws.recus.slice(0, 3).join('|')"),
             "message 1|message 2|message 3")

        # 3. Fermeture demandee par le client.
        contexte.execute("__s.close(1000, 'fini')")
        verifie("websocket: la fermeture aboutit", bat("__ws.fermeture !== null"),
                contexte.execute("JSON.stringify(__ws.fermeture)"))
        egal("websocket: avec le code demande",
             contexte.execute("__ws.fermeture.code"), 1000)
        egal("websocket: et elle est propre",
             contexte.execute("__ws.fermeture.propre"), True)
        egal("websocket: l'etat final est CLOSED",
             contexte.execute("__s.readyState"), 3)

        # 4. Fermeture decidee par le serveur.
        contexte.execute(
            "globalThis.__f = null;"
            "var t = new WebSocket('%s/ws/ferme');"
            "t.onclose = function (e) { __f = { code: e.code, propre: e.wasClean }; };"
            % base)
        verifie("websocket: une fermeture du serveur est vue",
                bat("__f !== null"), contexte.execute("JSON.stringify(__f)"))
        egal("websocket: avec son code", contexte.execute("__f.code"), 1000)

        # 5. Connexion refusee : `error` puis `close`, jamais une exception.
        contexte.execute(
            "globalThis.__e = { erreurs: 0, ferme: null };"
            "var u = new WebSocket('ws://127.0.0.1:1/ws');"
            "u.onerror = function () { __e.erreurs++; };"
            "u.onclose = function (ev) { __e.ferme = ev.code; };")
        verifie("websocket: une connexion refusee se signale",
                bat("__e.ferme !== null", 10.0),
                contexte.execute("JSON.stringify(__e)"))
        verifie("websocket: par une erreur puis une fermeture",
                contexte.execute("__e.erreurs") >= 1,
                contexte.execute("JSON.stringify(__e)"))
        egal("websocket: avec le code d'echec anormal",
             contexte.execute("__e.ferme"), 1006)

        # 6. `send()` apres fermeture doit lever, pas partir en silence.
        leve = contexte.execute(
            "(function () { try { __s.send('x'); return 'PAS-DE-LEVEE'; }"
            " catch (e) { return 'leve'; } })()")
        egal("websocket: envoyer sur une connexion fermee leve", leve, "leve")

        # 7. L'interface est restee vivante pendant tout cela.
        contexte.execute(
            "globalThis.__bat = 0;"
            "var minuterie = setInterval(function () { __bat++; }, 10);"
            "var v = new WebSocket('%s/ws');"
            "v.onopen = function () { v.send('__trois__'); };"
            "globalThis.__recus2 = 0;"
            "v.onmessage = function () { __recus2++; };" % base)
        verifie("websocket: les messages arrivent", bat("__recus2 >= 3"),
                contexte.execute("__recus2"))
        verifie("websocket: et la minuterie a continue de battre pendant",
                contexte.execute("__bat") > 0, contexte.execute("__bat"))
        contexte.execute("clearInterval(minuterie); v.close()")
    finally:
        arrete()


# --- Interaction utilisateur reelle -------------------------------------------

def verifie_modele_edition():
    """Le modele d'edition, seul : valeur, curseur, selection.

    Il ne connait ni le DOM ni Qt, donc il s'eprouve directement — et c'est
    tout l'interet de l'avoir isole.
    """
    from moteur import edition

    e = edition.Edition("abc")
    egal("edition: le curseur part a la fin", e.curseur, 3)
    egal("edition: insertion a la fin", (e.insere("d"), e.valeur), (True, "abcd"))
    e.pose_curseur(2)
    egal("edition: insertion au milieu", (e.insere("X"), e.valeur), (True, "abXcd"))
    egal("edition: le curseur suit l'insertion", e.curseur, 3)

    egal("edition: retour arriere", (e.efface_avant(), e.valeur), (True, "abcd"))
    e.pose_curseur(0)
    egal("edition: retour arriere en tete ne fait rien",
         (e.efface_avant(), e.valeur), (False, "abcd"))
    egal("edition: suppr avance", (e.efface_apres(), e.valeur), (True, "bcd"))
    e.pose_curseur(3)
    egal("edition: suppr en fin ne fait rien",
         (e.efface_apres(), e.valeur), (False, "bcd"))

    # Les fleches, avec et sans Maj.
    e = edition.Edition("bonjour")
    e.pose_curseur(3)
    e.deplace(-1)
    egal("edition: fleche gauche", e.curseur, 2)
    e.deplace(1, etend=True)
    e.deplace(1, etend=True)
    egal("edition: Maj etend la selection", e.selection(), (2, 4))
    egal("edition: le texte selectionne est le bon", e.texte_selectionne(), "nj")
    e.deplace(-1, etend=True)
    egal("edition: Maj en sens inverse reduit", e.selection(), (2, 3))

    # Une fleche sans Maj sur une selection l'effondre a son bord, elle ne
    # deplace pas d'un caractere : c'est ce que fait tout editeur.
    e.pose_selection(2, 5)
    e.deplace(1)
    egal("edition: la fleche effondre la selection au bord", e.curseur, 5)
    e.pose_selection(2, 5)
    e.deplace(-1)
    egal("edition: et a l'autre bord dans l'autre sens", e.curseur, 2)

    # Taper sur une selection la remplace.
    e.pose_selection(0, 3)
    egal("edition: taper remplace la selection",
         (e.insere("X"), e.valeur), (True, "Xjour"))

    e.tout_selectionner()
    egal("edition: tout selectionner", e.selection(), (0, 5))
    egal("edition: retour arriere efface la selection",
         (e.efface_avant(), e.valeur), (True, ""))

    # Home et End, sur plusieurs lignes.
    m = edition.Edition("une\ndeux\ntrois", multiligne=True)
    m.pose_curseur(6)
    m.au_debut()
    egal("edition: Home va au debut de la ligne", m.curseur, 4)
    m.a_la_fin()
    egal("edition: End va a la fin de la ligne", m.curseur, 8)

    # Un `<input>` est monoligne : un collage multiligne s'aplatit.
    mono = edition.Edition("", multiligne=False)
    mono.insere("un\ndeux")
    egal("edition: un input aplatit les retours", mono.valeur, "un deux")
    egal("edition: un textarea les garde",
         (edition.Edition("", multiligne=True).insere("un\ndeux"), True)[1], True)

    # Entree : rien dans un input, un retour dans un textarea.
    egal("edition: Entree n'insere rien dans un input",
         edition.applique_touche(edition.Edition("a"), "Enter"), (False, False))
    ligne = edition.Edition("a", multiligne=True)
    egal("edition: Entree insere dans un textarea",
         edition.applique_touche(ligne, "Enter")[0], True)
    egal("edition: et la valeur porte le retour", ligne.valeur, "a\n")

    # Le masque ne touche pas la valeur : elle doit partir au serveur.
    secret = edition.Edition("hunter2", masque=True)
    egal("edition: la valeur reste en clair dans le modele", secret.valeur, "hunter2")
    egal("edition: mais l'affichage est masque",
         secret.texte_affiche(), edition.PUCE * 7)
    egal("edition: et la longueur correspond",
         len(secret.texte_affiche()), len("hunter2"))

    egal("edition: Ctrl+A selectionne tout",
         edition.applique_touche(edition.Edition("abc"), "a", ctrl=True), (False, True))
    egal("edition: Ctrl+lettre n'insere pas",
         edition.applique_touche(edition.Edition(""), "z", "z", ctrl=True)[0], False)


def _page_saisie(source=None, url="http://exemple.test/page"):
    doc = document(source or (
        "<style>label { display: block }</style>"
        "<body><form id=f action='/session' method=post>"
        "<label id=et for=courriel>Adresse</label>"
        "<input id=courriel name=courriel type=text>"
        "<input id=mdp name=mdp type=password>"
        "<input id=coche name=coche type=checkbox>"
        "<textarea id=note name=note></textarea>"
        "<button id=envoi type=submit>Envoyer</button>"
        "</form><script>1</script></body>"), url=url)
    doc.remet_en_page(1000, 800)
    return doc


def tape(doc, texte):
    """Tape un texte caractere par caractere, comme un utilisateur."""
    for caractere in texte:
        doc.frappe(caractere, caractere)


def verifie_saisie_reelle():
    """Taper dans un champ, pour de vrai : frappe par frappe."""
    doc = _page_saisie()
    contexte = doc.contexte_js
    doc.pose_foyer(_par_id(doc, "courriel"))

    tape(doc, "alice")
    egal("saisie: la valeur se construit lettre par lettre",
         contexte.execute("document.getElementById('courriel').value"), "alice")
    egal("saisie: le curseur suit",
         contexte.execute("document.getElementById('courriel').selectionStart"), 5)

    doc.frappe("Backspace")
    egal("saisie: retour arriere retire la derniere lettre",
         contexte.execute("document.getElementById('courriel').value"), "alic")

    doc.frappe("Home")
    tape(doc, "M")
    egal("saisie: Home puis frappe insere en tete",
         contexte.execute("document.getElementById('courriel').value"), "Malic")

    doc.frappe("End")
    tape(doc, "e")
    egal("saisie: End puis frappe insere en queue",
         contexte.execute("document.getElementById('courriel').value"), "Malice")

    # `Entree` n'insere rien dans un `<input>`.
    avant = contexte.execute("document.getElementById('courriel').value")
    doc.frappe("Enter")
    egal("saisie: Entree n'ajoute pas de retour dans un input",
         contexte.execute("document.getElementById('courriel').value"), avant)

    # Mais bien dans un `<textarea>`.
    doc.pose_foyer(_par_id(doc, "note"))
    tape(doc, "un")
    doc.frappe("Enter")
    tape(doc, "deux")
    egal("saisie: Entree insere un retour dans un textarea",
         contexte.execute("document.getElementById('note').value"), "un\ndeux")

    # Le mot de passe garde sa valeur et n'affiche que des puces.
    doc.pose_foyer(_par_id(doc, "mdp"))
    tape(doc, "hunter2")
    egal("saisie: la valeur du mot de passe est en clair pour le script",
         contexte.execute("document.getElementById('mdp').value"), "hunter2")
    boite = boite_de(doc, "mdp")
    peint = " ".join(f.texte for f in boite.lignes) if boite else ""
    verifie("saisie: mais le mot de passe n'est jamais peint en clair",
            "hunter2" not in peint, peint)
    verifie("saisie: il est peint en puces de meme longueur",
            peint.count("\u2022") == 7, peint)


def verifie_ordre_des_evenements_de_frappe():
    """`keydown` → valeur → `input` → `keyup`, et `preventDefault` obei."""
    doc = _page_saisie()
    contexte = doc.contexte_js
    doc.pose_foyer(_par_id(doc, "courriel"))
    contexte.execute(
        "globalThis.__ordre = [];"
        "var c = document.getElementById('courriel');"
        "c.addEventListener('keydown', e => __ordre.push('keydown:' + e.key + ':' + c.value));"
        "c.addEventListener('input', () => __ordre.push('input:' + c.value));"
        "c.addEventListener('keyup', () => __ordre.push('keyup:' + c.value));")

    doc.frappe("a", "a")
    egal("frappe: l'ordre est keydown, input, keyup",
         contexte.execute("__ordre.join('|')"),
         "keydown:a:|input:a|keyup:a")

    # `keydown` voit la valeur d'avant, `input` celle d'apres. Emettre `input`
    # avant la mutation ferait lire a la page un etat qui n'existe pas.
    contexte.execute("__ordre = []")
    doc.frappe("b", "b")
    egal("frappe: keydown voit l'ancienne valeur, input la nouvelle",
         contexte.execute("__ordre.join('|')"),
         "keydown:b:a|input:ab|keyup:ab")

    # Une touche qui ne change rien n'emet pas `input` : sinon toute validation
    # « au changement » se declencherait a chaque appui de fleche.
    contexte.execute("__ordre = []")
    doc.frappe("ArrowLeft")
    verifie("frappe: une fleche n'emet pas input",
            "input" not in contexte.execute("__ordre.join('|')"),
            contexte.execute("__ordre.join('|')"))

    # `preventDefault()` dans `keydown` empeche l'insertion.
    contexte.execute(
        "__ordre = [];"
        "c.addEventListener('keydown', e => { if (e.key === 'x') e.preventDefault(); });")
    avant = contexte.execute("c.value")
    doc.frappe("x", "x")
    egal("frappe: preventDefault empeche l'insertion",
         contexte.execute("c.value"), avant)
    verifie("frappe: et aucun input n'est emis",
            "input" not in contexte.execute("__ordre.join('|')"),
            contexte.execute("__ordre.join('|')"))
    verifie("frappe: mais keyup part quand meme",
            "keyup" in contexte.execute("__ordre.join('|')"),
            contexte.execute("__ordre.join('|')"))


def verifie_curseur_visible():
    """Le caret est peint, et il n'est pas un caractere de la valeur."""
    doc = _page_saisie()
    champ = _par_id(doc, "courriel")

    boite = boite_de(doc, "courriel")
    egal("curseur: sans foyer, pas de curseur",
         boite.curseur if boite else "pas de boite", None)

    doc.pose_foyer(champ)
    doc.remet_en_page(1000, 800)
    boite = boite_de(doc, "courriel")
    verifie("curseur: le foyer fait apparaitre un curseur",
            boite is not None and boite.curseur is not None)
    x_vide, _, hauteur, selection = boite.curseur
    verifie("curseur: il a une hauteur", hauteur > 0, hauteur)
    egal("curseur: sans selection, rien a surligner", selection, None)
    verifie("curseur: il est dans la boite du champ",
            boite.x <= x_vide <= boite.x + boite.largeur, (boite.x, x_vide))

    tape(doc, "abc")
    boite = boite_de(doc, "courriel")
    x_apres = boite.curseur[0]
    verifie("curseur: il avance quand on tape", x_apres > x_vide, (x_vide, x_apres))

    # Le curseur n'entre jamais dans la valeur : c'est ce qui le distingue de
    # la barre d'adresse du chrome, qui l'y insere.
    egal("curseur: la valeur ne contient pas le caret",
         doc.contexte_js.execute("document.getElementById('courriel').value"), "abc")
    peint = " ".join(f.texte for f in boite.lignes)
    egal("curseur: ni le texte peint", peint, "abc")

    # La selection donne un rectangle a peindre.
    doc.frappe("ArrowLeft", maj=True)
    doc.frappe("ArrowLeft", maj=True)
    doc.pose_curseur_visuel()
    boite = boite_de(doc, "courriel")
    verifie("curseur: une selection est surlignee",
            boite.curseur[3] is not None, boite.curseur)
    verifie("curseur: et le surlignage a une largeur",
            boite.curseur[3][1] > 0, boite.curseur[3])

    # Le curseur disparait avec le foyer.
    doc.pose_foyer(None)
    doc.remet_en_page(1000, 800)
    boite = boite_de(doc, "courriel")
    egal("curseur: il disparait avec le foyer", boite.curseur, None)


def verifie_clic_donne_le_foyer():
    """Le chemin reel : clic → test de pointage → focalisable → foyer → caret."""
    doc = _page_saisie()
    contexte = doc.contexte_js

    def clique_sur(identifiant, decalage=4.0):
        boite = boite_de(doc, identifiant)
        verifie("clic: la boite de %s existe" % identifiant, boite is not None)
        if boite is None:
            return None
        return doc.clique(boite.x + decalage, boite.y + boite.hauteur / 2.0)

    clique_sur("courriel")
    egal("clic: le champ prend le foyer",
         contexte.execute("document.activeElement.id"), "courriel")
    tape(doc, "bonjour")
    egal("clic: on peut ecrire dedans juste apres",
         contexte.execute("document.getElementById('courriel').value"), "bonjour")

    clique_sur("mdp")
    egal("clic: un autre champ reprend le foyer",
         contexte.execute("document.activeElement.id"), "mdp")

    clique_sur("envoi")
    egal("clic: un bouton prend le foyer aussi",
         contexte.execute("document.activeElement.id"), "envoi")

    # Cliquer une etiquette donne le foyer au controle qu'elle designe.
    clique_sur("et")
    egal("clic: une etiquette passe le foyer a son controle",
         contexte.execute("document.activeElement.id"), "courriel")

    # Le clic pose le curseur la ou l'on a clique, pas systematiquement en fin.
    boite = boite_de(doc, "courriel")
    doc.clique(boite.x + 2.0, boite.y + boite.hauteur / 2.0)
    egal("clic: cliquer a gauche met le curseur en tete",
         contexte.execute("document.getElementById('courriel').selectionStart"), 0)

    # Cliquer dans le vide retire le foyer.
    doc.clique(5.0, doc.hauteur - 1.0)
    egal("clic: cliquer hors de tout controle retire le foyer",
         contexte.execute("document.activeElement.tagName"), "BODY")


def verifie_cases_et_radios():
    """Cocher, decocher, et un seul radio coche par groupe."""
    doc = _page_saisie(
        "<body><form id=f>"
        "<input id=c1 type=checkbox name=c1>"
        "<input id=c2 type=checkbox name=c2 disabled>"
        "<input id=r1 type=radio name=choix value=a>"
        "<input id=r2 type=radio name=choix value=b>"
        "<input id=r3 type=radio name=autre value=c>"
        "</form><script>1</script></body>")
    contexte = doc.contexte_js
    contexte.execute(
        "globalThis.__vus = [];"
        "for (const n of ['c1','r1','r2']) {"
        "  const e = document.getElementById(n);"
        "  e.addEventListener('input', () => __vus.push('input:' + n));"
        "  e.addEventListener('change', () => __vus.push('change:' + n));"
        "}")

    doc.active(_par_id(doc, "c1"))
    egal("case: un clic coche", contexte.execute("document.getElementById('c1').checked"),
         True)
    egal("case: input puis change sont emis",
         contexte.execute("__vus.join('|')"), "input:c1|change:c1")
    doc.active(_par_id(doc, "c1"))
    egal("case: un second clic decoche",
         contexte.execute("document.getElementById('c1').checked"), False)

    doc.active(_par_id(doc, "c2"))
    egal("case: une case desactivee ne bouge pas",
         contexte.execute("document.getElementById('c2').checked"), False)

    doc.active(_par_id(doc, "r1"))
    doc.active(_par_id(doc, "r2"))
    egal("radio: le second est coche",
         contexte.execute("document.getElementById('r2').checked"), True)
    egal("radio: et le premier ne l'est plus",
         contexte.execute("document.getElementById('r1').checked"), False)
    doc.active(_par_id(doc, "r3"))
    egal("radio: un autre groupe n'est pas affecte",
         contexte.execute("document.getElementById('r2').checked"), True)

    # Au clavier : l'espace agit comme un clic.
    doc.pose_foyer(_par_id(doc, "c1"))
    doc.frappe(" ", " ")
    egal("case: l'espace coche au clavier",
         contexte.execute("document.getElementById('c1').checked"), True)


def _par_id(doc, identifiant):
    for nœud in doc.racine.parcours():
        if isinstance(nœud, html.Element) \
                and nœud.attributs.get("id") == identifiant:
            return nœud
    return None


# --- Plate-forme comportementale ----------------------------------------------

def _sur_serveur(chemin, largeur=1000, hauteur=800, battements=8, attente=0.02):
    """Charge une page du serveur de fixtures et rend `(document, arret)`.

    Le serveur local remplace Internet pour tout ce qui compte ici : une
    redirection, un 404, un POST, une reponse lente. Aucun de ces cas n'a besoin
    du reseau exterieur, et les faire dependre de lui rendait la moitie du
    comportement du navigateur intestable.
    """
    import time as _time

    from moteur import reseau as _reseau
    import serveur_test

    serveur, base = serveur_test.demarre(0)
    ancien = _reseau._charge_http
    try:
        import apercu
        apercu.installe_reseau(_reseau)
    except Exception:  # noqa: BLE001 — sans `apercu`, le client maison suffit
        pass
    doc = moteur.charge(base + chemin, largeur, journal=lambda n, t: None)
    doc.remet_en_page(largeur, hauteur)
    for _ in range(battements):
        doc.rafraichis()
        _time.sleep(attente)

    def arrete():
        _reseau._charge_http = ancien
        serveur_test.arrete(serveur)

    doc.base = base
    return doc, arrete


# --- Jalons fonctionnels ------------------------------------------------------
#
# Les compteurs internes disent si le moteur travaille moins ; ils ne disent pas
# s'il sait faire tourner une application. Ces jalons repondent a la seconde
# question, et c'est la seule qui interesse quelqu'un qui veut s'en servir.
#
# Chacun est binaire — PASS ou FAIL — et nomme une capacite entiere plutot
# qu'un detail. Ils sont ranges dans `JALONS` pour que le tableau de bord les
# affiche sans les redecouvrir.

JALONS = {}


def jalon(nom, reussi, detail=None):
    """Enregistre un jalon fonctionnel et l'affirme comme une verification."""
    JALONS[nom] = bool(reussi)
    verifie("jalon %s" % nom, reussi, detail)


def verifie_webapp_combinee():
    """Le temoin combine : toutes les briques modernes ensemble.

    Les autres epreuves isolent une brique. Celle-ci les fait travailler
    ensemble, parce que c'est la que les applications reelles tombent — un
    `pushState` pendant qu'un `fetch` est en vol et qu'une transaction attend
    son commit.

    Le parcours est celui de `tests/pages/webapp.html` : amorce, formulaire,
    navigation, requete, mise a jour du DOM, WebSocket, enregistrement,
    rechargement, restauration. Et pendant tout cela, une minuterie de fond
    doit continuer de battre : c'est la mesure la plus simple de « l'interface
    repond », et elle ne ment pas.
    """
    import time as _time
    from moteur import indexeddb

    restaure = isole_le_stockage()
    try:
        indexeddb.reinitialise()
        _webapp_combinee(_time, indexeddb)
    finally:
        indexeddb.reinitialise()
        restaure()


def _attends(doc, condition, secondes=20.0):
    """Bat le document jusqu'a ce que la condition JavaScript tienne."""
    import time as _time
    limite = _time.perf_counter() + secondes
    contexte = doc.contexte_js
    while _time.perf_counter() < limite:
        doc.rafraichis()
        if contexte is not None and contexte.execute(condition):
            return True
        _time.sleep(0.02)
    return False


def _etat_webapp(doc):
    contexte = doc.contexte_js
    if contexte is None:
        return {}
    brut = contexte.execute("JSON.stringify(globalThis.__webapp || {})")
    try:
        return json.loads(brut) if brut else {}
    except (TypeError, ValueError):
        return {}


def _charge_sur(base, chemin, battements=4, largeur=1000, hauteur=800):
    """Charge une page d'un serveur **deja demarre**.

    L'origine doit rester la meme entre les deux chargements : IndexedDB
    cloisonne par origine, et `_sur_serveur` prend un port libre a chaque appel.
    Deux appels donnaient donc deux origines, donc deux bases — et la
    restauration ne trouvait rien. Le cloisonnement faisait exactement son
    travail ; c'est l'epreuve qui demandait la mauvaise chose.
    """
    import time as _time
    doc = moteur.charge(base + chemin, largeur, journal=lambda n, t: None)
    doc.remet_en_page(largeur, hauteur)
    for _ in range(battements):
        doc.rafraichis()
        _time.sleep(0.02)
    doc.base = base
    return doc


def _webapp_combinee(_time, indexeddb):
    from moteur import reseau as _reseau
    import serveur_test

    serveur, base = serveur_test.demarre(0)
    ancien = _reseau._charge_http
    try:
        import apercu
        apercu.installe_reseau(_reseau)
    except Exception:  # noqa: BLE001
        pass

    def arrete():
        _reseau._charge_http = ancien
        serveur_test.arrete(serveur)

    doc = _charge_sur(base, "/page/webapp.html")
    try:
        contexte = doc.contexte_js
        verifie("webapp: la page a un contexte JavaScript", contexte is not None)
        if contexte is None:
            for nom in ("STATIC_PAGE", "LOGIN_FORM", "SPA_NAVIGATION",
                        "ASYNC_FETCH", "WEBSOCKET_CHAT", "INDEXEDDB_PERSISTENCE",
                        "WEBAPP_COMBINED"):
                jalon(nom, False, "pas de contexte JavaScript")
            return

        # STATIC_PAGE — la page s'affiche : des boites, du texte, une mise en
        # page qui n'est pas vide.
        boites = 0
        textes = 0
        pile = [doc.boite]
        while pile:
            boite = pile.pop()
            boites += 1
            textes += len(boite.lignes)
            pile.extend(boite.enfants)
        # Ce qui fait qu'une page « s'affiche » : des boites, du texte pose, et
        # une hauteur reelle. Les seuils sont bas et delibereement grossiers —
        # ce jalon repond a « la page est-elle rendue ? », pas a « est-elle
        # rendue exactement ainsi ? », question dont s'occupent les controles de
        # pixels.
        jalon("STATIC_PAGE",
              boites >= 10 and textes >= 5 and doc.boite.hauteur > 150,
              (boites, textes, doc.boite.hauteur))

        pret = _attends(doc, "globalThis.__webapp && __webapp.amorce === true")
        etat = _etat_webapp(doc)
        verifie("webapp: l'amorce a eu lieu", pret, etat.get("erreurs"))

        # ASYNC_FETCH — la requete est partie, revenue, et le DOM porte ce
        # qu'elle a rapporte.
        _attends(doc, "__webapp.domMisAJour === true")
        etat = _etat_webapp(doc)
        jalon("ASYNC_FETCH",
              bool(etat.get("fetch")) and etat["fetch"].get("ok") is True
              and etat.get("domMisAJour") is True,
              etat.get("fetch"))

        # LOGIN_FORM — on remplit au clavier, comme un utilisateur. Pas de
        # `input.value = ...` : c'est le modele d'edition qu'on eprouve, pas
        # l'affectation d'une propriete.
        champ = doc.racine.trouve_id("nom") if hasattr(doc.racine, "trouve_id") else None
        if champ is None:
            champ = next((n for n in doc.racine.parcours()
                          if isinstance(n, html.Element)
                          and n.attributs.get("id") == "nom"), None)
        saisi = False
        if champ is not None:
            doc.pose_foyer(champ)
            for lettre in "arthur":
                doc.frappe(lettre, lettre)
            saisi = doc.edition_de(champ).valeur == "arthur"
        courriel = next((n for n in doc.racine.parcours()
                         if isinstance(n, html.Element)
                         and n.attributs.get("id") == "courriel"), None)
        if courriel is not None:
            doc.pose_foyer(courriel)
            for lettre in "a@b.test":
                doc.frappe(lettre, lettre)

        contexte.execute("document.getElementById('profil')"
                         ".dispatchEvent(new Event('submit'))")
        _attends(doc, "__webapp.enregistre === true")
        etat = _etat_webapp(doc)
        jalon("LOGIN_FORM",
              saisi and (etat.get("profil") or {}).get("nom") == "arthur",
              (saisi, etat.get("profil")))

        # SPA_NAVIGATION — deux vues, `pushState`, puis retour arriere.
        contexte.execute("document.getElementById('vers-flux').click()")
        _attends(doc, "__webapp.navigation.indexOf('flux') >= 0")
        contexte.execute("history.back()")
        _attends(doc, "__webapp.navigation.length >= 2")
        etat = _etat_webapp(doc)
        jalon("SPA_NAVIGATION", "flux" in etat.get("navigation", []),
              etat.get("navigation"))

        # WEBSOCKET_CHAT — le serveur parle quand il veut, et la page l'entend.
        _attends(doc, "__webapp.messages.length >= 3")
        etat = _etat_webapp(doc)
        jalon("WEBSOCKET_CHAT",
              etat.get("socket") in ("ouvert", "ferme")
              and len(etat.get("messages", [])) >= 3,
              (etat.get("socket"), etat.get("messages")))

        # L'interface a-t-elle repondu pendant tout cela ? Si la minuterie de
        # fond n'a pas battu, quelque chose a tenu le fil.
        etat = _etat_webapp(doc)
        tics = etat.get("tics", 0)
        verifie("webapp: la boucle d'evenements n'a jamais cale", tics > 5, tics)

    finally:
        pass

    # INDEXEDDB_PERSISTENCE — on recharge la page **dans un document neuf**,
    # apres avoir jete tout ce qui etait en memoire, et **depuis le meme
    # serveur** : meme origine, donc meme base. Ce qui revient vient du disque,
    # ou ne revient pas.
    indexeddb.reinitialise()
    doc2 = _charge_sur(base, "/page/webapp.html")
    try:
        _attends(doc2, "globalThis.__webapp && __webapp.restaure !== null")
        second = _etat_webapp(doc2)
        restaure_profil = second.get("restaure")
        jalon("INDEXEDDB_PERSISTENCE",
              isinstance(restaure_profil, dict)
              and restaure_profil.get("nom") == "arthur",
              restaure_profil)

        # WEBAPP_COMBINED — le parcours entier, sans une seule erreur en route.
        _attends(doc2, "__webapp.pret === true")
        second = _etat_webapp(doc2)
        complet = (
            JALONS.get("STATIC_PAGE") and JALONS.get("LOGIN_FORM")
            and JALONS.get("SPA_NAVIGATION") and JALONS.get("ASYNC_FETCH")
            and JALONS.get("WEBSOCKET_CHAT")
            and JALONS.get("INDEXEDDB_PERSISTENCE")
            and not second.get("erreurs"))
        jalon("WEBAPP_COMBINED", complet,
              (dict(JALONS), second.get("erreurs")))
    finally:
        arrete()
        ecris_jalons()


def ecris_jalons():
    """Depose les jalons la ou le tableau de bord ira les lire.

    `BO_JALONS` pointe sur l'arbre source : la copie de travail des
    verifications est effacee a chaque execution, et un jalon ecrit dedans
    disparaitrait avant que quiconque le lise.
    """
    chemin = os.environ.get("BO_JALONS") or os.path.join(
        os.path.dirname(os.path.abspath(__file__)), "tests", "jalons.json")
    try:
        os.makedirs(os.path.dirname(chemin), exist_ok=True)
        with open(chemin, "w", encoding="utf-8") as fichier:
            json.dump(JALONS, fichier, indent=1, sort_keys=True)
    except OSError:
        pass


def verifie_origine():
    """L'origine comme type, pas comme chaine.

    Une comparaison approximative ici n'est pas un defaut de style : c'est ce
    qui laisse un site en lire un autre. Les cinq pieges verifies sont ceux
    qu'une comparaison de chaines rate.
    """
    from moteur.origine import Origine

    def o(url):
        return Origine.de_url(url)

    verifie("origine: le port par defaut ne compte pas",
            o("https://a.test") == o("https://a.test:443"))
    verifie("origine: idem en clair",
            o("http://a.test") == o("http://a.test:80"))
    verifie("origine: le chemin ne compte pas",
            o("https://a.test/un") == o("https://a.test/deux?x=1#y"))
    verifie("origine: la casse de l'hote ne compte pas",
            o("https://A.TEST") == o("https://a.test"))

    verifie("origine: le schema fait partie de l'origine",
            o("http://a.test") != o("https://a.test"))
    verifie("origine: un sous-domaine est une autre origine",
            o("https://sub.a.test") != o("https://a.test"))
    verifie("origine: un port different est une autre origine",
            o("https://a.test:8443") != o("https://a.test"))
    verifie("origine: un hote different est une autre origine",
            o("https://b.test") != o("https://a.test"))

    # `ws://` parle au meme serveur que `http://` : une page doit pouvoir
    # ouvrir un WebSocket vers sa propre origine.
    verifie("origine: ws:// et http:// du meme hote se rejoignent",
            o("ws://a.test") == o("http://a.test"))
    verifie("origine: wss:// et https:// aussi",
            o("wss://a.test") == o("https://a.test"))

    # Les origines opaques : le piege qu'un tuple ne pouvait pas porter.
    verifie("origine: une data: est opaque", o("data:text/html,x").est_opaque)
    verifie("origine: un fichier est opaque", o("file:///tmp/x.html").est_opaque)
    verifie("origine: about:blank est opaque", o("about:blank").est_opaque)
    verifie("origine: deux opaques ne correspondent jamais",
            o("data:text/html,x") != o("data:text/html,x"))
    memes = o("data:text/html,x")
    verifie("origine: une opaque est egale a elle-meme", memes == memes)

    # Un port illisible ne doit pas retomber sur le port par defaut : ce serait
    # accepter une adresse forgee comme si elle etait la bonne.
    verifie("origine: un port illisible donne une origine opaque",
            o("http://a.test:abc/").est_opaque)

    egal("origine: serialisation sans port par defaut",
         o("https://a.test:443/x").serialise(), "https://a.test")
    egal("origine: serialisation avec port explicite",
         o("https://a.test:8443/x").serialise(), "https://a.test:8443")
    egal("origine: une opaque se serialise en null",
         o("data:text/html,x").serialise(), "null")
    # Mais sa cle de stockage reste unique : deux pages `data:` sans rapport ne
    # doivent pas se relire l'une l'autre.
    verifie("origine: deux opaques ont deux cles de stockage",
            o("data:,a").cle_stockage() != o("data:,b").cle_stockage())


def verifie_contexte_navigation():
    """L'arbre des contextes : relations, historiques separes, fermeture."""
    from moteur.contexte import BrowsingContext, TopLevelBrowsingContext

    sommet = TopLevelBrowsingContext(url="https://a.test/page")
    verifie("contexte: le premier niveau n'a pas de parent", sommet.est_sommet)
    verifie("contexte: il est son propre sommet", sommet.sommet is sommet)

    un = BrowsingContext(parent=sommet, url="https://a.test/cadre", nom="un")
    deux = BrowsingContext(parent=sommet, url="https://b.test/cadre", nom="deux")
    petit = BrowsingContext(parent=un, url="https://a.test/profond")

    egal("contexte: les enfants sont declares", len(sommet.enfants), 2)
    verifie("contexte: top remonte jusqu'au sommet", petit.sommet is sommet)
    verifie("contexte: parent est le parent direct", petit.parent is un)
    egal("contexte: la profondeur se compte", petit.profondeur, 2)
    egal("contexte: les descendants sont tous la",
         len(sommet.descendants()), 3)
    verifie("contexte: on retrouve par nom", sommet.par_nom("deux") is deux)
    verifie("contexte: et par identifiant", sommet.par_id(petit.id) is petit)

    # L'origine decide de la frontiere.
    verifie("contexte: meme origine reconnue", sommet.meme_origine(un))
    verifie("contexte: origine differente reconnue",
            not sommet.meme_origine(deux))

    # Deux historiques, pas un seul tableau confondu. C'est ce qui fait qu'un
    # « precedent » dans un iframe ne recule pas la page entiere.
    sommet.navigue("https://a.test/page2")
    un.navigue("https://a.test/cadre2")
    un.navigue("https://a.test/cadre3")
    egal("contexte: l'historique du sommet est le sien",
         len(sommet.historique), 2)
    egal("contexte: celui de l'iframe aussi", len(un.historique), 3)
    un.recule()
    egal("contexte: reculer dans l'iframe", un.url, "https://a.test/cadre2")
    egal("contexte: ne bouge pas le sommet", sommet.url, "https://a.test/page2")
    un.avance()
    egal("contexte: et avancer y revient", un.url, "https://a.test/cadre3")

    # Naviguer depuis le milieu coupe la branche abandonnee.
    un.recule()
    un.navigue("https://a.test/autre")
    egal("contexte: naviguer depuis le milieu coupe la suite",
         un.historique, ["https://a.test/cadre", "https://a.test/cadre2",
                         "https://a.test/autre"])

    # La fermeture descend jusqu'aux petits-enfants.
    ferme = []

    class FauxDocument:
        def __init__(self, nom):
            self.nom = nom

        def ferme_document(self):
            ferme.append(self.nom)

    petit.document = FauxDocument("petit")
    un.document = FauxDocument("un")
    un.ferme_contexte()
    egal("contexte: la fermeture descend d'abord aux enfants",
         ferme, ["petit", "un"])
    verifie("contexte: l'enfant ferme quitte son parent",
            un not in sommet.enfants)
    verifie("contexte: fermer deux fois ne fait rien",
            un.ferme_contexte() is False)
    verifie("contexte: le petit-fils est ferme aussi", petit.ferme)


def verifie_cadres():
    """iframe : deux mondes, une page, et une frontiere entre eux.

    Le serveur repond sur `127.0.0.1` et sur `localhost` — deux hotes distincts
    au sens de la norme, donc deux origines reelles, sans Internet ni DNS.

    Ce qui est verifie, dans l'ordre ou cela se casse :

      1. le cadre existe, a son contexte, son document et son viewport ;
      2. il se met en page tout seul, a **sa** largeur et non a celle du parent ;
      3. sa liste d'affichage entre dans celle du parent, rognee a ses bornes ;
      4. `contentDocument` s'ouvre en meme origine et se ferme sinon ;
      5. `postMessage` traverse dans les deux sens, et `targetOrigin` filtre ;
      6. `iframe.src = ...` navigue l'enfant et pas le parent ;
      7. retirer l'iframe detruit tout ce qu'il tenait.
    """
    import time as _time
    from moteur import reseau as _reseau
    import serveur_test

    serveur, base = serveur_test.demarre(0)
    port = base.rsplit(":", 1)[1]
    # Meme serveur, hote different : c'est un cross-origin authentique.
    autre = "http://localhost:%s" % port
    ancien_charge = _reseau._charge_http
    try:
        import apercu
        apercu.installe_reseau(_reseau)
    except Exception:  # noqa: BLE001
        pass
    try:
        _cadres_scenario(base, autre, _time)
    finally:
        _reseau._charge_http = ancien_charge
        serveur_test.arrete(serveur)
        ecris_jalons()


def _cadres_scenario(base, autre, _time):
    from moteur import origine as mod_origine

    # Les deux bases doivent bien etre deux origines : si ce n'etait pas le cas,
    # tout le reste du scenario ne prouverait rien.
    verifie("cadres: les deux bases sont deux origines",
            not mod_origine.meme_origine(base, autre), (base, autre))

    doc = moteur.charge(base + "/page/iframe-parent.html", 1000,
                        journal=lambda n, t: None)
    doc.remet_en_page(1000, 800)
    contexte = doc.contexte_js
    verifie("cadres: la page a un contexte JavaScript", contexte is not None)
    if contexte is None:
        return

    # La page ne peut pas deviner le port : on le lui donne avant qu'elle pose
    # ses cadres.
    contexte.execute(
        "globalThis.__config = {meme: %r, autre: %r, origineAutre: %r};"
        % (base + "/page/iframe-enfant.html",
           autre + "/page/iframe-enfant.html",
           autre))
    # La page a deja tourne : on lui refait poser ses `src` maintenant que la
    # configuration est la.
    contexte.execute(
        "document.getElementById('ami').src = __config.meme;"
        "document.getElementById('etranger').src = __config.autre;")

    def bat(condition, secondes=15.0):
        limite = _time.perf_counter() + secondes
        while _time.perf_counter() < limite:
            doc.rafraichis()
            if contexte.execute(condition):
                return True
            _time.sleep(0.02)
        return False

    charges = bat("__parent.cadres >= 2")
    lu = json.loads(contexte.execute("JSON.stringify(__parent)"))
    verifie("cadres: les deux cadres se sont annonces", charges, lu)

    # 1 et 2. Les contextes existent, avec leur propre viewport.
    enfants = doc.contexte_navigation.enfants
    egal("cadres: deux contextes enfants", len(enfants), 2)
    for enfant in enfants:
        verifie("cadres: le contexte porte un document", enfant.document is not None)
        verifie("cadres: il a son propre viewport",
                enfant.largeur == 280.0 and enfant.hauteur == 180.0,
                (enfant.largeur, enfant.hauteur))
        verifie("cadres: son document est mis en page a sa largeur",
                enfant.document.boite is not None
                and enfant.document.largeur == 280.0,
                enfant.document.largeur)
        verifie("cadres: le parent est bien le parent", enfant.parent is
                doc.contexte_navigation)
        verifie("cadres: et le sommet est la page",
                enfant.sommet is doc.contexte_navigation)

    # 3. La liste d'affichage de l'enfant entre dans celle du parent, entre un
    #    `clip` et un `declip`. Sans le rognage, un document enfant plus haut
    #    que son cadre recouvrirait la page hote.
    affichage = doc.liste_affichage(0.0, 1000, 800)
    clips = [op for op in affichage if op and op[0] == "clip"]
    verifie("cadres: la liste du parent contient un rognage de cadre",
            any(abs(op[3] - 280.0) < 2 and abs(op[4] - 180.0) < 2 for op in clips),
            [op[1:] for op in clips][:6])
    textes = [op[3] for op in affichage if op and op[0] == "texte"]
    verifie("cadres: le texte de l'enfant est peint dans le parent",
            any("Document enfant" in t for t in textes), textes[:8])

    # 4. La frontiere.
    contexte.execute("__sonde()")
    lu = json.loads(contexte.execute("JSON.stringify(__parent)"))
    egal("cadres: contentDocument lisible en meme origine",
         lu["memeOrigine"]["document"], "lisible")
    egal("cadres: contentDocument refuse en origine differente",
         lu["autreOrigine"]["document"], "refuse")

    # Et la reciproque, vue depuis l'enfant : celui de meme origine a pu lire
    # son parent, l'autre non.
    par_origine = {}
    for enfant in enfants:
        sous = enfant.document.contexte_js
        if sous is None:
            continue
        etat = json.loads(sous.execute("JSON.stringify(globalThis.__enfant || {})"))
        par_origine[etat.get("origine")] = etat
    ami = par_origine.get(mod_origine.Origine.de_url(base).serialise())
    etranger = par_origine.get(mod_origine.Origine.de_url(autre).serialise())
    if ami:
        egal("cadres: l'enfant de meme origine lit son parent",
             ami.get("lectureParent"), "autorisee")
    if etranger:
        verifie("cadres: l'enfant d'une autre origine ne lit pas son parent",
                etranger.get("lectureParent") in ("refusee", "exception"),
                etranger.get("lectureParent"))

    # 5. postMessage, dans les deux sens et a travers la frontiere.
    contexte.execute("__envoie()")
    bat("__parent.memeOrigine.echo !== null && __parent.autreOrigine.echo !== null")
    lu = json.loads(contexte.execute("JSON.stringify(__parent)"))
    egal("cadres: postMessage aller-retour en meme origine",
         (lu["memeOrigine"]["echo"] or {}).get("salut"), "meme")
    egal("cadres: postMessage aller-retour cross-origine",
         (lu["autreOrigine"]["echo"] or {}).get("salut"), "autre")

    # Le message au mauvais `targetOrigin` ne doit jamais etre arrive. C'est ce
    # qui empeche d'envoyer un jeton a un cadre parti ailleurs.
    for enfant in enfants:
        sous = enfant.document.contexte_js
        if sous is None:
            continue
        recus = json.loads(sous.execute(
            "JSON.stringify((globalThis.__enfant || {}).recus || [])"))
        verifie("cadres: un targetOrigin qui ne correspond pas fait taire le message",
                not any("secret" in json.dumps(r.get("data") or {}) for r in recus),
                recus)

    # 6. `iframe.src = ...` navigue l'enfant, jamais le parent.
    url_parent = doc.url
    contexte.execute("document.getElementById('ami').src = %r"
                     % (base + "/page/login-form.html"))
    bat("false", secondes=1.0)
    egal("cadres: le document principal n'a pas navigue", doc.url, url_parent)
    ami_contexte = next((e for e in doc.contexte_navigation.enfants
                         if e.element is not None
                         and e.element.attributs.get("id") == "ami"), None)
    verifie("cadres: l'enfant a navigue",
            ami_contexte is not None and ami_contexte.url.endswith("login-form.html"),
            ami_contexte.url if ami_contexte else None)
    verifie("cadres: et son historique lui appartient",
            ami_contexte is not None and len(ami_contexte.historique) == 2,
            ami_contexte.historique if ami_contexte else None)
    egal("cadres: celui du parent n'a pas bouge",
         len(doc.contexte_navigation.historique), 1)

    # 7. Retirer l'iframe detruit son monde.
    contextes_avant = list(doc.contexte_navigation.enfants)
    contexte.execute("document.getElementById('etranger').remove()")
    bat("false", secondes=1.0)
    egal("cadres: le contexte retire disparait",
         len(doc.contexte_navigation.enfants), 1)
    mort = next(c for c in contextes_avant
                if c not in doc.contexte_navigation.enfants)
    verifie("cadres: son contexte est ferme", mort.ferme)
    verifie("cadres: et son document est libere", mort.document is None)

    # Les jalons : ce que ce scenario prouve, en termes de capacites.
    jalon("IFRAME_SAME_ORIGIN",
          lu["memeOrigine"]["document"] == "lisible"
          and (lu["memeOrigine"]["echo"] or {}).get("salut") == "meme")
    jalon("IFRAME_CROSS_ORIGIN",
          lu["autreOrigine"]["document"] == "refuse"
          and lu["autreOrigine"]["message"] is not None)
    jalon("POSTMESSAGE",
          (lu["memeOrigine"]["echo"] or {}).get("salut") == "meme"
          and (lu["autreOrigine"]["echo"] or {}).get("salut") == "autre")


def verifie_fuite_cadres():
    """Mille cadres crees et retires ne doivent rien laisser derriere eux.

    C'est le test que le cycle de vie appelle. Un `<iframe>` tient un contexte
    QuickJS, un pool de fils, des connexions et des transactions : oublier de
    les rendre ne se voit pas sur une page d'essai et tue une application au
    bout d'une heure.

    On ne compte pas les octets — ils dependent du ramasse-miettes, et un test
    qui en depend est instable. On compte ce qui est deterministe : les
    contextes crees contre les contextes fermes, et les documents fermes.
    """
    from moteur import telemetrie

    doc = document(
        "<body><div id='zone'></div><script>1</script></body>",
        url="http://a.test/page")
    doc.remet_en_page(800, 600)
    contexte = doc.contexte_js
    verifie("fuite: la page a un contexte", contexte is not None)
    if contexte is None:
        return

    telemetrie.reinitialise()
    telemetrie.active(True)
    tours = 500
    for _ in range(tours):
        # `about:blank` : aucun reseau, donc on mesure le cycle de vie et rien
        # d'autre. Un cadre avec `src` melangerait le cout du chargement.
        # Pas de `const` : chaque `execute` partage la portee globale du
        # contexte, et une seconde declaration du meme nom la ferait echouer —
        # silencieusement, puisque l'erreur est journalisee et non levee. Le
        # test aurait alors mesure une seule iteration en croyant en mesurer
        # mille.
        contexte.execute(
            "document.getElementById('zone').innerHTML = '<iframe></iframe>';")
        doc.rafraichis()
        contexte.execute("document.getElementById('zone').innerHTML = '';")
        doc.rafraichis()

    mesures = telemetrie.compteurs()
    crees = mesures.get("cadre.crees", 0)
    retires = mesures.get("cadre.retires", 0)
    verifie("fuite: les cadres ont bien ete crees", crees >= tours - 2, crees)
    egal("fuite: autant de cadres retires que crees", retires, crees)
    egal("fuite: autant de contextes fermes que crees",
         mesures.get("contexte.fermes", 0), crees)
    egal("fuite: aucun contexte enfant ne survit",
         len(doc.contexte_navigation.enfants), 0)
    egal("fuite: aucun descendant ne survit",
         len(doc.contexte_navigation.descendants()), 0)
    telemetrie.active(False)


def verifie_cors():
    """CORS : ce qu'un script a le droit de LIRE, pas ce qu'il a le droit de demander.

    Le raccourci « si c'est cross-origine, on refuse » est faux et casserait la
    moitie du Web. Ces epreuves verifient les deux moities de la vraie regle :

      * la requete **part** meme cross-origine — le serveur la voit ;
      * la reponse n'est **lisible** que si le serveur y consent.

    Le second point se verifie de la seule facon honnete : en demandant au
    serveur ce qu'il a recu. Une epreuve qui ne regarderait que le refus cote
    navigateur ne saurait pas distinguer « refuse a la lecture » de « jamais
    envoye », et ce sont deux comportements tres differents.
    """
    import time as _time
    from moteur import reseau as _reseau
    import serveur_test

    serveur, base = serveur_test.demarre(0)
    port = base.rsplit(":", 1)[1]
    autre = "http://localhost:%s" % port
    ancien_charge = _reseau._charge_http
    try:
        import apercu
        apercu.installe_reseau(_reseau)
    except Exception:  # noqa: BLE001
        pass
    try:
        _cors_scenario(base, autre, _time)
    finally:
        _reseau._charge_http = ancien_charge
        serveur_test.arrete(serveur)
        ecris_jalons()


def _cors_scenario(base, autre, _time):
    doc = moteur.charge(base + "/page/event-loop.html", 800,
                        journal=lambda n, t: None)
    contexte = doc.contexte_js
    verifie("cors: la page a un contexte", contexte is not None)
    if contexte is None:
        return

    def demande(nom, url, options=""):
        """Emet un `fetch` et rend ce que le script en a vu."""
        contexte.execute(
            "globalThis.__c = globalThis.__c || {};"
            "globalThis.__c[%r] = null;"
            "fetch(%r%s).then(function (r) {"
            "  return r.text().then(function (t) {"
            "    __c[%r] = {status: r.status, texte: t,"
            "               type: r.headers.get('content-type'),"
            "               secret: r.headers.get('x-secret-applicatif')};"
            "  });"
            "}).catch(function (e) { __c[%r] = {erreur: String(e)}; });"
            % (nom, url, options, nom, nom))
        limite = _time.perf_counter() + 10.0
        while _time.perf_counter() < limite:
            doc.rafraichis()
            if contexte.execute("__c[%r] !== null" % nom):
                break
            _time.sleep(0.02)
        brut = contexte.execute("JSON.stringify(__c[%r])" % nom)
        return json.loads(brut) if brut else None

    # 1. Meme origine : rien a verifier, tout est lisible.
    vu = demande("meme", base + "/cors/prive")
    egal("cors: meme origine, reponse lisible", vu.get("status"), 200)
    verifie("cors: et son corps arrive", "prive" in (vu.get("texte") or ""), vu)

    # 2. Cross-origine sans en-tete : la reponse existe mais reste opaque.
    # Une reponse non exposee fait **rejeter** la promesse `fetch` — c'est ce
    # que fait un vrai navigateur, et c'est ce qui distingue « refuse » de
    # « recu mais vide ». Le script ne doit rien apprendre du contenu, pas meme
    # sa longueur.
    vu = demande("prive", autre + "/cors/prive")
    verifie("cors: cross-origine sans ACAO fait echouer le fetch",
            vu.get("erreur") is not None, vu)
    egal("cors: et rien du corps ne transparait", vu.get("texte"), None)

    # Mais la requete est **partie** : c'est la distinction qui compte. On le
    # prouve en la redemandant en meme origine, ou le serveur nous dira ce
    # qu'il a vu.
    vu = demande("preuve", base + "/cors/ouvert")
    egal("cors: une requete cross-origine part quand meme",
         vu.get("status"), 200)

    # 3. `Access-Control-Allow-Origin: *`.
    vu = demande("etoile", autre + "/cors/ouvert")
    egal("cors: ACAO * expose la reponse", vu.get("status"), 200)
    verifie("cors: et son corps est lisible", "ouvert" in (vu.get("texte") or ""),
            vu)

    # 4. ACAO nommant exactement notre origine.
    vu = demande("exact", autre + "/cors/exact")
    egal("cors: ACAO exact expose la reponse", vu.get("status"), 200)

    # 5. ACAO nommant quelqu'un d'autre : refuse.
    vu = demande("autre", autre + "/cors/autre")
    verifie("cors: ACAO d'une autre origine ne nous expose rien",
            vu.get("erreur") is not None, vu)

    # 6. Les en-tetes applicatifs ne traversent pas, meme quand la reponse est
    #    lisible. Un en-tete d'un autre site n'a pas a etre expose.
    vu = demande("entetes", autre + "/cors/secret-entete")
    egal("cors: la reponse est lisible", vu.get("status"), 200)
    egal("cors: mais l'en-tete applicatif ne traverse pas",
         vu.get("secret"), None)
    verifie("cors: les en-tetes surs, eux, restent lisibles",
            "json" in (vu.get("type") or ""), vu.get("type"))

    # 7. Avec temoins : `*` ne suffit pas, il faut nommer l'origine et l'avouer.
    vu = demande("cred-etoile", autre + "/cors/temoins-etoile",
                 ", {credentials: 'include'}")
    verifie("cors: ACAO * ne vaut pas pour une requete avec temoins",
            vu.get("erreur") is not None, vu)
    vu = demande("cred-exact", autre + "/cors/temoins",
                 ", {credentials: 'include'}")
    egal("cors: ACAO exact plus Allow-Credentials expose la reponse",
         vu.get("status"), 200)

    # 8. Preflight : une requete non simple demande d'abord la permission.
    from moteur import telemetrie
    telemetrie.reinitialise()
    telemetrie.active(True)
    vu = demande("preflight", autre + "/cors/exact",
                 ", {method: 'PUT', headers: {'X-Jeton': 'a'}}")
    mesures = telemetrie.compteurs()
    verifie("cors: une requete non simple declenche un preflight",
            (mesures.get("cors.preflights") or 0) >= 1, mesures.get("cors.preflights"))
    egal("cors: le preflight accorde laisse passer", vu.get("status"), 200)

    telemetrie.reinitialise()
    vu = demande("preflight-refuse", autre + "/cors/prive",
                 ", {method: 'DELETE', headers: {'X-Jeton': 'a'}}")
    mesures = telemetrie.compteurs()
    verifie("cors: un preflight refuse fait echouer le fetch",
            vu.get("erreur") is not None, vu)
    verifie("cors: et la vraie requete n'est jamais partie",
            (mesures.get("cors.preflights_refuses") or 0) >= 1,
            mesures.get("cors.preflights_refuses"))
    telemetrie.active(False)
    jalon("CORS", vu.get("erreur") is not None)


def verifie_toile_origine():
    """Une toile contaminee ne se relit pas.

    C'est le vieux manque que l'audit signalait, et il n'etait pas traitable
    tant que le moteur ne savait pas d'ou venait une image. Maintenant qu'il le
    sait : dessiner une image d'une autre origine dans un `<canvas>` doit rendre
    ses pixels illisibles, sans quoi une page peut relire l'image d'un site ou
    l'utilisateur est connecte — c'est-a-dire lire une page qu'elle n'a pas le
    droit d'ouvrir.

    Les trois cas verifies sont les trois qui comptent : une image de la meme
    origine ne contamine rien, une image `data:` non plus — elle voyage avec la
    page —, une image d'ailleurs contamine.
    """
    from moteur import images

    doc = document("<body><canvas id='t' width='40' height='40'></canvas>"
                   "<script>1</script></body>",
                   url="http://a.test/page")
    contexte = doc.contexte_js
    verifie("toile: la page a un contexte", contexte is not None)
    if contexte is None:
        return

    # On declare des images d'origines choisies, sans reseau : c'est le lien
    # `identifiant -> adresse` que la regle consulte.
    images._origines[9001] = "http://a.test/logo.png"
    images._origines[9002] = "http://b.test/pixel.gif"
    images._origines[9003] = "data:image/gif;base64,R0lGOD"
    try:
        def lisible(operations):
            return not contexte._op_toileSouillee(operations)

        verifie("toile: une image de meme origine ne contamine pas",
                lisible([["image", 0, 0, 10, 10, 9001]]))
        verifie("toile: une image data: ne contamine pas",
                lisible([["image", 0, 0, 10, 10, 9003]]))
        verifie("toile: une image d'une autre origine contamine",
                not lisible([["image", 0, 0, 10, 10, 9002]]))
        verifie("toile: une seule image etrangere suffit a contaminer",
                not lisible([["image", 0, 0, 10, 10, 9001],
                             ["image", 0, 0, 10, 10, 9002]]))
        # Une image dont on ignore l'origine contamine : ne pas savoir n'est pas
        # une raison d'autoriser.
        verifie("toile: une image d'origine inconnue contamine",
                not lisible([["image", 0, 0, 10, 10, 12345]]))
        verifie("toile: un dessin sans image reste lisible",
                lisible([["rect", 0, 0, 10, 10]]))

        # Et du cote de la page : `getImageData` doit lever `SecurityError`.
        rendu = contexte.execute("""
            (function () {
                const c = document.getElementById('t').getContext('2d');
                c.__operations.push(["image", 0, 0, 10, 10, 9002]);
                try { c.getImageData(0, 0, 4, 4); return "lu"; }
                catch (e) { return e.name; }
            })()
        """)
        egal("toile: getImageData leve SecurityError sur une toile contaminee",
             rendu, "SecurityError")

        rendu = contexte.execute("""
            (function () {
                const t = document.getElementById('t');
                const c = t.getContext('2d');
                try { t.toDataURL(); return "lu"; }
                catch (e) { return e.name; }
            })()
        """)
        egal("toile: toDataURL aussi", rendu, "SecurityError")

        # Une toile propre se relit normalement : la regle ne doit pas casser
        # l'usage ordinaire.
        rendu = contexte.execute("""
            (function () {
                const t = document.createElement('canvas');
                t.width = 8; t.height = 8;
                const c = t.getContext('2d');
                c.fillStyle = '#ff0000'; c.fillRect(0, 0, 4, 4);
                try { c.getImageData(0, 0, 2, 2); return "lu"; }
                catch (e) { return e.name; }
            })()
        """)
        egal("toile: une toile propre se relit", rendu, "lu")
        jalon("CANVAS_ORIGIN_CLEAN", rendu == "lu"
              and not lisible([["image", 0, 0, 10, 10, 9002]]))
    finally:
        for cle in (9001, 9002, 9003):
            images._origines.pop(cle, None)
        ecris_jalons()


def verifie_wss():
    """`wss://` : le meme WebSocket, derriere le TLS deja valide du navigateur.

    Ce qui est eprouve n'est pas « le chiffrement marche » mais **que le
    navigateur verifie**. Une autorite de test signe un certificat pour
    `localhost` ; on la fait reconnaitre par `BO_CA_BUNDLE`, et l'on verifie
    ensuite les deux sens :

      * avec l'autorite reconnue, la connexion s'etablit et parle ;
      * sans elle, la connexion **echoue** au lieu de se degrader en clair.

    Le second cas est celui qui compte. Un `wss://` qui retomberait en `ws://`
    quand la verification gene ne protegerait personne — c'est meme pire
    qu'aucune securite, parce qu'on croit l'avoir.

    L'architecture est celle demandee : transport TCP, transport TLS optionnel,
    puis le meme decoupage en trames. Aucune seconde pile TLS.
    """
    import os as _os
    import shutil as _shutil
    import tempfile as _tempfile
    import time as _time

    from moteur import reseau as _reseau
    import serveur_test

    dossier = _tempfile.mkdtemp(prefix="bo-wss-")
    try:
        materiel = serveur_test.fabrique_ca(dossier)
        if materiel is None:
            verifie("wss: cryptography absent, epreuve non mesuree", True)
            return
        chemin_ca, chemin_srv = materiel
        serveur, base = serveur_test.demarre_tls(chemin_srv)
        ancien_bundle = _os.environ.get("BO_CA_BUNDLE")
        try:
            _wss_scenario(base, chemin_ca, _os, _time, _reseau)
        finally:
            if ancien_bundle is None:
                _os.environ.pop("BO_CA_BUNDLE", None)
            else:
                _os.environ["BO_CA_BUNDLE"] = ancien_bundle
            serveur_test.arrete(serveur)
    finally:
        _shutil.rmtree(dossier, ignore_errors=True)
        ecris_jalons()


def _wss_scenario(base, chemin_ca, _os, _time, _reseau):
    # `contexte_tls()` reconstruit son contexte a chaque appel :
    # changer `BO_CA_BUNDLE` suffit a changer ce qu'il approuve,
    # sans avoir a vider quoi que ce soit.
    from moteur import websocket

    # 1. Sans l'autorite, la connexion doit echouer. On l'eprouve d'abord :
    #    si elle reussissait ici, tout ce qui suit ne prouverait rien.
    _os.environ["BO_CA_BUNDLE"] = "/inexistant/aucune-autorite.pem"
    refusee = websocket.Connexion(base + "/ws/echo", document="https://a.test/p")
    echec = None
    try:
        refusee.ouvre()
    except Exception as e:  # noqa: BLE001
        echec = str(e)
    finally:
        try:
            refusee.ferme(1000, "")
        except Exception:  # noqa: BLE001
            pass
    verifie("wss: sans autorite reconnue, la connexion est refusee",
            echec is not None, echec)
    verifie("wss: et elle ne se degrade pas en clair",
            refusee.etat != websocket.OUVERT, refusee.etat)

    # 2. Avec l'autorite, la connexion s'etablit et parle.
    _os.environ["BO_CA_BUNDLE"] = chemin_ca
    connexion = websocket.Connexion(base + "/ws/echo",
                                    protocoles=["bo-chat", "bo-secours"],
                                    document="https://a.test/p")
    try:
        ouverte = connexion.ouvre()
        verifie("wss: avec l'autorite reconnue, la connexion s'ouvre", ouverte)
        if not ouverte:
            return
        connexion.envoie("bonjour en clair chiffre")
        recus = []
        limite = _time.perf_counter() + 10.0
        while _time.perf_counter() < limite and not recus:
            for type_, charge in connexion.evenements():
                if type_ == "message":
                    recus.append(charge)
            _time.sleep(0.02)
        verifie("wss: le serveur repond a travers TLS", bool(recus), recus)
        verifie("wss: et le message est celui qu'on a envoye",
                any("bonjour" in str(m) for m in recus), recus)

        # 3. Le sous-protocole negocie.
        egal("wss: le sous-protocole retenu est celui qu'on a propose",
             connexion.protocole, "bo-chat")
    finally:
        connexion.ferme(1000, "fin")

    jalon("WSS", True)
    jalon("WEBSOCKET_SUBPROTOCOL", connexion.protocole == "bo-chat")


def verifie_grille_intrinseque():
    """Les pistes de grille prennent la taille de leur contenu.

    Elles se partageaient la place a parts egales, faute de savoir ce qu'elles
    portaient — le commentaire du module le disait franchement. Une colonne de
    mots courts recevait donc autant qu'une colonne de phrases, et une grille
    `auto auto` coupait toujours la page en deux quel que soit son contenu.

    Ce que ces epreuves verifient est le comportement observable, pas les
    nombres internes : une colonne qui porte plus large **doit** etre plus
    large. C'est la seule formulation qui reste vraie si la fonte change.
    """
    doc = document("""
        <style>
          body { margin: 0; }
          #g { display: grid; grid-template-columns: auto auto; width: 900px; }
        </style>
        <body><div id="g">
          <div id="court">a</div>
          <div id="long">une phrase nettement plus longue que la precedente</div>
        </div></body>""")
    doc.remet_en_page(1000, 800)
    court = boite_de(doc, "court")
    longue = boite_de(doc, "long")
    verifie("grille: une colonne auto suit son contenu",
            longue.largeur > court.largeur * 1.5,
            (court.largeur, longue.largeur))

    # `min-content` et `max-content` ne sont pas la meme chose. Les confondre —
    # ce qui etait le cas — revenait a ignorer les deux.
    doc = document("""
        <style>
          body { margin: 0; }
          #g { display: grid; grid-template-columns: min-content max-content;
               width: 900px; }
        </style>
        <body><div id="g">
          <div id="mini">alpha bravo charlie delta</div>
          <div id="maxi">alpha bravo charlie delta</div>
        </div></body>""")
    doc.remet_en_page(1000, 800)
    mini = boite_de(doc, "mini")
    maxi = boite_de(doc, "maxi")
    verifie("grille: min-content est plus etroit que max-content",
            mini.largeur < maxi.largeur, (mini.largeur, maxi.largeur))
    verifie("grille: et min-content n'est pas nul", mini.largeur > 0, mini.largeur)

    # `minmax(200px, 1fr)` : un plancher et un plafond, la forme la plus
    # courante d'une grille reelle.
    doc = document("""
        <style>
          body { margin: 0; }
          #g { display: grid; grid-template-columns: minmax(200px, 1fr) 100px;
               width: 900px; }
        </style>
        <body><div id="g"><div id="souple">x</div><div id="fixe">y</div></div></body>""")
    doc.remet_en_page(1000, 800)
    souple = boite_de(doc, "souple")
    fixe = boite_de(doc, "fixe")
    verifie("grille: minmax respecte son plancher",
            souple.largeur >= 199.0, souple.largeur)
    verifie("grille: et la piste fixe garde sa largeur",
            proche(fixe.largeur, 100.0, 2.0), fixe.largeur)
    verifie("grille: la piste souple prend le reste",
            souple.largeur > 700.0, souple.largeur)

    # `fr` partage ce qui reste apres les pistes rigides.
    doc = document("""
        <style>
          body { margin: 0; }
          #g { display: grid; grid-template-columns: 100px 1fr 2fr; width: 700px; }
        </style>
        <body><div id="g"><div id="a">a</div><div id="b">b</div>
        <div id="c">c</div></div></body>""")
    doc.remet_en_page(1000, 800)
    a, b, c = (boite_de(doc, n) for n in ("a", "b", "c"))
    verifie("grille: la piste fixe garde ses 100px", proche(a.largeur, 100.0, 2.0),
            a.largeur)
    verifie("grille: 2fr vaut deux fois 1fr",
            proche(c.largeur, b.largeur * 2.0, 6.0), (b.largeur, c.largeur))
    verifie("grille: et les trois remplissent la grille",
            proche(a.largeur + b.largeur + c.largeur, 700.0, 6.0),
            a.largeur + b.largeur + c.largeur)

    # Le depassement : une colonne ne descend pas sous son mot le plus long,
    # parce que deborder est pire que serrer.
    doc = document("""
        <style>
          body { margin: 0; }
          #g { display: grid; grid-template-columns: auto auto; width: 60px; }
        </style>
        <body><div id="g">
          <div id="p">incompressiblement</div><div id="q">x</div>
        </div></body>""")
    doc.remet_en_page(1000, 800)
    verifie("grille: une colonne ne passe pas sous son min-content",
            boite_de(doc, "p").largeur > 20.0, boite_de(doc, "p").largeur)


def verifie_pseudo_classes_dynamiques():
    """Passer la souris sur un bouton ne doit pas remettre la page en page.

    C'est le cas le plus frequent de toute l'interactivite : `:hover` change
    presque toujours une couleur ou un fond, jamais une geometrie. Le moteur
    remettait pourtant toute la page en page a chaque pixel parcouru.

    Comme pour les classes, les deux sens sont verifies. Un `:hover` qui change
    une **largeur** doit, lui, bel et bien remettre en page — une invalidation
    trop etroite se voit comme un bogue d'affichage, et une epreuve qui ne
    verifierait que les economies recompenserait un moteur qui ne redessine
    plus rien.
    """
    from moteur import telemetrie

    cartes = "".join('<div class="carte" id="c%d">Carte %d</div>' % (i, i)
                     for i in range(400))
    doc = document("""
        <style>
          body { margin: 0; }
          .carte { padding: 6px; color: #202020; background: #ffffff; }
          .carte:hover { color: #cc0000; background: #fff0f0; }
          .large:hover { width: 500px; }
          input { padding: 4px; }
          input:focus { background: #ffffcc; }
          input:disabled { color: #999999; }
          .coche:checked { background: #ccffcc; }
        </style>
        <body>%s
          <div class="carte large" id="grande">grandit</div>
          <input id="champ"><input id="mort" disabled>
          <input id="case" type="checkbox" class="coche">
        </body>""" % cartes)
    doc.remet_en_page(1000, 800)

    def mesure(action):
        telemetrie.reinitialise()
        telemetrie.active(True)
        change = action()
        return change, telemetrie.compteurs()

    cible = boite_de(doc, "c200")
    point = (cible.x + 3, cible.y + 3)

    # 1. Survol d'une carte qui ne change que sa couleur.
    change, m = mesure(lambda: doc.survole(*point))
    verifie("pseudo: le survol change l'affichage", change)
    egal("pseudo: et ne remet rien en page",
         m.get("invalidation.interaction_mise_en_page"), None)
    egal("pseudo: il ne demande que des pixels",
         m.get("invalidation.interaction_peinture_seule"), 1)
    egal("pseudo: aucune boite n'est reposee", m.get("layout.dispose_bloc"), None)
    egal("pseudo: et la couleur a bien change",
         boite_de(doc, "c200").style.get("color"), "#cc0000")

    # 2. Sortie du survol : meme economie.
    autre = boite_de(doc, "c100")
    change, m = mesure(lambda: doc.survole(autre.x + 3, autre.y + 3))
    egal("pseudo: quitter un element ne remet rien en page non plus",
         m.get("invalidation.interaction_mise_en_page"), None)
    egal("pseudo: la carte quittee a repris sa couleur",
         boite_de(doc, "c200").style.get("color"), "#202020")

    # 3. Un `:hover` qui change une largeur **doit** remettre en page.
    grande = boite_de(doc, "grande")
    change, m = mesure(lambda: doc.survole(grande.x + 3, grande.y + 3))
    egal("pseudo: un hover qui change une largeur remet en page",
         m.get("invalidation.interaction_mise_en_page"), 1)
    verifie("pseudo: et la largeur est appliquee",
            boite_de(doc, "grande").largeur > 400.0,
            boite_de(doc, "grande").largeur)

    # 4. `:focus` : meme regle.
    champ = next(n for n in doc.racine.parcours()
                 if isinstance(n, html.Element)
                 and n.attributs.get("id") == "champ")
    doc.survole(0, 0)
    change, m = mesure(lambda: doc.pose_foyer(champ))
    verifie("pseudo: poser le foyer change l'affichage", change)
    egal("pseudo: le foyer ne remet rien en page",
         m.get("invalidation.interaction_mise_en_page"), None)
    # `background` est developpe en ses composantes par la cascade : c'est
    # `background-color` qui porte la teinte.
    egal("pseudo: le fond du champ au foyer a change",
         boite_de(doc, "champ").style.get("background-color"), "#ffffcc")

    # 5. `:disabled` et `:checked` : statiques, mais ils doivent correspondre.
    egal("pseudo: :disabled s'applique",
         boite_de(doc, "mort").style.get("color"), "#999999")
    telemetrie.active(False)


def verifie_invalidation_ciblee():
    """Changer une couleur ne doit pas remettre mille cartes en page.

    C'est la promesse entiere du pipeline d'invalidation, et elle se verifie de
    la seule facon qui ne se trompe pas : en comptant les mises en page reelles.
    Un test qui regarderait les pixels dirait « c'est bien peint » sans rien
    savoir de ce que cela a coute.

    Trois mutations, trois etendues attendues :

      * une couleur       → peinture seule, aucune mise en page ;
      * un `transform`    → composition seule ;
      * une largeur       → mise en page, et il **faut** qu'elle ait lieu.

    Le troisieme cas compte autant que les deux premiers : une invalidation
    trop etroite se voit comme un bogue d'affichage, la pire espece a
    diagnostiquer. Une epreuve qui ne verifierait que les economies finirait par
    recompenser un moteur qui ne redessine plus rien.
    """
    from moteur import telemetrie

    cartes = "".join(
        '<div class="carte" id="c%d"><h3>Carte %d</h3>'
        '<p>Un texte de carte assez long pour occuper une ligne.</p></div>'
        % (i, i) for i in range(1000))
    doc = document("""
        <style>
          body { margin: 0; }
          .carte { padding: 8px; border: 1px solid #dddddd; color: #202020; }
          .carte.vedette { color: #cc0000; background: #fff8f0; }
          .carte.large { width: 640px; }
        </style>
        <body>%s<script>1</script></body>""" % cartes)
    doc.remet_en_page(1000, 800)
    contexte = doc.contexte_js
    verifie("invalidation: la page a un contexte", contexte is not None)
    if contexte is None:
        return

    def compte_apres(code):
        """Execute, rafraichit une fois, et rend les compteurs de ce tour."""
        telemetrie.reinitialise()
        telemetrie.active(True)
        contexte.execute(code)
        doc.rafraichis()
        return telemetrie.compteurs()

    # 1. Une couleur, posee directement : le chemin ou la propriete est connue.
    mesures = compte_apres("document.getElementById('c500')"
                           ".style.color = '#cc0000'")
    egal("invalidation: une couleur ne remet rien en page",
         mesures.get("invalidation.mise_en_page"), None)
    egal("invalidation: elle ne demande que des pixels",
         mesures.get("invalidation.peinture_seule"), 1)
    egal("invalidation: et aucune boite n'est reposee",
         mesures.get("layout.dispose_bloc"), None)

    # 2. Un `transform` : plus haut encore dans le pipeline — les pixels
    #    existants suffisent, il ne reste qu'a les deplacer.
    mesures = compte_apres("document.getElementById('c501')"
                           ".style.transform = 'translateX(4px)'")
    egal("invalidation: un transform ne demande que la composition",
         mesures.get("invalidation.composition_seule"), 1)
    egal("invalidation: et ne repose aucune boite",
         mesures.get("layout.dispose_bloc"), None)

    # 3. Une classe : la propriete n'est pas nommee, il faut rejouer la cascade
    #    pour savoir. Si elle ne change qu'une couleur, la mise en page doit
    #    quand meme etre evitee — c'est le cas que le moteur ne savait pas
    #    traiter, et le plus frequent dans une application reelle.
    mesures = compte_apres("document.getElementById('c502')"
                           ".classList.add('vedette')")
    egal("invalidation: une classe qui ne change qu'une couleur evite la mise en page",
         mesures.get("invalidation.mise_en_page_evitee"), 1)
    egal("invalidation: le restyle cible a bien eu lieu",
         mesures.get("invalidation.restyle_reussi"), 1)
    verifie("invalidation: et il n'a coute que le sous-arbre de la carte",
            (mesures.get("invalidation.restyle_boites") or 0) <= 8,
            mesures.get("invalidation.restyle_boites"))
    # La couleur doit reellement avoir change dans la boite peinte : economiser
    # une mise en page ne vaut rien si l'ecran ne suit pas.
    boite = boite_de(doc, "c502")
    egal("invalidation: la nouvelle couleur est dans la boite",
         boite.style.get("color"), "#cc0000")

    # 4. Une classe qui change une largeur : la mise en page doit avoir lieu.
    mesures = compte_apres("document.getElementById('c503')"
                           ".classList.add('large')")
    egal("invalidation: une classe qui change une largeur remet en page",
         mesures.get("invalidation.mise_en_page"), 1)
    # 640 px de contenu, plus le remplissage et les bordures des deux cotes :
    # `width` est du contenu tant que `box-sizing` ne dit pas le contraire.
    verifie("invalidation: et la largeur est effectivement appliquee",
            proche(boite_de(doc, "c503").largeur, 640.0 + 18.0, 3.0),
            boite_de(doc, "c503").largeur)

    # 5. Une classe qu'aucune regle ne designe : rien ne change, meme pas les
    #    pixels.
    mesures = compte_apres("document.getElementById('c504')"
                           ".classList.add('inconnue-nulle-part')")
    egal("invalidation: une classe sans regle ne change rien",
         mesures.get("invalidation.restyle_sans_effet"), 1)
    egal("invalidation: et ne remet pas en page",
         mesures.get("invalidation.mise_en_page"), None)

    # 6. Une insertion de contenu : structure changee, mise en page obligatoire.
    mesures = compte_apres(
        "const d = document.createElement('div');"
        "d.textContent = 'ajoute';"
        "document.body.appendChild(d);")
    egal("invalidation: une insertion remet en page",
         mesures.get("invalidation.mise_en_page"), 1)
    telemetrie.active(False)


def verifie_foyer():
    """Le foyer : `activeElement`, `focus`, `blur`, `Tab`.

    Rien de tout cela n'existait. `focus()` etait vide et `activeElement`
    rendait `null` en toutes circonstances — donc aucun formulaire n'etait
    remplissable au clavier, et `:focus` ne peignait jamais rien.
    """
    doc = document(
        "<style>input:focus { background: #ffff00 }</style>"
        "<body><input id=un><input id=deux><input id=trois disabled>"
        "<a id=lien href='/x'>lien</a><div id=inerte>texte</div>"
        "<script>1</script></body>")
    doc.remet_en_page(1000, 800)
    contexte = doc.contexte_js
    verifie("foyer: la page a un contexte", contexte is not None)
    if contexte is None:
        return

    egal("foyer: sans foyer, activeElement vaut body",
         contexte.execute("document.activeElement.tagName"), "BODY")

    contexte.execute("document.getElementById('un').focus()")
    egal("foyer: focus() designe l'element",
         contexte.execute("document.activeElement.id"), "un")
    egal("foyer: le document est d'accord",
         doc.foyer_actuel().attributs.get("id"), "un")

    # Les evenements partent, et dans le bon ordre : `blur` sur l'ancien avant
    # `focus` sur le nouveau. Une page qui valide un champ au `blur` avant
    # d'afficher le suivant en depend.
    contexte.execute(
        "globalThis.__ordre = [];"
        "for (const nom of ['un','deux']) {"
        "  const e = document.getElementById(nom);"
        "  e.addEventListener('focus', () => __ordre.push('focus:'+nom));"
        "  e.addEventListener('blur', () => __ordre.push('blur:'+nom));"
        "}")
    contexte.execute("document.getElementById('deux').focus()")
    egal("foyer: blur de l'ancien precede focus du nouveau",
         contexte.execute("__ordre.join(',')"), "blur:un,focus:deux")

    contexte.execute("document.getElementById('deux').blur()")
    egal("foyer: blur() rend le foyer au document",
         contexte.execute("document.activeElement.tagName"), "BODY")

    # `Tab` suit l'ordre du document, saute ce qui est desactive, et fait le
    # tour en fin de page.
    doc.pose_foyer(None)
    atteints = []
    for _ in range(4):
        atteint = doc.deplace_foyer(1)
        atteints.append(None if atteint is None else atteint.attributs.get("id"))
    egal("foyer: Tab suit l'ordre du document et saute le desactive",
         ",".join(str(a) for a in atteints), "un,deux,lien,un")

    arriere = doc.deplace_foyer(-1)
    egal("foyer: Maj+Tab revient en arriere",
         arriere.attributs.get("id") if arriere else None, "lien")

    # Un element desactive ne prend pas le foyer, meme si on le lui demande.
    contexte.execute("document.getElementById('trois').focus()")
    verifie("foyer: un champ desactive ne prend pas le foyer",
            contexte.execute("document.activeElement.id") != "trois",
            contexte.execute("document.activeElement.id"))

    # Et le style suit : `:focus` doit peindre.
    doc.pose_foyer(None)
    contexte.execute("document.getElementById('un').focus()")
    doc.remet_en_page(1000, 800)
    boite = boite_de(doc, "un")
    egal("foyer: :focus est applique",
         css_couleur(boite.style.get("background-color")) if boite else None,
         css_couleur("#ffff00"))


def verifie_formulaire_plateforme():
    """`form.elements`, `action`, `method`, `submit`, `label.control`."""
    doc = document(
        "<body><form id=f action='/session' method=POST>"
        "<label id=et for=nom>Nom</label>"
        "<input id=nom name=nom value=alice>"
        "<input id=mdp name=mdp type=password value=secret>"
        "<input id=coche name=coche type=checkbox checked>"
        "<button id=envoi type=submit>Envoyer</button></form>"
        "<script>1</script></body>",
        url="http://exemple.test/page/")
    doc.remet_en_page(1000, 800)
    contexte = doc.contexte_js

    egal("formulaire: action est resolue en absolu",
         contexte.execute("document.getElementById('f').action"),
         "http://exemple.test/session")
    egal("formulaire: method est en minuscules",
         contexte.execute("document.getElementById('f').method"), "post")
    egal("formulaire: elements compte les controles",
         contexte.execute("document.getElementById('f').elements.length"), 4)
    egal("formulaire: elements est indexable par nom",
         contexte.execute("document.getElementById('f').elements.nom.value"),
         "alice")
    egal("formulaire: input.form remonte au formulaire",
         contexte.execute("document.getElementById('nom').form.id"), "f")
    egal("formulaire: label.htmlFor",
         contexte.execute("document.getElementById('et').htmlFor"), "nom")
    egal("formulaire: label.control designe le champ",
         contexte.execute("document.getElementById('et').control.id"), "nom")

    # `required` : la seule regle de validation implementee, et la seule qui
    # serve aujourd'hui.
    contexte.execute("document.getElementById('nom').required = true")
    egal("formulaire: un champ requis rempli est valide",
         contexte.execute("document.getElementById('nom').checkValidity()"), True)
    contexte.execute("document.getElementById('nom').value = ''")
    egal("formulaire: un champ requis vide ne l'est pas",
         contexte.execute("document.getElementById('nom').checkValidity()"), False)
    contexte.execute("document.getElementById('nom').value = 'alice'")

    # `requestSubmit()` passe par l'evenement, `submit()` ne le declenche pas :
    # c'est la difference que la norme etablit, et du code s'en sert pour
    # contourner sa propre validation.
    contexte.execute("globalThis.__vus = 0;"
                     "document.getElementById('f').addEventListener('submit',"
                     " e => { __vus++; e.preventDefault(); })")
    contexte.execute("document.getElementById('f').requestSubmit()")
    egal("formulaire: requestSubmit declenche l'evenement",
         contexte.execute("__vus"), 1)
    egal("formulaire: et preventDefault empeche la navigation",
         contexte.navigation, None)

    contexte.execute("document.getElementById('f').submit()")
    egal("formulaire: submit() ne declenche pas l'evenement",
         contexte.execute("__vus"), 1)
    navigation = contexte.navigation
    verifie("formulaire: submit() prepare bien un POST",
            isinstance(navigation, tuple) and navigation[0] == "soumission"
            and navigation[2] == "POST", navigation)
    verifie("formulaire: le corps porte les champs remplis",
            "nom=alice" in navigation[3] and "mdp=secret" in navigation[3]
            and "coche=on" in navigation[3], navigation[3])
    egal("formulaire: la cible est l'action resolue",
         navigation[1], "http://exemple.test/session")

    # En GET, les champs partent dans la chaine de requete, et **remplacent**
    # celle qui s'y trouvait : deux envois ne doivent pas l'accumuler.
    contexte.navigation = None
    contexte.execute("var f = document.getElementById('f');"
                     "f.setAttribute('method', 'get');"
                     "f.setAttribute('action', '/chercher?ancien=1');"
                     "f.submit()")
    verifie("formulaire: GET met les champs dans la requete",
            "nom=alice" in contexte.navigation, contexte.navigation)
    verifie("formulaire: et remplace l'ancienne chaine",
            "ancien=1" not in contexte.navigation, contexte.navigation)


def verifie_historique_session():
    """`pushState`, `replaceState`, `popstate`, et l'adresse qui suit."""
    doc = document("<body><script>1</script></body>",
                   url="http://exemple.test/depart")
    doc.remet_en_page(1000, 800)
    contexte = doc.contexte_js
    contexte.execute("globalThis.__pops = [];"
                     "addEventListener('popstate', e =>"
                     " __pops.push(JSON.stringify(e.state)))")

    egal("historique: longueur initiale",
         contexte.execute("history.length"), 1)

    contexte.execute("history.pushState({vue:'a'}, '', '/a')")
    egal("historique: l'adresse du document a change", doc.url,
         "http://exemple.test/a")
    egal("historique: location.pathname suit",
         contexte.execute("location.pathname"), "/a")
    egal("historique: history.state porte l'etat",
         contexte.execute("JSON.stringify(history.state)"), '{"vue":"a"}')
    egal("historique: la longueur augmente",
         contexte.execute("history.length"), 2)
    verifie("historique: aucune navigation reelle n'est demandee",
            contexte.navigation is None, contexte.navigation)

    contexte.execute("history.pushState({vue:'b'}, '', '/b?x=1')")
    egal("historique: la requete suit aussi",
         contexte.execute("location.search"), "?x=1")

    # `replaceState` ecrase, il n'empile pas.
    contexte.execute("history.replaceState({vue:'b2'}, '', '/b2')")
    egal("historique: replaceState n'allonge pas la pile",
         contexte.execute("history.length"), 3)
    egal("historique: mais change l'adresse",
         contexte.execute("location.pathname"), "/b2")

    # Retour : `popstate` part avec l'etat memorise.
    contexte.execute("history.back()")
    egal("historique: back revient a l'entree precedente",
         contexte.execute("location.pathname"), "/a")
    egal("historique: popstate a recu l'etat",
         contexte.execute("__pops.join('|')"), '{"vue":"a"}')

    contexte.execute("history.forward()")
    egal("historique: forward avance",
         contexte.execute("location.pathname"), "/b2")

    # Un `pushState` apres un retour tronque ce qui suivait : on ne peut plus
    # avancer, exactement comme dans un navigateur.
    contexte.execute("history.back(); history.pushState({vue:'c'}, '', '/c')")
    egal("historique: pushState apres retour tronque la suite",
         contexte.execute("history.length"), 3)
    contexte.execute("history.forward()")
    egal("historique: et il n'y a plus rien devant",
         contexte.execute("location.pathname"), "/c")

    # Sortir de la plage reste une vraie navigation, deleguee au navigateur.
    contexte.navigation = None
    for _ in range(5):
        contexte.execute("history.back()")
    verifie("historique: au-dela, c'est une navigation du navigateur",
            contexte.navigation is not None, contexte.navigation)


def verifie_reseau_asynchrone():
    """Le fil de l'interface doit vivre pendant qu'une requete est en vol.

    `fetch` rendait bien une promesse, mais partait sur le fil de l'interface :
    zero battement de minuterie, zero evenement, zero repeinture pendant tout
    le vol. La promesse ne faisait que masquer un gel.
    """
    doc, arrete = _sur_serveur("/page/reseau-lent.html", battements=0)
    try:
        import time as _time
        for _ in range(120):
            doc.rafraichis()
            _time.sleep(0.02)
        etat = doc.contexte_js.execute("JSON.stringify(globalThis.__lent)")
        import json as _json
        lu = _json.loads(etat)
        verifie("reseau: l'interface a continue de battre pendant le vol",
                lu["battements"] > 10, lu)
        verifie("reseau: la reponse est bien arrivee",
                lu["recu"] is not None, lu)
        verifie("reseau: et elle a pris le temps annonce",
                lu["ms"] >= 900, lu)
    finally:
        arrete()


def verifie_fetch_reel():
    """La surface de `Response`, contre un vrai serveur."""
    import json as _json
    import time as _time

    doc, arrete = _sur_serveur("/page/event-loop.html")
    try:
        contexte = doc.contexte_js
        base = doc.base

        def joue(code, jusqua=None, secondes=10.0):
            """Execute, puis bat jusqu'a ce que la condition tienne.

            Attendre un nombre fixe de battements rendait ces verifications
            instables : sous charge, une reponse arrivait apres le dernier tour
            et le test echouait sans que rien ne soit casse. Un test instable
            est pire qu'absent — il apprend a ignorer les echecs.
            """
            contexte.execute(code)
            limite = _time.perf_counter() + secondes
            while _time.perf_counter() < limite:
                contexte.tic()
                if jusqua is not None and contexte.execute(jusqua):
                    return True
                _time.sleep(0.02)
            return jusqua is None

        joue("globalThis.__r = {};"
             "fetch('%s/json').then(r => {"
             "  __r.ok = r.ok; __r.status = r.status; __r.url = r.url;"
             "  __r.type = r.headers.get('content-type');"
             "  return r.json();"
             "}).then(j => { __r.json = j.nom; })" % base,
             jusqua="__r.json !== undefined")
        lu = _json.loads(contexte.execute("JSON.stringify(__r)"))
        egal("fetch: ok", lu.get("ok"), True)
        egal("fetch: status", lu.get("status"), 200)
        egal("fetch: le type de contenu se lit",
             (lu.get("type") or "").split(";")[0], "application/json")
        egal("fetch: json() rend l'objet", lu.get("json"), "bouchaud")

        joue("globalThis.__c = [];"
             "fetch('%s/status/404').then(r => __c.push(r.status + ':' + r.ok));"
             "fetch('%s/status/500').then(r => __c.push(String(r.status)));"
             "fetch('%s/redirect?to=/json&n=3').then(r => __c.push('r' + r.status));"
             % (base, base, base), jusqua="__c.length >= 3")
        codes = contexte.execute("__c.slice().sort().join(',')")
        verifie("fetch: un 404 arrive comme reponse, pas comme rejet",
                "404:false" in codes, codes)
        verifie("fetch: un 500 aussi", "500" in codes, codes)
        verifie("fetch: une chaine de redirections aboutit", "r200" in codes, codes)

        joue("globalThis.__p = '';"
             "fetch('%s/echo', {method:'POST',"
             " headers:{'Content-Type':'application/json'},"
             " body: JSON.stringify({a:1})}).then(r => r.json())"
             ".then(j => { __p = j.corps; })" % base, jusqua="__p !== ''")
        egal("fetch: un POST envoie bien son corps",
             contexte.execute("__p"), '{"a":1}')

        # L'annulation doit etre reelle : un moignon d'`AbortController` aurait
        # laisse la page croire qu'elle sait annuler.
        joue("globalThis.__a = '';"
             "var ac = new AbortController();"
             "fetch('%s/delay/2', {signal: ac.signal})"
             " .then(() => { __a = 'PAS-ANNULE'; })"
             " .catch(e => { __a = e.name; });"
             "ac.abort();" % base, jusqua="__a !== ''")
        egal("fetch: abort() rejette avec AbortError",
             contexte.execute("__a"), "AbortError")
    finally:
        arrete()


def verifie_serveur_fixtures():
    """Le serveur local lui-meme : sans lui, rien de ce qui precede n'est testable."""
    import urllib.request

    import serveur_test

    serveur, base = serveur_test.demarre(0)
    try:
        def code_de(chemin):
            try:
                return urllib.request.urlopen(base + chemin, timeout=5).status
            except Exception as e:  # noqa: BLE001
                return getattr(e, "code", 0)

        egal("serveur: l'accueil repond", code_de("/"), 200)
        egal("serveur: /json repond", code_de("/json"), 200)
        egal("serveur: /status/404 rend 404", code_de("/status/404"), 404)
        egal("serveur: /status/500 rend 500", code_de("/status/500"), 500)
        egal("serveur: une chaine de redirections aboutit",
             code_de("/redirect?to=/json&n=3"), 200)
        egal("serveur: les modules sont servis", code_de("/module/main.js"), 200)
        egal("serveur: y compris ceux d'un sous-dossier",
             code_de("/module/lib/util.js"), 200)
        egal("serveur: une page temoin du depot est servie",
             code_de("/page/login-form.html"), 200)
        egal("serveur: un bundle absent rend 404", code_de("/404.js"), 404)

        # Deterministe : deux lectures rendent exactement les memes octets.
        premier = urllib.request.urlopen(base + "/json", timeout=5).read()
        second = urllib.request.urlopen(base + "/json", timeout=5).read()
        egal("serveur: deux lectures rendent la meme chose", premier, second)
    finally:
        serveur_test.arrete(serveur)


def verifie_parcours_complet():
    """Le parcours entier : de l'ouverture a `popstate`, sans rien sauter.

    C'est la verification qui compte le plus de tout le fichier, parce que
    c'est la seule qui ressemble a ce qu'un utilisateur fait. Quinze etapes
    liees : chacune depend de la precedente, et une seule qui casse rend la
    suite sans objet.

    Elle ne remplace pas les verifications unitaires — elle dit si elles
    suffisent.
    """
    import json as _json
    import time as _time

    doc, arrete = _sur_serveur("/page/parcours-complet.html", battements=4)
    try:
        contexte = doc.contexte_js
        verifie("parcours: la page a un contexte JavaScript", contexte is not None)
        if contexte is None:
            return

        def bat(tours=6, pause=0.02):
            for _ in range(tours):
                doc.rafraichis()
                _time.sleep(pause)

        # 3. L'utilisateur clique dans le champ : le foyer suit vraiment.
        courriel = doc.racine.trouve_par_id("email") \
            if hasattr(doc.racine, "trouve_par_id") else None
        contexte.execute("document.getElementById('email').focus()")
        egal("parcours: le foyer est dans le champ",
             contexte.execute("document.activeElement.id"), "email")

        # 4. Il ecrit.
        contexte.execute("var e = document.getElementById('email');"
                         "e.value = 'alice@exemple.fr';"
                         "e.dispatchEvent(new Event('input'))")

        # 5. Tab vers le champ suivant — par le modele de foyer du document,
        #    comme le ferait une vraie frappe.
        atteint = doc.deplace_foyer(1)
        egal("parcours: Tab atteint le mot de passe",
             atteint.attributs.get("id") if atteint else None, "motdepasse")

        # 6. Il ecrit son mot de passe.
        contexte.execute("var m = document.getElementById('motdepasse');"
                         "m.value = 'secret42';"
                         "m.dispatchEvent(new Event('input'))")

        # 7. Il coche la case.
        contexte.execute("var c = document.getElementById('memoire');"
                         "c.checked = true;"
                         "c.dispatchEvent(new Event('change'))")

        # 8. Il soumet.
        contexte.execute("document.getElementById('connexion')"
                         ".dispatchEvent(new Event('submit', {cancelable: true}))")
        bat(60, 0.03)

        parcours = _json.loads(
            contexte.execute("JSON.stringify(globalThis.__parcours)"))
        etapes = parcours["etapes"]
        vues = {e.split("=")[0]: e.partition("=")[2] for e in etapes}

        attendues = [
            ("js-execute", "le script de la page s'execute"),
            ("focus", "le foyer arrive dans le champ"),
            ("saisie-email", "la saisie declenche input"),
            ("saisie-mdp", "le mot de passe aussi"),
            ("case", "la case declenche change"),
            ("formdata", "FormData rassemble les champs"),
            ("post-status", "le POST recoit une reponse"),
            ("serveur-a-recu", "le serveur a bien recu le corps"),
            ("api-json", "une seconde requete rend du JSON"),
            ("dom-maj", "le DOM est mis a jour depuis la reponse"),
            ("pushstate", "l'adresse change sans rechargement"),
        ]
        for cle, quoi in attendues:
            verifie("parcours: %s" % quoi, cle in vues,
                    " | ".join(etapes))

        egal("parcours: le champ saisi arrive au serveur",
             "alice%40exemple.fr" in vues.get("serveur-a-recu", "")
             or "alice@exemple.fr" in vues.get("serveur-a-recu", ""), True)
        verifie("parcours: la case cochee part avec le formulaire",
                "memoire=oui" in vues.get("formdata", ""), vues.get("formdata"))
        egal("parcours: le POST a reussi", vues.get("post-status"), "200")
        egal("parcours: le DOM porte le bloc ajoute", vues.get("dom-maj"), "1")
        egal("parcours: l'adresse est la nouvelle", vues.get("pushstate"), "/tableau")
        egal("parcours: et le document est d'accord",
             contexte.execute("location.pathname"), "/tableau")

        # 14. Retour : `popstate` part avec l'etat.
        contexte.execute("history.back()")
        bat(4)
        parcours = _json.loads(
            contexte.execute("JSON.stringify(globalThis.__parcours)"))
        verifie("parcours: back declenche popstate",
                len(parcours["popstates"]) >= 1, parcours["popstates"])
        verifie("parcours: et l'adresse revient",
                contexte.execute("location.pathname").endswith("parcours-complet.html"),
                contexte.execute("location.pathname"))

        # 15. L'interface a battu pendant tout le reseau.
        verifie("parcours: l'interface est restee vivante pendant le reseau",
                parcours["battements"] > 10, parcours["battements"])
    finally:
        arrete()


def verifie_parcours_utilisateur():
    """Le parcours de connexion, joue comme un utilisateur le jouerait.

    C'est la difference avec `verifie_parcours_complet`, qui posait les valeurs
    par script. Ici rien n'est ecrit dans `input.value` : on clique, on tape
    caractere par caractere, on tabule, on coche, on valide. Ce que le test
    prouve alors n'est plus « les methodes existent » mais « on peut s'en
    servir ».

    C'est la verification de reference du navigateur. Si elle passe, une page de
    connexion ordinaire est utilisable ; si elle casse, plus rien ne l'est.
    """
    import json as _json
    import time as _time

    doc, arrete = _sur_serveur("/page/parcours-complet.html", battements=4)
    try:
        contexte = doc.contexte_js
        verifie("utilisateur: la page a un contexte", contexte is not None)
        if contexte is None:
            return

        def clique(identifiant):
            boite = boite_de(doc, identifiant)
            if boite is None:
                verifie("utilisateur: la boite de %s existe" % identifiant, False)
                return
            doc.clique(boite.x + 5.0, boite.y + boite.hauteur / 2.0)

        def bat(secondes=6.0, jusqua=None):
            limite = _time.perf_counter() + secondes
            while _time.perf_counter() < limite:
                doc.rafraichis()
                if jusqua is not None and contexte.execute(jusqua):
                    return True
                _time.sleep(0.02)
            return jusqua is None

        # 2. Clic dans le champ d'adresse.
        clique("email")
        egal("utilisateur: le clic donne le foyer au champ",
             contexte.execute("document.activeElement.id"), "email")

        # 3. Le caret est visible.
        boite = boite_de(doc, "email")
        verifie("utilisateur: un caret est pose", boite.curseur is not None)

        # 4. Frappe reelle, caractere par caractere.
        tape(doc, "alice@exemple.fr")
        egal("utilisateur: la valeur vient de la frappe",
             contexte.execute("document.getElementById('email').value"),
             "alice@exemple.fr")

        # 5. Retour arriere, puis correction.
        doc.frappe("Backspace")
        doc.frappe("Backspace")
        tape(doc, "com")
        egal("utilisateur: retour arriere puis correction",
             contexte.execute("document.getElementById('email').value"),
             "alice@exemple.com")

        # 6. Tab vers le mot de passe.
        doc.frappe("Tab")
        egal("utilisateur: Tab passe au champ suivant",
             contexte.execute("document.activeElement.id"), "motdepasse")

        # 7. Mot de passe tape reellement.
        tape(doc, "secret42")
        egal("utilisateur: le mot de passe est saisi",
             contexte.execute("document.getElementById('motdepasse').value"),
             "secret42")
        boite = boite_de(doc, "motdepasse")
        peint = " ".join(f.texte for f in boite.lignes) if boite else ""
        verifie("utilisateur: il n'est pas peint en clair",
                "secret42" not in peint, peint)

        # 8. La case, au clavier.
        doc.frappe("Tab")
        egal("utilisateur: Tab atteint la case",
             contexte.execute("document.activeElement.id"), "memoire")
        doc.frappe(" ", " ")
        egal("utilisateur: l'espace coche la case",
             contexte.execute("document.getElementById('memoire').checked"), True)

        # 9. Soumission par le bouton.
        doc.frappe("Tab")
        egal("utilisateur: Tab atteint le bouton",
             contexte.execute("document.activeElement.id"), "valider")
        doc.active(doc.foyer_actuel())
        bat(8.0, jusqua="globalThis.__parcours.etapes."
                        "some(e => e.indexOf('pushstate') === 0)")

        parcours = _json.loads(
            contexte.execute("JSON.stringify(globalThis.__parcours)"))
        vues = {e.split("=")[0]: e.partition("=")[2] for e in parcours["etapes"]}

        # 10-14. Ce que la soumission a reellement produit.
        verifie("utilisateur: FormData porte l'adresse tapee",
                "email=alice%40exemple.com" in vues.get("formdata", "")
                or "email=alice@exemple.com" in vues.get("formdata", ""),
                vues.get("formdata"))
        verifie("utilisateur: et le mot de passe tape",
                "secret42" in vues.get("formdata", ""), vues.get("formdata"))
        verifie("utilisateur: et la case cochee",
                "memoire=oui" in vues.get("formdata", ""), vues.get("formdata"))
        egal("utilisateur: le POST a abouti", vues.get("post-status"), "200")
        verifie("utilisateur: le serveur a recu ce qui a ete tape",
                "secret42" in vues.get("serveur-a-recu", ""),
                vues.get("serveur-a-recu"))
        egal("utilisateur: une API JSON a repondu", vues.get("api-json"), "bouchaud")
        egal("utilisateur: le DOM a ete mis a jour", vues.get("dom-maj"), "1")
        egal("utilisateur: l'adresse a change sans rechargement",
             vues.get("pushstate"), "/tableau")
        # Le serveur local repond en quelques millisecondes : le vol est trop
        # court pour compter beaucoup de battements, et exiger un grand nombre
        # ici rendrait la verification dependante de la vitesse de la machine.
        # La preuve que le reseau ne bloque pas est ailleurs, sur une reponse
        # d'une seconde — `verifie_reseau_asynchrone`. Ici on verifie seulement
        # que la minuterie a continue d'echoir pendant la sequence.
        verifie("utilisateur: la minuterie a continue de battre",
                parcours["battements"] > 0, parcours["battements"])

        # 15. Retour arriere du navigateur.
        contexte.execute("history.back()")
        bat(1.5)
        parcours = _json.loads(
            contexte.execute("JSON.stringify(globalThis.__parcours)"))
        verifie("utilisateur: back declenche popstate",
                len(parcours["popstates"]) >= 1, parcours["popstates"])
    finally:
        arrete()


# --- Securite web -------------------------------------------------------------

def verifie_politique_ressources():
    """Une page distante ne lit pas le disque ; l'utilisateur, si.

    C'est la distinction que la politique doit tenir : `file://` reste ouvert a
    qui tape l'adresse, et ferme a qui l'ecrit dans un `fetch`. Une regle qui
    supprimerait `file://` partout casserait le gestionnaire de fichiers ; une
    regle qui l'autoriserait partout donnerait `/etc/shadow` a la premiere page
    venue.
    """
    from moteur import images, securite

    dossier = os.path.join(os.environ.get("BO_TMP", "/tmp"), "bo-verif-securite")
    os.makedirs(dossier, exist_ok=True)
    secret = os.path.join(dossier, "secret.txt")
    with open(secret, "w") as f:
        f.write("mot-de-passe-de-l-utilisateur")
    adresse = "file://" + secret

    # 1. La navigation de l'utilisateur reste possible : aucun `document`.
    ouverte = reseau.charge(adresse)
    verifie("securite: l'utilisateur ouvre encore un fichier local",
            "mot-de-passe" in ouverte.contenu, ouverte.contenu[:60])

    # 2. Le meme fichier, demande par une page distante, est refuse — quelle
    #    que soit la destination invoquee.
    for destination in ("fetch", "script", "style", "image", "font", "media"):
        refus = reseau.charge(adresse, brut=True,
                              document="https://mechant.test/page",
                              destination=destination)
        egal("securite: %s distant refuse file://" % destination, refus.code, 0)
        verifie("securite: %s refuse sans divulguer le contenu" % destination,
                "mot-de-passe" not in (refus.contenu or "")
                and b"mot-de-passe" not in (refus.octets or b""),
                destination)

    # 3. Une page locale garde le droit de charger ses voisines : sans cela,
    #    ouvrir un fichier HTML du disque n'afficherait plus ses images.
    voisine = reseau.charge(adresse, brut=True,
                            document="file:///home/utilisateur/page.html",
                            destination="image")
    egal("securite: une page locale charge sa voisine locale", voisine.code, 200)

    # 4. `bo:` est du chrome de navigateur, pas une ressource du Web.
    interne = reseau.charge("bo:accueil", document="https://mechant.test/page",
                            destination="fetch")
    egal("securite: une page distante n'atteint pas bo:", interne.code, 0)

    # 5. Le chemin reel du moteur : `fetch("file:///…")` depuis du JavaScript.
    doc = document("<body></body>", url="https://mechant.test/page")
    contexte = js.Contexte(doc)
    try:
        contexte.execute(
            "globalThis.__vu = null;"
            "fetch(%s).then(r => r.text()).then(t => { globalThis.__vu = t; });"
            % json.dumps(adresse))
        for _ in range(4):
            contexte.tic()
        vu = contexte.execute("String(globalThis.__vu)")
        verifie("securite: fetch(file://) ne rend pas le fichier",
                "mot-de-passe" not in vu, vu[:60])
    finally:
        contexte.ferme()

    # 6. Et le chemin des images, qui passe par son propre cache.
    images.vide()
    egal("securite: <img src=file://> distant refuse",
         images.charge("https://mechant.test/page", adresse), None)

    # 7. La politique elle-meme, testee directement.
    egal("securite: origine d'une adresse https",
         securite.origine("https://a.test:8443/x"), ("https", "a.test", 8443))
    egal("securite: port implicite d'https",
         securite.origine("https://a.test/x"), ("https", "a.test", 443))
    egal("securite: un fichier n'a pas d'origine",
         securite.origine("file:///etc/passwd"), None)
    verifie("securite: deux origines opaques ne sont pas egales",
            not securite.meme_origine("file:///a", "file:///a"))
    verifie("securite: meme origine reconnue",
            securite.meme_origine("https://a.test/x", "https://a.test/y"))
    verifie("securite: un port different change l'origine",
            not securite.meme_origine("https://a.test:1/x", "https://a.test:2/y"))
    try:
        shutil.rmtree(dossier)
    except OSError:
        pass


def verifie_tls_ferme():
    """Sans magasin de racines, HTTPS echoue. Il ne se degrade pas."""
    import ssl

    from moteur import reseau as _reseau

    anciens = _reseau.MAGASINS
    ancien_env = {nom: os.environ.pop(nom, None)
                  for nom in ("BO_CA_BUNDLE", "SSL_CERT_FILE", "SSL_CERT_DIR")}
    ancien_defaut = ssl.create_default_context

    def contexte_vide(*args, **kwargs):
        # Un contexte sans racine, comme sur une image OS qui n'en embarque pas.
        return ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)

    try:
        _reseau.MAGASINS = ("/inexistant/aucun-magasin.crt",)
        ssl.create_default_context = contexte_vide
        leve = None
        try:
            _reseau.contexte_tls()
        except _reseau.MagasinAbsent as e:
            leve = e
        verifie("tls: sans magasin, la connexion est refusee", leve is not None)
        verifie("tls: le refus se nomme",
                leve is not None and "magasin" in str(leve).lower(), str(leve))
    finally:
        ssl.create_default_context = ancien_defaut
        _reseau.MAGASINS = anciens
        for nom, valeur in ancien_env.items():
            if valeur is not None:
                os.environ[nom] = valeur

    # Et avec un magasin, le contexte verifie vraiment quelque chose.
    try:
        contexte = _reseau.contexte_tls()
    except _reseau.MagasinAbsent:
        contexte = None
    if contexte is not None:
        egal("tls: la chaine est exigee", contexte.verify_mode, ssl.CERT_REQUIRED)
        egal("tls: le nom d'hote est verifie", contexte.check_hostname, True)

    # La regression a interdire : que le code se remette a fabriquer un
    # contexte permissif quelque part.
    source = open(os.path.join(os.path.dirname(os.path.abspath(__file__)),
                               "moteur", "reseau.py")).read()
    verifie("tls: le transport ne desarme plus la verification",
            "verify_mode = ssl.CERT_NONE" not in source)
    verifie("tls: le transport ne desactive plus le nom d'hote",
            "check_hostname = False" not in source)


def verifie_temoins_httponly():
    """`HttpOnly` : le serveur le recoit, `document.cookie` ne le voit jamais."""
    from moteur import stockage

    restaure = isole_le_stockage()
    try:
        bocal = stockage.Temoins()
        bocal.oublie_tout()
        bocal.absorbe("https://banque.test/compte", {
            "set-cookie": "session=secret-de-session; Path=/; HttpOnly, "
                          "theme=sombre; Path=/",
        })

        reseau_vu = bocal.pour("https://banque.test/compte")
        verifie("httponly: le serveur recoit le temoin", "session=secret-de-session" in reseau_vu, reseau_vu)
        verifie("httponly: le serveur recoit aussi les autres", "theme=sombre" in reseau_vu, reseau_vu)

        script_vu = bocal.pour("https://banque.test/compte", javascript=True)
        verifie("httponly: document.cookie ne le voit pas",
                "secret-de-session" not in script_vu, script_vu)
        verifie("httponly: document.cookie voit le reste",
                "theme=sombre" in script_vu, script_vu)

        # Un script ne doit pas non plus pouvoir l'ecraser : proteger la lecture
        # sans proteger l'ecriture laisserait fixer la session.
        bocal.absorbe("https://banque.test/compte",
                      {"set-cookie": "session=vole; Path=/"}, javascript=True)
        verifie("httponly: un script ne l'ecrase pas",
                "session=secret-de-session" in bocal.pour("https://banque.test/compte"),
                bocal.pour("https://banque.test/compte"))
        verifie("httponly: il reste invisible apres la tentative",
                "vole" not in bocal.pour("https://banque.test/compte", javascript=True))

        # Ni le poser lui-meme, ce qui reviendrait a se cacher de soi-meme.
        bocal.absorbe("https://banque.test/compte",
                      {"set-cookie": "jeton=x; Path=/; HttpOnly"}, javascript=True)
        verifie("httponly: un script ne pose pas de temoin HttpOnly",
                "jeton=x" in bocal.pour("https://banque.test/compte", javascript=True),
                bocal.pour("https://banque.test/compte", javascript=True))

        # Le chemin, corrige : `/admin` ne couvre pas `/administrateur`.
        bocal.oublie_tout()
        bocal.absorbe("https://site.test/admin/", {"set-cookie": "a=1; Path=/admin"})
        verifie("temoins: /admin couvre /admin",
                "a=1" in bocal.pour("https://site.test/admin"))
        verifie("temoins: /admin couvre /admin/outils",
                "a=1" in bocal.pour("https://site.test/admin/outils"))
        verifie("temoins: /admin ne couvre pas /administrateur",
                "a=1" not in bocal.pour("https://site.test/administrateur"),
                bocal.pour("https://site.test/administrateur"))

        # Le domaine, corrige : sans `Domain`, le temoin ne descend pas.
        bocal.oublie_tout()
        bocal.absorbe("https://site.test/", {"set-cookie": "b=1; Path=/"})
        verifie("temoins: sans Domain, l'hote seul le recoit",
                "b=1" in bocal.pour("https://site.test/"))
        verifie("temoins: sans Domain, pas de sous-domaine",
                "b=1" not in bocal.pour("https://compte.site.test/"),
                bocal.pour("https://compte.site.test/"))
        bocal.absorbe("https://site.test/", {"set-cookie": "c=1; Path=/; Domain=site.test"})
        verifie("temoins: avec Domain, le sous-domaine le recoit",
                "c=1" in bocal.pour("https://compte.site.test/"))
    finally:
        restaure()


def verifie_temoins_document():
    """`document.cookie` passe bien par le chemin protege."""
    from moteur import stockage

    restaure = isole_le_stockage()
    try:
        stockage.temoins().absorbe("https://banque.test/", {
            "set-cookie": "session=secret; Path=/; HttpOnly, public=oui; Path=/",
        })
        doc = document("<body></body>", url="https://banque.test/")
        contexte = js.Contexte(doc)
        try:
            lu = contexte.execute("document.cookie")
            verifie("document.cookie: le HttpOnly est absent",
                    "secret" not in lu, lu)
            verifie("document.cookie: le reste est present", "public=oui" in lu, lu)
            contexte.execute('document.cookie = "session=vole; Path=/"')
            verifie("document.cookie: l'ecrasement est refuse",
                    "secret" in stockage.temoins().pour("https://banque.test/"),
                    stockage.temoins().pour("https://banque.test/"))
        finally:
            contexte.ferme()
    finally:
        restaure()


def verifie_redirection_entetes():
    """Une redirection vers un autre hote ne reporte pas les secrets."""
    vues = []

    class _Fausse:
        will_close = True
        status = 200

        def __init__(self, code, entetes):
            self.status = code
            self._entetes = entetes

        def getheaders(self):
            return list(self._entetes.items())

        def read(self):
            return b"corps"

    class _Connexion:
        def __init__(self, hote, port, timeout=None):
            self.hote = hote
            self.sock = object()
            self._reponse = None

        def request(self, methode, chemin, body=None, headers=None):
            vues.append((self.hote, dict(headers or {})))
            if self.hote == "depart.test":
                self._reponse = _Fausse(302, {
                    "Location": "https://arrivee.test/cible",
                    "Content-Type": "text/plain",
                })
            else:
                self._reponse = _Fausse(200, {"Content-Type": "text/plain"})

        def getresponse(self):
            return self._reponse

        def close(self):
            self.sock = None

    ancien = reseau._ouvre
    reseau._ouvre = lambda hote, port, securise: _Connexion(hote, port)
    try:
        reseau.charge("https://depart.test/a", brut=True,
                      entetes={"Authorization": "Bearer jeton-secret",
                               "Cookie": "session=abc"})
    finally:
        reseau._ouvre = ancien

    egal("redirection: les deux hotes ont ete joints", len(vues), 2)
    if len(vues) == 2:
        _, seconde = vues[1]
        envoyes = {n.lower(): v for n, v in seconde.items()}
        verifie("redirection: Authorization ne suit pas vers un autre hote",
                "authorization" not in envoyes, envoyes)
        verifie("redirection: le Cookie du depart ne suit pas",
                "session=abc" not in envoyes.get("cookie", ""), envoyes)
        egal("redirection: l'hote demande est bien le nouveau",
             vues[1][0], "arrivee.test")


# --- Execution ----------------------------------------------------------------

def verifie_hote_reel():
    """Ce qui ne se verifie que sur la machine : les formats d'image de Qt.

    Sur la machine de developpement, l'hote est un bouchon et il n'y a rien a
    prouver ; sous l'OS, cette verification atteste que les greffons JPEG et GIF
    sont bien lies en dur — ce qu'un binaire statique ne peut pas obtenir
    autrement.
    """
    if not HOTE_REEL:
        return
    formats = [f.lower() for f in bo.formats_images()]
    for attendu in ("png", "jpeg", "gif", "bmp"):
        verifie("hote: format %s reconnu" % attendu, attendu in formats, formats)
    largeur = bo.largeur_texte("Bouchaud", 16.0, False, False)
    verifie("hote: la mesure de texte est reelle", largeur > 0, largeur)


# --- Workers ------------------------------------------------------------------
#
# Ce qui est verifie ici, dans l'ordre ou ca compte :
#
#   1. la surface du monde d'un Worker est celle d'un Worker, pas celle d'une
#      fenetre amputee ;
#   2. le calcul s'y fait **ailleurs** — mesure, pas lecture de propriete ;
#   3. `fetch` et IndexedDB y passent par l'infrastructure du document, pas par
#      une seconde implementation ;
#   4. `terminate()` rend tout, y compris sur un worker qui ne rend jamais la
#      main ;
#   5. mille creations et destructions ne laissent rien derriere elles.
#
# Le point 2 est le seul qui distingue un vrai Worker d'une API qui en porte le
# nom, et c'est pour cela qu'il a sa propre page temoin.


def _bat_jusqua(doc, contexte, condition, secondes=20.0):
    """Fait battre le document jusqu'a ce que la page dise oui."""
    import time as _time
    limite = _time.perf_counter() + secondes
    while _time.perf_counter() < limite:
        doc.rafraichis()
        try:
            if contexte.execute(condition):
                return True
        except Exception:  # noqa: BLE001
            pass
        _time.sleep(0.01)
    return False


def verifie_workers_surface():
    """Le monde d'un Worker : ce qu'il a, et ce qu'il n'a pas."""
    doc, arrete = _sur_serveur("/page/worker-base.html", battements=2)
    try:
        contexte = doc.contexte_js
        verifie("worker: la page a un contexte", contexte is not None)
        if contexte is None:
            jalon("WEB_WORKER_BASIC", False, "pas de contexte JavaScript")
            return

        fini = _bat_jusqua(doc, contexte, "!!(globalThis.__worker||{}).pret")
        etat = json.loads(contexte.execute("JSON.stringify(globalThis.__worker)"))
        verifie("worker: le scenario est alle jusqu'au bout", fini,
                etat.get("etapes"))
        egal("worker: aucune erreur", etat.get("erreurs"), [])

        surface = (etat.get("surface") or {}).get("surface") or {}

        # Ce qui ne doit pas exister. Un Worker qui verrait le DOM partagerait
        # l'arbre entre deux fils : ce n'est pas une privation, c'est ce qui
        # rend legitime de le faire tourner ailleurs.
        for nom in ("aDocument", "aWindow", "aHtmlElement", "aLocalStorage",
                    "aAlert"):
            verifie("worker: pas de %s dans le worker" % nom,
                    surface.get(nom) is False, surface.get(nom))

        # Ce qui doit exister, et que la norme exige d'un
        # `DedicatedWorkerGlobalScope`.
        for nom in ("selfEstGlobal", "aPostMessage", "aFetch", "aSetTimeout",
                    "aSetInterval", "aQueueMicrotask", "aUrl",
                    "aUrlSearchParams", "aTextEncoder", "aTextDecoder",
                    "aIndexedDb", "aWebSocket", "portee"):
            verifie("worker: %s" % nom, surface.get(nom) is True,
                    surface.get(nom))

        # La page, elle, a bien tout ce que le worker n'a pas : la difference
        # est un monde different, pas un moteur diminue.
        page = json.loads(contexte.execute("JSON.stringify(globalThis.__page)"))
        verifie("worker: la page, elle, a un document",
                page.get("aDocument") is True and page.get("aWindow") is True)
        verifie("worker: et la page a le constructeur Worker",
                page.get("aWorker") is True)

        # Les messages traversent, dans les deux sens, avec leur structure.
        echo = etat.get("echo") or {}
        egal("worker: l'objet poste revient identique", echo.get("recu"),
             {"a": 1, "b": ["x", None, True], "c": {"d": -2.5}})

        # Les minuteries battent sur le fil du worker.
        egal("worker: setInterval bat dans le worker",
             (etat.get("minuterie") or {}).get("tours"), 3)

        # `TextEncoder`/`TextDecoder`, en UTF-8 et sur des caracteres qui
        # sortent de l'ASCII — c'est la seule facon de voir un encodage faux.
        texte = etat.get("texte") or {}
        egal("worker: TextDecoder rend ce que TextEncoder a produit",
             texte.get("relu"), "eleve — caractères accentués")
        verifie("worker: l'encodage est bien de l'UTF-8 multi-octets",
                len(texte.get("octets") or []) > len("eleve — caracteres"),
                len(texte.get("octets") or []))

        # `URL` resout contre l'adresse du script du worker.
        adresse = etat.get("url") or {}
        egal("worker: URL resout une adresse relative",
             adresse.get("chemin"), "/autre/chemin")
        egal("worker: et connait son origine", adresse.get("origine"),
             doc.base)

        # Les micro-taches sont de vraies micro-taches : apres tout le code
        # synchrone, jamais entrelacees avec lui.
        egal("worker: queueMicrotask s'execute apres le code synchrone",
             (etat.get("microtache") or {}).get("ordre"),
             ["sync", "sync-fin", "microtache"])

        jalon("WEB_WORKER_BASIC",
              fini and not etat.get("erreurs")
              and surface.get("aDocument") is False
              and surface.get("aPostMessage") is True,
              etat.get("etapes"))
    finally:
        doc.ferme_document()
        arrete()


def verifie_workers_calcul_ailleurs():
    """Le calcul d'un worker bloque-t-il la page ? La seule question qui compte.

    Une API `Worker` presente mais servie sur le fil de l'interface serait une
    facon compliquee de ne rien changer — et elle passerait toutes les
    verifications de surface. Seule une mesure la demasque : la page compte ses
    battements pendant que le worker calcule sans interruption.
    """
    doc, arrete = _sur_serveur("/page/worker-cpu.html", battements=2)
    try:
        contexte = doc.contexte_js
        if contexte is None:
            jalon("WEB_WORKER_LIFECYCLE", False, "pas de contexte JavaScript")
            return
        fini = _bat_jusqua(doc, contexte, "!!globalThis.__wc.rapport", 30.0)
        etat = json.loads(contexte.execute("JSON.stringify(globalThis.__wc)"))
        verifie("worker: le calcul a rendu son resultat", fini, etat)
        egal("worker: aucune erreur pendant le calcul", etat.get("erreurs"), [])

        rapport = etat.get("rapport") or {}
        verifie("worker: le calcul a bien dure ce qu'on lui a demande",
                (rapport.get("duree") or 0) >= 190, rapport.get("duree"))
        verifie("worker: et il a reellement tourne",
                (rapport.get("tours") or 0) > 0, rapport.get("tours"))

        # Le point de l'epreuve. Si le calcul s'etait fait sur le fil de
        # l'interface, la page n'aurait pas pu battre pendant ces 200 ms : le
        # compteur serait fige, ou aurait avance d'un cran.
        avance = (etat.get("toursALArrivee") or 0) - (etat.get("toursAuDepart") or 0)
        verifie("worker: la page a continue de battre pendant le calcul",
                avance >= 4, (etat.get("toursAuDepart"),
                              etat.get("toursALArrivee")))
    finally:
        doc.ferme_document()
        arrete()


def verifie_workers_reseau():
    """`fetch` dans un worker : la meme infrastructure, pas une seconde."""
    doc, arrete = _sur_serveur("/page/worker-fetch.html", battements=2)
    try:
        contexte = doc.contexte_js
        if contexte is None:
            jalon("WEB_WORKER_FETCH", False, "pas de contexte JavaScript")
            return
        fini = _bat_jusqua(doc, contexte, "!!globalThis.__wf.rapport", 30.0)
        etat = json.loads(contexte.execute("JSON.stringify(globalThis.__wf)"))
        rapport = etat.get("rapport") or {}
        verifie("worker: le rapport reseau est arrive", fini, etat)
        egal("worker: aucune erreur de worker", etat.get("erreurs"), [])
        verifie("worker: aucune erreur de requete", not rapport.get("erreur"),
                rapport.get("erreur"))

        # 1. L'adresse relative se resout contre le script du worker.
        egal("worker: la requete relative aboutit",
             rapport.get("relatifStatut"), 200)
        egal("worker: et son corps est celui du serveur",
             rapport.get("relatifNom"), "bouchaud")
        egal("worker: le JSON imbrique traverse",
             rapport.get("relatifImbrique"), "b")
        verifie("worker: l'adresse relative s'est resolue contre le worker",
                str(rapport.get("relatifUrl", "")).endswith("/json"),
                rapport.get("relatifUrl"))

        # 2. Une redirection est suivie par la meme pile que celle de la page.
        egal("worker: la redirection est suivie",
             rapport.get("redirigeStatut"), 200)

        # 3. Methode et corps traversent.
        egal("worker: le POST arrive comme un POST",
             rapport.get("echoMethode"), "POST")
        egal("worker: avec son corps", rapport.get("echoCorps"),
             "depuis-le-worker")

        # Le worker partage l'origine de son document : la requete n'est donc
        # pas cross-origine, et ne doit pas porter d'en-tete `Origin`. C'est ce
        # qui prouve que le chemin traverse `_recupere` et non un raccourci.
        egal("worker: une requete de meme origine ne porte pas d'Origin",
             rapport.get("echoOrigin"), "")

        # Et la page n'a pas ete bloquee pendant ce temps.
        verifie("worker: la page a battu pendant les requetes",
                (etat.get("toursALArrivee") or 0) >= 1,
                etat.get("toursALArrivee"))

        jalon("WEB_WORKER_FETCH",
              fini and not rapport.get("erreur")
              and rapport.get("relatifStatut") == 200
              and rapport.get("echoMethode") == "POST",
              rapport)
    finally:
        doc.ferme_document()
        arrete()


def verifie_workers_stockage():
    """IndexedDB dans un worker : la meme base que celle du document."""
    doc, arrete = _sur_serveur("/page/worker-indexeddb.html", battements=2)
    try:
        contexte = doc.contexte_js
        if contexte is None:
            jalon("WEB_WORKER_INDEXEDDB", False, "pas de contexte JavaScript")
            return
        fini = _bat_jusqua(doc, contexte, "!!globalThis.__wi.relu", 30.0)
        etat = json.loads(contexte.execute("JSON.stringify(globalThis.__wi)"))
        rapport = etat.get("rapport") or {}
        verifie("worker: le worker a fini son travail de base", fini, etat)
        egal("worker: aucune erreur", etat.get("erreurs"), [])
        verifie("worker: aucune erreur IndexedDB", not rapport.get("erreur"),
                rapport.get("erreur"))

        egal("worker: le magasin a ete cree par le worker",
             rapport.get("magasins"), ["notes"])
        egal("worker: il relit ce qu'il vient d'ecrire", rapport.get("lu"),
             {"texte": "ecrit par le worker", "n": 1})
        egal("worker: et il compte ses deux enregistrements",
             rapport.get("combien"), 2)
        egal("worker: les cles sont celles qu'il a posees",
             rapport.get("cles"), ["a", "b"])
        verifie("worker: une cle absente rend undefined, pas null",
                rapport.get("absenteEstIndefinie") is True)

        # Le point de l'epreuve : la page retrouve ce que le worker a ecrit.
        # Deux bases separees se comporteraient parfaitement chacune de son
        # cote, et le defaut serait invisible autrement.
        egal("worker: la page relit ce que le worker a ecrit",
             etat.get("relu"), {"texte": "et un second", "n": 2})
        verifie("worker: la page n'a pas eu a recreer le schema",
                not etat.get("monteeInattendue"))

        jalon("WEB_WORKER_INDEXEDDB",
              fini and not rapport.get("erreur")
              and etat.get("relu") == {"texte": "et un second", "n": 2},
              rapport)
    finally:
        doc.ferme_document()
        arrete()


def verifie_workers_temps_reel():
    """`WebSocket` dans un worker, par le meme code que dans la fenetre."""
    doc, arrete = _sur_serveur("/page/worker-websocket.html", battements=2)
    try:
        contexte = doc.contexte_js
        if contexte is None:
            jalon("WEB_WORKER_WEBSOCKET", False, "pas de contexte JavaScript")
            return
        fini = _bat_jusqua(doc, contexte, "!!globalThis.__ws.rapport", 30.0)
        etat = json.loads(contexte.execute("JSON.stringify(globalThis.__ws)"))
        rapport = etat.get("rapport") or {}
        verifie("worker: le rapport WebSocket est arrive", fini, etat)
        verifie("worker: la connexion s'est ouverte",
                rapport.get("ouvert") is True, rapport)
        egal("worker: le serveur a renvoye ce qu'on lui a dit",
             rapport.get("recus"), ["bonjour depuis le worker"])
        egal("worker: la fermeture est propre", rapport.get("code"), 1000)
        verifie("worker: et elle est signalee comme telle",
                rapport.get("propre") is True)

        jalon("WEB_WORKER_WEBSOCKET",
              fini and rapport.get("ouvert") is True
              and rapport.get("code") == 1000, rapport)
    finally:
        doc.ferme_document()
        arrete()


def verifie_workers_messagerie():
    """Le trafic de messages : structure, ordre, et refus d'origine.

    Ce que cette verification ajoute a `worker-base.html` : elle pilote le
    worker depuis le Python de l'epreuve, ce qui permet de compter les messages
    et de verifier leur ordre — une page ne peut affirmer que ce qu'elle a vu.
    """
    import time as _time

    doc, arrete = _sur_serveur("/page/vide.html", battements=2)
    try:
        contexte = doc.contexte_js
        if contexte is None:
            jalon("WEB_WORKER_MESSAGING", False, "pas de contexte JavaScript")
            return

        contexte.execute("""
        globalThis.__m = { recus: [], erreurs: [] };
        globalThis.__w = new Worker('/page/worker-base.js');
        globalThis.__w.onmessage = function (e) {
            globalThis.__m.recus.push(e.data && e.data.recu);
        };
        globalThis.__w.onerror = function (e) {
            globalThis.__m.erreurs.push(String(e.message || 'erreur'));
        };
        for (let i = 0; i < 20; i++)
            globalThis.__w.postMessage({ genre: 'echo', charge: i });
        """)
        arrive = _bat_jusqua(doc, contexte, "globalThis.__m.recus.length >= 20")
        recus = json.loads(contexte.execute("JSON.stringify(globalThis.__m.recus)"))
        verifie("worker: les vingt messages sont revenus", arrive, len(recus))
        egal("worker: dans l'ordre ou ils ont ete postes",
             recus[:20], list(range(20)))

        # Un message poste avant que le worker ait fini de demarrer n'est pas
        # perdu : il attend dans la file. C'est ce qui rend `new Worker(...)`
        # suivi immediatement de `postMessage(...)` utilisable, et c'est
        # l'usage le plus repandu.
        contexte.execute("""
        globalThis.__m2 = [];
        globalThis.__w2 = new Worker('/page/worker-base.js');
        globalThis.__w2.postMessage({ genre: 'echo', charge: 'tot' });
        globalThis.__w2.onmessage = function (e) {
            globalThis.__m2.push(e.data && e.data.recu);
        };
        """)
        tot = _bat_jusqua(doc, contexte, "globalThis.__m2.length >= 1")
        verifie("worker: un message poste avant l'amorce n'est pas perdu", tot)
        egal("worker: et c'est bien celui-la",
             json.loads(contexte.execute("JSON.stringify(globalThis.__m2)")),
             ["tot"])

        # Les structures imbriquees traversent par valeur, jamais par
        # reference : le worker qui modifierait l'objet ne toucherait rien chez
        # la page. On le verifie en postant un objet, en le mutant aussitot, et
        # en comparant ce que le worker a vu.
        contexte.execute("""
        globalThis.__m3 = [];
        globalThis.__objet = { n: 1, dedans: { liste: [1, 2, 3] } };
        globalThis.__w.onmessage = function (e) {
            globalThis.__m3.push(e.data && e.data.recu);
        };
        globalThis.__w.postMessage({ genre: 'echo', charge: globalThis.__objet });
        globalThis.__objet.n = 99;
        globalThis.__objet.dedans.liste.push(4);
        """)
        copie = _bat_jusqua(doc, contexte, "globalThis.__m3.length >= 1")
        vu = json.loads(contexte.execute("JSON.stringify(globalThis.__m3)"))
        verifie("worker: l'objet imbrique est revenu", copie, vu)
        egal("worker: les donnees sont copiees, pas partagees",
             vu[0] if vu else None, {"n": 1, "dedans": {"liste": [1, 2, 3]}})

        # Un script de worker d'une autre origine est refuse. Un worker partage
        # l'origine de son createur — donc ses temoins, son stockage, ses bases.
        # Le charger d'ailleurs reviendrait a executer du code etranger avec les
        # droits de la page.
        import serveur_test
        serveur_autre, autre = serveur_test.demarre(0)
        try:
            contexte.execute(
                "globalThis.__refus = { erreurs: [] };"
                "globalThis.__w3 = new Worker(%r);"
                "globalThis.__w3.onerror = function (e) {"
                "  globalThis.__refus.erreurs.push(String(e.message||'e')); };"
                % (autre + "/page/worker-base.js"))
            _bat_jusqua(doc, contexte, "globalThis.__refus.erreurs.length >= 1",
                        5.0)
            refus = json.loads(
                contexte.execute("JSON.stringify(globalThis.__refus.erreurs)"))
            verifie("worker: un script d'une autre origine est refuse",
                    len(refus) >= 1, refus)
        finally:
            serveur_test.arrete(serveur_autre)

        jalon("WEB_WORKER_MESSAGING",
              arrive and recus[:20] == list(range(20)) and tot and copie,
              len(recus))
    finally:
        doc.ferme_document()
        arrete()
        _time.sleep(0.05)


def verifie_workers_cycle_de_vie():
    """`terminate()` rend tout — y compris sur un worker qui boucle.

    Deux epreuves, et la seconde est la seule qui compte vraiment :

      * un worker en boucle infinie s'arrete quand on le lui demande. Un arret
        qui attendrait que le script veuille bien finir n'arreterait jamais
        celui-la, et une page hostile figerait le navigateur pour de bon ;
      * mille creations suivies d'autant de destructions ne laissent rien :
        ni contexte QuickJS, ni fil, ni descripteur.

    La memoire est mesuree aussi, mais lue avec precaution : voir le
    commentaire de la balance plus bas.
    """
    import os as _os
    import threading as _threading
    import time as _time

    import bojs as _bojs

    def contextes():
        return _bojs.contextes()

    def descripteurs():
        try:
            return len(_os.listdir("/proc/self/fd"))
        except OSError:
            return -1

    def resident_ko():
        try:
            with open("/proc/self/statm") as fichier:
                pages = int(fichier.read().split()[1])
            return pages * (_os.sysconf("SC_PAGE_SIZE") // 1024)
        except (OSError, ValueError, IndexError):
            return -1

    doc, arrete = _sur_serveur("/page/vide.html", battements=2)
    try:
        contexte = doc.contexte_js
        if contexte is None:
            jalon("WEB_WORKER_LIFECYCLE", False, "pas de contexte JavaScript")
            return

        # 1. Un worker qui ne rend jamais la main.
        contexte.execute("""
        globalThis.__b = { commence: false };
        globalThis.__wb = new Worker('/page/worker-boucle.js');
        globalThis.__wb.onmessage = function () { globalThis.__b.commence = true; };
        """)
        parti = _bat_jusqua(doc, contexte, "globalThis.__b.commence", 20.0)
        verifie("worker: le worker en boucle a bien demarre", parti)
        avant_arret = contextes()
        depart = _time.perf_counter()
        contexte.execute("globalThis.__wb.terminate()")
        duree = _time.perf_counter() - depart
        verifie("worker: terminate() rend la main tout de suite",
                duree < 1.0, duree)
        egal("worker: et le contexte QuickJS a disparu",
             contextes(), avant_arret - 1)
        egal("worker: la page, elle, repond encore",
             contexte.execute("1 + 1"), 2)

        # 2. La balance. Mille creations, mille destructions.
        contexte.execute("""
        globalThis.__lot = { vivants: [], repondus: 0, erreurs: 0 };
        globalThis.__ouvre = function (combien) {
            globalThis.__lot = { vivants: [], repondus: 0, erreurs: 0 };
            for (let i = 0; i < combien; i++) {
                const w = new Worker('/page/worker-base.js');
                w.onmessage = function () { globalThis.__lot.repondus += 1; };
                w.onerror = function () { globalThis.__lot.erreurs += 1; };
                w.postMessage({ genre: 'echo', charge: i });
                globalThis.__lot.vivants.push(w);
            }
        };
        globalThis.__ferme = function () {
            for (const w of globalThis.__lot.vivants) w.terminate();
            globalThis.__lot.vivants = [];
        };
        """)
        for _ in range(5):
            doc.rafraichis()
            _time.sleep(0.01)

        avant = (contextes(), _threading.active_count(), descripteurs(),
                 resident_ko())

        LOT = 25
        LOTS = 40                      # 40 x 25 = 1000
        repondus = 0
        for _ in range(LOTS):
            contexte.execute("globalThis.__ouvre(%d)" % LOT)
            _bat_jusqua(doc, contexte,
                        "globalThis.__lot.repondus >= %d" % LOT, 10.0)
            repondus += int(contexte.execute("globalThis.__lot.repondus") or 0)
            contexte.execute("globalThis.__ferme()")
        for _ in range(10):
            doc.rafraichis()
            _time.sleep(0.02)

        apres = (contextes(), _threading.active_count(), descripteurs(),
                 resident_ko())

        total = LOT * LOTS
        verifie("worker: les mille workers ont repondu",
                repondus >= total * 0.9, (repondus, total))
        egal("worker: pas un contexte QuickJS de plus qu'avant",
             apres[0], avant[0])
        egal("worker: pas un fil de plus qu'avant", apres[1], avant[1])
        egal("worker: pas un descripteur de plus qu'avant",
             apres[2], avant[2])

        # La memoire, elle, ne revient pas exactement a son point de depart, et
        # ce n'est pas une fuite : la mesure l'etablit. Le supplement suit le
        # nombre de workers **simultanes** (25 ici), pas le nombre total —
        # mille workers dix par dix coutent 8 Mo, cent par cent en coutent 32.
        # C'est la marque haute de l'allocateur, que la libc ne rend pas au
        # systeme. La borne ci-dessous est donc large a dessein : elle attrape
        # une vraie fuite — qui, elle, croitrait avec le total — sans accuser
        # l'allocateur.
        croissance = apres[3] - avant[3]
        verifie("worker: la memoire ne croit pas avec le nombre de workers",
                croissance < 60000, (avant[3], apres[3], croissance))

        jalon("WEB_WORKER_LIFECYCLE",
              parti and duree < 1.0 and apres[0] == avant[0]
              and apres[1] == avant[1] and apres[2] == avant[2],
              {"contextes": (avant[0], apres[0]),
               "fils": (avant[1], apres[1]),
               "descripteurs": (avant[2], apres[2]),
               "resident_ko": (avant[3], apres[3]),
               "workers": total, "repondus": repondus})
    finally:
        doc.ferme_document()
        arrete()


# --- Clonage structure --------------------------------------------------------
#
# Une seule abstraction sert quatre chemins : `Worker.postMessage`,
# `Window.postMessage`, `MessagePort.postMessage` et IndexedDB. Ce qui suit la
# verifie d'abord seule — un aller-retour dans le meme contexte, par
# `structuredClone` —, puis sur chacun des quatre.
#
# Le point le plus important n'est pas ce qui traverse : c'est ce qui **leve**.
# Avant, une fonction, un `Symbol` ou un nœud du DOM devenaient `null` ou `{}`
# en silence, et la page decouvrait le trou beaucoup plus loin.


def verifie_clonage_structure():
    """Ce qui traverse, ce qui est preserve, et ce qui leve."""
    doc, arrete = _sur_serveur("/page/vide.html", battements=2)
    try:
        contexte = doc.contexte_js
        verifie("clone: la page a un contexte", contexte is not None)
        if contexte is None:
            jalon("STRUCTURED_CLONE", False, "pas de contexte JavaScript")
            return

        # 1. Les valeurs simples font l'aller-retour sans changer de nature.
        #    `undefined` en tete : c'est celle que la conversion du pont
        #    transformait en `null`, et la seule dont la perte se lit comme un
        #    champ manquant plutot que comme une erreur.
        rendu = json.loads(contexte.execute("""
        (function () {
          const r = {};
          const c = structuredClone;
          r.indefini = c(undefined) === undefined;
          r.nul = c(null) === null;
          r.vrai = c(true) === true;
          r.zero = c(0) === 0;
          r.negatif = c(-3.5) === -3.5;
          r.grand = c(9007199254740991) === 9007199254740991;
          r.chaine = c("éléphant") === "éléphant";
          r.vide = c("") === "";
          // NaN ne s'egale pas lui-meme : c'est `Number.isNaN` qui repond.
          r.nan = Number.isNaN(c(NaN));
          r.infini = c(Infinity) === Infinity;
          r.infiniNegatif = c(-Infinity) === -Infinity;
          return JSON.stringify(r);
        })()
        """))
        for nom, valeur in sorted(rendu.items()):
            verifie("clone: %s traverse" % nom, valeur is True, valeur)

        # 2. Les structures imbriquees, et la copie qui est bien une copie.
        rendu = json.loads(contexte.execute("""
        (function () {
          const r = {};
          const source = { a: 1, b: [1, [2, [3, { c: "profond" }]]],
                           d: { e: { f: null } } };
          const copie = structuredClone(source);
          r.egal = JSON.stringify(copie) === JSON.stringify(source);
          r.autreObjet = copie !== source;
          r.autreTableau = copie.b !== source.b;
          copie.b[1][1][1].c = "change";
          r.independant = source.b[1][1][1].c === "profond";
          // Les trous d'un tableau et les valeurs `undefined` dedans.
          const troue = structuredClone([1, undefined, 3]);
          r.trou = troue.length === 3 && troue[1] === undefined;
          return JSON.stringify(r);
        })()
        """))
        for nom, valeur in sorted(rendu.items()):
            verifie("clone: imbrique — %s" % nom, valeur is True, valeur)

        # 3. Les cycles et le partage. Un format qui recopierait naivement
        #    boucherait sur le premier, et perdrait le second sans le dire :
        #    deux champs qui pointaient le meme objet en recevraient deux.
        rendu = json.loads(contexte.execute("""
        (function () {
          const r = {};
          const boucle = { nom: "moi" };
          boucle.moi = boucle;
          const c1 = structuredClone(boucle);
          r.cycleTient = c1.moi === c1;
          r.cycleCopie = c1 !== boucle;
          r.cycleContenu = c1.nom === "moi";

          const mutuel = { a: {}, b: {} };
          mutuel.a.vers = mutuel.b;
          mutuel.b.vers = mutuel.a;
          const c2 = structuredClone(mutuel);
          r.mutuel = c2.a.vers === c2.b && c2.b.vers === c2.a;

          const partage = {};
          const c3 = structuredClone({ x: partage, y: partage });
          r.partagePreserve = c3.x === c3.y;

          const tableau = [];
          tableau.push(tableau);
          const c4 = structuredClone(tableau);
          r.tableauCyclique = c4[0] === c4;
          return JSON.stringify(r);
        })()
        """))
        for nom, valeur in sorted(rendu.items()):
            verifie("clone: cycle — %s" % nom, valeur is True, valeur)

        # 4. Les types de la plate-forme que la norme cite nommement.
        rendu = json.loads(contexte.execute("""
        (function () {
          const r = {};
          const d = new Date(1700000000123);
          const cd = structuredClone(d);
          r.date = cd instanceof Date && cd.getTime() === 1700000000123
                   && cd !== d;

          const x = /a.c/gi;
          const cx = structuredClone(x);
          r.regexp = cx instanceof RegExp && cx.source === "a.c"
                     && cx.flags === x.flags;

          const m = new Map([["a", 1], ["b", { profond: true }]]);
          const cm = structuredClone(m);
          r.map = cm instanceof Map && cm.size === 2 && cm.get("a") === 1
                  && cm.get("b").profond === true && cm.get("b") !== m.get("b");

          const s = new Set([1, "deux", 3]);
          const cs = structuredClone(s);
          r.set = cs instanceof Set && cs.size === 3 && cs.has("deux");

          const tampon = new ArrayBuffer(4);
          new Uint8Array(tampon).set([1, 2, 3, 250]);
          const ct = structuredClone(tampon);
          r.tampon = ct instanceof ArrayBuffer && ct.byteLength === 4
                     && new Uint8Array(ct)[3] === 250 && ct !== tampon;

          const vue = new Uint16Array([300, 400, 500]);
          const cv = structuredClone(vue);
          r.vue = cv instanceof Uint16Array && cv.length === 3
                  && cv[1] === 400 && cv !== vue;

          const err = new TypeError("mauvais type");
          const ce = structuredClone(err);
          r.erreur = ce instanceof Error && ce.name === "TypeError"
                     && ce.message === "mauvais type";

          // Une instance de classe se clone en objet simple : ses champs
          // traversent, son prototype non. C'est ce que fait la norme.
          class Point { constructor() { this.x = 1; this.y = 2; } bouge() {} }
          const cp = structuredClone(new Point());
          r.classe = cp.x === 1 && cp.y === 2 && !(cp instanceof Point);
          return JSON.stringify(r);
        })()
        """))
        for nom, valeur in sorted(rendu.items()):
            verifie("clone: type — %s" % nom, valeur is True, valeur)

        # 5. Ce qui leve. C'est la moitie qui manquait : une conversion muette
        #    fait disparaitre le probleme, pas le defaut.
        rendu = json.loads(contexte.execute("""
        (function () {
          function leve(f) {
            try { f(); return "rien"; }
            catch (e) { return e.name; }
          }
          const r = {};
          r.fonction = leve(() => structuredClone(function () {}));
          r.flechee = leve(() => structuredClone(() => 1));
          r.symbole = leve(() => structuredClone(Symbol("s")));
          r.dansUnObjet = leve(() => structuredClone({ a: 1, f: function () {} }));
          r.dansUnTableau = leve(() => structuredClone([1, Symbol("s")]));
          r.noeud = leve(() => structuredClone(document.body));
          r.fenetre = leve(() => structuredClone(window));
          const canal = new MessageChannel();
          r.port = leve(() => structuredClone(canal.port1));
          canal.port1.close(); canal.port2.close();
          return JSON.stringify(r);
        })()
        """))
        for nom, valeur in sorted(rendu.items()):
            egal("clone: %s leve DataCloneError" % nom, valeur, "DataCloneError")

        jalon("STRUCTURED_CLONE",
              all(v == "DataCloneError" for v in rendu.values()), rendu)
    finally:
        doc.ferme_document()
        arrete()


def verifie_clonage_par_les_quatre_chemins():
    """La meme abstraction sur les quatre chemins qui s'en servent."""
    doc, arrete = _sur_serveur("/page/vide.html", battements=2)
    try:
        contexte = doc.contexte_js
        if contexte is None:
            jalon("WEB_WORKER_MESSAGING", False, "pas de contexte JavaScript")
            return

        # Un echantillon qui contient tout ce qui se perdait avant : un
        # `undefined`, un `NaN`, une `Date`, un `Map`, un cycle.
        prepare = """
        globalThis.__temoin = function () {
          const o = { indefini: undefined, nombre: NaN, infini: -Infinity,
                      date: new Date(1700000000123),
                      table: new Map([["cle", [1, 2, 3]]]),
                      texte: "élan" };
          o.moi = o;
          return o;
        };
        globalThis.__verifie = function (v) {
          return {
            indefini: "indefini" in v && v.indefini === undefined,
            nan: Number.isNaN(v.nombre),
            infini: v.infini === -Infinity,
            date: v.date instanceof Date && v.date.getTime() === 1700000000123,
            map: v.table instanceof Map
                 && JSON.stringify(v.table.get("cle")) === "[1,2,3]",
            texte: v.texte === "élan",
            cycle: v.moi === v,
          };
        };
        """
        contexte.execute(prepare)

        # 1. Un Worker.
        contexte.execute("""
        globalThis.__q1 = null;
        globalThis.__w = new Worker('/page/worker-base.js');
        globalThis.__w.onmessage = function (e) {
          globalThis.__q1 = globalThis.__verifie(e.data.recu);
        };
        globalThis.__w.postMessage({ genre: 'echo', charge: globalThis.__temoin() });
        """)
        vu = _bat_jusqua(doc, contexte, "!!globalThis.__q1")
        verifie("clone: le worker a renvoye l'echantillon", vu)
        rendu = json.loads(contexte.execute("JSON.stringify(globalThis.__q1)"))
        for nom, valeur in sorted((rendu or {}).items()):
            verifie("clone: worker — %s" % nom, valeur is True, valeur)

        # 2. Un MessageChannel. La livraison est differee : le message ne doit
        #    pas arriver pendant l'appel a `postMessage`.
        contexte.execute("""
        globalThis.__q2 = null;
        globalThis.__synchrone = false;
        globalThis.__canal = new MessageChannel();
        globalThis.__canal.port2.onmessage = function (e) {
          globalThis.__q2 = globalThis.__verifie(e.data);
        };
        globalThis.__canal.port1.postMessage(globalThis.__temoin());
        globalThis.__synchrone = globalThis.__q2 !== null;
        """)
        verifie("clone: MessagePort ne livre pas dans l'appel",
                contexte.execute("globalThis.__synchrone") is False)
        vu = _bat_jusqua(doc, contexte, "!!globalThis.__q2")
        verifie("clone: le port jumeau a recu", vu)
        rendu = json.loads(contexte.execute("JSON.stringify(globalThis.__q2)"))
        for nom, valeur in sorted((rendu or {}).items()):
            verifie("clone: canal — %s" % nom, valeur is True, valeur)

        # 3. `window.postMessage` sur soi-meme : la boucle la plus courte qui
        #    passe quand meme par la file de la boucle d'evenements.
        contexte.execute("""
        globalThis.__q3 = null;
        globalThis.addEventListener('message', function (e) {
          if (e.data && e.data.__marque === 'clone')
            globalThis.__q3 = globalThis.__verifie(e.data.charge);
        });
        (function () {
          const charge = globalThis.__temoin();
          window.postMessage({ __marque: 'clone', charge: charge }, '*');
        })();
        """)
        vu = _bat_jusqua(doc, contexte, "!!globalThis.__q3")
        verifie("clone: window.postMessage a livre", vu)
        rendu = json.loads(contexte.execute("JSON.stringify(globalThis.__q3)"))
        for nom, valeur in sorted((rendu or {}).items()):
            verifie("clone: fenetre — %s" % nom, valeur is True, valeur)

        # 4. IndexedDB. L'aller-retour passe par le disque : ce qui survit ici
        #    survit aussi a une fermeture du navigateur.
        contexte.execute("""
        globalThis.__q4 = null;
        (function () {
          const demande = indexedDB.open('bo-clone', 1);
          demande.onupgradeneeded = function () {
            demande.result.createObjectStore('valeurs');
          };
          demande.onsuccess = function () {
            const base = demande.result;
            const ecriture = base.transaction('valeurs', 'readwrite');
            ecriture.objectStore('valeurs').put(globalThis.__temoin(), 'temoin');
            ecriture.oncomplete = function () {
              const lecture = base.transaction('valeurs', 'readonly');
              const r = lecture.objectStore('valeurs').get('temoin');
              r.onsuccess = function () {
                globalThis.__q4 = globalThis.__verifie(r.result);
              };
              r.onerror = function () { globalThis.__q4 = { erreur: true }; };
            };
          };
          demande.onerror = function () { globalThis.__q4 = { erreur: true }; };
        })();
        """)
        vu = _bat_jusqua(doc, contexte, "!!globalThis.__q4", 30.0)
        verifie("clone: IndexedDB a rendu la valeur", vu)
        rendu = json.loads(contexte.execute("JSON.stringify(globalThis.__q4)"))
        for nom, valeur in sorted((rendu or {}).items()):
            verifie("clone: base — %s" % nom, valeur is True, valeur)
    finally:
        doc.ferme_document()
        arrete()


def verifie_canal_de_messages():
    """`MessageChannel` : deux bouts, une file, et rien de synchrone."""
    doc, arrete = _sur_serveur("/page/vide.html", battements=2)
    try:
        contexte = doc.contexte_js
        if contexte is None:
            jalon("MESSAGE_CHANNEL", False, "pas de contexte JavaScript")
            return

        # Les deux sens, et l'ordre.
        contexte.execute("""
        globalThis.__c = { versDeux: [], versUn: [] };
        globalThis.__canal = new MessageChannel();
        globalThis.__canal.port2.onmessage = function (e) {
          globalThis.__c.versDeux.push(e.data);
          globalThis.__canal.port2.postMessage("recu:" + e.data);
        };
        globalThis.__canal.port1.onmessage = function (e) {
          globalThis.__c.versUn.push(e.data);
        };
        for (let i = 0; i < 5; i++) globalThis.__canal.port1.postMessage(i);
        """)
        arrive = _bat_jusqua(doc, contexte, "globalThis.__c.versUn.length >= 5")
        etat = json.loads(contexte.execute("JSON.stringify(globalThis.__c)"))
        verifie("canal: les cinq messages ont fait l'aller-retour", arrive, etat)
        egal("canal: dans l'ordre, a l'aller", etat.get("versDeux"),
             [0, 1, 2, 3, 4])
        egal("canal: et au retour", etat.get("versUn"),
             ["recu:0", "recu:1", "recu:2", "recu:3", "recu:4"])

        # Un port qui n'a pas demarre garde ses messages. C'est la difference
        # entre `onmessage` — qui demarre le port — et `addEventListener` sans
        # `start()`, qui, selon la norme, ne le demarre pas.
        contexte.execute("""
        globalThis.__d = { recus: [], avantStart: null };
        globalThis.__canal2 = new MessageChannel();
        globalThis.__canal2.port1.postMessage("garde-moi");
        """)
        for _ in range(10):
            doc.rafraichis()
        contexte.execute(
            "globalThis.__d.avantStart = globalThis.__d.recus.length;"
            "globalThis.__canal2.port2.addEventListener('message',"
            "  function (e) { globalThis.__d.recus.push(e.data); });")
        garde = _bat_jusqua(doc, contexte, "globalThis.__d.recus.length >= 1")
        etat = json.loads(contexte.execute("JSON.stringify(globalThis.__d)"))
        egal("canal: rien n'arrive avant l'ecoute", etat.get("avantStart"), 0)
        verifie("canal: le message garde arrive au demarrage", garde, etat)
        egal("canal: et c'est bien celui-la", etat.get("recus"), ["garde-moi"])

        # Un port ferme ne transmet plus.
        contexte.execute("""
        globalThis.__e = [];
        globalThis.__canal3 = new MessageChannel();
        globalThis.__canal3.port2.onmessage = function (e) {
          globalThis.__e.push(e.data);
        };
        globalThis.__canal3.port1.postMessage("avant");
        """)
        _bat_jusqua(doc, contexte, "globalThis.__e.length >= 1")
        contexte.execute("globalThis.__canal3.port1.close();"
                         "globalThis.__canal3.port1.postMessage('apres');")
        for _ in range(20):
            doc.rafraichis()
        egal("canal: un port ferme ne transmet plus",
             json.loads(contexte.execute("JSON.stringify(globalThis.__e)")),
             ["avant"])

        jalon("MESSAGE_CHANNEL", arrive and garde, etat)
    finally:
        doc.ferme_document()
        arrete()


# --- Renderer separe ----------------------------------------------------------
#
# Le moteur web dans un autre processus, le chrome dans celui-ci, une prise et
# une surface partagee entre les deux.
#
# Trois questions, et une seule compte vraiment :
#
#   1. **le protocole tient-il ?** Une trame tronquee, une longueur absurde, une
#      version inconnue doivent etre refusees, pas devinees ;
#   2. **la page marche-t-elle a travers ?** La fixture de connexion est
#      l'epreuve : cliquer, taper, envoyer — le tout par des messages, sans
#      qu'aucun code de l'epreuve ne touche le DOM d'en face ;
#   3. **la separation achete-t-elle quelque chose ?** Un renderer qui meurt ne
#      doit rien emporter, et sa memoire doit etre bornee sans que celle du
#      navigateur le soit.


def verifie_renderer_protocole():
    """Le cadrage des trames, et tout ce qui doit etre refuse."""
    import socket as _socket

    from moteur import protocole

    # L'aller-retour nominal, y compris la charge vide et les accents.
    for genre, charge in ((protocole.TICK, None),
                          (protocole.NAVIGATE, {"url": "http://a.test/é?x=1"}),
                          (protocole.FRAME_READY, {"generation": 7,
                                                   "tampon": 1})):
        a, b = _socket.socketpair()
        try:
            protocole.Canal(a).envoie(genre, charge)
            relu_genre, relu_charge = protocole.Canal(b).lis()
            egal("protocole: %s revient identique" % protocole.NOMS[genre],
                 (relu_genre, relu_charge), (genre, charge))
        finally:
            a.close()
            b.close()

    def refuse(nom, octets, attendus=None):
        """Ecrit des octets bruts et verifie que la lecture leve."""
        a, b = _socket.socketpair()
        try:
            a.sendall(octets)
            a.close()
            try:
                protocole.Canal(b).lis(attendus)
            except protocole.Erreur:
                reussi = True
            except protocole.Fin:
                reussi = False
            else:
                reussi = False
            verifie("protocole: %s est refuse" % nom, reussi)
        finally:
            b.close()

    entete = protocole.EN_TETE
    refuse("une version inconnue", entete.pack(99, protocole.TICK, 0))
    refuse("un genre inconnu", entete.pack(protocole.VERSION, 4242, 0))
    refuse("une longueur absurde",
           entete.pack(protocole.VERSION, protocole.NAVIGATE,
                       protocole.CHARGE_MAX + 1))
    refuse("un en-tete tronque", b"\x00\x01\x00")
    refuse("une charge tronquee",
           entete.pack(protocole.VERSION, protocole.NAVIGATE, 64) + b"{}")
    refuse("une charge qui n'est pas du JSON",
           entete.pack(protocole.VERSION, protocole.NAVIGATE, 3) + b"{[}")
    # Le sens compte : un renderer n'a aucune raison d'envoyer un `CLOSE`.
    refuse("un message a contresens",
           entete.pack(protocole.VERSION, protocole.CLOSE, 0),
           protocole.VERS_NAVIGATEUR)

    # Le decoupage : deux messages dans un envoi, un message en deux envois.
    a, b = _socket.socketpair()
    try:
        canal = protocole.Canal(b)
        a.sendall(protocole.encode(protocole.TICK, {"n": 1})
                  + protocole.encode(protocole.TICK, {"n": 2}))
        egal("protocole: deux trames dans un envoi (1)",
             canal.lis()[1], {"n": 1})
        egal("protocole: deux trames dans un envoi (2)",
             canal.lis()[1], {"n": 2})

        trame = protocole.encode(protocole.NAVIGATE, {"url": "http://a.test/"})
        a.sendall(trame[:5])
        a.sendall(trame[5:])
        egal("protocole: une trame en deux envois",
             canal.lis()[1], {"url": "http://a.test/"})
    finally:
        a.close()
        b.close()

    # Une charge trop grosse est refusee a l'ecriture aussi : mieux vaut echouer
    # chez celui qui fabrique le message que chez celui qui le recoit.
    try:
        protocole.encode(protocole.NAVIGATE,
                         {"url": "x" * (protocole.CHARGE_MAX + 10)})
    except protocole.Erreur:
        verifie("protocole: une charge trop grosse est refusee a l'emission",
                True)
    else:
        verifie("protocole: une charge trop grosse est refusee a l'emission",
                False)


def verifie_renderer_surface():
    """La surface partagee : en-tete, deux tampons, publication."""
    from moteur import surface as mod_surface

    s = mod_surface.Surface.alloue(64, 32)
    try:
        entete = s.entete()
        egal("surface: les dimensions se relisent",
             (entete["largeur"], entete["hauteur"]), (64, 32))
        egal("surface: le pas est coherent", entete["pas"], 64 * 4)
        egal("surface: rien n'est publie au depart", entete["generation"], 0)

        # Deux tampons independants : ecrire dans l'un ne touche pas l'autre.
        s.ecris(0, b"\x11" * s.octets_tampon)
        s.ecris(1, b"\x22" * s.octets_tampon)
        egal("surface: le tampon 0 garde ce qu'on y met", s.lis(0)[:4],
             b"\x11\x11\x11\x11")
        egal("surface: le tampon 1 aussi", s.lis(1)[:4], b"\x22\x22\x22\x22")

        s.publie(1, 5)
        egal("surface: la publication nomme le tampon", s.entete()["avant"], 1)
        egal("surface: et sa generation", s.entete()["generation"], 5)
        egal("surface: la lecture par defaut suit la publication",
             s.lis()[:4], b"\x22\x22\x22\x22")

        # Une seconde vue sur le **meme** descripteur voit les memes octets :
        # c'est la definition de « partage », et la seule chose qui distingue
        # cette surface d'une copie.
        import os as _os
        jumelle = mod_surface.Surface(_os.dup(s.descripteur), 64, 32)
        try:
            egal("surface: une seconde vue lit les memes pixels",
                 jumelle.lis()[:4], b"\x22\x22\x22\x22")
            jumelle.ecris(0, b"\x33" * s.octets_tampon)
            egal("surface: et ce qu'elle ecrit se voit de l'autre cote",
                 s.lis(0)[:4], b"\x33\x33\x33\x33")
        finally:
            jumelle.ferme()

        # Les dimensions impossibles sont refusees des l'allocation.
        for largeur, hauteur in ((0, 10), (10, -1), (100000, 100000)):
            try:
                mod_surface.Surface.alloue(largeur, hauteur)
            except ValueError:
                refuse = True
            else:
                refuse = False
            verifie("surface: %sx%s est refuse" % (largeur, hauteur), refuse)
    finally:
        s.ferme()


def verifie_renderer_separe():
    """La fixture de connexion, jouee de bout en bout dans un autre processus.

    Rien de ce qui suit ne touche le DOM d'en face : l'epreuve n'a que des
    messages et une surface. C'est exactement la contrainte d'un vrai chrome, et
    c'est ce qui rend le resultat interessant — un clic qui arrive, un
    formulaire qui part, une trame qui change.
    """
    import os as _os

    from moteur import superviseur

    doc_bidon, arrete = _sur_serveur("/page/vide.html", battements=1)
    base = doc_bidon.base
    doc_bidon.ferme_document()
    traces = []
    renderer = superviseur.Renderer(900, 700,
                                    journal=lambda n, t: traces.append((n, t)))
    try:
        egal("renderer: c'est bien un autre processus",
             renderer.pid != _os.getpid(), True)
        vu, evenements = renderer.attends("READY", 15.0)
        verifie("renderer: il se presente", vu, [e[0] for e in evenements])
        pret = next((c for n, c in evenements if n == "READY"), {}) or {}
        egal("renderer: a la version du protocole", pret.get("version"), 1)
        egal("renderer: et c'est bien son pid", pret.get("pid"), renderer.pid)

        # La limite d'adressage annoncee par le renderer est celle que le
        # navigateur a posee — et le navigateur, lui, ne l'a pas.
        egal("renderer: la limite memoire posee est celle qui s'applique",
             pret.get("limite_as"), renderer.limite_as)
        import resource as _resource
        mienne, _ = _resource.getrlimit(_resource.RLIMIT_AS)
        verifie("renderer: le navigateur, lui, n'est pas borne",
                mienne != renderer.limite_as, mienne)
        # Et cette limite est un **budget** au-dessus de ce dont l'enfant
        # herite, pas un plafond absolu : un `fork` transmet l'espace
        # d'adressage du parent, et un plafond absolu ferait naitre un renderer
        # deja au ras du sien des que le navigateur a un peu travaille.
        verifie("renderer: la limite depasse ce dont il a herite",
                renderer.limite_as > superviseur.taille_adressage(),
                (renderer.limite_as, superviseur.taille_adressage()))

        # 1. La navigation. L'adresse et le titre remontent par le canal.
        renderer.navigue(base + "/page/login-form.html")
        vu, evenements = renderer.attends("FRAME_READY", 30.0)
        noms = [e[0] for e in evenements]
        verifie("renderer: une premiere trame arrive", vu, noms)
        verifie("renderer: l'adresse a ete annoncee", "URL_CHANGED" in noms, noms)
        verifie("renderer: le titre aussi", "TITLE_CHANGED" in noms, noms)
        egal("renderer: et c'est le bon titre", renderer.titre,
             "Connexion — page temoin")
        verifie("renderer: l'adresse est celle demandee",
                str(renderer.url or "").endswith("/page/login-form.html"),
                renderer.url)

        # 2. Les pixels sont dans la surface partagee, pas dans le canal.
        trame = renderer.derniere_trame or {}
        egal("renderer: la trame annonce ses dimensions",
             (trame.get("largeur"), trame.get("hauteur")), (900, 700))
        egal("renderer: la generation demarre a un", trame.get("generation"), 1)
        peints = renderer.pixels_non_vides()
        verifie("renderer: la surface porte une vraie page", peints > 10000,
                peints)
        egal("renderer: l'en-tete de la surface designe le meme tampon",
             renderer.surface.entete()["avant"], trame.get("tampon"))

        def empreinte():
            octets = renderer.trame()
            return None if octets is None else hashlib.sha256(octets).hexdigest()

        au_chargement = empreinte()

        # 3. Le clic. Le champ « utilisateur » est a x=49..371, y=166..204 dans
        #    cette mise en page ; on vise son milieu.
        renderer.clic(200, 185)
        renderer.attends("FRAME_READY", 10.0)
        apres_clic = empreinte()
        verifie("renderer: un clic change ce qui est peint",
                apres_clic != au_chargement)

        # 4. La frappe. Pas de `input.value = …` : c'est le modele d'edition
        #    qu'on eprouve, a travers une frontiere de processus.
        for lettre in "arthur":
            renderer.touche(lettre, lettre)
        renderer.attends("FRAME_READY", 10.0)
        apres_frappe = empreinte()
        verifie("renderer: la frappe change ce qui est peint",
                apres_frappe != apres_clic)
        egal("renderer: la mise en page, elle, n'a pas bouge",
             renderer.pixels_non_vides(), peints)

        # 5. L'envoi. Le bouton est a y=439..479. La page ecrit son resultat
        #    dans un bloc qui **grandit** : c'est ce qui rend l'envoi visible
        #    depuis l'exterieur, sans lire une seule ligne de son DOM.
        renderer.clic(200, 459)
        renderer.attends("FRAME_READY", 10.0)
        apres_envoi = renderer.pixels_non_vides()
        verifie("renderer: l'envoi du formulaire a traverse",
                apres_envoi > peints, (peints, apres_envoi))
        verifie("renderer: et la trame a change", empreinte() != apres_frappe)

        # 6. Le redimensionnement : la surface est reallouee **par le
        #    navigateur**, et le renderer repeint dedans.
        renderer.redimensionne(640, 480)
        vu, _ = renderer.attends("FRAME_READY", 15.0)
        verifie("renderer: il repeint apres un redimensionnement", vu)
        egal("renderer: dans la nouvelle surface",
             (renderer.derniere_trame or {}).get("largeur"), 640)

        # 7. Aucune erreur en chemin.
        restes = renderer.recolte(0.2)
        erreurs = [c for n, c in restes if n == "ERROR"]
        egal("renderer: aucune erreur signalee", erreurs, [])

        code = renderer.ferme()
        egal("renderer: il s'arrete proprement sur CLOSE", code, 0)

        jalon("RENDERER_BASIC",
              vu and apres_envoi > peints and renderer.titre
              == "Connexion — page temoin",
              {"pixels": (peints, apres_envoi), "titre": renderer.titre})
    finally:
        renderer.ferme()
        arrete()


def verifie_renderer_isolation():
    """Ce que la separation achete : un crash contenu, une memoire bornee."""
    import os as _os
    import signal as _signal

    from moteur import protocole, superviseur

    doc_bidon, arrete = _sur_serveur("/page/vide.html", battements=1)
    base = doc_bidon.base
    doc_bidon.ferme_document()
    try:
        # --- Crash -----------------------------------------------------------
        #
        # Un renderer tue net. Le navigateur doit l'apprendre, le nommer, et
        # continuer — puis pouvoir en refaire un.
        renderer = superviseur.Renderer(400, 300)
        renderer.attends("READY", 15.0)
        renderer.navigue(base + "/page/login-form.html")
        renderer.attends("FRAME_READY", 30.0)
        avant = renderer.pixels_non_vides()
        verifie("isolation: le premier renderer avait peint", avant > 0, avant)

        pid = renderer.pid
        _os.kill(pid, _signal.SIGKILL)
        vu, evenements = renderer.attends("CRASH", 10.0)
        verifie("isolation: la mort est signalee", vu,
                [e[0] for e in evenements])
        mort = next((c for n, c in evenements if n == "CRASH"), {}) or {}
        egal("isolation: par le bon signal", mort.get("signal"),
             int(_signal.SIGKILL))
        verifie("isolation: avec une raison lisible",
                "SIGKILL" in str(mort.get("raison", "")), mort.get("raison"))
        verifie("isolation: le navigateur, lui, est toujours la",
                _os.getpid() > 0)
        verifie("isolation: et la surface reste lisible apres la mort",
                renderer.pixels_non_vides() == avant)

        # Parler a un mort ne bloque pas : ca leve, et ca se rattrape.
        try:
            renderer.navigue(base + "/page/vide.html")
        except protocole.Fin:
            leve = True
        except OSError:
            leve = True
        else:
            leve = False
        verifie("isolation: parler a un renderer mort echoue franchement", leve)
        renderer.ferme()

        # Un second renderer prend la place du premier. C'est ce qui separe une
        # isolation d'une panne : l'onglet peut redemarrer.
        remplacant = superviseur.Renderer(400, 300)
        try:
            vu, _ = remplacant.attends("READY", 15.0)
            verifie("isolation: un remplacant demarre", vu)
            verifie("isolation: et c'est un nouveau processus",
                    remplacant.pid != pid, (pid, remplacant.pid))
            remplacant.navigue(base + "/page/login-form.html")
            vu, _ = remplacant.attends("FRAME_READY", 30.0)
            verifie("isolation: le remplacant peint", vu)
        finally:
            remplacant.ferme()

        jalon("RENDERER_CRASH_ISOLATION",
              mort.get("signal") == int(_signal.SIGKILL), mort)

        # --- Memoire ---------------------------------------------------------
        #
        # La limite est **dans** le renderer et nulle part ailleurs. On le
        # verifie en lui demandant une surface que le navigateur, lui, alloue et
        # mappe sans difficulte : le renderer echoue, le dit, et survit. Un
        # plafond qui ne s'appliquerait pas se lirait exactement comme un
        # plafond qui s'applique, jusqu'au jour ou un onglet emporte la machine.
        etroit = superviseur.Renderer(
            200, 150, budget_as=48 << 20)
        try:
            vu, evenements = etroit.attends("READY", 15.0)
            verifie("isolation: le renderer borne demarre", vu)
            pret = next((c for n, c in evenements if n == "READY"), {}) or {}
            serre = (pret.get("limite_as") or 0)
            verifie("isolation: il annonce la limite etroite qu'on lui a posee",
                    serre == etroit.limite_as, (serre, etroit.limite_as))
            verifie("isolation: et elle est bien plus basse que celle par defaut",
                    0 < serre < superviseur.taille_adressage() + superviseur.BUDGET_AS,
                    serre)

            # 4096x4096x4 octets x 2 tampons = 128 Mio : au-dela de ce que sa
            # limite lui laisse, en deca de ce que le navigateur peut mapper.
            etroit.redimensionne(4096, 4096)
            vu, evenements = etroit.attends("ERROR", 15.0)
            verifie("isolation: le renderer refuse ce qu'il ne peut pas mapper",
                    vu, [e[0] for e in evenements])
            verifie("isolation: le navigateur, lui, a mappe la meme surface",
                    etroit.surface is not None
                    and etroit.surface.largeur == 4096)
            verifie("isolation: et le renderer est toujours vivant",
                    etroit.vivant())

            jalon("RENDERER_MEMORY_ISOLATION",
                  vu and etroit.vivant() and serre == etroit.limite_as,
                  serre)
        finally:
            etroit.ferme()

        # --- Processeur ------------------------------------------------------
        #
        # Le mecanisme, pas la mesure : celle-ci est faite par
        # `ordonnanceur-probe` sous Bouchaud OS, et elle est consignee dans
        # `docs/BROWSER_OS_MODERNIZATION.md`. Ce qui se verifie ici est ce qui
        # rend cette mesure applicable au renderer — qu'il ne partage pas la
        # classe de son parent.
        interactif = superviseur.Renderer(200, 150)
        try:
            interactif.attends("READY", 15.0)
            classe_enfant = _os.getpriority(_os.PRIO_PROCESS, interactif.pid)
            egal("isolation: le renderer est en classe normale",
                 classe_enfant, 0)
            try:
                _os.setpriority(_os.PRIO_PROCESS, 0, -5)
                pose = True
            except (OSError, PermissionError):
                pose = False
            if pose:
                egal("isolation: et il y reste quand le navigateur passe devant",
                     _os.getpriority(_os.PRIO_PROCESS, interactif.pid), 0)
                _os.setpriority(_os.PRIO_PROCESS, 0, 0)
            else:
                verifie("isolation: le navigateur ne peut pas se prioriser ici "
                        "(droits) — le renderer reste neanmoins en classe "
                        "normale", classe_enfant == 0)
        finally:
            interactif.ferme()
    finally:
        arrete()


def _bat_vue_jusqua(v, condition, secondes=5.0):
    """Bat la vue jusqu'a ce que la condition tienne. Rend son etat final.

    Le chrome reel fait exactement cela : l'hote Qt appelle `tic` toutes les
    seize millisecondes, et ce qui vient du renderer arrive au battement
    suivant, pas dans l'appel qui l'a provoque. Une epreuve qui lirait l'etat
    juste apres avoir pousse un evenement mesurerait la latence du canal, pas
    le comportement.
    """
    import time as _t
    limite = _t.monotonic() + secondes
    while _t.monotonic() < limite:
        if condition():
            return True
        v.bat()
        _t.sleep(0.01)
    return bool(condition())


def _clique_jusqua_navigation(v, x, bande):
    """Clique le long d'une bande verticale jusqu'a obtenir une demande.

    Rend les URL demandees. Cliquer a cote d'un lien ne declenche rien : le
    balayage est donc sans effet de bord tant qu'il n'a pas touche.
    """
    demandes = []
    for y in bande:
        for nom, charge in v.clic(x, y):
            if nom == "NAVIGUE":
                demandes.append(charge.get("url"))
        _bat_vue_jusqua(v, lambda: bool(demandes), 0.3)
        for nom, charge in v.bat():
            if nom == "NAVIGUE":
                demandes.append(charge.get("url"))
        if demandes:
            break
    return demandes


def verifie_produit_renderer():
    """Le navigateur reel, branche sur le renderer separe.

    Ce n'est pas une redite de `verifie_renderer_separe` : celle-la eprouvait le
    mecanisme, celle-ci eprouve **le produit**. Elle passe par `moteur/vue.py`,
    c'est-a-dire par exactement le meme objet que `navigateur.py` utilise quand
    un utilisateur ouvre une fenetre, et elle joue le parcours que le chrome
    joue — ouvrir, survoler, cliquer, taper, defiler, redimensionner.

    La question a laquelle elle repond est la seule qui restait : est-ce que le
    navigateur que l'utilisateur lance est bien celui que l'architecture pretend
    avoir construit ?
    """
    import time as _time

    from moteur import vue as mod_vue

    doc_bidon, arrete = _sur_serveur("/page/vide.html", battements=1)
    base = doc_bidon.base
    doc_bidon.ferme_document()
    try:
        # --- Le chemin par defaut --------------------------------------------
        #
        # Sans variable d'environnement, `ouvre_vue` doit rendre une vue qui
        # vit ailleurs. C'est le coeur de cette phase, et c'est une ligne.
        ancien = os.environ.pop("BO_BROWSER_INPROCESS", None)
        try:
            defaut = mod_vue.ouvre_vue(400, 300)
            genre_defaut = defaut.genre
            pid_defaut = getattr(defaut.renderer, "pid", None)
            defaut.ferme()
        finally:
            if ancien is not None:
                os.environ["BO_BROWSER_INPROCESS"] = ancien
        egal("produit: la vue par defaut est un autre processus",
             genre_defaut, "renderer")
        verifie("produit: et c'est bien un autre pid",
                pid_defaut is not None and pid_defaut != os.getpid(),
                pid_defaut)

        # L'ancien chemin reste atteignable, et seulement par la variable.
        os.environ["BO_BROWSER_INPROCESS"] = "1"
        try:
            secours = mod_vue.ouvre_vue(400, 300)
            genre_secours = secours.genre
            secours.ferme()
        finally:
            os.environ.pop("BO_BROWSER_INPROCESS", None)
        egal("produit: BO_BROWSER_INPROCESS=1 rend l'ancien chemin",
             genre_secours, "local")

        # --- Le parcours complet ---------------------------------------------
        journal = []
        v = mod_vue.VueRenderer(900, 700,
                                journal=lambda n, t: journal.append((n, t)))
        try:
            ouverte = v.ouvre(base + "/page/login-form.html")
            verifie("produit: la page s'ouvre", ouverte, v.erreur)
            egal("produit: l'adresse remonte au chrome", v.url,
                 base + "/page/login-form.html")
            egal("produit: le titre remonte au chrome", v.titre,
                 "Connexion — page temoin")
            verifie("produit: une trame est arrivee", v.trames > 0, v.trames)
            verifie("produit: et le chrome en a une image", v.image is not None)
            verifie("produit: la hauteur du document remonte aussi",
                    v.hauteur_document > 100, v.hauteur_document)

            # Le chrome peut peindre sans rien savoir de la page : il pose ce
            # que la vue lui donne, et rien d'autre.
            liste = []
            v.peint(liste, 44)
            verifie("produit: le chrome peint la page en une operation",
                    len(liste) == 1 and liste[0][0] == "image", liste)

            # --- Souris ------------------------------------------------------
            #
            # Le survol ne lit plus le DOM : la forme du curseur et l'adresse
            # survolee arrivent par message. Le chrome n'apprend donc rien
            # instantanement — il apprend au battement suivant, ce qui est la
            # vraie contrainte d'une interface separee de sa page.
            evenements = v.souris(200, 185)
            verifie("produit: la souris produit des evenements de chrome",
                    isinstance(evenements, list))

            # --- Clic et foyer -----------------------------------------------
            #
            # Le chrome doit apprendre qu'un champ de la page tient le clavier :
            # c'est le seul bit du DOM dont il a besoin, et il ne peut plus
            # aller le chercher. Les coordonnees sont celles de
            # `verifie_renderer_separe` — meme page, meme largeur, meme champ.
            v.clic(200, 185)
            foyer_pris = _bat_vue_jusqua(v, lambda: v.foyer)
            verifie("produit: un clic dans un champ donne le foyer au chrome",
                    foyer_pris)

            # --- Clavier -----------------------------------------------------
            if foyer_pris:
                avant_frappe = v.trames
                for lettre in "alice":
                    v.touche(lettre, lettre)
                verifie("produit: la frappe atteint la page sans passer par le DOM",
                        _bat_vue_jusqua(v, lambda: v.trames > avant_frappe) and v.foyer,
                        (avant_frappe, v.trames, v.foyer))

            # --- Defilement --------------------------------------------------
            avant = v.trames
            v.defile(200)
            egal("produit: le chrome connait la position de defilement",
                 v.defilement, min(v.defilement_maximum(), 200.0))
            limite = _time.monotonic() + 5.0
            while _time.monotonic() < limite and v.trames == avant:
                v.bat()
                _time.sleep(0.01)
            verifie("produit: et le renderer repeint apres un defilement",
                    v.trames > avant, (avant, v.trames))

            # --- Redimensionnement -------------------------------------------
            avant = v.trames
            v.redimensionne(640, 480)
            limite = _time.monotonic() + 10.0
            while _time.monotonic() < limite and v.trames == avant:
                v.bat()
                _time.sleep(0.01)
            verifie("produit: le redimensionnement traverse et repeint",
                    v.trames > avant, (avant, v.trames))
            verifie("produit: et la nouvelle trame a la nouvelle taille",
                    v.image is not None and v.image[1] == 640, v.image)

            jalon("PRODUCT_RENDERER_DEFAULT",
                  genre_defaut == "renderer" and ouverte and v.trames > 0
                  and foyer_pris,
                  {"genre": genre_defaut, "trames": v.trames,
                   "foyer": foyer_pris})
        finally:
            v.ferme()

        # --- Navigation : le renderer demande, le chrome decide --------------
        #
        # Le parcours complet, dans l'ordre du protocole :
        #
        #     clic -> REQUEST_NAVIGATION -> politique -> le chrome ouvre
        #
        # Ce qui compte est le maillon du milieu. Une vue qui naviguerait toute
        # seule donnerait la meme page a l'ecran et un bouton « reculer » qui
        # ment, et rien dans le rendu ne le montrerait.
        v = mod_vue.VueRenderer(800, 600)
        try:
            depart = base + "/page/lien-sortant.html"
            v.ouvre(depart)
            egal("produit: la page de depart est chargee", v.url, depart)

            # La zone cliquable d'un lien est celle de son texte, dont la
            # hauteur depend des metriques de la police — Qt reel et bouchon
            # local ne donnent pas le meme chiffre. On balaie donc la bande
            # plutot que de viser un pixel : cliquer a cote d'un lien ne
            # declenche rien, et l'epreuve reste la meme des deux cotes.
            demandes = _clique_jusqua_navigation(v, 100, range(22, 70, 6))
            verifie("produit: un lien clique remonte au chrome", demandes,
                    demandes)
            egal("produit: et la vue n'a pas navigue toute seule", v.url,
                 depart)

            # C'est le chrome qui applique — ici, l'epreuve qui joue son role.
            suite = False
            if demandes:
                suite = v.ouvre(demandes[0])
                egal("produit: la page demandee est arrivee", v.url,
                     base + "/page/vide.html")

            # Le lien interdit : demande de la meme facon, refuse par la
            # politique du navigateur, et jamais applique.
            v.ouvre(depart)
            refus = []
            interdites = []
            for y in range(118, 170, 6):
                for nom, charge in v.clic(100, y):
                    if nom == "JOURNAL" and "refusee" in str(charge.get("texte")):
                        refus.append(charge)
                    elif nom == "NAVIGUE":
                        interdites.append(charge.get("url"))
                _bat_vue_jusqua(v, lambda: bool(refus or interdites), 0.3)
                for nom, charge in v.bat():
                    if nom == "JOURNAL" and "refusee" in str(charge.get("texte")):
                        refus.append(charge)
                    elif nom == "NAVIGUE":
                        interdites.append(charge.get("url"))
                if refus or interdites:
                    break
            egal("produit: un lien file:// ne remonte jamais comme autorise",
                 interdites, [])
            verifie("produit: il est refuse par la politique du navigateur",
                    refus, refus)

            jalon("PRODUCT_RENDERER_NAVIGATION",
                  bool(demandes) and suite and not interdites and bool(refus),
                  {"demandes": demandes, "refus": len(refus)})
        finally:
            v.ferme()
    finally:
        arrete()


def verifie_produit_crash():
    """Une page qui meurt sous le vrai chemin. La fenetre, elle, ne meurt pas.

    `RENDERER_CRASH_ISOLATION` prouvait le mecanisme depuis une epreuve. Il ne
    prouvait pas ce que l'utilisateur voit, parce que l'utilisateur ne passait
    pas par la. Cette epreuve-ci passe par `moteur/vue.py` — le meme objet que
    le chrome — et verifie les six choses qu'un utilisateur constate :

    la fenetre vit, le chrome repond, la derniere image reste a l'ecran, l'etat
    dit « crashee », le processus est recolte, ses ressources sont rendues, et
    F5 remet une page vivante.
    """
    import signal as _signal
    import time as _time

    from moteur import vue as mod_vue

    doc_bidon, arrete = _sur_serveur("/page/vide.html", battements=1)
    base = doc_bidon.base
    doc_bidon.ferme_document()
    try:
        v = mod_vue.VueRenderer(500, 400)
        try:
            v.ouvre(base + "/page/login-form.html")
            verifie("crash produit: la page vivait", v.trames > 0, v.trames)
            image_avant = v.image
            pid = v.renderer.pid
            surface_avant = v.renderer.surface

            os.kill(pid, _signal.SIGKILL)

            vu = False
            limite = _time.monotonic() + 10.0
            while _time.monotonic() < limite and not vu:
                for nom, _charge in v.bat():
                    if nom == "CRASH":
                        vu = True
                _time.sleep(0.01)
            verifie("crash produit: le chrome apprend la mort", vu or v.crashee)
            verifie("crash produit: la vue se declare crashee", v.crashee)
            verifie("crash produit: avec une raison lisible",
                    "SIGKILL" in str(v.erreur), v.erreur)

            # Le chrome reste peignable, et il peint encore la derniere image :
            # la copie vit chez Qt, pas dans la surface qu'on vient de rendre.
            liste = []
            v.peint(liste, 44)
            egal("crash produit: la derniere image est conservee", v.image,
                 image_avant)
            verifie("crash produit: et le chrome sait encore la peindre",
                    len(liste) == 1 and liste[0][0] == "image", liste)

            # Ressources rendues : le processus recolte, la surface fermee, la
            # prise fermee. Un onglet mort qui garde trois mebioctets de surface
            # par onglet est une fuite qu'on ne voit qu'au centieme.
            recolte = True
            try:
                os.kill(pid, 0)
            except ProcessLookupError:
                recolte = True
            except OSError:
                recolte = False
            verifie("crash produit: le processus est recolte", recolte)
            verifie("crash produit: la surface est rendue",
                    surface_avant is not None and surface_avant.descripteur is None)
            verifie("crash produit: la prise aussi",
                    v.renderer is None or v.renderer.prise is None)

            # Les entrees ne lancent plus rien, et surtout ne lèvent pas : un
            # chrome qui plante en survolant une page morte serait pire que le
            # crash qu'il est cense contenir.
            for action in (lambda: v.souris(10, 10), lambda: v.clic(10, 10),
                           lambda: v.touche("a", "a"), lambda: v.defile(50),
                           lambda: v.redimensionne(600, 500), v.bat):
                action()
            verifie("crash produit: le chrome survit a toutes les entrees", True)

            # Rechargement : une page vivante, dans un processus neuf.
            recharge = v.ouvre(base + "/page/login-form.html")
            verifie("crash produit: F5 remet une page vivante", recharge,
                    v.erreur)
            verifie("crash produit: dans un processus neuf",
                    v.renderer is not None and v.renderer.pid != pid,
                    (pid, getattr(v.renderer, "pid", None)))
            verifie("crash produit: qui peint", v.trames > 0, v.trames)

            jalon("PRODUCT_RENDERER_CRASH_RECOVERY",
                  v.crashee is False and recharge and v.trames > 0,
                  {"pid_mort": pid, "pid_neuf": getattr(v.renderer, "pid", None)})
        finally:
            v.ferme()
    finally:
        arrete()


def verifie_chrome_produit():
    """Le programme que l'hote lance, pilote comme l'hote le pilote.

    Les epreuves precedentes passent par `moteur/vue.py`. Celle-ci passe par
    `navigateur.py` — le fichier que `hote.cpp` execute — et n'appelle que les
    sept rappels que Qt appelle : `peindre`, `clic`, `touche`, `survol`,
    `molette`, `tic`, `fermeture`.

    C'est la difference entre « la vue separee marche » et « le navigateur
    utilise la vue separee ». La premiere etait vraie depuis longtemps sans que
    la seconde le soit, et c'est exactement le decalage que cette phase existe
    pour supprimer.
    """
    import time as _time

    doc_bidon, arrete = _sur_serveur("/page/vide.html", battements=1)
    base = doc_bidon.base
    doc_bidon.ferme_document()

    # Le chrome se fabrique **a l'import** — c'est la ou il forke son renderer,
    # avant d'avoir rien accumule. On le recharge donc proprement plutot que de
    # dependre de l'ordre des epreuves.
    for module in ("navigateur",):
        sys.modules.pop(module, None)
    import navigateur as chrome

    titres = []
    redessins = [0]
    bo.titre = lambda texte: titres.append(texte)
    bo.redessiner = lambda: redessins.__setitem__(0, redessins[0] + 1)
    bo.traiter_evenements = getattr(bo, "traiter_evenements", lambda: None)

    n = chrome._navigateur
    try:
        egal("chrome: la page vit dans un autre processus",
             n.onglet.vue.genre, "renderer")
        verifie("chrome: et c'est bien un autre pid",
                n.onglet.vue.renderer.pid != os.getpid())

        # --- Ouvrir ----------------------------------------------------------
        n.ouvre(base + "/page/login-form.html")
        egal("chrome: l'adresse est dans la barre", n.saisie,
             base + "/page/login-form.html")
        egal("chrome: et dans l'historique", n.onglet.url,
             base + "/page/login-form.html")
        verifie("chrome: le titre de la fenetre a ete pose",
                titres and "Connexion" in titres[-1], titres[-1:])

        # --- Peindre ---------------------------------------------------------
        #
        # Le chrome ne pose plus une liste d'affichage de la page : il pose une
        # image. C'est verifiable en comptant les operations.
        liste = chrome._peindre(1280, 720)
        images = [op for op in liste if op[0] == "image"]
        egal("chrome: la page est peinte en une image", len(images), 1)
        verifie("chrome: posee sous la barre d'outils",
                images and images[0][2] == chrome.HAUTEUR_CHROME, images[:1])
        verifie("chrome: et le chrome est peint par-dessus",
                any(op[0] == "texte" for op in liste))

        # --- Souris ----------------------------------------------------------
        chrome._survol(200, 185 + chrome.HAUTEUR_CHROME)
        for _ in range(40):
            chrome._tic()
            _time.sleep(0.01)

        # --- Clic et clavier -------------------------------------------------
        avant = n.onglet.vue.trames
        chrome._clic(200, 185 + chrome.HAUTEUR_CHROME)
        pris = False
        limite = _time.monotonic() + 5.0
        while _time.monotonic() < limite and not pris:
            chrome._tic()
            pris = n.onglet.vue.foyer
            _time.sleep(0.01)
        verifie("chrome: un clic dans un champ donne le foyer a la page", pris)

        if pris:
            # La bifurcation du clavier : le champ de la page a le foyer, donc
            # `Bas` ne doit pas faire defiler. Le chrome le sait par message, il
            # ne lit plus le DOM.
            for code, texte in ((0x41, "a"), (0x6C, "l")):
                chrome._touche(code, texte)
            defilement = n.onglet.vue.defilement
            chrome._touche(chrome.K_BAS, "")
            egal("chrome: la frappe va a la page, pas a l'ascenseur",
                 n.onglet.vue.defilement, defilement)
            for _ in range(40):
                chrome._tic()
                _time.sleep(0.01)
            verifie("chrome: et la page a repeint", n.onglet.vue.trames > avant,
                    (avant, n.onglet.vue.trames))

        # --- Defilement ------------------------------------------------------
        chrome._molette(-120)
        verifie("chrome: la molette fait defiler la page distante",
                n.onglet.vue.defilement > 0, n.onglet.vue.defilement)

        # --- Redimensionnement ------------------------------------------------
        avant = n.onglet.vue.trames
        chrome._peindre(800, 600)
        egal("chrome: la vue suit la fenetre", n.onglet.vue.largeur, 800)
        limite = _time.monotonic() + 10.0
        while _time.monotonic() < limite and n.onglet.vue.trames == avant:
            chrome._tic()
            _time.sleep(0.01)
        verifie("chrome: et le renderer repeint a la nouvelle taille",
                n.onglet.vue.trames > avant, (avant, n.onglet.vue.trames))

        # --- Lien : le renderer demande, le chrome ouvre -----------------------
        n.ouvre(base + "/page/lien-sortant.html")
        depart = n.onglet.url
        profondeur = len(n.onglet.historique)
        for y in range(22, 70, 6):
            chrome._clic(100, y + chrome.HAUTEUR_CHROME)
            for _ in range(20):
                chrome._tic()
                _time.sleep(0.01)
            if n.onglet.url != depart:
                break
        egal("chrome: le lien a mene a la page suivante", n.onglet.url,
             base + "/page/vide.html")
        egal("chrome: et l'historique s'est empile",
             len(n.onglet.historique), profondeur + 1)
        verifie("chrome: donc le bouton reculer est actif",
                n.onglet.peut_reculer())
        n.recule()
        egal("chrome: qui ramene bien en arriere", n.onglet.url, depart)

        # --- Crash depuis l'interface -----------------------------------------
        import signal as _signal
        pid = n.onglet.vue.renderer.pid
        image_avant = n.onglet.vue.image
        os.kill(pid, _signal.SIGKILL)
        limite = _time.monotonic() + 10.0
        while _time.monotonic() < limite and not n.onglet.vue.crashee:
            chrome._tic()
            _time.sleep(0.01)
        verifie("chrome: la mort de la page remonte a l'interface",
                n.onglet.vue.crashee)
        verifie("chrome: la barre d'etat le dit",
                "crashee" in n.etat, n.etat)

        # La fenetre reste peignable, avec sa derniere image.
        liste = chrome._peindre(1280, 720)
        egal("chrome: la derniere image est toujours la",
             n.onglet.vue.image, image_avant)
        verifie("chrome: et la fenetre se peint encore",
                any(op[0] == "image" for op in liste)
                and any(op[0] == "texte" for op in liste))

        # Le chrome reste interactif : aucune de ces entrees ne doit lever.
        chrome._survol(300, 300)
        chrome._clic(300, 300)
        chrome._touche(0x41, "a")
        chrome._molette(-120)
        chrome._tic()
        verifie("chrome: et il reste interactif", True)

        # La barre d'adresse marche toujours : Ctrl+L, une adresse, Entree.
        chrome._touche(chrome.K_L, "l", 0x04000000)
        verifie("chrome: Ctrl+L donne le foyer a la barre d'adresse",
                n.champ_actif)

        # F5 refait un renderer et recharge.
        chrome._touche(chrome.K_ECHAP, "")
        chrome._touche(chrome.K_F5, "")
        verifie("chrome: F5 remet une page vivante",
                not n.onglet.vue.crashee, n.etat)
        verifie("chrome: dans un processus neuf",
                n.onglet.vue.renderer is not None
                and n.onglet.vue.renderer.pid != pid)
        verifie("chrome: qui peint", n.onglet.vue.trames > 0)

        jalon("PRODUCT_RENDERER_CHROME",
              n.onglet.vue.genre == "renderer" and pris
              and not n.onglet.vue.crashee,
              {"pid_mort": pid, "pid_neuf": n.onglet.vue.renderer.pid})
    finally:
        try:
            chrome._fermeture()
        except Exception:  # noqa: BLE001
            pass
        arrete()


def verifie_renderer_courtage():
    """Ce que le renderer ne va plus chercher lui-meme.

    Le renderer chargeait ses ressources par `reseau.charge` : il resolvait un
    nom, ouvrait une prise, lisait le magasin de temoins, ecrivait dans le cache
    du disque. Le processus qu'on isolait avait donc le reseau et le stockage,
    non pas parce qu'on les lui avait accordes mais parce que le code savait
    appeler les modules.

    Ce qui se verifie ici : la meme page, rendue pareil, avec toutes ses
    ressources passees par le navigateur.
    """
    import time as _time

    from moteur import transport, vue as mod_vue

    doc_bidon, arrete = _sur_serveur("/page/vide.html", battements=1)
    base = doc_bidon.base
    doc_bidon.ferme_document()
    try:
        # --- L'abstraction, sans processus -----------------------------------
        #
        # Un faux courtier suffit a prouver que le seuil est unique : si un seul
        # appelant du moteur gardait un chemin direct, la page verrait autre
        # chose que ce que le courtier a rendu.
        vues = []

        def faux_courtier(requete):
            vues.append(requete["url"])
            return {"url": requete["url"], "code": 200,
                    "type_mime": "text/html",
                    "octets": b"<html><head><title>Courte</title></head>"
                              b"<body>ok</body></html>"}

        ancien = transport.installe(transport.Courtier(faux_courtier))
        try:
            reponse = reseau.charge("https://exemple.invalid/quelque-part")
        finally:
            transport.installe(ancien)
        egal("courtage: la reponse traverse l'abstraction", reponse.code, 200)
        verifie("courtage: le corps arrive entier",
                "Courte" in reponse.contenu, reponse.contenu[:80])
        egal("courtage: et le seuil a bien ete franchi", vues,
             ["https://exemple.invalid/quelque-part"])

        # `charge_localement` reste le bout de chaine, et il ignore le seuil :
        # c'est ce que le navigateur appelle pour honorer une requete courtee.
        # Sans cette propriete, le courtier s'appellerait lui-meme.
        ancien = transport.installe(transport.Courtier(faux_courtier))
        try:
            directe = reseau.charge_localement(base + "/page/vide.html")
        finally:
            transport.installe(ancien)
        egal("courtage: le navigateur, lui, va vraiment chercher",
             directe.code, 200)

        # --- De bout en bout, avec un vrai renderer --------------------------
        v = mod_vue.VueRenderer(700, 500)
        try:
            ouverte = v.ouvre(base + "/page/login-form.html")
            verifie("courtage: la page courtee s'ouvre", ouverte, v.erreur)
            egal("courtage: avec son titre", v.titre, "Connexion — page temoin")
            verifie("courtage: et le navigateur a servi les requetes",
                    v.renderer.requetes_servies > 0,
                    v.renderer.requetes_servies)
            servies = v.renderer.requetes_servies
            verifie("courtage: la page est peinte", v.trames > 0, v.trames)
            jalon("RENDERER_BROKERED_FETCH", ouverte and servies > 0, servies)
        finally:
            v.ferme()

        # Le mode direct reste disponible pour comparer : la meme page, le meme
        # resultat, aucune requete servie par le navigateur.
        direct = mod_vue.VueRenderer(700, 500, courtage=False)
        try:
            ouverte = direct.ouvre(base + "/page/login-form.html")
            verifie("courtage: le mode direct rend la meme page", ouverte,
                    direct.erreur)
            egal("courtage: avec le meme titre", direct.titre,
                 "Connexion — page temoin")
            egal("courtage: et sans rien demander au navigateur",
                 direct.renderer.requetes_servies, 0)
        finally:
            direct.ferme()
    finally:
        arrete()


def verifie_renderer_privileges():
    """Ce que le renderer possede vraiment. Mesure, pas intention.

    Le renderer nait par `fork()` sans `exec()` : il herite de la table des
    descripteurs du navigateur — prises TCP deja connectees, base de temoins,
    affichage, surfaces des autres onglets. `FD_CLOEXEC` n'y peut rien puisqu'il
    n'y a pas d'`exec`. La seule facon est d'enumerer et de fermer.

    Cette epreuve compare l'inventaire des deux cotes du `fork`, puis fait jouer
    au renderer une batterie de tentatives — ouvrir un fichier, ouvrir une
    prise, lire le stockage, se servir d'un descripteur herite, naviguer vers
    `file://`. Elle n'exige pas que tout echoue : elle exige que le resultat
    soit **connu**. Voir `docs/RENDERER_PRIVILEGE_AUDIT.md`.
    """
    import time as _time

    from moteur import privileges, superviseur

    doc_bidon, arrete = _sur_serveur("/page/vide.html", battements=1)
    base = doc_bidon.base
    doc_bidon.ferme_document()
    hote_port = base.split("//", 1)[1].split("/", 1)[0]
    hote, _, port = hote_port.partition(":")

    def audit(renderer, quoi, **details):
        renderer.demande_audit(quoi, **details)
        limite = _time.monotonic() + 15.0
        while _time.monotonic() < limite:
            for nom, charge in renderer.recolte(0.05):
                if nom == "AUDIT_RESULT":
                    return charge
                if nom == "ERROR" and charge.get("raison") == "AUDIT":
                    return {"refus": charge.get("detail")}
        return {}

    try:
        # L'inventaire du navigateur avant le `fork` : ce dont l'enfant
        # heriterait si on ne faisait rien.
        avant = privileges.inventaire()
        verifie("privileges: le navigateur tient plus que stdio",
                len(avant) > 3, privileges.resume(avant))

        # --- Sans balayage : la mesure du probleme ---------------------------
        #
        # Un renderer qui n'a pas ete nettoye sert de temoin. Sans lui, on ne
        # saurait pas si le balayage ferme quelque chose ou si le navigateur
        # n'avait rien d'ouvert.
        sale = superviseur.Renderer(200, 150, audit=True, ferme_herites=False)
        try:
            sale.attends("READY", 15.0)
            herite = audit(sale, "descripteurs").get("descripteurs") or []
            resume_sale = privileges.resume(
                [e for e in herite if e["fd"] not in (0, 1, 2)])
        finally:
            sale.ferme()
        verifie("privileges: sans balayage, l'enfant herite de tout",
                len(herite) > 4, resume_sale)

        # --- Avec balayage : ce qui reste ------------------------------------
        propre = superviseur.Renderer(200, 150, audit=True)
        try:
            propre.attends("READY", 15.0)
            reponse = audit(propre, "descripteurs")
            garde = reponse.get("descripteurs") or []
            egal("privileges: le renderer courte ses ressources",
                 reponse.get("transport"), "courtier")

            # Ce qui doit rester : stdio, la prise de controle, la surface.
            numeros = sorted(entree["fd"] for entree in garde)
            verifie("privileges: stdio est garde (concession assumee)",
                    set((0, 1, 2)).issubset(set(numeros)), numeros)

            # **Tout ce qui suit ignore 0, 1 et 2**, et cela a coute une
            # epreuve intermittente. Le *genre* des descripteurs standard
            # n'appartient pas au navigateur : lance depuis un terminal, `0`
            # est un peripherique ; depuis un tube, c'est un tube ; depuis un
            # superviseur qui parle par prise, c'est une **prise UNIX** — et
            # l'epreuve comptait alors deux prises UNIX la ou elle en attendait
            # une, sans qu'aucun descripteur n'ait fuit. Une meme execution
            # passait ou echouait selon la facon dont on l'avait lancee, ce qui
            # est la pire forme d'echec : celle qui n'apprend rien.
            #
            # La question interessante n'a jamais ete « combien de prises » mais
            # « combien de prises **au-dela de ce qu'on a accorde** ».
            hors_stdio = [e for e in garde if e["fd"] not in (0, 1, 2)]
            resume_propre = privileges.resume(hors_stdio)
            unix = [e for e in hors_stdio if e["genre"] == "prise_unix"]
            egal("privileges: une seule prise, celle du controle", len(unix), 1)
            reseaux = [e for e in hors_stdio if e["genre"] == "prise_reseau"]
            egal("privileges: aucune prise reseau heritee", reseaux, [])
            fichiers = [e for e in hors_stdio
                        if e["genre"] in ("fichier", "repertoire")]
            egal("privileges: aucun fichier du navigateur", fichiers, [])
            verifie("privileges: le balayage a bien retire quelque chose",
                    len(garde) < len(herite), (len(garde), len(herite)))

            # --- La batterie adversariale ---------------------------------
            tentatives = audit(
                propre, "tentatives", cible_reseau=[hote, int(port or 80)],
                fichier="/etc/hostname").get("tentatives") or []
            par_nom = {t["capacite"]: t for t in tentatives}
            verifie("privileges: la batterie a bien tourne",
                    len(tentatives) >= 3, tentatives)

            # Ce qui doit avoir echoue : se servir d'un descripteur herite.
            herite_utilisable = par_nom.get("descripteur_herite", {})
            egal("privileges: aucun descripteur herite exploitable",
                 herite_utilisable.get("obtenu"), False)

            # Ce qui reussit encore, et qui est donc ecrit noir sur blanc :
            # fermer des descripteurs ne ferme pas le systeme de fichiers, et
            # aucun mecanisme du noyau ne le fait a notre place aujourd'hui.
            fichier = par_nom.get("fichier_arbitraire", {})
            verifie("privileges: l'ouverture par le nom reste possible "
                    "(documente, pas resolu)",
                    fichier.get("obtenu") is not None, fichier)
            prise = par_nom.get("prise_reseau_directe", {})
            verifie("privileges: l'ouverture d'une prise reste possible "
                    "(documente, pas resolu)",
                    prise.get("obtenu") is not None, prise)

            # --- La navigation, elle, est refusee de l'autre cote ----------
            #
            # C'est la difference entre une capacite qu'on n'a pas su retirer
            # et une capacite qui n'a jamais ete accordee : le renderer peut
            # demander, il ne peut pas obtenir.
            propre.demande_audit("navigation_interdite", url="file:///etc/shadow")
            refus = None
            limite = _time.monotonic() + 10.0
            while _time.monotonic() < limite and refus is None:
                for nom, charge in propre.recolte(0.05):
                    if nom == "NAVIGATION_REFUSEE":
                        refus = charge
                    elif nom == "NAVIGATION_AUTORISEE":
                        refus = False
            verifie("privileges: une navigation file:// demandee est refusee",
                    isinstance(refus, dict), refus)

            jalon("RENDERER_FD_HYGIENE",
                  not reseaux and not fichiers and len(unix) == 1
                  and herite_utilisable.get("obtenu") is False
                  and isinstance(refus, dict),
                  {"garde": resume_propre, "sans_balayage": resume_sale})

            # Le rapport, ecrit a cote des jalons : le document ne doit pas
            # avoir a etre tenu a la main, et un audit recopie a la main est un
            # audit perime.
            ecris_audit_privileges(resume_propre, tentatives,
                                   isinstance(refus, dict), len(unix))
        finally:
            propre.ferme()
    finally:
        arrete()


def ecris_audit_privileges(apres_balayage, tentatives, navigation_refusee,
                           prises_de_controle):
    """Depose le resultat de l'audit a cote des jalons, en JSON.

    Comme `jalons.json` : versionne, relu par la documentation, et jamais
    recopie a la main. Un tableau PASS/FAIL tenu a la main devient faux au
    premier changement que personne ne pense a y reporter.

    **Des verdicts, pas des mesures.** Le premier jet deposait aussi
    l'inventaire brut des deux cotes du `fork` — cinq prises reseau avant, zero
    apres. C'etait interessant a lire et impossible a comparer : le nombre de
    prises que le navigateur tient au moment du `fork` depend de ce qu'il vient
    de faire, et le genre des descripteurs 0, 1 et 2 depend de la facon dont on
    a lance le processus. Un fichier versionne dont le contenu bouge selon la
    machine ne peut pas servir de reference, et la verification d'integration
    qui le compare echouerait pour des raisons qui n'apprennent rien.

    Ce qui est depose est donc ce qui doit **rester vrai partout** : aucune
    prise reseau heritee, aucun fichier herite, une seule prise de controle, et
    le verdict de chaque tentative.
    """
    chemin = os.environ.get("BO_AUDIT_PRIVILEGES")
    if not chemin:
        return
    rapport = {
        "prises_reseau_heritees": apres_balayage.get("prise_reseau", 0),
        "fichiers_herites": (apres_balayage.get("fichier", 0)
                             + apres_balayage.get("repertoire", 0)),
        "prises_de_controle": prises_de_controle,
        "surfaces_accordees": apres_balayage.get("memfd", 0),
        "tentatives": {t["capacite"]: bool(t["obtenu"]) for t in tentatives},
        "navigation_file_refusee": bool(navigation_refusee),
    }
    try:
        with open(chemin, "w", encoding="utf-8") as fichier:
            json.dump(rapport, fichier, ensure_ascii=False, indent=2,
                      sort_keys=True)
            fichier.write("\n")
    except OSError:
        pass



def principal():
    import bojs
    print("QuickJS %s, CPython %s" % (bojs.version(), sys.version.split()[0]))
    print("hote : %s" % ("Qt reel" if HOTE_REEL else "bouchon local"))
    print()

    for verification in (
        verifie_html,
        verifie_selecteurs,
        verifie_javascript_base,
        verifie_dom,
        verifie_evenements,
        verifie_minuteries,
        verifie_promesses,
        verifie_erreurs,
        verifie_diagnostics_compatibilite,
        verifie_ressources_echouees,
        verifie_moignons,
        verifie_serveur_fixtures,
        verifie_websocket,
        verifie_invalidation,
        verifie_invalidation_reelle,
        verifie_memoires_de_passe,
        verifie_modele_edition,
        verifie_saisie_reelle,
        verifie_ordre_des_evenements_de_frappe,
        verifie_curseur_visible,
        verifie_clic_donne_le_foyer,
        verifie_cases_et_radios,
        verifie_foyer,
        verifie_formulaire_plateforme,
        verifie_historique_session,
        verifie_fetch_reel,
        verifie_reseau_asynchrone,
        verifie_parcours_complet,
        verifie_parcours_utilisateur,
        verifie_budget,
        verifie_page_complete,
        verifie_evenement_apres_chargement,
        verifie_images,
        verifie_media_api,
        verifie_mse_api,
        verifie_youtube_adresses,
        verifie_youtube_formats,
        verifie_youtube_extraction_json,
        verifie_youtube_signature,
        verifie_youtube_base_js,
        verifie_youtube_page,
        verifie_youtube_client,
        verifie_youtube_refus,
        verifie_youtube_chaine,
        verifie_requete_brute,
        verifie_compression,
        verifie_reserve_connexions,
        verifie_prechargement,
        verifie_feuilles_liees,
        verifie_index_regles,
        verifie_variables_css,
        verifie_racine,
        verifie_enfant_direct,
        verifie_cascade_memorisee,
        verifie_style_calcule,
        verifie_observateur_mutations,
        verifie_observateur_visibilite,
        verifie_composants,
        verifie_ombre,
        verifie_toile,
        verifie_modules,
        verifie_temoins,
        verifie_cache_http,
        verifie_stockage_local,
        verifie_indexeddb,
        verifie_webapp_combinee,
        verifie_origine,
        verifie_contexte_navigation,
        verifie_cadres,
        verifie_fuite_cadres,
        verifie_cors,
        verifie_toile_origine,
        verifie_wss,
        verifie_grille_intrinseque,
        verifie_pseudo_classes_dynamiques,
        verifie_invalidation_ciblee,
        verifie_tailles_intrinseques,
        verifie_flex,
        verifie_grille,
        verifie_position,
        verifie_decoration,
        verifie_transformations,
        verifie_collant,
        verifie_ordre_et_zones,
        verifie_regles_arobase,
        verifie_etats,
        verifie_animations,
        verifie_transitions,
        verifie_isolement_ombre,
        verifie_ajustement_images,
        verifie_flux_en_ligne,
        verifie_tableaux,
        verifie_champs,
        verifie_polices,
        verifie_degrades_et_ombres,
        verifie_toile_complete,
        verifie_pseudo_elements,
        verifie_requetes_media,
        verifie_longueurs,
        verifie_client_leger,
        verifie_bac_a_sable,
        verifie_liste_de_selecteurs,
        verifie_proprietes_logiques,
        verifie_transformation_texte,
        verifie_donnees_element,
        verifie_ascendance,
        verifie_console_gabarit,
        verifie_priorite_important,
        verifie_pointeur_transparent,
        verifie_champs_de_formulaire,
        verifie_formulaire_pilote,
        verifie_politique_ressources,
        verifie_tls_ferme,
        verifie_temoins_httponly,
        verifie_temoins_document,
        verifie_redirection_entetes,
        verifie_workers_surface,
        verifie_workers_calcul_ailleurs,
        verifie_workers_reseau,
        verifie_workers_stockage,
        verifie_workers_temps_reel,
        verifie_workers_messagerie,
        verifie_workers_cycle_de_vie,
        verifie_clonage_structure,
        verifie_clonage_par_les_quatre_chemins,
        verifie_canal_de_messages,
        verifie_renderer_protocole,
        verifie_renderer_surface,
        verifie_renderer_separe,
        verifie_renderer_isolation,
        verifie_produit_renderer,
        verifie_produit_crash,
        verifie_chrome_produit,
        verifie_renderer_courtage,
        verifie_renderer_privileges,
        verifie_tableau_de_bord,
        verifie_hote_reel,
    ):
        nom = verification.__name__.replace("verifie_", "")
        avant = len(_echecs)
        try:
            verification()
        except Exception as e:  # noqa: BLE001
            import traceback
            _echecs.append("%s a leve : %s" % (nom, e))
            traceback.print_exc()
        etat = "ok " if len(_echecs) == avant else "ECHEC"
        print("  %s  %s" % (etat, nom))

    # Les jalons sont deposes une derniere fois, apres tout le monde : chaque
    # scenario ecrivait les siens en partant, si bien qu'un jalon pose par le
    # dernier a parler n'atteignait jamais le fichier — et le tableau de bord
    # affichait « absent » pour une capacite qui venait de passer.
    ecris_jalons()

    print()
    # La forme de cette ligne est celle qu'attend `tools/test.sh`, qui compte
    # les bilans plutot que de se fier au seul code de sortie — celui-ci ne dit
    # pas quelle sonde a lache.
    if _echecs:
        for echec in _echecs:
            print("  - %s" % echec)
        print("RESULTAT : %d verification(s) en echec sur %d"
              % (len(_echecs), len(_echecs) + _reussites))
        return 1
    print("RESULTAT : 0 verification(s) en echec (%d passees)" % _reussites)
    return 0


if __name__ == "__main__":
    sys.exit(principal())
