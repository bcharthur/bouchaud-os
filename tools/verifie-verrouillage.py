#!/usr/bin/env python3
"""Verifie que les appels systeme declares « sans BKL » le meritent vraiment.

Le retrait du gros verrou noyau se fait appel par appel. Chaque retrait repose
sur une preuve, et une preuve fausse ne se voit ni a la compilation ni au boot :
elle se voit un jour, sous charge, a quatre coeurs, sous la forme d'une
corruption qu'on ne saura pas relier a sa cause.

Ce script est la barriere externe. Il relit deux fichiers qui ne se parlent pas
autrement :

  * `src/compat/linux/bkl.rs` -- la table `SANS_BKL` : qui est libere, et pourquoi ;
  * `src/compat/linux/mod.rs` -- l'aiguillage : ce que l'appel fait REELLEMENT.

Et il refuse :

  1. un numero libere qui n'existe pas dans `nr.rs` ;
  2. un numero libere deux fois ;
  3. une ligne sans justification ;
  4. un appel libere dont le bras d'aiguillage fait autre chose que rendre une
     constante -- sauf s'il figure dans `AUDITS_NOMMES` ci-dessous, c'est-a-dire
     s'il a recu un audit ecrit, nomme, qu'on peut relire.

Le point (4) est celui qui compte. Il fait que declarer « sans verrou » un appel
qui touche la table des taches, la memoire utilisateur ou le systeme de fichiers
casse la CI, et non la machine de l'utilisateur. Il fait aussi qu'un appel
aujourd'hui trivial qui cesserait de l'etre -- une constante remplacee par un
vrai calcul -- ramene la question sur la table au lieu de passer inapercu.

Ce que ce script ne peut PAS faire : lire une fonction et decider si elle est
sure. Pour tout ce qui n'est pas une constante, la preuve est humaine, et
`AUDITS_NOMMES` est la liste de celles qui ont ete faites.

Code de retour : 0 si la table et l'aiguillage sont d'accord, 1 sinon.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
# La compatibilite Linux a quitte le coeur du noyau avec la fondation
# multiplateforme (0b3eb17) : ces trois fichiers vivent desormais sous
# `src/compat/linux/`. Le chemin est resolu, et non devine, pour que le
# prochain deplacement echoue en le disant plutot qu'en ne verifiant plus rien.
ABI = RACINE / "src" / "compat" / "linux"
NR = ABI / "nr.rs"
BKL = ABI / "bkl.rs"
DISPATCH = ABI / "mod.rs"

for _chemin in (NR, BKL, DISPATCH):
    if not _chemin.exists():
        raise SystemExit(
            f"verifie-verrouillage : {_chemin} est introuvable.\n"
            "La table des appels systeme a ete deplacee : mets ce chemin a jour,\n"
            "sinon cette barriere ne verifie plus rien."
        )

# Les appels liberes dont le bras d'aiguillage n'est PAS une constante, et dont
# l'audit est donc humain. Chaque entree nomme ou lire cet audit ; sans cela, la
# ligne n'a pas sa place ici.
AUDITS_NOMMES = {
    "MPROTECT": "jalon SMP4 -- domaine Arc<Process>::Mm + protocole TLB sur IRQ",
    "BRK": "jalon SMP4 -- domaine Arc<Process>::Mm + protocole TLB sur IRQ",
    # A1 lot 2 -- voir l'en-tete de bkl.rs et la preuve de duree de vie sur
    # `task::identite_courante`. Chacun de ces appels a son audit ecrit en
    # commentaire au-dessus de sa ligne dans SANS_BKL.
    # A1 lot 3 -- voir l'en-tete de bkl.rs et l'audit ecrit au-dessus de la
    # ligne POLL dans SANS_BKL : domaine table des descripteurs + verrou par
    # objet, `current_process_local` a la place de `current_process`, et les
    # trois branches a etat global qui prennent le verrou elles-memes.
    "POLL": "A1 lot 3 -- table des descripteurs + verrou par objet",
    "PPOLL": "A1 lot 3 -- table des descripteurs + verrou par objet",
    "GETPID": "A1 lot 2 -- domaine CPU-local, aucune lecture de TASKS",
    "GETTID": "A1 lot 2 -- domaine CPU-local, aucune lecture de TASKS",
    "GETUID": "A1 lot 2 -- domaine CPU-local + verrou metadata du Process",
    "GETEUID": "A1 lot 2 -- domaine CPU-local + verrou metadata du Process",
    "GETGID": "A1 lot 2 -- domaine CPU-local + verrou metadata du Process",
    "GETEGID": "A1 lot 2 -- domaine CPU-local + verrou metadata du Process",
    "CLOCK_GETTIME": "A1 lot 2 -- horloges atomiques + Mm ; verrou local a la branche CPUTIME",
    "CLOCK_GETRES": "A1 lot 2 -- constante calculee + Mm",
    "GETTIMEOFDAY": "A1 lot 2 -- ancre d'epoque atomique + Mm",
    "TIME": "A1 lot 2 -- ancre d'epoque atomique + Mm",
}

# Une constante rendue directement : `0`, `1`, `-errno::ENOSYS`.
CONSTANTE = re.compile(r"^-?(?:\d+|errno::[A-Z0-9_]+)$")

# BOUCHAUD_P3_POLL_SANS_BKL_V1
#
# Les sondes de readiness de `poll` ne doivent JAMAIS passer par
# `task::current_process()`, qui reprend le gros verrou. Remettre cet appel les
# ferait acquerir le verrou depuis zero une fois par descripteur et par sonde --
# 70 000 fois par seconde sur un vrai Ladybird -- et la liberation de `poll`
# couterait plus qu'elle ne rapporte, sans que rien ne le signale.
SONDES_READINESS = ("readable", "writable", "etat_pair", "readiness_deadline_ns")
FICHIER_SONDES = "src/compat/linux/file.rs"

erreurs = []


def echec(message):
    erreurs.append(message)


def verifie_sondes_readiness():
    """Aucune sonde de readiness ne doit reprendre le gros verrou."""
    chemin = RACINE / FICHIER_SONDES
    if not chemin.exists():
        echec(f"{FICHIER_SONDES} introuvable : impossible de verifier les sondes")
        return
    lignes = chemin.read_text(encoding="utf-8").splitlines()

    courante = None
    profondeur = 0
    for numero, brute in enumerate(lignes, 1):
        ligne = brute.split("//")[0]
        for sonde in SONDES_READINESS:
            if re.search(rf"\bfn\s+{sonde}\s*\(", ligne):
                courante = sonde
                profondeur = 0
        if courante is None:
            continue
        if "task::current_process()" in ligne:
            echec(
                f"{FICHIER_SONDES}:{numero} `{courante}` appelle "
                f"`task::current_process()`, qui REPREND le gros verrou.\n"
                f"           {brute.strip()}\n"
                "           Utilise `current_process_local()` : sans cela, "
                "liberer `poll` coute\n"
                "           une acquisition par descripteur et par sonde, soit "
                "plus que le gain."
            )
        profondeur += ligne.count("{") - ligne.count("}")
        if profondeur <= 0 and "}" in ligne:
            courante = None


def numeros_syscalls():
    valeurs = {}
    for nom, valeur in re.findall(
        r"pub const (\w+): u64 = (\d+);", NR.read_text(encoding="utf-8")
    ):
        valeurs[nom] = int(valeur)
    return valeurs


def table_sans_bkl():
    """Lit `SANS_BKL` : la liste des (nr::NOM, justification)."""
    source = BKL.read_text(encoding="utf-8")
    debut = source.find("pub const SANS_BKL")
    if debut < 0:
        echec("bkl.rs : table SANS_BKL introuvable")
        return []
    fin = source.find("];", debut)
    corps = source[debut:fin]
    # Les commentaires portent la justification longue ; la ligne, la courte.
    corps = re.sub(r"//[^\n]*", "", corps)
    return re.findall(r"\(\s*nr::(\w+)\s*,\s*\"([^\"]*)\"\s*\)", corps)


def bras_constants():
    """Les noms d'appels dont le bras d'aiguillage rend une constante."""
    source = DISPATCH.read_text(encoding="utf-8")
    debut = source.find("fn dispatch(")
    if debut < 0:
        echec("mod.rs : fonction dispatch introuvable")
        return {}
    corps = source[debut:]
    constants = {}
    for ligne in corps.splitlines():
        m = re.match(r"^ {8}([A-Z][A-Z0-9_]*(?:\s*\|\s*[A-Z][A-Z0-9_]*)*)\s*=>\s*(.+?),?$", ligne)
        if not m:
            continue
        motifs, valeur = m.group(1), m.group(2).strip().rstrip(",").strip()
        if not CONSTANTE.match(valeur):
            continue
        for nom in re.split(r"\s*\|\s*", motifs):
            constants[nom] = valeur
    return constants


def main():
    numeros = numeros_syscalls()
    if not numeros:
        echec("nr.rs : aucun numero d'appel systeme lu")
    table = table_sans_bkl()
    constants = bras_constants()

    vus = {}
    for nom, justification in table:
        if nom not in numeros:
            echec(f"SANS_BKL : `nr::{nom}` n'existe pas dans nr.rs")
            continue
        numero = numeros[nom]
        if numero in vus:
            echec(f"SANS_BKL : le numero {numero} est libere deux fois "
                  f"({vus[numero]} et {nom})")
        vus[numero] = nom
        if not justification.strip():
            echec(f"SANS_BKL : `nr::{nom}` est libere sans justification")
        if nom in AUDITS_NOMMES:
            continue
        if nom not in constants:
            echec(
                f"SANS_BKL : `nr::{nom}` est libere, mais son bras d'aiguillage "
                f"ne rend pas une constante et il n'a pas d'audit nomme. "
                f"Soit on ecrit l'audit dans AUDITS_NOMMES, soit on le remet "
                f"sous le gros verrou."
            )

    verifie_sondes_readiness()

    for nom in AUDITS_NOMMES:
        if nom not in vus.values():
            echec(f"AUDITS_NOMMES : `{nom}` n'est plus libere ; retirer son audit")

    print(f"nr.rs           : {len(numeros)} numeros d'appels systeme")
    print(f"SANS_BKL        : {len(table)} appels liberes")
    print(f"  dont constants: {sum(1 for n, _ in table if n in constants)}")
    print(f"  dont audites  : {sum(1 for n, _ in table if n in AUDITS_NOMMES)}")

    if erreurs:
        print()
        for message in erreurs:
            print(f"  ECHEC {message}")
        print(f"\n{len(erreurs)} probleme(s)")
        return 1
    print("\ntable et aiguillage d'accord")
    return 0


if __name__ == "__main__":
    sys.exit(main())
