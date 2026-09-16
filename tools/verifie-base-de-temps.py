#!/usr/bin/env python3
"""Garde-fou : l'instrument qui mesure le temps ne doit pas mentir dessus.

# Deux defauts, releves le meme jour sur la meme image

## 1. Le verdict de la sonde etait un OU

    (dt, dm, dt != 0 || dm != 0)

Il suffisait qu'UNE des deux horloges bouge pour annoncer `progress=1`. Une
horloge arretee passait ; deux horloges en desaccord passaient aussi.

Meme image, deux machines, 16 septembre 2026 :

    BOUCHAUD_HWPROBE_TIMER progress=1 ticks_delta=90 ms_delta=90   (saine)
    BOUCHAUD_HWPROBE_TIMER progress=1 ticks_delta=23 ms_delta=14   (fausse)

La seconde portait aussi `tsc_mhz=43989` et `lapic_hz=888654262`, contre 2096
et 62433112 sur la premiere : sa base de temps etait fausse d'un ordre de
grandeur, son horloge monotone avancait dix fois trop lentement, et la seule
ligne censee l'annoncer disait vert.

Ce n'est pas cosmetique. `monotonic_ms` porte `BOUCHAUD_BOOT_POINT ... t_ms=
delta_ms=`, le releve periodique et les echeances de l'ordonnanceur.

## 2. Un point d'amorcage refirait apres l'amorcage

    BOUCHAUD_BOOT_POINT navigateur-lance t_ms=5972 delta_ms=4683
    BOUCHAUD_BOOT_POINT navigateur-lance t_ms=6092 delta_ms=120
    BOUCHAUD_BOOT_POINT navigateur-lance t_ms=6105 delta_ms=13

Un par clic sur « Demarrer Ladybird ». Chaque repetition ecrasait l'instant du
point precedent -- donc le `delta_ms` du suivant -- et le dernier point
franchi, celui que l'ecran de faute affiche apres une panne.

# Ce qui est verifie

1. La decision d'accord des horloges est une fonction PURE.
2. Elle distingue « pas pu verifier » de « verifie » : quatre etats au moins.
3. La sonde emploie cette decision, et non une comparaison a zero.
4. Elle etend sa fenetre jusqu'a pouvoir conclure, avec un plafond.
5. Une base de temps incoherente est CRIEE, pas laissee a deduire.
6. La chronologie d'amorcage se ferme, et `point` refuse apres.
7. Le dernier point du bureau la ferme effectivement.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
ACCORD = RACINE / "src/kernel/time/accord.rs"
SONDE = RACINE / "src/platform/pc/hardware_probe.rs"
ECRAN = RACINE / "src/platform/pc/ecran_faute.rs"
WM = RACINE / "src/gui/window_manager.rs"


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
    for chemin in (ACCORD, SONDE, ECRAN, WM):
        if not chemin.exists():
            print("  - fichier absent : %s" % chemin)
            return 1

    accord = sans_commentaires(ACCORD.read_text(encoding="utf-8"))
    sonde = sans_commentaires(SONDE.read_text(encoding="utf-8"))
    ecran = sans_commentaires(ECRAN.read_text(encoding="utf-8"))
    wm = sans_commentaires(WM.read_text(encoding="utf-8"))

    # --- 1. La decision reste verifiable seule --------------------------
    if "pub fn accord(" not in accord:
        fautes.append(
            "accord.rs : `accord` a disparu ; la decision sur la base de temps "
            "n'est plus verifiable sur l'hote."
        )
    for interdit, pourquoi in (
        ("use crate::", "il depend du noyau"),
        ("unsafe", "il sort du domaine verifiable"),
    ):
        if interdit in accord:
            fautes.append(
                "accord.rs : `%s` est apparu ; %s, donc le fichier ne se compile "
                "plus seul et `tools/platform/test_accord_horloges.rs` cesserait "
                "de couvrir la decision." % (interdit, pourquoi)
            )

    # --- 2. « Pas pu verifier » n'est pas « verifie » -------------------
    for etat in ("AucuneNAvance", "UneSeuleAvance", "Desaccord", "FenetreTropCourte"):
        if etat not in accord:
            fautes.append(
                "accord.rs : l'etat `%s` a disparu. Les quatre facons dont une "
                "base de temps peut etre douteuse ne se corrigent pas de la meme "
                "facon, et les confondre est exactement ce que faisait le OU."
                % etat
            )
    decision = corps(accord, "pub fn accord(")
    if decision is None:
        fautes.append("accord.rs : le corps de `accord` est illisible.")
    else:
        if "FENETRE_MINIMALE" not in decision:
            fautes.append(
                "accord.rs : `accord` conclut sur n'importe quelle fenetre. Sous "
                "huit tics, un ecart d'une unite pese plus de douze pour cent : "
                "la quantification seule produirait des verdicts."
            )
        if "u128" not in decision:
            fautes.append(
                "accord.rs : `accord` calcule l'ecart sans passer par `u128`. "
                "`(grand - petit) * 100` deborde des qu'un compteur approche "
                "`u64::MAX`, et un debordement rendrait « tout va bien » sur la "
                "pire des entrees."
            )

    # --- 3 et 4. La sonde emploie la decision, et se donne de quoi juger -
    probe = corps(sonde, "fn time_progress_probe(")
    if probe is None:
        fautes.append("hardware_probe.rs : `time_progress_probe` introuvable.")
    else:
        if re.search(r"dt\s*!=\s*0\s*\|\|\s*dm\s*!=\s*0", probe):
            fautes.append(
                "hardware_probe.rs : le verdict est redevenu un OU. Il suffit "
                "alors qu'une seule horloge bouge pour annoncer `progress=1` -- "
                "une horloge MORTE passe, et c'est le defaut d'origine."
            )
        if "accord(" not in probe:
            fautes.append(
                "hardware_probe.rs : la sonde ne consulte plus "
                "`accord_horloges::accord`. Sa decision redevient locale, donc "
                "non verifiable sur l'hote."
            )
        if "FENETRE_MINIMALE" not in probe:
            fautes.append(
                "hardware_probe.rs : la sonde ne cherche plus a obtenir une "
                "fenetre assez longue. Cinq cent mille tours de boucle vide "
                "donnaient quatre-vingt-dix millisecondes sur une machine et "
                "quatorze sur une autre : le verdict deviendrait « fenetre trop "
                "courte » selon le materiel."
            )
        # La CONDITION DE SORTIE, pas une occurrence quelconque du plafond :
        # un `debug_assert!` qui le mentionne suffisait a satisfaire une
        # verification par sous-chaine, et la boucle restait sans borne.
        if not re.search(
            r"if\s*\(?[^\n{]*FENETRE_MINIMALE[^\n{]*\)?\s*\|\|\s*tours\s*>=\s*\d+\s*\{",
            probe,
        ):
            fautes.append(
                "hardware_probe.rs : la boucle d'elargissement n'a plus de "
                "plafond. Sur une machine dont IRQ0 est MORTE la condition "
                "d'arret n'arrive jamais -- et c'est precisement la panne qu'on "
                "cherche a nommer."
            )

    # --- 5. Une base fausse se crie -------------------------------------
    if "BOUCHAUD_HORLOGE_INCOHERENTE" not in sonde:
        fautes.append(
            "hardware_probe.rs : plus rien ne signale une base de temps "
            "incoherente. Tout ce qui se mesure ensuite s'y adosse ; il faut que "
            "la trace le dise, pas qu'on deduise l'ordre de grandeur en "
            "comparant deux machines."
        )

    # --- 6. La chronologie se ferme, et `point` refuse apres -------------
    if "pub fn amorcage_clos(" not in ecran:
        fautes.append(
            "ecran_faute.rs : `amorcage_clos` a disparu. Sans fin explicite, un "
            "point d'amorcage peut etre repose apres l'amorcage."
        )
    pose = corps(ecran, "pub fn point(")
    if pose is None:
        fautes.append("ecran_faute.rs : `point` introuvable.")
    elif not re.search(r"if\s+AMORCAGE_CLOS\.load\([^)]*\)\s*\{\s*return", pose):
        fautes.append(
            "ecran_faute.rs : `point` accepte encore un point apres la fermeture "
            "de la chronologie. Chaque repetition ecrase l'instant du point "
            "precedent -- donc le `delta_ms` du suivant -- et le dernier point "
            "franchi, celui que l'ecran de faute affiche apres une panne."
        )

    # --- 7. Le bureau ferme effectivement --------------------------------
    if "ecran_faute::amorcage_clos()" not in wm:
        fautes.append(
            "window_manager.rs : le dernier point du bureau ne ferme plus la "
            "chronologie. Elle resterait ouverte pour toute la session, et un "
            "clic sur « Demarrer Ladybird » reposerait un point d'amorcage."
        )
    if "navigateur-echec" not in wm:
        fautes.append(
            "window_manager.rs : le dernier point n'a plus de variante d'echec. "
            "Il annoncerait un lancement meme quand le binaire du navigateur est "
            "absent -- ce qui est le cas de toutes les images sans userland."
        )

    if fautes:
        print("base de temps : %d probleme(s)" % len(fautes))
        for faute in fautes:
            print("\n  - %s" % faute)
        return 1

    print(
        "base de temps : accord des horloges pur et teste, fenetre elargie et "
        "bornee, incoherence criee, chronologie d'amorcage close apres son "
        "dernier point"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
