// BOUCHAUD_P15_BROWSER_HANG_FORENSICS
//
// Fiche forensique par tache : ce que faisait chaque fil au moment ou l'on
// vient le chercher.
//
// # RECONSTRUCTION
//
// Le commit `b16a6fd6` (p18) incluait ce fragment sans l'avoir jamais pousse :
// aucune reference, aucun commit du depot ne le contient. Le HEAD ne compilait
// plus. Ce fichier le RECONSTITUE a partir des contrats exacts de ses sites
// d'appel -- `blocage.rs`, `sommeil.rs`, `futex.rs`, `registre.rs`,
// `diagnostic_stall.rs`, `usermode.rs` et la reponse BRDP `forensics status`.
// Si l'original existe encore sur le poste qui a produit p18, il doit
// remplacer celui-ci : l'API est la meme, les choix internes peuvent differer.
//
// # Pourquoi une fiche par EMPLACEMENT, et pas par CPU
//
// La sonde de blocage existante (`STALL_*`) est tenue par CPU : elle dit ce
// que fait un coeur. Un navigateur fige pose l'autre question -- ce que fait
// CHAQUE fil, y compris ceux qui ne tournent sur aucun coeur parce qu'ils
// attendent. Un WebContent bloque dans un futex depuis douze secondes n'occupe
// aucun CPU ; une sonde par coeur ne le voit pas.
//
// # Ce que coute la fiche, et pourquoi c'est borne
//
// Elle est ecrite sur les chemins les plus chauds du noyau : entree et sortie
// de chaque syscall, chaque faute de page, chaque tour de `poll`. D'ou trois
// regles :
//
//   - AUCUN VERROU, AUCUNE ALLOCATION. Des `store` relaxes sur des atomiques
//     statiques, et rien d'autre ;
//   - UN SEUL ECRIVAIN PAR FICHE. Seule la tache installee dans l'emplacement
//     l'ecrit, depuis le coeur qui l'execute -- une tache ne tourne jamais sur
//     deux coeurs a la fois. Aucune operation lecture-modification-ecriture
//     n'est donc necessaire, et aucune ligne de cache n'est disputee ;
//   - L'HEURE EST LUE EN TICKS, PAS EN NANOSECONDES. `monotonic_ns()` termine
//     par un `fetch_max` sur un atomique GLOBAL : l'appeler a chaque syscall
//     ferait voyager la meme ligne de cache entre les seize coeurs de la
//     machine de reference, sur le chemin le plus frequente du systeme.
//     `ticks()` est une simple lecture. Sa resolution -- la milliseconde -- est
//     exactement celle des ages rendus par `forensics status`.
//
// Les fiches sont alignees sur une ligne de cache : deux taches d'emplacements
// voisins, en syscall au meme instant sur deux coeurs, ne se volent pas leur
// ligne a chaque ecriture.
//
// # Ce que la fiche ne pretend pas
//
// Une photographie lue a froid, sans arreter l'ecrivain : deux champs d'une
// meme fiche peuvent appartenir a deux instants voisins. C'est le prix d'un
// releve qui ne bloque personne, et il est juste pour son usage -- trouver le
// fil qui ne bouge plus depuis des secondes, pas reconstituer une nanoseconde.

/// Aucun syscall en cours dans cette fiche.
///
/// La meme valeur que `STALL_NO_SYSCALL` : aucun numero reel ne l'atteint.
pub const FORENSIC_NO_SYSCALL: u64 = u64::MAX;

/// Aucune attente en cours.
const WAIT_NONE: u8 = 0;
/// Garee sur une `WaitQueue` (parking detache ou non, avec ou sans echeance).
const WAIT_WAIT_QUEUE: u8 = 1;
/// `sleep_ticks` : une echeance, et rien d'autre a attendre.
const WAIT_SLEEP: u8 = 2;
/// `futex_wait` : un mot memoire, et peut-etre une echeance.
const WAIT_FUTEX: u8 = 3;

/// Duree d'un tick. Les estampilles de la fiche sont en ticks : voir l'en-tete.
const NS_PAR_TICK_FORENSIQUE: u64 = 1_000_000_000 / crate::kernel::timer::TICKS_PER_SECOND;

/// Au-dela, une attente est comptee dans `waits_over_1s`.
const ATTENTE_LONGUE_NS: u64 = 1_000_000_000;

