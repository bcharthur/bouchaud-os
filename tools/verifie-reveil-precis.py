#!/usr/bin/env python3
"""Le reveil a l'echeance exacte au repos, borne a ce qui est prouve
(BOUCHAUD_REVEIL_PRECIS_V1, P13+).

Regles :

  1. ACTIF par defaut depuis l'A/B KVM CONVAINCANT de b6c99032 (run
     38732049308, cite par la statique) : `REVEIL_PRECIS` est un
     `AtomicBool::new(true)`, ecrit seulement par `veut_reveil_precis`,
     appele seulement par la commande shell `reveil-precis`. Le domaine
     prouve est le TSC-deadline SOUS HYPERVISEUR : le mode periodique (TCG)
     et le materiel nu sont deux experiences a part, chacune OFF, exigee
     avant tout armement dans son domaine, basculee par le shell seul ;
     l'hyperviseur est lu une fois a l'armement du minuteur, jamais dans la
     boucle d'inactivite ;
  2. l'armement se fait juste avant `sti; hlt` (interruptions encore
     coupees) et il est retire juste apres : `commit_scheduler_idle` appelle
     `arme_reveil_precis_avant_hlt` puis `retire_reveil_precis_apres_hlt`
     autour de son `sti; hlt`, et nulle part ailleurs (l'attente generique
     `wait_for_interrupt` n'arme rien) ;
  3. le minuteur ne garde jamais un compte de sommeil quand il sert le
     quantum : `arm_local_scheduler_timer` remet le compte du quantum en mode
     periodique et efface `ARME_PRECIS` en mode TSC-deadline ;
  4. la decision reste pure et testee sur l'hote : `reveil_precis.rs` ne
     touche a aucun registre, et `tools/smp/test_reveil_precis.rs` couvre
     l'echeance due, la politique desactivee et l'arrondi superieur du compte ;
  5. l'A/B a des criteres declares (`analyse_ab_reveil_precis.py`) : quorum
     de 4 tours, ordre AB/BA, armements nuls sous `off`.

Fail-closed ; tests negatifs par mutation.
"""
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
FICHIERS = {
    "smp": RACINE / "src/arch/x86_64/smp.rs",
    "idle": RACINE / "src/arch/x86_64/cpu/idle/scheduler.rs",
    "shell": RACINE / "src/shell/mod.rs",
    "flux": RACINE / ".github/workflows/os-primitives.yml",
    "pur": RACINE / "src/kernel/time/reveil_precis.rs",
    "test": RACINE / "tools/smp/test_reveil_precis.rs",
    "analyse": RACINE / "tools/ci/analyse_ab_reveil_precis.py",
}
INTERRUPTEUR = "static REVEIL_PRECIS: AtomicBool = AtomicBool::new(true);"
PREUVE = "CONVAINCANT de b6c99032 (run 38732049308"
MATERIEL_OFF = "static REVEIL_PRECIS_MATERIEL: AtomicBool = AtomicBool::new(false);"
MATERIEL_EXIGE = ("    if !SOUS_HYPERVISEUR.load(Ordering::Relaxed) && !REVEIL_PRECIS_MATERIEL.load(Ordering::Relaxed) {\n"
                  "        return;\n    }\n    let cible = crate::kernel::reveil_precis::decide(")
PERIODIQUE_OFF = "static REVEIL_PRECIS_PERIODIQUE: AtomicBool = AtomicBool::new(false);"
PERIODIQUE_EXIGE = ("    if MODE_TIMER_LOCAL.load(Ordering::Acquire) == MODE_LAPIC_PERIODIQUE {\n"
                    "        if !REVEIL_PRECIS_PERIODIQUE.load(Ordering::Relaxed) {\n            return;\n        }")
HLT = 'unsafe { asm!("sti; hlt", options(nostack)); }'
AUTOUR_HLT = ("    smp::arme_reveil_precis_avant_hlt();\n"
              "    " + HLT + "\n"
              "    smp::retire_reveil_precis_apres_hlt();\n")
PERIODIQUE = ("            if cpu < MAX_CPUS && ARME_PRECIS[cpu].swap(false, Ordering::Relaxed) {\n"
              "                let compte = LAPIC_COMPTE_QUANTUM.load(Ordering::Acquire);")
TSC = "                    ARME_PRECIS[cpu].store(false, Ordering::Relaxed);"
TESTS = ("une_echeance_due_ou_trop_proche_n_est_pas_armee", "la_politique_desactivee_n_arme_jamais",
         "periodique_le_compte_ne_tire_jamais_avant_l_echeance", "un_tsc_cible_deja_passe_n_est_pas_arme")


