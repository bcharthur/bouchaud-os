#!/usr/bin/env python3
from pathlib import Path
import re
import shutil

ROOT = Path(__file__).resolve().parent.parent
THREAD = ROOT / "src/kernel/process/thread.rs"
TEST_SRC = ROOT / "gate0/test_commutation_final.rs"
TEST_DST = ROOT / "tools/smp/test_commutation.rs"
RUN = ROOT / "run.ps1"
MARKER = "BOUCHAUD_GATE0_POST_SWITCH_HANDOFF_V2"

def read(path: Path) -> str:
    return path.read_text(encoding="utf-8").replace("\r\n", "\n")

def write(path: Path, data: str) -> None:
    with path.open("w", encoding="utf-8", newline="\n") as f:
        f.write(data)

def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"{label}: attendu 1 occurrence, trouve {count}")
    return text.replace(old, new, 1)

def regex_once(text: str, pattern: str, repl: str, label: str) -> str:
    out, count = re.subn(pattern, repl, text, count=1, flags=re.S)
    if count != 1:
        raise RuntimeError(f"{label}: attendu 1 remplacement regex, trouve {count}")
    return out

def patch_thread(text: str) -> str:
    if MARKER in text:
        return text

    text = replace_once(
        text,
        '''    /// CPU qui execute actuellement cette tache, -1 si elle est en runqueue.
    pub on_cpu: i8,
    /// Derniere migration effective, pour imposer une residence cache minimale.
''',
        '''    /// CPU qui possede encore l'execution/la pile de cette tache.
    ///
    /// Pendant une commutation sortante, la valeur RESTE >= 0 jusqu'a la
    /// confirmation post-switch. Ainsi aucun autre CPU ne peut republier la
    /// tache tant que l'ancien CPU utilise encore physiquement sa pile.
    pub on_cpu: i8,
    /// Vrai entre prepare_switch_handoff() et complete_switch_handoff().
    /// Protege par le BKL ; ce n'est pas une primitive atomique autonome.
    switching_out: bool,
    /// Derniere migration effective, pour imposer une residence cache minimale.
''',
        "Task.switching_out",
    )

    text = replace_once(
        text,
        '''            on_cpu: -1,
            last_migration_ns: 0,
''',
        '''            on_cpu: -1,
            switching_out: false,
            last_migration_ns: 0,
''',
        "Task::new switching_out",
    )

    text = replace_once(
        text,
        '''/// Zombie qui vient de quitter physiquement la pile de ce CPU. Le contexte
/// entrant le rend recyclable une fois le switch assembleur effectivement fini.
static RETIRED: [AtomicUsize; MAX_CPUS] = [const { AtomicUsize::new(NO_TASK) }; MAX_CPUS];
''',
        f'''// {MARKER}
/// Tache dont CE CPU est en train d'abandonner la pile.
///
/// Invariant Gate 0 : tant que cette case n'est pas completee depuis la pile
/// entrante, la tache sortante garde `on_cpu >= 0`, `switching_out = true` et
/// n'apparait dans AUCUNE runqueue. La publication n'arrive qu'apres le
/// `mov rsp, rsi` de switch_context, jamais apres le seul `mov [rdi], rsp`.
static SWITCH_PENDING: [AtomicUsize; MAX_CPUS] =
    [const {{ AtomicUsize::new(NO_TASK) }}; MAX_CPUS];
''',
        "RETIRED -> SWITCH_PENDING",
    )

    old_complete = '''/// A appeler dans le contexte qui vient de PRENDRE le CPU, BKL tenu. Le zombie
/// note avant le switch ne peut plus utiliser sa pile : son slot devient donc
/// recyclable sans use-after-free.
fn complete_retired() {
    let cpu = local_cpu();
    let retired = RETIRED[cpu].swap(NO_TASK, Ordering::AcqRel);
    if retired == NO_TASK { return; }
    if let Some(task) = tasks().get_mut(retired) {
        if task.state == TaskState::Zombie { task.on_cpu = -1; }
    }
}
'''
    new_complete = r'''/// RSP physique courant, uniquement pour verifier l'invariant de passation.
#[inline]
fn rsp_courant_passation() -> u64 {
    let rsp: u64;
    unsafe {
        core::arch::asm!(
            "mov {}, rsp",
            out(reg) rsp,
            options(nomem, nostack, preserves_flags)
        );
    }
    rsp
}

/// Prepare la sortie d'une tache SANS la publier.
///
/// Le BKL est tenu. La tache reste `on_cpu == cpu` et n'est pas mise en file.
/// C'est volontaire : `switch_context` sauvegarde d'abord son RSP puis change
/// de pile ; entre ces deux instructions, l'ancien CPU utilise encore la pile.
#[inline]
fn prepare_switch_handoff(index: usize, task: &mut Task, cpu: usize) {
    debug_assert!(
        smp_lock::held_by_current_cpu(),
        "task: preparation de passation sans BKL"
    );
    assert_eq!(
        task.on_cpu,
        cpu as i8,
        "task: passation d'une tache non residente sur ce CPU tid={}",
        task.tid
    );
    assert!(
        !task.switching_out,
        "task: double preparation de passation tid={}",
        task.tid
    );

    match SWITCH_PENDING[cpu].compare_exchange(
        NO_TASK,
        index,
        Ordering::AcqRel,
        Ordering::Acquire,
    ) {
        Ok(_) => task.switching_out = true,
        Err(previous) => panic!(
            "task: passation precedente non terminee cpu={} pending={} nouveau={}",
            cpu, previous, index
        ),
    }
}

/// Publie la tache sortante depuis la pile ENTRANTE.
///
/// C'est LE point qui ferme Gate 0 : `on_cpu` ne devient negatif qu'ici.
/// Un reveil concurrent pendant la commutation peut mettre `state=Ready`, mais
/// `publish_ready()` refuse encore la tache tant que `on_cpu >= 0`. Ici, apres
/// abandon physique de l'ancienne pile, on republie exactement une fois.
fn complete_switch_handoff() {
    let cpu = local_cpu();
    let outgoing = SWITCH_PENDING[cpu].load(Ordering::Acquire);
    if outgoing == NO_TASK {
        return;
    }

    debug_assert!(
        smp_lock::held_by_current_cpu(),
        "task: completion de passation sans BKL"
    );
    assert!(
        outgoing < tasks().len(),
        "task: passation vers slot invalide cpu={} slot={}",
        cpu,
        outgoing
    );

    let publish = {
        let task = &mut tasks()[outgoing];

        assert!(
            task.switching_out,
            "task: passation pending sans switching_out tid={}",
            task.tid
        );
        assert_eq!(
            task.on_cpu,
            cpu as i8,
            "task: outgoing publie avant completion tid={} on_cpu={} cpu={}",
            task.tid,
            task.on_cpu,
            cpu
        );

        #[cfg(debug_assertions)]
        {
            let rsp = rsp_courant_passation();
            let base = task.kstack_top.saturating_sub(KSTACK_SIZE as u64);
            debug_assert!(
                rsp < base || rsp >= task.kstack_top,
                "task: publication avant abandon physique de la pile tid={} rsp={:#x} pile={:#x}..{:#x}",
                task.tid,
                rsp,
                base,
                task.kstack_top
            );
        }

        task.last_cpu = cpu as u8;
        task.runq_cpu = cpu as u8;
        task.on_cpu = -1;
        task.switching_out = false;
        task.state == TaskState::Ready
    };

    SWITCH_PENDING[cpu].store(NO_TASK, Ordering::Release);

    if publish {
        publish_ready(outgoing);
    }
}
'''
    text = replace_once(text, old_complete, new_complete, "complete_switch_handoff")
    text = text.replace("complete_retired()", "complete_switch_handoff()")
    text = re.sub(r"\bRETIRED\b", "SWITCH_PENDING", text)

    # Le site de diagnostic 54 portait encore l'ancien NOM sans parentheses.
    # Le controle final cherchait volontairement tout vestige de
    # `complete_retired`; cette simple legende historique declenchait donc un
    # faux positif alors que le code executable avait deja ete remplace.
    #
    # Ne pas affaiblir le controle : mettre la documentation a jour.
    text = replace_once(
        text,
        "// 54=complete_retired, 55=activate_kernel, 61=timer+BKL.\n",
        "// 54=complete_switch_handoff, 55=activate_kernel, 61=timer+BKL.\n",
        "legende stall site 54",
    )

    sentinel_pattern = r'''// BOUCHAUD_P0_CTX_EN_VOL_V1\n.*?(?=fn runnable_local\(task: &Task, cpu: usize\) -> bool \{)'''
    sentinel_repl = '''// BOUCHAUD_GATE0_POST_SWITCH_HANDOFF_V2
//
// La tache sortante n'est plus publiee au moment ou `ctx.rsp` change.
// Elle devient schedulable uniquement depuis la continuation entrante, donc
// APRES le changement physique de pile. `ctx.rsp` redevient un simple contexte
// machine et n'est plus un drapeau de synchronisation.
'''
    text = regex_once(text, sentinel_pattern, sentinel_repl, "suppression sentinelle")

    count = text.count("        && contexte_publie(task)\n")
    if count != 2:
        raise RuntimeError(f"runnable contexte_publie: attendu 2, trouve {count}")
    text = text.replace("        && contexte_publie(task)\n", "")
    # Garde redondant volontaire : meme si un futur refactor modifiait on_cpu,
    # l'etat explicite de passation empeche toujours la selection.
    runnable_guard = "        && task.on_cpu < 0\n"
    if text.count(runnable_guard) < 2:
        raise RuntimeError("runnable on_cpu guard introuvable")
    # Limiter aux deux fonctions situees avant pick_next.
    start = text.index("fn runnable_local")
    end = text.index("fn pick_next", start)
    region = text[start:end]
    if region.count(runnable_guard) != 2:
        raise RuntimeError("runnable_local/steal: gardes on_cpu inattendus")
    region = region.replace(
        runnable_guard,
        runnable_guard + "        && !task.switching_out\n"
    )
    text = text[:start] + region + text[end:]

    old_local_pick = '''    if let Some(id) = crate::arch::x86_64::cpu_local::CpuId::from_index(cpu) {
        let local = crate::arch::x86_64::cpu_local::local(id);
        // Une tache en vol reste EXECUTABLE : la laisser tomber de la file la
        // perdrait pour de bon. On la remet donc en queue et on passe a la
        // suivante. Le tour de file est borne pour qu'une file entierement en
        // vol ne fasse pas tourner ce CPU indefiniment.
        let mut tours_restants = local.run_queue_len().saturating_add(1);
        while let Some(index) = local.dequeue() {
            if index < len && !contexte_publie(&tasks()[index]) {
                local.enqueue(index);
                tours_restants = tours_restants.saturating_sub(1);
                if tours_restants == 0 {
                    break;
                }
                continue;
            }
            if index < len && runnable_local(&tasks()[index], cpu) {
                return Some(index);
            }
        }
    }
'''
    new_local_pick = '''    if let Some(id) = crate::arch::x86_64::cpu_local::CpuId::from_index(cpu) {
        let local = crate::arch::x86_64::cpu_local::local(id);
        while let Some(index) = local.dequeue() {
            if index < len && runnable_local(&tasks()[index], cpu) {
                return Some(index);
            }
        }
    }
'''
    text = replace_once(text, old_local_pick, new_local_pick, "pick_next local")

    old_mark = '''    debug_assert!(task.on_cpu < 0, "task: tentative de double execution tid={}", task.tid);
    // BOUCHAUD_P0_CTX_EN_VOL_V1 : reprendre une tache dont `switch_context`
    // n'a pas encore ecrit le sommet ferait repartir CE CPU sur une pile deja
    // occupee par un autre. L'invariant est verifie ici, une fois, pour les
    // deux appelants (`switch_to` et `preempt_from_irq`).
    debug_assert!(
        contexte_publie(task),
        "task: reprise d'une tache dont le contexte est encore en vol tid={}",
        task.tid
    );
    debug_assert_eq!(task.last_account_ns, 0, "task: cursor CPU encore arme hors CPU tid={}", task.tid);
'''
    new_mark = '''    assert!(task.on_cpu < 0, "task: tentative de double execution tid={}", task.tid);
    assert!(
        !task.switching_out,
        "task: tentative de reprendre une tache dont la passation n'est pas terminee tid={}",
        task.tid
    );
    debug_assert_eq!(task.last_account_ns, 0, "task: cursor CPU encore arme hors CPU tid={}", task.tid);
'''
    text = replace_once(text, old_mark, new_mark, "mark_task_running invariant")

    text = replace_once(
        text,
        '''    task.on_cpu = cpu_id as i8;
    task.slice_start_ns = now;
''',
        '''    task.on_cpu = cpu_id as i8;
    task.switching_out = false;
    task.slice_start_ns = now;
''',
        "mark_task_running flag",
    )

    old_zombie = '''    if task.on_cpu >= 0 {
        let cpu = task.on_cpu as usize;
        if cpu < MAX_CPUS {
            RETRAITE_DEMANDEE[cpu].store(true, Ordering::Release);
        }
    }
'''
    new_zombie = '''    if task.on_cpu >= 0 && !task.switching_out {
        let cpu = task.on_cpu as usize;
        if cpu < MAX_CPUS {
            RETRAITE_DEMANDEE[cpu].store(true, Ordering::Release);
        }
    }
'''
    text = replace_once(text, old_zombie, new_zombie, "marque_zombie")

    text = replace_once(
        text,
        '''    if index >= tasks().len() || tasks()[index].state != TaskState::Ready
        || tasks()[index].on_cpu >= 0
    {
''',
        '''    if index >= tasks().len() || tasks()[index].state != TaskState::Ready
        || tasks()[index].on_cpu >= 0
        || tasks()[index].switching_out
    {
''',
        "publish_ready",
    )

    text = replace_once(
        text,
        '''    task.on_cpu = -1;

    let reuse = tasks().iter().position(|old| {
        old.state == TaskState::Zombie && old.on_cpu < 0
    });
''',
        '''    task.on_cpu = -1;
    task.switching_out = false;

    let reuse = tasks().iter().position(|old| {
        old.state == TaskState::Zombie && old.on_cpu < 0 && !old.switching_out
    });
''',
        "register/reuse pending-safe",
    )

    # Les compteurs de pression ne doivent jamais transformer un etat pending
    # en candidat, meme si un futur changement touche on_cpu.
    for label, old_pred, new_pred in (
        (
            "ready_count_cpu pending-safe",
            '''        t.state == TaskState::Ready
            && t.on_cpu < 0
            && t.runq_cpu as usize == cpu
''',
            '''        t.state == TaskState::Ready
            && t.on_cpu < 0
            && !t.switching_out
            && t.runq_cpu as usize == cpu
''',
        ),
        (
            "stealable_count_cpu pending-safe",
            '''        t.state == TaskState::Ready
            && t.on_cpu < 0
            && !t.noyau
''',
            '''        t.state == TaskState::Ready
            && t.on_cpu < 0
            && !t.switching_out
            && !t.noyau
''',
        ),
    ):
        text = replace_once(text, old_pred, new_pred, label)

    text = replace_once(
        text,
        '''        .filter(|t| t.state != TaskState::Zombie && t.on_cpu == cpu as i8)
''',
        '''        .filter(|t| {
            t.state != TaskState::Zombie
                && t.on_cpu == cpu as i8
                && !t.switching_out
        })
''',
        "running_count_cpu",
    )

    text = replace_once(
        text,
        '''pub fn schedule() -> bool {
    let _kernel = smp_lock::enter();
    let cur = current_index_raw();
''',
        '''pub fn schedule() -> bool {
    let _kernel = smp_lock::enter();
    complete_switch_handoff();
    let cur = current_index_raw();
''',
        "schedule drain",
    )

    text = replace_once(
        text,
        '''fn switch_to(from: usize, to: usize) {
    let _kernel = smp_lock::enter();
    let cpu_id = local_cpu();
''',
        '''fn switch_to(from: usize, to: usize) {
    let _kernel = smp_lock::enter();
    complete_switch_handoff();
    let cpu_id = local_cpu();
''',
        "switch_to drain",
    )

    old_switch_to = '''        deactivate_task_space(&*from_ptr, cpu_id);
        // BOUCHAUD_P0_CTX_EN_VOL_V1 : avant toute publication.
        marque_contexte_en_vol(&mut *from_ptr);
        if (*from_ptr).state == TaskState::Zombie {
            SWITCH_PENDING[cpu_id].store(from, Ordering::Release);
            // Reste marque running jusqu'a ce que le nouveau contexte confirme
            // que le switch assembleur a effectivement quitte cette pile.
        } else {
            (*from_ptr).on_cpu = -1;
            (*from_ptr).last_cpu = cpu_id as u8;
            (*from_ptr).runq_cpu = cpu_id as u8;
            if (*from_ptr).state == TaskState::Ready {
                if let Some(id) = crate::arch::x86_64::cpu_local::CpuId::from_index(cpu_id) {
                    crate::arch::x86_64::cpu_local::local(id).enqueue(from);
                }
            }
        }
        mark_task_running(&mut *to_ptr, cpu_id);
'''
    new_switch_to = '''        deactivate_task_space(&*from_ptr, cpu_id);
        prepare_switch_handoff(from, &mut *from_ptr, cpu_id);
        // PAS de on_cpu=-1, PAS d'enqueue ici. Cette pile est encore active.
        mark_task_running(&mut *to_ptr, cpu_id);
'''
    text = replace_once(text, old_switch_to, new_switch_to, "switch_to handoff")

    text = replace_once(
        text,
        '''fn switch_to_kernel() -> ! {
    let _kernel = smp_lock::enter();
    let cur = current_index_raw();
''',
        '''fn switch_to_kernel() -> ! {
    let _kernel = smp_lock::enter();
    complete_switch_handoff();
    let cpu_id = local_cpu();
    let cur = current_index_raw();
''',
        "switch_to_kernel drain",
    )

    old_kernel = '''        account_slice_end(&mut *ptr);
        deactivate_task_space(&*ptr, local_cpu());
        // BOUCHAUD_P0_CTX_EN_VOL_V1 : meme regle, meme raison -- un reveil
        // concurrent peut republier cette tache avant notre `switch_context`.
        marque_contexte_en_vol(&mut *ptr);
        if (*ptr).state == TaskState::Zombie {
            SWITCH_PENDING[local_cpu()].store(cur, Ordering::Release);
        } else {
            (*ptr).on_cpu = -1;
        }
        ptr
'''
    new_kernel = '''        account_slice_end(&mut *ptr);
        deactivate_task_space(&*ptr, cpu_id);
        prepare_switch_handoff(cur, &mut *ptr, cpu_id);
        ptr
'''
    text = replace_once(text, old_kernel, new_kernel, "switch_to_kernel handoff")

    old_preempt = '''        usermode::fxsave((*from_ptr).fpu_ptr() as *mut u8);
        (*from_ptr).fs_base = usermode::fs_base();
        deactivate_task_space(&*from_ptr, cpu_id);
        // BOUCHAUD_P0_CTX_EN_VOL_V1 : la fenetre est ici la PLUS large des
        // trois -- `drop(kernel)` rend le verrou plusieurs instructions avant
        // `switch_context`.
        marque_contexte_en_vol(&mut *from_ptr);
        (*from_ptr).on_cpu = -1;
        (*from_ptr).last_cpu = cpu_id as u8;
        (*from_ptr).runq_cpu = cpu_id as u8;
        if (*from_ptr).state == TaskState::Ready {
            if let Some(id) = crate::arch::x86_64::cpu_local::CpuId::from_index(cpu_id) {
                crate::arch::x86_64::cpu_local::local(id).enqueue(cur);
            }
        }
        mark_task_running(&mut *to_ptr, cpu_id);
'''
    new_preempt = '''        usermode::fxsave((*from_ptr).fpu_ptr() as *mut u8);
        (*from_ptr).fs_base = usermode::fs_base();
        deactivate_task_space(&*from_ptr, cpu_id);
        prepare_switch_handoff(cur, &mut *from_ptr, cpu_id);
        // Meme si drop(kernel) precede le switch, l'outgoing reste resident.
        mark_task_running(&mut *to_ptr, cpu_id);
'''
    text = replace_once(text, old_preempt, new_preempt, "preempt handoff")

    forbidden = [
        "CONTEXTE_EN_VOL",
        "marque_contexte_en_vol",
        "contexte_publie",
        "complete_retired",
    ]
    for token in forbidden:
        if token in text:
            raise RuntimeError(f"token obsolete encore present: {token}")
    if re.search(r"\bRETIRED\b", text):
        raise RuntimeError("RETIRED obsolete encore present")
    if text.count("prepare_switch_handoff(") < 4:
        raise RuntimeError("prepare_switch_handoff insuffisamment cable")
    if text.count("complete_switch_handoff()") < 6:
        raise RuntimeError("complete_switch_handoff insuffisamment cable")
    return text

