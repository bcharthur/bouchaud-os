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

/// Dort `ticks` ticks, que l'appelant tienne le gros verrou ou non.
///
/// # Le contrat a change, et voici ce qu'il etait
///
/// Cette fonction exigeait le gros verrou de son appelant :
///
/// ```ignore
/// debug_assert!(smp_lock::held_by_current_cpu(),
///               "task: sleep_ticks requiert le BKL externe de l'appelant");
/// ```
///
/// L'exigence ne decrivait rien de ce que le corps fait. Tout ce qu'il touche
/// est atomique -- `wake_deadline_ns` et `state` sont des stores ordonnes,
/// `arme_echeance` est un `fetch_min` -- et la boucle d'attente tourne de toute
/// facon SANS le verrou : `suspend_for_schedule()` le rend des la premiere
/// ligne, precisement parce qu'on ne dort jamais en le tenant.
///
/// L'assertion ne protegeait donc pas le sommeil. Elle FORCAIT ses appelants a
/// prendre un verrou global pour le lui rendre aussitot -- et c'est a ce titre
/// qu'elle maintenait `nanosleep` et `clock_nanosleep` sous le gros verrou,
/// alors qu'ils n'en avaient aucun usage.
///
/// Le gros verrou lui-meme n'existe plus (BOUCHAUD_BKL_SUPPRIME_V1).
/// L'echeance qu'un `sleep_ticks(ticks)` armerait s'il etait appele
/// maintenant.
///
/// # Pourquoi cette fonction existe
///
/// Mesurer le retard de reveil d'une tache demande de connaitre l'instant ou
/// elle DEVAIT repartir. Recopier la formule chez l'appelant marcherait
/// aujourd'hui et mentirait le jour ou la cadence du timer change -- et le
/// mensonge accuserait l'ordonnanceur d'un retard qui n'existe pas.
///
/// Elle ne change aucun comportement : `sleep_ticks` l'appelle, et c'est la
/// seule definition de l'echeance.
pub fn echeance_pour(ticks: u64) -> u64 {
    let duration_ns = ticks
        .max(1)
        .saturating_mul(1_000_000_000 / crate::kernel::timer::TICKS_PER_SECOND);
    crate::kernel::timer::monotonic_ns().saturating_add(duration_ns)
}

pub fn sleep_ticks(ticks: u64) {
    avec_tache(|| dort(ticks, true))
}

/// Sommeil NON interruptible : une tache condamnee le termine au lieu d'y
/// mourir. Pour les attentes du noyau qui doivent aller a leur terme meme
/// pendant une mort imposee (`attend_extinction_freres`).
pub(crate) fn sleep_ticks_noyau(ticks: u64) {
    avec_tache(|| dort(ticks, false))
}

/// Sommeil de `nanosleep` : ne rend la main avant l'echeance que pour un
/// signal qui fera quelque chose sur CE fil.
///
/// BOUCHAUD_SOMMEIL_SIGNAL_V1. Rend `None` a l'echeance, `Some(reste_ns)` si
/// un signal l'interrompt (l'appelant rend alors EINTR et ecrit `remain`).
///
/// `dort` rend la main des que la tache est reveillee. Depuis que la pose
/// d'un signal reveille tout le processus (BOUCHAUD_SIGNAL_INTERROMPT_POLL_V1), un
/// SIGCHLD a la disposition par defaut -- ignore -- reveillait chaque fil en
/// `sleep()` : la sonde compta-stress a vu `sleep(30)` rendre 0 au bout
/// d'une seconde, des la mort de son premier fils. Linux ne reveille pas un
/// sommeil pour un signal ignore, et rend EINTR pour un signal gere.
pub fn sleep_ticks_signalable(ticks: u64) -> Option<u64> {
    sleep_jusqua_signalable(echeance_pour(ticks), ticks)
}

/// BOUCHAUD_SOMMEIL_NS_V1 : le meme sommeil, a une echeance EXACTE
/// (`maintenant + ns`), sans arrondi au tick. `ns` est non nul.
pub fn sleep_ns_signalable(ns: u64) -> Option<u64> {
    let deadline = crate::kernel::timer::monotonic_ns().saturating_add(ns.max(1));
    // Les ticks ne servent plus qu'au journal du sommeil (forensique).
    sleep_jusqua_signalable(deadline, ns.div_ceil(1_000_000).max(1))
}

fn sleep_jusqua_signalable(deadline: u64, ticks: u64) -> Option<u64> {
    loop {
        // Parking refuse : la tache ne peut plus dormir -- elle est morte
        // (`exit_group` d'un autre fil). Rendre la main tout de suite, comme
        // avant : la frontiere de sortie d'appel systeme la retire. Boucler
        // ici la faisait TOURNER jusqu'a l'echeance -- un fil tue pendant un
        // `sleep(30)` brulait son coeur 30 s et retardait d'autant la mort de
        // son processus (os-primitives et endurance sous KVM, run
        // 37618172578 : machine vivante, occupee, qui n'avance plus).
        if !dort_jusqua(deadline, ticks, true) || current().state == TaskState::Zombie {
            return None;
        }
        let maintenant = crate::kernel::timer::monotonic_ns();
        if maintenant >= deadline {
            return None;
        }
        if signal_interrompt_attente() {
            return Some(deadline - maintenant);
        }
        // Reveil sans effet (signal ignore, ou destine a un autre fil) : on
        // se rendort jusqu'a la MEME echeance.
    }
}

fn dort(ticks: u64, interruptible: bool) {
    dort_jusqua(echeance_pour(ticks), ticks, interruptible);
}

