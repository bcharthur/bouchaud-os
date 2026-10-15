#!/usr/bin/env python3
"""Analyse de l'A/B de la variante d'IPI au reveil (BOUCHAUD_AB_REVEIL_IPI_V2).

    analyse_ab_reveil_ipi.py JOURNAL_SERIE TOURS SMP ACCEL

Seules les passes QUALIFIEES (QUALIFIE_OK / QUALIFIE_ECHEC) comptent, et
seulement par tours APPARIES (les deux bras qualifies dans le meme tour) ;
moins de QUORUM tours apparies : NON_CONCLUANT. Sortie 1 sur un defaut
d'instrument ou de surete, 0 sinon -- CONVAINCANT, NON_CONVAINCANT et
NON_CONCLUANT sont des mesures. Teste : tools/ci/reliability/test_ab_reveil_ipi.py.
"""
import math
import re
import statistics
import sys


def analyse(texte, tours, smp, nom):
    """Rend (lignes imprimees, verdict, code)."""
    sortie = []
    texte = re.sub(r"\x1b\[[0-9;]*m", "", texte)
    lignes = [re.sub(r"^(\[[^]]*\]){2}\[FPS:[^]]*\] ", "", l) for l in texte.splitlines()]
    bras, courant = [], None
    for l in lignes:
        # La sortie de `echo`, pas sa trace (`+ echo AB_BRAS ...`).
        m = re.match(r"AB_BRAS tour=(\d+) variante=(\S+)", l)
        if m:
            t = int(m.group(1))
            position = sum(1 for b in bras if b["tour"] == t)
            courant = {"tour": t, "variante": m.group(2), "position": position, "lignes": []}
            bras.append(courant)
            continue
        if courant is not None:
            courant["lignes"].append(l)
    QUANT = re.compile(r"^\s+(sous charge \((?:normal|interactif)\)|au repos)\s+median\s+(\d+) us\s+p95\s+(\d+) us"
                       r"\s+p99\s+(\d+) us\s+pire\s+(\d+) us\s+\[propres")
    fautes = []
    for b in bras:
        r = {"verdict": None, "ipi_variante": None, "ipi_depart": None, "bascule": None, "preuve": set()}
        tours_vus = []
        for l in b["lignes"]:
            q = QUANT.match(l)
            if q:
                r[q.group(1)] = tuple(int(x) for x in q.groups()[1:])
            m = re.match(r"^\s+(\d+) tours de calcul", l)
            if m:
                tours_vus.append(int(m.group(1)))
            m = re.match(r"^VERDICT_SONDE : (\S+)", l)
            if m:
                r["verdict"] = m.group(1)
            m = re.match(r"^\s+comptes .* : preuve_hote=(\S+) .*vol_hote_pour_mille=(-?\d+)", l)
            if m:
                r["preuve"].add(m.group(1))
                r["vol"] = max(r.get("vol", 0), int(m.group(2)))
            m = re.search(r"ORDO_VARIANTE reveil=(\S+) ipi_variante=(\d+)", l)
            # BOUCHAUD_AB_REVEIL_IPI_V3 : le releve de la bascule (avant le
            # verdict) est le depart, celui qui suit le verdict la fin.
            if m and r["verdict"] is None and r["ipi_depart"] is None:
                r["bascule"], r["ipi_depart"] = m.group(1), int(m.group(2))
            if m and r["verdict"] is not None and r["ipi_variante"] is None:
                r["ipi_variante"] = int(m.group(2))
        if len(tours_vus) >= 2:
            r["tours_normal"], r["tours_interactif"] = tours_vus[0], tours_vus[1]
        r["qualifie"] = r["verdict"] in ("QUALIFIE_OK", "QUALIFIE_ECHEC") \
            and "sous charge (interactif)" in r and "tours_interactif" in r
        b["r"] = r
        if r["verdict"] is None:
            fautes.append(f"tour {b['tour']} {b['variante']} : passe sans verdict")
        if nom == "kvm" and r["preuve"] != {"kvm-steal"}:
            fautes.append(f"tour {b['tour']} {b['variante']} : sous KVM sans preuve de l'hote ({sorted(r['preuve']) or '-'})")

    # Surete : les IPI de variante PENDANT chaque bras, du releve de sa
    # bascule (apres le delai de grace) au releve qui suit la sonde.
    # BOUCHAUD_AB_REVEIL_IPI_V3 -- run 38645396730 : le depart etait la fin du
    # bras precedent, releve SOUS LA VARIANTE avant la bascule ; deux IPI
    # envoyees dans cet intervalle etaient imputees au bras `defaut` suivant.
    envoyees = 0
    precedent = 0
    for b in bras:
        r = b["r"]
        if r["ipi_depart"] is None or r["bascule"] != b["variante"]:
            fautes.append(f"tour {b['tour']} {b['variante']} : bascule non relevee dans le bras "
                          f"(releve={r['bascule']})")
            continue
        if r["ipi_variante"] is None:
            continue
        delta = r["ipi_variante"] - r["ipi_depart"]
        if b["variante"] == "defaut" and delta != 0:
            fautes.append(f"surete : {delta} IPI de variante sous defaut (tour {b['tour']})")
        if b["variante"] != "defaut":
            envoyees += delta
        precedent = r["ipi_variante"]
    if envoyees == 0:
        fautes.append("surete : la variante n'a jamais envoye d'IPI (pas exercee)")
    if re.search(r"KERNEL PANIC|panicked at", texte):
        fautes.append("surete : panique")
    # La SORTIE de `echo AB_REVEIL_FIN`, pas sa trace (`+ echo AB_REVEIL_FIN`) :
    # la trace s'imprime avant l'execution et ne prouve rien.
    if not any(l.strip() == "AB_REVEIL_FIN" for l in lignes):
        fautes.append("le scenario n'est pas alle au bout")
    if len(bras) != 2 * tours:
        fautes.append(f"{len(bras)} passes sur {2 * tours}")

    D, X = "defaut", "ipi-interactive"
    sortie.append(f"  A/B reveil IPI, {nom} SMP{smp}, {tours} tours, ordre AB/BA alterne (valeurs PROPRES de la phase interactive)")
    sortie.append(f"  {'tour':<5}{'pos':<4}{'bras':<17}{'verdict':<16}{'p99':>7}{'pire':>8}{'tours_int':>11}{'tours_norm':>11}{'vol_hote':>9}")
    for b in bras:
        r = b["r"]
        i = r.get("sous charge (interactif)", (0, 0, 0, 0))
        sortie.append(f"  {b['tour']:<5}{b['position']:<4}{b['variante']:<17}{str(r['verdict']):<16}{i[2]:>7}{i[3]:>8}"
              f"{r.get('tours_interactif', 0):>11}{r.get('tours_normal', 0):>11}{r.get('vol', 0):>8}‰")

    # Les tours APPARIES : les deux bras qualifies dans le meme tour.
    par_tour = {}
    for b in bras:
        par_tour.setdefault(b["tour"], {})[b["variante"]] = b["r"]
    paires = [(t, v[D], v[X]) for t, v in sorted(par_tour.items()) if D in v and X in v and v[D]["qualifie"] and v[X]["qualifie"]]
    qualifies = {v: sum(1 for b in bras if b["variante"] == v and b["r"]["qualifie"]) for v in (D, X)}
    quorum = max(3, math.ceil(2 * tours / 3))
    sortie.append(f"  passes qualifiees : defaut {qualifies[D]}/{tours}, variante {qualifies[X]}/{tours} ; "
          f"tours apparies {len(paires)} (quorum {quorum}) ; une passe NON_QUALIFIANT n'est comptee pour aucun bras")

    if not fautes:
        if len(paires) < quorum:
            sortie.append(f"AB_REVEIL_IPI : NON_CONCLUANT accel={nom} smp={smp} tours={tours} apparies={len(paires)} quorum={quorum}"
                  " (trop peu de mesures qualifiees ; la politique par defaut est inchangee)")
        else:
            med = lambda xs: statistics.median(xs)
            p99 = (med([d["sous charge (interactif)"][2] for _, d, _ in paires]), med([x["sous charge (interactif)"][2] for _, _, x in paires]))
            pire = (med([d["sous charge (interactif)"][3] for _, d, _ in paires]), med([x["sous charge (interactif)"][3] for _, _, x in paires]))
            debit = (med([d["tours_interactif"] for _, d, _ in paires]), med([x["tours_interactif"] for _, _, x in paires]))
            equite = (med([d["tours_normal"] for _, d, _ in paires]), med([x["tours_normal"] for _, _, x in paires]))
            echecs_variante = sum(1 for b in bras if b["variante"] == X and b["r"]["verdict"] == "QUALIFIE_ECHEC")
            criteres = [
                ("latence", p99[1] <= 0.8 * p99[0] and pire[1] <= pire[0],
                 f"p99 median {p99[0]} -> {p99[1]} us (seuil {0.8 * p99[0]:.0f}), pire median {pire[0]} -> {pire[1]} us"),
                ("debit", debit[1] >= 0.95 * debit[0], f"tours (phase interactive) {debit[0]} -> {debit[1]}"),
                ("equite", abs(equite[1] - equite[0]) <= 0.05 * equite[0], f"tours (phase normale) {equite[0]} -> {equite[1]}"),
                ("surete", True, f"ipi_variante={precedent} ({envoyees} pendant les bras variante), 0 sous defaut, aucune panique"),
                ("robustesse", echecs_variante == 0, f"QUALIFIE_ECHEC sous la variante : {echecs_variante}"),
            ]
            for nom_c, ok, detail in criteres:
                sortie.append(f"  {'ok     ' if ok else 'NON    '} {nom_c:<11} {detail}")
            # Le biais de position, imprime : le meme bras en premier et en second.
            for v in (D, X):
                for pos in (0, 1):
                    vals = [b["r"]["sous charge (interactif)"][2] for b in bras
                            if b["variante"] == v and b["position"] == pos and b["r"]["qualifie"]]
                    if vals:
                        sortie.append(f"  position {pos} {v:<16} p99 median {statistics.median(vals)} us (n={len(vals)})")
            convaincant = all(ok for _, ok, _ in criteres)
            sortie.append(("AB_REVEIL_IPI : CONVAINCANT" if convaincant else "AB_REVEIL_IPI : NON_CONVAINCANT")
                  + f" accel={nom} smp={smp} tours={tours} apparies={len(paires)} (mesure ; la politique par defaut est inchangee)")
    for f in fautes:
        sortie.append(f"  ECHEC   {f}")
    if fautes:
        sortie.append(f"AB_REVEIL_IPI_ECHEC n={len(fautes)} accel={nom} smp={smp}")
        return sortie, "ECHEC", 1
    verdict = next((l.split(" : ")[1].split()[0] for l in sortie if l.startswith("AB_REVEIL_IPI : ")), "ECHEC")
    return sortie, verdict, 0


def main(argv):
    if len(argv) != 5:
        print(__doc__, file=sys.stderr)
        return 2
    texte = open(argv[1], encoding="utf-8", errors="replace").read()
    sortie, _, code = analyse(texte, int(argv[2]), argv[3], argv[4])
    print("\n".join(sortie))
    return code


if __name__ == "__main__":
    sys.exit(main(sys.argv))
