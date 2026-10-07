/// BOUCHAUD_COMPTA_SEQLOCK_V1 -- section d'ecriture de la comptabilite CPU.
///
/// Run 37589903681 (KVM) : panique « task: runtime > fenetre tid=118
/// delta=35786689 window=33031990 ». `mesure_processus` lisait
/// `last_account_ns` (ancien) puis les compteurs (deja credites de la
/// tranche) : la tranche comptait deux fois. Chaque champ est atomique, leur
/// ensemble ne l'etait pas.
///
/// La sequence `compta_seq` est impaire pendant la section ; un lecteur
/// (`lecture_compta`) relit tant qu'il la voit impaire ou changee.
///
/// UN SEUL ECRIVAIN, par construction :
///   * toute section porte sur la tache COURANTE du coeur qui l'execute
///     (`account_slice_end` a la commutation, au repli d'idle et a l'abandon ;
///     `finalise_task_running` apres la revendication CAS de `on_cpu` ;
///     `rearme_compta_apres_idle`). Deux coeurs ne peuvent pas tenir la meme
///     tache : `on_cpu` passe de -1 a un coeur par compare-exchange, et ne
///     revient a -1 qu'APRES `account_slice_end` ;
///   * le meme coeur ne peut pas s'interrompre lui-meme au milieu : la section
///     masque les interruptions. Sans cela, `attends_interruption` (qui
///     replie avec IF=1) pouvait etre preemptee par le minuteur, dont
///     `account_slice_end` ouvrait une seconde section imbriquee sur la meme
///     tache -- la sequence redevenait paire en pleine ecriture ;
///   * migration, reveil distant, signal, recolte d'un zombie, recyclage
///     d'emplacement : aucun n'ecrit ces champs (verifie par
///     `tools/verifie-compta-seqlock.py`).
///
/// La section couvre aussi le bloc par CPU (`COMPTA_DEBUT_NS`,
/// `COMPTA_USER_NS`, `COMPTA_NOYAU_NS`, `COMPTA_EN_NOYAU`) du coeur de la
/// tache tant qu'elle y est installee ; les frontieres d'appel systeme
/// (`frontiere_compta`), qui ne connaissent pas la tache, l'ecrivent sous la
/// sequence du coeur (`COMPTA_SEQ_CPU`). Le lecteur y prend
/// le partage utilisateur/noyau de la tranche vive ; sans cela il attribuait
/// toute la tranche au cote de `in_kernel` -- perime depuis le dernier repli
/// -- et `utime` reculait au repli suivant (sonde compta-stress, QEMU :
/// « utime=6<-8 stime=12<-7 »).
///
/// Ces champs-la sont `Relaxed` : les barrieres encadrent les donnees
/// (ecrivain : ouverture, `fence(Release)`, donnees ; lecteur : donnees,
/// `fence(Acquire)`, seconde lecture de la sequence).
///
/// L'assertion de parite attrape tout ecrivain qui violerait ces regles.
#[inline]
fn compta_section<R>(task: &Task, corps: impl FnOnce() -> R) -> R {
    interrupts::without_interrupts(|| {
        let seq = task.compta_seq.charge();
        debug_assert!(seq & 1 == 0, "compta: ecrivain concurrent tid={} seq={}", task.tid, seq);
        // Le coeur local : celui de la tache, seul a ecrire son bloc et ses
        // cumuls (`CUMUL_USER_NS`, `CUMUL_NOYAU_NS`), lus sous cette sequence
        // par `proc_cpu_cumul`.
        let cpu = local_cpu();
        let seq_cpu = COMPTA_SEQ_CPU[cpu].load(Ordering::Relaxed);
        debug_assert!(seq_cpu & 1 == 0, "compta: section imbriquee cpu={} seq={}", cpu, seq_cpu);
        task.compta_seq.range(seq.wrapping_add(1));
        COMPTA_SEQ_CPU[cpu].store(seq_cpu.wrapping_add(1), Ordering::Relaxed);
        core::sync::atomic::fence(Ordering::Release);
        let rendu = corps();
        COMPTA_SEQ_CPU[cpu].store(seq_cpu.wrapping_add(2), Ordering::Release);
        task.compta_seq.range(seq.wrapping_add(2));
        rendu
    })
}

