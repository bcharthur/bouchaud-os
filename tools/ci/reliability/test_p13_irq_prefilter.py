#!/usr/bin/env python3
"""Mutations du prefiltre dans le chemin de reveil IRQ P13."""
from pathlib import Path
import unittest
P = Path(__file__).resolve().parents[3] / 'src/kernel/process/thread/sommeil.rs'
def body(s):
    a=s.index('pub fn sert_echeances_au_vecteur_local() {')
    b=s.index('\n/// ',a)
    return '\n'.join(x for x in s[a:b].splitlines() if not x.lstrip().startswith('//'))
def safe(s):
    a='if !PROCHAINE_ECHEANCE.doit_balayer(crate::kernel::timer::monotonic_ns()) {\n        return;\n    }'
    b='if !crate::kernel::scheduler::preempt::preemption_noyau_sure()'
    c='|| transition_ouverte_sans_passation(local_cpu())'
    d='wake_sleepers_depuis_irq();'
    pos=[s.find(x) for x in [a,b,c,d]]
    return all(x>=0 for x in pos) and pos==sorted(pos) and s.count(d)==1 and 'commence_balayage' not in s
class TestPrefiltre(unittest.TestCase):
    def test_ordre_de_production(self):
        self.assertTrue(safe(body(P.read_text())))
    def test_mutations(self):
        s=body(P.read_text())
        mutations=[
            s.replace('if !PROCHAINE_ECHEANCE.doit_balayer','if PROCHAINE_ECHEANCE.doit_balayer'),
            s.replace('PROCHAINE_ECHEANCE.doit_balayer','PROCHAINE_ECHEANCE.commence_balayage'),
            s.replace('preemption_noyau_sure()','preemption_noyau_non_sure()'),
            s.replace('transition_ouverte_sans_passation(local_cpu())','false'),
            s.replace('wake_sleepers_depuis_irq();','/*skip*/'),
            s.replace('return;\n    }','/*continue*/\n    }',1),
            s.replace('    if !PROCHAINE_ECHEANCE.doit_balayer','    wake_sleepers_depuis_irq();\n    if !PROCHAINE_ECHEANCE.doit_balayer'),
        ]
        for i,m in enumerate(mutations):
            with self.subTest(mutation=i):
                self.assertFalse(safe(m))
    def test_bash_runner_parameter_expansion(self):
        root = P.parents[3]
        runner = (root / "tools/ci/run_p13_irq_prefilter_validation.sh").read_text()
        host = (root / "tools/smp/test-irq-deadline-prefilter.sh").read_text()
        self.assertIn('mode="${1:-host}"', runner)
        self.assertIn('"${RUSTC:=rustc}"', host)
        self.assertIn('"${TMPDIR:-/tmp}/p13-irq-prefilter.XXXXXX"', host)
        self.assertNotIn(r'\${', runner)
        self.assertNotIn(r'\${', host)
if __name__=='__main__':
    unittest.main()
