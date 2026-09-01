#[inline]
pub fn stall_site_set(site: u32, aux: u64) {
    let cpu = local_cpu();
    STALL_KERNEL_AUX[cpu].store(aux, Ordering::Release);
    STALL_KERNEL_SITE[cpu].store(site, Ordering::Release);
    if site != 0 {
        STALL_SITE_TENUE[cpu].store(site, Ordering::Release);
    }
}

/// Dernier site NON NUL marque sur ce CPU depuis la prise du verrou.
///
/// `stall_site_courant` seul ne suffit pas a attribuer une tenue : le site est
/// souvent efface avant que le verrou soit relache, et la jauge de tenue
/// maximale rendait alors un zero orphelin. Celui-ci ne repart de zero qu'a
/// l'acquisition suivante.
static STALL_SITE_TENUE: [AtomicU32; MAX_CPUS] = [const { AtomicU32::new(0) }; MAX_CPUS];

/// Site le plus recent marque pendant la tenue en cours.
#[inline]
pub fn stall_site_de_la_tenue() -> u32 {
    STALL_SITE_TENUE[local_cpu()].load(Ordering::Acquire)
}

/// Ouvre une nouvelle tenue : le site attribue repart de zero.
#[inline]
pub fn stall_site_tenue_reset() {
    STALL_SITE_TENUE[local_cpu()].store(0, Ordering::Release);
}

/// Site noyau marque sur CE CPU, pour attribuer une tenue de verrou.
#[inline]
pub fn stall_site_courant() -> u32 {
    STALL_KERNEL_SITE[local_cpu()].load(Ordering::Acquire)
}

#[inline]
pub fn stall_site_clear() {
    STALL_KERNEL_SITE[local_cpu()].store(0, Ordering::Release);
}

/// Marqueur de site pose depuis une interruption, rendu en sortant.
///
/// Une interruption s'execute *dans* le contexte qu'elle interrompt. Ecrire
/// `STALL_KERNEL_SITE` depuis le gestionnaire puis l'effacer detruit donc le
/// marqueur pose par la tache interrompue — et le timer le fait mille fois par
/// seconde. C'est exactement pourquoi `[SMP-STALL] site=0:0x0` n'a jamais rien
/// dit d'un blocage : au moment ou la sonde lit le champ, la derniere IRQ
/// vient de le remettre a zero. C'est aussi pourquoi `max_hold_site` designait
/// 31 (le marqueur de l'IPI) au lieu du code qui tenait reellement le verrou :
/// `stall_site_set` ecrase aussi `STALL_SITE_TENUE`.
///
/// Le gestionnaire sauve donc les trois champs a l'entree et les rend a la
/// sortie. Pendant l'interruption le site decrit l'interruption ; apres, il
/// decrit de nouveau la tache.
pub struct SiteIrq {
    cpu: usize,
    site: u32,
    aux: u64,
    tenue: u32,
}

impl SiteIrq {
    /// Sauve le site du contexte interrompu, puis marque celui de l'IRQ.
    #[inline]
    pub fn enter(site: u32, aux: u64) -> Self {
        let cpu = local_cpu();
        let garde = Self {
            cpu,
            site: STALL_KERNEL_SITE[cpu].load(Ordering::Acquire),
            aux: STALL_KERNEL_AUX[cpu].load(Ordering::Acquire),
            tenue: STALL_SITE_TENUE[cpu].load(Ordering::Acquire),
        };
        stall_site_set(site, aux);
        garde
    }
}

impl Drop for SiteIrq {
    #[inline]
    fn drop(&mut self) {
        STALL_KERNEL_AUX[self.cpu].store(self.aux, Ordering::Release);
        STALL_KERNEL_SITE[self.cpu].store(self.site, Ordering::Release);
        STALL_SITE_TENUE[self.cpu].store(self.tenue, Ordering::Release);
    }
}

#[inline]
fn local_cpu() -> usize {
    usermode::cpu_index().min(MAX_CPUS - 1)
}

#[inline]
fn current_index_raw() -> usize {
    CURRENT[local_cpu()].load(Ordering::Acquire)
}

