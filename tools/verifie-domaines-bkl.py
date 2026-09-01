#!/usr/bin/env python3
"""Toute prise du gros verrou doit dire a quel chemin elle appartient.

# Ce que ce garde-fou protege

La sortie du gros verrou ne se fait pas en une fois : elle se fait chemin par
chemin, et un chemin sorti doit ensuite le RESTER. Un total d'acquisitions ne
permet ni l'un ni l'autre. Il dit qu'il y en a beaucoup ; il ne dit pas
lesquelles restent legitimes, ni laquelle vient de reapparaitre.

`src/kernel/sync/domaine.rs` attribue chaque acquisition au chemin le plus
interieur ouvert sur ce CPU. L'attribution ne vaut que si elle est COMPLETE :
une seule acquisition non rattachee et le chiffre « chemins normaux » devient
un minorant, donc inutilisable comme critere.

# Les trois regles

  1. Chaque acquisition du gros verrou dans le code de production est
     precedee d'une portee de domaine.
  2. Un fichier dont le domaine est declare `Migre` ne contient AUCUNE
     acquisition : sinon le contrat est faux des la compilation, sans meme
     avoir besoin d'executer.
  3. Le chemin d'acquisition appelle bien `note_acquisition` : sans cela, tout
     le reste est du decor.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
SRC = RACINE / "src"
DOMAINE = SRC / "kernel" / "sync" / "domaine.rs"
METRIQUES = SRC / "kernel" / "sync" / "bkl" / "metriques.rs"

# Le verrou lui-meme, et la colle qui l'instrumente, ne s'attribuent pas : ils
# SONT le mecanisme. Les y obliger serait une recursion, pas une regle.
EXEMPTS = {
    "src/kernel/sync/bkl.rs",
    "src/kernel/sync/mod.rs",
}
EXEMPTS_PREFIXES = ("src/kernel/sync/bkl/",)

ACQUISITION = re.compile(r"\bsmp_lock::(?:enter|try_enter|try_enter_depuis_zero)\(\)")
PORTEE = re.compile(r"\bportee\(\s*(?:crate::kernel::sync::)?Domaine::(\w+)")


def exempt(relatif: str) -> bool:
    return relatif in EXEMPTS or relatif.startswith(EXEMPTS_PREFIXES)


def domaines_migres(source: str) -> set[str]:
    """Les domaines dont le contrat est `Migre`, lus dans `domaine.rs`."""
    bloc = re.search(r"Contrat::Migre\s*,", source)
    if not bloc:
        return set()
    # La forme est : `Self::A | Self::B => Contrat::Migre,`
    ligne = re.search(r"((?:Self::\w+\s*\|?\s*)+)=>\s*Contrat::Migre", source)
    if not ligne:
        return set()
    return set(re.findall(r"Self::(\w+)", ligne.group(1)))


def main() -> int:
    fautes = []
    if not DOMAINE.exists():
        print(f"introuvable : {DOMAINE}")
        return 1
    source_domaine = DOMAINE.read_text(encoding="utf-8")
    migres = domaines_migres(source_domaine)
    if not migres:
        fautes.append(
            "  domaine.rs  aucun domaine n'est declare `Migre` : le chantier "
            "n'a alors rien a prouver, et la metrique ne verifie rien"
        )

    # --- 3. le chemin d'acquisition compte-t-il encore ? --------------------
    metriques = METRIQUES.read_text(encoding="utf-8") if METRIQUES.exists() else ""
    if "note_acquisition(" not in metriques:
        fautes.append(
            "  bkl/metriques.rs  `probe_note_acquire` n'attribue plus les "
            "acquisitions : les compteurs par domaine resteraient a zero, ce "
            "qui se lit exactement comme un succes"
        )

    total_sites = 0
    for chemin in sorted(SRC.rglob("*.rs")):
        relatif = chemin.relative_to(RACINE).as_posix()
        if exempt(relatif):
            continue
        lignes = chemin.read_text(encoding="utf-8", errors="replace").split("\n")
        domaine_du_fichier = set()
        for numero, ligne in enumerate(lignes):
            nu = ligne.lstrip()
            if nu.startswith("//"):
                continue
            trouve = PORTEE.search(ligne)
            if trouve:
                domaine_du_fichier.add(trouve.group(1))
            if not ACQUISITION.search(ligne):
                continue
            total_sites += 1
            # --- 1. une portee juste au-dessus -----------------------------
            precedentes = lignes[max(0, numero - 3):numero]
            if not any(PORTEE.search(l) for l in precedentes):
                fautes.append(
                    f"  {relatif}:{numero + 1}  prend le gros verrou sans "
                    f"portee de domaine : cette acquisition serait comptee "
                    f"« indetermine », et le total des chemins normaux "
                    f"deviendrait un minorant"
                )
        # --- 2. un domaine migre ne prend pas le verrou ---------------------
        for nom in domaine_du_fichier & migres:
            if any(ACQUISITION.search(l) for l in lignes
                   if not l.lstrip().startswith("//")):
                fautes.append(
                    f"  {relatif}  porte le domaine `{nom}`, declare SORTI du "
                    f"gros verrou, et le prend pourtant : le contrat est faux "
                    f"des la compilation"
                )

    if fautes:
        print("domaines du gros verrou : regle violee")
        print("\n".join(fautes))
        return 1

    print(
        f"ok  {total_sites} acquisition(s) du gros verrou, toutes attribuees a "
        f"un domaine ; {len(migres)} domaine(s) declare(s) sorti(s) : "
        f"{', '.join(sorted(migres))}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