/// Rend `false` si le parking a ete refuse (la tache ne peut plus dormir).
fn dort_jusqua(deadline: u64, ticks: u64, interruptible: bool) -> bool {
    exige_tache_courante();
    // BOUCHAUD_P15_BROWSER_HANG_FORENSICS
    forensic_wait_begin_if_idle(WAIT_SLEEP, deadline, ticks);
    // BOUCHAUD_ATTRIBUTION_REVEIL_V1 : les instants de CE sommeil.
    let entree_ns = crate::kernel::timer::monotonic_ns();
    let tics_entree = crate::kernel::timer::ticks();
    current().reveil_pret_ns.range(0);
    current().reveil_elu_ns.range(0);
    current().reveil_commute_ns.range(0);
    current().reveil_passe_ns.range(0);
    current().reveil_par.range(0);
    current().wake_deadline_ns.range(deadline);
    // BOUCHAUD_PREUVE_HOTE_V1 : le fil arme range l'echeance et ses comptes
    // a l'entree du sommeil (echeance deja fixee : rien ne la deplace).
    {
        use crate::kernel::attribution_reveil as ar;
        let tache = indice_courant();
        if ar::ENREGISTREUR.arme_pour(tache) {
            let comptes = current().comptes_fil.cumuls();
            ar::ENREGISTREUR.note(tache, &ar::Sommeil { echeance_ns: deadline, entree_ns, comptes });
        }
    }
    // BOUCHAUD_CYCLE_DE_VIE_V1 : le sommeil est interruptible -- `nanosleep`,
    // et les attentes par tick des appels systeme. Les fils noyau, qui s'en
    // servent aussi, ne sont jamais condamnes. C'etait ICI l'ecriture qui
    // ecrasait `Zombie` par `Blocked`.
    match publie_parking(interruptible) {
        Parking::Gare => {}
        Parking::Refuse => {
            current().wake_deadline_ns.range(0);
            forensic_wait_clear(WAIT_SLEEP);
            return false;
        }
        Parking::Condamnee => meurt_au_parking(),
    }
    arme_echeance(deadline);

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
    current().wake_deadline_ns.range(0);
    {
        let mesure = crate::kernel::attribution_reveil::Mesure {
            echeance_ns: deadline,
            entree_ns,
            pret_ns: current().reveil_pret_ns.charge(),
            elu_ns: current().reveil_elu_ns.charge(),
            reprise_ns: crate::kernel::timer::monotonic_ns(),
            tics_entree,
            tics_reprise: crate::kernel::timer::ticks(),
            commute_ns: current().reveil_commute_ns.charge(),
            passe_ns: current().reveil_passe_ns.charge(),
        };
        let d = crate::kernel::attribution_reveil::decompose(&mesure);
        current().attribution_reveil.note(&d, mesure.reprise_ns, local_cpu() as u64, current().reveil_par.charge());
        // BOUCHAUD_ATTRIBUTION_REVEIL_V2 : un retard au-dela du seuil est
        // ventile (livraison, detection, election, reprise ; externe ou non).
        if d.total_ns > crate::kernel::attribution_reveil::SEUIL_CONTAMINATION_NS {
            use crate::kernel::attribution_reveil as ar;
            let mut absences = [ar::Absence::default(); 16];
            let (n, incomplet) = ar::absences_entre(mesure.echeance_ns, mesure.reprise_ns, &mut absences);
            let v = ar::ventile(&mesure, &d, local_cpu() as u32, &absences[..n], incomplet);
            current().ventilation_reveil.note(&v);
            // BOUCHAUD_PREUVE_HOTE_V1 : livraison + election dans les comptes
            // cumules du fil ; la reprise, le journal l'impute (fil installe).
            current().comptes_fil.note_ventilation(&v);
        }
    }
    // Echeance passee sans reveil : annuler le parking (sans effet si un
    // reveilleur l'a deja fait).
    termine_attente();
    // BOUCHAUD_P15_BROWSER_HANG_FORENSICS
    forensic_wait_clear(WAIT_SLEEP);
    true
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

/// La plus proche echeance de sommeil connue (borne inferieure ;
/// `echeances::JAMAIS` pendant un balayage). BOUCHAUD_REVEIL_PRECIS_V1 : un
/// le balayage consulte cette borne ; les timers idle utilisent la borne locale.
pub fn prochaine_echeance_ns() -> u64 {
    PROCHAINE_ECHEANCE.borne()
}

// Le balayage reste global, mais un CPU idle n'arme que les sommeils qui
// lui sont attachés. Armer la borne globale sur chaque AP faisait tirer
// plusieurs timers KVM pour un seul dormeur (puis un IPI vers son CPU).
static PROCHAINES_PAR_COEUR: [crate::kernel::echeances::Echeances; MAX_CPUS] =
    [const { crate::kernel::echeances::Echeances::neuve() }; MAX_CPUS];

pub fn prochaine_echeance_du_coeur(cpu: usize) -> u64 {
    PROCHAINES_PAR_COEUR.get(cpu).map_or(crate::kernel::echeances::JAMAIS, |b| b.borne())
}

/// L'echeance de la tache BLOQUEE installee sur ce coeur (0 : aucune).
/// BOUCHAUD_REVEIL_ECHEANCE_PROPRE_V1 : lue juste avant `hlt`, interruptions
/// coupees -- la tache, installee, ne peut ni changer de coeur ni etre
/// recyclee. Une echeance perimee sur une tache prete est ignoree.
pub fn echeance_tache_installee() -> u64 {
    let index = current_index_raw();
    if index == NO_TASK || index >= tasks().len() {
        return 0;
    }
    let tache = &tasks()[index];
    if tache.state != TaskState::Blocked {
        return 0;
    }
    tache.wake_deadline_ns.charge()
}

/// Declare une echeance. A appeler POUR CHAQUE `wake_deadline_ns` non nul.
///
/// `tools/verifie-echeances.py` refuse une ecriture qui ne passerait pas par
/// ici : une echeance inconnue du raccourci ne serait jamais servie.
pub(crate) fn arme_echeance(deadline_ns: u64) {
    PROCHAINE_ECHEANCE.arme(deadline_ns);
    PROCHAINES_PAR_COEUR[local_cpu()].arme(deadline_ns);
    arme_si_interactive(deadline_ns);
}

// BOUCHAUD_REVEIL_PRECIS_OCCUPE_V1
//
// Sur un coeur OCCUPE, un sommeil n'etait detecte qu'au tic du PIT (1 ms,
// coeur zero) ou au quantum (4 ms) : le reveil precis ne servait qu'un coeur
// au repos. Run 39387282034 (KVM SMP4, attribution) : « PREUVE et n'aggrave
// pas le pire cas au-dela du bruit au repos : pire avec 881 <= sans 225 +
// bruit au repos 330 = faux » -> QUALIFIE_ECHEC. Le calendrier absolu d'une
// tache periodique derive lentement par rapport au tic : son decalage de
// phase balaie la milliseconde, et un reveil sur cent attend presque un tic
// entier.
//
// La borne des sommeils INTERACTIFS, par coeur d'attache (`runq_cpu`) : un
// coeur occupe arme son minuteur sur la plus proche de cette borne et de la
// fin de son quantum (`smp::arm_local_scheduler_timer`). C'est une indication
// de latence, pas un mecanisme de reveil : une borne perimee ou perdue ramene
// au tic, jamais a un reveil perdu -- le balayage des echeances reste la seule
// voie de reveil. Les taches de classe normale gardent la granularite du tic.
static PROCHAINE_INTERACTIVE: [core::sync::atomic::AtomicU64; crate::arch::x86_64::smp::MAX_CPUS] =
    [const { core::sync::atomic::AtomicU64::new(crate::kernel::echeances::JAMAIS) };
        crate::arch::x86_64::smp::MAX_CPUS];

/// La plus proche echeance d'un sommeil interactif attache a `cpu`
/// (`echeances::JAMAIS` : aucune connue).
pub fn prochaine_echeance_interactive_ns(cpu: usize) -> u64 {
    PROCHAINE_INTERACTIVE
        .get(cpu)
        .map_or(crate::kernel::echeances::JAMAIS, |b| b.load(core::sync::atomic::Ordering::Relaxed))
}

/// La borne interactive de `cpu` si elle est echue a `maintenant_ns`,
/// recalculee d'abord sur les seules taches attachees a ce coeur.
///
/// Une interactive reveillee AUTREMENT (futex, signal) avant son echeance
/// laisse sa borne perimee ; le balayage global, lui, n'est pas du (la borne
/// globale a ete recalee sans elle) et ne la recalculerait pas. Sans ce
/// recalcul, le coeur occupe s'armerait a `maintenant + MARGE_NS` en boucle
/// jusqu'au prochain balayage -- une tempete d'interruptions. Appele avec les
/// interruptions coupees (armement du minuteur local).
pub fn borne_interactive_a_jour(cpu: usize, maintenant_ns: u64) -> u64 {
    let Some(borne) = PROCHAINE_INTERACTIVE.get(cpu) else {
        return crate::kernel::echeances::JAMAIS;
    };
    let valeur = borne.load(core::sync::atomic::Ordering::Relaxed);
    if valeur == crate::kernel::echeances::JAMAIS || valeur > maintenant_ns {
        return valeur;
    }
    let mut minimum = crate::kernel::echeances::JAMAIS;
    for index in 0..tasks().len() {
        let tache = &tasks()[index];
        let echeance = tache.wake_deadline_ns.charge();
        if echeance != 0
            && tache.state == TaskState::Blocked
            && tache.priorite.charge() == Priorite::Interactive
            && tache.runq_cpu.charge() as usize == cpu
            && echeance < minimum
        {
            minimum = echeance;
        }
    }
    // Une tache attachee a ce coeur qui s'endort pendant ce recalcul y tourne
    // elle-meme : elle a publie `Blocked` et son echeance avant d'armer, et
    // cette boucle la voit. Un `store` ne perd donc rien d'ici.
    borne.store(minimum, core::sync::atomic::Ordering::Relaxed);
    minimum
}

/// La tache courante pose une echeance : interactive, elle entre dans la borne
/// de son coeur d'attache, et ce coeur -- le sien, puisqu'elle y tourne --
/// avance son minuteur s'il portait plus tard.
fn arme_si_interactive(deadline_ns: u64) {
    if deadline_ns == 0 {
        return;
    }
    let index = current_index_raw();
    if index == NO_TASK || index >= tasks().len() {
        return;
    }
    let tache = &tasks()[index];
    if tache.priorite.charge() != Priorite::Interactive {
        return;
    }
    let coeur = tache.runq_cpu.charge() as usize;
    if let Some(borne) = PROCHAINE_INTERACTIVE.get(coeur) {
        borne.fetch_min(deadline_ns, core::sync::atomic::Ordering::Relaxed);
    }
    if coeur == local_cpu() {
        crate::arch::x86_64::smp::avance_pour_sommeil_interactif(deadline_ns);
    }
}

/// Recalcule la borne exactement, apres un balayage.
fn recalcule_echeance() {
    let mut minimum = crate::kernel::echeances::JAMAIS;
    let mut interactives = [crate::kernel::echeances::JAMAIS; crate::arch::x86_64::smp::MAX_CPUS];
    {
        // Le temporaire de `for ... tasks().len()` gardait deja le registre
        // pendant tout le scan. Reprendre tasks() pour chaque champ ajoutait
        // autant de prises/liberations imbriquees, toutes IRQ masquees.
        // Une vue explicite garde la MEME portee et tombe avant ALARMES.
        let registre = tasks();
        for tache in registre.iter() {
            let echeance = tache.wake_deadline_ns.charge();
            if tache.state == TaskState::Blocked && echeance != 0 {
                let coeur = tache.runq_cpu.charge() as usize;
                if let Some(borne) = PROCHAINES_PAR_COEUR.get(coeur) {
                    borne.recale(echeance);
                }
                if echeance < minimum {
                    minimum = echeance;
                }
                if tache.priorite.charge() == Priorite::Interactive
                    && coeur < interactives.len() && echeance < interactives[coeur]
                {
                    interactives[coeur] = echeance;
                }
            }
        }
    }
    // BOUCHAUD_REVEIL_PRECIS_OCCUPE_V1 : remises a JAMAIS au debut du balayage
    // (`wake_sleepers`), recalees ici par `fetch_min` -- une pose concurrente
    // n'est pas ecrasee.
    for (borne, valeur) in PROCHAINE_INTERACTIVE.iter().zip(interactives.iter()) {
        borne.fetch_min(*valeur, core::sync::atomic::Ordering::Relaxed);
    }
    // Les alarmes vivent en TICKS ; la borne est en nanosecondes. Convertir --
    // et non comparer les deux tels quels, ce qui est precisement l'erreur que
    // `fire_alarms` portait.
    let par_tick = 1_000_000_000 / crate::kernel::timer::TICKS_PER_SECOND;
    // BOUCHAUD_REVEIL_DU_V1 : une alarme n'echoit qu'au TIC (`fire_alarms`
    // compare des tics). Quand les tics retardent sur l'horloge monotone
    // (ecarts de tic sous KVM), son equivalent en nanosecondes est deja passe
    // sans qu'elle soit servable : une borne due sans rien a servir, qu'un
    // coeur au repos armerait a chaque marge jusqu'au tic suivant. Elle ne
    // peut pas echoir avant le prochain tic : sa borne ne descend pas sous
    // `maintenant + un tic` -- dans ce cas seulement : une alarme encore a
    // venir garde sa borne exacte.
    let maintenant = crate::kernel::timer::monotonic_ns();
    let alarmes = ALARMES.lock();
    for (_, echeance_ticks) in alarmes.iter() {
        let echeance = echeance_ticks.saturating_mul(par_tick);
        let echeance = if echeance <= maintenant { maintenant.saturating_add(par_tick) } else { echeance };
        PROCHAINES_PAR_COEUR[0].recale(echeance);
        if echeance < minimum {
            minimum = echeance;
        }
    }
    PROCHAINE_ECHEANCE.recale(minimum);
}

// DIAGNOSTIC de la queue au repos (BOUCHAUD_REVEIL_PRECIS_V1) : les reveils
// rendus plus de `SEUIL_TARDIF_NS` apres leur echeance, classes par le
// contexte du balayage qui les rend -- la tache elle-meme au retour de son
// `hlt` (`soi`), une autre tache (`autre` : le coeur n'etait pas au repos),
// ou une interruption (`irq`), ou la boucle d'un coeur sans tache
// (`inactif`). La source IRQ est DECLAREE par le chemin de preemption
// (`balayage_depuis_irq`), pas deduite. Aucun effet sur l'ordonnancement.
const SEUIL_TARDIF_NS: u64 = 200_000;
static TARDIFS_SOI: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static TARDIFS_AUTRE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static TARDIFS_IRQ: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static TARDIFS_INACTIF: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static BALAYAGE_IRQ: [core::sync::atomic::AtomicBool; crate::arch::x86_64::smp::MAX_CPUS] =
    [const { core::sync::atomic::AtomicBool::new(false) }; crate::arch::x86_64::smp::MAX_CPUS];

/// Seulement le balayage exécuté par le chemin de préemption local : une
/// autre IRQ, un autre CPU ou un réveil depuis une tâche doit notifier.
fn reveil_servi_dans_irq_locale(cible: usize) -> bool {
    let cpu = local_cpu();
    crate::kernel::scheduler::reveil::election_locale_en_cours(
        cible, cpu, BALAYAGE_IRQ[cpu].load(core::sync::atomic::Ordering::Relaxed))
}

/// `wake_sleepers` appele depuis le chemin de preemption (IRQ du quantum).
fn wake_sleepers_depuis_irq() {
    let cpu = local_cpu();
    if cpu < BALAYAGE_IRQ.len() {
        BALAYAGE_IRQ[cpu].store(true, core::sync::atomic::Ordering::Relaxed);
    }
    wake_sleepers();
    if cpu < BALAYAGE_IRQ.len() {
        BALAYAGE_IRQ[cpu].store(false, core::sync::atomic::Ordering::Relaxed);
    }
}

/// Service due sleeps before deciding whether an interrupted kernel task
/// has a targeted preemption request. Otherwise the request can never exist:
/// it is published by the wakeup that used to require preemption first.
/// The same lock/registry/IRQ and handoff exclusions as stack preemption
/// apply to this earlier scan; an unsafe context keeps its deferred path.
pub fn sert_echeances_au_vecteur_local() {
    if !crate::kernel::scheduler::preempt::preemption_noyau_sure()
        || transition_ouverte_sans_passation(local_cpu())
    { return; }
    wake_sleepers_depuis_irq();
}

/// `(soi, autre, irq, inactif)` : reveils tardifs par contexte de balayage.
pub fn compteurs_tardifs() -> [u64; 4] {
    use core::sync::atomic::Ordering::Relaxed;
    [TARDIFS_SOI.load(Relaxed), TARDIFS_AUTRE.load(Relaxed), TARDIFS_IRQ.load(Relaxed), TARDIFS_INACTIF.load(Relaxed)]
}

fn wake_sleepers() {
    let now = crate::kernel::timer::monotonic_ns();
    // Le raccourci : une charge atomique au lieu d'un balayage complet.
    if !PROCHAINE_ECHEANCE.commence_balayage(now) {
        return;
    }
    // Comme la borne globale : retirer seulement les bornes dues AVANT
    // le scan, puis fetch_min. Un armement concurrent ne peut être écrasé.
    for borne in PROCHAINES_PAR_COEUR.iter() {
        borne.commence_balayage(now);
    }
    // BOUCHAUD_REVEIL_PRECIS_OCCUPE_V1 : la borne interactive repart de zero ;
    // `recalcule_echeance` la recale exactement.
    for borne in PROCHAINE_INTERACTIVE.iter() {
        borne.store(crate::kernel::echeances::JAMAIS, core::sync::atomic::Ordering::Relaxed);
    }
    let cur_balayage = current_index_raw();
    let source_irq = BALAYAGE_IRQ
        .get(local_cpu())
        .map_or(false, |b| b.load(core::sync::atomic::Ordering::Relaxed));
    // BOUCHAUD_ATTRIBUTION_REVEIL_V1 : qui tient le coeur pendant ce balayage.
    let par = {
        let cur = cur_balayage;
        let en_irq = crate::arch::x86_64::cpu_local::CpuId::from_index(local_cpu())
            .map_or(false, |id| crate::arch::x86_64::cpu_local::local(id).irq_depth() > 0);
        if cur < tasks().len() {
            let t = &tasks()[cur];
            crate::kernel::attribution_reveil::empaquete_par(t.process.pid as u64, t.tid as u64, t.noyau, en_irq)
        } else {
            0
        }
    };
    for index in 0..tasks().len() {
        let registre = tasks();
        let task = &registre[index];
        if task.wake_deadline_ns != 0
            && now >= task.wake_deadline_ns.charge()
            && task.state.reveille()
        {
            // BOUCHAUD_REVEIL_SANS_EFFACER_LA_CLE_V1 : l'echeance n'est PAS
            // effacee ici. La tache, reveillee, peut s'etre deja reparquee
            // avec une NOUVELLE echeance ; l'effacer apres coup la
            // transformerait en attente infinie. Chaque attente efface la
            // sienne en reprenant la main. Une echeance perimee sur une tache
            // Ready est sans effet : le reveil exige Blocked -> Ready.
            task.futex_key.range(0);
            let retard = now.saturating_sub(task.wake_deadline_ns.charge());
            if retard > SEUIL_TARDIF_NS {
                use core::sync::atomic::Ordering::Relaxed;
                if source_irq {
                    TARDIFS_IRQ.fetch_add(1, Relaxed);
                } else if cur_balayage == NO_TASK {
                    TARDIFS_INACTIF.fetch_add(1, Relaxed);
                } else if cur_balayage == index {
                    TARDIFS_SOI.fetch_add(1, Relaxed);
                } else {
                    TARDIFS_AUTRE.fetch_add(1, Relaxed);
                }
            }
            // BOUCHAUD_ATTRIBUTION_REVEIL_V1 : echeance -> pret.
            task.reveil_elu_ns.range(0);
            task.reveil_commute_ns.range(0);
            task.reveil_passe_ns.range(0);
            task.reveil_par.range(par);
            task.reveil_pret_ns.range(now);
            publish_ready(index);
        }
    }
    fire_alarms(crate::kernel::timer::ticks());
    recalcule_echeance();
}

/// Echeance du prochain `SIGALRM` par processus : (pid, tick).
static ALARMES: RankedSpinLock<Vec<(u32, u64)>> =
    RankedSpinLock::new(LockClass::SchedulerAlarms, Vec::new());

/// Programme (ou annule, avec 0) l'alarme du processus courant.
/// Renvoie l'echeance precedente, 0 s'il n'y en avait pas.
pub fn set_alarm(deadline: u64) -> u64 {
    let pid = current().process.pid;
    let mut list = ALARMES.lock();
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
        PROCHAINES_PAR_COEUR[0].arme(deadline.saturating_mul(par_tick));
    }
    previous
}

