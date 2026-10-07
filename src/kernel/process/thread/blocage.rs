/// Reveille les taches d'un processus qui dorment, pour qu'elles constatent
/// un signal en attente.
pub fn wake_for_signal(pid: u32) {
    for index in 0..tasks().len() {
        let registre = tasks();
        let task = &registre[index];
        if task.process.pid == pid
            && task.state.reveille()
        {
            // BOUCHAUD_REVEIL_SANS_EFFACER_LA_CLE_V1 : ni la cle d'attente ni
            // l'echeance ne sont effacees ici -- la tache peut s'etre deja
            // reparquee avec les SIENNES. Voir `wake_wait_queue`. Chaque
            // attente efface ses propres champs en reprenant la main.
            //
            // `waiting_for_child` non plus : `sys_wait4` le pose et l'efface
            // lui-meme au reveil. L'effacer ICI, apres la transition, peut
            // tomber sur un parent deja revenu dans `wait4` et reparque avec
            // `waiting_for_child = true` : `notify_parent_of_exit` exige ce
            // drapeau, la sortie du fils ne le reveillerait plus. Un drapeau
            // perime sur une tache `Ready` est sans effet (il faut encore
            // `Blocked -> Ready`). (`futex_key` n'est plus jamais pose.)
            task.futex_key.range(0);
            publish_ready(index);
        }
    }
}

// BOUCHAUD_FINAL_V12_DETACHED_WAIT
//
// Preparation is under the WaitQueue BKL guard. Finish starts only after that
// guard has been dropped, so no WaitQueue-owned KernelGuard spans schedule().

/// Issue d'une publication de parking.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Parking {
    /// `Blocked` publie : l'appelant relit sa condition, puis dort.
    Gare,
    /// La tache n'etait plus prete : elle ne dort pas.
    Refuse,
    /// Attente interruptible d'une tache condamnee : l'appelant libere ce
    /// qu'il tient ET meurt (`meurt_au_parking`), sans dormir.
    Condamnee,
}

/// Publie `Blocked` pour la tache courante. Le coeur commun de toutes les
/// attentes (`kernel::cycle_vie`) :
///
///   1. l'interruptibilite est posee AVANT l'etat, pour qu'un tueur qui lit
///      `Blocked` la lise aussi ;
///   2. `Ready -> Blocked` par CAS : une tache qui n'est plus prete ne dort
///      pas ;
///   3. la condamnation est relue APRES la publication : c'est la moitie
///      « tache » du protocole croise avec `condamne`.
fn publie_parking(interruptible: bool) -> Parking {
    let task = current();
    task.attente_interruptible.range(interruptible);
    if !task.state.endort() {
        ENDORMISSEMENTS_REFUSES.fetch_add(1, Ordering::Relaxed);
        task.attente_interruptible.range(false);
        return Parking::Refuse;
    }
    if crate::kernel::cycle_vie::meurt_au_parking(task.condamnee.est_condamnee(), interruptible) {
        return Parking::Condamnee;
    }
    Parking::Gare
}

/// La tache courante, condamnee, meurt dans son attente interruptible au lieu
/// d'y dormir. Ne revient pas.
pub(crate) fn meurt_au_parking() -> ! {
    MORTES_AU_PARKING.fetch_add(1, Ordering::Relaxed);
    meurt_soi_meme(current());
    retire_exec_zombie_current()
}

/// Une attente d'appel systeme interruptible hors WaitQueue (`wait4`) :
/// publie `Blocked`. Faux : ne pas dormir (tache morte). Une tache condamnee
/// ne revient pas.
pub fn publie_attente_interruptible() -> bool {
    match publie_parking(true) {
        Parking::Gare => true,
        Parking::Refuse => false,
        Parking::Condamnee => meurt_au_parking(),
    }
}

/// Fin d'une attente : l'attente n'est plus interruptible, et un parking
/// encore publie (echeance, reveil parasite de `schedule`) est annule.
pub fn termine_attente() {
    let task = current();
    task.attente_interruptible.range(false);
    task.state.reveille();
}

pub(crate) fn prepare_park_current_on_detached(
    wait_queue_key: usize,
    deadline_ns: Option<u64>,
    interruptible: bool,
) -> Parking {
    // L'assertion « sous gros verrou » a disparu parce que la PRECONDITION a
    // disparu, non pour faire passer un controle : ce chemin est justement
    // celui qu'on sort du verrou. Ce qui reste vrai, et qui compte, est que
    // seule la tache COURANTE se gare elle-meme -- personne d'autre n'a le
    // droit de la declarer bloquee.
    debug_assert!(
        current_index_raw() != NO_TASK,
        "task: parking demande hors de toute tache"
    );

    // BOUCHAUD_P15_BROWSER_HANG_FORENSICS
    forensic_wait_begin_if_idle(
        WAIT_WAIT_QUEUE,
        wait_queue_key as u64,
        deadline_ns.unwrap_or(0),
    );
    {
        let task = current();
        task.wait_queue_key.range(wait_queue_key);
        task.wake_deadline_ns.range(deadline_ns.unwrap_or(0));
    }
    let issue = publie_parking(interruptible);
    if issue != Parking::Gare {
        let task = current();
        task.wait_queue_key.range(0);
        task.wake_deadline_ns.range(0);
        forensic_wait_clear(WAIT_WAIT_QUEUE);
        return issue;
    }

    if let Some(deadline) = deadline_ns {
        arme_echeance(deadline);
    }
    Parking::Gare
}

