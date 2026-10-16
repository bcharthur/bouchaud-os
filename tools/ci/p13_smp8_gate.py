#!/usr/bin/env python3
"""Gate SMP8 explicite : reprend les invariants stricts attribution sans
faire passer SMP8 pour SMP4 dans le qualificateur P13 baseline.
"""
from __future__ import annotations
import argparse
import re
import sys
from pathlib import Path

# Même contrat de champs que qualifie_p13.py, explicitement étendu à SMP8.
SONDE = re.compile(r'^VERDICT_SONDE : (\S+)')
COMPTE_HOTE = re.compile(r'^\s+comptes [^:]+ : preuve_hote=(\S+)')
ANSI = re.compile(r'\x1b\[[0-9;]*[A-Za-z]')
PREFIX = re.compile(r'^(?:\[[^\]]*\]){2}\[FPS:[^\]]*\] ')

def lignes_invite(raw: str) -> list[str]:
    return [PREFIX.sub('', ANSI.sub('', line.rstrip('\r'))) for line in raw.splitlines()]


def inspect(raw: str) -> list[str]:
    ls = lignes_invite(raw)
    faults = []
    if any('KERNEL PANIC' in l or 'panicked at' in l for l in ls):
        faults.append('panique noyau')
    if not any(l.strip() == 'ORDO_ATTRIBUTION_FIN' for l in ls):
        faults.append('marqueur ORDO_ATTRIBUTION_FIN exécuté absent')
    verdicts = [m.group(1) for l in ls if (m := SONDE.match(l))]
    if len(verdicts) != 2 or any(x != 'QUALIFIE_OK' for x in verdicts):
        faults.append(f'verdicts qualifiés {verdicts}, attendus 2 QUALIFIE_OK')
    proof = [m.group(1) for l in ls if (m := COMPTE_HOTE.match(l))]
    if len(proof) < 6 or any(x != 'kvm-steal' for x in proof):
        faults.append(f'preuve hôte KVM absente ou incohérente sur les phases ({len(proof)}/6)')
    return faults


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('journal', type=Path)
    ap.add_argument('--stdout', type=Path, required=True)
    a = ap.parse_args()
    if 'ordonnanceur-attribution-kvm-smp8' not in a.journal.parent.name:
        print('SMP8_PROVENANCE_ECHEC: mauvais nom de répertoire', file=sys.stderr)
        return 2
    if not a.journal.is_file() or not a.stdout.is_file():
        print('SMP8_PREUVE_ABSENTE', file=sys.stderr)
        return 2
    flaws = inspect(a.journal.read_text(encoding='utf-8', errors='replace'))
    stdout = a.stdout.read_text(encoding='utf-8', errors='replace')
    if not any(l.startswith('ORDONNANCEUR_ATTRIBUTION_OK passes=2 smp=8 accel=kvm preuve_hote=kvm-steal qualifie_ok=2 non_qualifiant=0 qualifie_echec=0') for l in stdout.splitlines()):
        flaws.append('marqueur d’agrégation SMP8 qualifié absent')
    if flaws:
        print('SMP8_BASELINE_NON_ETABLIE')
        for f in flaws:
            print('  - ' + f)
        return 1
    print('SMP8_ATTRIBUTION_STRICT_OK')
    return 0

if __name__ == '__main__':
    raise SystemExit(main())
