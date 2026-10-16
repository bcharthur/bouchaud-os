"""Régressions non destructives du pack de qualification P13 / Trigkey."""
import sys
import unittest
from pathlib import Path

CI = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(CI))

import p13_evidence_report as evidence
import p13_kvm_host_preflight as host
import p13_kvm_soak as soak
import p13_smp8_gate as smp8
import trigkey_evidence as trigkey


class P13Evidence(unittest.TestCase):
    def test_non_qualifiant_host_plus_charge(self):
        raw = '\n'.join([
            'ORDO_ATTRIBUTION_FIN',
            '  comptes au repos : preuve_hote=kvm-steal prouves=0 inexpliques=3 ecartes=0',
            'VERDICT_SONDE : NON_QUALIFIANT raison=charge sans effet mesurable',
            'VERDICT_SONDE : NON_QUALIFIANT raison=phase non jugeable retards par hote',
        ])
        d = evidence.diagnostique('ordonnanceur-attribution-kvm-smp8/serie.log', raw)
        self.assertEqual(d['qualification_attribution_indicative'], 'NON_ETABLIE')
        self.assertEqual(d['retards_inexpliques'], 3)
        self.assertEqual(d['verdicts']['NON_QUALIFIANT'], 2)
        self.assertEqual(d['causes']['charge_non_discriminante'], 1)

    def test_shell_echo_ne_vaut_pas_fin(self):
        d = evidence.diagnostique('ordonnanceur-attribution-kvm-smp1/serie.log',
                                  '+ echo ORDO_ATTRIBUTION_FIN\n')
        self.assertFalse(d['fin_executee'])

    def test_unqualified_independent_of_good_workflow(self):
        d = evidence.diagnostique('ordonnanceur-attribution-kvm-smp4/serie.log',
                                  'VERDICT_SONDE : QUALIFIE_OK\n')
        self.assertEqual(d['qualification_attribution_indicative'], 'NON_ETABLIE')


class KvmPreflight(unittest.TestCase):
    def test_cgroup_quota(self):
        self.assertEqual(host.quota_cpus('400000 100000'), 4)
        self.assertIsNone(host.quota_cpus('max 100000'))
        self.assertEqual(host.quota_cpus('erreur'), 0)

    def test_shared_host_refused(self):
        faults = host.controle(8, 8.0, 8, True, [], False, 1.0)
        self.assertTrue(any('dédié' in f for f in faults))

    def test_no_quota_no_cpu_hard_block(self):
        self.assertEqual(host.controle(12, None, 8, True, [], True, 1.0), [])


class SMP8AndSoak(unittest.TestCase):
    def test_echo_cannot_fake_smp8(self):
        self.assertTrue(smp8.inspect('+ echo ORDO_ATTRIBUTION_FIN\n'))

    def test_qemu_shell_echo_cannot_fake_endurance(self):
        good, missing = soak.markers_ok('+ echo ENDURANCE_ORDONNANCEUR_OK\nVERDICT_SONDE : NON_QUALIFIANT',
                                        ['ENDURANCE_ORDONNANCEUR_OK'])
        self.assertFalse(good)
        self.assertGreater(len(missing), 0)


class TrigkeyEvidence(unittest.TestCase):
    def test_missing_criteria_always_red(self):
        d = trigkey.evaluate('BOOTED', {'markers': {}})
        self.assertEqual(d['result'], 'NOT_ESTABLISHED')

    def test_fatal_blocks_success(self):
        criteria = {'markers': {k: k.upper() for k in trigkey.PHASES}}
        text = '\n'.join(k.upper() for k in trigkey.PHASES) + '\nKERNEL PANIC!'
        self.assertEqual(trigkey.evaluate(text, criteria)['result'], 'NOT_ESTABLISHED')

    def test_complete_evidence(self):
        criteria = {'markers': {k: k.upper() for k in trigkey.PHASES}}
        text = '\n'.join(k.upper() for k in trigkey.PHASES)
        self.assertEqual(trigkey.evaluate(text, criteria)['result'], 'PHYSICAL_SMOKE_OK')


if __name__ == '__main__':
    unittest.main()
