#!/usr/bin/env python3
"""Qualification stricte M1 sur UN journal serie de Bouchaud OS.

Ne transforme pas la presence de marqueurs attendus dans un README en preuve.
Requiert un journal runtime: toutes les observations sont nommees et bornees.
Aucun verdict PHYSICAL ne peut sortir d'un journal QEMU.
"""
import argparse
import json
import pathlib
import re
import sys

CHECKS = (
    ('uefi_boot', 'Bascule OVMF vers noyau', True),
    ('desktop', 'Bureau actif et composition', True),
    ('network_dhcp', 'Reseau DHCP configure', False),
    ('ladybird_ramdisk', 'Ramdisk Ladybird monte', False),
    ('ladybird_runtime', 'Runtime Ladybird actif', True),
    ('webcontent', 'WebContent observe dans ce boot', False),
    ('keyboard_to_desktop', 'Saisie clavier du bureau', False),
    ('mouse_to_desktop', 'Souris utilisee par le bureau', False),
    ('notes_saved_ramfs', 'Document Notes sauve en RAMFS', False),
    ('notes_persistent', 'Document retrouve apres reboot', True),
    ('https_navigation', 'Navigation HTTPS reelle', True),
)


def after_num(line: str, name: str):
    m = re.search(r'\b' + re.escape(name) + r'=(\d+)', line)
    return int(m.group(1)) if m else None


def parse(text: str, level: str = 'QEMU-TCG') -> dict:
    # N accepter que de vraies lignes serie, pas les commandes grep ou README
    # d'un journal PowerShell/bash qui contiendraient les memes mots.
    runtime_prefix = re.compile(r'^(?:\[\d{2}:\d{2}:\d{2}\]|\[[A-Za-z][A-Za-z0-9_-]+\]|BOUCHAUD_|SMP_|BdsDxe:)')
    lines = [line for line in text.splitlines() if runtime_prefix.match(line)]
    # Mode strict: un marqueur de documentation imprime dans un log n'est pas une preuve.
    boot = any('[kernel]' in l or 'BOUCHAUD_BOOT_POINT' in l or '[GUI-COMPOSITOR]' in l for l in lines)
    desktop = any('[PROC-SAMPLE]' in l and 'name=desktop' in l for l in lines)
    frames = any('[GUI-COMPOSITOR]' in l and (after_num(l, 'frames_composed') or 0) > 0 for l in lines)
    network = any('[NET-LIEN]' in l and re.search(r'\bip=(?!0\.0\.0\.0\b)\d+\.\d+\.\d+\.\d+', l) for l in lines)
    ramdisk = any('BOUCHAUD_TRIGKEY_LADYBIRD_RAMDISK_OK' in l for l in lines)
    runtime = any('BOUCHAUD_STAGE2_LADYBIRD_RUNTIME_OK' in l for l in lines)
    web = any('[PROC-SAMPLE]' in l and 'name=WebContent' in l for l in lines)
    keyboard = any('[GUI-INPUT]' in l and (after_num(l, 'touches_bureau') or 0) > 0 for l in lines)
    mouse = any('[GUI-COMPOSITOR-SOURCES]' in l and (after_num(l, 'souris') or 0) > 0 for l in lines)
    notes = any('BOUCHAUD_PRODUCT_NOTES_SAVED' in l and 'durable=false' in l for l in lines)
    # Aucune preuve de persistance ni d'HTTPS interactif n'est disponible dans le
    # protocole de journalisation v1. Des tests additionnels sont NECESSAIRES.
    statuses = {
        'uefi_boot': boot,
        'desktop': desktop and frames,
        'network_dhcp': network,
        'ladybird_ramdisk': ramdisk,
        'ladybird_runtime': runtime,
        'webcontent': web,
        'keyboard_to_desktop': keyboard,
        'mouse_to_desktop': mouse,
        'notes_saved_ramfs': notes,
        'notes_persistent': False,
        'https_navigation': False,
    }
    panic_patterns = ('KERNEL PANIC', 'DOUBLE FAULT', 'BOUCHAUD_TAS_OOM', 'BOUCHAUD_PILE_NOYAU_DEBORDEE')
    faults = [l[:250] for l in lines if any(token in l for token in panic_patterns)][:30]
    gates = [name for name, _title, blocking in CHECKS if blocking]
    qualified = not faults and all(statuses[name] for name in gates)
    return {
        'schema': 1, 'test': 'FIRST_USABLE_SESSION', 'level': level,
        'qualified': qualified, 'result': 'QUALIFIED' if qualified else 'NON_QUALIFIED',
        'total_lines': len(lines), 'faults': faults,
        'checks': [
            {'id':name, 'label':title, 'blocking':blocking,
             'result':'OBSERVED' if statuses[name] else 'NOT_PROVEN'}
            for name,title,blocking in CHECKS
        ],
        'notes': [
            'NOT_PROVEN n est jamais converti en reussite par manque de traces.',
            'notes_saved_ramfs ne prouve AUCUNE persistance.',
            'https_navigation requiert un verificateur de site et interaction externes.',
            'Ce rapport ne constitue jamais une validation PHYSICAL du Trigkey.',
        ],
    }


def markdown(report: dict) -> str:
    lines = [
        '# FIRST_USABLE_SESSION - Bouchaud OS', '',
        f"Niveau: **{report['level']}** ; verdict: **{report['result']}**", '',
        '| Critere | Preuve | Bloquant |', '|---|---|---|',
    ]
    for c in report['checks']:
        lines.append(f"| {c['label']} | {c['result']} | {'oui' if c['blocking'] else 'non'} |")
    lines += ['', f"Anomalies fatales relevees: {len(report['faults'])}", '']
    lines += [f'- {l}' for l in report['faults']]
    lines += ['', '## Limites', ''] + [f'- {n}' for n in report['notes']]
    return '\n'.join(lines) + '\n'


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--serial', required=True, type=pathlib.Path)
    ap.add_argument('--out', required=True, type=pathlib.Path)
    ap.add_argument('--level', choices=['QEMU-TCG','QEMU-KVM','PHYSICAL'], default='QEMU-TCG')
    a = ap.parse_args()
    if not a.serial.is_file():
        ap.error('journal serie introuvable: '+str(a.serial))
    try:
        raw=a.serial.read_bytes()
        if len(raw)>100*1024*1024:
            ap.error('journal trop grand (>100 MiB)')
        report=parse(raw.decode('utf-8', errors='replace'), level=a.level)
    except OSError as e:
        ap.error(str(e))
    # Le choix PHYSICAL est une declaration de provenance, pas une preuve.
    # Seuls des indicateurs de test physique explicites peuvent y etre utilises.
    if a.level == 'PHYSICAL':
        report['qualified'] = False
        report['result'] = 'NON_QUALIFIED'
        report['notes'].append('Validation PHYSICAL impose une collecte d identite PCI/USB/NVMe et interaction humaine.')
    a.out.mkdir(parents=True, exist_ok=True)
    (a.out/'first_usable_session.json').write_text(json.dumps(report, indent=2, ensure_ascii=False), encoding='utf-8')
    (a.out/'first_usable_session.md').write_text(markdown(report), encoding='utf-8')
    print('FIRST_USABLE_SESSION_'+report['result'])
    for check in report['checks']:
        print(check['id']+'='+check['result'])
    print('REPORT='+str(a.out/'first_usable_session.md'))
    return 0 if report['qualified'] else 3

if __name__ == '__main__':
    sys.exit(main())
