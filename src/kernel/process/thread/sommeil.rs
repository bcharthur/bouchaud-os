// --- Sommeil -----------------------------------------------------------------

/// Endort la tache courante pendant `ticks` ticks du timer.

/// Attente d'une boucle `poll`/`select`/`epoll_wait` qui n'a rien trouve de pret.
///
/// ## Le defaut que cette fonction corrige
///
/// Les trois boucles faisaient, chacune a sa facon :
///
/// ```text
/// if task::schedule() { continue; }        // une autre tache est prete
/// cpu::wait_for_interrupt();               // personne d'autre : on dort
/// ```
///
/// Le raisonnement est juste tache par tache, et faux pour la machine. Un
/// bureau Bouchaud avec le navigateur ouvert compte quatre processus — le
/// compositeur, le bootstrap, WebContent, RequestServer — et quand ils
/// attendent tous, ils attendent **tous dans `poll`**. Chacun reste alors
/// `Ready` du point de vue de l'ordonnanceur : `schedule()` trouve toujours un
/// autre candidat, rend `true`, et personne n'atteint jamais le `hlt`. Les
/// quatre se relaient a plein regime pour ne rien faire.
///
/// C'est ce qui affichait 100 % de processeur des le demarrage, avant meme
/// qu'une page soit demandee, et ce que le releve par processus montrait sans
/// qu'on le lise ainsi : la somme des parts valait le cœur entier alors
/// qu'aucun des quatre ne progressait.
///
/// ## Ce qu'elle fait
///
/// Elle marque la tache **bloquee** pour un tick, ce que `sleep_ticks` sait
/// deja faire. Les autres taches reellement pretes continuent d'etre elues ;
/// quand plus aucune ne l'est, `schedule()` tombe sur son propre
/// `wait_for_interrupt()` et le processeur s'arrete pour de bon — ce que la
/// mesure de charge compte enfin comme du repos.
///
/// La latence ne change pas : le tick vaut une milliseconde, exactement le
/// delai qu'imposait deja le `hlt` reveille par l'horloge. Le reveil logiciel
/// n'est pas perdu non plus — une tache qui rend un descripteur pret pendant
/// notre tick le trouvera pret a notre reveil, un tour de boucle plus tard.
// BOUCHAUD_CPU_OPT_ADAPTIVE_IO: attente adaptative pour poll/select/epoll.
//
// La pile reseau Bouchaud reste aujourd'hui pilotee par interrogation. Bloquer
// sans limite ici serait donc incorrect : personne ne pomperait les paquets.
// On garde une phase tres reactive a 1 ms, puis on espace progressivement les
// tours vides jusqu'a 8 ms. Une I/O qui repart quitte son syscall et le compteur
// local repart automatiquement de zero a l'appel suivant.
pub fn attends_io_adaptatif(tours_vides: &mut u32) {
    let ticks = match *tours_vides {
        0..=2 => 1,
        3..=7 => 2,
        8..=15 => 4,
        _ => 8,
    };
    *tours_vides = tours_vides.saturating_add(1).min(64);
    sleep_ticks(ticks);
}

/// Attente historique stricte d'un tick. Conservee pour les chemins ou la
/// latence minimale prime (console, audio, petites attentes protocolaires).
pub fn attends_un_tick() {
    sleep_ticks(1);
}

