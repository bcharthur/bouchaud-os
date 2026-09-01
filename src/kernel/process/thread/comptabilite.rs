/// Avance une seule fois le curseur CPU de la tâche jusqu'à `now`.
///
/// Toutes les frontières (syscall, préemption, blocage) utilisent le même
/// curseur. Une seconde frontière au même instant voit donc un delta nul au
/// lieu de recompter la tranche précédente.
// Ne touche plus que des atomiques : une reference partagee suffit.
fn account_until(task: &Task, now: u64) {
    if task.last_account_ns == 0 {
        return;
    }
    debug_assert!(task.on_cpu >= 0, "task: accounting armé pour une tâche hors CPU tid={}", task.tid);
    let cpu = local_cpu();

    // Ce que les frontieres d'appel systeme ont accumule depuis le dernier
    // repli, plus le fragment encore ouvert.
    let mut user = COMPTA_USER_NS[cpu].swap(0, Ordering::Relaxed);
    let mut noyau = COMPTA_NOYAU_NS[cpu].swap(0, Ordering::Relaxed);
    let debut = COMPTA_DEBUT_NS[cpu].load(Ordering::Relaxed);
    if debut != 0 {
        let fragment = now.saturating_sub(debut);
        if COMPTA_EN_NOYAU[cpu].load(Ordering::Relaxed) {
            noyau = noyau.saturating_add(fragment);
        } else {
            user = user.saturating_add(fragment);
        }
    }
    let elapsed = user.saturating_add(noyau);

    task.user_cpu_ns.range(task.user_cpu_ns.charge().saturating_add(user));
    task.kernel_cpu_ns.range(task.kernel_cpu_ns.charge().saturating_add(noyau));
    task.cpu_ns[cpu].range(task.cpu_ns[cpu].charge().saturating_add(elapsed));
    // EWMA 7/8 historique + 1/8 dernière tranche: stable mais réactif en
    // quelques quanta, sans utiliser les ticks comme unité.
    // Moyenne glissante : sept huitiemes de l'ancienne, un huitieme de la
    // derniere tranche. Lire puis ecrire n'a pas besoin d'etre atomique dans
    // son ensemble -- seule la tache elle-meme met a jour son temps recent, et
    // les lecteurs ne s'en servent que pour une heuristique de vol.
    task.recent_runtime_ns.range(
        task.recent_runtime_ns
            .charge()
            .saturating_mul(7)
            .saturating_add(elapsed)
            / 8,
    );
    // `in_kernel` doit survivre a un changement de contexte AU MILIEU d'un
    // appel systeme : une tache qui se bloque dans un `futex` repart du cote
    // noyau. On le range donc dans la tache au repli, et `mark_task_running` le
    // ressort au reveil.
    task.in_kernel.range(COMPTA_EN_NOYAU[cpu].load(Ordering::Relaxed));
    COMPTA_DEBUT_NS[cpu].store(now, Ordering::Relaxed);
    task.last_account_ns.range(now);
    task.slice_start_ns.range(now);
}

/// Marque une tache zombie, et previent le CPU sur lequel elle tourne.
///
/// Point de passage unique : `retire_current_if_zombie` s'execute a la sortie
/// de chaque appel systeme, et son chemin commun ne doit plus consulter la
/// table des taches. Il lit un drapeau par CPU ; c'est ici qu'on le pose.
///
/// Tous les appelants tiennent le gros verrou -- c'est ce qui leur permet de
/// tenir une `&mut Task` -- et `on_cpu` designe le CPU ou la tache s'execute,
/// ou -1 si elle n'est nulle part. Une tache qui n'est sur aucun CPU n'a
/// personne a prevenir : elle ne reviendra pas en espace utilisateur.
// Ne touche plus que des atomiques : une reference PARTAGEE suffit.
fn marque_zombie(task: &Task) {
    task.state.range(TaskState::Zombie);
    if task.on_cpu >= 0 && !task.switching_out.charge() {
        let cpu = task.on_cpu.charge() as usize;
        if cpu < MAX_CPUS {
            RETRAITE_DEMANDEE[cpu].store(true, Ordering::Release);
        }
    }
}

