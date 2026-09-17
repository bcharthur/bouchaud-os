impl Task {
    /// Cree une tache prete a demarrer en ring 3 avec la trame donnee.
    pub fn new(process: Arc<Process>, frame: TrapFrame) -> Box<Task> {
        let kstack = vec![0u8; KSTACK_SIZE];
        let kstack_top = (kstack.as_ptr() as u64 + KSTACK_SIZE as u64) & !0xF;
        // La page de garde occupe le PIED de l'allocation ; la pile utilisable
        // commence apres elle. Voir `GARDE_PILE` et `CANARI_PILE`.
        unsafe { pose_le_canari(kstack.as_ptr() as u64) };
        let fpu = vec![0u8; 512 + 16];
        let fpu_area = (fpu.as_ptr() as u64 + 15) & !0xF;
        unsafe {
            let area = fpu_area as *mut u8;
            core::ptr::copy_nonoverlapping(0x037Fu16.to_le_bytes().as_ptr(), area, 2);
            core::ptr::copy_nonoverlapping(0x1F80u32.to_le_bytes().as_ptr(), area.add(24), 4);
            core::ptr::copy_nonoverlapping(0x0000_FFBFu32.to_le_bytes().as_ptr(), area.add(28), 4);
        }

        let mut task = Box::new(Task {
            tid: alloc_tid(),
            process,
            state: EtatAtomique::neuf(TaskState::Ready),
            priorite: PrioriteAtomique::neuve(Priorite::Normale),
            affinity_mask: 0,
            runq_cpu: CoeurAtomique::neuf(u8::MAX),
            last_cpu: CoeurAtomique::neuf(u8::MAX),
            on_cpu: CoeurSigneAtomique::neuf(-1),
            switching_out: DrapeauAtomique::neuf(false),
            last_migration_ns: EcheanceAtomique::neuf(0),
            recent_runtime_ns: EcheanceAtomique::neuf(0),
            slice_start_ns: EcheanceAtomique::neuf(0),
            ready_since_ns: EcheanceAtomique::neuf(0),
            last_account_ns: EcheanceAtomique::neuf(0),
            user_cpu_ns: EcheanceAtomique::neuf(0),
            kernel_cpu_ns: EcheanceAtomique::neuf(0),
            cpu_ns: [const { EcheanceAtomique::neuf(0) }; MAX_CPUS],
            in_kernel: DrapeauAtomique::neuf(false),
            context_switches: EcheanceAtomique::neuf(0),
            migrations: EcheanceAtomique::neuf(0),
            frame,
            ctx: Context::default(),
            kstack,
            kstack_top,
            fpu,
            fpu_area,
            fs_base: 0,
            clear_child_tid: 0,
            futex_key: EcheanceAtomique::neuf(0),
            wait_queue_key: CleAtomique::neuf(0),
            wake_deadline_ns: EcheanceAtomique::neuf(0),
            waiting_for_child: DrapeauAtomique::neuf(false),
            fresh: true,
            ticks_cpu: EcheanceAtomique::neuf(0),
            noyau: false,
            migrable: false,
            latency_sensitive: DrapeauAtomique::neuf(false),
            budget_reveil_ns: EcheanceAtomique::neuf(0),
            entree_noyau: None,
        });
        amorce_pile(&mut task, task_trampoline, 0x0000_0002);
        task
    }

    pub fn new_kernel(process: Arc<Process>, entree: fn() -> !) -> Box<Task> {
        let mut task = Task::new(process, TrapFrame::new_user(0, 0));
        task.noyau = true;
        task.affinity_mask = 1;
        task.runq_cpu.range(0);
        task.last_cpu.range(0);
        task.entree_noyau = Some(entree);
        amorce_pile(&mut task, kernel_task_trampoline, 0x0000_0202);
        task
    }

    fn fpu_ptr(&self) -> u64 { self.fpu_area }

    /// La pile noyau de cette tache est-elle intacte ?
    ///
    /// Rend `false` des que l'un des `CANARI_MOTS` mots situes JUSTE SOUS le
    /// premier octet utilisable a bouge. C'est le premier endroit qu'une pile
    /// qui deborde atteint, puisqu'elle descend.
    ///
    /// Un debordement de pile noyau n'a pas d'autre symptome : il ecrit dans le
    /// tas voisin, silencieusement, et la faute apparait ailleurs -- ou nulle
    /// part, jusqu'a ce qu'un `RSP` sorte de toute region valide.
    pub fn pile_intacte(&self) -> bool {
        unsafe { canari_intact(self.kstack.as_ptr() as u64) }
    }

    /// De combien d'octets la garde a-t-elle ete entamee ?
    ///
    /// Relit la page entiere, ce que `pile_intacte` ne fait pas : cette
    /// reponse-la n'est utile qu'une fois, quand la rupture est deja constatee.
    /// Zero veut dire que la garde est intacte ; `GARDE_PILE` veut dire qu'elle
    /// a ete traversee de part en part, et le debordement est alors plus
    /// profond que ce que la garde peut mesurer.
    pub fn profondeur_dans_la_garde(&self) -> usize {
        unsafe { garde_entamee(self.kstack.as_ptr() as u64) }
    }

    /// Adresse du premier octet UTILISABLE de la pile.
    ///
    /// Le diagnostic de faute soustrait `RSP` de cette valeur : la difference
    /// est alors la profondeur du debordement, et non un decalage qui inclut la
    /// garde.
    pub fn kstack_base(&self) -> u64 {
        self.kstack.as_ptr() as u64 + GARDE_PILE as u64
    }
}

