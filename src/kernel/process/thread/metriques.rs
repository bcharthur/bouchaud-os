/// Snapshot SMP-NG2: charge physique, pression de runqueue, tache courante,
/// steals et migrations par CPU.
pub fn log_smp_load() {
    // Releve de DIAGNOSTIC. Il lisait toute la table sous gros verrou, une fois
    // par seconde, en bloquant les quatre coeurs pendant qu'il formatait du
    // texte. Le registre se lit sans verrou, et un compteur observe une
    // nanoseconde trop tot ne change aucune conclusion -- alors qu'un releve
    // qui fige la machine en change beaucoup.
    let online = smp::schedulable_cpus().max(1).min(MAX_CPUS);
    let mut line = alloc::string::String::from("[SMP-LOAD]");
    line.push_str(&alloc::format!(
        " total={} tlb={}",
        cpu::load_percent(),
        smp::tlb_shootdown_count(),
    ));

    for cpu_id in 0..online {
        let current_index = CURRENT[cpu_id].load(Ordering::Acquire);
        let (tid, pid) = if current_index != NO_TASK && current_index < tasks().len() {
            let t = &tasks()[current_index];
            (t.tid, t.process.pid)
        } else {
            (0, 0)
        };
        line.push_str(&alloc::format!(
            " c{}={} rq={} cur={}:{} steal={}/{} rej_bal={} rej_aff={} mig={}",
            cpu_id,
            cpu::load_percent_cpu(cpu_id),
            ready_count_cpu(cpu_id),
            pid,
            tid,
            RUNQ_STEALS[cpu_id].load(Ordering::Relaxed),
            STEAL_ATTEMPTS[cpu_id].load(Ordering::Relaxed),
            STEAL_REJECT_BALANCE[cpu_id].load(Ordering::Relaxed),
            STEAL_REJECT_AFFINITY[cpu_id].load(Ordering::Relaxed),
            CPU_MIGRATIONS[cpu_id].load(Ordering::Relaxed),
        ));
    }
    crate::kernel::dmesg::log_fmt(format_args!("{}", line));
    let (bkl_wait, bkl_hold, bkl_acq) = smp_lock::contention_stats();
    let (acq_enter, acq_try, acq_resume) = smp_lock::acquisitions_par_origine();
    let (max_tenue, max_site) = smp_lock::plus_longue_tenue();
    let (parked_waiters, parks, wake_ipis) = smp_lock::park_stats();
    // BOUCHAUD_P1_BKL_MAX_HOLD_PROVENANCE_V1 : le texte est fabrique ICI, une
    // fois par releve. Le chemin d'acquisition ne range que des entiers.
    let (max_cpu, max_tache, max_syscall, max_phase, max_site_acq, max_origine) =
        smp_lock::provenance_plus_longue_tenue();
    crate::kernel::dmesg::log_fmt(format_args!(
        "[BKL-STATS] wait_ns={} hold_ns={} acquisitions={} enter={} try_enter={} resume={} max_hold_ns={} max_hold_site={} preempt_irq_bkl_tenu={} identite_repli={} parked_waiters={} parks={} wake_ipis={}",
        bkl_wait, bkl_hold, bkl_acq, acq_enter, acq_try, acq_resume,
        max_tenue, max_site, preempt_irq_bkl_tenu(), identite_repli(),
        parked_waiters, parks, wake_ipis,
    ));
    crate::kernel::dmesg::log_fmt(format_args!(
        "[BKL-MAX-HOLD] ns={} cpu={} task={} syscall={} site_acquisition={} origine={} site_tenue={}",
        max_tenue,
        Absent(max_cpu as u64),
        Absent(max_tache as u64),
        EtatSyscall { nr: max_syscall, phase: max_phase, age_ticks: 0 },
        max_site_acq,
        match max_origine {
            1 => "enter",
            2 => "try_enter",
            3 => "resume_after_schedule",
            _ => "none",
        },
        max_site,
    ));
    // BOUCHAUD_P0_BKL_COMPTABILITE_V1
    //
    // Les grandeurs sont SEPAREES, et leurs unites annoncees : les melanger
    // est ce qui avait produit un `hold_pct` de 183 % sur un verrou exclusif.
    //
    //   tenue_ns    temps de muraille -- majore par la fenetre, comparable a elle
    //   attente_ns  temps de CPU -- quatre coeurs qui attendent en cumulent quatre
    //   reprise_ns  la part de l'attente subie apres une commutation
    //
    // `anomalies=0/0/0` est la CONDITION pour croire les trois premieres. Non
    // nulles, elles disent ou le modele s'est decroche de la machine, au lieu
    // de laisser un cumul absurde le suggerer.
    let c = smp_lock::comptes();
    let mut ventilation = String::new();
    for cpu_id in 0..online.min(MAX_CPUS) {
        let _ = core::fmt::Write::write_fmt(
            &mut ventilation,
            format_args!(
                " c{}=[parks_sur={} wakes_recus={}]",
                cpu_id,
                smp_lock::parks_sur(cpu_id),
                smp_lock::wakes_vers(cpu_id),
            ),
        );
    }
    crate::kernel::dmesg::log_fmt(format_args!(
        "[BKL-COMPTES] tenue_ns={} attente_ns={} attente_max_ns={} \
attente_max=[origine={} cpu={} appel={}] reprise_ns={} reprise_max_ns={} \
spins={} spins_irq_masquees={} parks={} wake_ipis={} reveils_sans_acq={} liberations_migrees={} \
anomalies={}/{}/{} proprietaire={}{}",
        c.tenue_ns, c.attente_ns, c.attente_max_ns,
        match c.attente_max_origine {
            1 => "enter",
            2 => "try_enter",
            3 => "resume_after_schedule",
            _ => "none",
        },
        Absent(c.attente_max_cpu as u64),
        if c.attente_max_seau == smp_lock::SEAU_NOYAU {
            "hors-syscall"
        } else {
            crate::kernel::abi::nr::name(c.attente_max_seau as u64)
        },
        c.reprise_ns, c.reprise_max_ns,
        c.spins, c.spins_irq_masquees, c.parks, c.wake_ipis, c.reveils_sans_acquisition,
        c.liberations_migrees,
        c.sans_debut, c.sur_tenue, c.horloge_a_rebours,
        Absent(c.proprietaire as u64),
        ventilation,
    ));
    // BOUCHAUD_C1_ATTRIBUTION_DOMAINE_V1
    //
    // Le chiffre du chantier « sortie du gros verrou ». `normaux` exclut le
    // boot precoce et la panique, ou le verrou reste legitime : les inclure
    // rendrait l'objectif inatteignable par construction, donc inutile.
    //
    // `regressions` doit valoir zero POUR TOUJOURS. Non nul, un chemin declare
    // sorti l'a repris, et le domaine fautif est NOMME -- ce qu'aucun total
    // d'acquisitions ne pouvait dire.
    {
        use crate::kernel::sync::{domaine, registre_domaines, Contrat, Domaine};
        let registre = registre_domaines();
        let mut ligne = alloc::string::String::from("[BKL-DOMAINES]");
        let _ = core::fmt::Write::write_fmt(
            &mut ligne,
            format_args!(
                " normaux={} regressions={} debordements={} premiere_regression={}",
                registre.acquisitions_chemins_normaux(),
                registre.total_violations(),
                registre.debordements(),
                match registre.premiere_regression() {
                    Some(fautif) => fautif.nom(),
                    None => "aucune",
                },
            ),
        );
        for code in 0..domaine::NOMBRE as u8 {
            let d = Domaine::depuis_code(code);
            let acquisitions = registre.acquisitions(d);
            // Un domaine a zero n'apprend rien tant qu'il n'a rien promis ;
            // un domaine SORTI a zero est au contraire la preuve recherchee.
            if acquisitions == 0 && !matches!(d.contrat(), Contrat::Migre) {
                continue;
            }
            let _ = core::fmt::Write::write_fmt(
                &mut ligne,
                format_args!(
                    " {}=[{} acq={} regressions={}]",
                    d.nom(), d.contrat().nom(), acquisitions, registre.violations(d),
                ),
            );
        }
        crate::kernel::dmesg::log_fmt(format_args!("{}", ligne));
    }
    // BOUCHAUD_C2_LATENCE_DANS_CHAQUE_TRACE_V1
    //
    // Ces deux releves n'etaient emis que par le rapport du navigateur, donc
    // uniquement quand un navigateur tournait. Les traces de stress -- memoire
    // SMP4, primitives, boot -- n'en portaient aucun, et les budgets de
    // latence n'avaient rien a verifier : ils ressortaient « absent du
    // journal », ce qui est honnete mais inutile.
    //
    // Ils sortent maintenant avec le reste du releve periodique. Ce sont les
    // deux chiffres qui mesurent ce que l'utilisateur RESSENT -- une
    // preemption reportee, une tache prete qui attend son coeur -- et ils
    // doivent exister dans toute trace ou l'on cherche un figement.
    crate::kernel::scheduler::preempt::log_stats();
    crate::kernel::scheduler::latency::log_stats();
    let (_, _, backing_reads, backing_bytes) = crate::fs::backing::stats();
    let (cache_hits, readahead_hits) = crate::fs::backing::cache_stats();
    let readahead_pages = crate::fs::backing::readahead_pages();
    let (clean_hits, clean_misses, clean_waits, clean_shared) =
        crate::kernel::clean_page_cache::stats();
    crate::kernel::dmesg::log_fmt(format_args!(
        "[BACKING-CACHE] reads={} bytes={} hits={} readahead_hits={} readahead_pages={} clean_hit={} clean_miss={} clean_wait={} clean_shared={} fault_wait={}",
        backing_reads, backing_bytes, cache_hits, readahead_hits, readahead_pages,
        clean_hits, clean_misses, clean_waits, clean_shared, demand_fault_waits(),
    ));
    let (resolved, retry, invalid, io_error, retired) = fault_outcome_stats();
    let (waitq_bkl, waitq_bkl_ns) = crate::kernel::sync::waitq_bkl_stats();
    let (waitq_detached, waitq_legacy, waitq_detached_ns, waitq_detached_max_ns, waitq_detached_loops, waitq_depth_violations) = crate::kernel::sync::waitq_detached_stats();
    let waitq_sans_verrou = crate::kernel::sync::waitq_wake_sans_verrou();
    let (ata_acquires, ata_wait_ns, ata_max_ns) = crate::drivers::ata::contention_stats();
    let (exec_wait_ns, exec_max_ns) = crate::kernel::abi::proc::exec_quiesce_stats();
    let (fault_registry_current, fault_registry_peak) = fault_registry_stats();
    let (retry_yields, retry_max_chain) = fault_retry_stats();
    let (clean_entries, clean_reclaimable) = crate::kernel::clean_page_cache::lifetime_stats();
    let (shared_nodes, shared_pages, shared_orphans) = crate::kernel::partage::lifetime_stats();
    crate::kernel::dmesg::log_fmt(format_args!(
        "[MM-NG6] fault_resolved={} fault_retry={} fault_invalid={} fault_io_error={} fault_retired={} fault_retry_yields={} fault_retry_max_chain={} fault_registry_current={} fault_registry_peak={} clean_cache_entries={} clean_cache_reclaimable={} shared_cache_nodes={} shared_cache_pages={} shared_cache_orphans={} pf_bkl_enters={} waitq_bkl_enters={} waitq_bkl_wait_ns={} waitq_wake_sans_verrou={} exec_wait_ns={} exec_max_ns={} ata_acquires={} ata_wait_ns={} ata_max_ns={}",
        resolved, retry, invalid, io_error, retired, retry_yields, retry_max_chain,
        fault_registry_current, fault_registry_peak,
        clean_entries, clean_reclaimable, shared_nodes, shared_pages, shared_orphans,
        pf_bkl_enters(), waitq_bkl, waitq_bkl_ns, waitq_sans_verrou,
        exec_wait_ns,
        exec_max_ns, ata_acquires, ata_wait_ns, ata_max_ns,
    ));
    let (cluster_attempts, cluster_mapped, cluster_miss, cluster_already, cluster_aborts, cluster_max_batch) = fault_cluster_stats();
    crate::kernel::dmesg::log_fmt(format_args!(
        "[MM-CLUSTER] attempts={} mapped={} cache_miss={} already={} aborts={} max_batch={}",
        cluster_attempts, cluster_mapped, cluster_miss, cluster_already, cluster_aborts, cluster_max_batch,
    ));
    let (zero_faults, zero_triggered, zero_mapped, zero_already, zero_aborts, zero_max_batch) = zero_fault_cluster_stats();
    crate::kernel::dmesg::log_fmt(format_args!(
        "[MM-ZERO-CLUSTER] faults={} triggered={} mapped={} already={} aborts={} max_batch={}",
        zero_faults, zero_triggered, zero_mapped, zero_already, zero_aborts, zero_max_batch,
    ));
    crate::serial_println!(
        "[WAITQ-DETACHED] waits={} legacy={} wait_ns={} wait_max_ns={} schedule_loops={} depth_violations={}",
        waitq_detached,
        waitq_legacy,
        waitq_detached_ns,
        waitq_detached_max_ns,
        waitq_detached_loops,
        waitq_depth_violations,
    );
    log_smp_sample(online);
}