def patch_run(text: str) -> str:
    marker = "BOUCHAUD_GATE0_AUTOSTART_V1"
    if marker in text:
        return text
    text = replace_once(
        text,
        '''    [switch]$LadybirdSansChrome,

    # Memoire donnee a la machine.
''',
        '''    [switch]$LadybirdSansChrome,

    # BOUCHAUD_GATE0_AUTOSTART_V1
    # Test reproductible uniquement : ouvre Ladybird M11 automatiquement.
    [switch]$Gate0Autostart,

    # Memoire donnee a la machine.
''',
        "run param",
    )
    old = '''        $lignesScenario = if ($IsLadybirdM9Test) {
            @(
                'echo "=== Ladybird M9 : HTTP distant via RequestServer ==="',
                'export BO_AUTOSTART_BROWSER=1',
                'export BOUCHAUD_M9_TEST=1'
            )
        }
        else {
            @(
                'echo "=== Bouchaud OS : bureau, navigateur au double-clic ==="',
                'echo "Navigateur : double-clic sur l icone, ou menu Demarrer"'
            )
        }
'''
    new = '''        $lignesScenario = if ($IsLadybirdM9Test) {
            @(
                'echo "=== Ladybird M9 : HTTP distant via RequestServer ==="',
                'export BO_AUTOSTART_BROWSER=1',
                'export BOUCHAUD_M9_TEST=1'
            )
        }
        elseif ($Gate0Autostart) {
            @(
                'echo "=== GATE0 : autostart Ladybird M11 complet ==="',
                'export BO_AUTOSTART_BROWSER=1',
                'export BOUCHAUD_GATE0=1'
            )
        }
        else {
            @(
                'echo "=== Bouchaud OS : bureau, navigateur au double-clic ==="',
                'echo "Navigateur : double-clic sur l icone, ou menu Demarrer"'
            )
        }
'''
    return replace_once(text, old, new, "run autorun")

def main():
    for p in (THREAD, RUN, TEST_SRC):
        if not p.is_file():
            raise SystemExit(f"fichier absent: {p}")
    write(THREAD, patch_thread(read(THREAD)))
    write(RUN, patch_run(read(RUN)))
    shutil.copyfile(TEST_SRC, TEST_DST)
    print("[GATE0] handoff post-switch applique")
    print("[GATE0] autostart M11 de validation applique")
    print("[GATE0] harnais SMP final installe")

if __name__ == "__main__":
    main()