// BOUCHAUD_COMPTA_IDLE_V1
//
// LE TEMPS PASSE EN `hlt` N'EST LE TEMPS DE PERSONNE
// --------------------------------------------------
// `account_until` impute a la tache courante tout l'ecart depuis
// `COMPTA_DEBUT_NS[cpu]`. Or la branche idle de `schedule()` execute un `hlt`
// SANS replier la tranche : la tache reste courante sur ce CPU, le curseur
// continue de courir, et le premier repli suivant lui attribue la totalite du
// sommeil.
//
// Ce defaut est ancien, mais il etait invisible tant que le bureau dormait par
// tranches de 4 a 16 ms. Depuis que le compositeur dort jusqu'au prochain
// evenement, ces tranches durent des centaines de millisecondes -- et le
// journal affiche `desktop cpu_pct=100` pendant que la machine ne fait rien.
//
// La comptabilite MACHINE, elle, etait deja juste : `prepare_scheduler_idle`
// pose `IDLE[cpu]` avant le `hlt`, et la charge globale se calcule depuis
// busy/idle. Seule l'imputation PAR TACHE etait fausse.
//
// Replier avant le `hlt` et rearmer apres suffit. On ne peut pas reutiliser
// `mark_task_running` pour rearmer : elle compte une migration, incremente les
// changements de contexte et exige `on_cpu < 0` -- rien de tout cela n'est vrai
// ici, la tache n'a pas quitte son CPU.

/// Replie la tranche de la tache courante avant un `hlt`, si elle est armee.
///
/// Rend `true` s'il faudra rearmer au reveil.
fn suspend_compta_pour_idle() -> bool {
    let index = current_index_raw();
    if index == NO_TASK {
        return false;
    }
    let Some(task) = tasks().get(index) else {
        return false;
    };
    if task.last_account_ns == 0 {
        return false;
    }
    account_slice_end(task);
    true
}

/// Rearme la comptabilite de la tache courante apres un `hlt`.
///
/// Le cote du mur -- noyau ou utilisateur -- est celui que le repli avait
/// range dans la tache : un fil noyau qui dort repart du cote noyau.
fn rearme_compta_apres_idle() {
    let index = current_index_raw();
    if index == NO_TASK {
        return;
    }
    let now = crate::kernel::timer::monotonic_ns();
    let cpu = local_cpu();
    let Some(task) = tasks().get(index) else {
        return;
    };
    task.last_account_ns.range(now);
    task.slice_start_ns.range(now);
    let en_noyau = task.in_kernel.charge();
    COMPTA_DEBUT_NS[cpu].store(now, Ordering::Relaxed);
    COMPTA_USER_NS[cpu].store(0, Ordering::Relaxed);
    COMPTA_NOYAU_NS[cpu].store(0, Ordering::Relaxed);
    COMPTA_EN_NOYAU[cpu].store(en_noyau, Ordering::Relaxed);
}

fn account_slice_end(task: &Task) {
    let now = crate::kernel::timer::monotonic_ns();
    account_until(task, now);
    task.last_account_ns.range(0);
    task.slice_start_ns.range(0);
    // Le CPU n'a plus de tache a qui imputer le temps : on desarme, sinon le
    // premier repli de la tache SUIVANTE lui attribuerait le temps passe entre
    // les deux.
    let cpu = local_cpu();
    COMPTA_DEBUT_NS[cpu].store(0, Ordering::Relaxed);
    COMPTA_USER_NS[cpu].store(0, Ordering::Relaxed);
    COMPTA_NOYAU_NS[cpu].store(0, Ordering::Relaxed);
}

/// Frontières syscall utilisées pour séparer user/kernel sans dépendre du PIT.
/// Frontiere utilisateur -> noyau. Ne touche que le bloc par CPU.
pub fn account_kernel_enter() {
    frontiere_compta(true);
}

/// Frontiere noyau -> utilisateur. Ne touche que le bloc par CPU.
pub fn account_kernel_exit() {
    frontiere_compta(false);
}

/// Ferme le fragment en cours et ouvre le suivant, du cote demande.
///
/// Les interruptions sont coupees : sans cela, le numero de CPU pourrait etre
/// lu ici et les compteurs mis a jour la-bas, apres une migration. C'est le
/// meme raisonnement que pour `identite_courante`, et c'est tout ce qu'il faut
/// -- aucun autre CPU n'ecrit dans notre case.
fn frontiere_compta(vers_noyau: bool) {
    interrupts::without_interrupts(|| {
        let cpu = local_cpu();
        let now = crate::kernel::timer::monotonic_ns();
        let debut = COMPTA_DEBUT_NS[cpu].load(Ordering::Relaxed);
        if debut != 0 {
            let ecoule = now.saturating_sub(debut);
            if COMPTA_EN_NOYAU[cpu].load(Ordering::Relaxed) {
                COMPTA_NOYAU_NS[cpu].fetch_add(ecoule, Ordering::Relaxed);
            } else {
                COMPTA_USER_NS[cpu].fetch_add(ecoule, Ordering::Relaxed);
            }
            COMPTA_DEBUT_NS[cpu].store(now, Ordering::Relaxed);
        }
        COMPTA_EN_NOYAU[cpu].store(vers_noyau, Ordering::Relaxed);
    });
}

pub fn account_resume_user_noreturn() {
    account_kernel_exit();
}

