#!/usr/bin/env python3
"""Une portee de domaine ne doit jamais enjamber une commutation de tache.

# Pourquoi cette regle existe

`src/kernel/sync/domaine.rs` attribue chaque prise du gros verrou au domaine le
plus interieur ouvert. La pile de portees est PAR CPU -- il le faut : le chemin
d'acquisition s'execute interruptions masquees et ne peut ni allouer ni dormir.

Une portee est donc valide tant que le code qu'elle couvre reste sur le meme
CPU et sur la meme pile. Des qu'elle enjambe une commutation, deux choses
faussent la mesure :

  * la tache reprend peut-etre sur un AUTRE coeur. Le `Drop` de la portee
    depile alors sur une pile qui ne l'a jamais empilee, et laisse l'entree sur
    le coeur d'origine ;
  * une commutation SANS RETOUR -- une tache demontee, une sortie definitive --
    ne fait jamais tourner le `Drop`. L'entree reste sur le CPU pour toujours,
    et toute acquisition ulterieure non etiquetee lui est attribuee. Si le
    domaine fuite est declare `Migre`, cela fabrique des REGRESSIONS qui
    n'existent pas.

Le second cas s'est produit pendant le chantier 1 : une portee `Processus`
posee dans `retire_current_if_zombie`, dont le chemin normal appelle
`retire_exec_zombie_current`, qui ne revient jamais.

# La regle

Une fonction qui ouvre une portee de domaine ne doit pas, dans la meme portee
lexicale, appeler une primitive qui commute ou qui peut bloquer.

Les exceptions sont NOMMEES ici, avec la raison qui les rend sures. Une
exception non listee est une faute.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
SRC = RACINE / "src"

PORTEE = re.compile(r"\bportee\(\s*(?:crate::kernel::sync::)?Domaine::(\w+)")

# Ce qui commute, ou qui peut commuter.
COMMUTE = re.compile(
    r"\b("
    r"switch_context|switch_to_kernel|schedule\(\)|schedule_sans_bkl|"
    r"park_current_on|park_current_on_until|finish_park_current_on_detached|"
    r"yield_now|wait_for_interrupt_releasing_bkl|"
    r"commute_sortie_definitive_si_possible|retire_exec_zombie_current|"
    r"wait_word_wait|sleep_ticks|safe_point\(\)"
    r")\b"
)

# Les exceptions, et pourquoi elles sont sures.
#
# `run` et `run_noyau` ouvrent leur portee `Processus` avant de lancer la
# premiere tache d'un programme, puis commutent vers elle. Leur pile est le
# contexte noyau du CPU appelant (`kernel_ctx()`), et c'est SUR CE MEME CPU que
# `switch_to_kernel` les fait reprendre : le `Drop` depile donc bien la pile qui
# a empile. La portee ne fuite pas, elle est seulement suspendue.
EXCEPTIONS = {
    ("src/kernel/process/thread/lifecycle.rs", "run"),
    ("src/kernel/process/thread/lifecycle.rs", "run_noyau"),
}

DEBUT_FONCTION = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:unsafe\s+)?(?:extern\s+\"[^\"]*\"\s+)?fn\s+(\w+)")


def fonctions(lignes: list[str]):
    """Decoupe grossierement en fonctions, par l'indentation de l'accolade.

    Un analyseur syntaxique complet serait plus juste ; il serait aussi une
    dependance de plus pour une regle qui se lit en trente lignes. Le decoupage
    par accolade de premier niveau suffit : les fonctions du noyau ne sont pas
    imbriquees dans d'autres fonctions au point de tromper ce compte.
    """
    courante = None
    debut = 0
    profondeur = 0
    for numero, ligne in enumerate(lignes):
        nu = ligne.split("//")[0]
        if courante is None:
            trouve = DEBUT_FONCTION.match(ligne)
            if trouve and "{" in nu:
                courante = trouve.group(1)
                debut = numero
                profondeur = nu.count("{") - nu.count("}")
                if profondeur <= 0:
                    courante = None
            continue
        profondeur += nu.count("{") - nu.count("}")
        if profondeur <= 0:
            yield courante, debut, numero
            courante = None


def main() -> int:
    fautes = []
    portees = 0
    for chemin in sorted(SRC.rglob("*.rs")):
        relatif = chemin.relative_to(RACINE).as_posix()
        lignes = chemin.read_text(encoding="utf-8", errors="replace").split("\n")
        utiles = [l if not l.lstrip().startswith("//") else "" for l in lignes]
        if not any(PORTEE.search(l) for l in utiles):
            continue
        for nom, debut, fin in fonctions(lignes):
            corps = utiles[debut:fin + 1]
            domaines = [PORTEE.search(l).group(1) for l in corps if PORTEE.search(l)]
            if not domaines:
                continue
            portees += len(domaines)
            if (relatif, nom) in EXCEPTIONS:
                continue
            for numero, ligne in enumerate(corps):
                trouve = COMMUTE.search(ligne)
                if trouve:
                    fautes.append(
                        f"  {relatif}:{debut + numero + 1}  `{nom}` ouvre la "
                        f"portee `{domaines[0]}` et appelle `{trouve.group(1)}` : "
                        f"la portee enjamberait une commutation, et la pile de "
                        f"domaines est PAR CPU"
                    )
                    break

    if fautes:
        print("portees de domaine : regle violee")
        print("\n".join(fautes))
        return 1

    print(
        f"ok  {portees} portee(s) de domaine, aucune n'enjambe une commutation "
        f"({len(EXCEPTIONS)} exception(s) nommee(s))"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
