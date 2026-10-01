/// Marque la tache courante terminee et rend la main.
///
/// Si d'autres threads du programme tournent encore, on bascule sur eux ;
/// sinon, retour au fil noyau qui a lance le programme.
pub fn exit_current(code: i32) -> ! {
    // BOUCHAUD_C70_UNE_SONDE_NE_RALENTIT_PAS_CE_QU_ELLE_MESURE
    //
    // Les sondes globales de mort de processus sont publiees PLUS BAS, hors
    // du verrou `process.lifecycle`. Elles y etaient d'abord, et trois
    // ecritures serie tenues sous un verrou de processus ont suffi a faire
    // sortir une course de reveil anterieure : `qemu / os primitives` s'est
    // fige, machine vivante, shell jamais repris.
    //
    // Une sonde ne prend pas de verrou (cf. `balayage_temoins`), et elle ne
    // rallonge pas davantage une section critique qu'elle observe.
    let mut dernier_thread = false;
    let mut pid_sortant = 0u32;
    {
        let task = current();
        marque_zombie(task);
        // pthread_join s'appuie sur cette ecriture suivie d'un futex_wake.
        let clear = task.clear_child_tid;
        if clear != 0 {
            let process = task.process.clone();
            process.mm.lock().space.write(clear, &0u32.to_le_bytes());
            futex_wake(clear, 1);
        }
        let process = task.process.clone();
        pid_sortant = process.pid;
        // BOUCHAUD_C61_UNE_MORT_DE_PROCESSUS_SE_NOMME_UNE_FOIS
        //
        // Le nom est lu AVANT `lifecycle.lock()` : le prendre a l'interieur
        // imbriquerait deux verrous du meme processus sur un chemin de sortie,
        // et l'ordre inverse existe ailleurs.
        let nom_sortie = process.metadata.lock().name.clone();
        let ppid_sortie = process.parent;
        let mut lifecycle = process.lifecycle.lock();
        let threads_avant = lifecycle.threads;
        if lifecycle.threads > 0 {
            lifecycle.threads -= 1;
        }
        lifecycle.exit_code = code;
        // BOUCHAUD_GROUPE_RECLAME_V1 : UNE seule transition vers le dernier fil.
        // `exit_group` remet `threads` a 1 ; un fil deja compte sorti pouvait
        // alors retrouver `threads == 0` et demonter le processus une seconde
        // fois. `zombie`, pose sous ce meme verrou par le premier, tranche.
        if lifecycle.threads == 0 && !lifecycle.zombie {
            // LA frontiere processus, et elle ne passe qu'une fois.
            //
            // `FAULT_FILE_BREAKDOWN` sortait a chaque fin de THREAD : au run
            // 35907201865, WebContent en a emis trois, ce qui se lisait comme
            // trois morts. Cette ligne-ci est posee la ou le dernier thread
            // tombe, donc exactement une fois par processus.
            crate::kernel::dmesg::log_fmt(format_args!(
                "PROCESS_EXIT t={} pid={} ppid={} image={} code={} \
threads_before={} threads_after=0 reason=dernier_thread",
                crate::kernel::timer::monotonic_ms(),
                process.pid,
                ppid_sortie,
                nom_sortie,
                code,
                threads_avant,
            ));
            // BOUCHAUD_C66_LE_PARTAGE_UTILISATEUR_NOYAU_A_LA_MORT
            //
            // GLOBAL, ET DIT COMME TEL. Ce ne sont PAS les compteurs de ce
            // processus : ce sont les cumuls de tous les CPU depuis le
            // demarrage. Leur interet est la DIFFERENCE entre deux sorties
            // consecutives, qui donne le partage utilisateur/noyau du travail
            // fait entre les deux -- la meme methode que
            // `analyse_faute_fichier.py` emploie deja pour les fautes.
            //
            // Cette ligne existe parce que `[PROC-STAT]` n'est emis que par
            // l'echantillonneur du bureau : un scenario `autorun` sans bureau
            // n'en produisait aucune, et le partage utilisateur/noyau y etait
            // tout simplement invisible.
            // DEUX LECTURES, ET ELLES NE MESURENT PAS LA MEME CHOSE.
            //
            // `replie_*` ne lit que `CUMUL_USER_NS`/`CUMUL_NOYAU_NS` : du
            // temps deja impute, jamais revise. `vue_*` y ajoute la TRANCHE EN
            // COURS de chaque tache vivante, attribuee selon son `in_kernel`.
            //
            // La difference n'est pas cosmetique. Une tache qui dort depuis
            // longtemps sans avoir replie sa tranche porte un `live` enorme,
            // et `vue_*` le verse d'un coup du cote ou son `in_kernel` pointe.
            // Un releve de `vue_noyau_ms` peut donc bondir de plus d'une
            // seconde en quarante millisecondes de temps reel -- ce qui est
            // physiquement impossible et signale une attribution en vol, pas
            // du travail.
            //
            // Publier les deux, c'est pouvoir dire laquelle des deux on lit.
            let vue = crate::kernel::task::proc_cpu_cumul();
            let (replie_user, replie_noyau) = crate::kernel::task::proc_cpu_compteurs();
            crate::kernel::dmesg::log_fmt(format_args!(
                "CPU_CUMUL scope=global t={} apres_pid={} \
replie_user_ms={} replie_noyau_ms={} vue_user_ms={} vue_noyau_ms={}",
                crate::kernel::timer::monotonic_ms(),
                process.pid,
                replie_user / 1_000_000,
                replie_noyau / 1_000_000,
                vue.user_ns / 1_000_000,
                vue.system_ns / 1_000_000,
            ));
            // Dernier thread : le processus devient zombie jusqu'a ce que son
            // parent le recolte par `wait4`. C'est ce qui permet au parent de
            // recuperer le code de sortie apres coup.
            lifecycle.zombie = true;
            // Les verrous d'enregistrement POSIX meurent avec leur detenteur.
            // Un WebContent qui plante ne doit pas laisser la base SQL du
            // navigateur verrouillee pour le reste de la session.
            crate::kernel::abi::verrous::libere_processus(process.pid);
            // Et la supervision apprend la mort du processus. C'est ici, et
            // seulement ici, que la difference entre « un rendu est mort » et
            // « le navigateur est mort » se decide : le courtier emporte ses
            // enfants, un rendu n'emporte que lui-meme.
            crate::kernel::navigateur::supervision::note_sortie(
                process.pid, code, crate::kernel::timer::monotonic_ns());
            dernier_thread = true;
        }
    }

    // Les compteurs globaux, une fois le verrou du processus RELACHE.
    //
    // Ils sont AUSSI publies par l'echantillonneur, et ce n'est pas un
    // doublon : sous Ladybird aucun service ne meurt pendant la fenetre
    // mesuree, et sur le banc l'echantillonneur ne tourne qu'une fois, avant
    // les lancements. Les deux cas ne se recouvrent pas.
    if dernier_thread {
        // P18_SERVICE_GUARDIAN_V1. Hors du verrou lifecycle: aucune
        // serialisation, allocation ou relance sous le verrou de sortie.
        let process = &current().process;
        let nom = process.metadata.lock().name.clone();
        crate::kernel::services::gardien::mort(
            process.pid, process.parent, &nom, code, false);
        let (bal_appels, bal_entrees, bal_ns, bal_pire, bal_candidats) =
            crate::kernel::clean_page_cache::balayage_stats();
        crate::kernel::dmesg::log_fmt(format_args!(
            "CACHE_BALAYAGE scope=global t={} appels={} entrees_parcourues={} \
total_us={} pire_us={} candidats_suffisants={}",
            crate::kernel::timer::monotonic_ms(),
            bal_appels, bal_entrees, bal_ns / 1_000, bal_pire / 1_000, bal_candidats,
        ));
        let (evites, en_table, recuperees) =
            crate::kernel::clean_page_cache::balayage_temoins();
        crate::kernel::dmesg::log_fmt(format_args!(
            "CACHE_BALAYAGE_TEMOINS scope=global evites={} entrees={} recuperees={}",
            evites, en_table, recuperees,
        ));
        let (yields, pire_chaine, reprises, chaines) =
            crate::kernel::task::fault_retry_cumul();
        crate::kernel::dmesg::log_fmt(format_args!(
            "FAULT_REPRISE scope=global t={} yields={} pire_chaine={} reprises={} chaines={}",
            crate::kernel::timer::monotonic_ms(),
            yields, pire_chaine, reprises, chaines,
        ));
    }

    // Previent le parent : SIGCHLD, et reveil s'il attendait dans `wait4`.
    notify_parent_of_exit();

    // Le programme de premier plan vient-il de se terminer ? Alors la session
    // est finie, et ce qu'il a laisse derriere lui n'a plus personne pour
    // l'attendre. C'est la semantique POSIX d'un shell : le meneur de session
    // part, le groupe de premier plan recoit SIGHUP.
    //
    // `run` faisait deja ce menage -- mais APRES son retour, c'est-a-dire
    // jamais, puisque c'est precisement ce qui l'empechait de revenir.
    let racine = RACINE_PREMIER_PLAN.load(Ordering::Acquire);
    if racine != 0 {
        let fini = {
            let process = &current().process;
            process.lifecycle.lock().zombie && process.pid == racine
        };
        if fini {
            let mut emportes = 0usize;
            for index in 0..tasks().len() {
                if tasks()[index].state == TaskState::Zombie {
                    continue;
                }
                let pid = tasks()[index].process.pid;
                if descend_de(pid, racine) {
                    marque_zombie(&tasks()[index]);
                    emportes += 1;
                }
            }
            if emportes > 0 {
                crate::kernel::dmesg::log_fmt(format_args!(
                    "task: pid {} termine, {} tache(s) de sa session arretees avec lui",
                    racine, emportes
                ));
            }
        }
    }

    // Cette continuation est terminee : le garde de l'appel systeme restera
    // sur sa pile condamnee et ne pourra jamais executer son Drop. Rendre sa
    // profondeur maintenant garantit que le choix final et `switch_to`
    // entrent bien dans le coeur scheduler sans BKL.
    // BOUCHAUD_C54_PAR_PID_OU_RIEN
    //
    // La version precedente imprimait `FAULT_FILE_BREAKDOWN pid=18` a partir
    // d'atomiques GLOBAUX. L'etiquette promettait une attribution que les
    // chiffres n'avaient pas : quand WebContent, le Compositor et le worker
    // faultent en meme temps, le « cout du worker » contenait celui des deux
    // autres. La difference entre deux sorties ne reparait rien -- elle
    // supposait qu'ils ne se chevauchent pas, ce qui est faux par definition
    // dans le cas qu'on veut mesurer.
    //
    // Les chiffres viennent maintenant du livre par PID, alimente faute par
    // faute depuis la `Note` qui connait son processus.
    if let Some(pid) = current_process_local().map(|p| p.pid) {
        if let Some((ph, perdues)) = crate::kernel::task::phases_fichier_du_processus(pid) {
            if ph.nombre != 0 {
                // La source du binaire : une lecture ATA et un memcpy depuis le
                // ramdisk UEFI ne se soignent pas pareil, et le meme service
                // n'a pas le meme cout sous QEMU et sur la Trigkey.
                let source = current_process_local()
                    .and_then(|p| {
                        let nom = p.metadata.lock().name.clone();
                        let fs = crate::fs::ramfs::fs();
                        fs.resolve(&nom, 0)
                    })
                    .map(|n| crate::fs::backing::kind(n).etiquette())
                    .unwrap_or("inconnu");
                crate::kernel::dmesg::log_fmt(format_args!(
                    "FAULT_FILE_BREAKDOWN t={} pid={} source={} faults={} total_us={} \
wait_us={} acquire_us={} acquire_hit_n={} acquire_hit_us={} acquire_miss_n={} \
acquire_miss_us={} acquire_miss_read_us={} acquire_wait_n={} acquire_wait_us={} \
backing_direct_us={} mm_lock_us={} map_us={} explained_us={} residual_us={} \
residual_pct={} worst_us={} lost_samples={}",
                    crate::kernel::timer::monotonic_ms(),
                    pid,
                    source,
                    ph.nombre,
                    ph.total_ns / 1_000,
                    ph.attente_ns / 1_000,
                    ph.acquire_ns / 1_000,
                    ph.hit_n, ph.hit_ns / 1_000,
                    ph.miss_n, ph.miss_ns / 1_000,
                    // « dont » : deja compris dans acquire_us, jamais additif.
                    ph.acquire_backing_ns / 1_000,
                    ph.wait_n, ph.wait_ns / 1_000,
                    ph.backing_direct_ns / 1_000,
                    ph.mm_ns / 1_000,
                    ph.map_ns / 1_000,
                    ph.explique_ns() / 1_000,
                    ph.residu_ns() / 1_000,
                    ph.residu_pct(),
                    ph.pire_ns / 1_000,
                    perdues,
                ));
            }
        }
    }

    // La vue SYSTEME, explicitement etiquetee comme telle. Elle repond a
    // « combien la machine entiere a lu », jamais a « combien ce service a
    // lu » -- et le suffixe l'empeche d'etre confondue avec la ligne au-dessus.
    {
        let (dr, db, dn, dw) = crate::fs::backing::disk_read_timing();
        let (mr, mb, mn, mw) = crate::fs::backing::memory_read_timing();
        let t = crate::kernel::clean_page_cache::acquire_timing();
        crate::kernel::dmesg::log_fmt(format_args!(
            "BACKING_DISK_GLOBAL scope=global reads={} bytes={} total_us={} worst_us={}",
            dr, db, dn / 1_000, dw / 1_000,
        ));
        crate::kernel::dmesg::log_fmt(format_args!(
            "BACKING_MEMORY_GLOBAL scope=global reads={} bytes={} total_us={} worst_us={}",
            mr, mb, mn / 1_000, mw / 1_000,
        ));
        crate::kernel::dmesg::log_fmt(format_args!(
            "CLEAN_PAGE_CACHE_GLOBAL scope=global hits={} miss={} waits={} \
hit_us={} miss_us={} miss_read_us={} wait_us={} worst_us={}",
            t.0, t.1, t.2, t.3 / 1_000, t.4 / 1_000, t.5 / 1_000, t.6 / 1_000,
            t.7 / 1_000,
        ));
        // Attribution des memes lectures. Tout atomique : aucun verrou pris
        // ici, conformement a `verifie-sondes-sans-verrou.py`.
        let maintenant = crate::kernel::timer::monotonic_ms();
        crate::fs::backing_attrib::publie(maintenant);
        crate::drivers::ata::publie_controleur(maintenant);
    }

    // BOUCHAUD_HOTFIX13_ASYNC_KERNEL_REAP_V2
    //
    // `spawn_noyau_*` cree un vrai Process + une ligne dans le registre
    // utilisateur, mais rend seulement un bool : personne ne recoit de PID,
    // personne ne peut faire wait4(), et un travailleur fini n'a donc AUCUN
    // recolteur. La campagne physique Internet l'a rendu mesurable : une
    // preuve finie ajoutait exactement une ligne `taches` au releve distant.
    //
    // Le lancement synchrone `run()` est different : sa racine est publiee
    // dans RACINE_PREMIER_PLAN et le fil noyau appelant attend son code de
    // sortie. On ne la recolte jamais ici.
    //
    // Un travailleur NOYAU, parent=0, dernier thread, qui n'est pas la racine
    // synchrone courante est donc un travailleur detache sans wait4 possible.
    // Toutes les sondes de sortie et la supervision ont deja ete publiees
    // au-dessus; c'est le dernier endroit sur sa pile ou ses deux registres
    // peuvent etre nettoyes avant la commutation definitive.
    if dernier_thread {
        let (detache, pid, nom) = {
            let task = current();
            let process = &task.process;
            let racine_synchrone = RACINE_PREMIER_PLAN.load(Ordering::Acquire);
            (
                task.noyau
                    && process.parent == 0
                    && process.pid != racine_synchrone,
                process.pid,
                process.metadata.lock().name.clone(),
            )
        };
        if detache {
            // `collect_child` retire l'Arc de PROCESSES et la ligne du
            // registre simple kernel::process. La tache courante conserve
            // encore son propre Arc jusqu'a sa commutation definitive.
            collect_child(pid);
            crate::kernel::dmesg::log_fmt(format_args!(
                "ASYNC_KERNEL_REAP t={} pid={} nom={}",
                crate::kernel::timer::monotonic_ms(),
                pid,
                nom,
            ));
        }
    }

    // BOUCHAUD_CONTINUATION_SYNCHRONE_V1 : OU VA CETTE PILE CONDAMNEE.
    //
    // Trois destinations, et une regle stricte pour la premiere :
    //
    //   1. la continuation du lancement synchrone (`run` / `run_noyau`), si
    //      CETTE mort termine sa racine et qu'elle est garee sur CE coeur ;
    //   2. une autre tache prete de ce coeur ;
    //   3. la boucle idle de ce coeur -- AP comme BSP.
    //
    // Avant ce lot, sur le BSP, « idle » ETAIT la continuation. Deux pannes en
    // sortaient, prouvees par `continuation-banc` sur le protocole d'origine :
    //
    //   * la mort de N'IMPORTE QUELLE tache du coeur zero, faute d'autre tache
    //     prete, reprenait le lancement d'un autre -- incident TRIGKEY,
    //     `RUN_NOYAU_RETOUR nom=desktop fil_mort=0`, puis la cascade
    //     `PROCESS_KILL raison=run_noyau_retour` (SMP4 et SMP8 : 1er fil court) ;
    //   * la mort de la racine, si une autre tache etait prete, partait vers
    //     elle et laissait la continuation garee pour toujours (SMP1, SMP2).
    //
    // La boucle d'attente sur la pile morte, son garde-fou de trente secondes
    // et sa commutation sans retour disparaissent avec cette confusion : une
    // racine pas encore terminee (fils, descendance de `run`) laisse sa
    // continuation garee, et c'est la derniere mort -- ou la boucle idle du
    // coeur proprietaire -- qui la reprend.
    let cpu_id = local_cpu();
    let cur = current_index_raw();
    // BOUCHAUD_C71_QUELLE_BRANCHE_POUR_LA_RACINE
    //
    // UNE ligne, et seulement quand la racine de premier plan meurt -- donc
    // une fois par commande de l'autorun, pas une forêt.
    let coeur_proprietaire = CONTINUATION_CPU.load(Ordering::Relaxed);
    if racine != 0 && pid_sortant == racine {
        crate::kernel::dmesg::log_fmt(format_args!(
            "RETOUR_SHELL t={} pid={} racine={} cpu={} branche={}",
            crate::kernel::timer::monotonic_ms(),
            pid_sortant,
            racine,
            cpu_id,
            if cpu_id == coeur_proprietaire { "coeur_proprietaire" } else { "autre_coeur" },
        ));
    }
    let garee = CONTINUATION_ETAT.load(Ordering::Acquire) == CONTINUATION_GAREE;
    let sortie = crate::kernel::continuation::Sortie {
        cpu: cpu_id,
        garee,
        cpu_continuation: coeur_proprietaire,
        racine_terminee: garee
            && racine_terminee(
                CONTINUATION_RACINE.load(Ordering::Relaxed),
                CONTINUATION_DESCENDANCE.load(Ordering::Relaxed),
            ),
        // Decide plus bas par `commute_sortie_definitive_si_possible` : la
        // continuation passe avant, quelle que soit la reponse.
        autre_prete: false,
    };
    if crate::kernel::continuation::destination(sortie)
        == crate::kernel::continuation::Destination::Continuation
    {
        if racine != 0 && pid_sortant == racine {
            crate::kernel::dmesg::log_fmt(format_args!(
                "RETOUR_SHELL_SAUT t={} pid={} cpu={} voie=racine_directe",
                crate::kernel::timer::monotonic_ms(),
                pid_sortant,
                cpu_id,
            ));
        }
        reprend_continuation("racine_terminee");
    }
    // Due, mais garee sur un autre coeur : sa boucle idle la reprendra.
    // On la reveille plutot que d'attendre son prochain tick.
    if crate::kernel::continuation::reveiller_proprietaire(sortie) {
        smp::reschedule_cpu(coeur_proprietaire);
    }
    commute_sortie_definitive_si_possible(cur, cpu_id);
    switch_to_kernel(if cpu_id == 0 { "bsp_idle" } else { "ap_idle" })
}

