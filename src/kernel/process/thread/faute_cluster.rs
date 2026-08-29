// V14 clustered publication for immutable read-only file mappings.
//
// One hardware #PF remains the authority. After its page is validated and
// published, we opportunistically map a small number of following clean pages.
// Every candidate is revalidated under Mm after its frame has been acquired,
// so mmap/munmap/mprotect can race without publishing stale PTEs.

const FAULT_CLUSTER_MAX_PAGES: u64 = 16;
static FAULT_CLUSTER_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static FAULT_CLUSTER_MAPPED: AtomicU64 = AtomicU64::new(0);
static FAULT_CLUSTER_CACHE_MISS: AtomicU64 = AtomicU64::new(0);
static FAULT_CLUSTER_ALREADY: AtomicU64 = AtomicU64::new(0);
static FAULT_CLUSTER_ABORTS: AtomicU64 = AtomicU64::new(0);
static FAULT_CLUSTER_MAX_BATCH: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
struct FaultClusterCandidate {
    page: u64,
    key: crate::kernel::clean_page_cache::Key,
    token: MappingToken,
    drapeaux: u64,
}

fn fault_cluster_candidates(
    regions: &[Promesse],
    current_page: u64,
) -> Vec<FaultClusterCandidate> {
    let page_size = crate::kernel::vmm::PAGE_SIZE;
    let Some(current) = crate::kernel::vma::trouve(regions, current_page) else {
        return Vec::new();
    };
    if current.drapeaux & crate::kernel::vmm::PTE_USER == 0
        || current.drapeaux & crate::kernel::vmm::PTE_WRITE != 0
    {
        return Vec::new();
    }
    let (node, mapping_start, file_offset, file_size) = match current.backing {
        PromesseBacking::File { node, mapping_start, file_offset, file_size } =>
            (node, mapping_start, file_offset, file_size),
        _ => return Vec::new(),
    };
    if !crate::fs::backing::is_disk_backed(node) { return Vec::new(); }
    let Some(generation) = crate::fs::backing::generation(node) else {
        return Vec::new();
    };
    let data_end = mapping_start.saturating_add(file_size);
    let effective_id = current.id;
    let mut out = Vec::with_capacity(FAULT_CLUSTER_MAX_PAGES as usize);

    for index in 1..=FAULT_CLUSTER_MAX_PAGES {
        let Some(next) = current_page.checked_add(page_size.saturating_mul(index)) else { break; };
        let Some(next_end) = next.checked_add(page_size) else { break; };
        if next < mapping_start || next_end > data_end { break; }
        let Some(region) = crate::kernel::vma::trouve(regions, next) else { break; };
        if region.id != effective_id
            || region.drapeaux & crate::kernel::vmm::PTE_USER == 0
            || region.drapeaux & crate::kernel::vmm::PTE_WRITE != 0
        {
            break;
        }
        let unique = regions.iter().filter(|candidate| {
            matches!(candidate.backing, PromesseBacking::File { .. })
                && candidate.chevauche(next, next_end)
        }).count() == 1;
        if !unique { break; }
        match region.backing {
            PromesseBacking::File {
                node: region_node,
                mapping_start: region_start,
                file_offset: region_offset,
                ..
            } if region_node == node
                && region_start == mapping_start
                && region_offset == file_offset => {}
            _ => break,
        }
        let offset = file_offset.saturating_add(next.saturating_sub(mapping_start));
        if offset % page_size != 0 { break; }
        let Some(token) = mapping_token(regions, next) else { break; };
        out.push(FaultClusterCandidate {
            page: next,
            key: crate::kernel::clean_page_cache::Key { node, offset, generation },
            token,
            drapeaux: region.drapeaux,
        });
    }
    out
}