/// Echeance de l'alarme du processus courant (0 s'il n'y en a pas).
pub fn peek_alarm() -> u64 {
    let pid = current().process.pid;
    ALARMES.lock()
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
    let expired: Vec<u32> = {
        let mut alarmes = ALARMES.lock();
        let expired = alarmes
            .iter()
            .filter(|(_, deadline)| maintenant_ticks >= *deadline)
            .map(|(pid, _)| *pid)
            .collect();
        alarmes.retain(|(_, deadline)| maintenant_ticks < *deadline);
        expired
    };
    if expired.is_empty() {
        return;
    }
    for pid in expired {
        if let Some(process) = process_by_pid(pid) {
            process.signals.lock().raise(crate::kernel::signal::SIGALRM);
        }
        // BOUCHAUD_SIGNAL_INTERROMPT_POLL_V1 : `poll` compris.
        reveille_pour_signal(pid);
    }
}

/// Cede le CPU une fois (`sched_yield`).
pub fn yield_now() {
    schedule();
}

// BOUCHAUD_POINT_COOPERATIF_V1
//
// UN FIL NOYAU N'EST PAS COUPE PAR LE QUANTUM : il ne quitte son coeur qu'a
// ses points volontaires (BOUCHAUD_PREEMPTION_NOYAU_SURE_V1 -- le quantum
// noyau qui le coupait n'importe ou a ete retire apres une panique). Pendant
// qu'il tourne, le balayage des echeances ne passe pas non plus : une tache
// dont le sommeil est echu attend qu'il rende la main.
//
// Mesure (BOUCHAUD_ATTRIBUTION_REVEIL_V1, `ordonnanceur-probe` SMP1 TCG) :
// reveils de l'interface en retard de 6,7 a 13 ms, TOUS les tics presents,
// reveillee par `services-metrics` -- dont le releve de 5 s tient le coeur 5
// a 8 ms d'un seul tenant (26 ms au demarrage). Un releve par fenetre de
// 4,8 s : exactement une valeur isolee par phase.
//
// Le point cooperatif cede le coeur, a un endroit choisi par le fil (aucun
// verrou tenu), si c'est sur (`cession_volontaire_sure`), si le fil a tourne
// au moins `SEGMENT_MIN_NS` depuis son point precedent, et si quelqu'un
// attend : une tache prete sur ce coeur, ou une echeance de sommeil echue.