/// Signale au parent qu'un de ses fils vient de se terminer.
///
/// Deux effets distincts, tous deux necessaires : `SIGCHLD` (que le parent
/// peut avoir choisi d'intercepter) et le reveil d'un `wait4` bloquant.
fn notify_parent_of_exit() {
    let (parent_pid, is_zombie) = {
        let process = &current().process;
        (process.parent, process.lifecycle.lock().zombie)
    };
    if !is_zombie { return; }
    notify_parent_of_exit_for(parent_pid);
}

// P18_CHILD_EXIT_NOTIFY_V1: le kill distant ne sort pas depuis le thread vise.
// Le meme chemin SIGCHLD/wait4 doit fonctionner pour une sortie forcee.
fn notify_parent_of_exit_for(parent_pid: u32) {
    if parent_pid == 0 { return; }
    for index in 0..tasks().len() {
        if tasks()[index].state == TaskState::Zombie {
            continue;
        }
        let matches = {
            let process = &tasks()[index].process;
            if process.pid == parent_pid {
                process.signals.lock().raise(crate::kernel::signal::SIGCHLD);
                true
            } else {
                false
            }
        };
        if matches
            && tasks()[index]
                .waiting_for_child
                .compare_exchange(true, false)
                .is_ok()
            && tasks()[index]
                .state
                .echange(TaskState::Blocked, TaskState::Ready)
        {
            publish_ready(index);
        }
    }
}