fn sample_list(values: &[u64], online: usize) -> String {
    let mut out = String::from("[");
    for cpu in 0..online {
        if cpu != 0 { out.push(','); }
        out.push_str(&alloc::format!("{}", values[cpu]));
    }
    out.push(']');
    out
}

fn log_smp_sample(online: usize) {
    let now = crate::kernel::timer::monotonic_ns();
    let mut current = SmpSamplePrevious {
        t_ns: now,
        ctx: CONTEXT_SWITCHES.load(Ordering::Relaxed),
        migrations: [0; MAX_CPUS],
        steal_ok: [0; MAX_CPUS],
        steal_try: [0; MAX_CPUS],
        reject_balance: [0; MAX_CPUS],
        reject_affinity: [0; MAX_CPUS],
        page_faults: [0; MAX_CPUS],
        tlb: smp::tlb_shootdown_count(),
        bkl_wait: 0,
        bkl_hold: 0,
        bkl_acq: 0,
        gpu_presents: 0,
        gpu_bytes: 0,
        irq_preemptions: IRQ_PREEMPTIONS.load(Ordering::Relaxed),
        deferred_preemptions: DEFERRED_PREEMPTIONS.load(Ordering::Relaxed),
    };
    (current.bkl_wait, current.bkl_hold, current.bkl_acq) = smp_lock::contention_stats();
    let gpu = crate::drivers::gpu::stats();
    current.gpu_presents = gpu.presents;
    current.gpu_bytes = gpu.bytes_presented;
    let mut load = [0u64; MAX_CPUS];
    let mut runnable = [0u64; MAX_CPUS];
    let mut rq = [0u64; MAX_CPUS];
    for cpu in 0..online {
        load[cpu] = crate::arch::x86_64::cpu::load_percent_cpu(cpu) as u64;
        runnable[cpu] = ready_count_cpu(cpu) as u64;
        if let Some(id) = crate::arch::x86_64::cpu_local::CpuId::from_index(cpu) {
            rq[cpu] = crate::arch::x86_64::cpu_local::local(id).run_queue_len() as u64;
        }
        current.migrations[cpu] = CPU_MIGRATIONS[cpu].load(Ordering::Relaxed);
        current.steal_ok[cpu] = RUNQ_STEALS[cpu].load(Ordering::Relaxed);
        current.steal_try[cpu] = STEAL_ATTEMPTS[cpu].load(Ordering::Relaxed);
        current.reject_balance[cpu] = STEAL_REJECT_BALANCE[cpu].load(Ordering::Relaxed);
        current.reject_affinity[cpu] = STEAL_REJECT_AFFINITY[cpu].load(Ordering::Relaxed);
        current.page_faults[cpu] = STALL_PF_BEGIN[cpu].load(Ordering::Relaxed);
    }
    let previous = unsafe { SMP_SAMPLE_PREVIOUS.replace(current) };
    let Some(previous) = previous else { return; };
    let elapsed = now.saturating_sub(previous.t_ns);
    if elapsed < 500_000_000 { return; }
    let delta = |a: &[u64; MAX_CPUS], b: &[u64; MAX_CPUS]| {
        let mut out = [0u64; MAX_CPUS];
        for cpu in 0..online { out[cpu] = a[cpu].saturating_sub(b[cpu]); }
        out
    };
    let migrations = delta(&current.migrations, &previous.migrations);
    crate::kernel::dmesg::log_fmt(format_args!(
        "[SMP-SAMPLE] v=2 t_ns={} window_ns={} load={} runnable={} rq={} ctx_delta={} mig_delta={} steal_ok_delta={} steal_try_delta={} steal_rej_bal_delta={} steal_rej_aff_delta={} bkl_wait_delta_ns={} bkl_hold_delta_ns={} bkl_acq_delta={} pf_delta={} tlb_delta={} irq_preempt_delta={} deferred_preempt_delta={} fb_presents_delta={} fb_bytes_delta={}",
        now, elapsed, sample_list(&load, online), sample_list(&runnable, online),
        sample_list(&rq, online), current.ctx.saturating_sub(previous.ctx),
        migrations[..online].iter().copied().sum::<u64>(),
        sample_list(&delta(&current.steal_ok, &previous.steal_ok), online),
        sample_list(&delta(&current.steal_try, &previous.steal_try), online),
        sample_list(&delta(&current.reject_balance, &previous.reject_balance), online),
        sample_list(&delta(&current.reject_affinity, &previous.reject_affinity), online),
        current.bkl_wait.saturating_sub(previous.bkl_wait),
        current.bkl_hold.saturating_sub(previous.bkl_hold),
        current.bkl_acq.saturating_sub(previous.bkl_acq),
        sample_list(&delta(&current.page_faults, &previous.page_faults), online),
        current.tlb.saturating_sub(previous.tlb),
        current.irq_preemptions.saturating_sub(previous.irq_preemptions),
        current.deferred_preemptions.saturating_sub(previous.deferred_preemptions),
        current.gpu_presents.saturating_sub(previous.gpu_presents),
        current.gpu_bytes.saturating_sub(previous.gpu_bytes),
    ));
    publie_bkl_par_appel(elapsed);
}

