// ---------------------------------------------------------------------------
// BOUCHAUD_PROFIL_RIP_V1 : ou les fils du navigateur passent leur temps
// ---------------------------------------------------------------------------
//
// Endurance KVM, run 37664604267 : un cadre `srcdoc` de 200x80 met 5,3 s a
// executer son premier script et 10,3 s a charger ; la boucle d'evenements de
// la page prend jusqu'a 4,5 s de retard sur une echeance de 100 ms. Pendant ce
// temps RequestServer brule 47 % d'un coeur pour 57 appels systeme par
// seconde. Les compteurs du noyau (appels, fautes, temps user/sys) disent
// COMBIEN, pas OU : c'est du calcul en espace utilisateur.
//
// A chaque tic de quantum (un par coeur, toutes les `SCHED_QUANTUM_TICKS` ms),
// le coeur range le RIP interrompu et le fil courant dans SON anneau -- un seul
// ecrivain par anneau, IRQ masquees, aucune allocation, aucun verrou. Le coeur
// zero sans timer local echantillonne IRQ0 au meme rythme, pour ne pas peser
// quatre fois plus que les autres. Une tache (l'echantillonneur des services,
// toutes les 5 s) vide les anneaux, agrege par fil et publie les RIP les plus
// frequents, que `tools/ci/profil_rip.py` symbolise contre les binaires
// exacts du run.
//
// Lecture concurrente : un emplacement peut etre reecrit pendant qu'on le lit
// (anneau plein). L'echantillon est alors faux d'un cran -- c'est un profil
// statistique, pas un journal ; les debordements sont comptes (`perdus`).

const PROFIL_CASES: usize = 2048;

static PROFIL_RIP: [[AtomicU64; PROFIL_CASES]; MAX_CPUS] =
    [const { [const { AtomicU64::new(0) }; PROFIL_CASES] }; MAX_CPUS];
/// tid << 32 | pid ; bit 63 : interrompu en espace utilisateur.
static PROFIL_QUI: [[AtomicU64; PROFIL_CASES]; MAX_CPUS] =
    [const { [const { AtomicU64::new(0) }; PROFIL_CASES] }; MAX_CPUS];
static PROFIL_ECRITS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
/// Curseur du lecteur ; seule la tache de publication y touche.
static PROFIL_LUS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static PROFIL_PERDUS: AtomicU64 = AtomicU64::new(0);
static PROFIL_TOTAL: AtomicU64 = AtomicU64::new(0);

const PROFIL_BIT_USER: u64 = 1 << 63;

/// Depuis `sonde_gel_tic`, donc depuis l'interruption du tic.
fn profil_rip_note(cpu: usize, rip: u64, depuis_utilisateur: bool, source: u64) {
    if cpu >= MAX_CPUS {
        return;
    }
    if source == GEL_SOURCE_PIT {
        // Le coeur zero recoit aussi le tic de quantum quand le timer local
        // tourne : IRQ0 ne compte alors pas. Sinon, une IRQ0 sur quatre.
        if crate::arch::x86_64::smp::local_scheduler_timer_enabled()
            || crate::kernel::timer::ticks() % crate::arch::x86_64::smp::SCHED_QUANTUM_TICKS != 0
        {
            return;
        }
    }
    if crate::arch::x86_64::cpu::is_idle(cpu) {
        return;
    }
    let liste = tasks();
    let courant = CURRENT[cpu].load(Ordering::Acquire);
    if courant == NO_TASK || courant >= liste.len() {
        return;
    }
    let tache = &liste[courant];
    if tache.noyau {
        return;
    }
    let qui = ((tache.tid as u64) << 32)
        | tache.process.pid as u64
        | if depuis_utilisateur { PROFIL_BIT_USER } else { 0 };
    let n = PROFIL_ECRITS[cpu].load(Ordering::Relaxed);
    let i = (n as usize) % PROFIL_CASES;
    PROFIL_RIP[cpu][i].store(rip, Ordering::Relaxed);
    PROFIL_QUI[cpu][i].store(qui, Ordering::Relaxed);
    PROFIL_ECRITS[cpu].store(n.wrapping_add(1), Ordering::Release);
}