/// Recense les processus fils zombies d'un pid donne.
pub fn zombie_children(parent_pid: u32) -> Vec<(u32, i32)> {
    let mut out = Vec::new();
    for process in processes().iter() {
        let lifecycle = process.lifecycle.lock();
        if process.parent == parent_pid && lifecycle.zombie {
            out.push((process.pid, lifecycle.exit_code));
        }
    }
    out
}

/// Ce pid a-t-il encore des fils (zombies ou vivants) ?
pub fn has_children(parent_pid: u32) -> bool {
    processes().iter().any(|p| p.parent == parent_pid)
}

/// Retire un fils zombie (il a ete recolte). Rend `true` si CET appel l'a retire.
///
/// BOUCHAUD_WAIT4_RECOLTE_UNIQUE_V1 : deux fils du meme parent dans `wait4`
/// pouvaient voir le meme zombie et le recolter tous les deux (meme pid rendu
/// deux fois) ; le gros verrou serialisait `wait4` entier. Le retrait sous le
/// verrou de PROCESSES decide desormais d'un seul gagnant.
pub fn collect_child(pid: u32) -> bool {
    let retire = {
        let mut table = PROCESSES.lock();
        let avant = table.len();
        table.retain(|p| p.pid != pid);
        table.len() != avant
    };
    if retire {
        crate::kernel::process::kill(pid);
    }
    retire
}

