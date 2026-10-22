#!/usr/bin/env python3
"""Fail-closed native ABI v1 guard: existing numbers, rights and wire layout never drift."""
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONTRACT = ROOT / 'tools' / 'architecture' / 'abi-v1-contract.json'


def get_enum(code, name):
    m = re.search(r'pub enum\s+' + re.escape(name) + r'\s*\{([^}]*)\}', code, re.S)
    if not m:
        raise ValueError(f'enum {name} introuvable')
    return {k: int(v, 0) for k, v in re.findall(r'\b(\w+)\s*=\s*(0x[\da-fA-F_]+|\d+)\s*,?', m.group(1))}


def collect(root):
    d = root / 'src/kernel/native/abi'
    ty = (d / 'types.rs').read_text(encoding='utf-8')
    nr = (d / 'numbers.rs').read_text(encoding='utf-8')
    wi = (d / 'wire.rs').read_text(encoding='utf-8')
    ver = {}
    for n in ('ABI_MAJOR', 'ABI_MINOR'):
        m = re.search(r'pub const\s+' + n + r':\s*u16\s*=\s*(\d+)\s*;', ty)
        if not m:
            raise ValueError(f'constante {n} absente')
        ver[n] = int(m.group(1))
    nums = {n: int(v.replace('_', ''), 16) for n, v in re.findall(
        r'pub const\s+(\w+):\s*u64\s*=\s*NATIVE_BASE\s*\+\s*(0x[0-9a-fA-F_]+)\s*;', nr)}
    rights = {n: int(bit) for n, bit in re.findall(
        r'pub const\s+(\w+):\s*Self\s*=\s*Self\(1\s*<<\s*(\d+)\)\s*;', ty)}
    wire = {}
    for name in ('RecvMeta', 'HandleInfo', 'WaitEvent'):
        m = re.search(r'pub struct\s+' + name + r'\s*\{([^}]*)\}', wi, re.S)
        if not m:
            raise ValueError(f'wire {name} absent')
        fields = re.findall(r'pub\s+(\w+):\s*(u\d+)', m.group(1))
        wire[name] = [f'{field}:{kind}' for field, kind in fields]
    return dict(version=ver, syscalls=nums, errors=get_enum(ty, 'Error'),
                object_kinds=get_enum(ty, 'ObjectKind'), rights_bits=rights, wire=wire)


def check(snapshot, contract):
    errors = []
    for group in ('syscalls', 'errors', 'object_kinds', 'rights_bits', 'wire'):
        for key, expected in contract[group].items():
            if snapshot[group].get(key) != expected:
                errors.append(f'{group}.{key}: attendu {expected!r}, trouve {snapshot[group].get(key)!r}')
    if snapshot['version']['ABI_MAJOR'] != contract['version']['ABI_MAJOR']:
        errors.append('ABI_MAJOR a change : nouvelle ABI majeure a discuter explicitement')
    additions = sorted(set(snapshot['syscalls']) - set(contract['syscalls']))
    if additions and snapshot['version']['ABI_MINOR'] <= contract['version']['ABI_MINOR']:
        errors.append('nouveaux syscalls sans augmentation de ABI_MINOR : ' + ','.join(additions))
    if snapshot['version']['ABI_MINOR'] < contract['version']['ABI_MINOR']:
        errors.append('ABI_MINOR a regresse')
    seen = {}
    for name, number in snapshot['syscalls'].items():
        if number in seen:
            errors.append(f'numero duplique: {name} et {seen[number]} = {number:#x}')
        seen[number] = name
        if not (0 <= number <= 0xff):
            errors.append(f'numero hors namespace natif: {name} = {number:#x}')
    return errors


def main():
    try:
        snap = collect(ROOT)
        contract = json.loads(CONTRACT.read_text(encoding='utf-8'))
        errors = check(snap, contract)
    except (OSError, ValueError, KeyError) as exc:
        errors = [str(exc)]
    for err in errors:
        print('ARCH_ABI_V1_FAIL', err)
    if errors:
        return 1
    print('ARCH_ABI_V1_OK preserved={} native_calls={} version={}.{}'.format(
        len(contract['syscalls']), len(snap['syscalls']),
        snap['version']['ABI_MAJOR'], snap['version']['ABI_MINOR']))
    print('STATIC_ONLY: test reel ring3 non deduit de ce resultat')
    return 0


if __name__ == '__main__':
    sys.exit(main())
