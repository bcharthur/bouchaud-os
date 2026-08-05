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

import struct
import sys
import types

# --- L'hote : le vrai s'il est la, un bouchon sinon --------------------------

_images = []


def _largeur_texte(texte, taille, gras=False, fixe=False):
    # Approximation suffisante pour la mise en page : la vraie mesure vient de
    # Qt, et aucune verification ici ne depend du dixieme de pixel.
    return len(texte) * taille * (0.62 if not fixe else 0.60) * (1.05 if gras else 1.0)


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
    bo.formats_images = lambda: ["png"]
    sys.modules["bo"] = bo
    HOTE_REEL = False

sys.path.insert(0, ".")

import moteur  # noqa: E402
from moteur import html, js, reseau  # noqa: E402

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
    doc = moteur.Document(reponse, 1000,
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
    egal("select: pseudo-classe ignoree", len(select("a:hover")), 2)
    egal("select: universel dans contexte", len(select("*")),
         len([n for n in doc.racine.parcours() if isinstance(n, html.Element)]))
    contexte.ferme()


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


def verifie_bac_a_sable():
    """Une page ne doit pas pouvoir atteindre le systeme."""
    doc = document("<body></body>")
    contexte = js.Contexte(doc)
    for expression in ("typeof require", "typeof process", "typeof std",
                       "typeof os", "typeof scriptArgs", "typeof loadFile",
                       "typeof Deno", "typeof __loadScript"):
        egal("bac a sable: %s" % expression, contexte.execute(expression), "undefined")
    contexte.ferme()


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
        verifie_bac_a_sable,
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