struct ProfilFil {
    pid: u32,
    user: u32,
    noyau: u32,
    /// (seau de 256 octets, echantillons, dernier RIP exact du seau)
    seaux: Vec<(u64, u32, u64)>,
    /// Les memes, pour les echantillons pris dans le noyau (appel systeme,
    /// faute) : un fil « noyau » a 90 % dit peu sans le chemin noyau.
    seaux_noyau: Vec<(u64, u32, u64)>,
}

fn profil_seau(seaux: &mut Vec<(u64, u32, u64)>, rip: u64) {
    let seau = rip & !0xff;
    match seaux.iter_mut().find(|s| s.0 == seau) {
        Some(s) => {
            s.1 += 1;
            s.2 = rip;
        }
        None => seaux.push((seau, 1, rip)),
    }
}

fn profil_top(seaux: &mut Vec<(u64, u32, u64)>, combien: usize) -> String {
    seaux.sort_unstable_by(|a, b| b.1.cmp(&a.1));
    let mut top = String::new();
    for (k, s) in seaux.iter().take(combien).enumerate() {
        if k > 0 {
            top.push(',');
        }
        let _ = core::fmt::Write::write_fmt(&mut top, format_args!("{:#x}:{}", s.2, s.1));
    }
    if top.is_empty() {
        top.push('-');
    }
    top
}

/// Vide les anneaux et publie une ligne `[PERF-RIP]` par fil assez present.
/// Depuis une tache, jamais depuis une IRQ.
pub fn publie_profil_rip() {
    use alloc::collections::BTreeMap;
    let mut fils: BTreeMap<u32, ProfilFil> = BTreeMap::new();
    let mut vus = 0u64;
    for cpu in 0..MAX_CPUS {
        let ecrits = PROFIL_ECRITS[cpu].load(Ordering::Acquire);
        let mut lus = PROFIL_LUS[cpu].load(Ordering::Relaxed);
        if ecrits.wrapping_sub(lus) > PROFIL_CASES as u64 {
            PROFIL_PERDUS.fetch_add(ecrits - lus - PROFIL_CASES as u64, Ordering::Relaxed);
            lus = ecrits - PROFIL_CASES as u64;
        }
        while lus != ecrits {
            let i = (lus as usize) % PROFIL_CASES;
            let qui = PROFIL_QUI[cpu][i].load(Ordering::Relaxed);
            let rip = PROFIL_RIP[cpu][i].load(Ordering::Relaxed);
            lus = lus.wrapping_add(1);
            vus += 1;
            let tid = ((qui & !PROFIL_BIT_USER) >> 32) as u32;
            let fil = fils.entry(tid).or_insert_with(|| ProfilFil {
                pid: qui as u32,
                user: 0,
                noyau: 0,
                seaux: Vec::new(),
                seaux_noyau: Vec::new(),
            });
            if qui & PROFIL_BIT_USER == 0 {
                fil.noyau += 1;
                profil_seau(&mut fil.seaux_noyau, rip);
            } else {
                fil.user += 1;
                profil_seau(&mut fil.seaux, rip);
            }
        }
        PROFIL_LUS[cpu].store(lus, Ordering::Relaxed);
    }
    let total = PROFIL_TOTAL.fetch_add(vus, Ordering::Relaxed) + vus;
    let maintenant = crate::kernel::timer::monotonic_ms();
    let base = crate::kernel::vmm::user_load_base();
    for (tid, fil) in fils.iter_mut() {
        // Un fil vu moins de 10 fois en 5 s (2 % d'un coeur au quantum de
        // 4 ms) n'explique aucune latence ; ne pas noyer le journal.
        if fil.user + fil.noyau < 10 {
            continue;
        }
        let image = process_by_pid(fil.pid)
            .map(|p| p.metadata.lock().name.clone())
            .unwrap_or_default();
        let top = profil_top(&mut fil.seaux, 8);
        let topn = profil_top(&mut fil.seaux_noyau, 4);
        crate::serial_println!(
            "[PERF-RIP] t={} pid={} tid={} image={} user={} noyau={} base={:#x} top={} topn={}",
            maintenant, fil.pid, tid, image, fil.user, fil.noyau, base, top, topn,
        );
    }
    crate::serial_println!(
        "[PERF-RIP-RESUME] t={} echantillons={} total={} perdus={}",
        maintenant, vus, total, PROFIL_PERDUS.load(Ordering::Relaxed),
    );
}
