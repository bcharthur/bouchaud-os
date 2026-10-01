#!/usr/bin/env python3
"""Une continuation synchrone n'est reprise que par la fin de sa racine.

BOUCHAUD_CONTINUATION_SYNCHRONE_V1

# Le defaut

TRIGKEY, T+60,519 s : `RUN_NOYAU_RETOUR nom=desktop fil_mort=0`, puis
quatorze `PROCESS_KILL raison=run_noyau_retour`. Sur le BSP, `KERNEL_CTX[0]`
etait a la fois l'idle du coeur et la continuation de `run_noyau` : la mort de
n'importe quelle tache du coeur zero, sans autre tache prete, reprenait le
lancement du bureau vivant ; et la mort de la racine, si une autre tache etait
prete, laissait la continuation garee pour toujours. `continuation-banc` l'a
reproduit sur le protocole d'origine (SMP4/8 : reprise etrangere ; SMP1/2 :
continuation perdue).

# La regle

  1. `switch_to_kernel` mene a `kernel_ctx()` (l'idle du coeur) et ne touche
     jamais `CONTINUATION` ;
  2. `run` et `run_noyau` garent la pile d'amorcage dans `CONTINUATION` par
     `gare_continuation`, jamais dans `kernel_ctx()` ;
  3. `consomme_continuation` refuse (panique) une continuation non
     `reprenable`, puis la consomme par CAS GAREE -> LIBRE ;
  4. le menage de `run_noyau` (reap, PROCESS_KILL, clear) n'a lieu qu'apres
     la verification `if !mort { panic!` ;
  5. la boucle idle -- AP et BSP, une seule -- teste la continuation due
     AVANT d'elire une tache ;
  6. le banc hote rejoue l'ancien et le nouveau protocole ;
  7. (BOUCHAUD_SORTIE_NON_PREEMPTEE_V1) la preemption noyau est refusee a une
     tache deja marquee zombie : coupee, elle n'est jamais republiee, et sa
     sortie -- dont la reprise de la continuation -- est perdue.

Fail-closed ; sept tests negatifs.
"""
import re
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
ORDO = "src/kernel/process/thread/ordonnancement.rs"
VIE = "src/kernel/process/thread/lifecycle.rs"
COURANT = "src/kernel/process/thread/courant.rs"
TEST = "tools/smp/test_continuation.rs"
PREEMPT = "src/kernel/scheduler/preempt.rs"
FICHIERS = (ORDO, VIE, COURANT, TEST, PREEMPT)


def sans_commentaires(texte: str) -> str:
    return "\n".join(l.split("//", 1)[0] for l in texte.splitlines())


def corps(texte: str, nom: str) -> str:
    m = re.search(r"\bfn\s+" + re.escape(nom) + r"\s*(?:<[^>{]*>)?\(", texte)
    if not m:
        return ""
    ouverture = texte.find("{", m.end())
    profondeur = 0
    for i in range(ouverture, len(texte)):
        if texte[i] == "{":
            profondeur += 1
        elif texte[i] == "}":
            profondeur -= 1
            if profondeur == 0:
                return texte[ouverture:i + 1]
    return ""


