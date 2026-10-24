#!/usr/bin/env python3
"""Une interactive sous budget ne cede pas son coeur a un calcul normal
(BOUCHAUD_CESSION_PAR_CLASSE_V1).

`pick_next` tire la tete de la file sans regarder la tache courante. Avant ce
lot, au quantum (`preempt_from_irq`) comme a un point sur
(`preempt::safe_point`), une interactive tout juste elue cedait donc son coeur
au premier calcul normal en attente et perdait jusqu'a un quantum (endurance
f23a1247, SMP4, cycle 11).

Regles :

  1. la regle est PURE et testee sur l'hote : `cession` dans
     `scheduler/reveil.rs`, ses cas dans `tools/smp/test_reveil_cible.rs`
     (garde sous budget, cede a une interactive, cede au-dela du budget --
     l'anti-famine --, une normale cede toujours) ;
  2. la borne anti-famine est le budget d'activation (`BUDGET_ACTIVATION_NS`),
     pas une constante a part ;
  3. les DEUX chemins de preemption la consultent avant d'elire :
     `preempt_from_irq` (avant `pick_next`) et `safe_point` (avant
     `schedule()`) ;
  4. la politique par classe est celle du demarrage ; la politique historique
     n'est accessible que par la commande `ordo-cession` (A/B), et elle compte
     les inversions qu'elle laisse passer.

Fail-closed ; tests negatifs par mutation.
"""
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
FICHIERS = {
    "reveil": RACINE / "src/kernel/scheduler/reveil.rs",
    "test": RACINE / "tools/smp/test_reveil_cible.rs",
    "irq": RACINE / "src/kernel/process/thread/preemption.rs",
    "point": RACINE / "src/kernel/scheduler/preempt.rs",
    "ordo": RACINE / "src/kernel/process/thread/ordonnancement.rs",
    "shell": RACINE / "src/shell/mod.rs",
    "flux": RACINE / ".github/workflows/os-primitives.yml",
}
REGLE = ("    if courante == Classe::Interactive && attente_interactive == 0 "
         "&& consomme_ns < BUDGET_ACTIVATION_NS {\n        return Cession::Garde;\n    }")
DEFAUT = ("static CESSION_PAR_CLASSE: core::sync::atomic::AtomicBool = "
          "core::sync::atomic::AtomicBool::new(true);")
IRQ = ("    if courante_garde_le_coeur(cur, cpu_id) {\n        note_renonce_battement(cpu_id, RENONCE_GARDE);\n"
       "        termine_transition_ordonnanceur();\n        stall_site_clear();\n"
       "        return;\n    }\n\n    let Some(next) = pick_next(cur, cpu_id) else {")
POINT = ("    if crate::kernel::task::courante_garde_le_coeur_local() {\n"
         "        local.request_resched();\n        return false;\n    }")
INVERSION = "        CESSIONS_INVERSEES.fetch_add(1, Ordering::Relaxed);\n        false"
TESTS = ("une_interactive_sous_budget_garde_son_coeur_face_a_des_normales",
         "une_interactive_cede_a_une_interactive_en_attente",
         "une_interactive_qui_calcule_redevient_preemptible",
         "une_normale_cede_toujours_quand_quelqu_un_attend",
         "le_quantum_ne_coupe_plus_une_interactive_pour_un_calcul_normal",
         "la_cession_par_classe_est_la_politique_par_defaut")


