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

# LA PANNE DU RUN 35907201865, RECONNUE POUR CE QU'ELLE EST.
#
# BOUCHAUD_C64_EXPLIQUER_LA_PANNE_PAS_LA_COMPTER
#
# Ce bras-la n'avait pas « 0 releve sur 4 » : il avait une cause, ecrite dans
# le journal quatre lignes plus bas. Le premier worker n'a jamais franchi son
# constructeur, `desktop` est revenu, l'autorun s'est termine et la machine
# s'est eteinte.
#
# Un diagnostic qui compte les absents sans lire la raison deja presente fait
# recommencer l'enquete a chaque run.
ETAPE = re.compile(
    r"HOST_WORKER_ETAPE origine=(?P<origine>\w+)_(?P<rang>\d+) etape=(?P<etape>\w+)"
)
SORTIE = re.compile(r"BOUCHAUD_SYSTEM_EXIT raison=(?P<raison>\S+)")
DESKTOP_RETOUR = re.compile(r"AUTORUN_DESKTOP_RETURN statut=(?P<statut>-?\d+)")
RUN_NOYAU = re.compile(
    r"RUN_NOYAU_RETOUR .*?fil_mort=(?P<mort>\d+) processus_vivants=(?P<vivants>\d+)"
)


def diagnostique_panne(texte, ordre):
    """Explique POURQUOI un bras n'a pas produit ses quatre mesures."""
    lignes = []
    etapes = list(ETAPE.finditer(texte))
    derniere = etapes[-1] if etapes else None
    if derniere:
        lignes.append(f"    derniere etape atteinte : rang={derniere.group('rang')}"
                      f" origine={derniere.group('origine')}"
                      f" phase={derniere.group('etape')}")
    else:
        lignes.append("    aucune etape de worker atteinte : la page n'a pas"
                      " commence la matrice")

    d = DESKTOP_RETOUR.search(texte)
    if d:
        lignes.append(f"    desktop est REVENU (statut={d.group('statut')}) :"
                      " le bureau a rendu la main")
    r = RUN_NOYAU.search(texte)
    if r:
        mort = r.group("mort") == "1"
        lignes.append(f"    run_noyau : fil_mort={r.group('mort')}"
                      f" processus_vivants={r.group('vivants')}")
        if not mort:
            lignes.append("    LE FIL BUREAU ETAIT ENCORE VIVANT : sortie"
                          " accidentelle, pas un arret demande")
    x = SORTIE.search(texte)
    if x:
        lignes.append(f"    extinction : raison={x.group('raison')}")

    rang = int(derniere.group("rang")) if derniere else 1
    phase = derniere.group("etape") if derniere else "avant_premier"
    raison = x.group("raison") if x else "inconnue"
    lignes.append(f"    ORDRE_WORKER_BANC_LIFETIME_FAIL ordre={ordre}"
                  f" rang={rang} phase={phase} reason={raison}")
    return lignes


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
            # Et surtout : POURQUOI. La raison est deja dans le journal.
            texte = open(chemin, encoding="utf-8", errors="replace").read()
            for l in diagnostique_panne(texte, d["ordre"]):
                print(l, file=sys.stderr)
            return 1
        # UN BRAS DUPLIQUE N'EST PAS UNE COLLISION, C'EST UNE EXPERIENCE
        # QUI N'A PAS EU LIEU.
        #
        # La version precedente ecrasait silencieusement le premier bras par
        # le second. Au run 35900151523, les deux boots etaient en `blob` --
        # l'URL n'emportait pas son parametre -- et le seul symptome fut
        # « bras trouves ['blob'] », plusieurs etapes plus loin.
        if d["ordre"] in bras:
            print(f"ordre : bras duplique {d['ordre']} ; la seconde execution"
                  " n'a probablement pas recu son ordre", file=sys.stderr)
            print("        verifier BO_SMOKE_URL et HOST_WORKER_ORDRE dans"
                  " les deux journaux.", file=sys.stderr)
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