fn fault_cluster_after_clean(
    processus: &Arc<Process>,
    fault_pml4: u64,
    regions: &[Promesse],
    current_page: u64,
) {
    let candidates = fault_cluster_candidates(regions, current_page);
    if candidates.is_empty() { return; }
    let mut mapped_batch = 0u64;
    for candidate in candidates {
        FAULT_CLUSTER_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
        let Some(frame) = crate::kernel::clean_page_cache::acquire(candidate.key) else {
            FAULT_CLUSTER_CACHE_MISS.fetch_add(1, Ordering::Relaxed);
            break;
        };

        let mut abort_mapping = false;
        let mapped = {
            let mut mm = processus.mm.lock();
            if mm.space.pml4() != fault_pml4
                || mapping_token(&mm.promesses, candidate.page).as_ref() != Some(&candidate.token)
            {
                abort_mapping = true;
                false
            } else if mm.space.translate(candidate.page).is_some() {
                FAULT_CLUSTER_ALREADY.fetch_add(1, Ordering::Relaxed);
                false
            } else if mm.space.map_foreign(candidate.page, frame, candidate.drapeaux) {
                mm.clean_pages.push(CleanPageMapping { virt: candidate.page, key: candidate.key });
                true
            } else {
                abort_mapping = true;
                false
            }
        };

        if mapped {
            FAULT_CLUSTER_MAPPED.fetch_add(1, Ordering::Relaxed);
            mapped_batch += 1;
        } else {
            crate::kernel::clean_page_cache::release(candidate.key);
        }
        if abort_mapping {
            FAULT_CLUSTER_ABORTS.fetch_add(1, Ordering::Relaxed);
            break;
        }
    }
    FAULT_CLUSTER_MAX_BATCH.fetch_max(mapped_batch, Ordering::Relaxed);
}

pub fn fault_cluster_stats() -> (u64, u64, u64, u64, u64, u64) {
    (
        FAULT_CLUSTER_ATTEMPTS.load(Ordering::Relaxed),
        FAULT_CLUSTER_MAPPED.load(Ordering::Relaxed),
        FAULT_CLUSTER_CACHE_MISS.load(Ordering::Relaxed),
        FAULT_CLUSTER_ALREADY.load(Ordering::Relaxed),
        FAULT_CLUSTER_ABORTS.load(Ordering::Relaxed),
        FAULT_CLUSTER_MAX_BATCH.load(Ordering::Relaxed),
    )
}


// V16 - clustered anonymous demand paging.
//
// Le run SMP4 montre que le cluster fichier plafonne rapidement alors que le
// nombre total de faults continue de monter par dizaines de milliers : le gros
// volume restant est l'allocation Zero (heap/arenas/stacks). On ne pre-peuple
// PAS un VMA entier. La fenetre ne s'ouvre qu'apres une vraie sequence de faults
// et reste bornee a 2/4/8 pages, soit 32 KiB maximum d'anticipation.
const ZERO_CLUSTER_CPUS: usize = 64;
const ZERO_CLUSTER_MAX_PAGES: u64 = 8;
static ZERO_EXPECTED_PML4: [AtomicU64; ZERO_CLUSTER_CPUS] =
    [const { AtomicU64::new(u64::MAX) }; ZERO_CLUSTER_CPUS];
static ZERO_EXPECTED_PAGE: [AtomicU64; ZERO_CLUSTER_CPUS] =
    [const { AtomicU64::new(u64::MAX) }; ZERO_CLUSTER_CPUS];
static ZERO_RUN: [AtomicU64; ZERO_CLUSTER_CPUS] =
    [const { AtomicU64::new(0) }; ZERO_CLUSTER_CPUS];
static ZERO_CLUSTER_FAULTS: AtomicU64 = AtomicU64::new(0);
static ZERO_CLUSTER_TRIGGERED: AtomicU64 = AtomicU64::new(0);
static ZERO_CLUSTER_MAPPED: AtomicU64 = AtomicU64::new(0);
static ZERO_CLUSTER_ALREADY: AtomicU64 = AtomicU64::new(0);
static ZERO_CLUSTER_ABORTS: AtomicU64 = AtomicU64::new(0);
static ZERO_CLUSTER_MAX_BATCH: AtomicU64 = AtomicU64::new(0);