/// En deca, on ne cede pas : un passage dans l'ordonnanceur couterait plus
/// qu'il ne rend.
const SEGMENT_MIN_NS: u64 = 1_000_000;

static POINTS_PIRE_SEGMENT_NS: AtomicU64 = AtomicU64::new(0);
static POINTS_PIRE_SITE: AtomicU64 = AtomicU64::new(0);
static POINTS_CESSIONS: AtomicU64 = AtomicU64::new(0);

/// Point cooperatif d'un fil noyau long. `depuis_ns` : l'instant du point
/// precedent (ou du debut de l'activation), mis a jour ici.
///
/// BOUCHAUD_CESSION_INTERACTIVE_V1 : une INTERACTIVE en attente dans la file
/// de ce coeur fait ceder sans le plancher de `SEGMENT_MIN_NS`. Sa demande de
/// preemption, ordinaire, ne coupe pas un fil noyau : elle attendait la fin
/// d'un segment d'au moins une milliseconde. Run 39424044836 (KVM SMP4) : pire
/// reveil interactif sous charge 476 et 760 us, detection a l'heure
/// (`tardif_us=1`), attente pret -> elu 466 et 679 us -- QUALIFIE_ECHEC.
#[track_caller]
pub fn point_cooperatif(depuis_ns: &mut u64) -> bool {
    let maintenant = crate::kernel::timer::monotonic_ns();
    let segment = maintenant.saturating_sub(*depuis_ns);
    if segment > POINTS_PIRE_SEGMENT_NS.load(Ordering::Relaxed) {
        POINTS_PIRE_SEGMENT_NS.store(segment, Ordering::Relaxed);
        POINTS_PIRE_SITE.store(core::panic::Location::caller() as *const core::panic::Location<'static> as u64, Ordering::Relaxed);
    }
    let interactive_attend = crate::arch::x86_64::cpu_local::CpuId::from_index(local_cpu())
        .map(|id| crate::arch::x86_64::cpu_local::local(id).attente_file().0 > 0)
        .unwrap_or(false);
    if (segment < SEGMENT_MIN_NS && !interactive_attend)
        || !crate::kernel::scheduler::preempt::cession_volontaire_sure()
    {
        return false;
    }
    let attendu = interactive_attend
        || ready_count_cpu(local_cpu()) > 0
        || PROCHAINE_ECHEANCE.doit_balayer(maintenant);
    if !attendu {
        *depuis_ns = maintenant;
        return false;
    }
    POINTS_CESSIONS.fetch_add(1, Ordering::Relaxed);
    if segment < SEGMENT_MIN_NS {
        CESSIONS_INTERACTIVES.fetch_add(1, Ordering::Relaxed);
    }
    schedule();
    *depuis_ns = crate::kernel::timer::monotonic_ns();
    true
}

// BOUCHAUD_ATTENTE_INTERACTIVE_V1 : une interactive UTILISATEUR restee prete
// plus de `ATTENTE_INTERACTIVE_LONGUE_NS` avant son election -- derriere un fil
// noyau (que sa demande ordinaire ne coupe pas) ou derriere une tache
// utilisateur ? Cumuls et pires, poses a l'election (atomiques, aucune
// emission), publies par `reveil-precis` hors IRQ. Un compteur nul n'innocente
// rien : il dit seulement qu'aucune attente n'a passe le seuil.
const ATTENTE_INTERACTIVE_LONGUE_NS: u64 = 200_000;
static ATTENTES_DERRIERE_NOYAU: AtomicU64 = AtomicU64::new(0);
static ATTENTES_DERRIERE_UTILISATEUR: AtomicU64 = AtomicU64::new(0);
static PIRE_DERRIERE_NOYAU_NS: AtomicU64 = AtomicU64::new(0);
static PIRE_DERRIERE_UTILISATEUR_NS: AtomicU64 = AtomicU64::new(0);
/// Cessions d'un point cooperatif AVANT le plancher, pour une interactive.
static CESSIONS_INTERACTIVES: AtomicU64 = AtomicU64::new(0);
// BOUCHAUD_ATTENTE_INTERACTIVE_OCCUPANT_V1 : l'EMPLACEMENT de l'occupant du
// pire cas, de chaque cote. Run 39455898970 (KVM SMP4) : 5 096 us d'attente
// derriere « un fil noyau » -- lequel ? Sans nom, impossible de dire si c'est
// la console serie, un pilote ou un point cooperatif manquant. Un indice
// (une lecture de `CURRENT`, aucune emission, aucun verrou) ; le nom est
// resolu par `reveil-precis`, hors IRQ.
static PIRE_DERRIERE_NOYAU_OCCUPANT: AtomicUsize = AtomicUsize::new(NO_TASK);
static PIRE_DERRIERE_UTILISATEUR_OCCUPANT: AtomicUsize = AtomicUsize::new(NO_TASK);
// Le coeur d'election du pire cas, et bit 8 : la tache a ete VOLEE (publiee
// sur une autre file). Volee, elle a attendu ailleurs : l'occupant nomme est
// celui du voleur, pas celui qui l'a fait attendre.
static PIRE_DERRIERE_NOYAU_OU: AtomicUsize = AtomicUsize::new(0);
static PIRE_DERRIERE_UTILISATEUR_OU: AtomicUsize = AtomicUsize::new(0);
// Battements du vecteur de replanification recus par le coeur d'election
// pendant l'attente du pire cas : 0 = elue sans battement (blocage, point sur),
// 1 = elue au premier (une attente bien au-dela du quantum dit alors que ce
// battement a ete retarde), >= 2 = des battements sont passes sans l'elire.
static PIRE_DERRIERE_NOYAU_BATTEMENTS: AtomicU64 = AtomicU64::new(0);
static PIRE_DERRIERE_UTILISATEUR_BATTEMENTS: AtomicU64 = AtomicU64::new(0);
// Le dernier battement du coeur d'election au pire cas : intervalle depuis le
// precedent, part masquee, vol de l'hote (us), et la derniere raison pour
// laquelle un battement de ce coeur n'a pas commute.
static PIRE_DERRIERE_NOYAU_BATTEMENT: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static PIRE_DERRIERE_UTILISATEUR_BATTEMENT: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];

