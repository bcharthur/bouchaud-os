#!/usr/bin/env python3
"""bouchaud-sdk abi.h must describe exactly the kernel native ABI.

Compares userland/libs/bouchaud-sdk/include/bouchaud/abi.h with
src/kernel/native/abi/{types,numbers}.rs: version, every syscall number,
every error code, object kinds, rights and signal bits. Any divergence fails
(exit 1). STATIC only: it proves the two descriptions agree, not that the
kernel behaves as described.
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
H = ROOT / 'userland/libs/bouchaud-sdk/include/bouchaud/abi.h'
TY = ROOT / 'src/kernel/native/abi/types.rs'
NR = ROOT / 'src/kernel/native/abi/numbers.rs'

ERRORS = {'InvalidCall': 'INVALID_CALL', 'InvalidArgument': 'INVALID_ARGUMENT',
          'BadHandle': 'BAD_HANDLE', 'WrongType': 'WRONG_TYPE',
          'AccessDenied': 'ACCESS_DENIED', 'WouldBlock': 'WOULD_BLOCK',
          'BufferTooSmall': 'BUFFER_TOO_SMALL', 'QueueFull': 'QUEUE_FULL',
          'PeerClosed': 'PEER_CLOSED', 'NoSpace': 'NO_SPACE', 'Fault': 'FAULT',
          'TooLarge': 'TOO_LARGE', 'NotFound': 'NOT_FOUND', 'TimedOut': 'TIMED_OUT',
          'DeviceError': 'DEVICE'}
KINDS = {'Channel': 'CHANNEL', 'Event': 'EVENT', 'WaitSet': 'WAITSET', 'Update': 'UPDATE', 'Display': 'DISPLAY',
         'SharedRegion': 'SHARED_REGION', 'Process': 'PROCESS'}


def enum(code, name):
    m = re.search(r'pub enum\s+' + name + r'\s*\{([^}]*)\}', code, re.S)
    return {k: int(v, 0) for k, v in re.findall(r'\b(\w+)\s*=\s*(0x[\da-fA-F_]+|\d+)', m.group(1))}


def main():
    h = H.read_text(encoding='utf-8')
    ty = TY.read_text(encoding='utf-8')
    nr = NR.read_text(encoding='utf-8')
    hdefs = {k: v for k, v in re.findall(r'#define\s+(BO_\w+)\s+(.+?)\s*(?:/\*.*)?$', h, re.M)}

    def hval(name):
        expr = hdefs[name]
        expr = re.sub(r'\bBO_NATIVE_BASE\b', '0', expr).replace('UL', '').replace('u', '')
        return eval(expr, {'__builtins__': {}})  # numeric literals and shifts only

    errors = []

    def same(label, rust, c_name):
        try:
            got = hval(c_name)
        except KeyError:
            errors.append(f'{label}: {c_name} absent de abi.h')
            return
        if got != rust:
            errors.append(f'{label}: noyau={rust} sdk={got}')

    for n in ('ABI_MAJOR', 'ABI_MINOR'):
        same(n, int(re.search(r'pub const ' + n + r': u16 = (\d+);', ty).group(1)), 'BO_' + n)
    for name, off in re.findall(r'pub const (\w+): u64 = NATIVE_BASE \+ (0x[0-9a-fA-F]+);', nr):
        same(f'syscall {name}', int(off, 16), 'BO_NR_' + name)
    for rust, c in ERRORS.items():
        same(f'error {rust}', enum(ty, 'Error')[rust], 'BO_E_' + c)
    for rust, c in KINDS.items():
        same(f'kind {rust}', enum(ty, 'ObjectKind')[rust], 'BO_KIND_' + c)
    rights = re.search(r'impl Rights \{(.*?)\n\}', ty, re.S).group(1)
    for name, bit in re.findall(r'pub const (\w+): Self = Self\(1 << (\d+)\);', rights):
        same(f'right {name}', 1 << int(bit), 'BO_RIGHT_' + name)
    signals = re.search(r'impl Signals \{(.*?)\n\}', ty, re.S).group(1)
    for name, bit in re.findall(r'pub const (\w+): Self = Self\(1 << (\d+)\);', signals):
        same(f'signal {name}', 1 << int(bit), 'BO_SIGNAL_' + name)

    for e in errors:
        print('SDK_ABI_DIVERGE', e)
    if errors:
        return 1
    print('SDK_ABI_OK syscalls={} version={}.{}'.format(
        len(re.findall(r'NATIVE_BASE \+', nr)), hval('BO_ABI_MAJOR'), hval('BO_ABI_MINOR')))
    return 0


if __name__ == '__main__':
    sys.exit(main())
