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
  5. dans les gestionnaires d'exception, CHAQUE reouverture d'IF est gardee
     par `if from_user(...)` (faute venue du ring 3 : trappe synchrone sur la
     pile noyau de la tache, comme un appel systeme), et aucun vecteur qui
     peut rouvrir IF n'a de pile IST (une IST est partagee par le coeur : y
     dormir la ferait ecraser par la faute suivante) ;
  6. les compteurs publies par `smpstat`.

BOUCHAUD_P19_FATAL_USER_IRQ_EXIT_V1 : la V1 de ce garde ne lisait que la
PREMIERE reouverture du fichier d'exceptions. Une seconde, ajoutee plus bas
et sans condition, passait. Toutes sont lues desormais (test negatif 9).

Fail-closed ; onze tests negatifs.
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
IDT = "src/arch/x86_64/idt.rs"
COMMUTATION = "src/kernel/process/thread/commutation.rs"
FICHIERS = (PREEMPT, PUR, ORDO, METRIQUES, EXCEPTIONS, TIMER, SMPF, IDT, COMMUTATION)
# Les vecteurs dont le gestionnaire peut rouvrir IF (faute venue du ring 3).
VECTEURS_IF = ("page_fault", "general_protection_fault", "invalid_opcode",
               "divide_error", "stack_segment_fault")

