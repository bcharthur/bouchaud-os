"""Fabrique un temoin a la taille d'un vrai site.

Les temoins locaux tiennent en quelques dizaines de regles : un index de
selecteurs n'y a rien a filtrer, et le banc d'essai ne peut donc pas voir son
effet. Les vrais sites, eux, servent des milliers de regles — pypi.org en
compte 8 192 pour 2 300 elements, et c'est la que le filtrage se joue.

Ce fichier fabrique un temoin de cette taille, avec la meme *forme* qu'une
feuille reelle plutot que des regles uniformes : des classes utilitaires en
majorite, des selecteurs descendants, des pseudo-elements, des attributs, des
`:is()` et des `:not()`. La forme compte autant que le nombre — c'est elle qui
decide de ce qu'un index peut ranger.
"""
import os
import random

random.seed(20260811)

SORTIE = "/home/user/bouchaud-os/tools/userland/navigateur/tests/pages/feuille-lourde.html"

COMPOSANTS = ["carte", "panneau", "entete", "pied", "barre", "menu", "liste",
              "champ", "bouton", "etiquette", "grille", "colonne", "media",
              "avatar", "insigne", "onglet", "modale", "info", "alerte", "note"]
VARIANTES = ["primaire", "secondaire", "discret", "vif", "plat", "borde",
             "compact", "large", "sombre", "clair"]
BALISES = ["div", "span", "p", "li", "a", "h2", "h3", "section", "article",
           "button", "label", "td", "nav", "header", "footer"]

regles = []

# 1. Classes utilitaires : le gros d'une feuille moderne (Tailwind, Bootstrap).
for composant in COMPOSANTS:
    for variante in VARIANTES:
        regles.append(".%s--%s { color: #1a1a1a; padding: 4px; }"
                      % (composant, variante))
for i in range(1, 65):
    regles.append(".p-%d { padding: %dpx; }" % (i, i))
    regles.append(".m-%d { margin: %dpx; }" % (i, i))
    regles.append(".w-%d { width: %dpx; }" % (i, i * 4))
    regles.append(".t-%d { font-size: %dpx; }" % (i, 8 + i % 24))

# 2. Selecteurs descendants : ceux qui coutent a verifier, pas a filtrer.
for composant in COMPOSANTS:
    for balise in BALISES[:8]:
        regles.append(".%s %s { line-height: 1.4; }" % (composant, balise))
        regles.append(".%s > .%s__corps %s { margin-top: 2px; }"
                      % (composant, composant, balise))

# 3. Regles par balise.
for balise in BALISES:
    regles.append("%s { box-sizing: border-box; }" % balise)
    regles.append("%s.actif { font-weight: 600; }" % balise)

# 4. Pseudo-elements : la moitie des sites posent leurs icones ainsi.
for composant in COMPOSANTS:
    regles.append(".%s::before { content: \"\"; width: 4px; }" % composant)
    regles.append(".%s::after { content: \"\"; width: 2px; }" % composant)

# 5. Attributs, `:is()`, `:not()` — les formes que l'index doit distinguer.
for genre in ("text", "email", "password", "search", "checkbox", "radio"):
    regles.append("[type=%s] { border: 1px solid #cccccc; }" % genre)
regles.append("[hidden] { display: none; }")
regles.append("[disabled] { color: #999999; }")
regles.append(":is(h2, h3, h4) { font-weight: 700; }")
regles.append(":is(.carte, .panneau) .titre { font-size: 18px; }")
regles.append(":not(.modale) { position: relative; }")
regles.append(":not(.brut) .contenu { padding: 2px; }")
regles.append("* { -webkit-font-smoothing: antialiased; }")
regles.append(":root { --fond: #ffffff; }")

# On complete jusqu'a la taille d'une vraie feuille.
while len(regles) < 4000:
    composant = random.choice(COMPOSANTS)
    variante = random.choice(VARIANTES)
    regles.append(".%s-%s-%d { color: #%02x%02x%02x; }"
                  % (composant, variante, len(regles),
                     random.randrange(256), random.randrange(256),
                     random.randrange(256)))

# --- Le document -------------------------------------------------------------
#
# 1 500 elements environ, avec la distribution de classes d'une vraie page :
# beaucoup d'elements portent deux ou trois classes utilitaires, peu portent un
# identifiant.
corps = []
for bloc in range(120):
    composant = COMPOSANTS[bloc % len(COMPOSANTS)]
    variante = VARIANTES[bloc % len(VARIANTES)]
    classes = "%s %s--%s p-%d m-%d" % (composant, composant, variante,
                                       1 + bloc % 64, 1 + (bloc * 3) % 64)
    corps.append('<section class="%s">' % classes)
    corps.append('  <h3 class="titre t-%d">Bloc %d</h3>' % (1 + bloc % 64, bloc))
    corps.append('  <div class="%s__corps contenu">' % composant)
    for ligne in range(8):
        corps.append('    <p class="w-%d">Texte de remplissage numero %d '
                     'pour le bloc %d, assez long pour occuper une ligne '
                     'entiere et forcer un retour.</p>'
                     % (1 + ligne % 64, ligne, bloc))
    corps.append('    <ul>')
    for point in range(4):
        corps.append('      <li class="%s"><a href="#a%d">Lien %d</a></li>'
                     % ("actif" if point == 0 else "", point, point))
    corps.append('    </ul>')
    corps.append('    <label>Champ <input type="text" value="v"></label>')
    corps.append('    <button class="bouton bouton--primaire">Valider</button>')
    corps.append('  </div>')
    corps.append('</section>')

document = """<!doctype html>
<html><head><meta charset="utf-8"><title>Feuille lourde</title>
<style>
%s
</style></head>
<body>
%s
</body></html>
""" % ("\n".join(regles), "\n".join(corps))

with open(os.path.abspath(SORTIE), "w", encoding="utf-8") as sortie:
    sortie.write(document)
print("%s : %d regles, %d octets" % (os.path.abspath(SORTIE), len(regles),
                                     len(document)))