def verifie(racine: Path) -> list[str]:
    try:
        src = {f: sans_commentaires((racine / f).read_text(encoding="utf-8")) for f in FICHIERS}
    except OSError as e:
        return [f"lecture impossible : {e}"]
    fautes = []
    stk = corps(src[ORDO], "switch_to_kernel")
    if "kernel_ctx()" not in stk or "CONTINUATION" in stk or "consomme_continuation" in stk:
        fautes.append(f"{ORDO} : switch_to_kernel ne mene plus a l'idle seul")
    for nom in ("run", "run_noyau"):
        c = corps(src[VIE], nom)
        if "gare_continuation(" not in c or "addr_of_mut!(CONTINUATION.rsp)" not in c:
            fautes.append(f"{VIE} : {nom} ne gare plus sa pile dans CONTINUATION")
        if "kernel_ctx()" in c:
            fautes.append(f"{VIE} : {nom} gare de nouveau sa pile dans l'idle du coeur")
    cons = corps(src[COURANT], "consomme_continuation")
    p_test, p_panic, p_cas = cons.find("reprenable("), cons.find("panic!"), cons.find("compare_exchange")
    if min(p_test, p_panic, p_cas) < 0 or not p_test < p_panic < p_cas:
        fautes.append(f"{COURANT} : consomme_continuation ne refuse plus une continuation non due")
    rn = corps(src[VIE], "run_noyau")
    p_mort, p_reap, p_kill = rn.find("if !mort {"), rn.find("reap();"), rn.find("PROCESS_KILL")
    if min(p_mort, p_reap, p_kill) < 0 or not p_mort < p_reap < p_kill:
        fautes.append(f"{VIE} : le menage de run_noyau n'attend plus une vraie fin")
    idle = corps(src[ORDO], "boucle_idle")
    p_due, p_pick = idle.find("continuation_due(cpu_id)"), idle.find("pick_next(")
    if p_due < 0 or p_pick < 0 or p_due > p_pick:
        fautes.append(f"{ORDO} : la boucle idle ne reprend plus la continuation due avant d'elire")
    for nom in ("secondary_cpu_loop", "idle_bsp_trampoline"):
        if "boucle_idle(" not in corps(src[ORDO], nom):
            fautes.append(f"{ORDO} : {nom} ne passe plus par la boucle idle commune")
    if "!crate::kernel::task::sortie_en_cours_locale()" not in corps(src[PREEMPT], "preemption_noyau_sure"):
        fautes.append(f"{PREEMPT} : une tache qui meurt peut de nouveau etre preemptee")
    if 'src/kernel/scheduler/continuation.rs' not in src[TEST] or "Regle::Ancienne" not in src[TEST]:
        fautes.append(f"{TEST} : le banc hote ne rejoue plus l'ancien protocole")
    return fautes


def mutation(fichier: str, avant: str, apres: str) -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        copie = Path(tmp)
        for f in FICHIERS:
            dest = copie / f
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_text((RACINE / f).read_text(encoding="utf-8"), encoding="utf-8")
        cible = copie / fichier
        texte = cible.read_text(encoding="utf-8")
        if avant not in texte:
            return False
        cible.write_text(texte.replace(avant, apres, 1), encoding="utf-8")
        return bool(verifie(copie))


def main() -> int:
    fautes = verifie(RACINE)
    if fautes:
        print("continuation synchrone : regle violee")
        print("\n".join("  " + f for f in fautes))
        return 1
    negatifs = [
        (ORDO, "    let target_rsp = kernel_ctx().rsp;\n    unsafe { switch_context(&mut (*from_ptr).ctx.rsp, target_rsp); }",
         "    let target_rsp = unsafe { CONTINUATION.rsp };\n    unsafe { switch_context(&mut (*from_ptr).ctx.rsp, target_rsp); }"),
        (VIE, "    gare_continuation(0, process.pid, unsafe { (*to_ptr).tid });\n    let continuation = unsafe { core::ptr::addr_of_mut!(CONTINUATION.rsp) };",
         "    let continuation = &mut kernel_ctx().rsp as *mut u64;"),
        (COURANT, "        panic!(\n            \"task: reprise de la continuation",
         "        crate::kernel::dmesg::log_fmt(format_args!(\n            \"task: reprise de la continuation"),
        (VIE, "    if !mort {\n        panic!(", "    if false {\n        panic!("),
        (ORDO, "        if continuation_due(cpu_id) {\n            let cible", "        if false {\n            let cible"),
        (TEST, '#[path = "../../src/kernel/scheduler/continuation.rs"]', '#[path = "continuation_copie.rs"]'),
        (PREEMPT, "        && !crate::kernel::task::sortie_en_cours_locale()\n", ""),
    ]
    for n, (f, a, b) in enumerate(negatifs, 1):
        if not mutation(f, a, b):
            print(f"continuation synchrone : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"CONTINUATION_SYNCHRONE_OK negatifs={len(negatifs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