/// Mots que porte la page de garde.
const GARDE_MOTS: usize = GARDE_PILE / 8;

/// Remplit la page de garde d'une pile fraiche.
///
/// # Securite
/// `base` doit etre le premier octet d'une allocation d'au moins `GARDE_PILE`
/// octets, alignee sur `u64`.
unsafe fn pose_le_canari(base: u64) {
    let mots = base as *mut u64;
    for index in 0..GARDE_MOTS {
        core::ptr::write_volatile(mots.add(index), CANARI_PILE);
    }
}

/// Le HAUT de la garde est-il encore intact ?
///
/// Relit les `CANARI_MOTS` derniers mots de la page -- ceux qu'une pile qui
/// descend touche en premier. C'est le controle du chemin chaud : huit
/// lectures a chaque election, la ou la page entiere en couterait cinq cent
/// douze.
///
/// # Securite
/// Voir [`pose_le_canari`].
unsafe fn canari_intact(base: u64) -> bool {
    let mots = base as *const u64;
    for index in (GARDE_MOTS - CANARI_MOTS)..GARDE_MOTS {
        if core::ptr::read_volatile(mots.add(index)) != CANARI_PILE {
            return false;
        }
    }
    true
}

/// Octets de garde reecrits, mesures depuis le HAUT de la page.
///
/// Parcourt la page du haut vers le bas et s'arrete au premier mot intact :
/// une pile qui deborde ecrit de facon contigue en descendant, et le premier
/// mot encore vivant marque donc la profondeur atteinte. Un motif retrouve
/// plus bas serait une coincidence, pas une preuve que la garde tient.
///
/// # Securite
/// Voir [`pose_le_canari`].
unsafe fn garde_entamee(base: u64) -> usize {
    let mots = base as *const u64;
    let mut entames = 0usize;
    let mut index = GARDE_MOTS;
    while index > 0 {
        index -= 1;
        if core::ptr::read_volatile(mots.add(index)) == CANARI_PILE {
            break;
        }
        entames += 1;
    }
    entames * 8
}

fn amorce_pile(task: &mut Task, trampoline: extern "C" fn() -> !, rflags: u64) {
    unsafe {
        let mut sp = task.kstack_top as *mut u64;

        // SysV x86-64 exige RSP % 16 == 8 a l'entree d'une fonction : l'appel
        // normal a deja empile une adresse de retour. `switch_context` finit
        // par `ret` vers le trampoline ; la case de bourrage reservee ici
        // reproduit donc cette adresse de retour fictive au sommet de pile.
        //
        // Sans elle, le premier fil noyau entrait avec RSP % 16 == 0. Le
        // Trigkey tombait alors en #GP puis en double faute au point `bureau`.
        sp = sp.sub(1); *sp = 0; // adresse de retour fictive apres le trampoline
        sp = sp.sub(1); *sp = trampoline as *const () as usize as u64;
        sp = sp.sub(1); *sp = rflags;
        for _ in 0..6 { sp = sp.sub(1); *sp = 0; }
        task.ctx.rsp = sp as u64;

        debug_assert_eq!(
            (task.ctx.rsp + 8 * 8) & 0xF,
            8,
            "pile initiale: alignement ABI invalide"
        );
    }
}

