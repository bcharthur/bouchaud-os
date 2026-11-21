"""Tests de BOUCHAUD_RAPPORT_GEL_V1 (rapport_gel.py)."""
import unittest

import rapport_gel as rg

# Ligne reelle du gel de main@eb6f115 (job 132498554631), telle que le banc
# l'avait coupee : sans `<site_prise>` ni `nmi=` (format de #324).
ANCIENNE = (
    "[18:15:20][ 72%:  4%:  5%][FPS:  0] [DIAG-EN-SOUFFRANCE] t=840000 demande_ms=811090 fil=1 "
    "cur=[9,50,20,60] site=[740 212 0 0] spin=[0x181915c8610/820143/p1@processus.rs:24 0x0/0/p-1@-:0 "
    "0x0/0/p-1@-:0 0x0/0/p-1@-:0] ipi=[2/0x8eff02/u0 820230/0x9d09d0/u0 820231/0x1/u0 820231/0x2/u0]"
)
# Ligne du temoin local (format complet, NMI comprise).
TEMOIN = (
    "[DIAG-EN-SOUFFRANCE] t=120000 demande_ms=90514 fil=1 cur=[-1,10,-1,-1] site=[0 0 0 0] "
    "spin=[0xc80880/118676/p1@banc_souffrance.rs:66<banc_souffrance.rs:31 0x0/0/p-1@-:0<-:0 "
    "0x0/0/p-1@-:0<-:0 0x0/0/p-1@-:0<-:0] ipi=[1/0x94a492/u0 3/0x9cd642/u0 0/0x0/u0 0/0x0/u0] "
    "nmi=[c0:- c1:0x9cd642/k/if1:0xc80880,0x9cd5a0,0xabdb20 c2:- c3:-]"
)


class Decoupe(unittest.TestCase):
    def test_format_complet(self):
        r = rg.decoupe(TEMOIN)
        self.assertEqual(r.demande_ms, 90514)
        self.assertEqual(r.cur, [-1, 10, -1, -1])
        a = r.spin[0]
        self.assertEqual((a.verrou, a.proprio), (0xc80880, 1))
        self.assertEqual(a.site, "banc_souffrance.rs:66")
        self.assertEqual(a.site_detenteur, "banc_souffrance.rs:31")
        self.assertIsNone(r.spin[1])
        n = r.nmi[1]
        self.assertEqual((n.rip, n.mode, n.si), (0x9cd642, "k", 1))
        self.assertEqual(n.pile, [0xc80880, 0x9cd5a0, 0xabdb20])
        self.assertIsNone(r.nmi[0])
        self.assertIsNone(r.nmi[2])

    def test_ancien_format_sans_site_de_prise_ni_nmi(self):
        r = rg.decoupe(ANCIENNE)
        self.assertEqual(r.spin[0].site, "processus.rs:24")
        self.assertEqual(r.spin[0].site_detenteur, "?")
        self.assertEqual(r.sites, [740, 212, 0, 0])
        self.assertEqual(r.ipi[1], (820230, 0x9d09d0, 0))
        self.assertEqual(r.nmi, [None] * 4)

    def test_hors_ligne(self):
        self.assertIsNone(rg.decoupe("[SCHED-RESUME] coeurs=4"))


class Rapport(unittest.TestCase):
    def test_sans_ligne_ne_conclut_rien_sur_le_noyau(self):
        out = rg.rapport("LADYBIRD_FUNCTIONAL_SMOKE fail raison=jalons manquants=7\n")
        self.assertEqual(len(out), 1)
        self.assertIn("verdict=indetermine", out[0])
        # Une absence de ligne ne disculpe pas le noyau.
        self.assertNotIn("continue d'ordonnancer", out[0])

    def test_fautif_et_taches(self):
        texte = (
            "[SMP-TASK] idx=9 tid=109 pid=13 rq=3 last=255 aff=0xf on=-1 kernel=true prio=Normale name=services-metrics\n"
            "[SMP-TASK] idx=50 tid=176 pid=25 rq=1 last=255 aff=0xf on=-1 kernel=false prio=Normale name=/bo-navigateur\n"
            + ANCIENNE.replace("t=840000", "t=810000") + "\n" + ANCIENNE + "\n"
        )
        out = rg.rapport(texte)
        self.assertIn("verdict=noyau_fige", out[0])
        self.assertTrue(any("progression irq_minuteur=vivante" in l for l in out))
        c0 = next(l for l in out if "coeur=0 " in l)
        self.assertIn("services-metrics", c0)
        self.assertIn("detenteur=coeur1", c0)
        self.assertIn("nmi=sans_reponse", c0)
        fautif = next(l for l in out if "RAPPORT_GEL fautif" in l)
        self.assertIn("coeur1", fautif)
        self.assertIn("pas de reponse NMI", fautif)

    def test_symbolisation_absente_sans_elf(self):
        self.assertEqual(rg.symbolise(None, [0x9cd642]), {})


