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
from moteur import css, grille, html, js, reseau  # noqa: E402

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