/// Annule un parking publie mais pas encore effectif.
///
/// Le protocole sans gros verrou publie `Blocked` AVANT de relire la
/// generation de la file. Quand cette relecture montre qu'un reveil est deja
/// passe, il faut defaire la publication -- sinon la tache resterait bloquee
/// en attendant un reveil qui a deja eu lieu.
pub(crate) fn annule_park_courant() {
    let task = current();
    task.wait_queue_key.range(0);
    task.wake_deadline_ns.range(0);
    task.attente_interruptible.range(false);
    // `Blocked -> Ready` ; sans effet si un reveilleur l'a deja fait.
    task.state.reveille();
    // BOUCHAUD_P15_BROWSER_HANG_FORENSICS
    forensic_wait_clear(WAIT_WAIT_QUEUE);
}

/// Returns `(notified_before_deadline, number_of_schedule_loops)`.
pub(crate) fn finish_park_current_on_detached(
    deadline_ns: Option<u64>,
) -> (bool, u64) {
    let mut loops = 0u64;

    loop {
        // Lecture d'un champ ATOMIQUE de notre PROPRE tache : le gros verrou
        // n'y apportait rien. Il etait pris, relache, et repris a chaque tour
        // de cette boucle d'attente -- des milliers de fois par seconde, pour
        // une seule instruction de chargement.
        let blocked = current().state == TaskState::Blocked;

        if !blocked {
            break;
        }

        loops = loops.saturating_add(1);
        schedule();
    }

    let notified = match deadline_ns {
        Some(deadline) => crate::kernel::timer::monotonic_ns() < deadline,
        None => true,
    };

    {
        // Deux ecritures atomiques sur notre propre tache. Personne d'autre ne
        // les lit pour decider quoi que ce soit a cet instant : la tache est en
        // train de reprendre la main, donc elle n'est plus candidate au reveil.
        let task = current();
        task.wait_queue_key.range(0);
        task.wake_deadline_ns.range(0);
        task.attente_interruptible.range(false);
    }
    // BOUCHAUD_P15_BROWSER_HANG_FORENSICS
    forensic_wait_clear(WAIT_WAIT_QUEUE);

    (notified, loops)
}

/// Endort la tache courante sur une WaitQueue. L'appelant doit avoir valide la
/// generation juste avant cet appel pour fermer le lost wakeup.
pub(crate) fn park_current_on(wait_queue_key: usize) {
    // BOUCHAUD_P15_BROWSER_HANG_FORENSICS
    forensic_wait_begin_if_idle(WAIT_WAIT_QUEUE, wait_queue_key as u64, 0);
    current().wait_queue_key.range(wait_queue_key);
    if publie_parking(false) == Parking::Gare {
        while current().state == TaskState::Blocked {
            schedule();
        }
    }
    current().wait_queue_key.range(0);
    // BOUCHAUD_P15_BROWSER_HANG_FORENSICS
    forensic_wait_clear(WAIT_WAIT_QUEUE);
}

/// Endort la tache sur une WaitQueue jusqu'a notification ou echeance.
///
/// Le deadline partage le mecanisme de `sleep_ticks`: l'IRQ timer remet la
/// tache Ready. La cle de queue reste posee jusqu'au reveil, de sorte qu'une
/// notification et l'echeance puissent courir sans perdre le reveil.
pub(crate) fn park_current_on_until(wait_queue_key: usize, deadline_ns: u64) -> bool {
    // BOUCHAUD_P15_BROWSER_HANG_FORENSICS
    forensic_wait_begin_if_idle(WAIT_WAIT_QUEUE, wait_queue_key as u64, deadline_ns);
    {
        let task = current();
        task.wait_queue_key.range(wait_queue_key);
        task.wake_deadline_ns.range(deadline_ns);
    }
    if publie_parking(false) == Parking::Gare {
        arme_echeance(deadline_ns);
        while current().state == TaskState::Blocked {
            schedule();
        }
    }
    let notified = crate::kernel::timer::monotonic_ns() < deadline_ns;
    let task = current();
    task.wait_queue_key.range(0);
    task.wake_deadline_ns.range(0);
    // BOUCHAUD_P15_BROWSER_HANG_FORENSICS
    forensic_wait_clear(WAIT_WAIT_QUEUE);
    notified
}