/// Contexte uniquement atomique, lu par smp_lock au moment exact ou un CPU
/// devient proprietaire. Aucun domaine Process verrouille n'est touche ici.
// BOUCHAUD_DF_FORENSIC_V1
//
// Identite de la tache courante pour un gestionnaire de faute fatale.
//
// Aucun verrou, aucune allocation, aucun `Arc::clone` : ce chemin s'execute
// apres qu'une faute a rendu l'etat du noyau douteux. Prendre un verrou qui se
// trouverait deja tenu par le CPU fautif transformerait un diagnostic en gel,
// et un gel ne se lit pas.
//
// Rend `None` si aucune tache n'est installee sur ce CPU -- ce qui est en soi
// une information : la faute a eu lieu dans la boucle idle ou avant le premier
// ordonnancement.
pub fn identite_pour_faute() -> Option<(usize, u32, u32, u64, u64, bool)> {
    let index = CURRENT[local_cpu()].load(Ordering::Acquire);
    if index == NO_TASK {
        return None;
    }
    // `tasks()` materialise le Vec global. Le lire sans verrou est un pari
    // assume ici et seulement ici : l'alternative est de ne rien pouvoir dire.
    let table = tasks();
    let task = table.get(index)?;
    Some((
        index,
        task.process.pid,
        task.tid,
        task.kstack_top,
        task.kstack_top.saturating_sub(KSTACK_SIZE as u64),
        task.in_kernel.charge(),
    ))
}

/// Une replanification est-elle demandee sur ce CPU ?
pub fn besoin_de_replanifier() -> bool {
    NEED_RESCHED[local_cpu()].load(Ordering::Acquire)
}

/// Nom du processus courant, sans allocation : une tranche empruntee.
pub fn nom_pour_faute() -> &'static str {
    let index = CURRENT[local_cpu()].load(Ordering::Acquire);
    if index == NO_TASK {
        return "<aucune>";
    }
    match tasks().get(index) {
        // `&str` emprunte a la `String` du processus : aucune copie, aucune
        // allocation. La duree de vie est etendue parce que ce chemin ne
        // revient jamais -- la faute est fatale.
        // `resource_group_name` est un champ NU du processus : le lire ne prend
        // aucun verrou. `metadata.name` serait plus precis mais vit derriere un
        // SpinLock, que le CPU fautif peut deja tenir -- l'attendre changerait
        // un diagnostic en gel.
        Some(task) => unsafe {
            &*(task.process.resource_group_name.as_str() as *const str)
        },
        None => "<index invalide>",
    }
}

pub fn stall_probe_local_context() -> (usize, u64, u32, u32, u64) {
    stall_probe_context_pour(local_cpu())
}

/// Meme releve, pour un CPU deja connu : evite un `rdmsr` a l'appelant qui a
/// deja son index sous la main.
pub fn stall_probe_context_pour(cpu: usize) -> (usize, u64, u32, u32, u64) {
    let cpu = cpu.min(MAX_CPUS - 1);
    let kernel = CURRENT_IS_KERNEL[cpu].load(Ordering::Acquire);
    (
        CURRENT[cpu].load(Ordering::Acquire),
        if kernel { STALL_NO_SYSCALL } else { STALL_SYSCALL_NR[cpu].load(Ordering::Acquire) },
        if kernel { 0 } else { STALL_SYSCALL_PHASE[cpu].load(Ordering::Acquire) },
        STALL_KERNEL_SITE[cpu].load(Ordering::Acquire),
        STALL_KERNEL_AUX[cpu].load(Ordering::Acquire),
    )
}

pub fn stall_ipi_observe(rip: u64, interrupted_user: bool) {
    let cpu = local_cpu();
    STALL_IPI_RIP[cpu].store(rip, Ordering::Release);
    STALL_IPI_USER[cpu].store(interrupted_user as u32, Ordering::Release);
    STALL_IPI_TICK[cpu].store(crate::kernel::timer::ticks(), Ordering::Release);
    STALL_IPI_COUNT[cpu].fetch_add(1, Ordering::Relaxed);
}

