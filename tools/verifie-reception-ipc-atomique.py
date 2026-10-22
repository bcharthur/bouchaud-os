#!/usr/bin/env python3
"""Conservative structural gate around the three native IPC copyout failure paths."""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CODE = ROOT / 'src/kernel/native/abi/dispatch.rs'


def check(src):
    errors = []
    start = src.find('fn channel_recv(args: [u64; 6]) -> Result<u64> {')
    end = src.find('\nfn handle_info(', start + 1)
    if start < 0 or end < 0:
        return ['channel_recv ou frontiere handle_info introuvable']
    body = src[start:end]
    if 'let message = channel.recv()?;' not in body:
        errors.append('dequeue message introuvable')
    # A size check on the actual popped message is necessary: front_sizes() and
    # recv() are separate operations and another receiver can race between them.
    if 'message.bytes.len() > data_cap' not in body or 'message.handles.len() > handles_cap' not in body:
        errors.append('pas de revalidation capacite apres dequeue (race multi-consommateurs)')
    # At least one requeue for overflow, one for each copyout failure.
    if body.count('channel.requeue_front(message);') < 4:
        errors.append('requeue manquant en cas de copie memoire utilisateur invalide')
    for copy in ('write(args[1], &message.bytes)', 'write_handles(args[3], &installed)',
                 'write(meta_ptr, &meta.bytes_le())'):
        # Ensure the failure handler following the copy contains a requeue.
        i = body.find('if let Err(error) = super::usercopy::' + copy)
        if i < 0:
            errors.append('branche copyout absente: ' + copy)
            continue
        snippet = body[i:body.find('\n    }', i) + 6]
        if 'channel.requeue_front(message);' not in snippet:
            errors.append('message perdu sur echec: ' + copy)
    return errors


def main():
    errors = check(CODE.read_text(encoding='utf-8'))
    for error in errors:
        print('ARCH_IPC_RECV_FAIL', error)
    if errors:
        return 1
    print('ARCH_IPC_RECV_STATIC_OK requeue=copyout+overflow')
    print('RUNTIME_PENDING: rejouer probes ring3 et contention sous QEMU')
    return 0


if __name__ == '__main__':
    sys.exit(main())
