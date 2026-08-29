#!/usr/bin/env python3
from pathlib import Path
import sys

root = Path(__file__).resolve().parents[2]
checks = {
    'src/gui/frame_clock.rs': [
        'pub fn note_frame(horloge_seule: bool)',
        'pub fn snapshot() -> Snapshot',
        '[FRAME-CLOCK]',
        'SAMPLE_MIN_TICKS: u64 = 500',
    ],
    'src/gui/mod.rs': ['pub mod frame_clock;'],
    'src/gui/reveil_v9.rs': [
        'crate::gui::frame_clock::note_frame(horloge_seule);',
        'crate::gui::frame_clock::publie();',
    ],
    'src/kernel/debug/journal.rs': [
        '[FPS:',
        '[Hz:',
        'crate::gui::frame_clock::snapshot()',
    ],
}

for rel, tokens in checks.items():
    p = root / rel
    if not p.exists():
        print('V14.1 missing:', rel)
        sys.exit(1)
    text = p.read_text(encoding='utf-8')
    for token in tokens:
        if token not in text:
            print('V14.1 contract missing:', rel, '=>', token)
            sys.exit(2)

hot = (root/'src/gui/frame_clock.rs').read_text(encoding='utf-8')
for forbidden in ('SpinLock', 'Mutex', 'alloc::', 'Vec<', 'String'):
    if forbidden in hot:
        print('V14.1 frame clock hot path is not lock/allocation free:', forbidden)
        sys.exit(3)

policy = (root/'src/gui/politique.rs').read_text(encoding='utf-8')
if 'pub const PERIODE_TRAME_MS: u64 = 16;' not in policy:
    print('V14.1 note: PERIODE_TRAME_MS is no longer 16 ms; verify target_hz semantics')

print('V14.1 FPS/Hz telemetry contracts: OK')
