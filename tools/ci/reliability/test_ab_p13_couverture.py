#!/usr/bin/env python3
"""Regression P13: P4 OK sur 7/7 ne vaut jamais campagne 8/8."""
import importlib.util
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / 'tools/ci/analyse_ab_reveil_precis.py'
spec = importlib.util.spec_from_file_location('p13_ab_couverture', SCRIPT)
ab = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(ab)


def bras(t, v, non_qualifiant=False):
    p = {
        'q': {phase: (10, 20, 30, 40) for phase in ab.PHASES},
        'tours': [1, 1], 'non_jugeable': {},
        'attribution': set(ab.PHASES),
        'reveils_attribues': {phase: 300 for phase in ab.PHASES},
        'effectifs': {phase: (300, 300, 'preuve-hote') for phase in ab.PHASES},
    }
    return {'tour': t, 'v': v, 'verdict': ab.NQ if non_qualifiant else ab.OK,
            'raison': 'charge sans effet mesurable :' if non_qualifiant else '',
            'detail': [], 'p': p}


class Couverture(unittest.TestCase):
    def analyse_synthetique(self, avec_bras_inadmissible):
        echantillons = [bras(t, mode, t == 3 and mode == 'on' and avec_bras_inadmissible)
                        for t in range(1, 5) for mode in ('off', 'on')]

        def historique(_texte, _tours, _smp, _nom, sortie):
            sortie.extend(echantillons)
            return ['AB_REVEIL_PRECIS : NON_CONCLUANT accel=kvm smp=4'], 'NON_CONCLUANT', 0

        def proprietes(_valides, _tours, _smp, _nom, _grain, politique=False):
            assert politique
            return ['AB_PROPRIETES_V1 : P4=QUALIFIE_OK'], {x: ab.OK for x in ['P1', 'P2', 'P4', 'P5']}

        with patch.object(ab, 'analyse_historique', side_effect=historique), \
             patch.object(ab, 'cinq_proprietes', side_effect=proprietes):
            return ab.analyse('', 4, 4, 'kvm')

    def test_couverture_incomplete_signalee_sans_faux_vert(self):
        lignes, verdict, code = self.analyse_synthetique(True)
        self.assertEqual((verdict, code), ('NON_CONCLUANT', 0))
        self.assertIn('AB_COUVERTURE_NON_EXHAUSTIVE bras_valides=7/8', '\n'.join(lignes))
        self.assertIn('AB_PROPRIETES_POLITIQUE : P4=QUALIFIE_OK', '\n'.join(lignes))

    def test_couverture_complete_sans_alerte(self):
        lignes, verdict, code = self.analyse_synthetique(False)
        self.assertEqual((verdict, code), ('CONVAINCANT', 0))
        self.assertNotIn('AB_COUVERTURE_NON_EXHAUSTIVE', '\n'.join(lignes))


if __name__ == '__main__':
    unittest.main()
