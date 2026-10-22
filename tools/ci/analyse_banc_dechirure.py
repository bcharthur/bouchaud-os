#!/usr/bin/env python3
"""Analyse du banc de dechirure du contrat de surface client
(BOUCHAUD_GUI_SURFACE_DEUX_TAMPONS_V1, P10+.3c).

    analyse_banc_dechirure.py JOURNAL_SERIE TOURS SMP ACCEL

Un demarrage par bras (`tools/ci/run_banc_dechirure.sh`), TOURS tours, ordre
v1/v2 aux tours impairs, v2/v1 aux pairs. La sonde `surface-probe` peint des
trames d'une seule couleur pendant une duree fixe ; l'instrument du
compositeur compte les compositions qui ont lu une trame non uniforme
(`compositions_mixtes`) et le noyau publie `GUI_SURFACE_BILAN` a la mort du
client.

CRITERES DECLARES AVANT LA MESURE (l'offre de deux tampons reste OFF par
defaut ; elle ne change qu'avec un A/B KVM CONVAINCANT) :

  surete     chaque bras : sonde sortie sans defaut, bilan publie, au moins
             20 compositions lues (l'instrument a vu la surface) ; v1 : un
             tampon ; v2 : deux tampons des deux cotes, AUCUNE composition
             mixte, aucune trame hors bornes, aucune ecriture sur la face ni
             sans liberation, au moins (trames - 2) liberations ; aucune
             panique ;
  temoin     mediane (sur les tours) des compositions mixtes v1 > 0 : le banc
             a reellement exerce la dechirure du contrat d'avant -- sinon
             NON_CONCLUANT (un zero en v2 ne prouverait rien) ;
  debit      mediane des compositions lues par seconde : v2 >= 0,80 x v1 (le
             contrat ne doit pas couter les trames affichees) ;
  quorum     4 tours complets, sinon NON_CONCLUANT.

Sortie 1 sur un defaut de surete ou d'instrument ; 0 sinon. Teste :
tools/ci/reliability/test_banc_dechirure.py.
"""
import re
import statistics
import sys

QUORUM = 4
LECTURES_MIN = 20
DEBIT_MIN = 0.80
BRAS = re.compile(r"^BANC_DECHIRURE tour=(\d+) mode=(v1|v2)$")
SONDE = re.compile(r"SURFACE_SONDE mode=(v1|v2) tampons=(\d) trames=(\d+) duree_ms=(\d+) .*"
                   r"liberations=(\d+) .* defaut=(\d+)")
BILAN = re.compile(r"GUI_SURFACE_BILAN pid=\d+ offerts=(\d) tampons=(\d) trames=(\d+) liberations=(\d+) "
                   r"liberations_retardees=(\d+) hors_bornes=(\d+) ecritures_sur_face=(\d+) "
                   r"ecritures_sans_liberation=(\d+) compositions_lues=(\d+) compositions_mixtes=(\d+)")


