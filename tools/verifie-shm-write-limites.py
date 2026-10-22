#!/usr/bin/env python3
"""SHM_WRITE must check bounds before copying attacker-controlled user bytes."""
import sys
from pathlib import Path

CODE = Path(__file__).resolve().parents[1] / 'src/kernel/native/abi/dispatch.rs'


def check(source):
    start = source.find('numbers::SHM_WRITE => {')
    end = source.find('\n        _ => Err(Error::InvalidCall)', start)
    if start < 0 or end < 0:
        return ['branche SHM_WRITE non reconnue']
    branch = source[start:end]
    checks = ('offset.checked_add(length)', 'end > region.len()',
              'super::usercopy::read(args[2], length)', 'region.write(offset, &data)')
    if not all(marker in branch for marker in checks):
        return ['contrat SHM_WRITE avant usercopy manquant']
    if not (branch.index('offset.checked_add(length)') < branch.index('end > region.len()')
            < branch.index('super::usercopy::read(args[2], length)')
            < branch.index('region.write(offset, &data)')):
        return ['SHM_WRITE fait un usercopy avant controle des bornes']
    return []


def main():
    errors = check(CODE.read_text(encoding='utf-8'))
    if errors:
        for err in errors:
            print('ARCH_SHM_BOUNDS_FAIL', err)
        return 1
    print('ARCH_SHM_BOUNDS_STATIC_OK')
    print('RUNTIME_PENDING: rejouer les tests SHM_READ/WRITE natifs sous QEMU')
    return 0


if __name__ == '__main__':
    sys.exit(main())