// BOUCHAUD_GROUPE_RECLAME_V1
//
// `exit_group` et `execve` tuent les autres fils du processus. Deux d'entre eux
// en meme temps -- deux `exit_group`, deux `execve`, ou l'un contre l'autre --
// se tuaient mutuellement, puis demontaient ou remplacaient le processus deux
// fois ; un `clone` concurrent pouvait enregistrer un fil APRES le balayage et
// survivre a la sortie du groupe. Le gros verrou, pris par l'aiguilleur pour
// ces appels, les serialisait. Le verrou `lifecycle` le fait desormais :
//
//   * le premier fil qui RECLAME le groupe gagne ; les autres, qui allaient de
//     toute facon etre tues, se retirent comme les fils tues par `execve`
//     (sans toucher `threads`, sans notifier le parent) ;
//   * `clone` relit la reclamation APRES avoir enregistre son fils : ou bien
//     le balayage du gagnant voit le fils, ou bien `clone` voit la
//     reclamation et marque lui-meme son fils zombie.

/// `true` si ce fil vient de reclamer le groupe ; `false` si un autre le tient.
pub fn reclame_groupe() -> bool {
    let process = current().process.clone();
    let mut lifecycle = process.lifecycle.lock();
    if lifecycle.groupe_en_sortie {
        return false;
    }
    lifecycle.groupe_en_sortie = true;
    true
}

/// Le fil a perdu la reclamation : il se retire comme un fil tue par `execve`.
pub fn retire_perdant_du_groupe() -> ! {
    {
        let task = current();
        marque_zombie(task);
    }
    retire_exec_zombie_current()
}

/// Apres l'enregistrement d'un fil par `clone` : si le groupe a ete reclame
/// entre-temps, le fil ne doit pas survivre au balayage qu'il a manque.
pub fn tue_si_groupe_reclame(process: &Arc<Process>, index: usize, tid: u32) {
    if !process.lifecycle.lock().groupe_en_sortie {
        return;
    }
    let table = tasks();
    if let Some(task) = table.get(index) {
        if task.tid == tid {
            marque_zombie(task);
        }
    }
}

/// Termine tous les threads du processus courant (`exit_group`).
pub fn exit_group(code: i32) -> ! {
    // BOUCHAUD_GROUPE_RECLAME_V1 : un seul fil demonte le groupe.
    if !reclame_groupe() {
        retire_perdant_du_groupe();
    }
    let (pid, tid, process) = {
        let task = current();
        (task.process.pid, task.tid, task.process.clone())
    };
    for task in tasks().iter() {
        if task.tid != tid && task.process.pid == pid {
            marque_zombie(task);
        }
    }

    // `exit_group` termine tous les autres threads du processus. Le thread
    // courant est donc le seul encore vivant; `exit_current` le decrementera
    // de 1 a 0 et rendra le processus zombie/recoltable par `wait4`.
    process.lifecycle.lock().threads = 1;

    exit_current(code)
}

