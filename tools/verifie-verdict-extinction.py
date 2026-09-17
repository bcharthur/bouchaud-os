#!/usr/bin/env python3
"""Garde-fou : l'ecran d'extinction ne doit pas accuser ce qui a marche.

# Le defaut, lu sur une photo du 17 septembre 2026

L'ecran titrait « Sauvegarde incomplete » sur une archive PARFAITE :

    ETAPE   : support=1 draine=1 marque=1 sync=1
    SUPPORT : ecritures=4500 echecs=0 consecutifs=0 err=0x0
    CLE BB  : 0 arret(s) de point, 0 recul(s), lot=16
    PERSIST : ECHEC d ecriture, /persist n est PAS a jour

Quatre mille cinq cents enregistrements poses, zero echec, zero perdu, marque
de fin et synchronisation reussies. Le seul « echec » etait l'absence de zone
/persist sur un Stage 2 a cle unique -- qui n'en demande aucune, et qui
l'annonce lui-meme au demarrage.

Le titre le plus visible de l'extinction faisait donc douter de la seule
chose qui avait parfaitement marche. C'est le genre de defaut qui coute une
session physique entiere : on part chercher une panne de sauvegarde qui
n'existe pas.

# Ce qui est verifie ici

1. `synchronise` distingue « aucune zone » de « l'ecriture a echoue ». Un
   code unique pour les deux etats rend les deux enquetes impossibles.
2. Le verdict d'extinction ne compte PAS l'absence de zone comme un echec,
   sur le chemin de l'extinction ET sur celui du redemarrage.
3. La ligne PERSIST du bloc de detail a bien TROIS cas, et non deux. C'est
   la STRUCTURE qui est verifiee, pas la formulation : figer le texte exact
   d'un message ferait crier cette garde a chaque reecriture, et une garde
   qui crie pour rien finit par etre ignoree.
4. Le bloc de detail est dessine AVANT la pause de `finish`, sinon il
   apparait a l'instant ou le courant est coupe -- c'est-a-dire jamais, ce
   que montre la photo du 16 septembre.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]

POWER = RACINE / "src/kernel/power.rs"
SYNC = RACINE / "src/fs/persistance/sync.rs"
IO = RACINE / "src/fs/persistance/io.rs"


def lit(chemin, fautes):
    if not chemin.exists():
        fautes.append("fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
        return None
    return chemin.read_text(encoding="utf-8")


def main():
    fautes = []
    power = lit(POWER, fautes)
    sync = lit(SYNC, fautes)
    io = lit(IO, fautes)

    # 1. Les deux situations ont deux codes.
    if sync is not None and "pub const SANS_ZONE" not in sync:
        fautes.append(
            "persistance/sync.rs : `SANS_ZONE` a disparu. « Rien a ecrire » et "
            "« je n'ai pas pu ecrire » redeviennent le meme code, et les deux "
            "enquetes qu'ils demandent redeviennent impossibles a separer."
        )
    if io is not None:
        debut = io.find("fn synchronise_snapshot")
        absence = io.find("SANS_ZONE", debut) if debut >= 0 else -1
        if absence < 0:
            fautes.append(
                "persistance/io.rs : l'absence de zone rend de nouveau un "
                "echec. Une machine qui n'a rien a ecrire serait declaree en "
                "panne d'ecriture -- ce qui a fait titrer « Sauvegarde "
                "incomplete » sur une archive intacte."
            )

    if power is not None:
        # 2. Le verdict ne compte pas l'absence comme un echec.
        verdicts = re.findall(r"let complet = vidage\.complet\(\) && persisted ([^;]+);", power)
        if not verdicts:
            fautes.append("power.rs : le verdict d'extinction est introuvable.")
        for v in verdicts:
            if ">= 0" in v or "== 0" in v:
                fautes.append(
                    "power.rs : le verdict compte de nouveau l'absence de zone "
                    "comme un echec (`persisted %s`). Le Stage 2 a cle unique "
                    "n'a aucune zone de persistance, et son extinction serait "
                    "declaree incomplete a chaque fois." % v.strip()
                )
        if len(verdicts) < 2:
            fautes.append(
                "power.rs : un seul chemin porte le verdict. L'extinction et "
                "le redemarrage doivent le calculer pareil, sinon l'un des "
                "deux ment."
            )

        # 3. La ligne PERSIST dit laquelle des deux situations.
        #
        # ANCRAGE DANS `rapporte_echec`, ET SUR LA BRANCHE.
        #
        # Chercher le texte dans tout le fichier trouvait la ligne du journal
        # serie, qui porte les memes mots : la garde se taisait alors que la
        # ligne AFFICHEE, la seule que l'utilisateur photographie, avait perdu
        # la distinction.
        debut_rapport = power.find("fn rapporte_echec(")
        corps_rapport = power[debut_rapport:] if debut_rapport >= 0 else ""
        if "persisted == crate::fs::persistance::SANS_ZONE" not in corps_rapport:
            fautes.append(
                "power.rs : le bloc de detail ne distingue plus « rien a "
                "ecrire » de « ECHEC d ecriture ». C'est la ligne que "
                "l'utilisateur lit sur la photo de son ecran."
            )

        # 4. Le detail est dessine AVANT la pause de `finish`.
        for bloc in re.findall(r"if !complet \{.*?\n    \}\n[^\n]*", power, re.S):
            pass
        rapport = power.find("rapporte_echec(&vidage")
        fin = power.find("power_screen::finish(complet)")
        if rapport < 0 or fin < 0:
            fautes.append("power.rs : le rapport d'echec ou `finish` a disparu.")
        elif rapport > fin:
            fautes.append(
                "power.rs : le detail est de nouveau dessine APRES `finish`. "
                "`finish(false)` s'arrete deux secondes pour laisser lire "
                "l'echec ; le detail apparaitrait donc a l'instant ou le "
                "courant est coupe, c'est-a-dire jamais. C'est exactement ce "
                "que montre la photo du 16 septembre : le titre, et rien "
                "dessous."
            )

    if fautes:
        for faute in fautes:
            print("FAUTE: %s" % faute)
        return 1
    print(
        "verdict d'extinction : absence de zone distinguee d'un echec, les "
        "deux chemins d'accord, detail lisible avant la coupure."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