// BOUCHAUD_ATTENTE_INTERACTIVE_CLASSEE_V1 : TOUTE attente pret -> elu d'une
// interactive utilisateur au-dela d'une milliseconde, classee par ce qui l'a
// terminee. Volee : elle a attendu sur une autre file. b0 : elue sans
// battement (blocage, cession, point sur). b1 : elue au premier battement,
// lui-meme a l'heure (le quantum). b1r : elue au premier battement, mais il
// est arrive plus d'une milliseconde apres le quantum (interruption retardee :
// voir la part masquee et le vol du pire cas). b2 : au moins un battement est
// passe sans l'elire (politique ou renoncement). Le retard de b1r se mesure
// sur l'attente de la tache, pas sur l'intervalle entre battements (qui compte
// le repos du coeur).
const ATTENTE_LONGUE_CLASSEE_NS: u64 = 1_000_000;
static LONGUES_VOLEES: AtomicU64 = AtomicU64::new(0);
static LONGUES_B0: AtomicU64 = AtomicU64::new(0);
static LONGUES_B1: AtomicU64 = AtomicU64::new(0);
static LONGUES_B1_RETARD: AtomicU64 = AtomicU64::new(0);
static LONGUES_B2: AtomicU64 = AtomicU64::new(0);

// Pourquoi un battement n'a pas commute (derniere raison, par coeur).
pub const RENONCE_TRANSITION: u8 = 1;
pub const RENONCE_PORTE: u8 = 2;
pub const RENONCE_ANTICIPE: u8 = 3;
pub const RENONCE_FILE_VIDE: u8 = 4;
pub const RENONCE_GARDE: u8 = 5;
pub const RENONCE_RIEN: u8 = 6;
pub const RENONCE_MEME: u8 = 7;
pub const RENONCE_DIFFEREE_APPEL: u8 = 8;
pub const RENONCE_DIFFEREE_FIL_NOYAU: u8 = 9;
static RENONCE_RAISON: [AtomicU8; MAX_CPUS] = [const { AtomicU8::new(0) }; MAX_CPUS];
static RENONCES: [AtomicU64; 10] = [const { AtomicU64::new(0) }; 10];

