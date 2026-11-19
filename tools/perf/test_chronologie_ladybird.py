import unittest

from chronologie_ladybird import chronologie


class Chronologie(unittest.TestCase):
    def test_horloges_separees_et_minuit(self):
        lignes = (
            "[23:59:59] [LB:FRAME] t=900 seq=1 present_us=100\n"
            "[00:00:01] [PERSIST-SLOW] total_ns=900000000\n"
            "[00:00:01] [PERSIST-SLOW] t_ms=2900 pid=20 io_ns=800000000\n"
            "[00:00:02] 3.010 WebContent(22): HTMLParserEndState: timed out in phase 3"
        )
        trame, ancien, nouveau, parseur = chronologie(lignes)
        self.assertEqual((trame["serie_s"], trame["invite_ms"]), (0, 900))
        self.assertEqual(ancien["serie_s"], 2)
        self.assertNotIn("invite_ms", ancien)
        self.assertEqual((nouveau["invite_ms"], nouveau["pid"]), (2900, 20))
        self.assertEqual((parseur["invite_ms"], parseur["pid"], parseur["phase"]), (3010, 22, 3))

    def test_domaine_sans_url_privee_et_classe(self):
        lignes = (
            "[12:00:00] [LB:NAV] onglet=1 demande url=https://example.com/path?q=secret\n"
            "[12:00:01] [SCHED-NG-CENTILES] classe=interactive p99_ns=100\n"
        )
        navigation, attente = chronologie(lignes)
        self.assertEqual(navigation["domaine"], "example.com")
        self.assertNotIn("secret", str(navigation))
        self.assertEqual((attente["classe"], attente["p99_ns"]), ("interactive", 100))
