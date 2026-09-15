#!/usr/bin/env python3
"""Garde-fou : un processus qui brule un coeur doit dire a quoi.

# Le defaut, lu sur la machine de reference

Releve physique du 15 septembre 2026 :

    [PROC-SAMPLE] ... name=WebContent cpu_pct=76 ctx_delta=4 ...

Soixante-seize pour cent d'un coeur pour QUATRE changements de contexte en
trente secondes. Un processus qui travaille change de contexte ; celui-la
tourne sur place. Le releve s'arretait la.

Le noyau comptait pourtant deja chaque appel systeme par numero --
`SYSCALL_HITS`, incremente a chaque entree du dispatch. Ce compte n'etait
lisible que par la commande interactive `syscalls`, c'est-a-dire au clavier.
Sur la machine de reference le clavier ne repond pas : la mesure existait et
n'atteignait jamais l'archive.

# Ce qui est verifie

1. Le classement est une fonction PURE, verifiable sur l'hote.
2. Les deux tables couvrent le meme nombre de numeros.
3. Le dispatch compte les reponses `EAGAIN`, pas seulement les appels.
4. Le releve porte des DELTAS de fenetre, pas le cumul depuis le demarrage.
5. La ligne sort du releve periodique, sans que personne ait a taper.
6. Elle nomme l'appel, son nombre et ses `EAGAIN`.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
PROFIL = RACINE / "src/compat/linux/profil.rs"
ABI = RACINE / "src/compat/linux/mod.rs"
METRIQUES = RACINE / "src/kernel/process/thread/metriques.rs"


def corps(source, entete):
    debut = source.find(entete)
    if debut < 0:
        return None
    i = source.find("{", debut)
    if i < 0:
        return None
    profondeur = 0
    for j in range(i, len(source)):
        if source[j] == "{":
            profondeur += 1
        elif source[j] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[i:j + 1]
    return None


def sans_commentaires(source):
    return "\n".join(
        ligne for ligne in source.splitlines()
        if not ligne.lstrip().startswith("//")
    )


def main():
    fautes = []
    for chemin in (PROFIL, ABI, METRIQUES):
        if not chemin.exists():
            print("  - fichier absent : %s" % chemin)
            return 1

    profil_brut = PROFIL.read_text(encoding="utf-8")
    profil = sans_commentaires(profil_brut)
    abi = sans_commentaires(ABI.read_text(encoding="utf-8"))
    metriques = sans_commentaires(METRIQUES.read_text(encoding="utf-8"))

    # --- 1. Le classement reste verifiable seul ----------------------------
    if "pub fn insere(" not in profil:
        fautes.append(
            "profil.rs : `insere` a disparu. Un top N borne est la seule partie "
            "difficile de ce releve -- une place ecrasee au lieu d'etre decalee "
            "produit une ligne plausible avec les mauvais chiffres, ce qui est "
            "pire que pas de ligne. Elle doit rester isolee et testable."
        )
    for interdit, pourquoi in (
        ("use crate::", "il depend du noyau"),
        ("unsafe", "il sort du domaine verifiable"),
    ):
        if interdit in profil:
            fautes.append(
                "profil.rs : `%s` est apparu ; %s, donc le fichier ne se compile "
                "plus seul et `tools/platform/test_profil_appels.rs` cesserait "
                "de couvrir le classement." % (interdit, pourquoi)
            )

    # --- 2. Les deux tables doivent couvrir les memes numeros --------------
    a = re.search(r"pub const NUMEROS: usize = (\d+);", profil)
    b = re.search(r"const SYSCALL_HITS_LEN: usize = (\d+);", abi)
    if not a or not b:
        fautes.append(
            "profil.rs / mod.rs : impossible de lire la taille des deux tables "
            "d'appels ; leur accord ne peut plus etre verifie."
        )
    elif a.group(1) != b.group(1):
        fautes.append(
            "profil.rs dit %s numeros, le dispatch en compte %s. Le desaccord ne "
            "se voit nulle part a l'execution : le classement parcourrait une "
            "plage differente de celle qui est comptee, et les appels des "
            "numeros hauts disparaitraient du releve sans un mot."
            % (a.group(1), b.group(1))
        )

    # --- 3. Les EAGAIN sont comptes ----------------------------------------
    dispatch = corps(abi, "pub fn handle(")
    if dispatch is None:
        fautes.append("mod.rs : `handle` introuvable.")
    elif not re.search(r"result\s*==\s*-errno::EAGAIN", dispatch) or \
            "profil::note_eagain" not in dispatch:
        fautes.append(
            "mod.rs : `handle` ne compte plus les reponses `EAGAIN`. Une boucle "
            "d'attente active ne se reconnait pas a l'appel qu'elle emet mais a "
            "sa reponse : `lire=1 200 000` ne dit pas s'il y a du travail, "
            "`lire=1 200 000 eagain=1 199 998` le dit."
        )

    # --- 4. Des deltas, pas un cumul ---------------------------------------
    classement = corps(profil, "pub fn plus_chauds(")
    if classement is None:
        fautes.append("profil.rs : `plus_chauds` introuvable.")
    else:
        if "HITS_PRECEDENTS" not in classement or ".swap(" not in classement:
            fautes.append(
                "profil.rs : `plus_chauds` ne fait plus la difference avec la "
                "fenetre precedente. Le cumul depuis le demarrage est domine par "
                "le demarrage : un navigateur qui se met a tourner en rond au "
                "bout de deux minutes n'y deplacerait pas le classement."
            )
        if "if appels == 0" not in classement:
            fautes.append(
                "profil.rs : `plus_chauds` classe des appels qui n'ont pas ete "
                "emis sur la fenetre ; le releve se remplirait de zeros."
            )

    # --- 5 et 6. La ligne sort seule, et elle nomme ------------------------
    releve = corps(abi, "pub fn log_profil_appels(")
    if releve is None:
        fautes.append("mod.rs : `log_profil_appels` introuvable.")
    else:
        for motif, pourquoi in (
            ("[SYSCALL-TOP]", "l'etiquette que cherche l'analyse d'archive"),
            ("window_ns=", "un nombre d'appels sans duree n'est pas un debit"),
            ("eagain=", "la reponse, et non l'appel, designe l'attente active"),
            ("nr::name(", "un numero nu oblige a ouvrir la table a la main"),
        ):
            if motif not in releve:
                fautes.append(
                    "mod.rs : `log_profil_appels` a perdu `%s` -- %s."
                    % (motif, pourquoi)
                )

    if "log_profil_appels" not in metriques:
        fautes.append(
            "metriques.rs : le releve periodique n'appelle plus le profil des "
            "appels systeme. Il redeviendrait accessible par la seule commande "
            "interactive `syscalls` -- donc au clavier, sur une machine dont le "
            "clavier est precisement ce qu'on cherche a reparer."
        )

    if fautes:
        print("profil des appels : %d probleme(s)" % len(fautes))
        for faute in fautes:
            print("\n  - %s" % faute)
        return 1

    print(
        "profil des appels : classement pur et teste, tables d'accord, EAGAIN "
        "comptes, deltas par fenetre, ligne emise sans clavier"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
