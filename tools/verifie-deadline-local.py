#!/usr/bin/env python3
"""One-shot : modele materiel et mutations executables, sans QEMU fictif."""
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
pure = (ROOT / 'src/kernel/time/reveil_precis.rs').read_text()
test = (ROOT / 'tools/smp/test_reveil_precis.rs').read_text()
smp = (ROOT / 'src/arch/x86_64/smp.rs').read_text()
writer = 'usermode::write_msr(IA32_TSC_DEADLINE, cible);'
assert smp.count('usermode::write_msr(IA32_TSC_DEADLINE,') == 1
assert smp.count('ARME_TSC[cpu].store(') == 1
body = smp.split('fn programme_deadline(', 1)[1].split('\n}', 1)[0]
assert '!::x86_64::instructions::interrupts::are_enabled()' in body
assert body.index('doit_programmer(') < body.index(writer) < body.index('ARME_TSC[cpu].store(')
assert 'EtatDeadline::Invalide as u8' in smp.split('pub fn arm_local_scheduler_timer()', 1)[1].split('\n}', 1)[0]
assert 'observe_deadline(etat,' in smp.split('pub fn note_entree_vecteur_local(', 1)[1].split('\n}', 1)[0]
mutations = [
    ('egalite seule', '!observe_deadline(etat, ancienne, maintenant).arme() || ancienne != nouvelle', 'ancienne != nouvelle'),
    ('expiration ignoree', 'cible <= maintenant', 'false'),
    ('limite exclusive', 'cible <= maintenant', 'cible < maintenant'),
    ('invalidation ignoree', 'Self::Quantum | Self::Interactif | Self::Idle', 'Self::Quantum | Self::Interactif | Self::Idle | Self::Invalide'),
    ('consommation ignoree', 'Self::Quantum | Self::Interactif | Self::Idle', 'Self::Quantum | Self::Interactif | Self::Idle | Self::Expire'),
    ('nouvelle cible ignoree', ' || ancienne != nouvelle', ''),
    ('IPI desarme toujours', 'etat.arme() && cible <= maintenant', 'etat.arme()'),
]
with tempfile.TemporaryDirectory(prefix='p13-deadline-') as directory:
    d = Path(directory)
    (d / 'test.rs').write_text(test.replace('../../src/kernel/time/reveil_precis.rs', 'pure.rs'))
    for name, source in [('reference', pure)] + [(name, pure.replace(old, new, 1)) for name, old, new in mutations]:
        (d / 'pure.rs').write_text(source)
        subprocess.run(['rustc', '--edition=2021', '--test', str(d / 'test.rs'), '-o', str(d / 'test')], check=True, capture_output=True)
        r = subprocess.run([str(d / 'test')], capture_output=True, timeout=10)
        assert (r.returncode == 0) == (name == 'reference'), (name, r.stdout.decode())
        print('DEADLINE_TEST_OK' if name == 'reference' else 'DEADLINE_MUTATION_REJETEE', name)
print('DEADLINE_LOCAL_OK mutations=' + str(len(mutations)))
