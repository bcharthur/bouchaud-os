#!/usr/bin/env python3
"""La comptabilite CPU ne s'ecrit que sous sa sequence.

BOUCHAUD_COMPTA_SEQLOCK_V1

# Ce que ce garde-fou protege

Run 37589903681 (KVM) : panique noyau « task: runtime > fenetre tid=118
delta=35786689 window=33031990 ». `mesure_processus` lisait le curseur d'une
tache et ses compteurs a deux instants, pendant que le coeur de la tache les
mettait a jour : la tranche comptait deux fois.

Le correctif est un verrou de sequence : les lecteurs (`lecture_compta`,
`proc_cpu_cumul`) relisent tant qu'une ecriture est en cours. Il ne vaut que
si TOUTE ecriture passe par une section. Une seule ecriture oubliee -- un
nouveau chemin de commutation, un repli ajoute pour un diagnostic -- et la
lecture dechiree revient, sans que rien ne le dise avant la prochaine panique
sous KVM.

# Les regles

1. Une ecriture (`range`, `store`, `fetch_add`, `swap`, `echange`) des champs
   de la tache (`last_account_ns`, `user_cpu_ns`, `kernel_cpu_ns`, `cpu_ns[]`)
   ou du bloc par coeur (`COMPTA_USER_NS`, `COMPTA_NOYAU_NS`,
   `COMPTA_DEBUT_NS`, `COMPTA_EN_NOYAU`, `CUMUL_USER_NS`, `CUMUL_NOYAU_NS`) se
   trouve DANS la fermeture d'un appel `compta_section(...)`, ou dans
   `account_until` (corps d'un repli, appele seulement en section), ou dans
   `frontiere_compta_bloc` (appele seulement par `frontiere_compta`, qui tient
   `COMPTA_SEQ_CPU`).
2. `account_until(` n'est appele que dans une section ; `frontiere_compta_bloc(`
   seulement depuis `frontiere_compta`.
3. `compta_section` masque les interruptions et verifie la parite (un second
   ecrivain la casserait) ; `Task::new` initialise `compta_seq`.
4. `lecture_compta` ne borne pas ses relectures : une borne rendrait, au bout
   du compte, la lecture dechiree.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
SRC = RACINE / "src"

CHAMPS_TACHE = r"(?:last_account_ns|user_cpu_ns|kernel_cpu_ns|cpu_ns\[[^\]]*\])"
BLOC = r"(?:COMPTA_USER_NS|COMPTA_NOYAU_NS|COMPTA_DEBUT_NS|COMPTA_EN_NOYAU|CUMUL_USER_NS|CUMUL_NOYAU_NS)\[[^\]]*\]"
OPS = r"\.(?:range|store|fetch_add|fetch_sub|swap|echange|compare_exchange)\("
ECRITURE = re.compile(rf"(?:\.{CHAMPS_TACHE}|\b{BLOC}){OPS}")
FONCTIONS_CORPS = {"account_until", "frontiere_compta_bloc"}


def sans_commentaires(texte: str) -> str:
    """Remplace commentaires et chaines par des blancs, positions gardees."""
    sortie = []
    i, n = 0, len(texte)
    while i < n:
        if texte.startswith("//", i):
            j = texte.find("\n", i)
            j = n if j < 0 else j
            sortie.append(" " * (j - i))
            i = j
        elif texte[i] == '"':
            j = i + 1
            while j < n and texte[j] != '"':
                j += 2 if texte[j] == "\\" else 1
            sortie.append('"' + " " * (min(j, n - 1) - i - 1) + '"')
            i = j + 1
        else:
            sortie.append(texte[i])
            i += 1
    return "".join(sortie)


def fonctions(code: str):
    """(nom, debut, fin) des corps de `fn`, par appariement d'accolades."""
    for m in re.finditer(r"\bfn\s+(\w+)\s*(?:<[^>{]*>)?\s*\(", code):
        ouverture = code.find("{", m.end())
        point_virgule = code.find(";", m.end())
        if ouverture < 0 or (0 <= point_virgule < ouverture):
            continue
        profondeur, j = 0, ouverture
        while j < len(code):
            if code[j] == "{":
                profondeur += 1
            elif code[j] == "}":
                profondeur -= 1
                if profondeur == 0:
                    break
            j += 1
        yield m.group(1), ouverture, j