#[repr(align(64))]
struct FicheForensique {
    // -- l'attente en cours ----------------------------------------------
    wait_kind: core::sync::atomic::AtomicU8,
    wait_key: AtomicU64,
    wait_aux: AtomicU64,
    wait_tick: AtomicU64,
    // -- le syscall en cours ---------------------------------------------
    syscall_nr: AtomicU64,
    syscall_tick: AtomicU64,
    arg0: AtomicU64,
    arg1: AtomicU64,
    arg2: AtomicU64,
    poll_phase: AtomicU32,
    poll_detail: AtomicU64,
    // -- le dernier syscall termine --------------------------------------
    last_syscall_nr: AtomicU64,
    last_syscall_result: core::sync::atomic::AtomicI64,
    last_syscall_duree_ns: AtomicU64,
    // -- les fautes ------------------------------------------------------
    fault_count: AtomicU64,
    last_fault_addr: AtomicU64,
    last_fault_tick: AtomicU64,
    // -- le dernier signe de vie, et le dernier contexte ring 3 -----------
    activity_tick: AtomicU64,
    user_rip: AtomicU64,
    user_rsp: AtomicU64,
    user_rbp: AtomicU64,
}

impl FicheForensique {
    const fn vide() -> Self {
        Self {
            wait_kind: core::sync::atomic::AtomicU8::new(WAIT_NONE),
            wait_key: AtomicU64::new(0),
            wait_aux: AtomicU64::new(0),
            wait_tick: AtomicU64::new(0),
            syscall_nr: AtomicU64::new(FORENSIC_NO_SYSCALL),
            syscall_tick: AtomicU64::new(0),
            arg0: AtomicU64::new(0),
            arg1: AtomicU64::new(0),
            arg2: AtomicU64::new(0),
            poll_phase: AtomicU32::new(POLL_HORS),
            poll_detail: AtomicU64::new(0),
            last_syscall_nr: AtomicU64::new(FORENSIC_NO_SYSCALL),
            last_syscall_result: core::sync::atomic::AtomicI64::new(0),
            last_syscall_duree_ns: AtomicU64::new(0),
            fault_count: AtomicU64::new(0),
            last_fault_addr: AtomicU64::new(0),
            last_fault_tick: AtomicU64::new(0),
            activity_tick: AtomicU64::new(0),
            user_rip: AtomicU64::new(0),
            user_rsp: AtomicU64::new(0),
            user_rbp: AtomicU64::new(0),
        }
    }

    /// Remet la fiche dans l'etat d'un emplacement jamais servi.
    ///
    /// Sans cela, une tache recyclee heriterait de l'attente, du syscall et
    /// des fautes de l'incarnation precedente -- et le releve accuserait un
    /// fil neuf d'etre fige depuis la mort de son predecesseur.
    fn remet_a_zero(&self) {
        self.wait_kind.store(WAIT_NONE, Ordering::Relaxed);
        self.wait_key.store(0, Ordering::Relaxed);
        self.wait_aux.store(0, Ordering::Relaxed);
        self.wait_tick.store(0, Ordering::Relaxed);
        self.syscall_nr.store(FORENSIC_NO_SYSCALL, Ordering::Relaxed);
        self.syscall_tick.store(0, Ordering::Relaxed);
        self.arg0.store(0, Ordering::Relaxed);
        self.arg1.store(0, Ordering::Relaxed);
        self.arg2.store(0, Ordering::Relaxed);
        self.poll_phase.store(POLL_HORS, Ordering::Relaxed);
        self.poll_detail.store(0, Ordering::Relaxed);
        self.last_syscall_nr.store(FORENSIC_NO_SYSCALL, Ordering::Relaxed);
        self.last_syscall_result.store(0, Ordering::Relaxed);
        self.last_syscall_duree_ns.store(0, Ordering::Relaxed);
        self.fault_count.store(0, Ordering::Relaxed);
        self.last_fault_addr.store(0, Ordering::Relaxed);
        self.last_fault_tick.store(0, Ordering::Relaxed);
        self.activity_tick.store(0, Ordering::Relaxed);
        self.user_rip.store(0, Ordering::Relaxed);
        self.user_rsp.store(0, Ordering::Relaxed);
        // Le dernier `Release` publie la remise a zero entiere.
        self.user_rbp.store(0, Ordering::Release);
    }
}