#[inline]
fn note_renonce_battement(cpu: usize, raison: u8) {
    if cpu < MAX_CPUS {
        RENONCE_RAISON[cpu].store(raison, Ordering::Relaxed);
    }
    RENONCES[raison as usize % 10].fetch_add(1, Ordering::Relaxed);
}

/// Depuis le gestionnaire du vecteur de replanification (coeur local).
#[inline]
pub fn note_renonce_battement_local(raison: u8) {
    note_renonce_battement(local_cpu(), raison);
}

fn nom_renonce(code: u64) -> &'static str {
    match code as u8 {
        RENONCE_TRANSITION => "transition",
        RENONCE_PORTE => "porte",
        RENONCE_ANTICIPE => "anticipe",
        RENONCE_FILE_VIDE => "file_vide",
        RENONCE_GARDE => "garde",
        RENONCE_RIEN => "rien",
        RENONCE_MEME => "meme",
        RENONCE_DIFFEREE_APPEL => "differee_appel",
        RENONCE_DIFFEREE_FIL_NOYAU => "differee_fil_noyau",
        _ => "-",
    }
}

/// A l'election d'une interactive utilisateur : `attente_ns` pret -> elu,
/// `derriere_noyau` -- l'occupant precedent du coeur etait un fil noyau,
/// `occupant` -- son emplacement.
fn note_attente_interactive(
    attente_ns: u64,
    derriere_noyau: bool,
    occupant: usize,
    cpu: usize,
    volee: bool,
    battements: u64,
) {
    if attente_ns <= ATTENTE_INTERACTIVE_LONGUE_NS {
        return;
    }
    let (intervalle, masque, vole) = crate::kernel::attribution_reveil::dernier_battement(cpu);
    if attente_ns > ATTENTE_LONGUE_CLASSEE_NS {
        let classe = if volee {
            &LONGUES_VOLEES
        } else if battements == 0 {
            &LONGUES_B0
        } else if battements == 1 {
            // BOUCHAUD_B1_RETARD_SANS_REPOS_V1 : le battement est en retard s'il
            // arrive plus d'une milliseconde apres le quantum COMPTE DEPUIS QUE
            // LA TACHE EST PRETE -- son attente. L'intervalle depuis le battement
            // precedent comptait aussi le repos du coeur (aucun battement au
            // repos) : run 39503091750, KVM SMP4, `b1_retard` sur usb-hid avec
            // `int21382` (21 ms d'intervalle) pour une attente de quelques ms.
            let quantum_ns = crate::arch::x86_64::smp::SCHED_QUANTUM_TICKS * 1_000_000;
            if attente_ns > quantum_ns + 1_000_000 { &LONGUES_B1_RETARD } else { &LONGUES_B1 }
        } else {
            &LONGUES_B2
        };
        classe.fetch_add(1, Ordering::Relaxed);
    }
    let (compte, pire, qui, ou, bat, detail) = if derriere_noyau {
        (&ATTENTES_DERRIERE_NOYAU, &PIRE_DERRIERE_NOYAU_NS, &PIRE_DERRIERE_NOYAU_OCCUPANT,
         &PIRE_DERRIERE_NOYAU_OU, &PIRE_DERRIERE_NOYAU_BATTEMENTS, &PIRE_DERRIERE_NOYAU_BATTEMENT)
    } else {
        (&ATTENTES_DERRIERE_UTILISATEUR, &PIRE_DERRIERE_UTILISATEUR_NS,
         &PIRE_DERRIERE_UTILISATEUR_OCCUPANT, &PIRE_DERRIERE_UTILISATEUR_OU,
         &PIRE_DERRIERE_UTILISATEUR_BATTEMENTS, &PIRE_DERRIERE_UTILISATEUR_BATTEMENT)
    };
    compte.fetch_add(1, Ordering::Relaxed);
    // Deux pires concurrents peuvent croiser leurs occupants : diagnostic
    // seulement, aucun verdict n'en depend.
    if pire.fetch_max(attente_ns, Ordering::Relaxed) < attente_ns {
        qui.store(occupant, Ordering::Relaxed);
        ou.store((cpu & 0xff) | ((volee as usize) << 8), Ordering::Relaxed);
        bat.store(battements, Ordering::Relaxed);
        detail[0].store(intervalle / 1000, Ordering::Relaxed);
        detail[1].store(masque / 1000, Ordering::Relaxed);
        detail[2].store(if vole == u64::MAX { u64::MAX } else { vole / 1000 }, Ordering::Relaxed);
        detail[3].store(
            if cpu < MAX_CPUS { RENONCE_RAISON[cpu].load(Ordering::Relaxed) as u64 } else { 0 },
            Ordering::Relaxed,
        );
    }
}