/// Lance une tache depuis le fil noyau et attend la fin du programme.
///
/// Renvoie le code de sortie du processus.
///
/// # Securite
/// A n'appeler que depuis le fil noyau appelant, `CURRENT` valant `usize::MAX`.
/// `KERNEL_CTX` est unique : un appel imbrique depuis une tache y ecraserait le
/// contexte du fil qui attend deja, et `set_current_index(usize::MAX` a la sortie
/// effacerait l'identite de la tache appelante. Les appelants verifient
/// [`in_user_task`] avant d'arriver ici — voir `exec::exec_image`.
pub fn run(mut first: Box<Task>) -> i32 {
    // BOUCHAUD_RUN_PORTE_TRANSITION_V1
    //
    // `run` et `run_noyau` prenaient le gros verrou. Ce qu'il protegeait
    // vraiment, c'est une fenetre : entre `set_current_index` et
    // `switch_context`, la tache courante de ce coeur est deja la nouvelle
    // alors qu'on execute encore sur la pile d'amorcage. Une preemption sur
    // IRQ a cet instant sauverait la pile d'amorcage dans le contexte de la
    // tache. Le BKL l'empechait (`preemption_noyau_sure` le refuse)... mais il
    // etait SUSPENDU juste avant `switch_context` : la fin de la fenetre
    // n'etait pas couverte.
    //
    // La primitive qui existe pour cela est la porte de transition de
    // l'ordonnanceur, celle que `switch_to` ouvre : `preempt_from_irq` renonce
    // quand elle est ouverte sans passation (C26), et la continuation
    // entrante la rend (`complete_switch_handoff`, cas « transition noyau ->
    // tache », qui l'attendait deja). Elle couvre la fenetre ENTIERE.
    // Le thread racine d'un lancement synchrone doit revenir sur la pile
    // noyau de son CPU appelant. Lui seul est pince; les pthreads qu'il cree
    // naissent avec une affinite machine complete et peuvent etre balances.
    let caller_cpu = local_cpu();
    if caller_cpu == 0 {
        assure_idle_coeur_zero();
    }
    first.affinity_mask = 1u64 << caller_cpu;
    first.runq_cpu.range(caller_cpu as u8);
    first.last_cpu.range(caller_cpu as u8);
    let process = first.process.clone();
    let racine = process.pid;
    let index = register(first);
    let cpu_id = local_cpu();
    assert!(commence_transition_ordonnanceur(), "run: transition scheduler deja ouverte");
    let to_ptr = unsafe {
        RACINE_PREMIER_PLAN.store(racine, Ordering::Release);
        let list = tasks();
        let ptr = unsafe { registre_pointeur_ordonnanceur(index) }.expect("registre: tache absente");
        mark_task_running(&mut *ptr, cpu_id);
        ptr
    };
    set_current_index(index);
    unsafe { install(&mut *to_ptr); }
    crate::platform::pc::ecran_faute::point_silencieux("run-noyau-installe");
    // La pile d'amorcage est garee dans LA continuation, a son proprietaire
    // -- plus dans `KERNEL_CTX`, qui est l'idle du coeur. Seule la fin de la
    // racine ET de sa descendance la rendra (BOUCHAUD_CONTINUATION_SYNCHRONE_V1).
    gare_continuation(cpu_id, racine, unsafe { (*to_ptr).tid }, true);
    let continuation = unsafe { core::ptr::addr_of_mut!(CONTINUATION.rsp) };
    crate::platform::pc::ecran_faute::point_silencieux("run-noyau-switch");
    unsafe { switch_context(continuation, (*to_ptr).ctx.rsp); }
    complete_switch_handoff();
    crate::kernel::dmesg::log_fmt(format_args!(
        "RETOUR_SHELL_REPRIS t={} racine={} cpu={}",
        crate::kernel::timer::monotonic_ms(),
        racine,
        local_cpu(),
    ));

    crate::kernel::vmm::activate_kernel();
    set_current_index(NO_TASK);
    clear_current_process_local();
    RACINE_PREMIER_PLAN.store(0, Ordering::Release);

    let (code, pid) = {
        (process.lifecycle.lock().exit_code, process.pid)
    };
    reap();
    // LE MENAGE EMPORTE LA SESSION, PAS LE SYSTEME.
    //
    // Ces trois lignes tuaient TOUS les processus et vidaient la table --
    // travailleurs noyau compris. Tant que le seul lancement synchrone etait
    // suivi d'un retour au shell, la difference ne se voyait pas ; elle se
    // voit des qu'un service doit survivre a l'execution d'un programme.
    //
    // Un `exec` depuis le shell ne doit pas arreter le fil d'entree, ni le fil
    // de montage : ils n'appartiennent pas a la session qu'on ferme.
    let condamnes: alloc::vec::Vec<u32> = processes()
        .iter()
        .filter(|p| p.pid == racine || descend_de(p.pid, racine))
        .map(|p| p.pid)
        .collect();
    for condamne in condamnes {
        crate::kernel::process::kill(condamne);
    }
    crate::kernel::process::kill(pid);
    code
}

/// Lance un fil noyau et rend la main AUSSITOT.
///
/// # Pourquoi `run_noyau` ne suffisait pas
///
/// `run_noyau` commute VERS la tache et ne revient qu'a sa mort : c'est ce
/// qu'il faut pour le bureau, qui est le fil principal, et c'est inutilisable
/// pour tout le reste. Le montage du disque interne etait donc appele EN LIGNE
/// depuis la boucle de trames du compositeur -- sur SA pile, dans SON quantum.
///
/// Les consequences etaient toutes du meme genre : ce que fait le disque, le
/// bureau le subit. Une commande lente est une trame perdue ; une faute dans le
/// chemin de stockage est un bureau mort ; et la profondeur de pile du montage
/// s'ajoute a celle du compositeur, qui est deja la plus longue chaine du
/// systeme.
///
/// Une tache lancee ici a SA pile, SON quantum et SA priorite. Le bureau ne la
/// voit plus.
///
/// Rend `false` si le processus ou la tache n'ont pas pu etre crees. L'appelant
/// decide alors -- faire le travail sur place vaut souvent mieux que ne pas le
/// faire du tout, et c'est a lui de le savoir.
pub fn spawn_noyau(entree: fn() -> !, nom: &str) -> bool {
    spawn_noyau_priorite(entree, nom, Priorite::Normale)
}

