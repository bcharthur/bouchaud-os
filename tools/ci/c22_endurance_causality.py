#!/usr/bin/env python3
"""Long-endurance C22 causal evidence. Does not alter any qualification verdict."""
import argparse
import csv
import json
import re
from pathlib import Path

ANSI = re.compile(r'\x1b\[[0-9;]*m')
TAB_OPEN = re.compile(r'\[LB:TAB\] ouvert onglet=(\d+)\b')
TAB_CLOSE = re.compile(r'\[LB:TAB\] ferme onglet=(\d+)\b')
SWAP = re.compile(r'\[LB\] PROCESS_SWAP onglet=(\d+)\b.*?ancien_pid=(\d+) nouveau_pid=(\d+)')
RELAIS = re.compile(r'\[LB:JS\] onglet=(\d+) log HOST_ENDURANCE_RELAIS \?cycle=(\d+)')
CHILD = re.compile(r'\[LB:JS\] onglet=(\d+) log HOST_ENDURANCE_ENFANT \?cycle=(\d+)')
NAV_FINISH = re.compile(r'\[LB:NAV-TRACE\] onglet=(\d+) nav=(\d+) evt=finish\b')
STATE = re.compile(r'\[LB:SWAP_STATE\] seq=(\d+) swaps=(\d+) views=(\d+) closed=(\d+) valid=(\d+) distinct_new=(\d+) new_live=(\d+) END')
FIN = re.compile(r'HOST_ENDURANCE_FIN cycles=(\d+) t_s=(\d+).*?onglets=(\d+)\b')
RSS = re.compile(r'\[C22:RSS\] t=(\d+) pid=(\d+) rss_kio=(\d+) anon_kio=(\d+) fichier_kio=(\d+)')
EXIT = re.compile(r'\[LB\] PROCESS_EXIT type=WebContent pid=(\d+)\b')
FIXTURE_CHILD = re.compile(r'GET /endurance-enfant\.html\?cycle=(\d+) HTTP/1\.1" (\d+)')

def analyze(serial, fixture):
    tabs, states, rss, exited = {}, [], {}, set()
    fin = None
    for lineno, raw in enumerate(serial.splitlines(), 1):
        line = ANSI.sub('', raw)
        if (m := TAB_OPEN.search(line)):
            tabs.setdefault(int(m[1]), {'tab': int(m[1]), 'opened_line': lineno,
                'closed_line': None, 'pid': None, 'cycle': None,
                'child_loaded': False, 'nav_finished': False})
        if (m := TAB_CLOSE.search(line)):
            tabs.setdefault(int(m[1]), {'tab': int(m[1])})['closed_line'] = lineno
        if (m := SWAP.search(line)):
            tabs.setdefault(int(m[1]), {'tab': int(m[1])}).update(
                old_pid=int(m[2]), pid=int(m[3]), swap_line=lineno)
        if (m := RELAIS.search(line)):
            tabs.setdefault(int(m[1]), {'tab': int(m[1])})['cycle'] = int(m[2])
        if (m := CHILD.search(line)):
            tabs.setdefault(int(m[1]), {'tab': int(m[1])})['child_loaded'] = True
        if (m := NAV_FINISH.search(line)) and int(m[2]) >= 2:
            tabs.setdefault(int(m[1]), {'tab': int(m[1])})['nav_finished'] = True
        if (m := STATE.search(line)):
            states.append(dict(line=lineno, seq=int(m[1]), swaps=int(m[2]),
                views=int(m[3]), closed=int(m[4]), valid=int(m[5]),
                distinct_new=int(m[6]), new_live=int(m[7])))
        if fin is None and (m := FIN.search(line)):
            fin = dict(line=lineno, cycles=int(m[1]), t_s=int(m[2]), tabs=int(m[3]))
        if (m := RSS.search(line)):
            rss.setdefault(int(m[2]), []).append(dict(t_ms=int(m[1]),
                rss_kio=int(m[3]), anon_kio=int(m[4]), file_kio=int(m[5])))
        if (m := EXIT.search(line)):
            exited.add(int(m[1]))
    served = {int(m[1]): int(m[2]) for m in FIXTURE_CHILD.finditer(fixture)}
    stale = []
    for t in tabs.values():
        if t.get('pid') is None or t.get('closed_line') is not None:
            continue
        cycle = t.get('cycle')
        stale.append({**t, 'pid_exited': t['pid'] in exited,
            'fixture_child_status': served.get(cycle),
            'fixture_child_request_seen': cycle in served})
    trends = []
    for pid, samples in sorted(rss.items()):
        if len(samples) < 2:
            continue
        a, b = samples[0], samples[-1]
        trends.append(dict(pid=pid, samples=len(samples), first=a, last=b,
            delta_rss_kio=b['rss_kio']-a['rss_kio'],
            delta_anon_kio=b['anon_kio']-a['anon_kio'],
            max_rss_kio=max(x['rss_kio'] for x in samples)))
    after = [s for s in states if fin and s['line'] > fin['line']]
    return dict(final_marker=fin,
        last_state=after[-1] if after else (states[-1] if states else None),
        post_fin_states=len(after), stale_swapped_tabs=stale,
        fixture_child_successes=sum(v == 200 for v in served.values()),
        fixture_child_failures={k:v for k,v in served.items() if v != 200},
        per_pid_rss=trends, interpretation=[
          'Missing fixture GET means the fixture did not log the request; it does not identify the broken network/IPC component.',
          'A parent JS message saying ferme=lui_meme does not prove an actual [LB:TAB] ferme event.',
          'RSS by image may mix different PIDs. Compare stable PIDs only.',
          'This analysis never modifies the endurance or C22 verdict.'])