/// `[occupant derriere noyau, occupant derriere utilisateur]` du pire cas,
/// `tid:nom@cpuN:bK[:volee]` (K battements pendant l'attente ; nom sans espace), `-` si aucun ou si l'emplacement est vide.
/// L'emplacement a pu etre recycle depuis : le nom est celui de son occupant
/// AU RELEVE -- un fil noyau, lui, ne meurt guere.
pub fn occupants_attente_interactive() -> [String; 2] {
    [(&PIRE_DERRIERE_NOYAU_OCCUPANT, &PIRE_DERRIERE_NOYAU_OU, &PIRE_DERRIERE_NOYAU_BATTEMENTS,
      &PIRE_DERRIERE_NOYAU_BATTEMENT),
     (&PIRE_DERRIERE_UTILISATEUR_OCCUPANT, &PIRE_DERRIERE_UTILISATEUR_OU,
      &PIRE_DERRIERE_UTILISATEUR_BATTEMENTS, &PIRE_DERRIERE_UTILISATEUR_BATTEMENT)].map(|(qui, ou, bat, detail)| {
        let index = qui.load(Ordering::Relaxed);
        if index == NO_TASK {
            return String::from("-");
        }
        let ou = ou.load(Ordering::Relaxed);
        let (cpu, volee) = (ou & 0xff, ou >> 8 != 0);
        let table = tasks();
        match table.get(index) {
            Some(tache) => {
                let nom = tache.process.metadata.lock().name.replace(' ', "_");
                let vole = detail[2].load(Ordering::Relaxed);
                alloc::format!(
                    "{}:{}@cpu{}:b{}{}:int{}:masq{}:vol{}:renonce_{}",
                    tache.tid, nom, cpu, bat.load(Ordering::Relaxed),
                    if volee { ":volee" } else { "" },
                    detail[0].load(Ordering::Relaxed), detail[1].load(Ordering::Relaxed),
                    if vole == u64::MAX { alloc::string::String::from("?") } else { alloc::format!("{}", vole) },
                    nom_renonce(detail[3].load(Ordering::Relaxed)),
                )
            }
            None => String::from("-"),
        }
    })
}