// BOUCHAUD_P2_BKL_PAR_APPEL_V1
//
// « BKL detenu 99,96 % de la fenetre » dit qu'il y a un probleme, pas OU. Le
// maximum et sa provenance donnent UN coupable — celui d'une seule tenue. Ils
// ne disent pas s'il est isole ou systematique, ni ce que les autres appels
// coutent a cote.
//
// Cette ligne classe les appels systeme par temps de DETENTION sur la fenetre.
// C'est elle qui permet d'affirmer un avant/apres chiffre plutot qu'une
// impression.
static mut BKL_APPEL_PRECEDENT: Option<alloc::vec::Vec<u64>> = None;

fn publie_bkl_par_appel(fenetre_ns: u64) {
    let seaux = smp_lock::nombre_de_seaux();
    let mut hold = alloc::vec![0u64; seaux];
    for index in 0..seaux {
        hold[index] = smp_lock::stats_du_seau(index).0;
    }
    let precedent = unsafe {
        let ancien = BKL_APPEL_PRECEDENT.replace(hold.clone());
        ancien
    };
    let Some(precedent) = precedent else { return; };
    if precedent.len() != seaux {
        return;
    }

    // Les trois plus gros consommateurs de la fenetre. Trois, parce qu'une
    // ligne de journal qui deroule cinquante appels ne se lit pas — et parce
    // qu'un quatrieme n'a jamais rien explique jusqu'ici.
    let mut classement: [(usize, u64); 3] = [(usize::MAX, 0); 3];
    for index in 0..seaux {
        let delta = hold[index].saturating_sub(precedent[index]);
        if delta == 0 {
            continue;
        }
        if delta > classement[2].1 {
            classement[2] = (index, delta);
            classement.sort_by(|a, b| b.1.cmp(&a.1));
        }
    }
    if classement[0].0 == usize::MAX {
        return;
    }

    let mut ligne = alloc::string::String::from("[BKL-SYSCALL]");
    let _ = core::fmt::Write::write_fmt(
        &mut ligne,
        format_args!(" window_ns={fenetre_ns}"),
    );
    for (index, delta) in classement.iter().copied() {
        if index == usize::MAX {
            continue;
        }
        let (_, attente, acquisitions, max_hold) = smp_lock::stats_du_seau(index);
        let nom = if index == smp_lock::SEAU_NOYAU {
            "hors-syscall"
        } else {
            crate::kernel::abi::nr::name(index as u64)
        };
        let part = if fenetre_ns == 0 { 0 } else { delta.saturating_mul(100) / fenetre_ns };
        let _ = core::fmt::Write::write_fmt(
            &mut ligne,
            format_args!(
                " {nom}=[hold_delta_ns={delta} hold_pct={part} \
                 max_hold_ns={max_hold} acq_total={acquisitions} wait_total_ns={attente}]"
            ),
        );
    }
    crate::kernel::dmesg::log_fmt(format_args!("{}", ligne));
}

