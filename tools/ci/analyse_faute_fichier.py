#!/usr/bin/env python3
"""Decomposition du chemin FichierPrive, PAR PROCESSUS.

BOUCHAUD_C54_PAR_PID_OU_RIEN

Les chiffres viennent du livre par PID, alimente faute par faute depuis la
`Note` qui connait son processus. Ce script ne fait AUCUNE difference entre
lignes : chaque ligne porte deja le total de SON processus.

La version precedente soustrayait deux releves globaux successifs, ce qui
supposait que les processus ne se chevauchent pas -- faux par definition dans
le cas qu'on veut mesurer.

# Ce qui est additif et ce qui ne l'est pas

`acquire_us` CONTIENT `acquire_miss_read_us` : l'acquisition du cache de pages
fait la lecture du support elle-meme. Les additionner compterait la lecture
deux fois. De meme `hit + miss + wait == acquire` : ce sont les trois ISSUES
d'une acquisition, pas trois etapes.

    explained = wait + acquire + backing_direct + mm_lock + map
"""
import re
import sys

LIGNE = re.compile(
    r"FAULT_FILE_BREAKDOWN t=(?P<t>\d+) pid=(?P<pid>\d+) source=(?P<source>\S+)"
    r" faults=(?P<faults>\d+) total_us=(?P<total>\d+) wait_us=(?P<wait>\d+)"
    r" acquire_us=(?P<acq>\d+) acquire_hit_n=(?P<hit_n>\d+)"
    r" acquire_hit_us=(?P<hit_us>\d+) acquire_miss_n=(?P<miss_n>\d+)"
    r" acquire_miss_us=(?P<miss_us>\d+) acquire_miss_read_us=(?P<read_us>\d+)"
    r" acquire_wait_n=(?P<wait_n>\d+) acquire_wait_us=(?P<wait_us2>\d+)"
    r" backing_direct_us=(?P<direct>\d+) mm_lock_us=(?P<mm>\d+)"
    r" map_us=(?P<map>\d+) explained_us=(?P<expl>\d+)"
    r" residual_us=(?P<res>\d+) residual_pct=(?P<pct>\d+)"
    r" worst_us=(?P<worst>\d+) lost_samples=(?P<lost>\d+)"
)


def main():
    if len(sys.argv) < 2:
        print("usage: analyse_faute_fichier.py <journal>", file=sys.stderr)
        return 2
    texte = open(sys.argv[1], encoding="utf-8", errors="replace").read()
    lignes = [m.groupdict() for m in LIGNE.finditer(texte)]
    if not lignes:
        brutes = texte.count("FAULT_FILE_BREAKDOWN")
        if brutes:
            print(f"  {brutes} ligne(s) presentes mais NON ANALYSABLES :",
                  file=sys.stderr)
            print("  le format de la sonde a change sans que ce script suive.",
                  file=sys.stderr)
            return 1
        print("  (aucune ligne FAULT_FILE_BREAKDOWN)")
        return 1

    def ms(v):
        return f"{int(v) / 1000:.1f}"

    print(f"  {'pid':>4} {'source':>11} {'fautes':>7} {'total_ms':>9}"
          f" {'attente':>8} {'acquire':>8} {'hit_n':>6} {'miss_n':>7}"
          f" {'dont_lect':>10} {'att_n':>6} {'direct':>7} {'mm':>6}"
          f" {'map':>6} {'residu':>8} {'%':>4} {'perdus':>7}")
    for l in lignes:
        print(f"  {l['pid']:>4} {l['source']:>11} {l['faults']:>7}"
              f" {ms(l['total']):>9} {ms(l['wait']):>8} {ms(l['acq']):>8}"
              f" {l['hit_n']:>6} {l['miss_n']:>7} {ms(l['read_us']):>10}"
              f" {l['wait_n']:>6} {ms(l['direct']):>7} {ms(l['mm']):>6}"
              f" {ms(l['map']):>6} {ms(l['res']):>8} {l['pct']:>4}"
              f" {l['lost']:>7}")

    # Un residu au-dessus de 5 % interdit de conclure : c'est la regle du
    # chantier, et l'annoncer vaut mieux que de laisser lire le tableau.
    hauts = [l for l in lignes if int(l["pct"]) > 5]
    print()
    if hauts:
        print(f"  RESIDU AU-DESSUS DE 5 % sur {len(hauts)} processus :")
        for l in hauts:
            print(f"    pid={l['pid']} residu={l['pct']} % "
                  f"({ms(l['res'])} ms sur {ms(l['total'])} ms)")
        print("  La decomposition n'explique pas ce demarrage. Instrumenter")
        print("  avant d'en tirer une conclusion d'optimisation.")
    else:
        print("  Tous les residus sont sous 5 %.")

    perdus = max(int(l["lost"]) for l in lignes)
    if perdus:
        print(f"  {perdus} echantillon(s) perdus : les totaux sont des")
        print("  PLANCHERS, pas des totaux.")
    print()
    print("  `acquire` contient `dont_lect` : ne pas les additionner.")
    print("  hit_n + miss_n + att_n = nombre d'acquisitions, pas d'etapes.")
    print("  explique = attente + acquire + direct + mm + map")
    return 0


if __name__ == "__main__":
    sys.exit(main())