def corps(texte: str, signature: str) -> str:
    i = texte.find(signature)
    if i < 0:
        return ""
    fin = texte.find("\n}\n", i)
    return texte[i:fin] if fin > 0 else ""


def verifie(t: dict, sources_src: str) -> list:
    fautes = []
    if t["smp"].count(INTERRUPTEUR) != 1 or PREUVE not in t["smp"]:
        fautes.append("smp.rs : le reveil precis n'est plus actif par defaut, ou la preuve KVM (run 38732049308) n'est plus citee.")
    if t["smp"].count(MATERIEL_OFF) != 1 or MATERIEL_EXIGE not in corps(t["smp"], "pub fn arme_reveil_precis_avant_hlt("):
        fautes.append("smp.rs : le TSC-deadline sur materiel nu n'est plus une experience a part, desactivee.")
    if "__cpuid" in corps(t["smp"], "pub fn arme_reveil_precis_avant_hlt(") \
            or "SOUS_HYPERVISEUR.store(" not in corps(t["smp"], "fn init_local_scheduler_timer("):
        fautes.append("smp.rs : l'hyperviseur n'est plus lu une fois a l'armement du minuteur (CPUID dans la boucle d'inactivite ?).")
    mat = [l for l in sources_src.splitlines() if "veut_reveil_precis_materiel(" in l and "pub fn " not in l]
    if len(mat) != 1 or "argv[2] == \"on\"" not in mat[0]:
        fautes.append("le materiel nu est bascule ailleurs que par `reveil-precis materiel`.")
    # Le mode periodique (TCG) a echoue a son A/B local : experience a part,
    # OFF, exigee avant tout armement periodique, basculee par le shell seul.
    if t["smp"].count(PERIODIQUE_OFF) != 1 or PERIODIQUE_EXIGE not in corps(t["smp"], "pub fn arme_reveil_precis_avant_hlt("):
        fautes.append("smp.rs : le mode periodique du reveil precis n'est plus une experience a part, desactivee.")
    per = [l for l in sources_src.splitlines() if "veut_reveil_precis_periodique(" in l and "pub fn " not in l]
    if len(per) != 1 or "argv[2] == \"on\"" not in per[0]:
        fautes.append("le mode periodique est bascule ailleurs que par `reveil-precis periodique`.")
    ecritures = t["smp"].count("REVEIL_PRECIS.swap(") + t["smp"].count("REVEIL_PRECIS.store(")
    if ecritures != 1 or "REVEIL_PRECIS.swap(oui, Ordering::Relaxed)" not in corps(t["smp"], "pub fn veut_reveil_precis("):
        fautes.append("smp.rs : la politique est ecrite ailleurs que par veut_reveil_precis.")
    appels = [l for l in sources_src.splitlines() if "veut_reveil_precis(" in l and "pub fn veut_reveil_precis" not in l]
    if len(appels) != 2 or any("crate::arch::x86_64::smp::veut_reveil_precis(" not in l for l in appels) \
            or corps(t["shell"], "fn reveil_precis(").count("veut_reveil_precis(") != 2:
        fautes.append("la politique est basculee ailleurs que par la commande reveil-precis.")
    if '"reveil-precis" => reveil_precis(argc, &argv),' not in t["shell"]:
        fautes.append("shell : la commande reveil-precis n'est plus branchee.")
    sommeil = corps(t["idle"], "pub fn commit_scheduler_idle(")
    if sommeil.count(HLT) != 1 or sommeil.count(AUTOUR_HLT) != 1:
        fautes.append("idle : l'armement precis n'encadre plus le `sti; hlt` de commit_scheduler_idle.")
    for f in ("arme_reveil_precis_avant_hlt(", "retire_reveil_precis_apres_hlt("):
        hors = [l for l in sources_src.splitlines() if f in l and "pub fn " not in l]
        if len(hors) != 1:
            fautes.append(f"{f.rstrip('(')} est appele hors de commit_scheduler_idle ({len(hors)} appels).")
    armer = corps(t["smp"], "pub fn arm_local_scheduler_timer(")
    if PERIODIQUE not in armer:
        fautes.append("smp.rs : le quantum periodique n'est plus restaure apres un compte de sommeil.")
    if TSC not in armer:
        fautes.append("smp.rs : le quantum TSC-deadline n'efface plus l'armement precis.")
    if any(m in t["pur"] for m in ("asm!", "write_msr", "unsafe", "static ")):
        fautes.append("reveil_precis.rs : la decision n'est plus pure.")
    for nom in TESTS:
        if f"fn {nom}()" not in t["test"]:
            fautes.append(f"test hote absent : {nom}.")
    for jalon in ("QUORUM = 4", 'voulu = ["off", "on"] if t % 2 else ["on", "off"]',
                  'b["v"] == "off" and delta != 0'):
        if jalon not in t["analyse"]:
            fautes.append(f"analyse A/B : critere retire ({jalon}).")
    if 'AB_SORTIE=$PWD/ab-reveil-precis-kvm-smp1 BO_QEMU_KVM=1 tools/ci/run_ab_reveil_precis.sh target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin 4 1' not in t["flux"]:
        fautes.append("CI : l'A/B KVM n'est plus mesure dans os-primitives.")
    return fautes