/// Attentes de plus d'une milliseconde, classees (BOUCHAUD_ATTENTE_INTERACTIVE_CLASSEE_V1) :
/// `[volees, b0, b1, b1_retard, b2]`, cumuls depuis le demarrage.
pub fn attentes_longues_classees() -> [u64; 5] {
    [
        LONGUES_VOLEES.load(Ordering::Relaxed),
        LONGUES_B0.load(Ordering::Relaxed),
        LONGUES_B1.load(Ordering::Relaxed),
        LONGUES_B1_RETARD.load(Ordering::Relaxed),
        LONGUES_B2.load(Ordering::Relaxed),
    ]
}

/// Le dernier battement d'un coeur, vu d'ailleurs (diagnostic) : `(nombre
/// d'IPI de quantum recues, adresse interrompue, interrompue en espace
/// utilisateur, derniere raison de ne pas commuter -- code `RENONCE_*`)`.
pub fn dernier_battement(cpu: usize) -> (u64, u64, bool, u8) {
    if cpu >= MAX_CPUS {
        return (0, 0, false, 0);
    }
    (
        STALL_IPI_COUNT[cpu].load(Ordering::Acquire),
        STALL_IPI_RIP[cpu].load(Ordering::Acquire),
        STALL_IPI_USER[cpu].load(Ordering::Acquire) != 0,
        RENONCE_RAISON[cpu].load(Ordering::Relaxed),
    )
}

/// Battements qui n'ont pas commute, par raison (indice = code `RENONCE_*`).
pub fn renonces_battement() -> [u64; 10] {
    core::array::from_fn(|i| RENONCES[i].load(Ordering::Relaxed))
}

/// `[derriere noyau, derriere utilisateur, pire noyau ns, pire utilisateur ns,
/// cessions interactives]`, cumuls depuis le demarrage.
pub fn compteurs_attente_interactive() -> [u64; 5] {
    [
        ATTENTES_DERRIERE_NOYAU.load(Ordering::Relaxed),
        ATTENTES_DERRIERE_UTILISATEUR.load(Ordering::Relaxed),
        PIRE_DERRIERE_NOYAU_NS.load(Ordering::Relaxed),
        PIRE_DERRIERE_UTILISATEUR_NS.load(Ordering::Relaxed),
        CESSIONS_INTERACTIVES.load(Ordering::Relaxed),
    ]
}

/// `(pire segment ns, site du point qui l'a termine, cessions)` depuis le
/// releve precedent ; remet le pire a zero.
pub fn releve_points_cooperatifs() -> (u64, u64, u64) {
    (
        POINTS_PIRE_SEGMENT_NS.swap(0, Ordering::Relaxed),
        POINTS_PIRE_SITE.swap(0, Ordering::Relaxed),
        POINTS_CESSIONS.load(Ordering::Relaxed),
    )
}

/// BOUCHAUD_CONSOLE_REPLI_EXPLIQUE_V1 -- diagnostic hors IRQ : l'indice de la
/// tache courante (pour qu'un fil noyau se nomme), l'occupant d'un coeur, et
/// `tid:nom etat=E on_cpu=C file=F` d'une tache. Lectures seules.
pub fn indice_tache_courante() -> usize {
    current_index_raw()
}

pub fn occupant_coeur(cpu: usize) -> usize {
    if cpu >= MAX_CPUS { NO_TASK } else { CURRENT[cpu].load(Ordering::Acquire) }
}

/// Etat brut d'une tache, sans allocation (diagnostic pris dans n'importe
/// quel contexte) : `(etat, on_cpu, file)` ; etat 0 prete, 1 bloquee,
/// 2 zombie, 9 introuvable.
pub fn etat_tache_brut(index: usize) -> (u8, i8, u8) {
    if index == NO_TASK {
        return (9, -1, 0);
    }
    let table = tasks();
    match table.get(index) {
        Some(t) => (t.state.charge().code(), t.on_cpu.charge(), t.runq_cpu.charge()),
        None => (9, -1, 0),
    }
}

pub fn decrit_tache(index: usize) -> String {
    if index == NO_TASK {
        return String::from("-");
    }
    let table = tasks();
    match table.get(index) {
        Some(t) => alloc::format!(
            "{}:{} etat={:?} on_cpu={} file={} noyau={}",
            t.tid,
            t.process.metadata.lock().name.replace(' ', "_"),
            t.state.charge(),
            t.on_cpu.charge(),
            t.runq_cpu.charge(),
            t.noyau as u8,
        ),
        None => String::from("?"),
    }
}