static FICHES_FORENSIQUES: [FicheForensique; MAX_TACHES] =
    [const { FicheForensique::vide() }; MAX_TACHES];

/// La fiche de la tache installee sur ce coeur, s'il y en a une.
///
/// `None` dans la boucle idle et avant le premier ordonnancement : une faute
/// ou un `poll` sans tache n'a pas de fiche a qui etre imputes.
#[inline]
fn fiche_forensique_courante() -> Option<&'static FicheForensique> {
    let index = CURRENT[local_cpu()].load(Ordering::Relaxed);
    // `NO_TASK` vaut `usize::MAX` : le meme test couvre l'absence de tache et
    // un indice hors registre.
    FICHES_FORENSIQUES.get(index)
}

/// Age en nanosecondes d'une estampille en ticks. Zero veut dire « jamais ».
#[inline]
fn age_forensique_ns(estampille_tick: u64, maintenant_tick: u64) -> u64 {
    if estampille_tick == 0 {
        return 0;
    }
    maintenant_tick
        .saturating_sub(estampille_tick)
        .saturating_mul(NS_PAR_TICK_FORENSIQUE)
}

// ---------------------------------------------------------------------------
// LES ECRIVAINS : appeles par la tache elle-meme, sur son propre coeur
// ---------------------------------------------------------------------------

/// Un emplacement du registre change d'incarnation.
///
/// Appele sous l'exclusion d'ecriture du registre, AVANT que la nouvelle
/// tache ne soit publiee : personne ne peut encore l'executer.
fn forensic_reset_slot(slot: usize) {
    if let Some(fiche) = FICHES_FORENSIQUES.get(slot) {
        fiche.remet_a_zero();
    }
}

/// Ouvre une attente -- sauf si une attente est deja ouverte.
///
/// # Pourquoi « si inactive »
///
/// Les attentes s'emboitent : `futex_wait` descend dans `wait_word_wait`, qui
/// peut garer la tache sur une `WaitQueue`. C'est l'attente EXTERNE qui
/// explique le blocage -- « ce fil attend un futex depuis douze secondes » --
/// et son heure de debut est la bonne. L'attente interne ne l'ecrase donc pas.
fn forensic_wait_begin_if_idle(kind: u8, key: u64, aux: u64) {
    let Some(fiche) = fiche_forensique_courante() else { return };
    if fiche.wait_kind.load(Ordering::Relaxed) != WAIT_NONE {
        return;
    }
    let maintenant = crate::kernel::timer::ticks();
    fiche.wait_key.store(key, Ordering::Relaxed);
    fiche.wait_aux.store(aux, Ordering::Relaxed);
    fiche.wait_tick.store(maintenant, Ordering::Relaxed);
    fiche.activity_tick.store(maintenant, Ordering::Relaxed);
    // Le genre en DERNIER, en `Release` : un lecteur qui le voit voit aussi la
    // cle et l'heure de CETTE attente, pas celles de la precedente.
    fiche.wait_kind.store(kind, Ordering::Release);
}

/// Ferme l'attente de ce genre, et seulement celle-la.
///
/// Symetrique de [`forensic_wait_begin_if_idle`] : la sortie d'une attente
/// interne ne ferme pas l'attente externe qui l'englobe.
fn forensic_wait_clear(kind: u8) {
    let Some(fiche) = fiche_forensique_courante() else { return };
    if fiche.wait_kind.load(Ordering::Relaxed) != kind {
        return;
    }
    fiche.wait_kind.store(WAIT_NONE, Ordering::Release);
    fiche
        .activity_tick
        .store(crate::kernel::timer::ticks(), Ordering::Relaxed);
}

/// Entree dans un syscall. Appele a CHAQUE syscall : quatre `store`.
fn forensic_syscall_enter(nr: u64) {
    let Some(fiche) = fiche_forensique_courante() else { return };
    let maintenant = crate::kernel::timer::ticks();
    fiche.syscall_tick.store(maintenant, Ordering::Relaxed);
    fiche.activity_tick.store(maintenant, Ordering::Relaxed);
    // Un syscall neuf commence hors de toute boucle `poll`.
    fiche.poll_phase.store(POLL_HORS, Ordering::Relaxed);
    fiche.syscall_nr.store(nr, Ordering::Release);
}

