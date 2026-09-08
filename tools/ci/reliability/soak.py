#!/usr/bin/env python3
"""Repeated QEMU boot campaign for bounded or long-running soak tests.

# Deux defauts de cette campagne, et pourquoi ils comptent

**Un cycle tronque n'est pas le meme essai.** La boucle repartait tant que
l'echeance n'etait pas atteinte, et donnait au dernier cycle le temps qui
restait -- parfois une seconde. Un demarrage d'une seconde ne prouve rien : il
n'atteint aucun marqueur, il ne finit aucune phase, et son verdict ne dit rien
du noyau. Le compter comme un cycle melange une mesure a un artefact de
minuterie.

**Un echec qui ne dit pas ce qui a echoue ne sert a rien.** La campagne
imprimait « cycle 11: ECHEC » et jetait le diagnostic que `run_one` avait
pourtant produit -- code de retour, journaux fatals, marqueurs manquants. On
savait qu'il y avait un probleme et rien de plus, ce qui coute un aller-retour
complet de CI pour decouvrir une information qu'on avait deja.
"""

from __future__ import annotations

import argparse
import json
import time
from pathlib import Path

import qemu_matrix

def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("bootimage", type=Path)
    parser.add_argument("--cpus", type=int, default=4)
    parser.add_argument("--duration-seconds", type=int, required=True)
    parser.add_argument("--cycle-seconds", type=int, default=120)
    parser.add_argument("--memory-mb", type=int, default=4096)
    parser.add_argument("--out-dir", type=Path, default=Path("reliability-soak"))
    parser.add_argument("--require", action="append", default=[])
    args = parser.parse_args()

    if args.duration_seconds <= 0 or args.cycle_seconds <= 0:
        raise SystemExit("les durees doivent etre positives")

    qemu = qemu_matrix.qemu_binary()
    started = time.monotonic()
    deadline = started + args.duration_seconds
    cycles = []
    index = 0

    tronques = 0
    while True:
        restant = deadline - time.monotonic()
        # UN CYCLE TRONQUE N'EST PAS UN CYCLE.
        #
        # Lancer le dernier cycle avec le temps qui reste -- parfois une
        # seconde -- produit un demarrage qui n'atteint rien et dont le verdict
        # ne dit rien du noyau. On s'arrete plutot que de mesurer autre chose
        # que ce qu'on croit mesurer, et on le DIT dans le resume.
        if restant < args.cycle_seconds:
            if restant > 0:
                tronques += 1
            break
        index += 1
        cycle_dir = args.out_dir / f"cycle-{index:04d}"
        result = qemu_matrix.run_one(
            qemu, args.bootimage, args.cpus, args.cycle_seconds, args.memory_mb,
            cycle_dir, args.require,
        )
        cycles.append(result)
        etat = "OK" if result["ok"] else "ECHEC"
        print(
            f"cycle {index}: {etat} log={result['log_bytes']} octets "
            f"timeout={result['timed_out']} rc={result['returncode']}"
        )
        # LE DIAGNOSTIC, LA OU L'ECHEC SE PRODUIT.
        #
        # `run_one` l'a deja calcule. Ne pas l'imprimer coute un aller-retour
        # complet de CI pour retrouver ce qu'on avait sous la main.
        if not result["ok"]:
            for finding in result["fatal_findings"]:
                print(f"  {finding['kind']}:{finding['line']}: {finding['text']}")
            for marker in result["required_markers_missing"]:
                print(f"  marqueur absent: {marker}")
            if not result["qemu_exit_ok"]:
                print(f"  QEMU a quitte avec le code {result['returncode']}")
            print(f"  journal complet : {result['log']}")
            break

    payload = {
        "schema": 2,
        "requested_seconds": args.duration_seconds,
        "cycle_seconds": args.cycle_seconds,
        "elapsed_seconds": round(time.monotonic() - started, 3),
        "cycles": cycles,
        # Le temps qu'on n'a PAS mesure, parce qu'il ne suffisait pas a un
        # cycle entier. Le taire ferait croire que la campagne a couvert toute
        # la duree demandee.
        "cycles_tronques_evites": tronques,
        "ok": bool(cycles) and all(c["ok"] for c in cycles),
    }
    args.out_dir.mkdir(parents=True, exist_ok=True)
    (args.out_dir / "summary.json").write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    reussis = sum(1 for c in cycles if c["ok"])
    print(
        f"soak : {reussis}/{len(cycles)} cycle(s) de {args.cycle_seconds}s en "
        f"{payload['elapsed_seconds']}s"
        + (f", {tronques} fin de campagne trop courte pour un cycle entier" if tronques else "")
    )
    if not cycles:
        # Zero cycle n'est pas une campagne verte : c'est une campagne qui n'a
        # rien mesure, et le vert qu'elle rendrait est indiscernable du vert
        # d'une campagne qui a tout passe.
        print(
            "ECHEC: aucun cycle n'a tourne -- la duree demandee est plus "
            "courte qu'un cycle."
        )
    return 0 if payload["ok"] else 1

if __name__ == "__main__":
    raise SystemExit(main())
