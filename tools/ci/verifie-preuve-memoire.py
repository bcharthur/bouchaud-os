#!/usr/bin/env python3
"""Regressions des preuves memoire et swaps : HOST, pas une preuve QEMU."""
from analyse_memoire import analyse


def memory(k, second, live=2, size=4734400):
    return (f'[00:00:{second:02}] [LB:MEM] ev=sample ctx=0 contexts_live={live} '
            f'backing_stores_live=2 backing_store_octets={size} skia_ressources_octets=0 '
            f'skia_ressources_limite=33554432 skia_polices_octets=123 '
            f'created_total={max(2+20*k, live)} destroyed_total={max(2+20*k, live)-live} END\n')


def fixture(lives=(2, 2, 2), sizes=(4734400,)*3, rss=(41652, 48276, 48280)):
    rows = []
    for k, t in enumerate((10, 30, 50)):
        for second in range(t-3, t):
            rows.append(memory(k, second, lives[k], sizes[k]))
        for pid, name in enumerate(('Compositor', 'ImageDecoder', 'RequestServer', 'WebContent'), 18):
            rows.append(f'[00:00:{t-1:02}] [PERF-PROC] t={t*1000} pid={pid} image={name} rss_kio={rss[k]} vss_kio=999999 taches=6\n')
        rows.append(f'[00:00:{t:02}] HOST_MEMOIRE_REPERE m={k} n=10\n')
        # Le prochain onglet cree un contexte DANS LA MEME SECONDE, APRES
        # le repere. L'ancien analyseur prenait cet etat futur pour la baseline.
        rows.append(memory(k, t, 3, 17226624))
    for pid, name in enumerate(('Compositor', 'ImageDecoder', 'RequestServer', 'WebContent'), 18):
        rows.append(f'[00:00:51] [PERF-PROC] t=51000 pid={pid} image={name} rss_kio={rss[-1]} vss_kio=999999 taches=6\n')
    return ''.join(rows)


good = fixture()
assert analyse(good)[0] == 0, analyse(good)
assert analyse(fixture((3, 13, 22)))[0] == 1
assert analyse(fixture((2, 3, 3)))[0] == 1  # pas de tolerance ajoutee
assert analyse(fixture(sizes=(4734400, 50857000, 97092000)))[0] == 1
assert analyse(fixture(rss=(40000, 50000, 60000)))[0] == 1
assert analyse(fixture(rss=(69548, 90832, 102208)))[0] == 1  # +11 Mio ne prouve pas un plateau
for bad in (
    '', good.replace('vss_kio=', 'vss_abime='), good.replace('ev=sample', 'ev=context_destroy'),
    good.replace('image=Compositor', 'image=Autre'),
    good.replace('m=1 n=10', 'm=1 n=9'),
    good.replace('destroyed_total=40', 'destroyed_total=39'),
    good.replace(memory(1, 29), ''),
    good.replace('[00:00:30] HOST_MEMOIRE', '[00:00:40] HOST_MEMOIRE'),
):
    assert analyse(bad)[0] == 2, analyse(bad)


print('MEMOIRE_PREUVE_TEST_OK croissance=5 inconclusifs=8')
