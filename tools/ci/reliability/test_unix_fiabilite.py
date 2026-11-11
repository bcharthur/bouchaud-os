"""Unix & Filesystem Reliability : le classement des etapes du banc QEMU.

Le banc ne vaut que s'il distingue ce qui a reussi de ce qui n'a pas tourne :
un TIMEOUT n'est jamais un succes, une machine vivante dont le shell ne repond
plus non plus, et une etape apres une panique n'a PAS ete executee.
"""
import importlib.util
import sys
import unittest
from pathlib import Path

RACINE = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("uf", RACINE / "tools/ci/unix_fiabilite.py")
uf = importlib.util.module_from_spec(spec)
sys.modules["uf"] = uf  # dataclasses resout ses annotations par le module
spec.loader.exec_module(uf)

P = "[01:00:00][ 1%: 0%: 0%][FPS: --] "


def etape(ident, sortie, fin=True, temoin=True):
    l = [P + f"+ echo UNIX_DEBUT {ident}", P + f"UNIX_DEBUT {ident}", P + "+ find /etc"]
    l += [P + s for s in sortie]
    if fin:
        l += [P + f"+ echo UNIX_FIN {ident}", P + f"UNIX_FIN {ident}"]
    if temoin:
        l += [P + f"UNIX_TEMOIN {ident}"]
    return l


E1 = uf.Etape("a", "find /etc", attendus=[r"^/etc/hosts$"], toutes=r"^/etc/")
E2 = uf.Etape("b", "find /bin", lignes=0)


class ClassementTests(unittest.TestCase):
    def test_pass(self):
        r = uf.classe([E1, E2], etape("a", ["/etc/hosts"]) + etape("b", []), "eteinte")
        self.assertEqual([x["statut"] for x in r], ["PASS", "PASS"])

    def test_fail_contenu(self):
        r = uf.classe([E1], etape("a", ["/etc/passwd"]), "eteinte")
        self.assertEqual(r[0]["statut"], "FAIL")
        self.assertTrue(any("absent" in x for x in r[0]["raisons"]))
        r = uf.classe([E1], etape("a", ["/etc/hosts", "/tmp/x"]), "eteinte")
        self.assertEqual(r[0]["statut"], "FAIL", "une ligne hors du motif impose")

    def test_lignes_du_noyau_ecartees(self):
        r = uf.classe([E1], etape("a", ["/etc/hosts", "[kernel] PROCESS_EXIT pid=3", "BOUCHAUD_X a=1"]), "eteinte")
        self.assertEqual(r[0]["statut"], "PASS")

    def test_panique_puis_not_run(self):
        l = etape("a", [], fin=False, temoin=False) + [P + "*** KERNEL PANIC ***", P + "panicked at src/fs/ramfs.rs:213:8:"]
        r = uf.classe([E1, E2], l, "panique")
        self.assertEqual([x["statut"] for x in r], ["KERNEL_PANIC", "NOT_RUN"])

    def test_lockdep_compte_comme_panique(self):
        l = etape("a", [], fin=False, temoin=False) + [P + "LOCKDEP inversion cpu=0 held_rank=50"]
        self.assertEqual(uf.classe([E1], l, "panique")[0]["statut"], "KERNEL_PANIC")

    def test_machine_muette_est_un_deadlock_suspecte(self):
        r = uf.classe([E1, E2], etape("a", [], fin=False, temoin=False), "muette")
        self.assertEqual([x["statut"] for x in r], ["DEADLOCK_SUSPECTED", "NOT_RUN"])

    def test_console_active_sans_fin_est_un_timeout(self):
        r = uf.classe([E1], etape("a", ["/etc/hosts"] * 50, fin=False, temoin=False), "bavarde")
        self.assertEqual(r[0]["statut"], "TIMEOUT")

    def test_fin_sans_temoin_n_est_pas_un_succes(self):
        # La commande a rendu la main, mais le shell ne repond plus au temoin.
        r = uf.classe([E1], etape("a", ["/etc/hosts"], temoin=False), "muette")
        self.assertEqual(r[0]["statut"], "DEADLOCK_SUSPECTED")

    def test_etape_jamais_commencee(self):
        r = uf.classe([E1, E2], etape("a", ["/etc/hosts"]), "eteinte")
        self.assertEqual([x["statut"] for x in r], ["PASS", "NOT_RUN"])

    def test_aucun_statut_hors_liste(self):
        for fin in ("eteinte", "panique", "muette", "bavarde"):
            for l in (etape("a", []), etape("a", [], fin=False, temoin=False)):
                for x in uf.classe([E1, E2], l, fin):
                    self.assertIn(x["statut"], uf.STATUTS)

    def test_script_encadre_chaque_etape(self):
        s = uf.script(["journal off"], [E1, E2]).splitlines()
        i = s.index("find /etc")
        self.assertEqual(s[i - 1], "echo UNIX_DEBUT a")
        self.assertEqual(s[i + 1:i + 3], ["echo UNIX_FIN a", "echo UNIX_TEMOIN a"])
        self.assertEqual(s[-2:], ["echo UNIX_SCENARIO_FIN", "poweroff"])


if __name__ == "__main__":
    unittest.main()