/// Avance une seule fois le curseur CPU de la tâche jusqu'à `now`.
///
/// Toutes les frontières (syscall, préemption, blocage) utilisent le même
/// curseur. Une seconde frontière au même instant voit donc un delta nul au
/// lieu de recompter la tranche précédente. A appeler dans `compta_section`.
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
    // LE MEME TEMPS, DANS UN COMPTEUR QUE LA MORT NE PEUT PAS REPRENDRE.
    //
    // Voir `CUMUL_USER_NS`. C'est ici, et nulle part ailleurs, que le temps
    // est deja decoupe en utilisateur et noyau ET rattache a un processeur
    // precis. L'addition se fait sur le CPU local, seul a ecrire dans sa
    // case : aucune contention entre coeurs.
    CUMUL_USER_NS[cpu].store(
        CUMUL_USER_NS[cpu].load(Ordering::Relaxed).saturating_add(user),
        Ordering::Relaxed,
    );
    CUMUL_NOYAU_NS[cpu].store(
        CUMUL_NOYAU_NS[cpu].load(Ordering::Relaxed).saturating_add(noyau),
        Ordering::Relaxed,
    );
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
    // BOUCHAUD_P0_REVEIL_CIBLE_V1 : le budget d'activation.
    //
    // Une somme, pas une moyenne : la moyenne glissante ci-dessus repond a
    // « cette tache est-elle couteuse en general », le budget repond a « cette
    // activation-ci a-t-elle depasse la borne ». Un fil d'entree periodique
    // qui fait 162 us par tour ne doit pas perdre son privilege parce qu'un
    // tour, une fois, a coute davantage. `publish_ready` le lit et le remet a
    // zero : sa valeur est donc toujours celle de l'activation qui vient de
    // s'achever.
    task.budget_reveil_ns
        .range(task.budget_reveil_ns.charge().saturating_add(elapsed));
    // `in_kernel` doit survivre a un changement de contexte AU MILIEU d'un
    // appel systeme : une tache qui se bloque dans un `futex` repart du cote
    // noyau. On le range donc dans la tache au repli, et `mark_task_running` le
    // ressort au reveil.
    task.in_kernel.range(COMPTA_EN_NOYAU[cpu].load(Ordering::Relaxed));
    COMPTA_DEBUT_NS[cpu].store(now, Ordering::Relaxed);
    task.last_account_ns.range(now);
    task.slice_start_ns.range(now);
}

/// Tue une tache : la courante meurt sur-le-champ, une autre est CONDAMNEE.
///
/// BOUCHAUD_CYCLE_DE_VIE_V1 -- point de passage unique de toute mort, voulue
/// (`exit`, retraite) ou imposee (`exit_group`, `execve`, fin de session,
/// `tue_processus`). La regle est `kernel::cycle_vie` :
///
///   * la tache COURANTE de ce coeur passe a `Zombie` elle-meme ;
///   * une AUTRE tache n'est jamais passee a `Zombie` par simple ecriture.
///     Elle est condamnee, et `condamne` decide : tuee sur place si elle est
///     parquee dans une attente interruptible, reveillee si elle est bloquee
///     sur son coeur, laissee a sa frontiere sinon.
///
/// Avant ce lot, l'etat etait ECRIT : une tache tuee en entrant dans
/// `nanosleep` ecrasait `Zombie` par `Blocked`, et revenait en espace
/// utilisateur une fois son processus recolte.
fn marque_zombie(task: &Task) {
    let courante = try_current().map(|c| c.tid) == Some(task.tid);
    if courante {
        meurt_soi_meme(task);
    } else {
        condamne(task);
    }
}

