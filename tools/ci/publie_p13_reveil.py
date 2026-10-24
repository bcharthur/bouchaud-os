#!/usr/bin/env python3
"""Expose existing evidence in public job annotations; never changes a verdict."""
from pathlib import Path
import re
import sys
from analyse_ab_reveil_precis import analyse


def diagnostic(raw, smp):
    lines, verdict, _ = analyse(raw, 4, str(smp), 'kvm')
    report = [line for line in lines if line.startswith(('  DEFAUT ', '  P1 ', '  P2 ', '  P3 ', '  P4 ', '  P5 ', 'AB_REVEIL_PRECIS :'))]
    clean = re.sub(r'\x1b\[[0-9;]*m', '', raw)
    keys = ('voulu', 'minuteur', 'armes', 'armes_occupes', 'avances_occupees',
            'tirs', 'tirs_anticipes', 'refus_quantum_avant', 'refus_sans_borne', 'refus_autres')
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
    return verdict, '\n'.join(report)


def main():
    for smp in (1, 4):
        path = Path(f'ab-reveil-precis-kvm-smp{smp}/serie.log')
        if not path.is_file():
            print(f'::warning title=P13 KVM SMP{smp}::Journal absent : {path}')
            continue
        verdict, report = diagnostic(path.read_text(errors='replace'), smp)
        escaped = report.replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
        print(f'::notice title=P13 KVM SMP{smp} {verdict}::{escaped}')
        print(report)


if __name__ == '__main__':
    main()
