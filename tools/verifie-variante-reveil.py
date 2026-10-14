#!/usr/bin/env python3
"""La variante experimentale d'IPI au reveil reste une experience
(BOUCHAUD_REVEIL_IPI_INTERACTIF_EXPERIMENTAL).

Regles :

  1. desactivee au demarrage : l'interrupteur est un `AtomicBool::new(false)`,
     et seule la commande shell `ordo-variante` le bascule ;
  2. sous `Defaut`, la decision est EXACTEMENT `decide` -- et `decide` garde
     sa politique : une interactive non sensible ne coupe personne
     (`une_tache_ordinaire_ne_coupe_personne`) ;
  3. la variante ne pose jamais de demande ciblee : son bras dans
     `publish_ready` appelle `request_cpu`, jamais `demande_ciblee` (aucun
     domaine de preemption des fils noyau) ;
  4. la CI mesure l'A/B sous KVM.

Fail-closed ; tests negatifs par mutation.
"""
import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
FICHIERS = {
    "reveil": RACINE / "src/kernel/scheduler/reveil.rs",
    "creation": RACINE / "src/kernel/process/thread/creation.rs",
    "shell": RACINE / "src/shell/mod.rs",
    "test": RACINE / "tools/smp/test_reveil_cible.rs",
    "flux": RACINE / ".github/workflows/os-primitives.yml",
}
INTERRUPTEUR = ("static VARIANTE_IPI_INTERACTIVE: core::sync::atomic::AtomicBool = "
                "core::sync::atomic::AtomicBool::new(false);")
DEFAUT_INCHANGE = ("    let d = decide(cible, reveille);\n"
                   "    if variante != Variante::IpiInteractive || d != Decision::DemandeDifferee {\n"
                   "        return d;\n    }")


def bras(creation: str) -> str:
    m = re.search(r"ReveilDecision::DemandeAvecIpi => \{(.*?)\n        \}", creation, re.S)
    return m.group(1) if m else ""


def verifie(t: dict, sources_src: str) -> list:
    fautes = []
    if t["reveil"].count(INTERRUPTEUR) != 1:
        fautes.append("reveil.rs : la variante n'est plus desactivee au demarrage.")
    if "VARIANTE_IPI_INTERACTIVE.store(" in t["reveil"].replace(
            "VARIANTE_IPI_INTERACTIVE.store(v == Variante::IpiInteractive", "", 1):
        fautes.append("reveil.rs : l'interrupteur est ecrit hors de choisit_variante.")
    appels = [l for l in sources_src.splitlines() if "choisit_variante(" in l and "pub fn choisit_variante" not in l]
    if not appels or any("choisit_variante(Variante::" not in l for l in appels) \
            or sources_src.count("choisit_variante(") != 3:
        fautes.append("la variante est basculee ailleurs que par la commande ordo-variante.")
    if '"ordo-variante" => ordo_variante(argc, &argv),' not in t["shell"]:
        fautes.append("shell : la commande ordo-variante a disparu.")
    if DEFAUT_INCHANGE not in t["reveil"]:
        fautes.append("reveil.rs : sous Defaut, decide_selon n'est plus exactement decide.")
    if not re.search(r"fn une_tache_ordinaire_ne_coupe_personne\(\) \{\s*let cible = coeur_occupe\(Classe::Normale, "
                     r"false, 50_000_000\);\s*assert_eq!\(decide\(&cible, &ordinaire\(\)\), Decision::DemandeDifferee\);",
                     t["test"]):
        fautes.append("test_reveil_cible.rs : la politique par defaut n'est plus verrouillee (une_tache_ordinaire_ne_coupe_personne).")
    b = bras(t["creation"])
    if "preempt::request_cpu(target)" not in b or "demande_ciblee" in b or "smp::reschedule_cpu(target)" not in b:
        fautes.append("creation.rs : le bras de la variante pose une demande ciblee ou n'envoie pas son IPI.")
    if t["flux"].count("BO_QEMU_KVM=1 tools/ci/run_ab_reveil_ipi.sh") != 2 \
            or not re.search(r"BO_QEMU_KVM=1 tools/ci/run_ab_reveil_ipi.sh \S+ \d+ 8\n", t["flux"]):
        fautes.append("os-primitives.yml : l'A/B de la variante n'est plus mesure sous KVM (SMP4 et SMP8).")
    return fautes


def sources() -> str:
    return "\n".join(p.read_text(encoding="utf-8") for p in sorted((RACINE / "src").rglob("*.rs")))


def main() -> int:
    t = {}
    for nom, chemin in FICHIERS.items():
        if not chemin.is_file():
            print(f"variante de reveil : {chemin} absent")
            return 1
        t[nom] = chemin.read_text(encoding="utf-8")
    src = sources()
    fautes = verifie(t, src)
    if fautes:
        print("variante de reveil : regle violee")
        for f in fautes:
            print("  " + f)
        return 1
    negatifs = [
        ("reveil", "core::sync::atomic::AtomicBool::new(false);", "core::sync::atomic::AtomicBool::new(true);"),
        ("reveil", DEFAUT_INCHANGE,
         "    let d = decide(cible, reveille);\n    if d != Decision::DemandeDifferee {\n        return d;\n    }"),
        ("creation", "            crate::kernel::scheduler::preempt::request_cpu(target);\n"
                     "            crate::kernel::scheduler::preempt::note_reveil_ipi_variante();",
         "            crate::kernel::scheduler::preempt::demande_ciblee(target);\n"
         "            crate::kernel::scheduler::preempt::note_reveil_ipi_variante();"),
        ("test", "assert_eq!(decide(&cible, &ordinaire()), Decision::DemandeDifferee);\n}",
         "assert_eq!(decide(&cible, &ordinaire()), Decision::DemandeAvecIpi);\n}"),
        ("flux", "BO_QEMU_KVM=1 tools/ci/run_ab_reveil_ipi.sh", "tools/ci/run_ab_reveil_ipi.sh"),
    ]
    for n, (cle, a, b) in enumerate(negatifs, 1):
        if a not in t[cle]:
            print(f"variante de reveil : le test negatif {n} ne s'applique plus ({cle}) -- garde inoperant")
            return 1
        mute = dict(t)
        mute[cle] = t[cle].replace(a, b, 1)
        if not verifie(mute, src):
            print(f"variante de reveil : le test negatif {n} ne rougit pas ({cle}) -- garde inoperant")
            return 1
    # Une bascule hors du shell : au demarrage d'un service, par exemple.
    if not verifie(t, src + "\nfn demarre() { choisit_variante(Variante::IpiInteractive); }\n"):
        print("variante de reveil : une bascule hors du shell ne rougit pas -- garde inoperant")
        return 1
    print(f"[OK] variante de reveil : desactivee au demarrage, basculee par ordo-variante seule, Defaut = decide, "
          f"jamais de demande ciblee, A/B KVM en CI (negatifs={len(negatifs) + 1})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