/// Reveille au plus `limit` taches inscrites sur la queue.
/// Reveille jusqu'a `limit` taches arretees sur cette file. SANS GROS VERROU.
///
/// # Ce qui a rendu le verrou inutile
///
/// Cette boucle le prenait pour deux raisons, et les deux ont disparu.
///
/// La table pouvait se REALLOUER sous les pieds du lecteur : le registre a
/// maintenant des emplacements a adresse stable, lus sans verrou.
///
/// Le « lire l'etat, decider, ecrire l'etat » n'etait atomique que grace a
/// lui : deux CPU reveillant la meme tache l'auraient vue bloquee tous les
/// deux, et l'auraient mise deux fois en file d'execution. C'est desormais un
/// `compare_exchange` -- exactement un gagnant, par construction.
///
/// # Reveil superflu, et pourquoi il est acceptable
///
/// La cle est lue avant la transition. Une tache qui changerait de file entre
/// les deux serait reveillee pour rien. C'est sans danger : toute attente du
/// noyau reverifie sa condition en reprenant la main -- c'est la forme
/// `while etat == Blocked { schedule() }`. Rendre ce cas impossible couterait
/// un verrou par tache, pour supprimer un reveil rare et inoffensif.
pub(crate) fn wake_wait_queue(wait_queue_key: usize, limit: usize) -> usize {
    let mut woke = 0;
    for index in 0..registre_longueur() {
        if woke == limit {
            break;
        }
        let Some(tache) = registre_tache(index) else { continue };
        if tache.wait_queue_key != wait_queue_key {
            continue;
        }
        // Le gagnant du compare_exchange est le seul a poursuivre.
        if !tache.state.reveille() {
            continue;
        }
        // BOUCHAUD_REVEIL_SANS_EFFACER_LA_CLE_V1
        //
        // Le reveilleur n'efface PLUS la cle d'attente. Il le faisait APRES
        // la transition, et cette ecriture tardive pouvait tomber sur l'attente
        // SUIVANTE de la meme tache :
        //
        //   tache    publie Blocked (cle Q), n'est pas encore sortie du coeur
        //   reveil   Blocked -> Ready gagne
        //   tache    voit Ready, sort de sa boucle, repart, n'a pas son tour,
        //            se reparque : cle Q, Blocked
        //   reveil   efface la cle -> la tache est Blocked avec cle 0
        //
        // Plus aucun reveil ne la retrouve (la recherche se fait par cle), et
        // la mise en file qui suit est jetee (tache non eligible). Observe :
        // endurance SMP4, verrou du controleur ATA -- le lecteur dont c'etait
        // le tour `Blocked cle_attente=0`, tous les autres parques derriere
        // lui, neuf minutes de machine figee.
        //
        // Chaque attente efface deja sa propre cle en reprenant la main
        // (`finish_park_current_on_detached`, `park_current_on`,
        // `park_current_on_until`, `annule_park_courant`). Une cle perimee sur
        // une tache Ready est sans effet : tout reveil exige Blocked -> Ready.
        publish_ready(index);
        woke += 1;
    }
    woke
}

/// Y a-t-il un signal livrable pour la tache courante ?
///
/// Consulte par les attentes bloquantes (`poll`, `wait4`, futex) : une attente
/// sans limite de temps doit pouvoir etre interrompue par un signal.
pub fn signal_pending() -> bool {
    match try_current() {
        Some(task) => task.process.signals.lock().next_deliverable().is_some(),
        None => false,
    }
}

/// Un signal en attente doit-il INTERROMPRE une attente de disponibilite
/// (`poll`, `select`, `epoll_wait`) par `EINTR` ?
///
/// BOUCHAUD_SIGNAL_INTERROMPT_POLL_V1
///
/// Oui s'il est livrable (non bloque) ET qu'il fera quelque chose : un
/// gestionnaire installe, ou une action par defaut qui termine. Un signal
/// ignore (`SIG_IGN`, ou `SIGCHLD` sans gestionnaire) n'interrompt rien :
/// Linux ne le met meme pas en file, et un `EINTR` pour lui serait un faux
/// reveil que bien des programmes traitent en erreur.
///
/// Run 37584587000 : le WebContent d'un onglet meurt, le navigateur n'en sait
/// jamais rien. LibCore attend en `poll(-1)` et n'apprend la mort d'un fils
/// QUE par son gestionnaire de SIGCHLD ; ce `poll` ne rendait jamais la main
/// pour un signal (sigchld-multifil-probe, cas `attente` : pere fige plus de
/// huit minutes apres la mort de son fils, meme l'alarme de 10 s ignoree).
pub fn signal_interrompt_attente() -> bool {
    // Le cas courant (aucun signal) reste un simple test de bits : cette
    // fonction est appelee a chaque reveil de `poll`. Le parcours du
    // registre n'a lieu que si un signal attend vraiment.
    if !signal_a_effet_en_attente() {
        return false;
    }
    // BOUCHAUD_SIGNAL_FIL_PRINCIPAL_V1 : le signal attend le fil principal ;
    // un autre fil en `poll` ne doit pas en sortir en boucle sur EINTR.
    !laisse_le_signal_au_fil_principal()
}