pub fn stall_ipi_bkl_result(acquired: bool) {
    let cpu = local_cpu();
    if acquired {
        STALL_IPI_BKL_HIT[cpu].fetch_add(1, Ordering::Relaxed);
    } else {
        STALL_IPI_BKL_MISS[cpu].fetch_add(1, Ordering::Relaxed);
    }
}

pub fn stall_pf_begin(addr: u64) {
    let cpu = local_cpu();
    STALL_PF_BEGIN[cpu].fetch_add(1, Ordering::Relaxed);
    stall_site_set(20, addr);
}

pub fn stall_pf_phase(site: u32, aux: u64) {
    stall_site_set(site, aux);
}

pub fn stall_pf_file_begin(source_offset: u64) {
    STALL_PF_FILE_BEGIN[local_cpu()].fetch_add(1, Ordering::Relaxed);
    stall_site_set(242, source_offset);
}

pub fn stall_pf_file_done(got: usize, wanted: usize) {
    STALL_PF_FILE_DONE[local_cpu()].fetch_add(1, Ordering::Relaxed);
    let packed = ((got as u64) << 32) | (wanted as u64 & 0xffff_ffff);
    stall_site_set(243, packed);
}

pub fn stall_pf_done(addr: u64) {
    STALL_PF_DONE[local_cpu()].fetch_add(1, Ordering::Relaxed);
    stall_site_set(299, addr);
}

pub fn stall_pf_fail(addr: u64) {
    STALL_PF_FAIL[local_cpu()].fetch_add(1, Ordering::Relaxed);
    stall_site_set(298, addr);
}


// --- Sonde de stall SMP : aucun verrou Process, uniquement atomiques. ---
pub fn stall_syscall_enter(nr: u64) {
    let cpu = local_cpu();
    STALL_SYSCALL_NR[cpu].store(nr, Ordering::Release);
    STALL_SYSCALL_TICK[cpu].store(crate::kernel::timer::ticks(), Ordering::Release);
    STALL_SYSCALL_PHASE[cpu].store(1, Ordering::Release);
}

pub fn stall_syscall_bkl_acquired() {
    STALL_SYSCALL_PHASE[local_cpu()].store(2, Ordering::Release);
}

/// Ou en est la boucle de disponibilite de `poll`/`select`, par CPU.
///
/// # Pourquoi c'est necessaire
///
/// La sonde de blocage ne disait que `syscall=7 phase=2` et `site=0`. Sur le
/// gel observe au 25 aout, cela suffisait a savoir QUE le CPU etait dans
/// `poll` avec le verrou, et pas du tout OU : le balayage des descripteurs,
/// l'emission d'un accuse, l'attente, ou la reprise. Deux corrections de suite
/// ont ete guidees a l'aveugle par ce manque.
///
/// Une case par CPU, un `store` relaxe par transition. Rien n'est journalise
/// en continu -- c'est la sonde de blocage qui LIT cette case quand une tenue
/// depasse son seuil, donc au plus une ligne par seconde et seulement quand
/// quelque chose ne va pas.
pub const POLL_HORS: u32 = 0;
pub const POLL_ENTREE: u32 = 1;
pub const POLL_BALAYAGE: u32 = 2;
pub const POLL_PRET: u32 = 3;
pub const POLL_ATTENTE: u32 = 4;
pub const POLL_REVEIL: u32 = 5;
pub const POLL_RETOUR: u32 = 6;

// BOUCHAUD_P2_VM_PHASE_V1
//
// Les phases d'un `madvise`/`munmap`, pour que « quelle phase tenait le
// verrou » ait une reponse au lieu d'un zero.
//
// Les quatre etapes n'ont pas du tout le meme profil : la validation est
// courte et sous le verrou du `Mm`, la preparation parcourt la plage, le
// shootdown TLB RELACHE le gros verrou et attend des CPU distants, et la
// finition rend les frames. Une tenue de plusieurs secondes vient forcement
// de l'une des trois qui restent sous le verrou, et il faut savoir laquelle.
pub const VM_HORS: u32 = 0;
/// Validation de la plage contre les VMA, sous le verrou du `Mm`.
pub const VM_VALIDATION: u32 = 20;
/// Retrait des PTE et collecte des frames a rendre.
pub const VM_PREPARATION: u32 = 21;
/// Invalidation TLB distante. Le gros verrou est SUSPENDU pendant cette phase.
pub const VM_SHOOTDOWN: u32 = 22;
/// Retour des frames a l'allocateur, gros verrou repris.
pub const VM_FINITION: u32 = 23;
/// Retour des references du cache de pages propres.
pub const VM_PAGES_PROPRES: u32 = 24;

