#!/usr/bin/env python3
"""Fabrique les assets Atelier (BOUCHAUD_ATELIER_ASSETS_V1).

Sources (versionnees) -> ressources pretes pour le noyau (versionnees aussi) :

  assets/atelier/sources/lucide/*.svg   sous-ensemble Lucide (ISC), tel quel
  assets/atelier/sources/logo/*.svg     marque Bouchaud OS (provisoire)
  assets/atelier/sources/fond/*.jpg     photographie du fond (LGPLv3, KDE)
  APPS (ci-dessous)                     tuiles applicatives : degrade + glyphe Lucide
  sources/apps/doom.svg                 logo officiel DOOM + tuile Atelier

  src/assets/atelier/apps/<id>-<taille>.png       tuiles, RGBA
  src/assets/atelier/symboles/<nom>-<taille>.png  glyphes blancs + alpha (masques teintables)
  src/assets/atelier/logo/bouchaud-os-<taille>.png
  src/assets/atelier/fond/<nom>-<l>x<h>.png       fond recadre « cover », RGB
  src/assets/atelier/MANIFEST.json                empreintes, tailles, licences

Chaque taille est RENDUE depuis le vecteur (resvg), jamais reduite depuis une
grande image : un glyphe de 18 px garde des traits nets. La CI ne relance pas
cette fabrique (elle demande resvg) : `tools/verifie-assets-atelier.py` verifie
que les sorties correspondent au manifeste et que chaque source a sa licence.

Dependances de DEVELOPPEMENT : `pip install resvg-py pillow`.
"""
import hashlib
import io
import json
import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[2]
SOURCES = RACINE / "assets/atelier/sources"
SORTIE = RACINE / "src/assets/atelier"

# Une tuile : (identifiant, glyphe Lucide, degrade haut, degrade bas, encre du glyphe)
APPS = [
    ("navigateur",   "compass",                "#6f93ff", "#5546dc", "#ffffff"),
    ("calculatrice", "calculator",             "#a3abc0", "#5d6580", "#ffffff"),
    ("terminal",     "terminal",               "#3c425d", "#161927", "#ffffff"),
    ("fichiers",     "folder",                 "#68b6ff", "#2c78f2", "#ffffff"),
    ("edition",      "file-pen-line",          "#ffffff", "#e6e9f3", "#4b57d6"),
    ("services",     "server-cog",             "#62d6a0", "#1c9a5e", "#ffffff"),
    ("notes",        "notebook-pen",           "#ffd96e", "#f3a21f", "#5b3b00"),
    ("insights",     "chart-no-axes-combined", "#a091ff", "#6950dd", "#ffffff"),
    ("moniteur",     "activity",               "#ff9c7c", "#e5534a", "#ffffff"),
    ("journal",      "scroll-text",            "#7fd5ea", "#2b97bd", "#ffffff"),
    ("parametres",   "settings",               "#bcc2d0", "#666d82", "#ffffff"),
]
TAILLES_APPS = (18, 40, 56)
SYMBOLES = [
    ("layout-grid", (22,)),
    # Barre haute (16 px) et centre de controle (18 px). L'etat du reseau se
    # lit par la FORME du glyphe, pas seulement par sa teinte.
    ("ethernet-port", (16, 18)), ("unplug", (16, 18)), ("circle-alert", (16, 18)),
    ("sliders-horizontal", (16,)),
    ("power", (18,)), ("rotate-ccw", (18,)), ("log-out", (18,)),
    ("activity", (18,)), ("monitor", (18,)),
    # Fichiers Atelier (increment D) : barre laterale, navigation, menu.
    ("house", (16,)), ("file-text", (16,)), ("image", (16,)), ("download", (16,)),
    ("hard-drive", (16,)), ("folder", (16,)), ("layout-grid", (16,)), ("list", (16,)),
    ("chevron-left", (16,)), ("chevron-right", (16,)), ("arrow-up", (16,)),
    ("copy", (16,)),
]

# Icones de types de fichiers (increment D) : (identifiant, glyphe Lucide du
# badge, teinte du badge). Le dossier a sa propre forme.
TYPES_FICHIERS = [
    ("fichier",    "file",      "#8b90a3"),
    ("texte",      "file-text", "#4b7bec"),
    ("image",      "image",     "#22a06b"),
    ("code",       "code",      "#7c5cff"),
    ("archive",    "archive",   "#e08a12"),
    ("executable", "binary",    "#3c425d"),
]
TAILLES_FICHIERS = (20, 48)
TAILLES_LOGO = (18, 72)
FONDS = [("summer_1am", "summer_1am-2560x1600.jpg", ((1280, 720), (1920, 1080)))]


def glyphe(nom: str) -> str:
    """Les elements internes d'un SVG Lucide, sans son enveloppe."""
    texte = (SOURCES / "lucide" / f"{nom}.svg").read_text(encoding="utf-8")
    corps = re.search(r"<svg[^>]*>(.*)</svg>", texte, re.S).group(1)
    return re.sub(r"<!--.*?-->", "", corps, flags=re.S).strip()