def write_report(data, out):
    p = Path(out)
    p.mkdir(parents=True, exist_ok=True)
    (p/'causality.json').write_text(json.dumps(data, ensure_ascii=False, indent=2)+'\n')
    fields = ['tab','cycle','old_pid','pid','swap_line','closed_line','child_loaded',
              'nav_finished','pid_exited','fixture_child_request_seen','fixture_child_status']
    with (p/'stale-tabs.csv').open('w', newline='') as f:
        w = csv.DictWriter(f, fieldnames=fields, extrasaction='ignore')
        w.writeheader()
        w.writerows(data['stale_swapped_tabs'])
    lines = ['# C22 endurance causal witness', '',
             'Diagnostic only; the strict C22/endurance verdict remains unchanged.',
             '', '## Last post-FIN swap state', '', str(data['last_state']),
             '', '## Swapped tabs still open', '']
    for t in data['stale_swapped_tabs']:
        lines.append(
            f"- tab {t['tab']}, cycle {t.get('cycle')}, PID {t['pid']}: "
            f"child_loaded={t.get('child_loaded')}, nav_finished={t.get('nav_finished')}, "
            f"pid_exited={t['pid_exited']}, fixture_GET_seen={t['fixture_child_request_seen']} "
            f"(swap serial line {t.get('swap_line')})")
    lines.extend(['', '## RSS by PID', ''])
    for t in data['per_pid_rss']:
        if t['samples'] >= 10:
            lines.append(
                f"- PID {t['pid']}: {t['samples']} samples, RSS "
                f"{t['first']['rss_kio']} -> {t['last']['rss_kio']} KiB "
                f"(delta {t['delta_rss_kio']:+} KiB), "
                f"anon delta {t['delta_anon_kio']:+} KiB")
    lines.extend(['', '## Attribution limits', ''] + ['- '+x for x in data['interpretation']])
    (p/'causality.md').write_text('\n'.join(lines)+'\n')

def selftest():
    serial = '\n'.join([
        '[LB:TAB] ouvert onglet=18 page=86',
        '[LB:JS] onglet=18 log HOST_ENDURANCE_RELAIS ?cycle=51',
        '[LB] PROCESS_SWAP onglet=18 raison=autre_site ancien_pid=20 nouveau_pid=85',
        '[C22:RSS] t=100 pid=20 rss_kio=74000 anon_kio=12000 fichier_kio=60000',
        '[C22:RSS] t=200 pid=20 rss_kio=80000 anon_kio=18000 fichier_kio=60000',
        'HOST_ENDURANCE_FIN cycles=239 t_s=1203 cadres_ok=239 cadres_echus=0 workers_ok=239 onglets=1',
        '[LB:SWAP_STATE] seq=3 swaps=1 views=2 closed=0 valid=1 distinct_new=1 new_live=1 END'])
    d = analyze(serial, '[fixture] "GET /endurance-enfant.html?cycle=3 HTTP/1.1" 200 -')
    assert len(d['stale_swapped_tabs']) == 1
    assert d['stale_swapped_tabs'][0]['pid'] == 85
    assert d['stale_swapped_tabs'][0]['fixture_child_request_seen'] is False
    assert d['last_state']['new_live'] == 1
    assert d['per_pid_rss'][0]['delta_rss_kio'] == 6000
    good = serial+'\n[LB:TAB] ferme onglet=18 page=82\n[LB] PROCESS_EXIT type=WebContent pid=85 statut=0\n'
    assert not analyze(good, '')['stale_swapped_tabs']
    print('C22_ENDURANCE_CAUSALITY_SELFTEST_OK')

def main():
    p = argparse.ArgumentParser()
    p.add_argument('serial', nargs='?')
    p.add_argument('--fixture')
    p.add_argument('--out')
    p.add_argument('--selftest', action='store_true')
    a = p.parse_args()
    if a.selftest:
        return selftest()
    if not a.serial or not a.fixture or not a.out:
        p.error('serial, --fixture and --out required')
    data = analyze(Path(a.serial).read_text(errors='replace'),
                   Path(a.fixture).read_text(errors='replace'))
    write_report(data, a.out)
    print('C22_ENDURANCE_CAUSALITY_OK',
          'stale_tabs='+str(len(data['stale_swapped_tabs'])),
          'last_state='+str(data['last_state']))
    return 0

if __name__ == '__main__':
    raise SystemExit(main())