/// Instantane d'un processus pour le journal : (pid, nom, ticks, octets).
pub struct Mesure {
    pub pid: u32,
    pub nom: String,
    pub resource_group_id: u32,
    pub resource_group_name: String,
    /// Ticks CPU consommes depuis le dernier releve.
    pub ticks: u64,
    /// Compatibilite historique : desormais RSS reel, pas taille virtuelle.
    pub octets: u64,
    pub rss_octets: u64,
    pub vss_octets: u64,
    pub taches: usize,
    pub cpu_map_ns: [u64; MAX_CPUS],
    pub migrations: u64,
    pub context_switches: u64,
    pub runnable_threads: usize,
}

/// Compteurs de la derniere mesure, pour rendre un delta plutot qu'un cumul.
///
/// Un cumul depuis le demarrage ne dit rien d'utile : au bout d'une minute,
/// tout le monde a « beaucoup » de ticks. Ce qu'on veut lire, c'est ce qui s'est
/// passe depuis la ligne precedente du journal.
static mut MESURE_PRECEDENTE: Option<Vec<(u32, u64, [u64; MAX_CPUS], u64, u64)>> = None;
static mut MESURE_NS_PRECEDENT: u64 = 0;
/// Runtime par TID au snapshot précédent, uniquement pour vérifier l'invariant
/// qu'un thread ne peut consommer plus d'un CPU logique sur une fenêtre.
static mut MESURE_TACHE_PRECEDENTE: Option<Vec<(u32, u64)>> = None;