/// La tache courante meurt : `Zombie`, et le drapeau de retraite de son coeur.
// Ne touche plus que des atomiques : une reference PARTAGEE suffit.
fn meurt_soi_meme(task: &Task) {
    // IL N'Y A PLUS DE PHOTOGRAPHIE DES COMPTEURS ICI, ET C'EST UNE CORRECTION.
    //
    // BOUCHAUD_C33_L_ECART_INEXPLIQUE
    //
    // `TEMPS_MORT_NS` additionnait ici `user_cpu_ns + kernel_cpu_ns` pour
    // chiffrer ce que la somme des vivants allait perdre. La mesure a montre
    // que ce chiffre est FAUX, et de beaucoup :
    //
    //     ecart_ms=1025   zombies_ms=1030   temps_mort_ms=0
    //
    // Mille trente millisecondes tenues par un zombie, et la photographie en
    // annoncait zero. La raison tient a l'instant choisi : a cet endroit, la
    // tache est encore SUR SON PROCESSEUR et la tranche ouverte n'est pas
    // repliee. Une tache qui brule du temps utilisateur sans faire le moindre
    // appel systeme -- exactement la charge d'epreuve de ce banc -- n'a donc
    // presque rien dans ses compteurs au moment ou elle meurt. La seconde
    // d'avant lui est imputee juste apres, par `account_until`, qui alimente
    // le cumulatif ET les compteurs de la tache : le cumulatif la garde, la
    // somme des vivants ne la voit plus, et la photographie l'a ratee.
    //
    // Ce que la somme des vivants a perdu se LIT desormais, au lieu d'etre
    // photographie : `proc_cpu_zombies` additionne les compteurs des zombies
    // au moment de la lecture -- donc complets -- et `TEMPS_RECYCLE_NS`
    // ramasse ceux dont l'emplacement a ete reutilise. Les deux couvrent
    // exactement les deux facons de quitter la somme des vivants.
    if task.state.meurt() {
        // BOUCHAUD_CONTINUATION_SYNCHRONE_V1 : publie APRES l'etat ; qui lit
        // le compteur (Acquire) voit donc le zombie.
        MORTS.fetch_add(1, Ordering::Release);
    }
    if task.on_cpu >= 0 && !task.switching_out.charge() {
        let cpu = task.on_cpu.charge() as usize;
        if cpu < MAX_CPUS {
            RETRAITE_DEMANDEE[cpu].store(true, Ordering::Release);
        }
    }
}

// BOUCHAUD_CYCLE_DE_VIE_V1 : ce que les tueurs ont fait, par issue.
static CONDAMNATIONS: AtomicU64 = AtomicU64::new(0);
static TUEES_PARQUEES: AtomicU64 = AtomicU64::new(0);
static REVEILLEES_POUR_MOURIR: AtomicU64 = AtomicU64::new(0);
static MORTES_A_LA_FRONTIERE: AtomicU64 = AtomicU64::new(0);
static MORTES_AU_PARKING: AtomicU64 = AtomicU64::new(0);
/// Attentes refusees parce que la tache n'etait plus prete. Zero est
/// l'invariant : seule une tache vivante s'endort, et une tache morte ne
/// s'execute plus.
static ENDORMISSEMENTS_REFUSES: AtomicU64 = AtomicU64::new(0);

/// condamnations, tuees parquees, reveillees pour mourir, mortes a la
/// frontiere, mortes au parking, endormissements refuses.
pub fn compteurs_cycle_vie() -> (u64, u64, u64, u64, u64, u64) {
    (
        CONDAMNATIONS.load(Ordering::Relaxed),
        TUEES_PARQUEES.load(Ordering::Relaxed),
        REVEILLEES_POUR_MOURIR.load(Ordering::Relaxed),
        MORTES_A_LA_FRONTIERE.load(Ordering::Relaxed),
        MORTES_AU_PARKING.load(Ordering::Relaxed),
        ENDORMISSEMENTS_REFUSES.load(Ordering::Relaxed),
    )
}