/// Sortie du syscall en cours.
///
/// `execve` reussi et le retrait d'un zombie ne repassent pas toujours par
/// ici. La fiche reste alors marquee du dernier appel jusqu'au prochain
/// syscall de la tache, qui l'ecrase -- ou jusqu'au recyclage de l'emplacement.
fn forensic_syscall_exit() {
    let Some(fiche) = fiche_forensique_courante() else { return };
    fiche.syscall_nr.store(FORENSIC_NO_SYSCALL, Ordering::Release);
    fiche.poll_phase.store(POLL_HORS, Ordering::Relaxed);
    fiche
        .activity_tick
        .store(crate::kernel::timer::ticks(), Ordering::Relaxed);
}

/// Ce qu'a rendu le syscall qui s'acheve, et ce qu'il a coute.
///
/// La duree est celle que le chemin de syscall a deja mesuree : aucune
/// seconde lecture d'horloge.
pub fn forensic_syscall_result(nr: u64, result: i64, duree_ns: u64) {
    let Some(fiche) = fiche_forensique_courante() else { return };
    fiche.last_syscall_nr.store(nr, Ordering::Relaxed);
    fiche.last_syscall_result.store(result, Ordering::Relaxed);
    fiche.last_syscall_duree_ns.store(duree_ns, Ordering::Relaxed);
    fiche
        .activity_tick
        .store(crate::kernel::timer::ticks(), Ordering::Relaxed);
}

/// Le contexte ring 3 a l'entree du syscall.
///
/// `rip` est l'adresse de RETOUR en espace utilisateur : c'est elle qui permet
/// de retrouver l'appelant dans la table de symboles de Ladybird. Les trois
/// premiers arguments suffisent a lire la plupart des attentes -- descripteur,
/// adresse de futex, delai.
pub fn forensic_user_frame(frame: &TrapFrame) {
    let Some(fiche) = fiche_forensique_courante() else { return };
    fiche.user_rip.store(frame.rip, Ordering::Relaxed);
    fiche.user_rsp.store(frame.rsp, Ordering::Relaxed);
    fiche.user_rbp.store(frame.rbp, Ordering::Relaxed);
    fiche.arg0.store(frame.rdi, Ordering::Relaxed);
    fiche.arg1.store(frame.rsi, Ordering::Relaxed);
    fiche.arg2.store(frame.rdx, Ordering::Relaxed);
}

/// La phase de `poll`/`select`, par tache.
///
/// La sonde par coeur (`POLL_PHASE`) la porte deja ; la fiche la garde ATTACHEE
/// AU FIL, ce qui la conserve quand le fil quitte son coeur pour dormir.
fn forensic_poll_phase_set(phase: u32, detail: u64) {
    let Some(fiche) = fiche_forensique_courante() else { return };
    fiche.poll_detail.store(detail, Ordering::Relaxed);
    fiche.poll_phase.store(phase, Ordering::Release);
}

/// Une faute de page de la tache courante.
///
/// Le compteur de la fiche n'a qu'un ecrivain : charge puis range, sans
/// operation atomique de lecture-modification-ecriture. Le total global des
/// fautes n'est PAS recompte ici -- `STALL_PF_BEGIN` le tient deja par coeur,
/// et une seconde source de verite divergerait tot ou tard.
fn forensic_fault(addr: u64) {
    let Some(fiche) = fiche_forensique_courante() else { return };
    let maintenant = crate::kernel::timer::ticks();
    let fautes = fiche.fault_count.load(Ordering::Relaxed);
    fiche.fault_count.store(fautes.wrapping_add(1), Ordering::Relaxed);
    fiche.last_fault_addr.store(addr, Ordering::Relaxed);
    fiche.last_fault_tick.store(maintenant, Ordering::Relaxed);
    fiche.activity_tick.store(maintenant, Ordering::Relaxed);
}

// ---------------------------------------------------------------------------
// LES LECTEURS : a froid, depuis BRDP, sans arreter personne
// ---------------------------------------------------------------------------

/// Le releve agrege de `forensics status`.
#[derive(Clone, Copy, Debug, Default)]
pub struct ForensicCounts {
    pub ready: u32,
    pub blocked: u32,
    pub zombie: u32,
    /// Taches actuellement dans un syscall.
    pub active_syscalls: u32,
    /// Taches dont l'attente ouverte dure depuis plus d'une seconde.
    pub waits_over_1s: u32,
    /// Fautes de page depuis l'amorcage, tous coeurs confondus.
    pub faults: u64,
}

