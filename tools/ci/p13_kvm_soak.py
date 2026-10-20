#!/usr/bin/env python3
"""Endurance SMP8 EXPLICITEMENT sous KVM, dédiée, sans extrapoler depuis TCG.

N'édite pas les critères P13 ; aucun `NON_QUALIFIANT` ne compte comme cycle réussi.
Le scénario est créé par fabrique-scenario-endurance.sh et doit être fourni.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

# Le détecteur du dépôt est importé uniquement au moment du run.

ANSI = re.compile(r'\x1b\[[0-9;]*[A-Za-z]')
PREFIX = re.compile(r'^(?:\[[^\]]*\]){2}\[FPS:[^\]]*\] ')


def markers_ok(raw: str, required: list[str]) -> tuple[bool, list[str]]:
    lines = [PREFIX.sub('', ANSI.sub('', x.rstrip('\r'))) for x in raw.splitlines()]
    # Les préfixes shell `+ echo` ne peuvent pas se substituer aux sorties.
    missing = [x for x in required if not any(x in line and not line.startswith('+ ') for line in lines)]
    verdicts = [x for x in lines if x.startswith('VERDICT_SONDE : ')]
    qualified = len(verdicts) >= 1 and all(x.startswith('VERDICT_SONDE : QUALIFIE_OK') for x in verdicts)
    if not verdicts:
        missing.append('VERDICT_SONDE absent')
    elif not qualified:
        missing.append('VERDICT_SONDE non QUALIFIE_OK')
    # Le label KVM ne remplace pas la preuve du temps volé provenant du noyau :
    # exiger trois comptes par verdict (repos, normal, interactif).
    proofs = [line for line in lines if re.search(r'^\s+comptes [^:]+ : preuve_hote=(\S+)', line)]
    if len(proofs) < 3 * len(verdicts) or any('preuve_hote=kvm-steal' not in line for line in proofs):
        missing.append(f'comptes kvm-steal incomplets: {len(proofs)}/{3 * len(verdicts)}')
    return not missing, missing


def run_one(qemu: str, boot: Path, disk: Path, log: Path, sec: int, required: list[str]) -> dict:
    sys.path.insert(0, str(Path(__file__).resolve().parent / 'reliability'))
    from logscan import scan_file
    log.parent.mkdir(parents=True, exist_ok=True)
    image = log.parent / 'scenario.img'
    shutil.copyfile(disk, image)
    command = [qemu, '-enable-kvm', '-cpu', 'host,kvm-steal-time=on',
               '-drive', f'format=raw,file={boot}', '-drive', f'format=raw,file={image}',
               '-m', '4096', '-smp', '8', '-display', 'none', '-no-reboot',
               '-netdev', 'user,id=net0', '-device', 'e1000,netdev=net0',
               '-audiodev', 'none,id=muet', '-device', 'AC97,audiodev=muet',
               '-serial', f'file:{log}']
    timed_out = False
    rc = None
    try:
        proc = subprocess.run(command, timeout=sec, check=False)
        rc = proc.returncode
    except subprocess.TimeoutExpired:
        timed_out = True
    scanned = scan_file(log, required_markers=required)
    text = log.read_text(encoding='utf-8', errors='replace') if log.exists() else ''
    strict, missing = markers_ok(text, required + ['ENDURANCE_ORDONNANCEUR_OK', '=== AUTORUN FIN === statut=0'])
    findings = [{'kind': f.kind, 'line': f.line, 'text': f.text} for f in scanned.findings]
    result = {'log': str(log), 'seconds': sec, 'timed_out': timed_out,
              'qemu_rc': rc, 'required_missing': list(scanned.required_markers_missing) + missing,
              'findings': findings, 'kvm_requested': True,
              'ok': (timed_out or rc == 0) and scanned.ok and strict}
    if result['ok']:
        image.unlink(missing_ok=True)
    return result


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('bootimage', type=Path)
    ap.add_argument('--disk', type=Path, required=True)
    ap.add_argument('--markers', type=Path, required=True)
    ap.add_argument('--minutes', type=int, choices=(30, 60, 90, 150), default=30)
    ap.add_argument('--cycle-seconds', type=int, default=300)
    ap.add_argument('--output', type=Path, required=True)
    ns = ap.parse_args()
    if ns.cycle_seconds < 120:
        ap.error('--cycle-seconds >= 120 requis pour le scénario complet')
    for f in (ns.bootimage, ns.disk, ns.markers):
        if not f.is_file():
            ap.error(f'fichier absent : {f}')
    required = [l.strip() for l in ns.markers.read_text(encoding='utf-8').splitlines() if l.strip()]
    if not required:
        ap.error('marqueurs vides')
    qemu = shutil.which('qemu-system-x86_64')
    if not qemu:
        ap.error('QEMU absent')
    ns.output.mkdir(parents=True, exist_ok=False)
    total = ns.minutes * 60
    # Nombre de cycles entiers, pas budget tronqué par le coût des redémarrages.
    expected_min = (total + ns.cycle_seconds - 1) // ns.cycle_seconds
    results = []
    for _ in range(expected_min):
        idx = len(results) + 1
        log = ns.output / f'cycle-{idx:04d}' / 'smp8.log'
        r = run_one(qemu, ns.bootimage, ns.disk, log, ns.cycle_seconds, required)
        results.append(r)
        print(f"SMP8_KVM_CYCLE n={idx} verdict={'QUALIFIE' if r['ok'] else 'NON_QUALIFIANT'} missing={len(r['required_missing'])} fatals={len(r['findings'])}", flush=True)
    # Aucun cycle tronqué ou ignoré ne contribue à la preuve.
    qualified = len(results) >= expected_min and all(r['ok'] for r in results)
    payload = {'schema': 1, 'mode': 'kvm_host_8cpu', 'boot_sha256': hashlib.sha256(ns.bootimage.read_bytes()).hexdigest(),
               'cycles': results, 'expected_min_cycles': expected_min,
               'result': 'QUALIFIE' if qualified else 'NOT_ESTABLISHED'}
    (ns.output / 'summary.json').write_text(json.dumps(payload, indent=2) + '\n', encoding='utf-8')
    print(f"SMP8_KVM_ENDURANCE verdict={payload['result']} cycles={len(results)}/{expected_min}")
    return 0 if qualified else 1


if __name__ == '__main__':
    raise SystemExit(main())