pub fn sleep_ticks(ticks: u64) {
    debug_assert!(
        smp_lock::held_by_current_cpu(),
        "task: sleep_ticks requiert le BKL externe de l'appelant"
    );
    // BOUCHAUD_P0_CONTRAT_PROFONDEUR_V1 : voir `verifie_profondeur_rendue`.
    let profondeur_entree = smp_lock::profondeur_locale();
    let duration_ns = ticks.max(1)
        .saturating_mul(1_000_000_000 / crate::kernel::timer::TICKS_PER_SECOND);
    let deadline = crate::kernel::timer::monotonic_ns().saturating_add(duration_ns);
    {
        let task = current();
        task.wake_deadline_ns.range(deadline);
        task.state.range(TaskState::Blocked);
    }
    arme_echeance(deadline);

    // `syscall_dispatch` conserve un BKL externe. Le suspendre ici, avant
    // meme de savoir si schedule() trouvera une autre tache, ferme le cas ou
    // wake_sleepers() remet la tache courante Ready parce que son court delai
    // a deja expire : schedule() ne dort alors pas et ne suspendrait sinon que
    // son propre niveau recursif, laissant le niveau syscall acquis pendant
    // toute la boucle poll/select/epoll.
    let outer_depth = smp_lock::suspend_for_schedule();
    while crate::kernel::timer::monotonic_ns() < deadline {
        // schedule() fait deja HLT si la tache est bloquee et seule.
        schedule();
        // Meme lecture atomique que la boucle d'attente detachee : le gros
        // verrou etait pris et relache a chaque tour pour un seul chargement.
        let ready = current().state == TaskState::Ready;
        if ready {
            break;
        }
    }
    smp_lock::resume_after_schedule(outer_depth);
    verifie_profondeur_rendue("sleep_ticks", profondeur_entree);
    let task = current();
    task.wake_deadline_ns.range(0);
    task.state.range(TaskState::Ready);
}

/// Reveille les taches dont le sommeil est echu, et declenche les `SIGALRM`.
// BOUCHAUD_SCHED_ECHEANCE_HINT_V1
//
// # Ce que `wake_sleepers` coutait
//
// `schedule()` l'appelle a CHAQUE tour. Une tache bloquee -- un fil arrete sur
// un futex, un `poll` en attente -- reste dans sa boucle `while Blocked
// { schedule() }` : elle s'endort au `hlt`, le tick la reveille une
// milliseconde plus tard, elle reprend le gros verrou, appelle `schedule()`,
// qui balaie TOUTE la table des taches, puis se rendort.
//
// Avec quatre fils de Ladybird bloques -- ce que `[SMP-STALL]` montrait, deux
// CPU dans `futex` pendant cent secondes d'affilee --, cela fait quatre mille
// balayages complets par seconde, chacun sous le gros verrou, pour ne rien
// trouver. C'est ce que mesurait `bkl_wait_delta_ns` : sept secondes d'attente
// du verrou par fenetre de cinq.
//
// # Le raccourci
//
// Une borne INFERIEURE de la plus proche echeance. Tant que l'heure ne l'a pas
// atteinte, aucune tache ne peut etre due, et le balayage est inutile.
//
// Le sens de l'inegalite est ce qui rend le raccourci sur : la borne peut etre
// TROP TOT -- on balaie pour rien, ce qui coute mais ne perd rien --, jamais
// trop tard. Chaque pose d'echeance la ramene vers le passe par `fetch_min`,
// et chaque balayage reel la recalcule exactement.
static PROCHAINE_ECHEANCE: crate::kernel::echeances::Echeances =
    crate::kernel::echeances::Echeances::neuve();

/// Declare une echeance. A appeler POUR CHAQUE `wake_deadline_ns` non nul.
///
/// `tools/verifie-echeances.py` refuse une ecriture qui ne passerait pas par
/// ici : une echeance inconnue du raccourci ne serait jamais servie.
pub(crate) fn arme_echeance(deadline_ns: u64) {
    PROCHAINE_ECHEANCE.arme(deadline_ns);
}

/// Recalcule la borne exactement, apres un balayage.
fn recalcule_echeance() {
    let mut minimum = crate::kernel::echeances::JAMAIS;
    for index in 0..tasks().len() {
        let echeance = tasks()[index].wake_deadline_ns.charge();
        if tasks()[index].state == TaskState::Blocked && echeance != 0 && echeance < minimum {
            minimum = echeance;
        }
    }
    // Les alarmes vivent en TICKS ; la borne est en nanosecondes. Convertir --
    // et non comparer les deux tels quels, ce qui est precisement l'erreur que
    // `fire_alarms` portait.
    let par_tick = 1_000_000_000 / crate::kernel::timer::TICKS_PER_SECOND;
    for (_, echeance_ticks) in alarms().iter() {
        let echeance = echeance_ticks.saturating_mul(par_tick);
        if echeance < minimum {
            minimum = echeance;
        }
    }
    PROCHAINE_ECHEANCE.recale(minimum);
}