def verifie(t: dict, sources_src: str) -> list:
    fautes = []
    if t["reveil"].count(REGLE) != 1:
        fautes.append("reveil.rs : la regle de cession n'est plus bornee par le budget d'activation.")
    if t["reveil"].count(DEFAUT) != 1:
        fautes.append("reveil.rs : la cession par classe n'est plus la politique du demarrage.")
    ecrit = [l for l in sources_src.splitlines() if "choisit_cession_par_classe(" in l
             and "pub fn choisit_cession_par_classe" not in l and "use " not in l]
    if len(ecrit) != 2 or any('=> choisit_cession_par_classe(' not in l for l in ecrit) \
            or "CESSION_PAR_CLASSE.store(" in t["reveil"].replace(
                "CESSION_PAR_CLASSE.store(oui, core::sync::atomic::Ordering::Relaxed);", "", 1):
        fautes.append("la politique de cession est basculee ailleurs que par ordo-cession.")
    if '"ordo-cession" => ordo_cession(argc, &argv),' not in t["shell"]:
        fautes.append("shell : la commande ordo-cession n'est plus branchee.")
    if t["irq"].count(IRQ) != 1:
        fautes.append("preempt_from_irq : la regle n'est plus consultee avant pick_next.")
    point = t["point"]
    ordre = [point.find(jalon) for jalon in (
        "local.clear_resched();", POINT, "REQUEST_NS[index].swap", "let switched = crate::kernel::task::schedule();")]
    if point.count(POINT) != 1 or min(ordre) < 0 or ordre != sorted(ordre):
        fautes.append("safe_point : la cession refusee doit conserver sa demande avant toute consommation ou election.")
    if t["ordo"].count(INVERSION) != 1:
        fautes.append("ordonnancement.rs : la politique historique ne compte plus ses inversions.")
    if "cession(classe, consomme, interactives, normales)" not in t["ordo"] \
            or "tache.budget_reveil_ns.charge().saturating_add(tranche)" not in t["ordo"]:
        fautes.append("ordonnancement.rs : la consommation de l'activation n'est plus celle du budget.")
    for nom in TESTS:
        if f"fn {nom}()" not in t["test"]:
            fautes.append(f"test hote absent : {nom}.")
    if 'AB_SORTIE=$PWD/ab-cession-kvm-smp4 BO_QEMU_KVM=1 tools/ci/run_ab_cession.sh target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin 6 4' not in t["flux"]:
        fautes.append("CI : l'A/B KVM n'est plus mesure dans os-primitives.")
    return fautes


def sources() -> str:
    return "\n".join(p.read_text(encoding="utf-8") for p in sorted((RACINE / "src").rglob("*.rs")))


def negatifs(t: dict, src: str) -> list:
    mutations = [
        ("A/B KVM retire de la CI", "flux", 'AB_SORTIE=$PWD/ab-cession-kvm-smp4 BO_QEMU_KVM=1 tools/ci/run_ab_cession.sh target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin 6 4', ""),
        ("sans borne de budget", "reveil", " && consomme_ns < BUDGET_ACTIVATION_NS {", " {"),
        ("garde face a une interactive", "reveil", " && attente_interactive == 0 && consomme_ns", " && consomme_ns"),
        ("historique par defaut", "reveil", DEFAUT, DEFAUT.replace("new(true)", "new(false)")),
        ("quantum sans regle", "irq", "    if courante_garde_le_coeur(cur, cpu_id) {", "    if false {"),
        ("point sur sans regle", "point", "    if crate::kernel::task::courante_garde_le_coeur_local() {", "    if false {"),
        ("demande perdue au refus", "point", "        local.request_resched();\n        return false;", "        return false;"),
        ("horodatage consomme avant decision", "point", "    local.clear_resched();", "    let _ = REQUEST_NS[index].swap(0, Ordering::AcqRel);\n    local.clear_resched();"),
        ("inversions non comptees", "ordo", INVERSION, "        false"),
        ("tranche ouverte oubliee", "ordo", "tache.budget_reveil_ns.charge().saturating_add(tranche)",
         "tache.budget_reveil_ns.charge()"),
        ("anti-famine non teste", "test", "fn une_interactive_qui_calcule_redevient_preemptible()", "fn x()"),
        ("commande debranchee", "shell", '"ordo-cession" => ordo_cession(argc, &argv),', ""),
    ]
    echecs = []
    for nom, cle, avant, apres in mutations:
        if avant not in t[cle]:
            echecs.append(f"mutation inapplicable : {nom}")
            continue
        m = dict(t)
        m[cle] = t[cle].replace(avant, apres, 1)
        s = src.replace(t[cle], m[cle]) if cle != "test" else src
        if not verifie(m, s):
            echecs.append(f"mutation non detectee : {nom}")
    s = src + "\nfn x() { crate::kernel::scheduler::reveil::choisit_cession_par_classe(false); }\n"
    if not verifie(t, s):
        echecs.append("mutation non detectee : bascule hors du shell")
    return echecs


def main() -> int:
    t = {k: p.read_text(encoding="utf-8") for k, p in FICHIERS.items()}
    src = sources()
    fautes = verifie(t, src) + negatifs(t, src)
    if fautes:
        for f in fautes:
            print(f"verifie-cession-par-classe: {f}")
        return 1
    print("verifie-cession-par-classe: OK (regle pure bornee par le budget, quantum et point sur, "
          "classe par defaut, inversions comptees en historique ; 13 negatifs)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
