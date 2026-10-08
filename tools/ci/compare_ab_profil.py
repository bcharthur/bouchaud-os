#!/usr/bin/env python3
"""Compare deux endurances, profil RIP actif (A) et coupe (B).

BOUCHAUD_PROFIL_RIP_V1

    tools/ci/compare_ab_profil.py SORTIE_A SORTIE_B

Lit la sortie de run_ladybird_endurance.sh de chaque bras (ligne
HOST_ENDURANCE_FIN, APPELS_TENDANCE, RSS_TENDANCE) et imprime, metrique par
metrique, A, B et l'ecart relatif (A - B) / B :

    PROFIL_RIP_OVERHEAD metrique=lat_cadre_p50_ms a=... b=... ecart_pct=...

puis PROFIL_RIP_OVERHEAD_SYNTHESE avec le plus grand ecart DEFAVORABLE sur
les metriques cles. Un seul run par bras : c'est une mesure, pas une
statistique -- l'ecart entre deux runs identiques est du meme ordre et doit
etre lu comme tel. Rend 1 si un bras n'a pas de ligne FIN, 0 sinon.
"""
import re
import sys
from pathlib import Path

FIN = re.compile(r"HOST_ENDURANCE_FIN (.*)")
APPELS = re.compile(r"APPELS_TENDANCE pid=\d+ image=(\S+) vie_s=(\d+) appels_par_s=(\d+)")
RSS = re.compile(r"RSS_TENDANCE pid=\d+ image=(\S+) releves=\d+ vie_s=(\d+) .*?dernier_kio=(\d+) .*?cpu_pct=(\S+)")
# Plus grand = pire, pour ces metriques ; pour `cycles` et `raf`, plus petit = pire.
CLES = ["cycles", "lat_cadre_p50_ms", "lat_cadre_p95_ms", "lat_cadre_p99_ms", "lat_doc_moy_ms",
        "lat_img_moy_ms", "retard_boucle_moy_ms", "raf"]
PLUS_PETIT_PIRE = {"cycles", "raf"}


def lit(chemin: Path) -> dict:
    mesures = {}
    texte = chemin.read_text(errors="replace") if chemin.is_file() else ""
    fin = FIN.search(texte)
    if fin:
        for champ in fin[1].replace('"', "").split():
            if "=" in champ:
                cle, valeur = champ.split("=", 1)
                if re.fullmatch(r"-?\d+", valeur):
                    mesures[cle] = int(valeur)
    # Le processus de plus longue vie, par image : le service, pas un onglet.
    for m in APPELS.finditer(texte):
        cle = f"appels_par_s_{m[1]}"
        if int(m[2]) >= mesures.get(f"_vie_{cle}", -1):
            mesures[f"_vie_{cle}"] = int(m[2])
            mesures[cle] = int(m[3])
    for m in RSS.finditer(texte):
        for cle, valeur in ((f"rss_dernier_kio_{m[1]}", m[3]), (f"cpu_pct_{m[1]}", m[4])):
            if re.fullmatch(r"\d+", valeur) and int(m[2]) >= mesures.get(f"_vie_{cle}", -1):
                mesures[f"_vie_{cle}"] = int(m[2])
                mesures[cle] = int(valeur)
    return mesures


def main(argv) -> int:
    if len(argv) != 3:
        print(__doc__.strip().splitlines()[4], file=sys.stderr)
        return 2
    a, b = lit(Path(argv[1])), lit(Path(argv[2]))
    if "cycles" not in a or "cycles" not in b:
        print(f"PROFIL_RIP_OVERHEAD_INCONCLUSIF fin_a={'cycles' in a} fin_b={'cycles' in b}")
        return 1
    pire = (0.0, "-")
    for cle in sorted(set(a) & set(b)):
        if cle.startswith("_"):
            continue
        va, vb = a[cle], b[cle]
        ecart = 100.0 * (va - vb) / vb if vb else 0.0
        print(f"PROFIL_RIP_OVERHEAD metrique={cle} a={va} b={vb} ecart_pct={ecart:+.1f}")
        if cle in CLES:
            defavorable = -ecart if cle in PLUS_PETIT_PIRE else ecart
            if defavorable > pire[0]:
                pire = (defavorable, cle)
    print(f"PROFIL_RIP_OVERHEAD_SYNTHESE pire_ecart_defavorable_pct={pire[0]:.1f} metrique={pire[1]} "
          "(un run par bras : a lire contre l'ecart entre deux runs identiques)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