# Fichier -> pourquoi il peut rouvrir IF. Aucun n'est execute depuis un
# gestionnaire d'interruption avec IF a rouvrir.
IF_AUTORISES = {
    "src/arch/x86_64/cpu/idle/lock_park.rs": "idle : sti; hlt",
    "src/arch/x86_64/cpu/idle/scheduler.rs": "idle : sti; hlt",
    "src/arch/x86_64/idt/exceptions.rs": "faute VENUE DE L'ESPACE UTILISATEUR seulement (#PF, et sortie fatale #UD/#GP/#DE/#SS/#PF)",
    "src/arch/x86_64/interrupts.rs": "amorcage (BSP, AP)",
    "src/arch/x86_64/pat.rs": "restauration d'un IF leve a l'entree",
    "src/arch/x86_64/smp.rs": "fin d'amorcage, restauration",
    "src/arch/x86_64/usermode.rs": "entree d'appel systeme (contexte de tache)",
    "src/drivers/input/ps2_keyboard.rs": "attente en tache",
    "src/drivers/serial/uart16550.rs": "restauration d'un IF leve a l'entree",
    "src/kernel/memory/virtual.rs": "attente de quiescence en tache",
    COMMUTATION: "restauration du IF de la pile reprise, apres restauration irq_depth",
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

    commutation = corps(src[COMMUTATION], "switch_context")
    etapes = ["let enabled = interrupts::are_enabled();", "interrupts::disable();",
              "irq_context.suspend();", "switch_context_raw(from, to);",
              "irq_context.restore(depth);", "if enabled { interrupts::enable(); }"]
    positions = [commutation.find(e) for e in etapes]
    if min(positions) < 0 or positions != sorted(positions):
        fautes.append(f"{COMMUTATION} : IF/irq_depth ne suivent plus la pile commutee")
    if len(OUVRE_IF.findall(src[COMMUTATION])) != 1:
        fautes.append(f"{COMMUTATION} : reouverture IF hors restauration conditionnelle")
    if "local.irq_depth() == 1" not in corps(src[PREEMPT], "preemption_noyau_sure"):
        fautes.append(f"{PREEMPT} : preemption depuis une IRQ imbriquee possible")
    # Every returning IDT entry, including spurious IRQs, participates.
    # The two terminal panic handlers never resume a task or return by IRET.
    for f in (racine / "src/arch/x86_64/idt").glob("*.rs"):
        texte = sans_commentaires(f.read_text())
        for nom in re.findall(r'extern "x86-interrupt" fn (\w+)\(', texte):
            if nom not in ("panic_stop_handler", "double_fault_handler"):
                if "GsGuard::enter(&stack)" not in corps(texte, nom):
                    fautes.append(f"{f.name}:{nom} : entree/sortie IRQ non suivie")

    e = src[EXCEPTIONS]
    sites = [m.start() for m in OUVRE_IF.finditer(e)]
    if not sites:
        fautes.append(f"{EXCEPTIONS} : la faute de page utilisateur ne rouvre plus IF")
    for i in sites:
        avant = e[max(0, i - 160):i]
        if not re.search(r"if from_user\(&?stack\) \{\s*(x86_64::instructions::)?$", avant):
            ligne = e.count("\n", 0, i) + 1
            fautes.append(f"{EXCEPTIONS}:{ligne} : IF rouvert dans un gestionnaire hors faute venue de l'espace utilisateur")
    pf = corps(e, "page_fault_handler")
    passage = "x86_64::instructions::interrupts::without_interrupts(|| _gs.trappe_utilisateur());"
    fin_resolue = pf.find("return;", pf.find("FaultOutcome::Resolved"))
    if e.count("_gs.trappe_utilisateur()") != 1 or passage not in pf:
        fautes.append("la continuation fatale seule doit quitter IRQ, IF masque")
    if not 0 <= pf.find("Cr2::read_raw()") < fin_resolue < pf.find(passage) < pf.find("log_fault_mapping(addr)"):
        fautes.append("sortie IRQ avant fin du peuplement ou apres les diagnostics fatals")
    if "if from_user(&stack) && crate::kernel::task::in_user_task() {" not in pf[:pf.find(passage)]:
        fautes.append("continuation fatale hors tache utilisateur")
    if "assert!(self.swapped && self.irq_active);" not in corps(e, "trappe_utilisateur"):
        fautes.append("une exception noyau peut quitter le contexte IRQ")
    drop = corps(e, "drop")
    if not 0 <= drop.find("interrupts::disable();") < drop.find("swapgs()") or "if self.irq_active {" not in drop:
        fautes.append("retour GS avec IF ouvert ou double sortie IRQ apres trappe utilisateur")
    if "preempt::safe_point();" in pf:
        fautes.append("point sur de retour #PF non qualifie reintroduit")
    idt = src[IDT]
    for m in re.finditer(r"\.set_stack_index\(", idt):
        instruction = idt[idt.rfind(";", 0, m.start()) + 1:m.start()]
        if "IDT.double_fault" not in instruction:
            fautes.append(f"{IDT} : pile IST hors de la double faute ({instruction.strip()[:60]})")
    for v in VECTEURS_IF:
        if not re.search(r"IDT\." + v + r"\.set_handler_fn\(\w+\);", idt):
            fautes.append(f"{IDT} : {v} n'est plus installe sans IST (forme attendue introuvable)")
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
        (EXCEPTIONS, "        x86_64::instructions::interrupts::without_interrupts(|| _gs.trappe_utilisateur());\n", ""),
        (EXCEPTIONS, "assert!(self.swapped && self.irq_active);", "assert!(self.irq_active);"),
        (EXCEPTIONS, "        x86_64::instructions::interrupts::disable();\n", ""),
        (EXCEPTIONS, "        if self.irq_active {", "        if true {"),
        (EXCEPTIONS, "            crate::kernel::task::account_fault_exit(mur_avant_faute);", "            let _ = crate::kernel::scheduler::preempt::safe_point();\n            crate::kernel::task::account_fault_exit(mur_avant_faute);"),
        (EXCEPTIONS, "        crate::kernel::task::stall_pf_fail(addr);", "        _gs.trappe_utilisateur();\n        crate::kernel::task::stall_pf_fail(addr);"),
        (EXCEPTIONS, 'fn breakpoint_handler(stack: InterruptStackFrame) {\n    let _gs = GsGuard::enter(&stack);',
         'fn breakpoint_handler(stack: InterruptStackFrame) {'),
        (COMMUTATION, "if enabled { interrupts::enable(); }", "interrupts::enable();"),
        (COMMUTATION, "irq_context.restore(depth);", "irq_context.restore(0);"),
        (COMMUTATION, "irq_context.suspend();", "irq_context.depth();"),
        (PREEMPT, "local.irq_depth() == 1", "local.irq_depth() <= 1"),
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
        # 9. Une seconde reouverture, SANS condition, plus bas que la premiere.
        (EXCEPTIONS, '    println!("exception: breakpoint (int3) capturee, on continue");\n',
         '    println!("exception: breakpoint (int3) capturee, on continue");\n'
         '    x86_64::instructions::interrupts::enable();\n'),
        # 10. La sortie fatale rouvre IF sans relire l'origine de la faute.
        (EXCEPTIONS, "    if from_user(stack) {\n        x86_64::instructions::interrupts::enable();\n    }\n",
         "    x86_64::instructions::interrupts::enable();\n"),
        # 11. Une pile IST sur un vecteur qui peut rouvrir IF et dormir.
        (IDT, "IDT.general_protection_fault.set_handler_fn(general_protection_handler);",
         "IDT.general_protection_fault.set_handler_fn(general_protection_handler)"
         ".set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);"),
    ]
    for n, (f, a, b) in enumerate(negatifs, 1):
        if not mutation(f, a, b):
            print(f"preemption noyau : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"PREEMPTION_NOYAU_OK negatifs={len(negatifs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