def sources() -> str:
    return "\n".join(p.read_text(encoding="utf-8") for p in sorted((RACINE / "src").rglob("*.rs")))


def negatifs(t: dict, src: str) -> list:
    mutations = [
        ("periodique actif par defaut", "smp", PERIODIQUE_OFF, PERIODIQUE_OFF.replace("false", "true")),
        ("periodique sans autorisation", "smp", PERIODIQUE_EXIGE,
         "    if MODE_TIMER_LOCAL.load(Ordering::Acquire) == MODE_LAPIC_PERIODIQUE {"),
        ("A/B KVM retire de la CI", "flux", 'AB_SORTIE=$PWD/ab-reveil-precis-kvm-smp1 BO_QEMU_KVM=1 tools/ci/run_ab_reveil_precis.sh target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin 4 1', ""),
        ("reveil precis desactive par defaut", "smp", INTERRUPTEUR, INTERRUPTEUR.replace("true", "false")),
        ("preuve plus citee", "smp", PREUVE, "CONVAINCANT"),
        ("materiel nu actif par defaut", "smp", MATERIEL_OFF, MATERIEL_OFF.replace("false", "true")),
        ("materiel nu sans autorisation", "smp", MATERIEL_EXIGE, "    let cible = crate::kernel::reveil_precis::decide("),
        ("armement sorti du hlt", "idle", "    smp::arme_reveil_precis_avant_hlt();\n", ""),
        ("retrait oublie", "idle", "    smp::retire_reveil_precis_apres_hlt();\n", ""),
        ("quantum periodique non restaure", "smp", PERIODIQUE, PERIODIQUE.replace("swap(false", "load(")),
        ("armement TSC jamais efface", "smp", TSC, ""),
        ("decision impure", "pur", "pub fn decide(", "unsafe fn decide("),
        ("test d'echeance due retire", "test", "fn une_echeance_due_ou_trop_proche_n_est_pas_armee()", "fn x()"),
        ("quorum abaisse", "analyse", "QUORUM = 4", "QUORUM = 1"),
        ("commande debranchee", "shell", '"reveil-precis" => reveil_precis(argc, &argv),', ""),
    ]
    echecs = []
    for nom, cle, avant, apres in mutations:
        m = dict(t)
        if avant not in m[cle]:
            echecs.append(f"mutation inapplicable : {nom}")
            continue
        m[cle] = m[cle].replace(avant, apres, 1)
        s = src.replace(t[cle], m[cle]) if cle in ("smp", "idle", "shell", "pur") else src
        if not verifie(m, s):
            echecs.append(f"mutation non detectee : {nom}")
    # Une bascule ajoutee ailleurs.
    s = src + "\nfn x() { crate::arch::x86_64::smp::veut_reveil_precis(true); }\n"
    if not verifie(t, s):
        echecs.append("mutation non detectee : bascule hors du shell")
    return echecs


def main() -> int:
    t = {k: p.read_text(encoding="utf-8") for k, p in FICHIERS.items()}
    src = sources()
    fautes = verifie(t, src)
    fautes += negatifs(t, src)
    if fautes:
        for f in fautes:
            print(f"verifie-reveil-precis: {f}")
        return 1
    print("verifie-reveil-precis: OK (politique on par defaut (A/B KVM 38732049308), TSC-deadline sous hyperviseur seul, armement autour du hlt, quantum restaure, "
          "decision pure testee, periodique et materiel nu a part, A/B a criteres declares ; 17 negatifs)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
