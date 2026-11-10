#!/usr/bin/env python3
"""Benchmark honnete de Bouchaud OS, tire des journaux du banc GPU.

  bench_bouchaud.py DOSSIER   (sortie de tools/ci/run_gpu_composition.sh)

Imprime un tableau Markdown et des lignes `BENCH cle=valeur`. Chaque valeur
est une MEDIANE des releves de 30 s (le premier, qui contient le demarrage,
est ecarte), et chaque ligne dit d'ou elle vient. Une mesure absente est
ecrite `n/m`, jamais inventee.
"""
import re
import statistics
import sys
from pathlib import Path

ANSI = re.compile(r"\x1b\[[0-9;]*m")


def texte(p):
    try:
        return ANSI.sub("", p.read_bytes().decode("utf-8", "replace"))
    except OSError:
        return ""


def releves(t):
    out = []
    for l in t.splitlines():
        if "[GPU-COMPOSITION]" in l:
            out.append({k: int(v) for k, v in re.findall(r"(\w+)=(\d+)", l.split("[GPU-COMPOSITION]", 1)[1])})
    return out[1:]


def med(vals):
    vals = [v for v in vals if v is not None]
    return statistics.median(vals) if vals else None


def champ(rs, cle):
    return med([r.get(cle) for r in rs])


def boot_ms(t):
    m = re.search(r"BOUCHAUD_BOOT_POINT bureau-premiere-trame t_ms=(\d+)", t)
    return int(m[1]) if m else None


def premiere_trame_client(t):
    m = re.search(r"premiere trame du client pid=\d+ \(\d+x\d+\) apres (\d+) ms", t)
    return int(m[1]) if m else None


def f(v, unite="", div=1, dec=0):
    if v is None:
        return "n/m"
    v = v / div
    return f"{v:,.{dec}f}".replace(",", " ") + unite


def main():
    d = Path(sys.argv[1])
    g, c, o, r, mire = (texte(d / n / "serie.log") for n in ("ab-gpu", "ab-cpu", "occlusion", "repli", "mire"))
    rg, rc, ro, rr = releves(g), releves(c), releves(o), releves(r)
    boots = [b for b in (boot_ms(x) for x in (g, c, o, r, mire)) if b is not None]
    lignes = [
        ("Démarrage : noyau → première trame du bureau", f(med(boots), " ms"), "5 démarrages, médiane"),
        ("DOOM : lancement → première image", f(med([premiere_trame_client(g), premiere_trame_client(c)]), " ms"), "bras GPU et CPU"),
        ("DOOM : trames du bureau par seconde — GPU", f(champ(rg, "trames_bureau"), " /s", 30, 1), "composition GPU"),
        ("DOOM : trames du bureau par seconde — CPU", f(champ(rc, "trames_bureau"), " /s", 30, 1), "composition processeur"),
        ("Temps CPU de dessin du bureau par trame — GPU", f(champ(rg, "us_dessin_cpu_par_trame"), " µs"), "fil du bureau"),
        ("Temps CPU de dessin du bureau par trame — CPU", f(champ(rc, "us_dessin_cpu_par_trame"), " µs"), "fil du bureau"),
        ("Pixels clients recopiés par le CPU — GPU", f(champ(rg, "px_client_copies_cpu"), " /s", 30), "sous le curseur seulement"),
        ("Pixels clients recopiés par le CPU — CPU", f(champ(rc, "px_client_copies_cpu"), " /s", 30), "toute la fenêtre"),
        ("Présentation par trame — GPU (attente, le fil dort)", f(champ(rg, "us_presentation_par_trame"), " µs"), "GL logiciel de l'hôte"),
        ("Présentation par trame — CPU (VIRGL, 2D)", f(champ(rc, "us_presentation_par_trame"), " µs"), ""),
        ("Présentation par trame — virtio-gpu 2D sans VIRGL", f(champ(rr, "us_presentation_par_trame"), " µs"), "repli"),
        ("Fenêtre cachée : pixels clients CPU", f(champ(ro, "px_client_copies_cpu"), " /s", 30), "DOOM sous une autre fenêtre"),
    ]
    # BOUCHAUD_GPU_ATTRIBUTION_V1 : centiles par trame et par commande
    # (bornes hautes de classes de puissance de deux, dernier releve).
    def dernier(t, balise):
        lignes_b = [l for l in t.splitlines() if balise in l]
        return lignes_b[-1].split(balise, 1)[1] if lignes_b else ""
    for nom_bras, t in (("GPU", g), ("CPU", c), ("2D sans VIRGL", r)):
        m = re.search(r"presentation_us_p50=(\d+) p95=(\d+) p99=(\d+) max=(\d+)", dernier(t, "[GPU-TRAMES]"))
        if m:
            lignes.append((f"Présentation par trame p50/p95/p99 — {nom_bras}",
                           f"≤ {m[1]}/{m[2]}/{m[3]} µs", f"max {m[4]} µs depuis le démarrage ; bornes hautes de classes"))
        m = re.search(r"dessin_cpu_us_p50=(\d+) p95=(\d+) p99=(\d+) max=(\d+)", dernier(t, "[GPU-TRAMES]"))
        if m:
            lignes.append((f"Dessin CPU par trame p50/p95/p99 — {nom_bras}",
                           f"≤ {m[1]}/{m[2]}/{m[3]} µs", f"max {m[4]} µs depuis le démarrage"))
        cmd = re.findall(r"(\w+)=(\d+):(\d+)/(\d+)/(\d+)/(\d+)", dernier(t, "[GPU-COMMANDES]"))
        for nom_c, n, p50, p95, p99, mx in cmd:
            lignes.append((f"Commande {nom_c} p50/p95/p99 — {nom_bras}", f"≤ {p50}/{p95}/{p99} µs", f"{n} commandes sur 30 s, max {mx} µs depuis le démarrage"))
    print("| Mesure | Valeur | Conditions |")
    print("|---|---:|---|")
    for nom, val, cond in lignes:
        print(f"| {nom} | {val} | {cond} |")
    for nom, val, _ in lignes:
        print("BENCH " + re.sub(r"\W+", "_", nom).strip("_").lower() + "=" + val.replace(" ", ""))


if __name__ == "__main__":
    main()