/// Lance un travailleur noyau avec une priorite CHOISIE.
///
/// `spawn_noyau` fixe `Normale`, et c'est le bon defaut pour un travail de
/// fond. Mais une priorite qui n'est jamais demandee autrement que par defaut
/// ne peut pas etre mise a l'epreuve : sans deux classes reellement en
/// concurrence, rien ne dit si `Interactive` change une decision ou n'est
/// qu'une etiquette.
/// Lance un travailleur noyau SENSIBLE A LA LATENCE.
///
/// # Ce que cette propriete donne, et ce qu'elle ne donne pas
///
/// Elle NE DONNE PAS une priorite permanente. `Priorite::Interactive` dit
/// « sers-moi avant le fond de file quand nous sommes tous deux prets » ;
/// `latency_sensitive` dit autre chose : « le delai entre mon reveil et mon
/// election doit rester court ». Les deux sont orthogonaux, et c'est le second
/// qui manquait -- une tache interactive publiee sur un coeur occupe par un
/// fil noyau n'avait, avant ce lot, aucun chemin vers le processeur.
///
/// Concretement, la tache obtient :
///
///   * un PLACEMENT au reveil : le coeur qui la servira le plus tot, un coeur
///     au repos de preference ;
///   * le droit de COUPER l'occupant du coeur choisi, fil noyau compris.
///
/// Les deux sont bornes par `scheduler::reveil::BUDGET_ACTIVATION_NS` : une
/// activation qui depasse la borne desarme le privilege jusqu'a ce que la
/// suivante repasse dessous. FAIBLE BUDGET, FORTE EXIGENCE DE REVEIL.
///
/// Le scheduler ne connait AUCUN nom de tache. L'entree USB la demande
/// aujourd'hui ; l'audio, le compositeur et tout service temps reel souple la
/// demanderont par le meme appel.
pub fn spawn_noyau_sensible(entree: fn() -> !, nom: &str, priorite: Priorite) -> bool {
    spawn_noyau_interne(entree, nom, priorite, true)
}

pub fn spawn_noyau_priorite(entree: fn() -> !, nom: &str, priorite: Priorite) -> bool {
    spawn_noyau_interne(entree, nom, priorite, false)
}

fn spawn_noyau_interne(
    entree: fn() -> !,
    nom: &str,
    priorite: Priorite,
    latency_sensitive: bool,
) -> bool {
    // AUCUN GROS VERROU ICI, ET C'EST DELIBERE.
    //
    // `run_noyau` ouvre la porte de transition parce qu'il COMMUTE : il touche
    // l'etat du coeur courant, la tache courante et la pile noyau du moment
    // (BOUCHAUD_RUN_PORTE_TRANSITION_V1). Lancer une tache ne touche rien de
    // tout cela.
    //
    // `new_process` se protege par ses propres verrous -- l'espace
    // d'adressage, la table des processus, les champs du descripteur -- et
    // `register` prend ce qu'il lui faut. Une tache non enregistree n'est
    // visible de personne : il n'y a rien a serialiser entre les deux.
    //
    // Le garde-fou des budgets compte les sites du gros verrou par
    // sous-systeme, et il a refuse ce lot tant que celui-ci en ajoutait un
    // quatrieme au domaine Processus. Il avait raison : un site rajoute dans
    // un sous-systeme qu'on allege annule le travail sans echouer a aucun test.
    let Some(process) = new_process(nom, 0) else {
        return false;
    };
    let mut task = Task::new_kernel(process, entree);
    // NORMALE, ET C'EST LE POINT.
    //
    // Interactive la mettrait a egalite avec le bureau, qu'elle est justement
    // censee cesser de deranger. Un travail de fond est un travail de fond.
    task.priorite.range(priorite);
    // MIGRABLE, PARCE QU'UN TRAVAILLEUR DE FOND N'A PAS DE COEUR A LUI.
    //
    // `register` epingle par defaut toute tache noyau au coeur zero. Pour un
    // travail de fond, cette regle produit exactement l'inverse de ce qu'on
    // veut : la tache est creee, enregistree, visible -- et jamais elue, des
    // que le coeur zero n'est pas lui-meme dans l'ordonnanceur. Le fil de
    // montage l'a montre sur un demarrage sans bureau : `on=-1`, trois coeurs
    // au repos a cote, et rien.
    //
    // Rien dans un travailleur lance par cette fonction ne depend du coeur
    // zero : il n'a ni pile heritee, ni etat local de coeur, ni contexte
    // d'appelant. Il se declare donc migrable.
    task.migrable = true;
    task.latency_sensitive.range(latency_sensitive);
    register(task);
    true
}