#[inline]
fn etat_cycle_vie(etat: TaskState) -> crate::kernel::cycle_vie::Etat {
    match etat {
        TaskState::Ready => crate::kernel::cycle_vie::Etat::Pret,
        TaskState::Blocked => crate::kernel::cycle_vie::Etat::Bloque,
        TaskState::Zombie => crate::kernel::cycle_vie::Etat::Zombie,
    }
}

/// Condamne une tache qui n'est pas la courante de ce coeur.
///
/// Ordre : la condamnation est publiee (`SeqCst`) AVANT la lecture de l'etat.
/// La tache, elle, publie `Blocked` avant de relire sa condamnation : au
/// moins l'un des deux voit l'autre (`kernel::cycle_vie`, la fenetre du
/// parking).
fn condamne(task: &Task) {
    use crate::kernel::cycle_vie::{action_tueur, ActionTueur, VueTueur};
    if task.condamnee.condamne() {
        CONDAMNATIONS.fetch_add(1, Ordering::Relaxed);
    }
    loop {
        let sur_coeur = task.on_cpu >= 0 || task.switching_out.charge();
        let vue = VueTueur {
            etat: etat_cycle_vie(task.state.charge()),
            sur_coeur,
            interruptible: task.attente_interruptible.charge(),
        };
        match action_tueur(vue) {
            ActionTueur::Rien | ActionTueur::AttendSonEvenement => return,
            ActionTueur::Frontiere => {
                // Elle s'execute peut-etre : prevenir son coeur, et le forcer
                // a entrer dans le noyau si elle est en espace utilisateur.
                let cpu = task.on_cpu.charge();
                if cpu >= 0 && (cpu as usize) < MAX_CPUS {
                    RETRAITE_DEMANDEE[cpu as usize].store(true, Ordering::Release);
                    if cpu as usize != local_cpu() {
                        smp::reschedule_cpu(cpu as usize);
                    }
                }
                return;
            }
            ActionTueur::TueSurPlace => {
                if task.state.tue_parquee() {
                    TUEES_PARQUEES.fetch_add(1, Ordering::Relaxed);
                    MORTS.fetch_add(1, Ordering::Release);
                    return;
                }
                // Un reveilleur l'a remise prete entre-temps : redecider.
            }
            ActionTueur::Reveille => {
                if task.state.reveille() {
                    REVEILLEES_POUR_MOURIR.fetch_add(1, Ordering::Relaxed);
                    // Meme publication qu'un reveilleur ordinaire : la
                    // passation la republiera si elle est encore sur son
                    // coeur.
                    for index in 0..tasks().len() {
                        if tasks()[index].tid == task.tid {
                            publish_ready(index);
                            break;
                        }
                    }
                    return;
                }
            }
        }
        core::hint::spin_loop();
    }
}

/// La tache courante, condamnee, meurt a une frontiere (retour d'appel
/// systeme, de faute, preemption depuis l'espace utilisateur, premier
/// passage en espace utilisateur). Ne revient pas.
fn meurt_a_la_frontiere() -> ! {
    MORTES_A_LA_FRONTIERE.fetch_add(1, Ordering::Relaxed);
    meurt_soi_meme(current());
    retire_exec_zombie_current()
}