fn online_affinity_mask() -> u64 {
    let online = smp::schedulable_cpus().max(1).min(MAX_CPUS).min(64);
    if online >= 64 { u64::MAX } else { (1u64 << online) - 1 }
}

#[inline]
fn allowed_on(task: &Task, cpu: usize) -> bool {
    cpu < 64 && task.affinity_mask & (1u64 << cpu) != 0
}

fn running_count_cpu(cpu: usize) -> usize {
    tasks().iter().filter(|t| {
        t.state != TaskState::Zombie && t.on_cpu == cpu as i8 && !t.switching_out.charge()
    }).count()
}

/// La charge d'une file. Deux lectures atomiques depuis le chantier 2 : cette
/// fonction est appelee une fois PAR CPU a chaque reveil, et prenait jusqu'ici
/// un verrou masquant les interruptions sur chacun des coeurs interroges.
fn queue_pressure(cpu_id: usize) -> usize {
    crate::arch::x86_64::cpu_local::CpuId::from_index(cpu_id)
        .map(|id| crate::arch::x86_64::cpu_local::local(id).run_queue_len())
        .unwrap_or(0)
}

/// Ce qu'un voleur peut prendre a ce CPU.
///
/// BOUCHAUD_P0_REVEIL_CIBLE_V1 : les DEUX bandes, et non la normale seule.
///
/// La regle precedente -- « ne jamais rendre un coeur candidat pour une tache
/// interactive, une migration lui couterait sa reponse » -- protege un cas qui
/// n'existe que si la tache finit par etre elue. Une interactive derriere un
/// fil noyau ne l'est pas : elle a attendu 6,78 s sur le releve TRIGKEY. Le
/// cache froid d'une migration se compte en microsecondes.
///
/// `FileCpu::vole` sert toujours la bande NORMALE en premier : le travail de
/// fond reste ce qui se deplace le mieux. Seule la condition d'etre EXAMINE
/// change.
fn pression_volable(cpu_id: usize) -> usize {
    let (interactives, normales) = crate::arch::x86_64::cpu_local::CpuId::from_index(cpu_id)
        .map(|id| crate::arch::x86_64::cpu_local::local(id).attente_file())
        .unwrap_or((0, 0));
    crate::kernel::scheduler::reveil::pression_de_secours(interactives, normales)
}

fn choose_runq_cpu(mask: u64) -> u8 {
    let online = smp::schedulable_cpus().max(1).min(MAX_CPUS);
    let mut best_cpu = 0usize;
    let mut best_score = usize::MAX;
    for cpu_id in 0..online {
        if cpu_id >= 64 || mask & (1u64 << cpu_id) == 0 { continue; }
        let rq = queue_pressure(cpu_id);
        let running = running_count_cpu(cpu_id);
        let measured = cpu::load_percent_cpu(cpu_id) as usize;
        let bsp_penalty = if cpu_id == 0 && online > 1 { 24 } else { 0 };
        let score = rq.saturating_mul(32)
            .saturating_add(running.saturating_mul(16))
            .saturating_add(measured)
            .saturating_add(bsp_penalty);
        if score < best_score { best_score = score; best_cpu = cpu_id; }
    }
    best_cpu as u8
}

/// L'etat d'un coeur pour la politique de reveil, en lectures ATOMIQUES.
///
/// Aucun acces a la table des taches : ce chemin s'execute aussi depuis une
/// IRQ, sur un coeur qui n'est pas celui qu'on interroge.
fn etat_coeur_reveil(cpu: usize, masque_affinite: u64, maintenant_ns: u64) -> ReveilCoeur {
    let en_ligne = cpu < smp::schedulable_cpus().min(MAX_CPUS);
    let (sensible, interactive, depuis_ns) = crate::kernel::task::profil_occupant(cpu);
    let (attente_interactive, attente_normale) =
        crate::arch::x86_64::cpu_local::CpuId::from_index(cpu)
            .map(|id| crate::arch::x86_64::cpu_local::local(id).attente_file())
            .unwrap_or((0, 0));
    ReveilCoeur {
        en_ligne,
        autorise: cpu < 64 && masque_affinite & (1u64 << cpu) != 0,
        inactif: cpu::is_idle(cpu),
        occupant_sensible: sensible,
        occupant_classe: if interactive { ReveilClasse::Interactive } else { ReveilClasse::Normale },
        residence_ns: if depuis_ns == 0 { 0 } else { maintenant_ns.saturating_sub(depuis_ns) },
        attente_interactive,
        attente_normale,
    }
}

