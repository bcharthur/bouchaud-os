#!/usr/bin/env python3
"""Le navigateur ne doit plus juger ses polices au COMPTE.

# Le defaut

`assurePolices()` se contentait de `if (familles > 0) return;`. Qt rend
toujours au moins une famille — son repli interne, une bitmap ASCII sans
aucune lettre accentuee. Le compte etait donc satisfait, DejaVu n'etait jamais
chargee, et toute page s'affichait dans cette bitmap : des lettres carrees, et
un carre vide a la place de chaque lettre accentuee.

Rien ne le signalait. Le journal disait meme « N familles trouvees par Qt ».

# Ce que ce garde-fou exige

`hote.cpp` ne peut pas etre compile ici — il demande Qt, Python et brotli, et
il se construit sur la machine cible. Ces quatre regles sont donc verifiees sur
la source, la ou elles se lisent :

  1. aucune sortie anticipee sur un simple COMPTE de familles ;
  2. la presence de la famille de repli est verifiee par son NOM ;
  3. `fabriqueFonte` ne pose une famille demandee par la page que si Qt la
     connait — sinon Qt substitue son repli interne, et on revient au carre ;
  4. `bo.police()` invalide le cache des familles, faute de quoi la police que
     la page vient de livrer serait jugee inconnue et jamais utilisee.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
HOTE = RACINE / "tools" / "userland" / "navigateur" / "hote.cpp"


def corps(source: str, signature: str) -> str:
    """Le corps de la fonction dont la ligne commence par `signature`."""
    debut = source.index(signature)
    profondeur = 0
    for position in range(debut, len(source)):
        if source[position] == "{":
            profondeur += 1
        elif source[position] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[debut : position + 1]
    raise SystemExit(f"corps introuvable : {signature}")


def main() -> int:
    if not HOTE.exists():
        print(f"ok  {HOTE.name} absent, rien a verifier")
        return 0
    source = HOTE.read_text(encoding="utf-8")
    fautes = []

    assure = corps(source, "void assurePolices()")
    if re.search(r"familles\s*>\s*0\s*\)\s*\{?\s*[^}]*return", assure):
        fautes.append(
            "  assurePolices : sortie anticipee sur un COMPTE de familles. "
            "Qt en rend toujours au moins une — son repli bitmap ASCII — donc "
            "le compte ne prouve rien. Demander la famille par son nom."
        )
    if "familleConnue(" not in assure:
        fautes.append(
            "  assurePolices : la presence de la police de repli doit etre "
            "verifiee par son NOM (`familleConnue`)"
        )
    if "insertSubstitution" not in assure:
        fautes.append(
            "  assurePolices : sans substitution des familles generiques du "
            "web, une page qui demande « Google Sans » retombe sur le repli "
            "interne de Qt, pas sur DejaVu"
        )

    fonte = corps(source, "QFont fabriqueFonte(")
    if "familleConnue(famille)" not in fonte:
        fautes.append(
            "  fabriqueFonte : une famille inconnue posee telle quelle laisse "
            "Qt substituer son repli interne. La resoudre ici, ou l'on sait ce "
            "que la base contient."
        )
    if "FAMILLE_REPLI" not in fonte:
        fautes.append(
            "  fabriqueFonte : aucune famille de repli explicite"
        )

    police = corps(source, "PyObject *bo_police(")
    if "oublieFamilles()" not in police:
        fautes.append(
            "  bo_police : le cache des familles doit etre invalide, sinon la "
            "police que la page vient de livrer est jugee inconnue et jamais "
            "utilisee"
        )

    if fautes:
        print("polices du navigateur : regle violee")
        print("\n".join(fautes))
        return 1

    print(
        "ok  hote.cpp : les polices sont verifiees par leur nom, les familles "
        "generiques substituees, et le cache invalide a chaque @font-face"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