/// La tache courante est-elle condamnee ? Une lecture `SeqCst` de son drapeau.
pub fn condamnee_courante() -> bool {
    match try_current() {
        Some(task) => task.condamnee.est_condamnee(),
        None => false,
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
    let table = tasks();
    let Some(task) = table.get(index) else {
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
    let table = tasks();
    let Some(task) = table.get(index) else {
        return;
    };
    compta_section(task, || {
        task.last_account_ns.range(now);
        task.slice_start_ns.range(now);
        COMPTA_DEBUT_NS[cpu].store(now, Ordering::Relaxed);
        COMPTA_USER_NS[cpu].store(0, Ordering::Relaxed);
        COMPTA_NOYAU_NS[cpu].store(0, Ordering::Relaxed);
        COMPTA_EN_NOYAU[cpu].store(task.in_kernel.charge(), Ordering::Relaxed);
    });
}

fn account_slice_end(task: &Task) {
    let now = crate::kernel::timer::monotonic_ns();
    // Une seule section : la tranche close ET le curseur remis a zero. Un
    // lecteur entre les deux compterait sinon du temps hors processeur.
    compta_section(task, || {
        account_until(task, now);
        task.last_account_ns.range(0);
        task.slice_start_ns.range(0);
        // Le CPU n'a plus de tache a qui imputer le temps : on desarme, sinon
        // le premier repli de la tache SUIVANTE lui attribuerait le temps
        // passe entre les deux.
        let cpu = local_cpu();
        COMPTA_DEBUT_NS[cpu].store(0, Ordering::Relaxed);
        COMPTA_USER_NS[cpu].store(0, Ordering::Relaxed);
        COMPTA_NOYAU_NS[cpu].store(0, Ordering::Relaxed);
    });
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

// BOUCHAUD_C66_LE_TEMPS_DE_FAUTE_ETAIT_COMPTE_EN_UTILISATEUR
//
// `account_kernel_enter` / `account_kernel_exit` n'etaient appelees QUE depuis
// `usermode.rs`, autour du corps d'un appel systeme. Le gestionnaire de faute
// de page n'en contenait aucune. Tout ce qu'il fait -- allouer une trame,
// attendre le cache de pages, DECLENCHER ET ATTENDRE UNE LECTURE ATA, recopier
// la page, poser la traduction -- se deroulait donc avec `COMPTA_EN_NOYAU`
// reste a `false`, et tombait dans `user_ns`.
//
// Consequence directe sur la campagne de mesure en cours : le releve
//
//     user_ms=92250 sys_ms=671
//
// avait ete lu comme « ce segment est limite par le CPU en espace utilisateur,
// donc ce n'est pas de l'attente disque ». Cette lecture ne tenait pas : la
// seule chose que `sys_ms` mesurait, c'etait le corps des appels systeme. Une
// attente de lecture de plusieurs secondes dans une faute de page apparaissait
// comme du calcul utilisateur.
//
// # Pourquoi une SAUVEGARDE, et pas `account_kernel_enter`
//
// Une faute peut survenir alors qu'on est deja du cote noyau. Poser `true` a
// l'entree puis `false` a la sortie ferait basculer le CPU du mauvais cote
// pour tout le reste de l'appel systeme englobant. Ces deux fonctions rendent
// donc l'etat PRECEDENT et le restaurent, au lieu d'ecrire une valeur absolue.
//
// # Pourquoi pas une garde RAII
//
// Le gestionnaire de faute se termine parfois sans retourner : `kill_faulting_
// task` et `releve_faute_fatale` ne rendent pas la main. Une garde `Drop` n'y
// serait jamais executee -- c'est exactement le defaut qui avait corrompu la
// comptabilite des domaines du gros verrou sur les chemins sans retour de
// `sys_execve`. On pose donc un couple entrer/sortir explicite.
//
// Et si l'on sort quand meme sans restaurer, sur un chemin de mort : la tache
// est tuee, et `install()` reecrit `COMPTA_EN_NOYAU` depuis le `in_kernel` de
// la tache entrante au prochain changement de contexte. L'etat se repare de
// lui-meme a la premiere commutation ; il ne peut pas rester faux durablement.

/// Le cote du mur user/noyau tel qu'il etait AVANT la faute.
///
/// `#[must_use]` : oublier de le rendre a `account_fault_exit` laisserait le
/// CPU marque « en noyau » pour la suite de la tranche.
#[must_use = "a rendre a account_fault_exit, sinon le mur user/noyau reste fausse"]
pub struct MurAvantFaute(bool);

/// Entre dans le gestionnaire de faute : le temps qui suit est du temps noyau.
pub fn account_fault_enter() -> MurAvantFaute {
    MurAvantFaute(frontiere_compta(true))
}

/// Sort du gestionnaire de faute et rend au CPU le cote qu'il avait avant.
pub fn account_fault_exit(avant: MurAvantFaute) {
    frontiere_compta(avant.0);
}

/// Ferme le fragment en cours et ouvre le suivant, du cote demande.
///
/// Les interruptions sont coupees : sans cela, le numero de CPU pourrait etre
/// lu ici et les compteurs mis a jour la-bas, apres une migration. C'est le
/// meme raisonnement que pour `identite_courante`, et c'est tout ce qu'il faut
/// -- aucun autre CPU n'ecrit dans notre case.
///
/// Rend le cote sur lequel on se trouvait AVANT l'appel. Les frontieres
/// d'appel systeme l'ignorent -- elles savent de quel cote elles vont. Les
/// fautes de page s'en servent pour restaurer au lieu d'ecraser.
fn frontiere_compta(vers_noyau: bool) -> bool {
    interrupts::without_interrupts(|| {
        // BOUCHAUD_COMPTA_SEQLOCK_V1 : le bloc se lit sous la sequence de SON
        // coeur (`COMPTA_SEQ_CPU`). Ce coeur en est le seul ecrivain, IRQ
        // masquees ; rien d'autre qu'un compteur local n'est touche ici.
        let cpu = local_cpu();
        let seq = COMPTA_SEQ_CPU[cpu].load(Ordering::Relaxed);
        debug_assert!(seq & 1 == 0, "compta: frontiere dans une section cpu={} seq={}", cpu, seq);
        COMPTA_SEQ_CPU[cpu].store(seq.wrapping_add(1), Ordering::Relaxed);
        core::sync::atomic::fence(Ordering::Release);
        let avant = frontiere_compta_bloc(vers_noyau);
        COMPTA_SEQ_CPU[cpu].store(seq.wrapping_add(2), Ordering::Release);
        avant
    })
}

fn frontiere_compta_bloc(vers_noyau: bool) -> bool {
    let cpu = local_cpu();
    let now = crate::kernel::timer::monotonic_ns();
    let debut = COMPTA_DEBUT_NS[cpu].load(Ordering::Relaxed);
    let avant = COMPTA_EN_NOYAU[cpu].load(Ordering::Relaxed);
    if debut != 0 {
        let ecoule = now.saturating_sub(debut);
        if avant {
            COMPTA_NOYAU_NS[cpu].fetch_add(ecoule, Ordering::Relaxed);
        } else {
            COMPTA_USER_NS[cpu].fetch_add(ecoule, Ordering::Relaxed);
        }
        COMPTA_DEBUT_NS[cpu].store(now, Ordering::Relaxed);
    }
    COMPTA_EN_NOYAU[cpu].store(vers_noyau, Ordering::Relaxed);
    avant
}

pub fn account_resume_user_noreturn() {
    account_kernel_exit();
}

// BOUCHAUD_C68_OU_VA_LE_TEMPS_NOYAU
//
// Le run 35944547625 a rendu le partage honnete, et le resultat est net :
//
//     WebWorker #1 (froid)   user_ms=724   sys_ms=113594
//
// Mais le livre des fautes n'explique que 13 285 ms de ces 113 594. Deux
// candidats pour les cent secondes restantes ont ete poses puis REFUTES sur
// banc local en quelques minutes -- `CACHE_BALAYAGE appels=0` et
// `FAULT_REPRISE reprises=0`. Ni le balayage de secours du cache, ni la
// chaine de reprise des fautes.
//
// Reste le temps noyau qui n'est pas de la faute : le CORPS DES APPELS
// SYSTEME. Aucune sonde ne le decoupait. Celle-ci le fait, par numero.
//
// # Ce qu'elle ne mesure PAS
//
// Les appels qui ne rendent pas la main -- `execve` reussi, `exit` -- ne
// passent jamais par la borne de sortie. Leur cout est absent de cette table,
// et c'est volontaire : `PERF_EXECVE` le mesure deja, avec ses propres
// bornes. Compter ici une duree jamais fermee serait pire que ne rien
// compter.
//
// # CE QU'ELLE MESURE, ET CE QU'ELLE NE MESURE PAS
//
// Du temps ECOULE entre les deux bornes, pas du temps CPU. Un appel qui
// BLOQUE -- `poll`, `futex`, `read` -- compte son attente. Le run
// 35955074619 le montre sans ambiguite :
//
//     poll   ecoule_ms=4002863  appels=10007  moyen_us=400006
//
// Quatre cents millisecondes par appel, et 4405 secondes de total sur un run
// de 387 secondes. Ce total n'est donc PAS comparable a `sys_ms`, et il ne
// peut pas servir a attribuer du temps processeur. La ligne porte
// `mesure=ecoule_inclut_blocage` pour qu'on ne s'y trompe pas.
//
// # Ce qu'elle coute
//
// Deux lectures d'horloge par appel systeme, en plus des deux que les bornes
// user/noyau font deja. Sur une charge a cent mille appels, c'est du bruit ;
// la table est publiee avec son nombre d'appels pour qu'on puisse le verifier
// plutot que le croire.
const SYSCALL_MAX: usize = 512;
static SYSCALL_NS: [AtomicU64; SYSCALL_MAX] = [const { AtomicU64::new(0) }; SYSCALL_MAX];
static SYSCALL_N: [AtomicU64; SYSCALL_MAX] = [const { AtomicU64::new(0) }; SYSCALL_MAX];
/// Les appels dont le numero depasse la table. Non nul = la table ment par omission.
static SYSCALL_HORS_TABLE: AtomicU64 = AtomicU64::new(0);

/// Impute `ecoule` au numero `nr`. Appele par la borne de sortie d'appel systeme.
pub fn impute_syscall(nr: u64, ecoule: u64) {
    let index = nr as usize;
    if index >= SYSCALL_MAX {
        SYSCALL_HORS_TABLE.fetch_add(1, Ordering::Relaxed);
        return;
    }
    SYSCALL_NS[index].fetch_add(ecoule, Ordering::Relaxed);
    SYSCALL_N[index].fetch_add(1, Ordering::Relaxed);
}

/// Publie les `combien` appels systeme les plus couteux, en temps cumule.
///
/// GLOBAL, et dit comme tel : ce sont tous les processus depuis le demarrage.
pub fn publie_syscall_top(combien: usize) {
    let mut total_ns = 0u64;
    let mut total_n = 0u64;
    for i in 0..SYSCALL_MAX {
        total_ns = total_ns.saturating_add(SYSCALL_NS[i].load(Ordering::Relaxed));
        total_n = total_n.saturating_add(SYSCALL_N[i].load(Ordering::Relaxed));
    }
    crate::kernel::dmesg::log_fmt(format_args!(
        "SYSCALL_TEMPS scope=global mesure=ecoule_inclut_blocage t={} \
ecoule_ms={} appels={} hors_table={}",
        crate::kernel::timer::monotonic_ms(),
        total_ns / 1_000_000,
        total_n,
        SYSCALL_HORS_TABLE.load(Ordering::Relaxed),
    ));
    // Selection par passes successives : pas d'allocation dans un chemin de
    // sortie de processus, et `combien` est petit.
    let mut plafond = u64::MAX;
    for _ in 0..combien {
        let mut meilleur = None;
        for i in 0..SYSCALL_MAX {
            let ns = SYSCALL_NS[i].load(Ordering::Relaxed);
            if ns == 0 || ns > plafond {
                continue;
            }
            match meilleur {
                Some((_, m)) if ns <= m => {}
                _ => meilleur = Some((i, ns)),
            }
        }
        let Some((i, ns)) = meilleur else { break };
        let n = SYSCALL_N[i].load(Ordering::Relaxed);
        crate::kernel::dmesg::log_fmt(format_args!(
            "SYSCALL_TEMPS scope=global nr={} nom={} ecoule_ms={} appels={} moyen_us={}",
            i,
            crate::kernel::abi::nr::name(i as u64),
            ns / 1_000_000,
            n,
            if n != 0 { ns / n / 1_000 } else { 0 },
        ));
        plafond = ns.saturating_sub(1);
    }
}