/// Publie une tache prete, exactement une fois, dans une runqueue physique.
///
/// # BOUCHAUD_P0_REVEIL_CIBLE_V1 -- ce que cette fonction faisait, et ce que
/// cela coutait
///
/// Elle posait la tache sur son COEUR PRECEDENT, quelle que soit sa charge, et
/// n'envoyait l'IPI de replanification que si ce coeur DORMAIT. Un coeur
/// occupe ne recevait qu'un `need_resched` differe -- lequel n'est servi par
/// AUCUN des trois mecanismes du systeme quand l'occupant est une tache noyau
/// (cf. `scheduler::reveil`). Aucun voleur ne venait non plus : la pression
/// volable ne comptait que la bande normale.
///
/// Releve TRIGKEY : `hid_wake_to_run_max_us = 6782927`, pour un corps de
/// scrutation de 162 us et une prise de verrou de 2 us.
///
/// Trois changements, dans cet ordre de preference :
///
///   1. LE PLACEMENT. Une tache qui declare `latency_sensitive` est posee sur
///      le coeur qui la servira le plus tot -- un coeur au repos si la machine
///      en a un, et sur seize coeurs elle en a presque toujours. Cela ne coute
///      une commutation a personne.
///   2. LA DECISION. L'etat du coeur est relu APRES la mise en file et APRES
///      la barriere -- l'ordre du motif croise n'est pas touche -- et la
///      politique dit alors s'il faut un IPI, une demande ciblee, ou rien.
///   3. LA DEMANDE CIBLEE. Elle seule ouvre la preemption d'un fil noyau, et
///      seulement pour une tache sensible restee sous son budget.
fn publish_ready(index: usize) {
    if index >= tasks().len() || tasks()[index].state != TaskState::Ready
        || tasks()[index].on_cpu >= 0 || tasks()[index].switching_out.charge()
    { return; }

    let maintenant = crate::kernel::timer::monotonic_ns();
    if tasks()[index].ready_since_ns == 0 {
        tasks()[index].ready_since_ns.range(maintenant);
    }

    // LE BUDGET EST LU ET REMIS A ZERO ICI, ET NULLE PART AILLEURS.
    //
    // Sa valeur au moment d'une publication est donc exactement ce que
    // l'activation PRECEDENTE a consomme. Une tache periodique qui fait 162 us
    // par tour reste sous la borne ; une tache qui se met a calculer la
    // depasse et perd son privilege des le tour suivant. Aucun crochet a poser
    // sur les chemins de blocage : le point de lecture est le point de remise
    // a zero.
    let reveille = ReveilTache {
        classe: match tasks()[index].priorite.charge() {
            Priorite::Interactive => ReveilClasse::Interactive,
            Priorite::Normale => ReveilClasse::Normale,
        },
        sensible: tasks()[index].latency_sensitive.charge(),
        consomme_ns: tasks()[index].budget_reveil_ns.echange(0),
    };

    let precedent = tasks()[index].runq_cpu.charge() as usize;
    let historique = if allowed_on(&tasks()[index], precedent) {
        precedent
    } else {
        choose_runq_cpu(tasks()[index].affinity_mask) as usize
    };

    let target = if reveille.privilegiee() {
        let masque = tasks()[index].affinity_mask;
        let en_ligne = smp::schedulable_cpus().max(1).min(MAX_CPUS);
        let mut selection = ReveilSelection::neuve(precedent, true);
        for cpu_id in 0..en_ligne {
            selection.propose(cpu_id, &etat_coeur_reveil(cpu_id, masque, maintenant));
        }
        match selection.retenu() {
            Some(choisi) => {
                if choisi != historique {
                    crate::kernel::scheduler::preempt::note_placement_deplace();
                }
                choisi
            }
            None => historique,
        }
    } else {
        historique
    };

    tasks()[index].runq_cpu.range(target as u8);
    if let Some(id) = crate::arch::x86_64::cpu_local::CpuId::from_index(target) {
        // L'IDENTITE, pas l'indice : un emplacement recycle ne doit pas
        // heriter de l'entree laissee par son occupant precedent.
        let Some(identite) = registre_id(index) else { return };
        // La BANDE, pas seulement la file. La classe d'ordonnancement etait
        // lue par l'election et par rien d'autre : la file etait une seule
        // FIFO, et une tache interactive attendait derriere le rendu. Elle est
        // desormais materialisee dans la structure -- deux bandes, servies
        // dans l'ordre, la normale garantie par la borne anti-famine.
        let bande = match tasks()[index].priorite.charge() {
            Priorite::Interactive => crate::kernel::scheduler::runqueue::Bande::Interactive,
            Priorite::Normale => crate::kernel::scheduler::runqueue::Bande::Normale,
        };
        crate::arch::x86_64::cpu_local::local(id).enqueue_bande(identite.en_mot(), bande);
    } else {
        // Coeur inadressable : la tache reste en file logique, personne a
        // prevenir.
        crate::kernel::scheduler::preempt::note_reveil_en_file();
        return;
    }
    // Seconde moitie du motif croise (voir `idle_enter`). La mise en file
    // ci-dessus se termine par une liberation de verrou -- une simple ecriture
    // sur x86 --, et `is_idle` est une lecture. Sans barriere, le processeur
    // peut executer la lecture AVANT que l'ecriture ne quitte le tampon : nous
    // lirions « pas idle » alors que le coeur s'endort, et lui lirait une file
    // vide alors que nous venons de la remplir. Personne n'envoie l'IPI.
    //
    // Une lecture SeqCst ne suffirait pas : sur x86 elle reste un `mov` et ne
    // vide pas le tampon d'ecriture. Il faut la barriere.
    //
    // LA DECISION SE PREND APRES CETTE BARRIERE, ET C'EST TOUT L'ENJEU. Le
    // choix du coeur, lui, a pu etre fait sur un etat perime -- au pire il
    // pose la tache sur un coeur qui vient de se charger, ce qui coute une
    // election. L'IPI, lui, ne peut pas etre decide sur un etat perime : c'est
    // la moitie du motif croise. Ou bien nous lisons ici `is_idle` vrai et
    // nous envoyons l'IPI, ou bien le coeur qui s'endort relit notre file dans
    // `commit_scheduler_idle` et renonce a dormir. Au moins l'un des deux voit
    // l'autre ; aucun reveil ne se perd.
    core::sync::atomic::fence(Ordering::SeqCst);

    let etat = etat_coeur_reveil(target, u64::MAX, crate::kernel::timer::monotonic_ns());
    match crate::kernel::scheduler::reveil::decide(&etat, &reveille) {
        ReveilDecision::ReveilImmediat => {
            crate::kernel::scheduler::preempt::request_cpu(target);
            crate::kernel::scheduler::preempt::note_reveil_immediat();
            crate::kernel::scheduler::preempt::note_ipi_reveil();
            smp::reschedule_cpu(target);
        }
        ReveilDecision::PreemptionCiblee => {
            crate::kernel::scheduler::preempt::demande_ciblee(target);
            crate::kernel::scheduler::preempt::note_reveil_cible();
            crate::kernel::scheduler::preempt::note_ipi_reveil();
            smp::reschedule_cpu(target);
        }
        ReveilDecision::DemandeDifferee => {
            crate::kernel::scheduler::preempt::request_cpu(target);
            crate::kernel::scheduler::preempt::note_reveil_differe();
            // Le comportement historique, INCHANGE : un coeur qui vient de
            // s'endormir entre la mise en file et ici recoit quand meme son
            // IPI. C'est la relecture, pas la decision, qui ferme la course.
            if etat.inactif {
                crate::kernel::scheduler::preempt::note_ipi_reveil();
                smp::reschedule_cpu(target);
            }
        }
        ReveilDecision::MiseEnFile => {
            crate::kernel::scheduler::preempt::note_reveil_en_file();
        }
    }
}