pub fn run_noyau(entree: fn() -> !, nom: &str) -> i32 {
    // Porte de transition et non gros verrou : BOUCHAUD_RUN_PORTE_TRANSITION_V1
    // (voir `run`).
    if in_user_task() {
        crate::kernel::dmesg::log("task: run_noyau imbrique refuse");
        return -1;
    }
    let process = match new_process(nom, 0) {
        Some(process) => process,
        None => return -1,
    };
    if local_cpu() == 0 {
        assure_idle_coeur_zero();
    }
    let mut task = Task::new_kernel(process.clone(), entree);
    crate::platform::pc::ecran_faute::point_silencieux("run-noyau-pile-creee");
    task.priorite.range(Priorite::Interactive);
    task.affinity_mask = 1;
    task.runq_cpu.range(0);
    task.last_cpu.range(0);
    let index = register(task);
    assert!(commence_transition_ordonnanceur(), "run_noyau: transition scheduler deja ouverte");
    let to_ptr = unsafe {
        let list = tasks();
        let ptr = unsafe { registre_pointeur_ordonnanceur(index) }.expect("registre: tache absente");
        mark_task_running(&mut *ptr, 0);
        ptr
    };
    set_current_index(index);
    unsafe { install(&mut *to_ptr); }
    gare_continuation(0, process.pid, unsafe { (*to_ptr).tid }, false);
    let continuation = unsafe { core::ptr::addr_of_mut!(CONTINUATION.rsp) };
    unsafe { switch_context(continuation, (*to_ptr).ctx.rsp); }
    complete_switch_handoff();

    crate::kernel::vmm::activate_kernel();
    set_current_index(NO_TASK);
    clear_current_process_local();
    let (code, pid) = {
        (process.lifecycle.lock().exit_code, process.pid)
    };

    // BOUCHAUD_C60_QUI_A_FAIT_REVENIR_LE_BUREAU
    //
    // `switch_context` vient de rendre la main au contexte d'amorcage. Deux
    // choses tres differentes peuvent l'expliquer :
    //
    //   le fil est MORT        -- `exit_current`, donc une sortie voulue
    //   le fil est VIVANT      -- l'ordonnanceur est simplement revenu ici
    //                             faute de tache prete sur ce coeur
    //
    // Le second cas est une SORTIE ACCIDENTELLE : `run_noyau` rend la main,
    // le shell reprend, l'autorun se termine et la machine s'eteint, alors
    // que le bureau n'a jamais demande a s'arreter.
    //
    // Au run 35907201865, le bras HTTP s'est eteint sur `autorun_termine`
    // SANS aucun `BOUCHAUD_BUREAU_FIN` -- or les deux seuls chemins qui
    // posent `quit` en emettent un. Cette ligne dit laquelle des deux
    // situations on est en train de vivre, au lieu de la faire deviner.
    //
    // Les processus encore vivants sont NOMMES avant d'etre tues juste en
    // dessous : sans cela, la boucle de nettoyage efface l'etat qui expliquait
    // l'arret.
    let mort = process.lifecycle.lock().threads == 0;
    {
        let vivants = processes().len();
        crate::kernel::dmesg::log_fmt(format_args!(
            "RUN_NOYAU_RETOUR t={} nom={} pid={} code={} fil_mort={} \
processus_vivants={}",
            crate::kernel::timer::monotonic_ms(),
            nom,
            pid,
            code,
            mort as u8,
            vivants,
        ));
        for reste in processes().iter() {
            crate::kernel::dmesg::log_fmt(format_args!(
                "RUN_NOYAU_VIVANT t={} pid={} nom={}",
                crate::kernel::timer::monotonic_ms(),
                reste.pid,
                reste.metadata.lock().name,
            ));
        }
    }

    // BOUCHAUD_CONTINUATION_SYNCHRONE_V1 : LE MENAGE CI-DESSOUS N'A LIEU
    // QU'APRES UNE VRAIE FIN.
    //
    // Il tue tous les processus restants et vide la table. Execute apres un
    // faux retour -- racine vivante --, il transformait une erreur locale de
    // l'ordonnanceur en extinction de la machine : a T+60,519 s sur la
    // TRIGKEY, quatorze processus tues dont le reseau, l'USB et Ladybird.
    //
    // Ce faux retour est desormais impossible par construction : seule
    // `consomme_continuation` rend cette pile, et elle refuse tant que la
    // racine vit. S'il se produit QUAND MEME, c'est une corruption : on
    // s'arrete ici, nomme, au lieu d'en effacer les traces.
    if !mort {
        panic!(
            "run_noyau: continuation reprise racine vivante nom={} pid={} -- invariant viole",
            nom, pid
        );
    }

    reap();
    for stale in processes().iter() {
        // Chaque mise a mort est nommee : c'est le seul endroit ou l'on sait
        // encore QUI a ete tue et POURQUOI -- parce que le fil appelant est
        // revenu, pas parce que ce processus avait fini.
        crate::kernel::dmesg::log_fmt(format_args!(
            "PROCESS_KILL t={} pid={} nom={} raison=run_noyau_retour parent={}",
            crate::kernel::timer::monotonic_ms(),
            stale.pid,
            stale.metadata.lock().name,
            nom,
        ));
        crate::kernel::process::kill(stale.pid);
    }
    PROCESSES.lock().clear();
    crate::kernel::process::kill(pid);
    code
}

/// Detruit les taches zombies (piles noyau, espaces d'adressage).
///
/// # Securite
/// A n'appeler que depuis le fil noyau appelant, `CURRENT` valant `usize::MAX` :
/// la table est un `Vec` et `CURRENT` en est un indice. Depuis une tache,
/// utiliser [`nettoie_zombies`].
pub fn reap() {
    // Le compactage a disparu, et c'est le registre qui l'a rendu inutile.
    //
    // `retain` supprimait les zombies en UP, ce qui DECALE les indices. Les
    // `CURRENT` par CPU en sont, et ce n'etait tolerable que parce qu'un seul
    // coeur tournait. Les emplacements du registre ayant maintenant une adresse
    // ET un indice stables a vie, compacter serait faux dans tous les cas.
    //
    // La memoire n'est pas perdue pour autant : `registre_ajoute` reutilise
    // l'emplacement d'une tache morte en ECRASANT son contenu, ce qui libere
    // au passage sa pile noyau et sa zone FPU. La reclamation a simplement lieu
    // au moment ou quelqu'un en a besoin, plutot qu'a intervalle regulier --
    // et elle ne coute plus un balayage de toute la table.
}

pub fn nettoie_zombies() {
    if smp::schedulable_cpus() <= 1 && current_index_raw() == NO_TASK {
        reap();
    }
    // SMP : aucun deplacement d'indice ; reclamation au prochain register().
}

/// Change la classe d'ordonnancement de toutes les taches d'un processus.
///
/// Variante de [`pose_priorite`] pour un processus **autre** que le courant :
/// le gestionnaire de fenetres declare interactif le navigateur qu'il vient de
/// lancer, sans que celui-ci ait a le demander.
pub fn pose_priorite_de(pid: u32, priorite: Priorite) {
    for task in tasks().iter() {
        if task.process.pid == pid {
            task.priorite.range(priorite);
        }
    }
}

/// Le processus est-il termine, et avec quel code ?
pub fn code_de_sortie(pid: u32) -> Option<i32> {
    processes().iter().find_map(|p| {
        let lifecycle = p.lifecycle.lock();
        if p.pid == pid && lifecycle.zombie {
            Some(lifecycle.exit_code)
        } else {
            None
        }
    })
}
/// Un processus et tous ses descendants, du plus proche au plus lointain.
///
/// Un navigateur n'est pas un processus, c'est un arbre : l'interface forke un
/// renderer par onglet, qui peut lui-meme forker. Fermer la fenetre en ne tuant
/// que la racine laisserait les renderers tourner sans personne pour lire ce
/// qu'ils produisent — du calcul pur, indefiniment, sur un cœur unique.
pub fn arbre_de(racine: u32) -> Vec<u32> {
    let mut cibles = vec![racine];
    let mut index = 0;
    while index < cibles.len() {
        let parent = cibles[index];
        for process in processes().iter() {
            if process.parent == parent && !cibles.contains(&process.pid) {
                cibles.push(process.pid);
            }
        }
        index += 1;
    }
    cibles
}