/// Mesure tous les processus vivants et remet les compteurs a la reference.
///
/// Rend aussi le nombre total de ticks ecoules sur la periode, seul denominateur
/// honnete d'un pourcentage : compter sur l'horloge murale donnerait des totaux
/// qui depassent 100 % des que la machine dort.
pub fn mesure_processus() -> (Vec<Mesure>, u64) {
    let now = crate::kernel::timer::monotonic_ns();
    let previous_ns = unsafe { MESURE_NS_PRECEDENT };
    let window = if previous_ns == 0 { now.max(1) } else { now.saturating_sub(previous_ns).max(1) };
    let previous_tasks = unsafe { MESURE_TACHE_PRECEDENTE.clone().unwrap_or_default() };
    let mut current_tasks: Vec<(u32, u64)> = Vec::new();
    let mut cumuls: Vec<(u32, u64, [u64; MAX_CPUS], u64, u64)> = Vec::new();
    let mut mesures: Vec<Mesure> = Vec::new();

    for task in tasks().iter() {
        if task.state == TaskState::Zombie {
            continue;
        }
        let (pid, nom, group_id, group_name, rss_octets, vss_octets) = {
            let process = &task.process;
            let usage = crate::kernel::resource::memory_usage(process);
            let name = process.metadata.lock().name.clone();
            (process.pid, name, process.resource_group_id,
                process.resource_group_name.clone(), usage.rss, usage.vss)
        };
        // Inclure la tranche actuellement en cours sans modifier le curseur :
        // le delta du prochain snapshot soustraira exactement ce même préfixe.
        let live = if task.last_account_ns != 0 {
            now.saturating_sub(task.last_account_ns.charge())
        } else { 0 };
        let runtime = task.user_cpu_ns.charge()
            .saturating_add(task.kernel_cpu_ns.charge())
            .saturating_add(live);
        // Instantane du temps par coeur. Les cases sont atomiques ; on les
        // recopie en valeurs pour le calcul qui suit.
        let mut cpu_map_snapshot = [0u64; MAX_CPUS];
        for cpu in 0..MAX_CPUS {
            cpu_map_snapshot[cpu] = task.cpu_ns[cpu].charge();
        }
        if live != 0 && task.on_cpu >= 0 {
            let cpu = task.on_cpu.charge() as usize;
            if cpu < MAX_CPUS { cpu_map_snapshot[cpu] = cpu_map_snapshot[cpu].saturating_add(live); }
        }
        if let Some((_, before)) = previous_tasks.iter().find(|(tid, _)| *tid == task.tid) {
            let delta = runtime.saturating_sub(*before);
            debug_assert!(delta <= window.saturating_add(1_000_000),
                "task: runtime > fenêtre tid={} delta={} window={}", task.tid, delta, window);
        }
        current_tasks.push((task.tid, runtime));
        match cumuls.iter_mut().find(|(autre, _, _, _, _)| *autre == pid) {
            Some((_, total, cpu_map, migrations, switches)) => {
                *total = total.saturating_add(runtime);
                *migrations = migrations.saturating_add(task.migrations.charge());
                *switches = switches.saturating_add(task.context_switches.charge());
                for cpu in 0..MAX_CPUS {
                    cpu_map[cpu] = cpu_map[cpu].saturating_add(cpu_map_snapshot[cpu]);
                }
            }
            None => {
                cumuls.push((pid, runtime, cpu_map_snapshot, task.migrations.charge(), task.context_switches.charge()));
                mesures.push(Mesure {
                    pid,
                    nom,
                    resource_group_id: group_id,
                    resource_group_name: group_name,
                    ticks: 0,
                    octets: rss_octets,
                    rss_octets,
                    vss_octets,
                    taches: 0,
                    cpu_map_ns: [0; MAX_CPUS],
                    migrations: 0,
                    context_switches: 0,
                    runnable_threads: 0,
                });
            }
        }
        if let Some(mesure) = mesures.iter_mut().find(|m| m.pid == pid) {
            mesure.taches += 1;
            mesure.migrations = mesure.migrations.saturating_add(task.migrations.charge());
            mesure.context_switches =
                mesure.context_switches.saturating_add(task.context_switches.charge());
            if task.state == TaskState::Ready {
                mesure.runnable_threads += 1;
            }
        }
    }

    let precedents = unsafe {
        let pointeur = &raw mut MESURE_PRECEDENTE;
        if (*pointeur).is_none() {
            *pointeur = Some(Vec::new());
        }
        (*pointeur).as_ref().unwrap().clone()
    };

    for mesure in mesures.iter_mut() {
        let cumul = cumuls
            .iter()
            .find(|(pid, _, _, _, _)| *pid == mesure.pid)
            .map_or(0, |(_, total, _, _, _)| *total);
        let avant = precedents
            .iter()
            .find(|(pid, _, _, _, _)| *pid == mesure.pid)
            .map_or(0, |(_, total, _, _, _)| *total);
        mesure.ticks = cumul.saturating_sub(avant);
        let current_map = cumuls
            .iter()
            .find(|(pid, _, _, _, _)| *pid == mesure.pid)
            .map_or([0; MAX_CPUS], |(_, _, map, _, _)| *map);
        let previous_map = precedents
            .iter()
            .find(|(pid, _, _, _, _)| *pid == mesure.pid)
            .map_or([0; MAX_CPUS], |(_, _, map, _, _)| *map);
        for cpu in 0..MAX_CPUS {
            mesure.cpu_map_ns[cpu] = current_map[cpu].saturating_sub(previous_map[cpu]);
        }
        let (_, _, _, current_migrations, current_switches) = cumuls
            .iter().find(|(pid, _, _, _, _)| *pid == mesure.pid)
            .copied().unwrap_or((mesure.pid, 0, [0; MAX_CPUS], 0, 0));
        let (_, _, _, previous_migrations, previous_switches) = precedents
            .iter().find(|(pid, _, _, _, _)| *pid == mesure.pid)
            .copied().unwrap_or((mesure.pid, 0, [0; MAX_CPUS], 0, 0));
        mesure.migrations = current_migrations.saturating_sub(previous_migrations);
        mesure.context_switches = current_switches.saturating_sub(previous_switches);
    }

    unsafe {
        MESURE_PRECEDENTE = Some(cumuls);
        MESURE_TACHE_PRECEDENTE = Some(current_tasks);
        MESURE_NS_PRECEDENT = now;
    }
    // Vue processus: 100% représente un CPU logique complet; un processus
    // multithread peut donc atteindre N*100%. La topbar conserve séparément
    // sa convention 100%=machine entière.
    (mesures, window)
}