/// Compte les taches par etat, et celles qui semblent figees.
///
/// Parcourt le registre emplacement par emplacement, chacun sous sa propre
/// garde de lecture : c'est une commande de diagnostic, pas un chemin chaud.
///
/// `now` est l'heure monotone de l'appelant. Les ages des fiches se calculent
/// en ticks -- l'horloge de leurs estampilles -- pour ne jamais melanger deux
/// horloges qui divergent sous QEMU/TCG, ou l'IRQ du PIT peut prendre du
/// retard sur le TSC.
pub fn forensic_counts(now: u64) -> ForensicCounts {
    let _ = now;
    let maintenant_tick = crate::kernel::timer::ticks();
    let mut releve = ForensicCounts::default();
    let longueur = registre_longueur().min(MAX_TACHES);
    for (slot, fiche) in FICHES_FORENSIQUES.iter().enumerate().take(longueur) {
        let Some(tache) = registre_tache(slot) else { continue };
        match tache.state.charge() {
            TaskState::Ready => releve.ready += 1,
            TaskState::Blocked => releve.blocked += 1,
            TaskState::Zombie => releve.zombie += 1,
        }
        drop(tache);
        if fiche.syscall_nr.load(Ordering::Acquire) != FORENSIC_NO_SYSCALL {
            releve.active_syscalls += 1;
        }
        if fiche.wait_kind.load(Ordering::Acquire) != WAIT_NONE
            && age_forensique_ns(fiche.wait_tick.load(Ordering::Relaxed), maintenant_tick)
                >= ATTENTE_LONGUE_NS
        {
            releve.waits_over_1s += 1;
        }
    }
    releve.faults = STALL_PF_BEGIN
        .iter()
        .map(|compteur| compteur.load(Ordering::Relaxed))
        .fold(0u64, |total, n| total.wrapping_add(n));
    releve
}

/// Ce que faisait un fil, lu a froid.
#[derive(Clone, Copy, Debug)]
pub struct ForensicThread {
    pub slot: usize,
    pub generation: u32,
    pub pid: u32,
    pub tid: u32,
    pub state: TaskState,
    pub priority: Priorite,
    pub on_cpu: i8,
    pub last_cpu: u8,
    pub in_kernel: bool,
    pub wait_kind: u8,
    pub wait_key: u64,
    pub wait_aux: u64,
    pub wait_age_ns: u64,
    pub wait_queue_key: usize,
    pub futex_key: u64,
    pub wake_deadline_ns: u64,
    pub syscall_nr: u64,
    pub syscall_age_ns: u64,
    pub arg0: u64,
    pub arg1: u64,
    pub arg2: u64,
    pub poll_phase: u32,
    pub poll_detail: u64,
    pub last_syscall_nr: u64,
    pub last_syscall_result: i64,
    pub last_syscall_duration_ns: u64,
    pub fault_count: u64,
    pub last_fault_addr: u64,
    pub last_fault_age_ns: u64,
    pub last_activity_age_ns: u64,
    pub user_rip: u64,
    pub user_rsp: u64,
    pub user_rbp: u64,
    pub user_cpu_ns: u64,
    pub kernel_cpu_ns: u64,
    pub context_switches: u64,
    pub migrations: u64,
    pub ready_age_ns: u64,
}