/// Termine de force toutes les taches d'un processus.
///
/// Employe quand le proprietaire d'une fenetre disparait : un client dont plus
/// personne ne compose la surface n'a plus de raison de peindre, et le laisser
/// vivre laisserait aussi vivante la surface qu'il projette.
pub fn tue_processus(pid: u32, code: i32) {
    // P18_SERVICE_GUARDIAN_V1: une mort imposee ne passe pas par
    // exit_current; enregistrer avant que le processus soit zombie.
    if let Some(process) = process_by_pid(pid) {
        if !process.lifecycle.lock().zombie {
            let nom = process.metadata.lock().name.clone();
            crate::kernel::services::gardien::mort(
                pid, process.parent, &nom, code, true);
        }
    }
    let courant = try_current().map(|t| t.tid);
    for task in tasks().iter() {
        if Some(task.tid) == courant {
            continue;
        }
        if task.process.pid == pid {
            marque_zombie(task);
        }
    }
    if let Some(process) = process_by_pid(pid) {
        let nouveau_zombie = {
            let mut lifecycle = process.lifecycle.lock();
            if lifecycle.zombie {
                false
            } else {
                lifecycle.threads = 0;
                lifecycle.exit_code = code;
                lifecycle.zombie = true;
                true
            }
        };
        if nouveau_zombie {
            // P18_CHILD_EXIT_NOTIFY_V1: une terminaison admin est une vraie
            // sortie de processus. Le parent peut relancer son enfant via
            // SIGCHLD/wait4 et le registre ne laisse pas un faux "vivant".
            let role = crate::kernel::navigateur::supervision::note_sortie(
                pid, code, crate::kernel::timer::monotonic_ns());
            if let Some(role) = role {
                crate::kernel::dmesg::log_fmt(format_args!(
                    "LADYBIRD_CHILD_EXIT pid={} role={} code={} source=force",
                    pid, role.nom(), code,
                ));
            }
            crate::kernel::dmesg::log_fmt(format_args!(
                "PROCESS_EXIT t={} pid={} ppid={} code={} reason=force",
                crate::kernel::timer::monotonic_ms(), pid, process.parent, code,
            ));
            notify_parent_of_exit_for(process.parent);
        }
    }
}

/// Termine tous les autres threads du processus courant.
///
/// Utilise par `execve` : apres le remplacement de l'image, il ne doit rester
/// qu'un fil, sinon les autres reprendraient dans un espace d'adressage qui
/// n'existe plus.
pub fn terminate_sibling_threads() {
    let (pid, tid) = {
        let task = current();
        (task.process.pid, task.tid)
    };
    for task in tasks().iter() {
        if task.tid != tid && task.process.pid == pid {
            marque_zombie(task);
        }
    }
}

/// Called with the BKL immediately before returning from a syscall. An exec
/// may have retired this sibling while it ran an audited BKL-bypass syscall.
/// Ne rentre pas en espace utilisateur si la tache courante a ete tuee.
///
/// Appelee a la sortie de CHAQUE appel systeme, pour une condition qui est
/// fausse presque toujours. Elle lisait `current().state`, donc `TASKS`, donc
/// exigeait le gros verrou a chaque retour d'appel systeme.
///
/// Chemin commun : une lecture atomique d'un drapeau par CPU, rien d'autre.
///
/// Chemin rare : le drapeau est leve, on RELIT
/// l'etat reel avant d'agir. Le drapeau n'est donc qu'un filtre -- s'il est
/// devenu obsolete (la tache qu'il visait a deja quitte ce CPU), la relecture
/// le constate et l'on repart sans rien faire.
///
/// Une tache qui s'execute ne change pas de CPU sans passer par un changement
/// de contexte, et un zombie n'est jamais reordonnance : le drapeau du CPU
/// courant designe donc bien la tache courante.
// BOUCHAUD_V16_2_ZOMBIE_RETIRE_NONRETURNING
//
// Un sibling tué par execve peut encore se trouver dans un syscall sans BKL.
// À la sortie du syscall, il est déjà Zombie. `schedule()` n'est cependant pas
// une primitive "ne revient jamais": s'il n'existe momentanément aucune autre
// tâche locale prête, il peut rendre la main après son chemin idle. Revenir
// ensuite dans le syscall d'un zombie est interdit et provoquait le panic
// "zombie task resumed after exec quiescence".
//
// La retraite exec est différente d'un `exit_current`: execve a déjà fixé la
// comptabilité de cycle de vie du processus à un seul thread après la
// quiescence. On ne décrémente donc pas `lifecycle.threads` et on ne notifie pas
// le parent. On retire uniquement CETTE tâche de son CPU, puis on choisit une
// autre tâche locale ou on retourne définitivement au contexte noyau du CPU.
fn retire_exec_zombie_current() -> ! {
    let cpu_id = local_cpu();
    let cur = current_index_raw();

    // BOUCHAUD_C39_PORTEES_ABANDONNEES -- voir `exit_current` : meme classe de
    // defaut, meme pile condamnee, meme compensation.
    commute_sortie_definitive_si_possible(cur, cpu_id);

    // Aucun runnable local à cet instant. Le contexte noyau/AP idle reprendra
    // le scheduling. La pile de ce sibling ne doit plus jamais être réactivée.
    switch_to_kernel("retraite_exec")
}

// BOUCHAUD_C1_RETRAITE_SANS_REPRISE_V1
//
// CE QUE LE GROS VERROU CONFIRMAIT ICI, ET CE QUI LE CONFIRME MAINTENANT
//
// Cette fonction le prenait pour « confirmer l'etat » avant de demonter la
// tache. Les trois choses qu'elle lit sont pourtant deja atomiques et deja
// LOCALES a ce CPU :
//
//   * `RETRAITE_DEMANDEE[cpu]` est un drapeau par CPU, pose par la commutation
//     de CE CPU et efface par CE CPU ; aucun autre coeur ne le touche ;
//   * `in_user_task()` lit l'indice courant de CE CPU ;
//   * `current().state` est un `EtatAtomique`.
//
// Il n'y avait donc rien a serialiser. Et la prise avait un cout reel : elle
// est sur le chemin de sortie de CHAQUE appel systeme et de chaque faute de
// page resolue -- ce n'est que le test du drapeau, faux presque toujours, qui
// l'evitait.
//
// Le gros verrou lui-meme n'existe plus (BOUCHAUD_BKL_SUPPRIME_V1).
pub fn retire_current_if_zombie() {
    let cpu = interrupts::without_interrupts(local_cpu);
    if !RETRAITE_DEMANDEE[cpu].load(Ordering::Acquire) {
        return;
    }
    // Aucun garde RAII ici : `retire_exec_zombie_current` commute sans retour,
    // et un `Drop` ne s'executerait jamais.
    RETRAITE_DEMANDEE[cpu].store(false, Ordering::Release);
    if in_user_task() && current().state == TaskState::Zombie {
        retire_exec_zombie_current();
    }
}