QEMU = """[QEMU-CPU] ech=1 cpu=0 rip=0xab84cd cpl=0 if=0 hlt=0 rsp=0x10000201d80 pile=[0x814dbd,0x94a492]
[QEMU-CPU] ech=1 cpu=1 rip=0x9cd642 cpl=0 if=1 hlt=0 rsp=0x1803c41ff70 pile=[0xc80880,0x9cd5a0]
[QEMU-CPU] ech=2 cpu=0 rip=0x94a492 cpl=0 if=1 hlt=0 rsp=0x10000201ef0 pile=[0x91ac40]
[QEMU-CPU] ech=2 cpu=1 rip=0x9cd642 cpl=0 if=1 hlt=0 rsp=0x1803c41ff70 pile=[0xc80880,0x9cd5a0]
"""


class Qemu(unittest.TestCase):
    """BOUCHAUD_RELEVE_CPUS_QEMU_V1 : registres vus par QEMU."""

    def test_decoupe(self):
        c = rg.cpus_qemu(QEMU)
        self.assertEqual(len(c), 4)
        self.assertEqual((c[1].cpu, c[1].rip, c[1].cpl, c[1].si, c[1].hlt), (1, 0x9cd642, 0, 1, 0))
        self.assertEqual(c[1].pile, [0xc80880, 0x9cd5a0])

    def test_boucle_noyau_sans_ligne_de_souffrance(self):
        out = rg.rapport(QEMU)
        self.assertIn("verdict=pas_de_ligne_noyau", out[0])
        v = [l for l in out if "qemu_verdict=" in l]
        self.assertEqual(len(v), 1)
        self.assertIn("boucle_noyau coeur=1 rip=0x9cd642", v[0])

    def test_tous_en_hlt(self):
        texte = QEMU.replace("hlt=0", "hlt=1")
        self.assertTrue(any("qemu_verdict=vm_au_repos" in l for l in rg.rapport(texte)))

    def test_rip_utilisateur_immobile_n_est_pas_une_boucle_noyau(self):
        texte = QEMU.replace("cpu=1 rip=0x9cd642 cpl=0", "cpu=1 rip=0x9cd642 cpl=3")
        out = rg.rapport(texte)
        self.assertFalse(any("boucle_noyau" in l for l in out))
        self.assertTrue(any("qemu_verdict=vm_active" in l for l in out))

    def test_avec_ligne_de_souffrance_les_registres_qemu_s_ajoutent(self):
        out = rg.rapport(TEMOIN + "\n" + QEMU)
        self.assertIn("verdict=noyau_fige", out[0])
        self.assertTrue(any("qemu_verdict=boucle_noyau" in l for l in out))

    def test_detenteur_mobile_distinct_du_coeur_en_attente(self):
        from unittest.mock import patch

        text = TEMOIN + "\n" + QEMU.replace(
            "ech=2 cpu=1 rip=0x9cd642", "ech=2 cpu=1 rip=0x9cd647"
        ).replace("ech=2 cpu=0 rip=0x94a492", "ech=2 cpu=0 rip=0xab84cd")
        symbols = {
            0x9cd642: "os::banc_souffrance::detenteur (banc_souffrance.rs:39)",
            0x9cd647: "os::banc_souffrance::detenteur (banc_souffrance.rs:40)",
            0xab84cd: "spin (spin.rs:10)",
        }
        with patch.object(rg, "symbolise", return_value=symbols):
            report = rg.rapport(text, "temoin")
            self.assertTrue(any("qemu_verdict=boucle_noyau coeur=0" in line for line in report))
            self.assertIn("RAPPORT_GEL qemu_detenteur_confirme coeur=1 [os::banc_souffrance::detenteur]", report)
            interrupted = text.replace("ech=1 cpu=1 rip=0x9cd642", "ech=1 cpu=1 rip=0xab84cd")
            interrupted += "[QEMU-CPU] ech=3 cpu=1 rip=0x9cd647 cpl=0 if=1 hlt=0 rsp=0x1803c41ff70 pile=[]\n"
            self.assertTrue(any("qemu_detenteur_confirme coeur=1" in line for line in rg.rapport(interrupted, "temoin")))
            missing = interrupted.replace("ech=2 cpu=1", "ech=4 cpu=2")
            self.assertFalse(any("qemu_detenteur_confirme" in line for line in rg.rapport(missing, "temoin")))
            for invalid in (
                text.replace("cpu=1 rip=0x9cd647 cpl=0", "cpu=1 rip=0xab84cd cpl=0"),
                text.replace("cpu=1 rip=0x9cd647 cpl=0 if=1 hlt=0", "cpu=1 rip=0x9cd647 cpl=0 if=1 hlt=1"),
            ):
                self.assertFalse(any("qemu_detenteur_confirme" in line for line in rg.rapport(invalid, "temoin")))


if __name__ == "__main__":
    unittest.main()