def sections(code: str):
    """Etendues des appels `compta_section(` (parentheses appariees)."""
    for m in re.finditer(r"\bcompta_section\s*\(", code):
        profondeur, j = 0, m.end() - 1
        while j < len(code):
            if code[j] == "(":
                profondeur += 1
            elif code[j] == ")":
                profondeur -= 1
                if profondeur == 0:
                    break
            j += 1
        yield m.start(), j


def enclosante(fns, position):
    meilleure = None
    for nom, debut, fin in fns:
        if debut <= position <= fin and (meilleure is None or debut > meilleure[1]):
            meilleure = (nom, debut, fin)
    return meilleure[0] if meilleure else None


def main() -> int:
    fautes = []
    vu_section = vu_lecture = vu_init = False
    for fichier in sorted(SRC.rglob("*.rs")):
        brut = fichier.read_text(encoding="utf-8")
        if not any(mot in brut for mot in ("_cpu_ns", "last_account_ns", "COMPTA_", "CUMUL_", "account_until", "frontiere_compta")):
            continue
        code = sans_commentaires(brut)
        relatif = fichier.relative_to(RACINE).as_posix()
        fns = list(fonctions(code))
        etendues = list(sections(code))

        def ligne(pos):
            return code.count("\n", 0, pos) + 1

        def en_section(pos):
            return any(a <= pos <= b for a, b in etendues)

        for m in ECRITURE.finditer(code):
            nom = enclosante(fns, m.start())
            if en_section(m.start()) or nom in FONCTIONS_CORPS:
                continue
            fautes.append(f"  {relatif}:{ligne(m.start())}  ecriture `{m.group(0)}` hors section "
                          f"(fonction `{nom}`) : un lecteur la verrait a moitie")

        for m in re.finditer(r"\baccount_until\s*\(", code):
            if code[max(0, m.start() - 3):m.start()] == "fn ":
                continue
            if not en_section(m.start()):
                fautes.append(f"  {relatif}:{ligne(m.start())}  `account_until` appele hors `compta_section`")
        for m in re.finditer(r"\bfrontiere_compta_bloc\s*\(", code):
            if code[max(0, m.start() - 3):m.start()] == "fn ":
                continue
            if enclosante(fns, m.start()) != "frontiere_compta":
                fautes.append(f"  {relatif}:{ligne(m.start())}  `frontiere_compta_bloc` appele hors `frontiere_compta`")

        for nom, debut, fin in fns:
            corps = code[debut:fin]
            if nom == "compta_section":
                vu_section = True
                if "without_interrupts" not in corps:
                    fautes.append(f"  {relatif}  `compta_section` ne masque plus les interruptions : "
                                  "le minuteur pourrait ouvrir une section imbriquee")
                if "debug_assert!(seq & 1 == 0" not in corps:
                    fautes.append(f"  {relatif}  `compta_section` ne verifie plus la parite")
                if "COMPTA_SEQ_CPU" not in corps:
                    fautes.append(f"  {relatif}  `compta_section` ne tient plus la sequence du coeur")
            if nom == "frontiere_compta" and "COMPTA_SEQ_CPU" not in corps:
                fautes.append(f"  {relatif}  `frontiere_compta` n'ecrit plus le bloc sous `COMPTA_SEQ_CPU`")
            if nom == "lecture_compta":
                vu_lecture = True
                if re.search(r"essais\s*>=", corps):
                    fautes.append(f"  {relatif}  `lecture_compta` borne ses relectures : elle rendrait une lecture dechiree")
        if "compta_seq: EcheanceAtomique::neuf(0)" in code:
            vu_init = True

    if not vu_section:
        fautes.append("  `compta_section` a disparu")
    if not vu_lecture:
        fautes.append("  `lecture_compta` a disparu")
    if not vu_init:
        fautes.append("  `Task::new` n'initialise plus `compta_seq`")

    if fautes:
        print("comptabilite CPU : regle du verrou de sequence violee")
        print("\n".join(fautes))
        return 1
    print("comptabilite CPU : toutes les ecritures sous sequence")
    return 0


if __name__ == "__main__":
    sys.exit(main())
