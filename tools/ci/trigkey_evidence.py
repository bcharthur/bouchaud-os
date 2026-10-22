#!/usr/bin/env python3
"""Contrôle des marqueurs RÉELS du boot Trigkey sans inventer de succès.

Les marqueurs doivent être renseignés à partir des sorties serial constatées :
le fichier exemple contient des champs vides et ne peut JAMAIS valider.
"""
from __future__ import annotations
import argparse
import json
import re
import sys
from pathlib import Path

FATAL = re.compile(r'KERNEL PANIC|panicked at|ASSERTION FAILED|VERIFICATION FAILED')
ANSI = re.compile(r'\x1b\[[0-9;]*[A-Za-z]')
PREFIX = re.compile(r'^(?:\[[^\]]*\]){2}\[FPS:[^\]]*\] ')
PHASES = ('boot', 'display', 'input', 'network', 'ladybird', 'https')


def evaluate(log: str, criteria: dict) -> dict:
    requested = criteria.get('markers', {})
    lines = [PREFIX.sub('', ANSI.sub('', x)) for x in log.splitlines()]
    found, missing = {}, []
    for step in PHASES:
        token = requested.get(step)
        if not isinstance(token, str) or not token.strip():
            missing.append(f'{step}: marqueur non défini')
            found[step] = False
        else:
            ok = any(token == line.strip() or token in line for line in lines
                     if not line.lstrip().startswith('+ echo'))
            found[step] = ok
            if not ok:
                missing.append(f'{step}: absent: {token}')
    issues = [line[:250] for line in lines if FATAL.search(line)]
    return {'result': 'PHYSICAL_SMOKE_OK' if not missing and not issues else 'NOT_ESTABLISHED',
            'found': found, 'missing': missing, 'fatal_lines': issues[:15],
            'note': 'Marqueurs définis par l’opérateur : examiner les captures vidéo/réseau pour valider leur sens.'}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--log', type=Path, required=True)
    ap.add_argument('--criteria', type=Path, required=True)
    ap.add_argument('--output', type=Path, required=True)
    args = ap.parse_args()
    try:
        criteria = json.loads(args.criteria.read_text(encoding='utf-8-sig'))
        data = evaluate(args.log.read_text(encoding='utf-8', errors='replace'), criteria)
    except (OSError, ValueError) as exc:
        print(f'TRIGKEY_PREUVE_ERREUR {exc}', file=sys.stderr)
        return 2
    if args.output.exists():
        print('Refus d’écraser le rapport', file=sys.stderr)
        return 2
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(data, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')
    print('TRIGKEY_' + data['result'])
    for x in data['missing']:
        print(' - ' + x)
    for x in data['fatal_lines']:
        print(' FATAL - ' + x)
    return 0 if data['result'] == 'PHYSICAL_SMOKE_OK' else 1


if __name__ == '__main__':
    raise SystemExit(main())
