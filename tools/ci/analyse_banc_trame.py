#!/usr/bin/env python3
"""Analyse de l'A/B « une trame, une soumission » (BOUCHAUD_VIRTIO_GPU_TRAME_V1, P10+.3b).

    analyse_banc_trame.py JOURNAL_SERIE TOURS ACCEL

Un demarrage (`-vga none -device virtio-gpu-pci`), TOURS tours ; chaque tour
passe `banc-trame` (100 trames de bureau typiques, 8 regions) sous
`gpu-trame off` (deux commandes par region) et `gpu-trame on` (plan de la
trame : quelques transferts, un flush), ordre AB aux tours impairs, BA aux
pairs.

CRITERES DECLARES AVANT LA MESURE KVM (la politique par defaut est passee a
`on` sur l'A/B KVM CONVAINCANT de b6c99032, run 38732049308 ; un A/B KVM
NON_CONVAINCANT la rend a `off`) :

  surete     chaque banc : 100 trames, aucun echec, scanout actif ; sous `on`,
             une presentation par trame ; sous `off`, une par region ;
             aucune panique ;
  commandes  mediane des commandes par trame : on <= 0,5 x off ;
  latence    mediane des p50 par trame : on <= 0,90 x off ; mediane des p95 :
             on <= 1,10 x off ;
  quorum     4 tours complets, sinon NON_CONCLUANT.

Sortie 1 sur un defaut de surete ou d'instrument ; 0 sinon. Teste :
tools/ci/reliability/test_banc_trame.py.
"""
import re
import statistics
import sys

QUORUM = 4
LIGNE = re.compile(r"\[BANC-TRAME\] ecran=(\S+) trame=(on|off) n=(\d+) regions=(\d+) p50_us=(\d+) p95_us=(\d+) "
                   r"p99_us=(\d+) max_us=(\d+) commandes=(\d+) commandes_par_trame_x10=(\d+) presentations_ok=(\d+) "
                   r"echecs=(\d+) pixels=(\d+) actif=(\d)")
BRAS = re.compile(r"^AB_TRAME tour=(\d+) trame=(on|off)$")


def analyse(texte, tours, nom):
    sortie, fautes = [], []
    texte = re.sub(r"\x1b\[[0-9;]*m", "", texte)
    lignes = [re.sub(r"^(\[[^]]*\]){2}\[FPS:[^]]*\] ", "", l).rstrip("\r") for l in texte.splitlines()]
    if re.search(r"KERNEL PANIC|panicked at", texte):
        fautes.append("panique")
    if not any(l == "AB_TRAME_FIN" for l in lignes):
        fautes.append("le scenario n'est pas alle au bout")
    bras, courant = [], None
    for l in lignes:
        m = BRAS.match(l)
        if m:
            courant = {"tour": int(m.group(1)), "v": m.group(2), "m": None}
            bras.append(courant)
            continue
        m = LIGNE.search(l)
        if m and courant is not None:
            g = m.groups()
            courant["m"] = {"ecran": g[0], "trame": g[1], "n": int(g[2]), "regions": int(g[3]), "p50": int(g[4]),
                            "p95": int(g[5]), "p99": int(g[6]), "commandes": int(g[8]), "cpt10": int(g[9]),
                            "ok": int(g[10]), "echecs": int(g[11]), "actif": int(g[13])}
    for b in bras:
        nb, m = f"tour {b['tour']} {b['v']}", b["m"]
        if m is None:
            fautes.append(f"{nb} : banc sans mesure")
            continue
        if m["ecran"] != "virtio-gpu-2d" or m["trame"] != b["v"]:
            fautes.append(f"{nb} : mesure sur {m['ecran']} sous trame={m['trame']}")
        attendu = m["n"] if b["v"] == "on" else m["n"] * m["regions"]
        if m["n"] != 100 or m["echecs"] != 0 or m["actif"] != 1 or m["ok"] != attendu:
            fautes.append(f"{nb} : {m['ok']} presentations sur {attendu}, {m['echecs']} echec(s), actif={m['actif']}")
    for t in range(1, tours + 1):
        ordre = [b["v"] for b in bras if b["tour"] == t]
        voulu = ["off", "on"] if t % 2 else ["on", "off"]
        if ordre and ordre != voulu:
            fautes.append(f"tour {t} : ordre {ordre}, attendu {voulu} (AB/BA)")
    par_tour = {}
    for b in bras:
        if b["m"] is not None:
            par_tour.setdefault(b["tour"], {})[b["v"]] = b["m"]
    paires = sorted(t for t, d in par_tour.items() if set(d) == {"on", "off"})

    sortie.append(f"  A/B une trame, une soumission, virtio-gpu 2D, {nom} : {len(paires)}/{tours} tours complets")
    sortie.append(f"  {'tour':<5}{'trame':<7}{'p50':>8}{'p95':>8}{'p99':>8}{'cmd/trame':>11}")
    for b in bras:
        m = b["m"] or {}
        sortie.append(f"  {b['tour']:<5}{b['v']:<7}{m.get('p50', 0):>8}{m.get('p95', 0):>8}{m.get('p99', 0):>8}"
                      f"{m.get('cpt10', 0) / 10:>11.1f}")
    if fautes:
        sortie.extend(f"  DEFAUT {f}" for f in fautes)
        sortie.append(f"BANC_TRAME_ECHEC n={len(fautes)} accel={nom}")
        return sortie, "DEFAUT", 1
    if len(paires) < QUORUM:
        sortie.append(f"BANC_TRAME : NON_CONCLUANT accel={nom} tours={len(paires)}/{tours}")
        return sortie, "NON_CONCLUANT", 0

    def med(v, k):
        return statistics.median([par_tour[t][v][k] for t in paires])
    criteres = [
        (f"commandes par trame : on {med('on', 'cpt10') / 10} <= 0,5 x off {med('off', 'cpt10') / 10}",
         med("on", "cpt10") <= 0.5 * med("off", "cpt10")),
        (f"p50 par trame : on {med('on', 'p50')} <= 0,90 x off {med('off', 'p50')}", med("on", "p50") <= 0.90 * med("off", "p50")),
        (f"p95 par trame : on {med('on', 'p95')} <= 1,10 x off {med('off', 'p95')}", med("on", "p95") <= 1.10 * med("off", "p95")),
    ]
    for txt, ok in criteres:
        sortie.append(f"  {'ok     ' if ok else 'ECHOUE '} {txt}")
    verdict = "CONVAINCANT" if all(ok for _, ok in criteres) else "NON_CONVAINCANT"
    sortie.append(f"BANC_TRAME : {verdict} accel={nom} tours={len(paires)}/{tours} (politique par defaut : on depuis b6c99032 ; un KVM NON_CONVAINCANT la rend a off)")
    return sortie, verdict, 0


def main(argv):
    if len(argv) != 4:
        print(__doc__, file=sys.stderr)
        return 2
    with open(argv[1], encoding="utf-8", errors="replace") as f:
        sortie, _, code = analyse(f.read(), int(argv[2]), argv[3])
    print("\n".join(sortie))
    return code


if __name__ == "__main__":
    sys.exit(main(sys.argv))
