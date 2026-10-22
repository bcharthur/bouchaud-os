#!/usr/bin/env python3
"""Static fail-closed guard for IPC output destination validation and rollback.

Passing this static gate does NOT prove an IPC race test under QEMU.
"""
from pathlib import Path
import sys
ROOT=Path(__file__).resolve().parents[1]
MARK='BOUCHAUD_MATURITY_IPC_PREFLIGHT_V1'


def check(usercopy:str, dispatch:str)->list[str]:
    issues=[]
    if MARK not in usercopy or MARK not in dispatch:
        issues.append('missing marker')
    if 'pub fn validate_write(addr: u64, len: usize) -> Result<()>' not in usercopy:
        issues.append('missing validate_write')
    if 'resident_range(addr, len, true)' not in usercopy:
        issues.append('validation must check writable residence')
    a=dispatch.find('fn channel_recv(args: [u64; 6]) -> Result<u64> {')
    b=dispatch.find('\nfn handle_info(',a)
    if a<0 or b<0:
        return issues+['missing channel_recv boundaries']
    body=dispatch[a:b]
    marks=['let message = channel.recv()?;',
           'super::usercopy::validate_write(args[1], message.bytes.len())?',
           'super::usercopy::validate_write(',
           'message.handles.len().checked_mul(8).ok_or(Error::TooLarge)?',
           'super::usercopy::validate_write(meta_ptr, RecvMeta::BYTE_LEN)?',
           'if let Err(error) = copyout_preflight {',
           'let installed = match table.insert_many(&entries) {']
    positions=[]
    for m in marks:
        i=body.find(m)
        if i<0:
            issues.append('missing: '+m)
        positions.append(i)
    if any(i<0 for i in positions) or positions!=sorted(positions):
        issues.append('copyout preflight not before handles installation')
    if 'if let Err(error) = copyout_preflight {\n        channel.requeue_front(message);\n        return Err(error);' not in body:
        issues.append('failed preflight does not requeue')
    if body.count('channel.requeue_front(message);') < 5:
        issues.append('copyout rollback paths removed')
    for copy in ('write(args[1], &message.bytes)', 'write_handles(args[3], &installed)',
                 'write(meta_ptr, &meta.bytes_le())'):
        if copy not in body:
            issues.append('missing runtime copyout: '+copy)
    return issues


def main()->int:
    code=(ROOT/'src/kernel/native/abi/usercopy.rs').read_text(encoding='utf8')
    dispatch=(ROOT/'src/kernel/native/abi/dispatch.rs').read_text(encoding='utf8')
    issues=check(code,dispatch)
    for issue in issues:
        print('ARCH_IPC_COPYOUT_PREFLIGHT_FAILED:',issue)
    if issues:
        return 1
    print('ARCH_IPC_COPYOUT_PREFLIGHT_STATIC_OK')
    print('RUNTIME_PENDING: test faulting user buffers, COW, handles and competing receivers in guest ring3')
    return 0

if __name__=='__main__':
    sys.exit(main())
