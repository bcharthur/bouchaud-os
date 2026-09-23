#!/usr/bin/env python3
"""La premiere position, ou l'origine ?

BOUCHAUD_C57_LA_PREMIERE_POSITION_OU_L_ORIGINE

Deux bras, MEME binaire, deux demarrages QEMU froids, seul l'ordre change :

    bras blob :  blob_1  http_2  blob_3  http_4
    bras http :  http_1  blob_2  http_3  blob_4

Trois lectures, et une seule sera vraie :

  * le PREMIER est lent des deux cotes      -> demarrage a froid du WebWorker
  * `blob` est lent meme en second          -> l'origine est en cause
  * autre chose                             -> on rapporte sans expliquer

Fail-closed : sans les quatre releves de chaque bras, on ne conclut pas.
"""
import re
import sys

AB = re.compile(
    r"HOST_WORKER_AB rang=(?P<rang>\d+) origine=(?P<origine>\w+)"
    r" repond=(?P<repond>\d+) ms=(?P<ms>\d+)"
)
ORDRE = re.compile(r"HOST_WORKER_ORDRE ordre=(?P<ordre>\w+)")


def lis(chemin):
    try:
        texte = open(chemin, encoding="utf-8", errors="replace").read()
    except OSError as exc:
        return None, f"journal illisible : {exc}"
    releves = []
    vus = set()
    for m in AB.finditer(texte):
        cle = (int(m.group("rang")), m.group("origine"))
        if cle in vus:          # la page relaie chaque ligne deux fois
            continue
        vus.add(cle)
        releves.append({
            "rang": int(m.group("rang")),
            "origine": m.group("origine"),
            "repond": int(m.group("repond")),
            "ms": int(m.group("ms")),
        })
    o = ORDRE.search(texte)
    return {"ordre": o.group("ordre") if o else "?", "releves": releves}, None


def main():
    if len(sys.argv) < 3:
        print("usage: analyse_worker_ordre.py <bras-blob> <bras-http>",
              file=sys.stderr)
        return 2

    bras = {}
    for chemin in sys.argv[1:3]:
        d, err = lis(chemin)
        if err:
            print(f"ordre : {chemin} : {err}", file=sys.stderr)
            return 1
        if len(d["releves"]) != 4:
            print(f"ordre : {chemin} n'a que {len(d['releves'])} releve(s) sur 4",
                  file=sys.stderr)
            print("        sans les quatre, la comparaison ne tranche rien.",
                  file=sys.stderr)
            return 1
        bras[d["ordre"]] = d["releves"]

    if set(bras) != {"blob", "http"}:
        print(f"ordre : bras trouves {sorted(bras)}, attendu blob et http",
              file=sys.stderr)
        return 1

    print(f"  {'bras':>6} {'rang':>5} {'origine':>8} {'repond':>7} {'ms':>9}")
    for nom in ("blob", "http"):
        for r in sorted(bras[nom], key=lambda x: x["rang"]):
            print(f"  {nom:>6} {r['rang']:>5} {r['origine']:>8}"
                  f" {r['repond']:>7} {r['ms']:>9}")

    premier = {nom: sorted(v, key=lambda x: x["rang"])[0] for nom, v in bras.items()}
    # `blob` en seconde position, dans le bras qui commence par http.
    blob_second = next(r for r in bras["http"] if r["rang"] == 2)
    http_second = next(r for r in bras["blob"] if r["rang"] == 2)

    print()
    print(f"  cold_first_blob_ms   = {premier['blob']['ms']}")
    print(f"  warm_second_http_ms  = {http_second['ms']}")
    print(f"  cold_first_http_ms   = {premier['http']['ms']}")
    print(f"  warm_second_blob_ms  = {blob_second['ms']}")

    # Le seuil n'est pas un budget : c'est l'ecart a partir duquel « lent » et
    # « rapide » cessent d'etre discutables. Dix fois.
    def lent(a, b):
        return a >= 10 * max(b, 1)

    print()
    premiers_lents = (lent(premier["blob"]["ms"], http_second["ms"])
                      and lent(premier["http"]["ms"], blob_second["ms"]))
    blob_lent_partout = lent(blob_second["ms"], http_second["ms"])

    if premiers_lents and not blob_lent_partout:
        print("  VERDICT : le PREMIER worker est lent des deux cotes, et `blob`")
        print("  redevient rapide en seconde position.")
        print("  -> DEMARRAGE A FROID du WebWorker. L'origine n'y est pour rien.")
    elif blob_lent_partout:
        print("  VERDICT : `blob` reste lent meme en seconde position.")
        print("  -> L'ORIGINE est en cause, pas seulement le demarrage a froid.")
    else:
        print("  VERDICT : aucune des deux lectures ne s'impose sur ces nombres.")
        print("  Rapporter les mesures ; ne pas inventer d'explication.")
    print("ORDRE_WORKER_ANALYSE_OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