def tuile(nom_glyphe: str, haut: str, bas: str, encre: str) -> str:
    # Grille 256 : carre arrondi de 240 (rayon 56, ~23 %), glyphe dans le carre
    # central de 128 (echelle 128/24), trait 1.75 en unites Lucide.
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">
  <defs>
    <linearGradient id="g" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="{haut}"/><stop offset="1" stop-color="{bas}"/>
    </linearGradient>
    <linearGradient id="h" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#ffffff" stop-opacity="0.30"/>
      <stop offset="0.55" stop-color="#ffffff" stop-opacity="0"/>
    </linearGradient>
  </defs>
  <rect x="8" y="8" width="240" height="240" rx="56" fill="url(#g)"/>
  <rect x="8" y="8" width="240" height="240" rx="56" fill="url(#h)"/>
  <rect x="8.75" y="8.75" width="238.5" height="238.5" rx="55.25" fill="none"
        stroke="#1d2130" stroke-opacity="0.10" stroke-width="1.5"/>
  <g transform="translate(64 64) scale(5.3333)" fill="none" stroke="{encre}"
     stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round">
    {glyphe(nom_glyphe)}
  </g>
</svg>"""


def symbole(nom: str) -> str:
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none"
  stroke="#ffffff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
  {glyphe(nom)}
</svg>"""


def dossier() -> str:
    # Grille 256 : languette arriere plus sombre, corps avant en degrade, un
    # liseré clair en haut du corps. Meme bleu que la tuile Fichiers.
    return """<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">
  <defs>
    <linearGradient id="c" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#7cc0ff"/><stop offset="1" stop-color="#3d86f4"/>
    </linearGradient>
  </defs>
  <path d="M20 58 a18 18 0 0 1 18 -18 h56 a18 18 0 0 1 13 5.5 l15 15.5 h96 a18 18 0 0 1 18 18 v28 h-216 z"
        fill="#2c6fd8"/>
  <rect x="20" y="78" width="216" height="146" rx="20" fill="url(#c)"/>
  <rect x="20" y="78" width="216" height="146" rx="20" fill="none" stroke="#1d2130"
        stroke-opacity="0.10" stroke-width="2"/>
  <path d="M40 80 h176" stroke="#ffffff" stroke-opacity="0.45" stroke-width="3" stroke-linecap="round"/>
</svg>"""


def document(nom_glyphe: str, teinte: str) -> str:
    # Page blanche a coin replie, filet discret, badge de type en bas a
    # gauche portant le glyphe Lucide en blanc.
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">
  <path d="M58 16 h100 l58 58 v148 a18 18 0 0 1 -18 18 h-140 a18 18 0 0 1 -18 -18 v-188 a18 18 0 0 1 18 -18 z"
        fill="#ffffff" stroke="#1d2130" stroke-opacity="0.16" stroke-width="4"/>
  <path d="M158 16 v40 a18 18 0 0 0 18 18 h40" fill="#eef0f6" stroke="#1d2130"
        stroke-opacity="0.16" stroke-width="4" stroke-linejoin="round"/>
  <rect x="62" y="122" width="112" height="112" rx="26" fill="{teinte}"/>
  <g transform="translate(78 138) scale(3.3333)" fill="none" stroke="#ffffff"
     stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    {glyphe(nom_glyphe)}
  </g>
</svg>"""


def glyphe_colore(nom: str, teinte: str) -> str:
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none"
  stroke="{teinte}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
  {glyphe(nom)}
</svg>"""


def rend(svg: str, taille: int) -> bytes:
    import resvg_py
    from PIL import Image
    brut = bytes(resvg_py.svg_to_bytes(svg_string=svg, width=taille, height=taille))
    # Reencode par Pillow : PNG RGBA 8 bits standard, sans fragments exotiques
    # que le decodeur du noyau n'a pas a connaitre.
    im = Image.open(io.BytesIO(brut)).convert("RGBA")
    assert im.size == (taille, taille), im.size
    sortie = io.BytesIO()
    im.save(sortie, format="PNG", optimize=True)
    return sortie.getvalue()


def fond(source: Path, largeur: int, hauteur: int) -> bytes:
    from PIL import Image
    im = Image.open(source).convert("RGB")
    sl, sh = im.size
    # « cover » centre : on remplit l'ecran, on coupe ce qui deborde.
    echelle = max(largeur / sl, hauteur / sh)
    l, h = round(sl * echelle), round(sh * echelle)
    im = im.resize((l, h), Image.LANCZOS)
    x, y = (l - largeur) // 2, (h - hauteur) // 2
    im = im.crop((x, y, x + largeur, y + hauteur))
    sortie = io.BytesIO()
    im.save(sortie, format="PNG", optimize=True)
    return sortie.getvalue()


def ecrit(chemin: Path, octets: bytes, manifeste: dict, **meta):
    chemin.parent.mkdir(parents=True, exist_ok=True)
    chemin.write_bytes(octets)
    rel = chemin.relative_to(RACINE).as_posix()
    manifeste["fichiers"][rel] = {"sha256": hashlib.sha256(octets).hexdigest(),
                                  "octets": len(octets), **meta}