def analyse(texte, tours, smp, nom):
    sortie, fautes = [], []
    texte = re.sub(r"\x1b\[[0-9;]*m", "", texte)
    lignes = [re.sub(r"^(\[[^]]*\]){2}\[FPS:[^]]*\] ", "", l).rstrip("\r") for l in texte.splitlines()]
    if re.search(r"KERNEL PANIC|panicked at", texte):
        fautes.append("panique")
    bras, courant = [], None
    for l in lignes:
        m = BRAS.match(l)
        if m:
            courant = {"tour": int(m.group(1)), "v": m.group(2), "sonde": None, "bilan": None}
            bras.append(courant)
            continue
        if courant is None:
            continue
        m = SONDE.search(l)
        if m:
            g = m.groups()
            courant["sonde"] = {"mode": g[0], "tampons": int(g[1]), "trames": int(g[2]), "duree_ms": int(g[3]),
                                "liberations": int(g[4]), "defaut": int(g[5])}
        m = BILAN.search(l)
        if m:
            g = [int(x) for x in m.groups()]
            courant["bilan"] = dict(zip(("offerts", "tampons", "trames", "liberations", "retardees", "hors_bornes",
                                         "sur_face", "sans_liberation", "lues", "mixtes"), g))
    for b in bras:
        nb = f"tour {b['tour']} {b['v']}"
        s, bi = b["sonde"], b["bilan"]
        if s is None or bi is None:
            fautes.append(f"{nb} : sonde ou bilan absent")
            continue
        if s["defaut"] != 0 or s["mode"] != b["v"]:
            fautes.append(f"{nb} : sonde en defaut ({s['defaut']}) ou sous le mauvais mode ({s['mode']})")
        if bi["lues"] < LECTURES_MIN:
            fautes.append(f"{nb} : {bi['lues']} compositions lues (< {LECTURES_MIN}) : l'instrument n'a pas vu la surface")
        attendu = 2 if b["v"] == "v2" else 1
        if s["tampons"] != attendu or bi["tampons"] != attendu:
            fautes.append(f"{nb} : contrat a {s['tampons']}/{bi['tampons']} tampon(s), attendu {attendu}")
        if b["v"] == "v2":
            if bi["mixtes"] != 0:
                fautes.append(f"{nb} : {bi['mixtes']} composition(s) mixte(s) sous le contrat a deux tampons")
            if bi["hors_bornes"] or bi["sur_face"] or bi["sans_liberation"]:
                fautes.append(f"{nb} : violation du contrat (hors_bornes={bi['hors_bornes']} sur_face={bi['sur_face']} "
                              f"sans_liberation={bi['sans_liberation']})")
            if bi["liberations"] < bi["trames"] - 2:
                fautes.append(f"{nb} : {bi['liberations']} liberations pour {bi['trames']} trames")
    for t in range(1, tours + 1):
        ordre = [b["v"] for b in bras if b["tour"] == t]
        voulu = ["v1", "v2"] if t % 2 else ["v2", "v1"]
        if ordre and ordre != voulu:
            fautes.append(f"tour {t} : ordre {ordre}, attendu {voulu} (AB/BA)")
    par_tour = {}
    for b in bras:
        if b["sonde"] is not None and b["bilan"] is not None and b["sonde"]["duree_ms"] > 0:
            par_tour.setdefault(b["tour"], {})[b["v"]] = b
    paires = sorted(t for t, d in par_tour.items() if set(d) == {"v1", "v2"})

    sortie.append(f"  banc de dechirure, {nom} SMP{smp} : {len(paires)}/{tours} tours complets")
    sortie.append(f"  {'tour':<5}{'bras':<5}{'trames':>8}{'lues':>7}{'mixtes':>8}{'lues/s':>8}{'liberations':>13}")

    def par_s(b):
        return b["bilan"]["lues"] * 1000.0 / max(1, b["sonde"]["duree_ms"])
    for b in bras:
        if b["sonde"] and b["bilan"]:
            sortie.append(f"  {b['tour']:<5}{b['v']:<5}{b['bilan']['trames']:>8}{b['bilan']['lues']:>7}"
                          f"{b['bilan']['mixtes']:>8}{par_s(b):>8.1f}{b['bilan']['liberations']:>13}")
    if fautes:
        sortie.extend(f"  DEFAUT {f}" for f in fautes)
        sortie.append(f"BANC_DECHIRURE_ECHEC n={len(fautes)} accel={nom} smp={smp}")
        return sortie, "DEFAUT", 1
    if len(paires) < QUORUM:
        sortie.append(f"BANC_DECHIRURE : NON_CONCLUANT accel={nom} smp={smp} tours={len(paires)}/{tours}")
        return sortie, "NON_CONCLUANT", 0
    mixtes_v1 = statistics.median(par_tour[t]["v1"]["bilan"]["mixtes"] for t in paires)
    debit = (statistics.median(par_s(par_tour[t]["v1"]) for t in paires),
             statistics.median(par_s(par_tour[t]["v2"]) for t in paires))
    temoin = mixtes_v1 > 0
    sortie.append(f"  {'ok     ' if temoin else 'ECHOUE '} temoin : mediane des compositions mixtes v1 {mixtes_v1} > 0")
    sortie.append(f"  ok      surete : 0 composition mixte en v2 sur {len(paires)} tours")
    ok_debit = debit[1] >= DEBIT_MIN * debit[0]
    sortie.append(f"  {'ok     ' if ok_debit else 'ECHOUE '} debit : v2 {debit[1]:.1f} lues/s >= {DEBIT_MIN:.2f} x v1 {debit[0]:.1f}")
    if not temoin:
        sortie.append(f"BANC_DECHIRURE : NON_CONCLUANT accel={nom} smp={smp} tours={len(paires)}/{tours} "
                      f"(le contrat d'avant n'a pas dechire ici : le zero de v2 ne prouve rien)")
        return sortie, "NON_CONCLUANT", 0
    verdict = "CONVAINCANT" if ok_debit else "NON_CONVAINCANT"
    sortie.append(f"BANC_DECHIRURE : {verdict} accel={nom} smp={smp} tours={len(paires)}/{tours} "
                  f"(offre de deux tampons par defaut : off)")
    return sortie, verdict, 0


def main(argv):
    if len(argv) != 5:
        print(__doc__, file=sys.stderr)
        return 2
    with open(argv[1], encoding="utf-8", errors="replace") as f:
        sortie, _, code = analyse(f.read(), int(argv[2]), argv[3], argv[4])
    print("\n".join(sortie))
    return code


if __name__ == "__main__":
    sys.exit(main(sys.argv))