/// Un signal livrable et qui fera quelque chose est-il en attente ?
pub fn signal_a_effet_en_attente() -> bool {
    let Some(task) = try_current() else { return false };
    let signals = task.process.signals.lock();
    let mut prets = signals.pending & !signals.blocked;
    while prets != 0 {
        let signal = prets.trailing_zeros() + 1;
        prets &= prets - 1;
        if signal == crate::kernel::signal::SIGKILL || signal == crate::kernel::signal::SIGSTOP {
            return true;
        }
        let action = signals.actions[signal as usize - 1];
        if action.handler == crate::kernel::signal::SIG_IGN {
            continue;
        }
        if action.handler == crate::kernel::signal::SIG_DFL
            && matches!(crate::kernel::signal::default_action(signal), crate::kernel::signal::DefaultAction::Ignore)
        {
            continue;
        }
        return true;
    }
    false
}

/// Le fil COURANT doit-il laisser un signal de processus en attente au fil
/// principal ?
///
/// BOUCHAUD_SIGNAL_FIL_PRINCIPAL_V1
///
/// Un signal adresse au PROCESSUS (kill, SIGCHLD, alarme) peut, selon POSIX,
/// echoir a n'importe quel fil qui ne le bloque pas. Linux, lui, le donne au
/// fil PRINCIPAL des qu'il peut le prendre (`complete_signal`), et LibCore en
/// depend : `EventLoopManagerUnix::handle_signal` ecrit dans le tube de
/// reveil du FIL QUI RECOIT le signal, et rend la main sans rien faire si ce
/// fil n'a pas de boucle d'evenements. Livre au premier fil qui rentrait en
/// mode utilisateur -- dans le navigateur, un fil de travail toujours actif,
/// jamais le fil principal endormi en `poll` -- le SIGCHLD etait perdu
/// (sigchld-multifil-probe, cas `boucle` : 10,3 s, reveil par l'alarme ; run
/// 37589903681 : aucun [LB] SIGCHLD_RECU, aucune reprise de WebContent).
///
/// Fil principal : le plus ancien fil vivant du processus (plus petit tid).
/// On le lui laisse s'il le prendra BIENTOT : endormi dans une attente de
/// disponibilite (il sera reveille, `poll` rendra EINTR), ou dans le noyau
/// (il repasse par la fin d'appel systeme, ou la livraison a lieu). S'il
/// calcule en mode utilisateur ou dort ailleurs (futex, sommeil), le fil
/// courant livre, comme avant : aucun signal n'attend indefiniment.
pub fn laisse_le_signal_au_fil_principal() -> bool {
    let Some(courant) = try_current() else { return false };
    let pid = courant.process.pid;
    let registre = tasks();
    let mut principal: Option<usize> = None;
    for index in 0..registre.len() {
        let tache = &registre[index];
        if tache.process.pid != pid || tache.state == TaskState::Zombie {
            continue;
        }
        if principal.map_or(true, |i| tache.tid < registre[i].tid) {
            principal = Some(index);
        }
    }
    let Some(index) = principal else { return false };
    let fil = &registre[index];
    if fil.tid == courant.tid {
        return false;
    }
    match fil.state.charge() {
        TaskState::Blocked => fil.wait_queue_key.charge() as usize == crate::kernel::fd::readiness_cle(),
        TaskState::Ready => fil.in_kernel.charge(),
        TaskState::Zombie => false,
    }
}

/// Un signal vient d'etre pose sur `pid` : reveiller ses fils endormis ET
/// les attentes de disponibilite, pour qu'elles le voient.
///
/// `wake_for_signal` remet `Ready` les fils DEJA bloques. Un fil entre son
/// balayage de descripteurs et son parking ne l'est pas encore : sans
/// changement de generation de la file de disponibilite, il se garerait
/// apres le signal et dormirait pour rien. `notify_readiness` change cette
/// generation (`WaitQueue` : relecture du ticket apres publication).
pub fn reveille_pour_signal(pid: u32) {
    wake_for_signal(pid);
    crate::kernel::fd::notify_readiness();
}

/// Termine de force toutes les taches (utilise apres une faute fatale).
pub fn kill_all(code: i32) {
    for task in tasks().iter() {
        marque_zombie(task);
        task.process.lifecycle.lock().exit_code = code;
    }
}