def main() -> int:
    manifeste = {
        "schema": "bouchaud-atelier-assets/1",
        "licences": {
            "lucide": {"licence": "ISC", "source": "lucide-static 1.67.0 (npm)",
                       "texte": "assets/atelier/sources/lucide/LICENSE"},
            "fond-summer_1am": {"licence": "LGPL-3.0", "auteur": "Risto Saukonpää",
                                "source": "KDE/plasma-workspace-wallpapers summer_1am",
                                "metadata": "assets/atelier/sources/fond/metadata.json"},
            "logo-bouchaud-os": {"licence": "projet Bouchaud OS", "statut": "PROVISOIRE",
                                 "source": "assets/atelier/sources/logo/bouchaud-os-provisoire.svg"},
            "tuiles-apps": {"licence": "projet Bouchaud OS (tuiles) + ISC (glyphes Lucide)"},
            "logo-doom": {
                "licence": "PD-textlogo (Wikimedia Commons), marque deposee",
                "auteur": "id Software / Bethesda Softworks",
                "source": "https://commons.wikimedia.org/wiki/File:Doom_%E2%80%93_Game%E2%80%99s_logo.svg",
                "original": "assets/atelier/sources/apps/doom-logo-officiel.svg",
                "tuile": "assets/atelier/sources/apps/doom.svg"
            },
            "icones-fichiers": {"licence": "projet Bouchaud OS (formes) + ISC (glyphes Lucide)"},
        },
        "fichiers": {},
    }
    for ident, nom, haut, bas, encre in APPS:
        svg = tuile(nom, haut, bas, encre)
        (SOURCES / "apps").mkdir(parents=True, exist_ok=True)
        (SOURCES / "apps" / f"{ident}.svg").write_text(svg + "\n", encoding="utf-8")
        for t in TAILLES_APPS:
            ecrit(SORTIE / "apps" / f"{ident}-{t}.png", rend(svg, t), manifeste,
                  largeur=t, hauteur=t, alpha=True, licence="tuiles-apps", glyphe=nom)
    # DOOM : veritable silhouette du logo officiel, pas un glyphe de substitution.
    # La tuile vectorielle a une source accompagnee de son attribution.
    doom_svg = (SOURCES / "apps" / "doom.svg").read_text(encoding="utf-8")
    for taille in TAILLES_APPS:
        ecrit(SORTIE / "apps" / f"doom-{taille}.png", rend(doom_svg, taille), manifeste,
              largeur=taille, hauteur=taille, alpha=True,
              licence="logo-doom", glyphe="lettrage officiel DOOM")
    (SOURCES / "fichiers").mkdir(parents=True, exist_ok=True)
    # En 48 px : page + badge. En 20 px, un badge serait illisible : c'est le
    # glyphe Lucide du type, trace dans sa teinte, qui porte l'information.
    formes = [("dossier", dossier(), dossier(), "folder")] + [
        (ident, document(nom, teinte), glyphe_colore(nom, teinte), nom)
        for ident, nom, teinte in TYPES_FICHIERS]
    for ident, grand, petit, nom in formes:
        (SOURCES / "fichiers" / f"{ident}.svg").write_text(grand + "\n", encoding="utf-8")
        for t in TAILLES_FICHIERS:
            svg = grand if t >= 32 else petit
            ecrit(SORTIE / "fichiers" / f"{ident}-{t}.png", rend(svg, t), manifeste,
                  largeur=t, hauteur=t, alpha=True, licence="icones-fichiers", glyphe=nom)
    for nom, tailles in SYMBOLES:
        for t in tailles:
            ecrit(SORTIE / "symboles" / f"{nom}-{t}.png", rend(symbole(nom), t), manifeste,
                  largeur=t, hauteur=t, alpha=True, licence="lucide")
    logo = (SOURCES / "logo" / "bouchaud-os-provisoire.svg").read_text(encoding="utf-8")
    for t in TAILLES_LOGO:
        ecrit(SORTIE / "logo" / f"bouchaud-os-{t}.png", rend(logo, t), manifeste,
              largeur=t, hauteur=t, alpha=True, licence="logo-bouchaud-os")
    for nom, fichier, tailles in FONDS:
        for (l, h) in tailles:
            ecrit(SORTIE / "fond" / f"{nom}-{l}x{h}.png", fond(SOURCES / "fond" / fichier, l, h),
                  manifeste, largeur=l, hauteur=h, alpha=False, licence=f"fond-{nom}")
    (SORTIE / "MANIFEST.json").write_text(json.dumps(manifeste, indent=2, ensure_ascii=False) + "\n",
                                          encoding="utf-8")
    total = sum(f["octets"] for f in manifeste["fichiers"].values())
    print(f"ATELIER_ASSETS_FABRIQUES fichiers={len(manifeste['fichiers'])} octets={total}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