/// Marque la phase VM courante. Meme canal que `poll_phase_set` : c'est ce que
/// la sonde de blocage et la provenance du maximum de tenue relisent.
#[inline]
pub fn vm_phase_set(phase: u32, detail: u64) {
    let cpu = local_cpu();
    STALL_KERNEL_AUX[cpu].store(detail, Ordering::Release);
    STALL_SYSCALL_PHASE[cpu].store(phase, Ordering::Release);
}

static POLL_PHASE: [AtomicU32; MAX_CPUS] = [const { AtomicU32::new(0) }; MAX_CPUS];
/// Detail de la phase : le descripteur en cours de sondage, ou le tour de
/// boucle. Sans lui, « balayage » ne dit pas QUEL descripteur bloque.
static POLL_DETAIL: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];

#[inline]
pub fn poll_phase_set(phase: u32, detail: u64) {
    let cpu = local_cpu();
    POLL_DETAIL[cpu].store(detail, Ordering::Release);
    POLL_PHASE[cpu].store(phase, Ordering::Release);
}

/// Etapes d'UN tour de balayage, encodees dans le detail.
///
/// « balayage detail=0 » ne distinguait pas deux instructions differentes : le
/// tout premier descripteur avant meme sa lecture, et le descripteur 0 une
/// fois lu. Le gel du 25 aout a rendu exactement cette valeur pendant cent
/// sept secondes, donc sans dire laquelle des six etapes tenait le CPU.
pub const ETAPE_LIT_FD: u32 = 1;
pub const ETAPE_LIT_EVENTS: u32 = 2;
pub const ETAPE_LISIBLE: u32 = 3;
pub const ETAPE_INSCRIPTIBLE: u32 = 4;
pub const ETAPE_ETAT_PAIR: u32 = 5;
pub const ETAPE_REND_REVENTS: u32 = 6;

/// Descripteur pas encore connu a cette etape.
pub const FD_INCONNU: u32 = u32::MAX;

/// `etape | index | descripteur` dans un seul mot, sans collision possible.
#[inline]
pub fn poll_detail(etape: u32, index: usize, fd: u32) -> u64 {
    ((etape as u64) << 48) | (((index as u64) & 0xffff) << 32) | fd as u64
}

/// Inverse de [`poll_detail`], pour la sonde.
#[inline]
pub fn poll_detail_decode(detail: u64) -> (u32, u64, u32) {
    (
        (detail >> 48) as u32,
        (detail >> 32) & 0xffff,
        (detail & 0xffff_ffff) as u32,
    )
}

#[inline]
pub fn poll_phase(cpu: usize) -> (u32, u64) {
    let cpu = cpu.min(MAX_CPUS - 1);
    (
        POLL_PHASE[cpu].load(Ordering::Acquire),
        POLL_DETAIL[cpu].load(Ordering::Acquire),
    )
}

/// Phase 3 : dans le corps de l'appel, SANS le gros verrou.
///
/// La phase 2 veut dire « le verrou est pris ». Un appel libere n'y passe plus
/// jamais ; lui faire dire 2 rendrait la sonde de blocage menteuse au moment
/// precis ou l'on s'en sert pour savoir qui detient quoi.
pub fn stall_syscall_sans_verrou() {
    STALL_SYSCALL_PHASE[local_cpu()].store(3, Ordering::Release);
}

pub fn stall_syscall_exit() {
    let cpu = local_cpu();
    STALL_SYSCALL_PHASE[cpu].store(0, Ordering::Release);
    STALL_SYSCALL_NR[cpu].store(STALL_NO_SYSCALL, Ordering::Release);
}

