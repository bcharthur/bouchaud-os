#!/usr/bin/env python3
"""Un fil noyau n'est commute depuis une interruption que sur demande ciblee,
dans un contexte sur -- jamais depuis une IRQ imbriquee.

BOUCHAUD_PREEMPTION_NOYAU_SURE_V1 (retire BOUCHAUD_QUANTUM_NOYAU_V1)

Le noyau n'est pas preemptible en son sein : le registre des taches, le
shootdown TLB et le compositeur en font leur contrat. Le quantum noyau
(`1df8a88b`) coupait tout fil noyau a n'importe quel point IF=1 sans verrou
compte ; un panic l'a mesure (`services-metrics` commute sous garde de
lecture du registre). La decision est `kernel::preemption_noyau::decide`,
pure, testee exhaustivement sur l'hote (`tools/smp/test_preemption_noyau.rs`).
Ce garde verifie son CABLAGE et l'invariant structurel dont elle depend :

  1. `accorde_preemption_noyau` sort sans demande ciblee et n'accorde que ce
     que `decide` accorde ; `decide` exige la demande ET `contexte_sur` ;
  2. `contexte_sur` exige les sept conditions (section non preemptable,
     verrous simples, lockdep, LECTURE DU REGISTRE, SHOOTDOWN EN VOL, sortie
     en cours, interruptions MASQUEES) ; le contexte les lit reellement ;
  3. le quantum noyau ne revient pas par la bande (`QUANTUM_NOYAU_NS`,
     `fil_noyau_quantum_epuise`, `tranche_noyau_locale`) ;
  4. pas d'IRQ imbriquee PAR CONSTRUCTION : aucune porte de trappe dans
     l'IDT, et les seuls fichiers qui rouvrent IF sont ceux de la liste
     ci-dessous -- tout nouveau site doit etre justifie ici ;
  5. les compteurs publies par `smpstat`.

Fail-closed ; huit tests negatifs.
"""
import re
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
PREEMPT = "src/kernel/scheduler/preempt.rs"
PUR = "src/kernel/scheduler/preemption_noyau.rs"
ORDO = "src/kernel/process/thread/ordonnancement.rs"
METRIQUES = "src/kernel/process/thread/metriques.rs"
EXCEPTIONS = "src/arch/x86_64/idt/exceptions.rs"
TIMER = "src/arch/x86_64/idt/timer.rs"
SMPF = "src/arch/x86_64/smp.rs"
FICHIERS = (PREEMPT, PUR, ORDO, METRIQUES, EXCEPTIONS, TIMER, SMPF)

# Fichier -> pourquoi il peut rouvrir IF. Aucun n'est execute depuis un
# gestionnaire d'interruption avec IF a rouvrir.
IF_AUTORISES = {
    "src/arch/x86_64/cpu/idle/lock_park.rs": "idle : sti; hlt",
    "src/arch/x86_64/cpu/idle/scheduler.rs": "idle : sti; hlt",
    "src/arch/x86_64/idt/exceptions.rs": "faute de page VENUE DE L'ESPACE UTILISATEUR seulement",
    "src/arch/x86_64/interrupts.rs": "amorcage (BSP, AP)",
    "src/arch/x86_64/pat.rs": "restauration d'un IF leve a l'entree",
    "src/arch/x86_64/smp.rs": "fin d'amorcage, restauration",
    "src/arch/x86_64/usermode.rs": "entree d'appel systeme (contexte de tache)",
    "src/drivers/input/ps2_keyboard.rs": "attente en tache",
    "src/drivers/serial/uart16550.rs": "restauration d'un IF leve a l'entree",
    "src/kernel/memory/virtual.rs": "attente de quiescence en tache",
    "src/kernel/process/thread/registre.rs": "restauration d'un IF leve a l'entree",
    "src/kernel/sync/spinlock.rs": "restauration d'un IF leve a la prise",
    "src/net/transport/tcp.rs": "attente en tache",
}
OUVRE_IF = re.compile(r'interrupts::enable\(\)|interrupts::enable_and_hlt\(\)|"sti(;[^"]*)?"')
PORTE_TRAPPE = re.compile(r"disable_interrupts\(\s*false\s*\)|TrapGate|set_gate_type")


def sans_commentaires(t: str) -> str:
    return "\n".join(l.split("//", 1)[0] for l in t.splitlines())


def corps(texte: str, nom: str) -> str:
    m = re.search(r"\bfn\s+" + re.escape(nom) + r"\s*\(", texte)
    if not m:
        return ""
    o = texte.find("{", m.end())
    p = 0
    for i in range(o, len(texte)):
        p += texte[i] == "{"
        p -= texte[i] == "}"
        if p == 0:
            return texte[o:i + 1]
    return ""


