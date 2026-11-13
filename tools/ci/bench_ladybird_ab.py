#!/usr/bin/env python3
"""BOUCHAUD_PERF_V1 : A/B de Ladybird reel, PLUSIEURS demarrages par bras.

  bench_ladybird_ab.py ENVIRONNEMENT BRAS_A=log1,log2,... BRAS_B=log1,log2,...

Chaque journal est un demarrage a froid du smoke BrowserHost
(tools/ci/run_ladybird_browser_host.sh), meme noyau, meme artefact, meme
page ; seul le bras change. Pour chaque mesure : mediane et P95 SUR LES
DEMARRAGES (une valeur par demarrage), et le nombre de demarrages qui l'ont
produite. Une mesure absente est `n/m`, jamais inventee. L'ENVIRONNEMENT est
imprime tel quel (TCG, KVM, WHPX et materiel reel ne se melangent pas).

Sortie : un tableau Markdown, des lignes `LADYBIRD_AB cle=... bras=... p50=... p95=... n=...`,
puis `LADYBIRD_AB_OK` si chaque bras a au moins 2 demarrages fonctionnels.
Aucun seuil de gain n'est impose : le banc MESURE ; la conclusion est ecrite
dans la PR avec les chiffres.
"""
import math
import re
import statistics
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import bench_ladybird as bl  # noqa: E402


def derniere(t, balise):
    lignes = [l for l in t.splitlines() if balise in l]
    if not lignes:
        return {}
    return {k: int(v) for k, v in re.findall(r"(\w+)=(\d+)", lignes[-1].split(balise, 1)[1])}


def mesures(t):
    m = bl.mesures(t)
    perf = derniere(t, "[LB:PERF]")
    rendu = derniere(t, "M11_RENDER_STATS")
    bilan = derniere(t, "GUI_SURFACE_BILAN")
    tous_present = [int(v) for v in re.findall(r"\[LB:PERF\] .*present_us_moy=(\d+)", t)]
    m.update({
        "present_us_moy_fin": perf.get("present_us_moy"),
        "present_us_pire": perf.get("present_us_pire"),
        "present_us_moy_mediane_releves": statistics.median(tous_present) if tous_present else None,
        "trames_presentees": perf.get("trames"),
        "copie_px": perf.get("copie_px"),
        "trames_partielles": perf.get("trames_partielles"),
        "trames_completes": perf.get("trames_completes"),
        "tampons_retenus": rendu.get("tampons"),
        "attentes_de_tampon": rendu.get("attentes"),
        "liberations": rendu.get("liberations"),
        "compositions_mixtes": bilan.get("compositions_mixtes"),
        "octets_serie": len(t.encode("utf-8", "replace")),
    })
    return m


LIBELLES = [
    ("smoke_ok", "Smoke fonctionnel (1 = OK)", ""),
    ("boot_bureau_ms", "Démarrage → première trame du bureau", "ms"),
    ("lancement_premiere_image_ms", "Lancement du navigateur → première image", "ms"),
    ("premiere_trame_client_ms", "Processus navigateur → première trame cliente", "ms"),
    ("webcontent_demarrage_ms", "Démarrage à froid de WebContent", "ms"),
    ("present_us_moy_mediane_releves", "Présentation moyenne (médiane des relevés)", "µs"),
    ("present_us_pire", "Présentation, pire cas du dernier relevé", "µs"),
    ("trames_presentees", "Trames présentées (dernier relevé)", ""),
    ("copie_px", "Pixels recopiés (dernier relevé)", "px"),
    ("tampons_retenus", "Tampons retenus par le chrome", ""),
    ("attentes_de_tampon", "Attentes de tampon libre", ""),
    ("compositions_mixtes", "Compositions mixtes (déchirure lue)", ""),
    ("rss_somme_des_max_kio", "Mémoire : somme des RSS max", "Kio"),
    ("octets_serie", "Volume du journal série", "o"),
]


def quantile(v, q):
    v = sorted(v)
    return v[max(0, min(len(v) - 1, math.ceil(q * len(v)) - 1))]


def resume(valeurs):
    v = [x for x in valeurs if x is not None]
    if not v:
        return None, None, 0
    return statistics.median(v), quantile(v, 0.95), len(v)


def f(x, u):
    if x is None:
        return "n/m"
    return (f"{x:,.0f}".replace(",", " ") + (f" {u}" if u else "")).strip()


def main(argv):
    if len(argv) < 4:
        print(__doc__, file=sys.stderr)
        return 2
    env = argv[1]
    bras = []
    for a in argv[2:]:
        nom, _, logs = a.partition("=")
        textes = [bl.texte(p) for p in logs.split(",") if p]
        bras.append((nom, [mesures(t) for t in textes if t]))
    print(f"Environnement : **{env}**. Médiane / P95 sur les démarrages à froid (n = démarrages mesurés).\n")
    print("| Mesure | " + " | ".join(f"{n} p50 / p95 (n)" for n, _ in bras) + " |")
    print("|---|" + "---:|" * len(bras))
    for cle, lib, u in LIBELLES:
        cellules = []
        for nom, ms in bras:
            p50, p95, n = resume([m.get(cle) for m in ms])
            cellules.append(f"{f(p50, u)} / {f(p95, u)} ({n})")
            print(f"LADYBIRD_AB cle={cle} bras={nom} p50={p50} p95={p95} n={n}", file=sys.stderr)
        print(f"| {lib} | " + " | ".join(cellules) + " |")
    print("| Latence entrée → présentation | non instrumentée | non instrumentée |")
    ok = all(sum(1 for m in ms if m.get("smoke_ok")) >= 2 for _, ms in bras)
    print("\nLADYBIRD_AB_OK" if ok else "\nLADYBIRD_AB_INSUFFISANT (moins de 2 démarrages fonctionnels dans un bras)")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
