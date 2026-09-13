#!/usr/bin/env python3
"""Garde-fou : l'horloge rendue au ring 3 a la resolution qu'elle annonce.

# Le defaut, lu sur la machine de reference

Le releve du 12 septembre 2026 contient cette ligne :

    [PROC-SAMPLE] pid=12 name=/usr/libexec/ladybird/WebContent
                  cpu_pct=21 ctx_delta=3 runnable_threads=1 threads=1

Vingt et un pour cent de processeur pour TROIS changements de contexte en
trente secondes. Un processus qui travaille change de contexte ; celui-la
tournait sur place, et il tenait a lui seul le coeur 5 a quatre-vingt-dix-sept
pour cent.

Deux defauts se conjuguaient, et tous deux dans la conversion du temps :

1. `clock_gettime` calculait un nombre de MILLISECONDES puis le rendait comme
   un `timespec`. Le champ des nanosecondes etait donc toujours un multiple
   d'un million : l'horloge avancait par marches d'une milliseconde. Une
   boucle qui mesure une duree ecoulee y lit zero pour tout ce qui dure moins
   que la marche, et une boucle qui attend que le temps avance y tourne a vide
   pendant toute sa duree. L'horloge monotone du noyau est pourtant lue au TSC
   et a toute la resolution qu'il faut -- elle etait jetee a la conversion.

2. `timespec_ms` TRONQUAIT : `nanosleep(500 us)` rendait zero milliseconde,
   et le sommeil rendait la main sans dormir. C'est aussi une faute de
   contrat : POSIX garantit qu'un sommeil dure AU MOINS ce qu'on a demande.

# Ce qui est verifie

Que `clock_gettime` parte des nanosecondes, que le `timespec` rendu porte un
reste en nanosecondes et non un multiple d'un million, qu'une duree non nulle
ne soit jamais arrondie a zero, et qu'une duree NULLE le reste -- un `poll`
sans attente doit continuer de ne pas attendre.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
COMPAT = RACINE / "src/compat/linux/mod.rs"


def sans_commentaires(texte):
    return "\n".join(
        l for l in texte.splitlines() if not l.lstrip().startswith("//")
    )


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


def main():
    if not COMPAT.exists():
        print("  - fichier absent : src/compat/linux/mod.rs")
        return 1
    source = sans_commentaires(COMPAT.read_text(encoding="utf-8"))
    fautes = []

    horloge = corps(source, "fn sys_clock_gettime(")
    if horloge is None:
        fautes.append("mod.rs : `sys_clock_gettime` a disparu.")
    else:
        if "monotonic_ns()" not in horloge:
            fautes.append(
                "mod.rs : `clock_gettime` ne lit plus l'horloge monotone en "
                "nanosecondes. Une horloge qui avance par marches d'une "
                "milliseconde fait tourner a vide toute boucle qui attend "
                "qu'elle avance."
            )
        if "realtime_ns()" not in horloge:
            fautes.append(
                "mod.rs : `clock_gettime` rend une horloge murale en "
                "millisecondes. Le champ des nanosecondes du `timespec` ne "
                "vaudrait plus que des multiples d'un million."
            )
        if "% 1_000_000_000" not in horloge or "/ 1_000_000_000" not in horloge:
            fautes.append(
                "mod.rs : le `timespec` rendu n'est plus decoupe en secondes "
                "et nanosecondes ; la resolution annoncee serait fausse."
            )
        if "(ms % 1000) * 1_000_000" in horloge:
            fautes.append(
                "mod.rs : le champ des nanosecondes redevient un multiple "
                "d'un million. C'est l'ecriture exacte du defaut d'origine."
            )

    duree = corps(source, "fn timespec_ms(")
    if duree is None:
        fautes.append("mod.rs : `timespec_ms` a disparu.")
    else:
        if "div_ceil" not in duree:
            fautes.append(
                "mod.rs : une duree demandee est de nouveau TRONQUEE. "
                "`nanosleep(500 us)` rendrait zero milliseconde, le sommeil "
                "rendrait la main sans dormir, et un programme qui s'endort en "
                "boucle pour quelques centaines de microsecondes tournerait a "
                "plein regime sans jamais changer de contexte."
            )
        if "max(1)" not in duree:
            fautes.append(
                "mod.rs : une duree non nulle peut de nouveau valoir zero "
                "milliseconde. POSIX garantit qu'un sommeil dure AU MOINS ce "
                "qu'on a demande."
            )
        if "if total_ns == 0" not in duree:
            fautes.append(
                "mod.rs : une duree NULLE n'est plus distinguee. Un `poll` "
                "sans attente doit continuer de ne pas attendre : l'arrondir "
                "a une milliseconde ferait dormir un appel non bloquant."
            )

    # `realtime_ns` doit s'ancrer sur la meme seconde RTC que `realtime_ms`,
    # faute de quoi les deux horloges divergeraient.
    ancre = corps(source, "pub fn realtime_ns()")
    if ancre is None:
        fautes.append("mod.rs : `realtime_ns` a disparu.")
    else:
        if "realtime_ms()" not in ancre:
            fautes.append(
                "mod.rs : `realtime_ns` ne pose plus l'ancre par le meme "
                "chemin que `realtime_ms` ; les deux horloges divergeraient."
            )
        if "monotonic_ns()" not in ancre:
            fautes.append(
                "mod.rs : `realtime_ns` n'ajoute plus le temps ecoule mesure "
                "en nanosecondes ; elle retomberait a la seconde RTC."
            )
        if "EPOCH_MONO_NS" not in ancre:
            fautes.append(
                "mod.rs : `realtime_ns` ne part plus de l'ancre NANOSECONDE. "
                "Convertir l'ancre en ticks du PIT ferait avancer l'horloge "
                "murale de tout le retard que le PIT accumule sous charge -- "
                "et c'est pour eviter ce retard que l'horloge monotone lit le "
                "TSC et non le PIT."
            )

    if fautes:
        print("horloge utilisateur : %d probleme(s)\n" % len(fautes))
        for f in fautes:
            print("  - %s\n" % f)
        return 1
    print(
        "horloge utilisateur : clock_gettime en nanosecondes sur les deux "
        "horloges, timespec decoupe sans arrondi a la milliseconde, duree "
        "demandee jamais tronquee a zero, duree nulle preservee"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