fn wake_sleepers() {
    let now = crate::kernel::timer::monotonic_ns();
    // Le raccourci : une charge atomique au lieu d'un balayage complet.
    if !PROCHAINE_ECHEANCE.doit_balayer(now) {
        return;
    }
    for index in 0..tasks().len() {
        if tasks()[index].state == TaskState::Blocked
            && tasks()[index].wake_deadline_ns != 0
            && now >= tasks()[index].wake_deadline_ns.charge()
        {
            tasks()[index].wake_deadline_ns.range(0);
            tasks()[index].futex_key.range(0);
            tasks()[index].state.range(TaskState::Ready);
            publish_ready(index);
        }
    }
    fire_alarms(crate::kernel::timer::ticks());
    recalcule_echeance();
}

/// Echeance du prochain `SIGALRM` par processus : (pid, tick).
static mut ALARMS: Option<Vec<(u32, u64)>> = None;

fn alarms() -> &'static mut Vec<(u32, u64)> {
    unsafe {
        if ALARMS.is_none() {
            ALARMS = Some(Vec::new());
        }
        ALARMS.as_mut().unwrap()
    }
}

/// Programme (ou annule, avec 0) l'alarme du processus courant.
/// Renvoie l'echeance precedente, 0 s'il n'y en avait pas.
pub fn set_alarm(deadline: u64) -> u64 {
    let pid = current().process.pid;
    let list = alarms();
    let previous = list
        .iter()
        .find(|(p, _)| *p == pid)
        .map(|(_, t)| *t)
        .unwrap_or(0);
    list.retain(|(p, _)| *p != pid);
    if deadline != 0 {
        list.push((pid, deadline));
        // La borne est en nanosecondes, l'alarme en ticks.
        let par_tick = 1_000_000_000 / crate::kernel::timer::TICKS_PER_SECOND;
        arme_echeance(deadline.saturating_mul(par_tick));
    }
    previous
}

/// Echeance de l'alarme du processus courant (0 s'il n'y en a pas).
pub fn peek_alarm() -> u64 {
    let pid = current().process.pid;
    alarms()
        .iter()
        .find(|(p, _)| *p == pid)
        .map(|(_, t)| *t)
        .unwrap_or(0)
}

/// Leve les `SIGALRM` dont l'echeance est atteinte.
// BOUCHAUD_SCHED_ALARME_UNITE_V1
//
// Les alarmes sont posees en TICKS -- `sys_alarm` et `sys_setitimer` ecrivent
// `timer::ticks() + n` --, et `fire_alarms` recevait des NANOSECONDES. Une
// nanoseconde vaut un millionieme de tick : `now >= deadline` etait donc vrai
// des le premier appel, et tout `alarm(60)` livrait son `SIGALRM`
// immediatement.
//
// Rien ne le signalait : un signal livre trop tot ressemble a un signal livre.
// La fonction prend desormais des ticks, comme les valeurs qu'elle compare.
fn fire_alarms(maintenant_ticks: u64) {
    let expired: Vec<u32> = alarms()
        .iter()
        .filter(|(_, deadline)| maintenant_ticks >= *deadline)
        .map(|(pid, _)| *pid)
        .collect();
    if expired.is_empty() {
        return;
    }
    alarms().retain(|(_, deadline)| maintenant_ticks < *deadline);
    for pid in expired {
        if let Some(process) = process_by_pid(pid) {
            process.signals.lock().raise(crate::kernel::signal::SIGALRM);
        }
        wake_for_signal(pid);
    }
}

/// Cede le CPU une fois (`sched_yield`).
pub fn yield_now() {
    schedule();
}