/// La fiche d'un emplacement, jointe a l'etat que la tache publie deja.
///
/// Rend `None` pour un emplacement vide ou hors registre. `now` sert aux
/// champs que la tache tient en nanosecondes (`ready_since_ns`) ; les ages de
/// la fiche se lisent en ticks, voir [`forensic_counts`].
pub fn forensic_thread(slot: usize, now: u64) -> Option<ForensicThread> {
    if slot >= registre_longueur().min(MAX_TACHES) {
        return None;
    }
    let tache = registre_tache(slot)?;
    let maintenant_tick = crate::kernel::timer::ticks();
    let fiche = &FICHES_FORENSIQUES[slot];

    let wait_kind = fiche.wait_kind.load(Ordering::Acquire);
    let syscall_nr = fiche.syscall_nr.load(Ordering::Acquire);
    let ready_since = tache.ready_since_ns.charge();

    let releve = ForensicThread {
        slot,
        generation: EMPLACEMENTS[slot].generation.load(Ordering::Acquire),
        pid: tache.process.pid,
        tid: tache.tid,
        state: tache.state.charge(),
        priority: tache.priorite.charge(),
        on_cpu: tache.on_cpu.charge(),
        last_cpu: tache.last_cpu.charge(),
        in_kernel: tache.in_kernel.charge(),
        wait_kind,
        wait_key: fiche.wait_key.load(Ordering::Relaxed),
        wait_aux: fiche.wait_aux.load(Ordering::Relaxed),
        wait_age_ns: if wait_kind == WAIT_NONE {
            0
        } else {
            age_forensique_ns(fiche.wait_tick.load(Ordering::Relaxed), maintenant_tick)
        },
        wait_queue_key: tache.wait_queue_key.charge(),
        futex_key: tache.futex_key.charge(),
        wake_deadline_ns: tache.wake_deadline_ns.charge(),
        syscall_nr,
        syscall_age_ns: if syscall_nr == FORENSIC_NO_SYSCALL {
            0
        } else {
            age_forensique_ns(fiche.syscall_tick.load(Ordering::Relaxed), maintenant_tick)
        },
        arg0: fiche.arg0.load(Ordering::Relaxed),
        arg1: fiche.arg1.load(Ordering::Relaxed),
        arg2: fiche.arg2.load(Ordering::Relaxed),
        poll_phase: fiche.poll_phase.load(Ordering::Acquire),
        poll_detail: fiche.poll_detail.load(Ordering::Relaxed),
        last_syscall_nr: fiche.last_syscall_nr.load(Ordering::Relaxed),
        last_syscall_result: fiche.last_syscall_result.load(Ordering::Relaxed),
        last_syscall_duration_ns: fiche.last_syscall_duree_ns.load(Ordering::Relaxed),
        fault_count: fiche.fault_count.load(Ordering::Relaxed),
        last_fault_addr: fiche.last_fault_addr.load(Ordering::Relaxed),
        last_fault_age_ns: age_forensique_ns(
            fiche.last_fault_tick.load(Ordering::Relaxed),
            maintenant_tick,
        ),
        last_activity_age_ns: age_forensique_ns(
            fiche.activity_tick.load(Ordering::Relaxed),
            maintenant_tick,
        ),
        user_rip: fiche.user_rip.load(Ordering::Relaxed),
        user_rsp: fiche.user_rsp.load(Ordering::Relaxed),
        user_rbp: fiche.user_rbp.load(Ordering::Relaxed),
        user_cpu_ns: tache.user_cpu_ns.charge(),
        kernel_cpu_ns: tache.kernel_cpu_ns.charge(),
        context_switches: tache.context_switches.charge(),
        migrations: tache.migrations.charge(),
        ready_age_ns: if ready_since == 0 { 0 } else { now.saturating_sub(ready_since) },
    };
    Some(releve)
}

// ---------------------------------------------------------------------------
// LES NOMS, tels qu'ils partent sur le fil
// ---------------------------------------------------------------------------

pub fn forensic_state_name(etat: TaskState) -> &'static str {
    match etat {
        TaskState::Ready => "ready",
        TaskState::Blocked => "blocked",
        TaskState::Zombie => "zombie",
    }
}

pub fn forensic_priority_name(priorite: Priorite) -> &'static str {
    match priorite {
        Priorite::Interactive => "interactive",
        Priorite::Normale => "normal",
    }
}

pub fn forensic_wait_name(kind: u8) -> &'static str {
    match kind {
        WAIT_NONE => "none",
        WAIT_WAIT_QUEUE => "wait_queue",
        WAIT_SLEEP => "sleep",
        WAIT_FUTEX => "futex",
        _ => "unknown",
    }
}

/// Les phases de la boucle de disponibilite, celles de `poll_phase_set`.
pub fn forensic_poll_name(phase: u32) -> &'static str {
    match phase {
        POLL_HORS => "none",
        POLL_ENTREE => "entry",
        POLL_BALAYAGE => "scan",
        POLL_PRET => "ready",
        POLL_ATTENTE => "wait",
        POLL_REVEIL => "wakeup",
        POLL_RETOUR => "return",
        _ => "unknown",
    }
}
