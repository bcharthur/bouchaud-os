#!/usr/bin/env python3
"""Les assets Atelier sont traces, licencies et minimaux (BOUCHAUD_ATELIER_ASSETS_V1).

Regles :
  1. chaque fichier de `src/assets/atelier/` est au manifeste, avec la meme
     empreinte et la meme taille -- une image remplacee a la main se voit ;
  2. chaque entree du manifeste designe une licence declaree, dont le texte
     ou la provenance est present dans `assets/atelier/sources/` ;
  3. le logo est marque PROVISOIRE tant qu'aucune identite n'est validee ;
  4. chaque `include_bytes!` du noyau vers `assets/atelier/` vise un fichier du
     manifeste ;
  5. seul un SOUS-ENSEMBLE de Lucide est versionne (pas la bibliotheque), et
     chaque glyphe porte son en-tete de licence ;
  6. `assets/atelier/ATTRIBUTIONS.md` cite chaque auteur et chaque licence.

Fail-closed ; negatifs par mutation.
"""
import hashlib
import json
import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
SORTIE = RACINE / "src/assets/atelier"
SOURCES = RACINE / "assets/atelier/sources"
LUCIDE_MAX = 40


def lit(chemin: Path) -> bytes:
    return chemin.read_bytes()


def etat_disque() -> dict:
    fichiers = {p.relative_to(RACINE).as_posix(): lit(p) for p in SORTIE.rglob("*")
                if p.is_file() and p.name != "MANIFEST.json"}
    lucide = {p.name: p.read_text(encoding="utf-8") for p in (SOURCES / "lucide").glob("*.svg")}
    code = "\n".join(p.read_text(encoding="utf-8") for p in (RACINE / "src/gui").rglob("*.rs"))
    return {
        "manifeste": json.loads((SORTIE / "MANIFEST.json").read_text(encoding="utf-8")),
        "fichiers": fichiers,
        "lucide": lucide,
        "licence_lucide": (SOURCES / "lucide/LICENSE").is_file(),
        "licence_fond": (SOURCES / "fond/LICENSE-LGPL-3.0.txt").is_file()
                        and (SOURCES / "fond/metadata.json").is_file(),
        "logo": (SOURCES / "logo/bouchaud-os-provisoire.svg").read_text(encoding="utf-8"),
        "attributions": (RACINE / "assets/atelier/ATTRIBUTIONS.md").read_text(encoding="utf-8"),
        "code": code,
    }


def verifie(t: dict) -> list:
    f = []
    m = t["manifeste"]
    entrees = m.get("fichiers", {})
    # 1
    for rel, octets in t["fichiers"].items():
        e = entrees.get(rel)
        if e is None:
            f.append(f"{rel} : absent du manifeste")
        elif e.get("sha256") != hashlib.sha256(octets).hexdigest() or e.get("octets") != len(octets):
            f.append(f"{rel} : empreinte ou taille differente du manifeste")
    for rel in entrees:
        if rel not in t["fichiers"]:
            f.append(f"{rel} : au manifeste mais absent du disque")
    # 2
    licences = m.get("licences", {})
    for rel, e in entrees.items():
        if e.get("licence") not in licences:
            f.append(f"{rel} : licence non declaree ({e.get('licence')})")
    if not t["licence_lucide"]:
        f.append("licence Lucide (ISC) absente")
    if not t["licence_fond"]:
        f.append("licence LGPL-3.0 ou metadonnees KDE du fond absentes")
    # 3
    if "PROVISOIRE" not in t["logo"] or licences.get("logo-bouchaud-os", {}).get("statut") != "PROVISOIRE":
        f.append("logo : n'est plus marque PROVISOIRE")
    # 4
    for chemin in re.findall(r'include_bytes!\("\.\./(?:\.\./)?assets/atelier/([^"]+)"\)', t["code"]):
        if f"src/assets/atelier/{chemin}" not in entrees:
            f.append(f"include_bytes vers {chemin} : hors manifeste")
    for nom in re.findall(r'tuile!\("([a-z_]+)"\)', t["code"]):
        for taille in ("18", "40", "56"):
            if f"src/assets/atelier/apps/{nom}-{taille}.png" not in entrees:
                f.append(f"tuile {nom}-{taille} : hors manifeste")
    # 5
    if len(t["lucide"]) > LUCIDE_MAX:
        f.append(f"Lucide : {len(t['lucide'])} glyphes versionnes (> {LUCIDE_MAX}) -- seulement le sous-ensemble utilise")
    for nom, svg in t["lucide"].items():
        if "@license lucide-static" not in svg:
            f.append(f"Lucide {nom} : en-tete de licence absent")
    # 6
    for mot in ("Lucide", "ISC", "Risto Saukonp", "LGPL-3.0", "PROVISOIRE"):
        if mot not in t["attributions"]:
            f.append(f"ATTRIBUTIONS.md : « {mot} » absent")
    return f


def main() -> int:
    t = etat_disque()
    fautes = verifie(t)
    un = next(iter(t["fichiers"]), None)
    mutations = []
    if un:
        mutations.append(("image remplacee a la main", lambda x: x["fichiers"].__setitem__(un, x["fichiers"][un] + b"\0")))
        mutations.append(("fichier hors manifeste", lambda x: x["fichiers"].__setitem__("src/assets/atelier/apps/intrus.png", b"x")))
        mutations.append(("licence non declaree", lambda x: x["manifeste"]["fichiers"][un].__setitem__("licence", "inconnue")))
    mutations += [
        ("logo declare definitif", lambda x: x.__setitem__("logo", x["logo"].replace("PROVISOIRE", "officiel"))),
        ("licence Lucide retiree", lambda x: x.__setitem__("licence_lucide", False)),
        ("bibliotheque entiere", lambda x: x["lucide"].update({f"g{i}.svg": "<!-- @license lucide-static -->" for i in range(64)})),
        ("attribution du fond retiree", lambda x: x.__setitem__("attributions", x["attributions"].replace("Risto Saukonp", "X"))),
        ("include hors manifeste", lambda x: x.__setitem__("code", x["code"] + '\ninclude_bytes!("../../assets/atelier/fond/autre.png")')),
    ]
    for nom, muter in mutations:
        m = json.loads(json.dumps({k: v for k, v in t.items() if k != "fichiers"}))
        m["fichiers"] = dict(t["fichiers"])
        muter(m)
        if not verifie(m):
            fautes.append(f"mutation non detectee : {nom}")
    for faute in fautes:
        print(f"ECHEC {faute}")
    if fautes:
        return 1
    print(f"assets atelier : ok ({len(t['fichiers'])} fichiers, {len(t['lucide'])} glyphes Lucide, {len(mutations)} mutations detectees)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