#[inline]
fn zero_cluster_window(fault_pml4: u64, page: u64) -> u64 {
    ZERO_CLUSTER_FAULTS.fetch_add(1, Ordering::Relaxed);
    let cpu = crate::arch::x86_64::usermode::cpu_index().min(ZERO_CLUSTER_CPUS - 1);
    let expected_pml4 = ZERO_EXPECTED_PML4[cpu].load(Ordering::Relaxed);
    let expected_page = ZERO_EXPECTED_PAGE[cpu].load(Ordering::Relaxed);
    let sequential = expected_pml4 == fault_pml4 && expected_page == page;
    let run = if sequential {
        ZERO_RUN[cpu].fetch_add(1, Ordering::Relaxed) + 1
    } else {
        ZERO_RUN[cpu].store(1, Ordering::Relaxed);
        1
    };
    ZERO_EXPECTED_PML4[cpu].store(fault_pml4, Ordering::Relaxed);
    let window: u64 = if run < 2 { 0 } else if run < 4 { 2 } else if run < 8 { 4 } else { 8 };
    let step = crate::kernel::vmm::PAGE_SIZE;
    ZERO_EXPECTED_PAGE[cpu].store(
        page.saturating_add(step.saturating_mul(window.saturating_add(1))),
        Ordering::Relaxed,
    );
    window
}

fn fault_cluster_after_zero(
    processus: &Arc<Process>,
    fault_pml4: u64,
    current_page: u64,
    effective_id: u64,
    expected_flags: u64,
) {
    let window = zero_cluster_window(fault_pml4, current_page).min(ZERO_CLUSTER_MAX_PAGES);
    if window == 0 { return; }
    ZERO_CLUSTER_TRIGGERED.fetch_add(1, Ordering::Relaxed);
    let step = crate::kernel::vmm::PAGE_SIZE;
    let mut mapped_batch = 0u64;
    let mut mm = processus.mm.lock();
    if mm.space.pml4() != fault_pml4 { ZERO_CLUSTER_ABORTS.fetch_add(1, Ordering::Relaxed); return; }
    for n in 1..=window {
        let next = current_page.saturating_add(step.saturating_mul(n));
        // Extraire les metadonnees du VMA dans un petit scope : on ne garde
        // jamais une reference dans `mm.promesses` pendant qu'on emprunte
        // `mm.space` mutablement (important pour le borrow-checker du Guard).
        let candidate_flags = match crate::kernel::vma::trouve(&mm.promesses, next) {
            Some(region)
                if region.id == effective_id
                    && region.drapeaux == expected_flags
                    && matches!(region.backing, PromesseBacking::Zero) => region.drapeaux,
            _ => break,
        };
        if mm.space.translate(next).is_some() {
            ZERO_CLUSTER_ALREADY.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        if !mm.space.map_alloc(next, step, candidate_flags) {
            ZERO_CLUSTER_ABORTS.fetch_add(1, Ordering::Relaxed);
            break;
        }
        // Ceci n'est PAS une faute materielle supplementaire : la page a ete
        // anticipee. `FAULTS_ZERO` reste donc le compteur des #PF Zero reels.
        ZERO_CLUSTER_MAPPED.fetch_add(1, Ordering::Relaxed);
        mapped_batch += 1;
    }
    ZERO_CLUSTER_MAX_BATCH.fetch_max(mapped_batch, Ordering::Relaxed);
}

pub fn zero_fault_cluster_stats() -> (u64, u64, u64, u64, u64, u64) {
    (
        ZERO_CLUSTER_FAULTS.load(Ordering::Relaxed),
        ZERO_CLUSTER_TRIGGERED.load(Ordering::Relaxed),
        ZERO_CLUSTER_MAPPED.load(Ordering::Relaxed),
        ZERO_CLUSTER_ALREADY.load(Ordering::Relaxed),
        ZERO_CLUSTER_ABORTS.load(Ordering::Relaxed),
        ZERO_CLUSTER_MAX_BATCH.load(Ordering::Relaxed),
    )
}