pub fn register(mut task: Box<Task>) -> usize {
    let _domaine = crate::kernel::sync::portee(crate::kernel::sync::Domaine::Processus);
    let _kernel = smp_lock::enter();
    if task.noyau && !task.migrable {
        // Les taches noyau historiques supposent le coeur zero, et rien ne
        // dit qu'elles y survivraient ailleurs. Le defaut reste donc leur
        // comportement d'avant, a la lettre.
        task.affinity_mask = 1;
        task.runq_cpu.range(0);
        task.last_cpu.range(0);
    } else if task.noyau {
        // UN TRAVAILLEUR NOYAU EPINGLE AU COEUR ZERO NE TOURNE PAS SI LE
        // COEUR ZERO N'EST PAS DANS L'ORDONNANCEUR.
        //
        // C'est exactement ce qui arrivait au fil de montage sur un demarrage
        // sans bureau : la tache etait creee, enregistree, visible dans
        // `[SMP-TASK]` avec `on=-1`, et n'etait jamais elue. Le coeur zero
        // tenait la boucle interactive hors de toute tache ; les trois autres
        // coeurs, eux, tournaient a vide, interdits de la prendre par un
        // masque d'affinite valant un.
        //
        // Une tache qui se declare migrable est donc placee comme n'importe
        // quelle autre : sur le coeur le moins charge parmi ceux en ligne.
        task.affinity_mask = online_affinity_mask();
        task.runq_cpu.range(choose_runq_cpu(task.affinity_mask));
        task.last_cpu.range(u8::MAX);
    } else {
        if task.affinity_mask == 0 {
            task.affinity_mask = online_affinity_mask();
        } else {
            task.affinity_mask &= online_affinity_mask();
            if task.affinity_mask == 0 { task.affinity_mask = online_affinity_mask(); }
        }
        if task.runq_cpu == u8::MAX || !allowed_on(&task, task.runq_cpu.charge() as usize) {
            task.runq_cpu.range(choose_runq_cpu(task.affinity_mask));
        }
    }
    task.on_cpu.range(-1);
    task.switching_out.range(false);

    // Le registre choisit l'emplacement lui-meme, sous son propre verrou :
    // c'est la SEULE section critique qui reste sur ce chemin. Le predicat dit
    // ce qu'est un emplacement recyclable -- une tache morte, sur aucun coeur,
    // et qui n'est pas en train de commuter. Une tache qui commute encore
    // possede sa pile noyau ; la reecrire la ferait reprendre sur une autre.
    let identite = registre_ajoute(task, |ancienne| {
        ancienne.state == TaskState::Zombie
            && ancienne.on_cpu < 0
            && !ancienne.switching_out.charge()
    })
    .expect("registre des taches plein");
    let index = identite.emplacement();

    {
        let registered = &tasks()[index];
        let process = &registered.process;
        let metadata = process.metadata.lock();
        crate::serial_println!(
            "[SMP-TASK] idx={} tid={} pid={} rq={} last={} aff={:#x} on={} kernel={} prio={:?} name={}",
            index, registered.tid, process.pid, registered.runq_cpu,
            registered.last_cpu, registered.affinity_mask, registered.on_cpu,
            registered.noyau, registered.priorite, metadata.name.as_str(),
        );
    }
    publish_ready(index);
    index
}

