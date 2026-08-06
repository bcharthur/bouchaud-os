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

    def faux_charge(url, methode="GET", corps=None, entetes=None, brut=False):
        vues["brut"] = brut
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
    contexte.ferme()


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
          #b { width: 200px; padding: 10px; border-width: 5px;
               box-sizing: border-box; }
          #n { width: 200px; padding: 10px; border-width: 5px; }
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

    def charge(url, methode="GET", corps=None, entetes=None, brut=False):
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
    # La descendance, elle, atteint bien les deux.
    egal("enfant direct: la descendance atteint le petit-fils",
         imbrique.style.get("border-color"), "#0000ff")

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
        verifie_media_api,
        verifie_mse_api,
        verifie_youtube_adresses,
        verifie_youtube_formats,
        verifie_youtube_extraction_json,
        verifie_youtube_signature,
        verifie_youtube_base_js,
        verifie_youtube_page,
        verifie_requete_brute,
        verifie_compression,
        verifie_reserve_connexions,
        verifie_prechargement,
        verifie_feuilles_liees,
        verifie_index_regles,
        verifie_enfant_direct,
        verifie_cascade_memorisee,
        verifie_style_calcule,
        verifie_observateur_mutations,
        verifie_observateur_visibilite,
        verifie_composants,
        verifie_ombre,
        verifie_toile,
        verifie_modules,
        verifie_flex,
        verifie_grille,
        verifie_position,
        verifie_pseudo_elements,
        verifie_requetes_media,
        verifie_longueurs,
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
