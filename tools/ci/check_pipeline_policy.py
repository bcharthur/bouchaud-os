#!/usr/bin/env python3
"""Garde-fous simples contre le retour de la CI monolithique."""
from pathlib import Path
import sys

root = Path('.github/workflows')
errors = []

# Ces workflows sont volontairement des outils de debug / campagnes lourdes.
manual_only = {
    'system-health.yml',
    'os-primitives.yml',
    'mm-ng6-smp4.yml',
    'ladybird-browser-host.yml',
    'ladybird-platform-smp4.yml',
    'ladybird-native-browser-v16.yml',
}
for name in manual_only:
    text = (root / name).read_text(encoding='utf-8')
    if '\n  push:' in text or '\n  pull_request:' in text or '\n  schedule:' in text:
        errors.append(f'{name}: doit rester manuel dans CI v2')

# Le producteur canonique doit conserver le nom consomme par run.ps1.
canon = (root / 'ladybird-native-browser.yml').read_text(encoding='utf-8')
if 'name: ladybird-native-browser' not in canon:
    errors.append('ladybird-native-browser.yml: nom canonique absent')
if 'bouchaud-ladybird-native-browser' not in canon:
    errors.append('ladybird-native-browser.yml: artefact canonique absent')

if errors:
    print('\n'.join('ECHEC: ' + e for e in errors), file=sys.stderr)
    raise SystemExit(1)
print('CI_POLICY_OK')
