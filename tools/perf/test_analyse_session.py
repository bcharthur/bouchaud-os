#!/usr/bin/env python3
"""BOUCHAUD_PERF_SESSION_V1 -- l'analyseur lit les formats EXACTS du noyau."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import analyse_session as a  # noqa: E402

PREFIXE = "\x1b[90m[\x1b[36m22:36:58\x1b[90m][ 54%:  1%:  5%][FPS: --] \x1b[0m"
JOURNAL = "\n".join([
    PREFIXE + "BOUCHAUD_TSC_EARLY_CALIBRATION_OK hz=4192000000 source=pit2",
    PREFIXE + "BOUCHAUD_TSC_CONTROLE hz=4192000000 source=pit2 hz_selon_ticks=4100000000 rapport_pour_mille=978 cycles_par_ms=4192000",
    PREFIXE + "SMP_NG2_SCHEDULER online=2 mode=thread-load-balance+work-steal quantum=4ms",
    "[PERF-PROC] t=1000 pid=20 image=WebContent rss_kio=1000 vss_kio=9000 taches=9 fautes=10 total_us=50 pire_us=9 "
    "zero=8/40us fichier=1/5us partage=0/0us copie=1/5us attente=0/0us echec=0 user_ms=100 sys_ms=50",
    "[PERF-PROC] t=2000 pid=20 image=WebContent rss_kio=1415000 vss_kio=9000 taches=9 fautes=1188795 total_us=9000000 "
    "pire_us=900 zero=1136932/7000000us fichier=1/5us partage=0/0us copie=1/5us attente=0/0us echec=0 user_ms=9000 sys_ms=21000",
    "[PERF-PROC] t=2000 pid=31 image=WebContent rss_kio=200000 vss_kio=9000 taches=9 fautes=5 total_us=1 "
    "pire_us=1 zero=2/1us fichier=1/0us partage=0/0us copie=1/0us attente=0/0us echec=0 user_ms=10 sys_ms=5",
    "[PROC-SAMPLE] v=1 t_ns=5 pid=20 name=/usr/libexec/ladybird/WebContent cpu_pct=97 cpu_map=[60,37] ctx_delta=4 mig_delta=0 runnable_threads=1 threads=9 rss=1 vss=1",
    "[PROC-SAMPLE] v=1 t_ns=6 pid=20 name=/usr/libexec/ladybird/WebContent cpu_pct=50 cpu_map=[50,0] ctx_delta=4 mig_delta=0 runnable_threads=1 threads=9 rss=1 vss=1",
    "[PERF-RIP] t=5000 pid=20 tid=99 image=/usr/libexec/ladybird/WebContent user=30 noyau=70 base=0x400000400000 top=- topn=0x88f30f:6,0x88817a:5",
    "[PERF-RIP] t=10000 pid=20 tid=99 image=/usr/libexec/ladybird/WebContent user=10 noyau=20 base=0x400000400000 top=- topn=0x88f30f:4",
    "[SMP-IPI] t=60000 c0=100/0ms/0x1/u0 c1=200/0ms/0x1/u0 c2=0/0ms/0x0/u0 c3=0/0ms/0x0/u0",
    "[PROC-STAT] capacite_ms=1 occupe_ms=1 depassements=0 pire_depassement_ms=0 compta_relues=1 replis_apic=1 ticks_ms=140000 mono_ms=150000 index_coeur=rdpid",
    "BOUCHAUD_CONSOLE_BILAN active=1 repli_securite=0 lignes=115 emises=114 pleines=0 replis_differes=0 vidanges_aidees=0 reentrees=0 differees_pendant_vidange=0 reveils=2 jeton_occupe=0 ligne_pire_us=1 morceau_pire_us=1 retard_pire_us=1 ajout_pire_us=1 occupation_pire=1 synchrones=7 synchrone_pire_us=900 verrou_abandons=0",
    "[SYSCALL-TOP] window_ns=1000000000 appels=500 distincts=2 cumul=9 madvise=300/eagain=0 clock_gettime=200/eagain=0",
    "[SYSCALL-TOP] window_ns=1000000000 appels=50 distincts=1 cumul=9 clock_gettime=50/eagain=0",
    "[GPU-COMPOSITION] disponible=0 periode_ms=1000 trames_bureau=76 trames_clients=3 trames_gpu=0 blits=1",
    "[PERSIST-SLOW] n=3 total_ns=1500000000 lock_ns=1",
    "HTMLParserEndState timeout", "HTMLParserEndState timeout",
])


class Analyse(unittest.TestCase):
    def test_lit_les_formats_du_noyau(self):
        s = a.analyse(a.ANSI.sub("", JOURNAL))
        self.assertEqual(s["horloge"]["verdict"], "coherente")
        self.assertEqual(s["coeurs_en_ligne"], 2)
        wc = s["processus"]["WebContent"]
        # Le DERNIER releve de chaque pid, puis la somme des pid de l'image.
        self.assertEqual(wc["pids"], 2)
        self.assertEqual(wc["fautes"], 1188800)
        self.assertEqual(wc["zero"], 1136934)
        self.assertEqual(wc["zero_us"], 7000001)
        self.assertEqual(wc["sys_ms"], 21005)
        self.assertEqual(wc["rss_kio_max"], 1415000)
        self.assertEqual(s["appels"]["madvise"], 300)
        self.assertEqual(s["appels"]["clock_gettime"], 250)
        self.assertIs(s["gpu_disponible"], False)
        self.assertEqual(s["persist_lent"]["n"], 1)
        self.assertEqual(s["parseur_html_timeouts"], 2)
        self.assertEqual(s["cpu_pct_pic"]["WebContent"], 97)

    def test_pic_de_rss_sur_tous_les_releves(self):
        j = ("[PERF-PROC] t=1 pid=9 image=WebContent rss_kio=900000 vss_kio=1 taches=1 fautes=1 total_us=1 pire_us=1 "
             "zero=0/0us fichier=0/0us partage=0/0us copie=0/0us attente=0/0us echec=0 user_ms=1 sys_ms=1\n"
             "[PERF-PROC] t=2 pid=9 image=WebContent rss_kio=100000 vss_kio=1 taches=1 fautes=2 total_us=1 pire_us=1 "
             "zero=0/0us fichier=0/0us partage=0/0us copie=0/0us attente=0/0us echec=0 user_ms=2 sys_ms=2")
        wc = a.analyse(j)["processus"]["WebContent"]
        self.assertEqual(wc["rss_kio_max"], 900000)  # le pic, pas le dernier releve
        self.assertEqual(wc["fautes"], 2)            # les cumuls, du dernier

    def test_coeurs_rip_ipi_journal(self):
        s = a.analyse(a.ANSI.sub("", JOURNAL), duree_hote_s=160.0)
        # PROC-SAMPLE porte un CHEMIN : la cle est le nom de base.
        self.assertEqual(s["cpu_pct_moyen"]["WebContent"], 74)
        self.assertEqual(s["repartition_coeurs_pct"]["WebContent"], [55, 18])
        self.assertEqual(s["echantillons_multicoeur"]["WebContent"], 1)
        self.assertEqual(s["rip_user_noyau"]["WebContent"], [40, 90])
        self.assertEqual(s["rip_noyau_chaudes"]["0x88f30f"], 10)
        self.assertEqual((s["ipi_total"], s["ipi_par_s"]), (300, 5))
        self.assertEqual(s["console"]["synchrones"], 7)
        self.assertEqual(s["coherence"]["ticks_sur_mono_pour_mille"], 933)
        self.assertEqual(s["coherence"]["invite_sur_hote_pour_mille"], 937)
        self.assertEqual(a.verdict_final(s)[0], "coherente")

    def test_controle_hote_prime_et_refuse_une_derive(self):
        texte = "[PROC-STAT] capacite_ms=1 occupe_ms=1 depassements=0 ticks_ms=500 mono_ms=600000 index_coeur=x"
        # Sans TSC_CONTROLE, l'horloge seule est « inconnue » ; l'hote tranche.
        self.assertEqual(a.verdict_final(a.analyse(texte, 610.0))[0], "coherente")
        self.assertEqual(a.verdict_final(a.analyse(texte, 150.0))[0], "incoherente")
        self.assertEqual(a.verdict_final(a.analyse(texte))[0], "inconnue")

    def test_horloge_incoherente_refusee(self):
        h = a.horloge("BOUCHAUD_TSC_CONTROLE hz=1000000000 source=pit2 hz_selon_ticks=4000000000 rapport_pour_mille=4000 cycles_par_ms=1")
        self.assertEqual(h["verdict"], "incoherente")

    def test_sans_tsc_l_horloge_est_inconnue(self):
        self.assertEqual(a.horloge("BOUCHAUD_HPET_MONOTONIC_READY base=0xfed00000 period_fs=10000000")["verdict"], "inconnue")

    def test_mesure_absente_reste_absente(self):
        s = a.analyse("")
        self.assertIsNone(s["coeurs_en_ligne"])
        self.assertIsNone(s["gpu_disponible"])
        self.assertEqual(s["processus"], {})


if __name__ == "__main__":
    unittest.main()
