#!/usr/bin/env python3
"""Cout PAR LANCEMENT du chemin FichierPrive, par difference.

BOUCHAUD_C51_OU_SONT_LES_HUIT_SECONDES

Les compteurs du noyau sont CUMULATIFS. C'est voulu : les remettre a zero a
chaque processus perdrait l'information des que deux se chevauchent. Mais cela
veut dire qu'additionner les totaux compterait chaque faute autant de fois
qu'il reste de processus a sortir.

Ce script prend donc les DIFFERENCES entre deux sorties successives. La
premiere ligne est prise telle quelle : rien ne la precede.
"""
import re
import sys

LIGNE = re.compile(
    r"FAULT_FILE_BREAKDOWN t=(\d+) pid=(-?\d+) faults=(\d+)"
    r" total_us=(\d+) wait_us=(\d+) cache_us=(\d+)"
    r" backing_us=(\d+) mm_lock_us=(\d+) map_us=(\d+)"
    r" explique_us=(\d+) residual_us=(\d+)"
    r" ata_reads=(\d+) ata_bytes=(\d+) ata_us=(\d+)"
    r" mem_reads=(\d+) mem_bytes=(\d+) mem_us=(\d+)"
    r" cc_hits=(\d+) cc_miss=(\d+) cc_waits=(\d+) cc_hit_us=(\d+)"
    r" cc_miss_us=(\d+) cc_miss_read_us=(\d+) cc_wait_us=(\d+)"
)
CHAMPS = ["t", "pid", "faults", "total", "wait", "cache", "backing", "mm",
          "map", "explique", "residual", "ata_reads", "ata_bytes", "ata_us",
          "mem_reads", "mem_bytes", "mem_us",
          "cc_hits", "cc_miss", "cc_waits", "cc_hit_us", "cc_miss_us",
          "cc_miss_read_us", "cc_wait_us"]


def main():
    if len(sys.argv) < 2:
        print("usage: analyse_faute_fichier.py <journal>", file=sys.stderr)
        return 2
    texte = open(sys.argv[1], encoding="utf-8", errors="replace").read()
    lignes = [dict(zip(CHAMPS, map(int, m.groups()))) for m in LIGNE.finditer(texte)]
    if not lignes:
        print("  (aucune ligne FAULT_FILE_BREAKDOWN)")
        return 1

    def ms(v):
        return f"{v / 1000:.1f}"

    print(f"  {'#':>2} {'pid':>4} {'fautes':>7} {'total_ms':>9} {'attente':>8}"
          f" {'acquire':>8} {'cc_miss':>8} {'cc_lect':>8} {'cc_att':>7}"
          f" {'map':>6} {'residu':>7} {'ata_n':>6} {'ata_ms':>8}")
    precedent = None
    for i, l in enumerate(lignes, 1):
        if precedent is None:
            d = dict(l)
        else:
            d = {k: l[k] - precedent[k] for k in l}
            d["pid"] = l["pid"]
        print(f"  {i:>2} {d['pid']:>4} {d['faults']:>7} {ms(d['total']):>9}"
              f" {ms(d['wait']):>8} {ms(d['cache']):>8}"
              f" {ms(d['cc_miss_us']):>8} {ms(d['cc_miss_read_us']):>8}"
              f" {ms(d['cc_wait_us']):>7}"
              f" {ms(d['map']):>6} {ms(d['residual']):>7}"
              f" {d['ata_reads']:>6} {ms(d['ata_us']):>8}")
        precedent = l

    print()
    print("  `acquire` = temps total dans clean_page_cache::acquire, vu du")
    print("  chemin de faute. `cc_miss` en est la part des defauts, dont")
    print("  `cc_lect` est la LECTURE DU SUPPORT qu'`acquire` fait lui-meme --")
    print("  c'est du disque, pas du cache. `cc_att` est l'attente d'un autre")
    print("  coeur deja en train de charger la meme page.")
    print()
    print("  `total` est la latence VECUE par CE processus, attente comprise.")
    print("  Les colonnes `cc_*` et `ata_*` sont SYSTEME : elles comptent le")
    print("  travail de tous les coeurs sur l'intervalle. Une difference entre")
    print("  les deux n'est pas une incoherence -- elle dit qu'une partie du")
    print("  chargement a ete payee hors des fautes de ce processus (lecture")
    print("  anticipee, chargement de l'image par execve).")
    print("  `residu` est ce que la decomposition n'explique pas : publie,")
    print("  jamais reparti.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