/// Signale qu'une commutation est souhaitable au prochain point sur.
pub fn set_need_resched() {
    NEED_RESCHED[local_cpu()].store(true, Ordering::Release);
}

pub fn take_need_resched() -> bool {
    NEED_RESCHED[local_cpu()].swap(false, Ordering::AcqRel)
}

pub fn request_deferred_preempt() {
    DEFERRED_PREEMPTIONS.fetch_add(1, Ordering::Relaxed);
    set_need_resched();
}

pub fn current_is_kernel_task() -> bool {
    CURRENT_IS_KERNEL[local_cpu()].load(Ordering::Acquire)
}

pub fn note_wm_heartbeat() {
    WM_HEARTBEAT_TICK.store(crate::kernel::timer::ticks(), Ordering::Relaxed);
    WM_WATCHDOG_ARMED.store(true, Ordering::Release);
}

/// Tick du dernier battement du bureau. Deuxieme reponse de la chaine de
/// diagnostic : le noyau vit-il, et le fil du bureau vit-il ?
pub fn wm_heartbeat() -> u64 {
    WM_HEARTBEAT_TICK.load(Ordering::Relaxed)
}

pub fn watchdog_from_timer() {
    if !WM_WATCHDOG_ARMED.load(Ordering::Acquire) { return; }
    let now = crate::kernel::timer::ticks();
    let heartbeat = WM_HEARTBEAT_TICK.load(Ordering::Relaxed);
    let last_warning = WM_LAST_WARNING_TICK.load(Ordering::Relaxed);
    let silence = now.wrapping_sub(heartbeat);
    let seuil = 2 * crate::kernel::timer::TICKS_PER_SECOND;
    // V14: keep the 2 s detection threshold, but serialise at most one warning
    // every 10 s. Under TCG, printing diagnostics is itself emulated I/O.
    let periode_rapport = 10 * crate::kernel::timer::TICKS_PER_SECOND;
    if silence >= seuil && now.wrapping_sub(last_warning) >= periode_rapport {
        WM_LAST_WARNING_TICK.store(now, Ordering::Relaxed);
        crate::serial_println!(
            "[sched-watchdog] desktop sans heartbeat depuis {} ms ; switches={} irq-preempt={} deferred={} online={}",
            silence.saturating_mul(1000) / crate::kernel::timer::TICKS_PER_SECOND,
            CONTEXT_SWITCHES.load(Ordering::Relaxed),
            IRQ_PREEMPTIONS.load(Ordering::Relaxed),
            DEFERRED_PREEMPTIONS.load(Ordering::Relaxed),
            smp::schedulable_cpus(),
        );
    }
}

