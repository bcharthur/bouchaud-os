#!/usr/bin/env python3
"""La matrice materielle ne doit pas declarer ce qu'aucun test ne verifie.

# Ce que ce garde-fou protege

Un tableau materiel se remplit de bonnes intentions plus vite que n'importe
quel autre. « NVMe : prevu » ne coute rien a ecrire, et fait croire a un support
qui n'existe pas -- jusqu'au jour ou quelqu'un branche un disque en se fiant au
tableau.

La regle du depot est deja ecrite dans `docs/PORTABILITY_MATRIX.md` : aucune
valeur n'est inventee, chaque ligne renvoie a une verification. Ce script la
rend executable pour la section materielle du chantier 10 :

  1. toute ligne dont l'etat commence par `oui` doit NOMMER une verification ;
  2. une ligne sans verification doit porter `absent`, `inconnu` ou `partiel` --
     jamais `oui` ;
  3. les fichiers de test cites doivent EXISTER. Une matrice qui renvoie a un
     test supprime est pire qu'une matrice vide : elle a l'air verifiee.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
MATRICE = RACINE / "docs" / "PORTABILITY_MATRIX.md"
TITRE = "## Matrice materielle"

ETATS_SANS_PREUVE = ("absent", "inconnu", "a mesurer", "—", "-")
FICHIER = re.compile(r"`([A-Za-z0-9_./-]+\.(?:rs|py|sh))`")


def lignes_de_la_section(texte: str) -> list[str]:
    debut = texte.find(TITRE)
    if debut < 0:
        return []
    section = texte[debut:]
    return [ligne for ligne in section.split("\n") if ligne.startswith("|")]


def main() -> int:
    if not MATRICE.exists():
        print(f"introuvable : {MATRICE}")
        return 1

    texte = MATRICE.read_text(encoding="utf-8")
    lignes = lignes_de_la_section(texte)
    if not lignes:
        print(f"section « {TITRE} » introuvable dans PORTABILITY_MATRIX.md")
        return 1

    fautes = []
    verifiees = 0
    for ligne in lignes:
        cellules = [c.strip() for c in ligne.strip("|").split("|")]
        if len(cellules) < 3 or cellules[0].startswith("---") or cellules[0] == "Materiel":
            continue
        materiel, etat, preuve = cellules[0], cellules[1], cellules[2]
        etat_nu = etat.replace("*", "").strip().lower()
        preuve_nue = preuve.replace("*", "").strip()

        sans_preuve = preuve_nue in ("", "—", "-")
        if etat_nu.startswith("oui") and sans_preuve:
            fautes.append(
                f"  « {materiel} » est declare `{etat}` sans nommer de "
                f"verification : c'est exactement la ligne qui fait croire a un "
                f"support qui n'existe pas"
            )
            continue

        if sans_preuve:
            if not any(etat_nu.startswith(mot) for mot in ETATS_SANS_PREUVE):
                fautes.append(
                    f"  « {materiel} » n'a pas de verification : son etat doit "
                    f"etre `absent`, `inconnu` ou `partiel`, pas `{etat}`"
                )
            continue

        for cite in FICHIER.findall(preuve):
            candidats = list(RACINE.rglob(cite.split("/")[-1]))
            if not candidats:
                fautes.append(
                    f"  « {materiel} » renvoie a `{cite}`, qui n'existe pas : "
                    f"une matrice qui cite un test supprime a l'air verifiee "
                    f"sans l'etre"
                )
        verifiees += 1

    if fautes:
        print("matrice materielle : regle violee")
        print("\n".join(fautes))
        return 1

    print(f"ok  {verifiees} ligne(s) materielles nomment leur verification")
    return 0


if __name__ == "__main__":
    sys.exit(main())