fn index_of(tid: u32) -> Option<usize> { tasks().iter().position(|t| t.tid == tid) }
/// Une tache par son identifiant de fil, en lecture PARTAGEE.
pub fn by_tid(tid: u32) -> Option<GardeLectureTache> {
    registre_tache(index_of(tid)?)
}
pub fn live_count() -> usize {
    tasks().iter().filter(|t| t.state != TaskState::Zombie).count()
}
fn ready_count() -> usize { tasks().iter().filter(|t| t.state == TaskState::Ready).count() }
fn ready_count_cpu(cpu: usize) -> usize {
    tasks().iter().filter(|t| {
        t.state == TaskState::Ready && t.on_cpu < 0 && !t.switching_out.charge()
            && t.runq_cpu.charge() as usize == cpu && allowed_on(t, cpu)
    }).count()
}
fn stealable_count_cpu(cpu: usize) -> usize {
    tasks().iter().filter(|t| {
        t.state == TaskState::Ready && t.on_cpu < 0 && !t.switching_out.charge() && !t.noyau
            && t.runq_cpu.charge() as usize != cpu && allowed_on(t, cpu)
    }).count()
}
fn running_count() -> usize {
    tasks().iter().filter(|t| t.state != TaskState::Zombie && t.on_cpu >= 0).count()
}
