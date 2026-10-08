#!/usr/bin/env python3
"""Regressions de la preuve cumulative des process swaps (HOST)."""
from preuve_swaps import verify

def state(seq):
    return f'[LB:SWAP_STATE] seq={seq} swaps=20 views=1 closed=20 valid=20 distinct_new=20 new_live=0 END\n'


def proof(seq, tab):
    return f'[LB:SWAP_PROOF] seq={seq} onglet={tab} old=19 new={tab+20} changes=1 closed=1 new_live=0 END\n'


# Trois marqueurs ponctuels absents ne perdent aucune identite du bilan final.
good = 'PROCESS_S abime\nHOST_MEMOIRE_FIN n=10\n' + state(1)
good += ''.join(proof(1, t) for t in range(2, 22) if t not in (5, 7, 14))
good += state(2) + ''.join(proof(2, t) for t in (5, 7, 14))
assert verify(good, 10)[0]
for bad in (
    '', good.replace('HOST_MEMOIRE_FIN', 'FIN_ABSENT'),
    good.replace(proof(2, 14), ''), good.replace('new=34 ', 'new=19 '),
    good.replace('closed=1', 'closed=0'), good.replace('new_live=0', 'new_live=1'),
    good.replace('changes=1', 'changes=2'), good.replace('swaps=20', 'swaps=19'),
    good.replace('views=1', 'views=2'), good.replace(state(2), state(1)),
    good.replace('new=34 ', 'new=25 '), good.replace(' END', ' EN'),
):
    assert not verify(bad, 10)[0], bad

fin = 'HOST_ENDURANCE_FIN cycles=60 t_s=300 cadres_ok=60 cadres_echus=0 workers_ok=60 onglets=20 raf=1\n'
assert verify(fin + state(1), 20, endurance=True)[0]
for field, bad_value in (('swaps', 19), ('views', 2), ('closed', 19), ('valid', 19), ('distinct_new', 19), ('new_live', 1)):
    bad = state(1).replace(f'{field}={20 if field not in ("views", "new_live") else (1 if field == "views" else 0)} ', f'{field}={bad_value} ')
    assert not verify(fin + bad, 20, endurance=True)[0], bad
assert not verify(fin + state(1) + state(1), 20, endurance=True)[0]
print('SWAPS_PREUVE_TEST_OK memoire_negatifs=12 endurance_negatifs=7')