/// Etat syscall d'un CPU, rendu lisible sans rien allouer.
///
/// Ce releve sort du gestionnaire du PIT, avant toute tentative de verrou :
/// construire une `String` ici ferait entrer l'allocateur dans le seul chemin
/// cense survivre a un noyau bloque. D'ou l'adaptateur `Display`.
struct EtatSyscall {
    nr: u64,
    phase: u32,
    age_ticks: u64,
}

impl core::fmt::Display for EtatSyscall {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // `18446744073709551615` etait la sentinelle « aucun appel en cours ».
        // Elle se lisait comme une valeur, pas comme une absence.
        if self.nr == STALL_NO_SYSCALL {
            return f.write_str("none");
        }
        let phase = match self.phase {
            0 => "sortie",
            1 => "attente-verrou",
            2 => "verrou-tenu",
            3 => "sans-verrou",
            _ => "?",
        };
        write!(
            f,
            "{}({})/{}/{}t",
            crate::kernel::abi::nr::name(self.nr),
            self.nr,
            phase,
            self.age_ticks,
        )
    }
}

/// Rend `none` plutot qu'une sentinelle.
///
/// `18446744073709551615` et `4294967295` se lisent comme des valeurs alors
/// qu'ils veulent dire « pas encore observe ». Un journal qui les affiche
/// oblige a savoir de tete lesquels sont des absences.
struct Absent(u64);

impl core::fmt::Display for Absent {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.0 == u64::MAX || self.0 == u32::MAX as u64 {
            return f.write_str("none");
        }
        write!(f, "{}", self.0)
    }
}

/// Numero d'acquisition BKL vu au releve precedent.
///
/// Seul le PIT du BSP ecrit ici, une fois par seconde : pas de course.
static STALL_DERNIER_ACQUIRE_SEQ: AtomicU64 = AtomicU64::new(u64::MAX);

