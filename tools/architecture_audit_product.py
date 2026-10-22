#!/usr/bin/env python3
"""Cross-platform truthful report; evidence is not promoted across tiers."""
import argparse
import json
import re
from pathlib import Path


def audit(root):
    def read(path):
        p = root / path
        return p.read_text(encoding='utf-8', errors='replace') if p.exists() else ''

    usermode = read('src/arch/x86_64/usermode.rs')
    native = read('src/kernel/native/abi/numbers.rs')
    rootfs = read('src/fs/mod.rs')
    main = read('src/main.rs')
    tests = read('tools/test.sh')
    findings = [
        {
            'id': 'ring3', 'scope': 'kernel/user privilege',
            'code_present': 'syscall_entry' in usermode and 'syscall_dispatch' in usermode and 'USER_CS' in usermode,
            'runtime_proof': 'PENDING',
            'evidence': 'tools/test.sh ring3-selftest; tools/native/run-native-ipc-ring3.ps1',
            'warning': 'Un syscall present ne prouve pas une isolation CPL3/MMU.'
        },
        {
            'id': 'native_abi', 'scope': 'native syscalls',
            'code_present': 'pub const VERSION' in native and 'pub const CHANNEL_CREATE' in native,
            'runtime_proof': 'PENDING',
            'evidence': 'tools/native/test_native_abi.rs; tools/verifie-contrat-abi-v1.py',
            'warning': 'Version 1 existante, pas licence de compatibilite perpetuelle.'
        },
        {
            'id': 'filesystem', 'scope': 'VFS/disk',
            'code_present': 'pub mod ramfs' in rootfs and 'pub mod persistance' in rootfs,
            'runtime_proof': 'PENDING',
            'evidence': 'tools/verifie-persistance.py + deux boots avec disque data',
            'warning': 'RAMFS + archive + sync A/B ne sont pas un FS bloc general avec cache unifie.'
        },
        {
            'id': 'isolation_services', 'scope': 'GUI/userland',
            'code_present': 'mod gui;' in main,
            'runtime_proof': 'PENDING',
            'evidence': 'Mesurer CPL et CR3 de chaque service + crash injecte du WM',
            'warning': 'Code GUI lie au noyau, migration graduelle vers processus userland requise.'
        },
        {
            'id': 'security_debt', 'scope': 'Rust lint budget',
            'code_present': '#![allow(static_mut_refs)]' in main,
            'runtime_proof': 'NOT_APPLICABLE',
            'evidence': 'Retrait progressif par module apres revue des alias/locks',
            'warning': 'allow(static_mut_refs) crate-wide : dette de surete, ne pas la cacher.'
        },
        {
            'id': 'host_runtime', 'scope': 'ring3 regression suite',
            'code_present': 'ring3-selftest' in tests,
            'runtime_proof': 'PENDING',
            'evidence': 'tools/test.sh (ne conclure vert que sur log de ce run)',
            'warning': 'STATIC/HOST != QEMU != PHYSICAL.'
        },
    ]
    return {'schema': 1, 'findings': findings, 'status': 'SOURCE_AUDIT_ONLY'}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--repo', type=Path, default=Path(__file__).resolve().parents[1])
    ap.add_argument('--json', type=Path)
    a = ap.parse_args()
    report = audit(a.repo.resolve())
    for f in report['findings']:
        print(f"{f['id']}: source_present={int(f['code_present'])} runtime={f['runtime_proof']} | {f['warning']}")
    if a.json:
        a.json.parent.mkdir(parents=True, exist_ok=True)
        a.json.write_text(json.dumps(report, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
    return 0 if all(f['code_present'] for f in report['findings']) else 2


if __name__ == '__main__':
    raise SystemExit(main())
