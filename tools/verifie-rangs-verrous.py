#!/usr/bin/env python3
"""Verifie l'ordre des verrous RANGES du noyau.

LA REGLE
--------
`src/kernel/sync/lockdep.rs` donne un rang a chaque classe de verrou. On ne
peut prendre un verrou que si son rang est STRICTEMENT SUPERIEUR au plus haut
rang deja tenu. Un ordre total interdit tout cycle, donc tout interblocage par
inversion. Deux verrous de MEME rang sont refuses eux aussi : deux objets de la
meme classe pris ensemble sont un cycle des qu'un autre chemin les prend dans
l'autre sens.

POURQUOI CE VERIFICATEUR EXISTE
-------------------------------
`lockdep` fait deja ce controle a l'execution -- mais seulement si le chemin
fautif est REELLEMENT PARCOURU, sur la bonne machine, dans le bon
entrelacement. C'est precisement ce qu'un interblocage par inversion ne
garantit jamais.

Et le depot en a fait l'experience. Les rangs `FdTable` et `Vfs` etaient
declares depuis le debut, mais AUCUN verrou ne les portait : la table des
descripteurs et le RAMFS etaient de simples `SpinLock`. Pendant ce temps
`write`, `openat` et `getdents` tenaient le RAMFS pendant qu'ils reportaient un
offset dans la table des descripteurs -- l'ordre exactement inverse de celui
que `lockdep` declarait. Seul le gros verrou empechait les deux sens de se
croiser. La declaration etait juste ; rien ne la reliait au code.

Ce verificateur lit la source et ferme cet ecart en deux points :

  1. il refuse toute inversion visible statiquement ;
  2. il suit les classes qui ne portent encore AUCUN verrou, pour qu'un rang
     declare ne puisse pas rester decoratif sans que cela se voie.

CE QU'IL NE PEUT PAS VOIR
-------------------------
La duree de vie d'un garde TEMPORAIRE : dans `path_string(&fs(), n)` le garde
meurt a la fin de l'instruction. Le verificateur ne considere donc comme tenu
qu'un garde dont l'expression EST la prise (`let fs = ramfs::fs();`). Une prise
enfouie dans une expression plus grande est comptee comme instantanee -- ce qui
est vrai, mais rend la surete dependante d'une regle sur les temporaires.

Il ne suit pas non plus un garde passe a une fonction : l'appelee peut prendre
un verrou de rang inferieur sans que cela se lise ici. C'est `lockdep`, au
runtime, qui couvre ce cas.

Code de retour : 0 si l'ordre est respecte.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
LOCKDEP = RACINE / "src" / "kernel" / "sync" / "lockdep.rs"
SOURCES = RACINE / "src"

# Comment chaque verrou range se lit DANS LA SOURCE.
#
# Cette table est le pont entre une classe declaree et le code qui la porte.
# Chaque entree est verifiee ci-dessous contre une declaration
# `RankedSpinLock::new(LockClass::…)` reelle : une classe qui perdrait son
# verrou rendrait ce fichier rouge au lieu de le rendre silencieux.
ACCES = {
    "SchedulerTransition": re.compile(r"\bcommence_transition_ordonnanceur\s*\(\)"),
    "SchedulerAlarms": re.compile(r"\bALARMES\s*\.\s*lock\s*\(\)"),
    "FdTable": re.compile(r"[\w.]*\bfiles\s*\.\s*lock\s*\(\)"),
    "Vfs": re.compile(r"\b(?:crate::fs::)?ramfs::fs\s*\(\)"),
    "ProcessTable": re.compile(r"\bTABLE\s*\.\s*lock\s*\(\)"),
    "PosixRecord": re.compile(r"\bVERROUS\s*\.\s*lock\s*\(\)"),
}

# L'expression liee doit ETRE la prise pour que le garde soit tenu au-dela de
# l'instruction.
LIAISON_EXACTE = {
    # La transition est un verrou manuel dont la duree traverse volontairement
    # `switch_context`; elle n'a donc pas de garde Rust lie a une variable.
    "SchedulerTransition": re.compile(r"^commence_transition_ordonnanceur\(\)$"),
    "SchedulerAlarms": re.compile(r"^ALARMES\.lock\(\)$"),
    "FdTable": re.compile(r"^[\w.]*files\s*\.\s*lock\(\)$"),
    "Vfs": re.compile(r"^(?:crate::fs::)?ramfs::fs\(\)$"),
    "ProcessTable": re.compile(r"^TABLE\.lock\(\)$"),
    "PosixRecord": re.compile(r"^VERROUS\.lock\(\)$"),
}

# Classes declarees dans `lockdep.rs` qu'AUCUN verrou ne porte encore.
#
# Ce n'est pas une liste d'exceptions : c'est la dette restante du chantier 1,
# rendue comptable. Elle ne peut que RETRECIR. Cabler une classe, c'est la
# retirer d'ici ; une classe qui y reapparaitrait signalerait un verrou
# redevenu anonyme.
NON_CABLEES = {
    "TaskTable",
    "Process",
    "PageCache",
    "Vm",
    "Network",
    "Driver",
    "Persistence",
}

LIAISON = re.compile(r"\blet\s+(?:mut\s+)?(\w+)\s*=\s*([^;]*?)\s*;")
LIBERATION = re.compile(r"\bdrop\s*\(\s*(\w+)\s*\)")
DECLARATION = re.compile(r"RankedSpinLock::new\(\s*(?:crate::kernel::sync::)?"
                         r"(?:lockdep::)?LockClass::(\w+)")
DECLARATION_MANUELLE = re.compile(
    r"lockdep::acquired\(\s*LockClass::(\w+)\s*\)"
)


def rangs() -> dict[str, int]:
    """Lit les rangs depuis `lockdep.rs`.

    Les relire plutot que les recopier evite le seul defaut que ce
    verificateur ne pourrait pas signaler : un rang change dans le noyau et
    laisse tel quel ici.
    """
    texte = LOCKDEP.read_text(encoding="utf-8")
    bloc = re.search(r"pub enum LockClass \{(.*?)\n\}", texte, re.S)
    if not bloc:
        raise SystemExit("ECHEC  enum LockClass introuvable dans lockdep.rs")
    return {n: int(v) for n, v in re.findall(r"(\w+)\s*=\s*(\d+)", bloc.group(1))}


def sans_commentaires(ligne: str) -> str:
    position = ligne.find("//")
    return ligne if position < 0 else ligne[:position]


def classe(fragment: str) -> str | None:
    for nom, motif in ACCES.items():
        if motif.search(fragment):
            return nom
    return None


def verifie(chemin: Path, rang: dict[str, int]) -> list[str]:
    fautes: list[str] = []
    profondeur = 0
    tenus: list[tuple[str, str, int, int]] = []  # classe, nom, profondeur, ligne

    for numero, brute in enumerate(chemin.read_text(encoding="utf-8",
                                                    errors="replace").splitlines(), 1):
        ligne = sans_commentaires(brute)

        # Profondeur MINIMALE atteinte dans la ligne : `} else {` ferme bien le
        # bloc precedent, meme si son solde d'accolades est nul.
        courante = minimum = profondeur
        for caractere in ligne:
            if caractere == "{":
                courante += 1
            elif caractere == "}":
                courante -= 1
                minimum = min(minimum, courante)
        tenus = [t for t in tenus if t[2] <= minimum]

        for nom in LIBERATION.findall(ligne):
            if any(t[1] == nom for t in tenus) and tenus[-1][1] != nom:
                fautes.append(
                    f"{chemin.relative_to(RACINE)}:{numero} liberation hors LIFO : "
                    f"`drop({nom})` alors que `{tenus[-1][1]}` est au sommet.\n"
                    f"           {brute.strip()}\n"
                    "           `lockdep::released` exige un ordre LIFO : cette "
                    "forme panique au runtime."
                )
            tenus = [t for t in tenus if t[1] != nom]

        liaison = LIAISON.search(ligne)
        liee = None
        if liaison and classe(liaison.group(2)):
            expression = liaison.group(2).strip()
            candidate = classe(expression)
            if LIAISON_EXACTE[candidate].match(expression):
                liee = candidate
        prise = liee or (None if liee else classe(ligne))

        if prise:
            for tenue, nom, _, source in tenus:
                if rang[tenue] >= rang[prise]:
                    fautes.append(
                        f"{chemin.relative_to(RACINE)}:{numero} inversion : "
                        f"{tenue}({rang[tenue]}) tenu depuis la ligne {source} "
                        f"(`{nom}`), prise de {prise}({rang[prise]}).\n"
                        f"           {brute.strip()}\n"
                        "           L'ordre va du rang le plus PETIT au plus grand. "
                        "Relache le garde avant."
                    )
            if liee:
                tenus.append((liee, liaison.group(1), profondeur, numero))

        profondeur = courante

    return fautes


def main() -> int:
    rang = rangs()

    inconnues = set(ACCES) - set(rang)
    if inconnues:
        print(f"ECHEC  classes inconnues de lockdep.rs : {', '.join(sorted(inconnues))}")
        return 1

    # Chaque classe cablee doit l'etre par un RankedSpinLock ou par une prise
    # lockdep manuelle explicite. Le second cas couvre la porte du scheduler :
    # son etat doit survivre au changement de pile et ne peut donc pas etre un
    # garde RAII.
    declarees = set()
    for chemin in SOURCES.rglob("*.rs"):
        source = chemin.read_text(encoding="utf-8", errors="replace")
        declarees |= set(DECLARATION.findall(source))
        declarees |= set(DECLARATION_MANUELLE.findall(source))

    sans_verrou = set(ACCES) - declarees
    if sans_verrou:
        print("ECHEC  classes suivies ici mais sans verrou range ni prise manuelle : "
              f"{', '.join(sorted(sans_verrou))}")
        print("       Un rang declare sans verrou ne protege rien. Voir l'en-tete.")
        return 1

    attendues = set(rang) - set(ACCES)
    regressions = declarees & NON_CABLEES
    if regressions:
        print("ECHEC  classes cablees dans le noyau mais absentes de ACCES : "
              f"{', '.join(sorted(regressions))}")
        print("       Ajoute-les a ACCES/LIAISON_EXACTE, sinon leur ordre n'est "
              "verifie qu'au runtime.")
        return 1
    if attendues != NON_CABLEES:
        manquantes = NON_CABLEES - attendues
        nouvelles = attendues - NON_CABLEES
        print("ECHEC  la dette de cablage a bouge sans que le contrat suive.")
        if nouvelles:
            print(f"       Classes devenues non cablees : {', '.join(sorted(nouvelles))}")
        if manquantes:
            print(f"       Classes cablees, a retirer de NON_CABLEES : "
                  f"{', '.join(sorted(manquantes))}")
        return 1

    fautes: list[str] = []
    for chemin in sorted(SOURCES.rglob("*.rs")):
        fautes += verifie(chemin, rang)

    if fautes:
        print("ECHEC  ordre des verrous ranges\n")
        for faute in fautes:
            print(f"  {faute}\n")
        return 1

    ordre = " -> ".join(f"{n}({rang[n]})" for n in sorted(ACCES, key=lambda k: rang[k]))
    print(f"ok  ordre respecte : {ordre} ; {len(NON_CABLEES)} classe(s) "
          "encore sans verrou")
    return 0


if __name__ == "__main__":
    sys.exit(main())
