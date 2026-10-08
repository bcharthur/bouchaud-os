#!/usr/bin/env python3
"""Swaps distincts et fermes, prouves par un journal cumulatif.

Les repetitions survivent aux pertes serie, jamais elles ne multiplient les
swaps. Sans etat final apres FIN et chaque identite complete, rouge.
"""
import re
import sys
from pathlib import Path

ANSI = re.compile(r"\x1b\[[0-9;]*m")
STATE = re.compile(r"\[LB:SWAP_STATE\] seq=(\d+) swaps=(\d+) views=(\d+) closed=(\d+) valid=(\d+) distinct_new=(\d+) new_live=(\d+) END")
PROOF = re.compile(r"\[LB:SWAP_PROOF\] seq=(\d+) onglet=(\d+) old=(-?\d+) new=(-?\d+) changes=(\d+) closed=(\d+) new_live=(\d+) END")


def verify(text, n, endurance=False):
    if n < 1:
        return False, "n invalide"
    lines = ANSI.sub("", text).splitlines()
    expected = n if endurance else 2 * n
    fin = rf'HOST_ENDURANCE_FIN cycles=\d+ t_s=\d+ cadres_ok=\d+ cadres_echus=\d+ workers_ok=\d+ onglets={n} ' if endurance else rf'HOST_MEMOIRE_FIN n={n}(?!\d)'
    end = next((i for i, l in enumerate(lines) if re.search(fin, l)), None)
    if end is None:
        return False, "FIN absent"
    states, proofs = {}, {}
    last_seq = 0
    for i, line in enumerate(lines):
        if m := STATE.search(line):
            seq, count, views, closed, valid, distinct, live = map(int, m.groups())
            if seq <= last_seq:
                return False, "sequence non monotone (runs melanges)"
            last_seq = seq
            if i > end:
                states[seq] = (count, views, closed, valid, distinct, live)
        if i > end and (m := PROOF.search(line)):
            seq, tab, old, new, changes, closed, live = map(int, m.groups())
            if seq not in states or states[seq] != (expected, 1, expected, expected, expected, 0):
                continue
            if old <= 0 or new <= 0 or old == new or changes != 1 or closed != 1 or live != 0:
                return False, f"swap incomplet onglet={tab} old={old} new={new} changes={changes} closed={closed} new_live={live}"
            identity = (old, new)
            if tab in proofs and proofs[tab] != identity:
                return False, f"identite changee onglet={tab}"
            proofs[tab] = identity
    if endurance:
        # Un dernier enfant peut encore se fermer juste apres FIN. Le dernier
        # bilan complet doit prouver sa fermeture et sa recolte, sans delai ajoute.
        if not states or list(states.values())[-1] != (expected, 1, expected, expected, expected, 0):
            return False, f"etat final incoherent {states}"
        if any(v[0] != expected for v in states.values()):
            return False, "nombre de swaps change apres FIN"
        return True, f"swaps={expected}/{expected} closed={expected} nouveaux_webcontent_recoltes={expected}"
    if not states or any(v != (expected, 1, expected, expected, expected, 0) for v in states.values()):
        return False, f"etat final incoherent {states}"
    if len(proofs) != expected or len({v[1] for v in proofs.values()}) != expected:
        return False, f"preuves distinctes={len(proofs)}/{expected}"
    return True, f"swaps={len(proofs)}/{expected} closed={len(proofs)} nouveaux_webcontent_recoltes={len(proofs)}"


if __name__ == "__main__":
    ok, detail = verify(Path(sys.argv[1]).read_text(errors="replace"), int(sys.argv[2]), endurance="--endurance" in sys.argv[3:])
    print(f"PROCESS_SWAPS_PREUVE {'OK' if ok else 'ECHEC'} {detail}")
    sys.exit(0 if ok else 1)