def verifie(racine: Path) -> list[str]:
    try:
        src = {f: sans_commentaires((racine / f).read_text(encoding="utf-8")) for f in FICHIERS}
    except OSError as e:
        return [f"lecture impossible : {e}"]
    fautes = []

    a = corps(src[PREEMPT], "accorde_preemption_noyau")
    if not re.search(r"if !ciblee \{\s*return false;", a):
        fautes.append(f"{PREEMPT} : accorde_preemption_noyau ne sort plus sans demande ciblee")
    if "preemption_noyau::decide(&contexte)" not in a:
        fautes.append(f"{PREEMPT} : accorde_preemption_noyau ne passe plus par decide")
    vrais = [m.start() for m in re.finditer(r"\btrue\b", a)]
    garde = a.find("crate::kernel::preemption_noyau::decide(&contexte) {")
    fin_garde = a.find("}", garde)
    if garde < 0 or any(not (garde < v < fin_garde) for v in vrais):
        fautes.append(f"{PREEMPT} : une preemption est accordee hors de decide")
    d = corps(src[PUR], "decide")
    if "c.demande_ciblee && contexte_sur(c)" not in d:
        fautes.append(f"{PUR} : decide n'exige plus la demande ciblee ET le contexte sur")

    s = corps(src[PUR], "contexte_sur")
    for cond in ("c.preempt_count == 0", "c.verrous_simples == 0", "c.profondeur_lockdep == 0",
                 "c.lectures_registre == 0", "!c.shootdown_en_vol", "!c.sortie_en_cours",
                 "!c.interruptions_ouvertes"):
        if cond not in s:
            fautes.append(f"{PUR} : contexte_sur n'exige plus {cond}")
    c = corps(src[PREEMPT], "contexte_noyau")
    for lu in ("interruptions_ouvertes: x86_64::instructions::interrupts::are_enabled()",
               "lectures_registre: crate::kernel::task::lectures_registre_locales()",
               "shootdown_en_vol: smp::shootdown_en_vol_local()",
               "sortie_en_cours: crate::kernel::task::sortie_en_cours_locale()"):
        if lu not in c:
            fautes.append(f"{PREEMPT} : le contexte ne lit plus `{lu.split(':')[0]}` reellement")
    if "TLB_SLOTS[cpu].sequence.load(Ordering::Acquire) != 0" not in corps(src[SMPF], "shootdown_en_vol_local"):
        fautes.append(f"{SMPF} : shootdown_en_vol_local ne lit plus l'emplacement du coeur")
    if "contexte_sur(&c)" not in corps(src[PREEMPT], "preemption_noyau_sure"):
        fautes.append(f"{PREEMPT} : preemption_noyau_sure ne passe plus par contexte_sur")
    for revenant in ("QUANTUM_NOYAU_NS", "fil_noyau_quantum_epuise", "tranche_noyau_locale"):
        if revenant in src[PREEMPT] or revenant in src[ORDO]:
            fautes.append(f"{revenant} est revenu : le quantum noyau coupe un fil noyau n'importe ou")

    e = src[EXCEPTIONS]
    i = e.find("x86_64::instructions::interrupts::enable();")
    if i < 0 or "if from_user(&stack) {" not in e[max(0, i - 80):i]:
        fautes.append(f"{EXCEPTIONS} : IF rouvert dans un gestionnaire hors faute venue de l'espace utilisateur")
    for f in sorted((racine / "src").rglob("*.rs")):
        rel = str(f.relative_to(racine))
        texte = sans_commentaires(f.read_text(encoding="utf-8", errors="replace"))
        if PORTE_TRAPPE.search(texte):
            fautes.append(f"{rel} : porte de trappe dans l'IDT (IF non masque a l'entree)")
        if OUVRE_IF.search(texte) and rel not in IF_AUTORISES:
            fautes.append(f"{rel} : rouvre IF -- site non justifie (IRQ imbriquee possible ?)")
    if OUVRE_IF.search(src[TIMER]):
        fautes.append(f"{TIMER} : le gestionnaire du minuteur rouvre IF")

    r = corps(src[PREEMPT], "log_reveil")
    if "refus_lecteur=" not in r or "if_ouvert=" not in r:
        fautes.append(f"{PREEMPT} : log_reveil ne publie plus refus_lecteur / if_ouvert")
    if "scheduler::preempt::log_reveil();" not in corps(src[METRIQUES], "log_smp_load"):
        fautes.append(f"{METRIQUES} : log_reveil n'est plus appele par smpstat")
    return fautes


def mutation(fichier: str, a: str, b: str) -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        for f in sorted((RACINE / "src").rglob("*.rs")):
            rel = str(f.relative_to(RACINE))
            d = Path(tmp) / rel
            d.parent.mkdir(parents=True, exist_ok=True)
            t = f.read_text(encoding="utf-8", errors="replace")
            if rel == fichier:
                if a not in t:
                    return False
                t = t.replace(a, b, 1)
            d.write_text(t, encoding="utf-8")
        return bool(verifie(Path(tmp)))


def main() -> int:
    fautes = verifie(RACINE)
    if fautes:
        print("preemption noyau : regle violee")
        print("\n".join("  " + f for f in fautes))
        return 1
    negatifs = [
        (PUR, "        && c.lectures_registre == 0\n", "\n"),
        (PUR, "        && !c.shootdown_en_vol\n", "\n"),
        (PUR, "        && !c.interruptions_ouvertes\n", "\n"),
        (PUR, "    c.demande_ciblee && contexte_sur(c)", "    contexte_sur(c)"),
        (PREEMPT, "    let ciblee = demande_ciblee_pendante();\n    if !ciblee {\n        return false;\n    }\n",
         "    let ciblee = demande_ciblee_pendante();\n"),
        (PREEMPT, "pub fn preemptions_noyau_if_ouvert", "pub const QUANTUM_NOYAU_NS: u64 = 8_000_000;\npub fn preemptions_noyau_if_ouvert"),
        (TIMER, "    let interrupted_user = from_user(&stack);\n",
         "    let interrupted_user = from_user(&stack);\n    x86_64::instructions::interrupts::enable();\n"),
        (METRIQUES, "    crate::kernel::scheduler::preempt::log_reveil();\n", ""),
    ]
    for n, (f, a, b) in enumerate(negatifs, 1):
        if not mutation(f, a, b):
            print(f"preemption noyau : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"PREEMPTION_NOYAU_OK negatifs={len(negatifs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
