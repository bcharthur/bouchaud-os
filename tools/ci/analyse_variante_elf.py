#!/usr/bin/env python3
"""Avant / apres de liaison, sur les bornes qui comptent.

BOUCHAUD_C67_LA_TABLE_DE_RELOCATION_DU_DEMARRAGE

La cible est `execve fin -> main`, mesuree a ~92 s sur les deux bras du banc
d'ordre du run 35912027322 -- donc independante de l'origine du script.

Un `-static-pie` glibc applique ses relocations dans `_dl_relocate_static_pie`
AVANT `main`. Le bras RELR encode les memes relocations dans un champ de bits
`DT_RELR` : il supprime le PARCOURS de la table (9,7 Mio de `.rela.dyn`) et
garde les 405 396 ECRITURES. Le banc separe donc deux couts qu'on confondait.

Si `exec_fin -> main` s'effondre, le cout etait la lecture de la table, donc
des fautes de page. S'il ne bouge pas, ce sont les ecritures.

La variante ET_EXEC, qui aurait supprime les deux, est REFUTEE : avec la glibc
statique, lier a 0x400000000000 echoue (`R_X86_64_PLT32` tronque contre les
symboles faibles indefinis de `libc.a`). Le detail est dans
`tools/ladybird/prepare-browser-runtime-link.py`.

Fail-closed : sans les bornes des deux bras, on ne conclut pas.
"""
import re
import sys

ETAPE = re.compile(
    r"WORKER_ETAPE t=(?P<t>\d+)(?: agent=\S+)? etape=(?P<etape>\w+)"
)
EXECVE = re.compile(r"PERF_EXECVE t=(?P<t>\d+) image=(?P<image>\S*WebWorker) ")
AB = re.compile(r"HOST_WORKER_AB rang=1 origine=(?P<o>\w+) repond=\d+ ms=(?P<ms>\d+)")
AB4 = re.compile(r"HOST_WORKER_AB rang=(?P<rang>\d+) \S+ repond=\d+ ms=(?P<ms>\d+)")


def bornes(chemin):
    try:
        texte = open(chemin, encoding="utf-8", errors="replace").read()
    except OSError as exc:
        return None, f"journal illisible : {exc}"

    b = {}
    m = EXECVE.search(texte)
    if m:
        b["execve_fin"] = int(m.group("t"))
    # Le PREMIER worker seulement : c'est lui le cas froid.
    for e in ETAPE.finditer(texte):
        nom = e.group("etape")
        if nom not in b:
            b[nom] = int(e.group("t"))
    a = AB.search(texte)
    if a:
        b["cold_total_ms"] = int(a.group("ms"))
    chauds = [int(x.group("ms")) for x in AB4.finditer(texte)
              if x.group("rang") != "1"]
    if chauds:
        b["warm_max_ms"] = max(chauds)
    return b, None


SEGMENTS = [
    ("exec_fin -> processus_lance", "execve_fin", "processus_lance"),
    ("processus_lance -> main", "processus_lance", "main"),
    ("exec_fin -> main", "execve_fin", "main"),
    ("main -> ipc_pret", "main", "ipc_pret"),
    ("boucle -> script_charge", "boucle_prete", "script_charge"),
]


def main():
    if len(sys.argv) < 3:
        print("usage: analyse_variante_elf.py <bras-pie> <bras-relr>",
              file=sys.stderr)
        return 2
    bras = {}
    for nom, chemin in (("PIE", sys.argv[1]), ("RELR", sys.argv[2])):
        b, err = bornes(chemin)
        if err:
            print(f"variante : {nom} : {err}", file=sys.stderr)
            return 1
        if "execve_fin" not in b or "main" not in b:
            print(f"variante : bras {nom} sans les bornes execve_fin/main",
                  file=sys.stderr)
            print(f"           bornes trouvees : {sorted(b)}", file=sys.stderr)
            print("           sans elles, l'avant/apres ne tranche rien.",
                  file=sys.stderr)
            return 1
        bras[nom] = b

    print(f"  {'segment':<30} {'PIE (ms)':>12} {'RELR (ms)':>14} {'delta':>12}")
    dominant = None
    for libelle, debut, fin in SEGMENTS:
        vals = {}
        for nom in ("PIE", "RELR"):
            b = bras[nom]
            vals[nom] = (b[fin] - b[debut]) if (debut in b and fin in b) else None
        if vals["PIE"] is None or vals["RELR"] is None:
            print(f"  {libelle:<30} {'-':>12} {'-':>14} {'-':>12}")
            continue
        delta = vals["RELR"] - vals["PIE"]
        print(f"  {libelle:<30} {vals['PIE']:>12} {vals['RELR']:>14} {delta:>+12}")
        if libelle == "exec_fin -> main":
            dominant = (vals["PIE"], vals["RELR"])

    print()
    for cle, libelle in (("cold_total_ms", "cold premier worker"),
                         ("warm_max_ms", "pire worker chaud")):
        a = bras["PIE"].get(cle)
        b = bras["RELR"].get(cle)
        if a is None or b is None:
            print(f"  {libelle:<24} {'-':>12} {'-':>14}")
        else:
            print(f"  {libelle:<24} {a:>12} {b:>14} {b - a:>+12}")

    print()
    if dominant is None:
        print("  Segment dominant non mesurable : rien n'est confirme.")
        return 1
    pie, exe = dominant
    if pie > 0 and exe <= pie // 2:
        print(f"  CAUSE CONFIRMEE : `exec_fin -> main` passe de {pie} a {exe} ms.")
        print("  Les relocations de demarrage du static-PIE en etaient bien")
        print("  la cause dominante.")
    elif pie > 0 and exe >= pie * 8 // 10:
        print(f"  HYPOTHESE REFUTEE : {pie} -> {exe} ms, l'essentiel demeure.")
        print("  Les relocations ne sont PAS la cause dominante ; instrumenter")
        print("  `_init`, `.init_array` et `__libc_start_call_main` avant")
        print("  d'optimiser quoi que ce soit.")
    else:
        print(f"  PARTIEL : {pie} -> {exe} ms. Le gain est reel mais n'explique")
        print("  pas tout ; rapporter les nombres sans trancher.")
    print("VARIANTE_ELF_ANALYSE_OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