#[derive(Clone, Copy, Debug)]
pub struct OrdonnanceurStats {
    pub switches: u64,
    pub irq_preemptions: u64,
    pub deferred_preemptions: u64,
    pub transitions: u64,
    pub transitions_refusees: u64,
    pub detach_bkl_legacy: u64,
    pub wm_age_ms: u64,
    pub ready: usize,
    pub live: usize,
}

pub fn diagnostic_ordonnanceur() -> OrdonnanceurStats {
    let now = crate::kernel::timer::ticks();
    let heartbeat = WM_HEARTBEAT_TICK.load(Ordering::Relaxed);
    OrdonnanceurStats {
        switches: CONTEXT_SWITCHES.load(Ordering::Relaxed),
        irq_preemptions: IRQ_PREEMPTIONS.load(Ordering::Relaxed),
        deferred_preemptions: DEFERRED_PREEMPTIONS.load(Ordering::Relaxed),
        transitions: TRANSITIONS_ORDONNANCEUR.load(Ordering::Relaxed),
        transitions_refusees: TRANSITIONS_ORDONNANCEUR_REFUSEES.load(Ordering::Relaxed),
        detach_bkl_legacy: DETACHEMENTS_BKL_LEGACY.load(Ordering::Relaxed),
        wm_age_ms: now.saturating_sub(heartbeat).saturating_mul(1000)
            / crate::kernel::timer::TICKS_PER_SECOND,
        ready: ready_count(),
        live: live_count(),
    }
}
