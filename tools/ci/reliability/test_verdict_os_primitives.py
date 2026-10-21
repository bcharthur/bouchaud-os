"""BOUCHAUD_VERDICT_OS_PRIMITIVES_V1 : le verdict d'os-primitives sur des
journaux canoniques. Run 39503091649 : rouge sur une ligne PRESENTE (SIGPIPE
sous pipefail). Chaque negatif retire ou abime UNE preuve et doit echouer."""
import subprocess
import tempfile
import unittest
from pathlib import Path

RACINE = Path(__file__).resolve().parents[3]
VERDICT = RACINE / "tools/ci/verdict_os_primitives.sh"
PREFIXE = "\x1b[90m[\x1b[36m23:51:05\x1b[90m][ 62%: 10%:  3%][FPS: --] \x1b[0m"
MARQUEURS = ["VERROUS_POSIX_OK", "EXEC_FD_OK", "WAL_PROBE_OK echecs=0", "DISQUE_PROBE_OK echecs=0", "NOM_LONG_OK echecs=0",
             "SENDFILE_OK echecs=0", "RENOMMAGE_OK cas=24", "FAUTE_NONCANONIQUE_OK", "FAUTE_MULTIFIL_OK essais=16",
             "NOEUDS_CHURN_OK cycles=8000", "SIGCHLD_MULTIFIL_OK", "SIGMASQUE_FIL_OK", "COMPTA_STRESS_OK", "MTIME_STABLE_OK",
             "PERSIST_COURSE_OK", "EOF_FERMETURE_OK cas=6 tours=2", "TUBE_ATOMICITE_OK tailles=3 ecrivains=4 tours=300 modes=2",
             "STATUT_SORTIE_OK cas=11", "SESSION_PERE_SORT fils=4"]
DETAIL_UD2 = ("[kernel] PROCESS_FAULT_DETAIL pid=34 tid=130 cpu=2 fs_base=0x400000409698 "
              "fs_base_range=0x400000409698 insn=0f0b66662e0f1f8400000000000f1f00")
DETAIL_AUTRE = ("[kernel] PROCESS_FAULT_DETAIL pid=31 tid=127 cpu=3 fs_base=0x400000409698 "
                "fs_base_range=0x400000409698 insn=488b3031c0488d3d7d5e0000e8a60900")
STAT = ("[PROC-STAT] capacite_ms=1 occupe_ms=1 depassements=0 pire_depassement_ms=0 compta_relues=3 replis_apic=2 "
        "ticks_ms=1 mono_ms=1 index_coeur=gs tranches_gonflees=0 pire_gonflement_us=0 chevauchements=0 pire_chevauchement_us=0")


def journal(remplissage=0, sans=(), detail=DETAIL_UD2, statut="0", extra=(), stat=STAT):
    lignes = ["BOUCHAUD_TSC_EARLY_CALIBRATION_OK hz=2100000000 source=cpuid15"]
    # Le remplissage precede : un lecteur qui s'arreterait au premier match
    # laisserait son producteur ecrire encore des megaoctets (SIGPIPE).
    lignes += [f"[kernel] bruit {i} " + "x" * 120 for i in range(remplissage)]
    lignes += [DETAIL_AUTRE, detail]
    lignes += [m for m in MARQUEURS if m not in sans]
    lignes += [stat]
    lignes += ["+ echo SESSION_INVITE_REVENUE"] + ([] if "SESSION_INVITE_REVENUE" in sans else ["SESSION_INVITE_REVENUE"])
    lignes += ["+ echo PRIMITIVES_FIN"] + ([] if "PRIMITIVES_FIN" in sans else ["PRIMITIVES_FIN"])
    lignes += list(extra)
    lignes += [f"=== AUTORUN FIN === statut={statut}"]
    lignes += [f"[kernel] apres {i} " + "y" * 120 for i in range(remplissage)]
    return "".join(PREFIXE + l + "\n" for l in lignes)


def verdict(texte):
    with tempfile.TemporaryDirectory() as d:
        p = Path(d) / "serie.log"
        p.write_text(texte, encoding="utf-8")
        r = subprocess.run(["bash", str(VERDICT), str(p)], capture_output=True, text=True, timeout=120)
        return r.returncode, r.stdout, r.stderr


class Verdict(unittest.TestCase):
    def test_journal_complet(self):
        code, out, err = verdict(journal())
        self.assertEqual(code, 0, err)
        self.assertIn("OS_PRIMITIVES_VERDICT_OK", out)

    def test_gros_journal_sans_faux_rouge_sigpipe(self):
        # ~4 Mo : `sed | grep -q` echouait ici sous pipefail.
        code, out, err = verdict(journal(remplissage=30000))
        self.assertEqual(code, 0, err[-2000:])
        self.assertNotIn("Broken pipe", err)

    def test_temoin_absent(self):
        code, _, err = verdict(journal(detail="[kernel] rien"))
        self.assertEqual(code, 1)
        self.assertIn("FAUTE_DETAIL_ECHEC", err)

    def test_temoin_incorrect(self):
        code, _, err = verdict(journal(detail=DETAIL_UD2.replace("insn=0f0b", "insn=0b66")))
        self.assertEqual(code, 1)
        self.assertIn("FAUTE_DETAIL_ECHEC", err)

    def test_autorun_en_erreur(self):
        code, _, err = verdict(journal(statut="139"))
        self.assertEqual(code, 1)
        self.assertIn("AUTORUN_ECHEC statut=139", err)

    def test_autorun_absent(self):
        code, _, err = verdict(journal().replace("=== AUTORUN FIN === statut=0", "fin coupee"))
        self.assertEqual(code, 1)
        self.assertIn("AUTORUN_ECHEC statut=absent", err)

    def test_marqueur_de_sonde_absent(self):
        code, _, err = verdict(journal(sans=("COMPTA_STRESS_OK",)))
        self.assertEqual(code, 1)
        self.assertIn("MARQUEUR_ABSENT COMPTA_STRESS_OK", err)

    def test_trace_du_shell_ne_vaut_pas_marqueur(self):
        code, _, err = verdict(journal(sans=("PRIMITIVES_FIN",)))
        self.assertEqual(code, 1)
        self.assertIn("MARQUEUR_ABSENT PRIMITIVES_FIN", err)

    def test_panique(self):
        code, _, err = verdict(journal(extra=["KERNEL PANIC cpu=1 courant.rs:276:5"]))
        self.assertEqual(code, 1)
        self.assertIn("panique noyau", err)

    def test_ata_en_echec(self):
        code, _, err = verdict(journal(extra=["ata: lecture lba=12 erreur"]))
        self.assertEqual(code, 1)

    def test_invariant_de_compta_viole(self):
        code, _, err = verdict(journal(stat=STAT.replace("chevauchements=0", "chevauchements=3")))
        self.assertEqual(code, 1)
        self.assertIn("INVARIANT_COMPTA_ECHEC", err)

    def test_journal_vide(self):
        code, _, err = verdict("")
        self.assertEqual(code, 1)


if __name__ == "__main__":
    unittest.main()
