#!/usr/bin/env python3
"""Expose existing evidence in public job annotations; never changes a verdict."""
from pathlib import Path
import argparse
import re
import sys
from analyse_ab_reveil_precis import analyse


def diagnostic(raw, smp):
    lines, verdict, _ = analyse(raw, 4, str(smp), 'kvm')
    report = list(dict.fromkeys(line for line in lines if line.startswith((
        '  DEFAUT ', '  P1 ', '  P2 ', '  P3 ', '  P4 ', '  P5 ',
        'AB_REVEIL_PRECIS :', 'AB_BRAS_POLITIQUE ', '  preuve tour=', '  tour='))))
    clean = re.sub(r'\x1b\[[0-9;]*m', '', raw)
    keys = ('voulu', 'minuteur', 'armes', 'armes_occupes', 'avances_occupees',
            'tirs', 'tirs_anticipes', 'auto_ipi_evites', 'refus_quantum_avant', 'refus_sans_borne', 'refus_autres')
    for line in clean.splitlines():
        if '+ echo ' in line:
            continue
        if 'AB_PRECIS tour=' in line:
            report.append(line[line.index('AB_PRECIS tour='):])
        elif 'REVEIL_TIMER_LOCAL ' in line:
            report.append(line[line.index('REVEIL_TIMER_LOCAL '):])
        elif 'REVEIL_PRECIS voulu=' in line:
            fields = dict(re.findall(r'(\w+)=([^\s]+)', line))
            report.append('REVEIL_PRECIS ' + ' '.join(f'{k}={fields.get(k, "absent")}' for k in keys))
        elif 'attribution au repos :' in line or 'attribution sous charge (interactif) :' in line:
            report.append(line[line.index('attribution '):])
    return verdict, '\n'.join(report)


def blocs(report, limite=3000):
    """Keep every byte; GitHub's public annotation view truncates long messages."""
    block, taille = [], 0
    for char in report:
        n = len(char.encode('utf-8'))
        if taille + n > limite:
            yield ''.join(block)
            block, taille = [], 0
        block.append(char)
        taille += n
    if block:
        yield ''.join(block)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--smp', type=int, choices=(1, 4))
    args = parser.parse_args()
    for smp in ((args.smp,) if args.smp else (1, 4)):
        path = Path(f'ab-reveil-precis-kvm-smp{smp}/serie.log')
        if not path.is_file():
            print(f'::warning title=P13 KVM SMP{smp}::Journal absent : {path}')
            continue
        verdict, report = diagnostic(path.read_text(errors='replace'), smp)
        parts = list(blocs(report))
        for n, part in enumerate(parts, 1):
            escaped = part.replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
            print(f'::notice title=P13 KVM SMP{smp} {verdict} {n}/{len(parts)}::P13_KVM_ANNOTATION smp={smp} part={n}/{len(parts)}%0A{escaped}')
        print(report)


if __name__ == '__main__':
    main()
