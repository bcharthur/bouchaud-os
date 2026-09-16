#!/usr/bin/env python3
"""Garde-fou : les sondes du BSP doivent tourner AUSSI en multicoeur.

# Le defaut, mesure le 16 septembre 2026

Le handler d'IRQ0 gardait trois appels derriere une condition :

    let balanced_bsp = blackbox_cpu == 0 && online > 1;
    if !balanced_bsp {
        stall_probe_from_timer();
        ...
        watchdog_from_timer();
    }

L'intention -- sortir les diagnostics lourds du hard IRQ du BSP quand
d'autres coeurs sont disponibles -- suppose que QUELQU'UN D'AUTRE les
execute. Personne ne le fait : IRQ0 n'est livree qu'au BSP, et aucun chemin
AP n'appelle ces fonctions. Sur toute machine a plus d'un coeur, elles ne
tournaient donc NULLE PART.

Mesure, meme noyau, meme image, seul le nombre de coeurs change :

    | configuration    | [SMP-SNAPSHOT] | [SCHED-FILE] |
    |------------------|----------------|--------------|
    | porte, 4 coeurs  |              0 |            0 |
    | porte, 1 coeur   |              7 |            7 |
    | sans porte, 4    |              9 |           36 |

Le defaut ne se voyait pas parce que les campagnes QEMU tournent en `-smp 1`,
ou la condition ne s'applique pas. La machine de reference, elle, a seize
coeurs : son releve physique ne porte aucune de ces lignes.

Ce qui a ete perdu :

  * `[SMP-SNAPSHOT]` / `[SMP-STALL]` -- l'etat du gros verrou ;
  * `[SCHED-FILE]` / `[SCHED-TACHE]` -- les seules sondes qui, d'apres le
    commentaire de `signale_etat_ordonnancement`, distinguent « la tache est
    en file et son coeur dort » de « la tache attend quelque chose qui ne
    vient pas » ;
  * `[sched-watchdog]` -- le battement du bureau, c'est-a-dire l'alarme qui
    existe precisement pour nommer le blocage en cours.

# Ce qui est verifie ici

1. `stall_probe_from_timer` et `watchdog_from_timer` sont appeles sans
   condition sur le nombre de coeurs.
2. Chacune garde sa PROPRE limitation de debit. Limiter par la frequence est
   le bon controle du cout ; ne jamais appeler ne l'est pas.
3. `echantillonne_tache_bsp` RESTE garde, lui, et ce n'est pas un oubli : en
   SMP le coeur zero recoit aussi les interruptions de quantum, ou
   `reschedule.rs` fait deja la comptabilite. Compter les deux doublerait le
   temps CPU du BSP.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
TIMER = RACINE / "src/arch/x86_64/idt/timer.rs"
STALL = RACINE / "src/kernel/process/thread/diagnostic_stall.rs"
METRIQUES = RACINE / "src/kernel/process/thread/metriques.rs"


def code_seul(source):
    sans = re.sub(r"/\*.*?\*/", "", source, flags=re.S)
    return "\n".join(re.sub(r"//.*$", "", l) for l in sans.splitlines())


def main():
    fautes = []
    for chemin in (TIMER, STALL, METRIQUES):
        if not chemin.exists():
            fautes.append("fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
    if fautes:
        print("sondes multicoeur : %d probleme(s)" % len(fautes))
        for f in fautes:
            print("\n  - %s" % f)
        return 1

    timer = code_seul(TIMER.read_text(encoding="utf-8"))

    # 1. Les deux appels ne doivent plus dependre du nombre de coeurs.
    for appel, quoi in (
        ("stall_probe_from_timer()",
         "l'etat de l'ordonnancement et du gros verrou"),
        ("watchdog_from_timer()",
         "le battement du bureau, c'est-a-dire l'alarme qui nomme un blocage"),
    ):
        if appel not in timer:
            fautes.append(
                "timer.rs : `%s` a disparu du handler. %s ne serait plus "
                "releve nulle part." % (appel, quoi.capitalize())
            )
            continue
        # La ligne de l'appel, et la condition qui la precede sur la meme ligne.
        for ligne in timer.splitlines():
            if appel in ligne and "balanced_bsp" in ligne:
                fautes.append(
                    "timer.rs : `%s` est de nouveau garde par `balanced_bsp`. "
                    "IRQ0 n'est livree qu'au BSP et aucun chemin AP n'appelle "
                    "cette fonction : sur toute machine a plus d'un coeur, "
                    "elle ne tournerait NULLE PART -- ce qui a coute %s sur la "
                    "machine de reference." % (appel, quoi)
                )

    # Un bloc `if !balanced_bsp { ... }` qui engloberait de nouveau les appels.
    for m in re.finditer(r"if\s+!balanced_bsp[^\n]*\{", timer):
        reste = timer[m.end():m.end() + 900]
        profondeur = 1
        corps = []
        for c in reste:
            if c == "{":
                profondeur += 1
            elif c == "}":
                profondeur -= 1
                if profondeur == 0:
                    break
            corps.append(c)
        corps = "".join(corps)
        for appel in ("stall_probe_from_timer()", "watchdog_from_timer()"):
            if appel in corps:
                fautes.append(
                    "timer.rs : `%s` est de nouveau dans un bloc "
                    "`if !balanced_bsp`. Cette condition est vraie des que le "
                    "coeur zero n'est pas seul." % appel
                )

    # 2. Chaque sonde borne son propre cout.
    stall = code_seul(STALL.read_text(encoding="utf-8"))
    i = stall.find("pub fn stall_probe_from_timer")
    if i == -1:
        fautes.append("diagnostic_stall.rs : `stall_probe_from_timer` a disparu.")
    elif "TICKS_PER_SECOND" not in stall[i:i + 400]:
        fautes.append(
            "diagnostic_stall.rs : `stall_probe_from_timer` ne se limite plus "
            "elle-meme. Appelee a chaque tic sans borne, elle justifierait de "
            "nouveau la garde qu'on vient de retirer."
        )

    metriques = code_seul(METRIQUES.read_text(encoding="utf-8"))
    j = metriques.find("pub fn watchdog_from_timer")
    if j == -1:
        fautes.append("metriques.rs : `watchdog_from_timer` a disparu.")
    elif "periode_rapport" not in metriques[j:j + 900]:
        fautes.append(
            "metriques.rs : `watchdog_from_timer` n'espace plus ses alertes. "
            "Une alarme qui sonne a chaque tic ne se lit plus."
        )

    # 4. La cadence doit suivre le COUT REEL, pas une constante.
    stall_code = code_seul(STALL.read_text(encoding="utf-8"))
    i = stall_code.find("snapshot_period")
    if i == -1:
        fautes.append(
            "diagnostic_stall.rs : la cadence des instantanes a disparu."
        )
    else:
        fenetre = stall_code[i:i + 500]
        if "presence_com1" not in fenetre:
            fautes.append(
                "diagnostic_stall.rs : la cadence des instantanes ne depend "
                "plus du port serie. Cinq secondes se justifient quand chaque "
                "octet part par entree-sortie emulee ; sur une machine sans "
                "COM1 -- `com1=bus-flottant`, ce que dit le releve physique -- "
                "les lignes tombent dans le tambour RAM et ne coutent rien. "
                "Le releve du 16 septembre 20:08 s'arrete a 7,87 s : la sonde "
                "n'a parle QU'UNE fois, et l'instantane suivant serait tombe "
                "trois secondes trop tard."
            )
        if not re.search(r"\bTICKS_PER_SECOND\b[^*]*\}", fenetre):
            fautes.append(
                "diagnostic_stall.rs : la branche rapide ne vaut plus une "
                "seconde ; une cadence plus lente que la panne ne la voit pas."
            )

    # 3. L'exception assumee.
    if "echantillonne_tache_bsp" in timer and "balanced_bsp" not in timer:
        fautes.append(
            "timer.rs : `echantillonne_tache_bsp` n'est plus garde. En SMP le "
            "coeur zero recoit aussi les interruptions de quantum, ou "
            "reschedule.rs fait deja la comptabilite : compter les deux "
            "doublerait le temps CPU du BSP."
        )

    if fautes:
        print("sondes multicoeur : %d probleme(s)" % len(fautes))
        for f in fautes:
            print("\n  - %s" % f)
        return 1

    print(
        "sondes multicoeur : sonde d'ordonnancement et chien de garde appeles "
        "sans condition de coeurs, chacun borne par sa propre cadence."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
