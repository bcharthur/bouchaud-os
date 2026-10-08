#!/usr/bin/env python3
"""Tendance memoire par PROCESSUS, depuis les releves [PERF-PROC] du noyau.

BOUCHAUD_TENDANCE_RSS_V1 (convergence P6).

    tools/ci/tendance_rss.py SERIE_PROPRE [--min-releves N] [--prefixe TEXTE]

Le premier et le dernier releve d'une IMAGE melangent des processus
differents (un WebContent qui meurt, un autre qui nait). Ici chaque pid est
suivi seul ; pour ceux qui vivent assez longtemps, la pente de la SECONDE
moitie de leur vie (moindres carres, kio/min) separe une montee de
demarrage (caches qui se remplissent) d'une croissance qui ne s'arrete pas.

Sortie : une ligne `RSS_TENDANCE pid= image= releves= vie_s= premier_kio=
milieu_kio= dernier_kio= max_kio= pente2_kio_min=` par pid suivi. Mesure
seulement : ce script n'echoue que sur une entree illisible.
"""
import re
import sys

LIGNE = re.compile(r"\[PERF-PROC\] t=(\d+) pid=(\d+) image=(\S+) rss_kio=(\d+)")
# BOUCHAUD_PERF_PROC_CPU_V1 : temps processeur cumule (absent des noyaux
# anterieurs : la colonne cpu vaut alors « ? »).
CPU = re.compile(r" user_ms=(\d+) sys_ms=(\d+)")


def pente(points):
    n = len(points)
    if n < 2:
        return 0.0
    mx = sum(t for t, _ in points) / n
    my = sum(r for _, r in points) / n
    den = sum((t - mx) ** 2 for t, _ in points)
    if den == 0:
        return 0.0
    return sum((t - mx) * (r - my) for t, r in points) / den


def main(argv):
    if len(argv) < 2:
        print(__doc__.strip().splitlines()[2], file=sys.stderr)
        return 2
    min_releves = 6
    if "--min-releves" in argv:
        min_releves = int(argv[argv.index("--min-releves") + 1])
    # Le banc indentait par `| sed` : sous `pipefail`, une erreur ici l'aurait
    # arrete avant son verdict. Le prefixe est donc pose ici.
    prefixe = argv[argv.index("--prefixe") + 1] if "--prefixe" in argv else ""
    par_pid = {}
    with open(argv[1], "r", errors="replace") as f:
        for ligne in f:
            m = LIGNE.search(ligne)
            if not m:
                continue
            t, pid, image, rss = int(m[1]), int(m[2]), m[3].rsplit("/", 1)[-1], int(m[4])
            # Un pid recycle par une autre image est un autre processus.
            cle = (pid, image)
            c = CPU.search(ligne)
            cpu_ms = int(c[1]) + int(c[2]) if c else None
            par_pid.setdefault(cle, []).append((t, rss, cpu_ms))
    suivis = 0
    for (pid, image), pts in sorted(par_pid.items(), key=lambda kv: kv[1][0][0]):
        if len(pts) < min_releves:
            continue
        suivis += 1
        pts.sort()
        moitie = pts[len(pts) // 2:]
        # t en ms -> pente en kio par minute
        p2 = pente([(t / 60000.0, r) for t, r, _ in moitie])
        # Part d'UN coeur sur la vie observee, et sur la seconde moitie.
        def part(a, b):
            if a[2] is None or b[2] is None or b[0] <= a[0]:
                return "?"
            return f"{100.0 * (b[2] - a[2]) / (b[0] - a[0]):.0f}"
        cpu = part(pts[0], pts[-1])
        cpu2 = part(moitie[0], moitie[-1])
        print(
            f"{prefixe}RSS_TENDANCE pid={pid} image={image} releves={len(pts)} "
            f"vie_s={(pts[-1][0] - pts[0][0]) // 1000} premier_kio={pts[0][1]} "
            f"milieu_kio={pts[len(pts) // 2][1]} dernier_kio={pts[-1][1]} "
            f"max_kio={max(r for _, r, _ in pts)} pente2_kio_min={p2:.0f} "
            f"cpu_pct={cpu} cpu2_pct={cpu2}"
        )
    print(f"{prefixe}RSS_TENDANCE_FIN pids={len(par_pid)} suivis={suivis} min_releves={min_releves}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
