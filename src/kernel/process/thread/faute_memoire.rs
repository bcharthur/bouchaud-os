/// Ressources partagees par tous les threads d'un meme programme.
/// Peuple a la demande la page fautive, si elle a ete promise.
///
/// Rend `true` si la faute est reparee et que l'instruction peut etre reprise.
///
/// C'est la contrepartie de `MAP_NORESERVE` (voir `abi::mem::sys_mmap`) : la
/// plage a ete promise sans etre peuplee, et c'est ici qu'elle le devient, une
/// page a la fois, avec les droits enregistres a la reservation.
///
/// La fonction est volontairement stricte : elle ne peuple **que** ce qui a ete
/// promis. Une faute hors de toute promesse reste une faute, et le processus
/// meurt comme avant — sans quoi le noyau transformerait chaque dereference de
/// pointeur nul en allocation silencieuse, et l'on perdrait le seul mecanisme
/// qui signale ces defauts.
static FAULTS_ZERO: AtomicU64 = AtomicU64::new(0);
static FAULTS_FILE: AtomicU64 = AtomicU64::new(0);
static FAULT_WAITS: AtomicU64 = AtomicU64::new(0);
static FAULT_RESOLVED: AtomicU64 = AtomicU64::new(0);
static FAULT_RETRY: AtomicU64 = AtomicU64::new(0);
static FAULT_INVALID: AtomicU64 = AtomicU64::new(0);
static FAULT_IO_ERROR: AtomicU64 = AtomicU64::new(0);
static FAULT_RETIRED: AtomicU64 = AtomicU64::new(0);
static PF_BKL_ENTERS: AtomicU64 = AtomicU64::new(0);
static FAULT_REGISTRY_PEAK: AtomicU64 = AtomicU64::new(0);
static FAULT_RETRY_YIELDS: AtomicU64 = AtomicU64::new(0);
static FAULT_RETRY_MAX_CHAIN: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultOutcome {
    Resolved,
    Retry,
    Invalid,
    IoError,
    Retired,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MappingPart {
    id: u64,
    start: u64,
    end: u64,
    drapeaux: u64,
    backing: PromesseBacking,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MappingToken {
    effective_id: u64,
    parts: Vec<MappingPart>,
}

fn mapping_token(regions: &[Promesse], page: u64) -> Option<MappingToken> {
    let page_end = page.checked_add(crate::kernel::vmm::PAGE_SIZE)?;
    let effective_id = crate::kernel::vma::trouve(regions, page)?.id;
    let parts = regions
        .iter()
        .filter(|region| region.chevauche(page, page_end))
        .map(|region| MappingPart {
            id: region.id,
            start: region.debut.max(page),
            end: region.fin.min(page_end),
            drapeaux: region.drapeaux,
            backing: region.backing,
        })
        .collect();
    Some(MappingToken { effective_id, parts })
}

include!("faute_cluster.rs");

#[derive(Clone, PartialEq, Eq)]
enum FaultPageState {
    Missing,
    Loading,
    Present,
    Failed(FaultOutcome),
    Cancelled,
    Retired,
}

struct FaultPage {
    pml4: u64,
    page: u64,
    token: MappingToken,
    state: SpinLock<FaultPageState>,
    waiters: crate::kernel::sync::WaitQueue,
}

static FAULT_PAGES: SpinLock<Vec<Arc<FaultPage>>> = SpinLock::new(Vec::new());

fn fault_page(pml4: u64, page: u64, token: &MappingToken) -> Arc<FaultPage> {
    let mut pages = FAULT_PAGES.lock();
    if let Some(entry) = pages.iter().find(|entry| {
        entry.pml4 == pml4 && entry.page == page && entry.token == *token
    }) {
        return Arc::clone(entry);
    }
    let entry = Arc::new(FaultPage {
        pml4,
        page,
        token: token.clone(),
        state: SpinLock::new(FaultPageState::Missing),
        waiters: crate::kernel::sync::WaitQueue::new(),
    });
    pages.push(Arc::clone(&entry));
    FAULT_REGISTRY_PEAK.fetch_max(pages.len() as u64, Ordering::Relaxed);
    entry
}

fn forget_fault_page(entry: &Arc<FaultPage>) {
    FAULT_PAGES.lock().retain(|candidate| !Arc::ptr_eq(candidate, entry));
}

pub fn forget_fault_space(pml4: u64) {
    retire_fault_records(|entry| entry.pml4 == pml4);
}

/// Retire only loaders whose virtual page intersects a changed mapping range.
/// Unrelated mmap/brk/mprotect operations therefore cannot cancel this fault.
pub fn retire_fault_range(pml4: u64, start: u64, len: u64) {
    let end = start.saturating_add(len);
    cancel_fault_records(|entry| {
        entry.pml4 == pml4 && entry.page < end
            && entry.page.saturating_add(crate::kernel::vmm::PAGE_SIZE) > start
    });
}

fn transition_fault_records(
    mut predicate: impl FnMut(&FaultPage) -> bool,
    terminal: FaultPageState,
) {
    // Publish the terminal state while every matching entry is still
    // discoverable. A concurrent lookup can therefore never recreate a second
    // loader in the remove-before-cancel window.
    let entries = {
        let registry = FAULT_PAGES.lock();
        let entries: Vec<_> = registry.iter().filter(|entry| predicate(entry)).cloned().collect();
        for entry in &entries {
            *entry.state.lock() = terminal.clone();
        }
        entries
    };
    for entry in &entries {
        entry.waiters.wake_all();
    }
    FAULT_PAGES.lock().retain(|candidate| {
        !entries.iter().any(|entry| Arc::ptr_eq(entry, candidate))
    });
}

fn cancel_fault_records(predicate: impl FnMut(&FaultPage) -> bool) {
    transition_fault_records(predicate, FaultPageState::Cancelled);
}

fn retire_fault_records(predicate: impl FnMut(&FaultPage) -> bool) {
    transition_fault_records(predicate, FaultPageState::Retired);
}

fn record_fault_outcome(outcome: FaultOutcome) {
    match outcome {
        FaultOutcome::Resolved => &FAULT_RESOLVED,
        FaultOutcome::Retry => &FAULT_RETRY,
        FaultOutcome::Invalid => &FAULT_INVALID,
        FaultOutcome::IoError => &FAULT_IO_ERROR,
        FaultOutcome::Retired => &FAULT_RETIRED,
    }
    .fetch_add(1, Ordering::Relaxed);
}

/// Resolve one demand-fault attempt. Mapping replacement is a typed retry,
/// never an accidental SIGSEGV; a genuine missing/protected VMA is Invalid.
pub fn peuple_a_la_demande(adresse: u64, protection_fault: bool) -> FaultOutcome {
    if !crate::kernel::vmm::is_user_addr(adresse) || !in_user_task() || protection_fault {
        record_fault_outcome(FaultOutcome::Invalid);
        return FaultOutcome::Invalid;
    }

    let Some(processus) = current_process_local() else {
        record_fault_outcome(FaultOutcome::Retired);
        return FaultOutcome::Retired;
    };

    let page = adresse & !(crate::kernel::vmm::PAGE_SIZE - 1);
    let (fault_pml4, token) = {
        let mm = processus.mm.lock();
        let Some(token) = mapping_token(&mm.promesses, page) else {
            record_fault_outcome(FaultOutcome::Invalid);
            return FaultOutcome::Invalid;
        };
        (mm.space.pml4(), token)
    };
    let record = fault_page(fault_pml4, page, &token);
    loop {
        let mut state = record.state.lock();
        match &*state {
            FaultPageState::Loading => {
                let ticket = record.waiters.ticket();
                drop(state);
                FAULT_WAITS.fetch_add(1, Ordering::Relaxed);
                record.waiters.wait(ticket);
            }
            FaultPageState::Present => {
                let resolved = processus.mm.lock().space.translate(page).is_some();
                if resolved {
                    record_fault_outcome(FaultOutcome::Resolved);
                    return FaultOutcome::Resolved;
                }
                *state = FaultPageState::Loading;
                break;
            }
            FaultPageState::Missing => {
                *state = FaultPageState::Loading;
                break;
            }
            FaultPageState::Failed(outcome) => {
                let outcome = *outcome;
                drop(state);
                record_fault_outcome(outcome);
                return outcome;
            }
            FaultPageState::Cancelled => {
                drop(state);
                record_fault_outcome(FaultOutcome::Retry);
                return FaultOutcome::Retry;
            }
            FaultPageState::Retired => {
                drop(state);
                record_fault_outcome(FaultOutcome::Retired);
                return FaultOutcome::Retired;
            }
        }
    }

    let loaded = peuple_page_loader(&processus, adresse, fault_pml4, &token);
    let outcome = {
        let mut state = record.state.lock();
        match *state {
            FaultPageState::Retired => FaultOutcome::Retired,
            FaultPageState::Cancelled => FaultOutcome::Retry,
            _ => {
                *state = if loaded == FaultOutcome::Resolved {
                    FaultPageState::Present
                } else {
                    FaultPageState::Failed(loaded)
                };
                loaded
            }
        }
    };
    record.waiters.wake_all();
    // The PTE (for Present) or the Arc already held by each waiter is the
    // durable state. Keeping terminal records here leaked one Vec entry per
    // fault and made future lookup O(total faults). Publish, wake, then detach.
    forget_fault_page(&record);
    record_fault_outcome(outcome);
    outcome
}

fn peuple_page_loader(
    processus: &Arc<Process>,
    adresse: u64,
    fault_pml4: u64,
    token: &MappingToken,
) -> FaultOutcome {
    stall_pf_phase(210, adresse);
    stall_pf_phase(211, adresse);
    let mut p = processus.mm.lock();
    let page = adresse & !(crate::kernel::vmm::PAGE_SIZE - 1);
    stall_pf_phase(212, page);
    if p.space.pml4() != fault_pml4 {
        return FaultOutcome::Retired;
    }
    if mapping_token(&p.promesses, page).as_ref() != Some(token) {
        return FaultOutcome::Retry;
    }

    let effective = match crate::kernel::vma::trouve(&p.promesses, page) {
        Some(region) => region,
        None => return FaultOutcome::Invalid,
    };
    if effective.drapeaux & crate::kernel::vmm::PTE_USER == 0 {
        return FaultOutcome::Invalid;
    }
    if p.space.translate(page).is_some() {
        return FaultOutcome::Resolved;
    }

    match effective.backing {
        PromesseBacking::Zero => {
            stall_pf_phase(220, page);
            let zero_id = effective.id;
            let zero_flags = effective.drapeaux;
            if !p.space.map_alloc_accounted(
                page,
                crate::kernel::vmm::PAGE_SIZE,
                zero_flags,
                crate::kernel::vmm::ResidentKind::Anonymous,
            ) {
                return FaultOutcome::IoError;
            }
            FAULTS_ZERO.fetch_add(1, Ordering::Relaxed);
            // Le fault courant est maintenant autoritaire. L'anticipation est
            // opportuniste et ne change jamais son resultat : elle valide le
            // meme VMA Zero sous Mm et s'arrete au premier doute.
            drop(p);
            fault_cluster_after_zero(processus, fault_pml4, page, zero_id, zero_flags);
            stall_pf_phase(229, page);
            FaultOutcome::Resolved
        }
        PromesseBacking::Framebuffer { phys_base, mapping_start, phys_offset } => {
            let phys = phys_base.saturating_add(phys_offset)
                .saturating_add(page.saturating_sub(mapping_start));
            if p.space.map_foreign_accounted(
                page,
                phys,
                effective.drapeaux,
                crate::kernel::vmm::ResidentKind::Device,
            ) {
                FaultOutcome::Resolved
            } else {
                FaultOutcome::IoError
            }
        }
        PromesseBacking::SharedFile { node, mapping_start, file_offset, .. } => {
            let source = file_offset.saturating_add(page.saturating_sub(mapping_start));
            let numero = source / crate::kernel::vmm::PAGE_SIZE;
            drop(p);
            let lease = match crate::kernel::partage::page(node, numero) {
                Some(lease) => lease,
                None => return FaultOutcome::IoError,
            };
            let mut p = processus.mm.lock();
            if p.space.pml4() != fault_pml4 {
                return FaultOutcome::Retired;
            }
            if mapping_token(&p.promesses, page).as_ref() != Some(token) {
                return FaultOutcome::Retry;
            }
            if p.space.translate(page).is_some() {
                return FaultOutcome::Resolved;
            }
            if !p.space.map_foreign_accounted(
                page,
                lease.frame(),
                effective.drapeaux,
                crate::kernel::vmm::ResidentKind::Shared,
            ) {
                return FaultOutcome::IoError;
            }
            FAULTS_FILE.fetch_add(1, Ordering::Relaxed);
            FaultOutcome::Resolved
        }
        PromesseBacking::File { .. } => {
            let regions = p.promesses.clone();
            let page_end = page + crate::kernel::vmm::PAGE_SIZE;
            let current_flags = effective.drapeaux;
            let mut clean_key = None;
            if effective.drapeaux & crate::kernel::vmm::PTE_WRITE == 0 {
                let covering: Vec<_> = regions.iter().filter(|region| {
                    matches!(region.backing, PromesseBacking::File { .. })
                        && region.chevauche(page, page_end)
                }).collect();
                if covering.len() == 1 {
                    if let PromesseBacking::File { node, mapping_start, file_offset, file_size } = effective.backing {
                        let data_end = mapping_start.saturating_add(file_size);
                        let offset = file_offset.saturating_add(page.saturating_sub(mapping_start));
                        if page >= mapping_start && page_end <= data_end
                            && offset % crate::kernel::vmm::PAGE_SIZE == 0
                            && crate::fs::backing::is_disk_backed(node)
                        {
                            if let Some(generation) = crate::fs::backing::generation(node) {
                                clean_key = Some(crate::kernel::clean_page_cache::Key { node, offset, generation });
                            }
                        }
                    }
                }
            }
            drop(p);

            if let Some(key) = clean_key {
                if let Some(frame) = crate::kernel::clean_page_cache::acquire(key) {
                    // No process-MM guard is held while readahead performs disk/cache work.
                    crate::kernel::readahead::observe_clean(key);
                    let mut mm = processus.mm.lock();
                    let outcome = if mm.space.pml4() != fault_pml4 {
                        FaultOutcome::Retired
                    } else if mapping_token(&mm.promesses, page).as_ref() != Some(token) {
                        FaultOutcome::Retry
                    } else if mm.space.translate(page).is_some() {
                        FaultOutcome::Resolved
                    } else if mm.space.map_foreign_accounted(
                        page,
                        frame,
                        current_flags,
                        crate::kernel::vmm::ResidentKind::FilePrivate,
                    ) {
                        mm.clean_pages.push(CleanPageMapping { virt: page, key });
                        FAULTS_FILE.fetch_add(1, Ordering::Relaxed);
                        // The current mapping reference now owns `key`. Publish
                        // verified neighbours only after dropping Mm; their cache
                        // acquisition may perform disk I/O and must never extend
                        // this critical section.
                        drop(mm);
                        fault_cluster_after_clean(processus, fault_pml4, &regions, page);
                        return FaultOutcome::Resolved;
                    } else {
                        FaultOutcome::IoError
                    };
                    crate::kernel::clean_page_cache::release(key);
                    return outcome;
                }
            }

            // Build the complete private page off-MM and publish it only
            // after the range-local token has been revalidated. Mapping a zero
            // frame before I/O allowed concurrent mprotect to leave a present
            // but never-populated page behind.
            let mut page_data = [0u8; crate::kernel::vmm::PAGE_SIZE as usize];
            for region in regions {
                if !region.chevauche(page, page_end) { continue; }
                let (node, mapping_start, file_offset, file_size) = match region.backing {
                    PromesseBacking::File { node, mapping_start, file_offset, file_size } =>
                        (node, mapping_start, file_offset, file_size),
                    _ => continue,
                };
                let start = core::cmp::max(page, mapping_start);
                let end = core::cmp::min(page_end, mapping_start.saturating_add(file_size));
                if end <= start { continue; }
                let wanted = (end - start) as usize;
                let destination = (start - page) as usize;
                let source_offset = file_offset.saturating_add(start.saturating_sub(mapping_start));
                stall_pf_file_begin(source_offset);
                let got = crate::fs::backing::read_at(
                    node,
                    source_offset as usize,
                    &mut page_data[destination..destination + wanted],
                );
                stall_pf_file_done(got, wanted);
                if got != wanted { return FaultOutcome::IoError; }
            }

            let mut mm = processus.mm.lock();
            if mm.space.pml4() != fault_pml4 { return FaultOutcome::Retired; }
            if mapping_token(&mm.promesses, page).as_ref() != Some(token) {
                return FaultOutcome::Retry;
            }
            if mm.space.translate(page).is_some() { return FaultOutcome::Resolved; }
            if !mm.space.map_alloc_accounted(
                page,
                crate::kernel::vmm::PAGE_SIZE,
                effective.drapeaux,
                crate::kernel::vmm::ResidentKind::FilePrivate,
            ) {
                return FaultOutcome::IoError;
            }
            if !mm.space.write(page, &page_data) {
                let retirement = mm.space.prepare_unmap(page, crate::kernel::vmm::PAGE_SIZE);
                drop(mm);
                let depth = smp_lock::suspend_for_schedule();
                retirement.invalidation().execute();
                smp_lock::resume_after_schedule(depth);
                processus.mm.lock().space.finish_unmap(retirement);
                return FaultOutcome::IoError;
            }
            FAULTS_FILE.fetch_add(1, Ordering::Relaxed);
            FaultOutcome::Resolved
        }
    }
}

pub fn demand_fault_stats() -> (u64, u64) {
    (
        FAULTS_ZERO.load(Ordering::Relaxed),
        FAULTS_FILE.load(Ordering::Relaxed),
    )
}

pub fn demand_fault_waits() -> u64 {
    FAULT_WAITS.load(Ordering::Relaxed)
}

pub fn fault_outcome_stats() -> (u64, u64, u64, u64, u64) {
    (
        FAULT_RESOLVED.load(Ordering::Relaxed),
        FAULT_RETRY.load(Ordering::Relaxed),
        FAULT_INVALID.load(Ordering::Relaxed),
        FAULT_IO_ERROR.load(Ordering::Relaxed),
        FAULT_RETIRED.load(Ordering::Relaxed),
    )
}

pub fn fault_registry_stats() -> (u64, u64) {
    (
        FAULT_PAGES.lock().len() as u64,
        FAULT_REGISTRY_PEAK.load(Ordering::Relaxed),
    )
}

/// Diagnostic d'une faute que la Memory Fabric n'a pas pu servir.
pub fn log_fault_mapping(adresse: u64) {
    if !in_user_task() {
        return;
    }

    let process = current_process();
    let mut p = process.mm.lock();
    let page = adresse & !(crate::kernel::vmm::PAGE_SIZE - 1);
    let present = p.space.translate(page).is_some();
    let writable = p.space.writable(page);

    if let Some(region) = crate::kernel::vma::trouve(&p.promesses, page) {
        crate::println!(
            "[memfabric] FAULT_FATAL pid={} app={} addr={:#x} page={:#x} vma={:#x}..{:#x} backing={} flags={:#x} present={} writable={}",
            process.pid,
            process.metadata.lock().name,
            adresse,
            page,
            region.debut,
            region.fin,
            region.backing.label(),
            region.drapeaux,
            present,
            writable
        );
    } else {
        let (avant, apres) =
            crate::kernel::vma::voisines(&p.promesses, page);
        crate::println!(
            "[memfabric] FAULT_FATAL pid={} app={} addr={:#x} page={:#x} AUCUNE_VMA vmas={} present={} writable={}",
            process.pid,
            process.metadata.lock().name,
            adresse,
            page,
            p.promesses.len(),
            present,
            writable
        );
        if let Some(region) = avant {
            crate::println!(
                "[memfabric] vma precedente {:#x}..{:#x} backing={} flags={:#x}",
                region.debut,
                region.fin,
                region.backing.label(),
                region.drapeaux
            );
        }
        if let Some(region) = apres {
            crate::println!(
                "[memfabric] vma suivante {:#x}..{:#x} backing={} flags={:#x}",
                region.debut,
                region.fin,
                region.backing.label(),
                region.drapeaux
            );
        }
    }
}