/// Appelee par le PIT BSP AVANT tout try_enter(BKL). Si les logs normaux
/// meurent parce qu'un AP garde le BKL, cette ligne continue donc a sortir.
pub fn stall_probe_from_timer() {
    let now = crate::kernel::timer::ticks();
    if now == 0 || now % crate::kernel::timer::TICKS_PER_SECOND != 0 {
        return;
    }

    // Syscall state is CPU-local. Kernel threads must not inherit the previous
    // user task's syscall label in SMP/BKL diagnostics.
    let k0 = CURRENT_IS_KERNEL[0].load(Ordering::Acquire);
    let k1 = CURRENT_IS_KERNEL[1].load(Ordering::Acquire);
    let k2 = CURRENT_IS_KERNEL[2].load(Ordering::Acquire);
    let k3 = CURRENT_IS_KERNEL[3].load(Ordering::Acquire);
    let nr0 = if k0 { STALL_NO_SYSCALL } else { STALL_SYSCALL_NR[0].load(Ordering::Acquire) };
    let nr1 = if k1 { STALL_NO_SYSCALL } else { STALL_SYSCALL_NR[1].load(Ordering::Acquire) };
    let nr2 = if k2 { STALL_NO_SYSCALL } else { STALL_SYSCALL_NR[2].load(Ordering::Acquire) };
    let nr3 = if k3 { STALL_NO_SYSCALL } else { STALL_SYSCALL_NR[3].load(Ordering::Acquire) };
    let ph0 = if k0 { 0 } else { STALL_SYSCALL_PHASE[0].load(Ordering::Acquire) };
    let ph1 = if k1 { 0 } else { STALL_SYSCALL_PHASE[1].load(Ordering::Acquire) };
    let ph2 = if k2 { 0 } else { STALL_SYSCALL_PHASE[2].load(Ordering::Acquire) };
    let ph3 = if k3 { 0 } else { STALL_SYSCALL_PHASE[3].load(Ordering::Acquire) };
    let st0 = STALL_SYSCALL_TICK[0].load(Ordering::Acquire);
    let st1 = STALL_SYSCALL_TICK[1].load(Ordering::Acquire);
    let st2 = STALL_SYSCALL_TICK[2].load(Ordering::Acquire);
    let st3 = STALL_SYSCALL_TICK[3].load(Ordering::Acquire);
    let age0 = if ph0 == 0 { 0 } else { now.wrapping_sub(st0) };
    let age1 = if ph1 == 0 { 0 } else { now.wrapping_sub(st1) };
    let age2 = if ph2 == 0 { 0 } else { now.wrapping_sub(st2) };
    let age3 = if ph3 == 0 { 0 } else { now.wrapping_sub(st3) };
    let site0 = STALL_KERNEL_SITE[0].load(Ordering::Acquire);
    let site1 = STALL_KERNEL_SITE[1].load(Ordering::Acquire);
    let site2 = STALL_KERNEL_SITE[2].load(Ordering::Acquire);
    let site3 = STALL_KERNEL_SITE[3].load(Ordering::Acquire);
    let aux0 = STALL_KERNEL_AUX[0].load(Ordering::Acquire);
    let aux1 = STALL_KERNEL_AUX[1].load(Ordering::Acquire);
    let aux2 = STALL_KERNEL_AUX[2].load(Ordering::Acquire);
    let aux3 = STALL_KERNEL_AUX[3].load(Ordering::Acquire);

    // Un releve periodique n'est pas un blocage. La ligne s'appelait
    // `[SMP-STALL]` a chaque seconde, y compris avec `owner=0
    // depth=[0,0,0,0]` -- c'est-a-dire avec un verrou libre et personne
    // dedans. Une alarme qui sonne en permanence ne se lit plus.
    //
    // Le verdict se prend sur une donnee, pas sur la periodicite : si le
    // numero d'acquisition n'a PAS bouge depuis le releve precedent, personne
    // n'a pris le verrou pendant toute cette seconde. Joint a un proprietaire
    // non nul, cela veut dire qu'une seule et meme tenue dure depuis au moins
    // une seconde. C'est un blocage, et rien d'autre ne l'est.
    let owner = crate::kernel::smp_lock::stall_probe_owner_token();
    let acquire_seq = crate::kernel::smp_lock::stall_probe_acquire_seq();
    let precedent = STALL_DERNIER_ACQUIRE_SEQ.swap(acquire_seq, Ordering::AcqRel);
    let bloque = owner != 0 && precedent == acquire_seq;

    // V14: the probe still executes every second so a continuous BKL hold is
    // detected with the same latency, but healthy snapshots are printed only
    // every five seconds. Serial I/O is extremely expensive under TCG.
    let snapshot_period = 5 * crate::kernel::timer::TICKS_PER_SECOND;
    if !bloque && now % snapshot_period != 0 {
        return;
    }

    crate::serial_println!(
        "[{}] t={} owner={} depth=[{},{},{},{}] cur=[{},{},{},{}] site=[{}:{:#x} {}:{:#x} {}:{:#x} {}:{:#x}] syscall=[{} {} {} {}]",
        if bloque { "SMP-STALL" } else { "SMP-SNAPSHOT" },
        now,
        owner,
        crate::kernel::smp_lock::stall_probe_depth(0),
        crate::kernel::smp_lock::stall_probe_depth(1),
        crate::kernel::smp_lock::stall_probe_depth(2),
        crate::kernel::smp_lock::stall_probe_depth(3),
        CURRENT[0].load(Ordering::Acquire),
        CURRENT[1].load(Ordering::Acquire),
        CURRENT[2].load(Ordering::Acquire),
        CURRENT[3].load(Ordering::Acquire),
        site0, aux0, site1, aux1, site2, aux2, site3, aux3,
        EtatSyscall { nr: nr0, phase: ph0, age_ticks: age0 },
        EtatSyscall { nr: nr1, phase: ph1, age_ticks: age1 },
        EtatSyscall { nr: nr2, phase: ph2, age_ticks: age2 },
        EtatSyscall { nr: nr3, phase: ph3, age_ticks: age3 },
    );

    // Un CPU qui tourne sur un verrou tournant ne laisse aucune autre trace :
    // pas d'acquisition BKL, pas de faute, pas de changement de tache. Cette
    // ligne est la seule qui distingue un noyau bloque d'un noyau occupe, et
    // elle ne sort que si un CPU attend depuis assez longtemps pour que ce soit
    // anormal.
    for cpu in 0..4usize {
        let Some(attente) = crate::kernel::sync::attente_verrou(cpu) else {
            continue;
        };
        let genre = if attente.etat == crate::kernel::sync::ATTENTE_REENTRANTE {
            "reentrant"
        } else {
            "contendu"
        };
        crate::serial_println!(
            "[SMP-SPIN] cpu={} genre={} verrou={:#x} proprio={} depuis={}ms site={}:{}",
            cpu,
            genre,
            attente.verrou,
            attente.proprietaire as i64,
            now.wrapping_sub(attente.depuis),
            attente.fichier,
            attente.ligne,
        );
    }

    let prov = crate::kernel::smp_lock::stall_probe_provenance();
    let owner_cpu = if prov.owner_token == 0 { 255usize } else { prov.owner_token - 1 };
    let held = if prov.owner_token == 0 || prov.generation == 0 {
        0
    } else {
        now.wrapping_sub(prov.since_tick)
    };

    // Ou en est la boucle de disponibilite du proprietaire, s'il y en a un qui
    // dure. Rien n'est imprime en marche normale : la ligne ne sort que
    // lorsqu'une tenue depasse le demi-seconde, c'est-a-dire exactement quand
    // `syscall=7 phase=2 site=0` ne suffit plus a dire ou l'on est bloque.
    if owner_cpu < MAX_CPUS && held >= 500 {
        // Poll state is CPU-local and belongs to user tasks. A kernel thread on
        // the same CPU must not inherit the previous user's poll phase.
        let (phase, detail) = if CURRENT_IS_KERNEL[owner_cpu].load(Ordering::Acquire) {
            (POLL_HORS, 0)
        } else {
            poll_phase(owner_cpu)
        };
        let nom = match phase {
            POLL_ENTREE => "entree",
            POLL_BALAYAGE => "balayage",
            POLL_PRET => "pret",
            POLL_ATTENTE => "attente",
            POLL_REVEIL => "reveil",
            POLL_RETOUR => "retour",
            _ => "hors-poll",
        };
        if phase == POLL_BALAYAGE {
            let (etape, index, fd) = poll_detail_decode(detail);
            let quoi = match etape {
                ETAPE_LIT_FD => "lit-fd",
                ETAPE_LIT_EVENTS => "lit-events",
                ETAPE_LISIBLE => "lisible",
                ETAPE_INSCRIPTIBLE => "inscriptible",
                ETAPE_ETAT_PAIR => "etat-pair",
                ETAPE_REND_REVENTS => "rend-revents",
                _ => "?",
            };
            crate::serial_println!(
                "[SMP-POLL] cpu={} tenue={}ms phase=balayage etape={} index={} fd={} tx_plein={}",
                owner_cpu,
                held,
                quoi,
                index,
                fd as i32,
                crate::drivers::e1000::tx_anneau_plein(),
            );
        } else {
            crate::serial_println!(
                "[SMP-POLL] cpu={} tenue={}ms phase={} detail={:#x} tx_plein={}",
                owner_cpu,
                held,
                nom,
                detail,
                crate::drivers::e1000::tx_anneau_plein(),
            );
        }
    }
    let (live_site, live_aux, live_depth) = if owner_cpu < MAX_CPUS {
        (
            STALL_KERNEL_SITE[owner_cpu].load(Ordering::Acquire),
            STALL_KERNEL_AUX[owner_cpu].load(Ordering::Acquire),
            crate::kernel::smp_lock::stall_probe_depth(owner_cpu),
        )
    } else {
        (0, 0, 0)
    };
    let last_rel_age = if prov.last_release_tick == 0 {
        0
    } else {
        now.wrapping_sub(prov.last_release_tick)
    };
    crate::serial_println!(
        "[SMP-PROV] t={} owner={} cpu={} gen={} coherent={} held={}ms depth={} acq={} rel={} reent={} kind={} task={} syscall={}:{} acquired_site={}:{:#x} live_site={}:{:#x} lastrel={}@cpu{}:kind{} gen={} age={}ms",
        now, prov.owner_token, owner_cpu, prov.generation, prov.coherent as u8,
        held, live_depth, prov.acquire_seq, prov.release_seq, prov.reenter_seq,
        prov.acquire_kind, prov.task, prov.syscall_nr, prov.syscall_phase,
        prov.site, prov.aux, live_site, live_aux, prov.last_release_tick,
        prov.last_release_cpu, prov.last_release_kind, prov.last_release_gen, last_rel_age,
    );

    let ipi_age = |cpu: usize| {
        let count = STALL_IPI_COUNT[cpu].load(Ordering::Acquire);
        let tick = STALL_IPI_TICK[cpu].load(Ordering::Acquire);
        if count == 0 { 0 } else { now.wrapping_sub(tick) }
    };
    crate::serial_println!(
        "[SMP-IPI] t={} c0={}/{}ms/{:#x}/u{}/{}/{} c1={}/{}ms/{:#x}/u{}/{}/{} c2={}/{}ms/{:#x}/u{}/{}/{} c3={}/{}ms/{:#x}/u{}/{}/{}",
        now,
        STALL_IPI_COUNT[0].load(Ordering::Acquire), ipi_age(0), STALL_IPI_RIP[0].load(Ordering::Acquire), STALL_IPI_USER[0].load(Ordering::Acquire), STALL_IPI_BKL_HIT[0].load(Ordering::Acquire), STALL_IPI_BKL_MISS[0].load(Ordering::Acquire),
        STALL_IPI_COUNT[1].load(Ordering::Acquire), ipi_age(1), STALL_IPI_RIP[1].load(Ordering::Acquire), STALL_IPI_USER[1].load(Ordering::Acquire), STALL_IPI_BKL_HIT[1].load(Ordering::Acquire), STALL_IPI_BKL_MISS[1].load(Ordering::Acquire),
        STALL_IPI_COUNT[2].load(Ordering::Acquire), ipi_age(2), STALL_IPI_RIP[2].load(Ordering::Acquire), STALL_IPI_USER[2].load(Ordering::Acquire), STALL_IPI_BKL_HIT[2].load(Ordering::Acquire), STALL_IPI_BKL_MISS[2].load(Ordering::Acquire),
        STALL_IPI_COUNT[3].load(Ordering::Acquire), ipi_age(3), STALL_IPI_RIP[3].load(Ordering::Acquire), STALL_IPI_USER[3].load(Ordering::Acquire), STALL_IPI_BKL_HIT[3].load(Ordering::Acquire), STALL_IPI_BKL_MISS[3].load(Ordering::Acquire),
    );

    crate::serial_println!(
        "[SMP-PF] t={} c0={}/{}/{}/{}/{} c1={}/{}/{}/{}/{} c2={}/{}/{}/{}/{} c3={}/{}/{}/{}/{}",
        now,
        STALL_PF_BEGIN[0].load(Ordering::Acquire), STALL_PF_DONE[0].load(Ordering::Acquire), STALL_PF_FAIL[0].load(Ordering::Acquire), STALL_PF_FILE_BEGIN[0].load(Ordering::Acquire), STALL_PF_FILE_DONE[0].load(Ordering::Acquire),
        STALL_PF_BEGIN[1].load(Ordering::Acquire), STALL_PF_DONE[1].load(Ordering::Acquire), STALL_PF_FAIL[1].load(Ordering::Acquire), STALL_PF_FILE_BEGIN[1].load(Ordering::Acquire), STALL_PF_FILE_DONE[1].load(Ordering::Acquire),
        STALL_PF_BEGIN[2].load(Ordering::Acquire), STALL_PF_DONE[2].load(Ordering::Acquire), STALL_PF_FAIL[2].load(Ordering::Acquire), STALL_PF_FILE_BEGIN[2].load(Ordering::Acquire), STALL_PF_FILE_DONE[2].load(Ordering::Acquire),
        STALL_PF_BEGIN[3].load(Ordering::Acquire), STALL_PF_DONE[3].load(Ordering::Acquire), STALL_PF_FAIL[3].load(Ordering::Acquire), STALL_PF_FILE_BEGIN[3].load(Ordering::Acquire), STALL_PF_FILE_DONE[3].load(Ordering::Acquire),
    );
}

