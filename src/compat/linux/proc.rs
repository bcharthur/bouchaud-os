//! Cycle de vie des processus : `fork`, `execve`, `wait4`, signaux.
//!
//! Ces quatre appels forment un tout : un shell `fork`, l'enfant `execve`, le
//! parent `wait4`, et le noyau lui envoie `SIGCHLD`. Les implementer separement
//! n'aurait pas de sens — c'est leur enchainement qui doit fonctionner.

use alloc::sync::Arc;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::arch::x86_64::{smp, usermode::{self, TrapFrame}};
use crate::kernel::abi::{errno, user_read_u64, user_write, user_write_u32};
use crate::kernel::signal::{self, SigAction, SIG_DFL, SIG_IGN};
use crate::kernel::task::{self, FileTable, Mm, MmState, Process, ProcessLifecycle, ProcessMetadata, Task};
use crate::kernel::sync::SpinLock;
use crate::kernel::{elf, vmm};

static FORK_APPELS: AtomicU64 = AtomicU64::new(0);
static FORK_TOTAL_NS: AtomicU64 = AtomicU64::new(0);
static FORK_COPIE_NS: AtomicU64 = AtomicU64::new(0);
static FORK_PAGES_COPIEES: AtomicU64 = AtomicU64::new(0);
static FORK_PIRE_NS: AtomicU64 = AtomicU64::new(0);

static EXEC_QUIESCE_WAIT_NS: AtomicU64 = AtomicU64::new(0);
static EXEC_QUIESCE_MAX_NS: AtomicU64 = AtomicU64::new(0);

pub fn exec_quiesce_stats() -> (u64, u64) {
    (
        EXEC_QUIESCE_WAIT_NS.load(Ordering::Relaxed),
        EXEC_QUIESCE_MAX_NS.load(Ordering::Relaxed),
    )
}

/// `fork` : duplique le processus courant.
///
/// La copie de l'espace d'adressage est immediate (voir
/// `AddressSpace::duplicate`). La table de descripteurs est clonee, ce qui
/// partage les objets sous-jacents — tubes, sockets, `eventfd` : c'est
/// exactement ce que POSIX demande, et ce sur quoi repose le chainage
/// `cmd1 | cmd2`.
pub fn sys_fork(frame: &TrapFrame) -> i64 {
    let parent = task::current_process();
    // BOUCHAUD_P23_COUT_DU_FORK
    //
    // Le commentaire de `duplicate` assume la recopie immediate ; personne ne
    // l'avait CHIFFREE. Or c'est le chemin que prend chaque processus du
    // navigateur -- `Core::Process::spawn` fait `fork` puis `execve`, et la
    // copie entiere est jetee a la ligne suivante.
    //
    // `tools/ci/run_cout_fork.sh` mesure la loi en anneau 3 : environ deux
    // millisecondes par mebioctet resident, lineaire sur quatre paliers. Ce
    // qui manquait etait de savoir COMBIEN un `fork` donne du navigateur
    // coute, et sur quelles pages.
    let debut_ns = crate::kernel::timer::monotonic_ns();

    let (space, compte, brk_start, brk, mmap_next, partages, limite_as, promesses, clean_pages) = {
        let mm = parent.mm.lock();
        let (space, compte) = match mm.space.duplicate() {
            Some(resultat) => resultat,
            None => return -errno::ENOMEM,
        };
        (space, compte, mm.brk_start, mm.brk, mm.mmap_next, mm.partages.clone(),
            mm.limite_as, mm.promesses.clone(), mm.clean_pages.clone())
    };
    let apres_espace_ns = crate::kernel::timer::monotonic_ns();
    let files = parent.files.lock().clone();
    let (cwd, uid, gid, name, ecran) = {
        let metadata = parent.metadata.lock();
        (metadata.cwd, metadata.uid, metadata.gid, metadata.name.clone(), metadata.ecran)
    };
    let signals = parent.signals.lock().clone();
    let parent_pid = parent.pid;
    let resource_group_id = parent.resource_group_id;
    let resource_group_name = parent.resource_group_name.clone();

    // `duplicate` a re-mappe les pages empruntees sur les memes frames — c'est
    // la semantique de `MAP_SHARED` a travers `fork`. Le fils tient donc
    // reellement ces plages, et doit en prendre les references : sans cela, le
    // premier `munmap` du pere evincerait des frames que le fils lit encore.
    let mut retained_clean = Vec::new();
    for mapping in &clean_pages {
        if !crate::kernel::clean_page_cache::retain(mapping.key) {
            for key in retained_clean {
                crate::kernel::clean_page_cache::release(key);
            }
            drop(space);
            return -errno::ENOMEM;
        }
        retained_clean.push(mapping.key);
    }
    for plage in &partages {
        crate::kernel::partage::mappe(plage.node);
    }
    // Une prise de reference par page propre, chacune sous le verrou GLOBAL du
    // cache. Sur un navigateur dont le texte partage fait des centaines de
    // mebioctets, ce n'est pas le meme cout que la recopie et il ne se corrige
    // pas au meme endroit : il est donc borne a part.
    let apres_references_ns = crate::kernel::timer::monotonic_ns();

    let pid = crate::kernel::process::spawn(&name, uid as u16);
    let nom_journal = name.clone();
    let mut child_signals = signals;
    // Les signaux en attente ne sont pas herites : ils appartenaient au parent.
    child_signals.pending = 0;

    let child = Arc::new(Process {
        pid,
        parent: parent_pid,
        resource_group_id,
        resource_group_name,
        mm: Arc::new(Mm::new(MmState { space, brk_start, brk, mmap_next,
            partages, limite_as, promesses, clean_pages })),
        files: Arc::new(FileTable::new(files)),
        metadata: SpinLock::new(ProcessMetadata { name, cwd, uid, gid, ecran }),
        lifecycle: SpinLock::new(ProcessLifecycle { exit_code: 0, zombie: false, threads: 1 }),
        signals: SpinLock::new(child_signals),
    });
    task::register_process(child.clone());

    // L'enfant reprend a l'instruction suivant le `syscall`, avec 0 dans rax :
    // c'est la seule chose qui distingue les deux retours de `fork`.
    let mut child_frame = *frame;
    child_frame.rax = 0;
    let mut child_task = Task::new(child, child_frame);
    // La base FS doit suivre : c'est le TLS, et la premiere chose que fait la
    // libc dans l'enfant est de lire `%fs:0` pour retrouver sa structure de
    // thread. Une base nulle la ferait dereferencer l'adresse 0.
    child_task.fs_base = usermode::fs_base();
    task::register(child_task);

    let fin_ns = crate::kernel::timer::monotonic_ns();
    let kio_copies = compte.pages_copiees.saturating_mul(crate::kernel::vmm::PAGE_SIZE) / 1024;
    FORK_TOTAL_NS.fetch_add(fin_ns.saturating_sub(debut_ns), Ordering::Relaxed);
    FORK_COPIE_NS.fetch_add(apres_espace_ns.saturating_sub(debut_ns), Ordering::Relaxed);
    FORK_PAGES_COPIEES.fetch_add(compte.pages_copiees, Ordering::Relaxed);
    FORK_APPELS.fetch_add(1, Ordering::Relaxed);
    FORK_PIRE_NS.fetch_max(fin_ns.saturating_sub(debut_ns), Ordering::Relaxed);
    crate::kernel::dmesg::log_fmt(format_args!(
        "PERF_FORK t={} pere={} enfant={} image={} duree_us={} copie_us={} references_us={} reste_us={} pages_copiees={} pages_empruntees={} kio_copies={}",
        fin_ns / 1_000_000,
        parent_pid,
        pid,
        nom_journal,
        fin_ns.saturating_sub(debut_ns) / 1_000,
        apres_espace_ns.saturating_sub(debut_ns) / 1_000,
        apres_references_ns.saturating_sub(apres_espace_ns) / 1_000,
        fin_ns.saturating_sub(apres_references_ns) / 1_000,
        compte.pages_copiees,
        compte.pages_empruntees,
        kio_copies,
    ));

    pid as i64
}

/// Ce que tous les `fork` de la machine ont coute, pour le releve periodique.
pub fn fork_stats() -> (u64, u64, u64, u64, u64) {
    (
        FORK_APPELS.load(Ordering::Relaxed),
        FORK_TOTAL_NS.load(Ordering::Relaxed),
        FORK_COPIE_NS.load(Ordering::Relaxed),
        FORK_PAGES_COPIEES.load(Ordering::Relaxed),
        FORK_PIRE_NS.load(Ordering::Relaxed),
    )
}

/// Lit un tableau de chaines C termine par un pointeur nul (`argv`, `envp`).
fn read_string_array(addr: u64) -> Option<Vec<String>> {
    let mut out = Vec::new();
    if addr == 0 {
        return Some(out);
    }
    for index in 0..1024u64 {
        let pointer = user_read_u64(addr + index * 8)?;
        if pointer == 0 {
            break;
        }
        // La chaine peut vivre dans une page file-backed encore non
        // residente (typiquement .rodata du programme qui appelle execve).
        // `user_string` sait materialiser ces pages avant copyin.
        out.push(super::user_string(pointer)?);
    }
    Some(out)
}

/// `execve` : remplace l'image du processus courant.
///
/// Ne revient jamais en cas de succes — le processus repart au point d'entree
/// du nouveau programme.
pub fn sys_execve(path_addr: u64, argv_addr: u64, envp_addr: u64) -> i64 {
    // BOUCHAUD_C55_LES_QUARANTE_TROIS_SECONDES
    //
    // Le segment `fork_exit -> exec_enter` vaut 43,4 s pour le premier
    // WebWorker de la baseline #347. Aucune faute POST-exec ne peut
    // l'expliquer : il se situe AVANT l'entree dans `execve`. Le confondre
    // avec le cout des fautes de page serait attribuer a la memoire une
    // attente d'ordonnancement.
    //
    // Deux bornes le decoupent : `CHILD_AFTER_FORK` (l'enfant est enfin
    // planifie) et celle-ci (il entre dans l'appel). L'intervalle entre les
    // deux est ce que l'enfant EXECUTE avant d'appeler `execve` ; ce qui
    // precede est ce qu'il a ATTENDU.
    crate::kernel::dmesg::log_fmt(format_args!(
        "EXECVE_SYSCALL_ENTER t={} pid={}",
        crate::kernel::timer::monotonic_ms(),
        crate::kernel::task::current_process_local()
            .map(|p| p.pid as i64)
            .unwrap_or(-1),
    ));
    // Tout doit etre lu **avant** de detruire l'ancien espace d'adressage :
    // le chemin, les arguments et l'environnement y vivent encore.
    let path = match crate::kernel::abi::resolve_user_path(path_addr) {
        Some(path) => crate::kernel::security::filesystem::canonical_at(
            crate::kernel::security::filesystem::AT_FDCWD,
            path.as_str(),
        ).unwrap_or(path),
        None => return -errno::EFAULT,
    };
    let argv = match read_string_array(argv_addr) {
        Some(list) => list,
        None => return -errno::EFAULT,
    };
    let envp = match read_string_array(envp_addr) {
        Some(list) => list,
        None => return -errno::EFAULT,
    };

    let process = task::current_process();
    let cwd = process.metadata.lock().cwd;
    // BOUCHAUD_P23_ETAPES_DE_L_EXECVE
    //
    // `construit_tache` publiait deja `PERF_EXEC_PRET`, mais AUCUN enfant du
    // navigateur ne passe par la : le courtier fait `fork` puis `execve`, donc
    // par ici. Les vingt-trois secondes entre `processus_lance` et `main` du
    // premier WebWorker n'avaient, cote noyau, pas une seule borne.
    //
    // Les etapes sont separees parce que leurs remedes le sont : resoudre un
    // chemin, lire des en-tetes, poser des promesses, construire une pile,
    // attendre que les autres coeurs lachent l'ancien CR3, et LIBERER l'ancien
    // espace ne se corrigent pas au meme endroit.
    let debut_ns = crate::kernel::timer::monotonic_ns();

    let node = {
        let fs = crate::fs::ramfs::fs();
        match fs.resolve(&path, cwd) {
            Some(node) if fs.nodes[node].kind == crate::fs::ramfs::NodeKind::File => node,
            Some(_) => return -errno::EISDIR,
            None => return -errno::ENOENT,
        }
    };
    let apres_ouverture_ns = crate::kernel::timer::monotonic_ns();
    if elf::parse_node(node).is_err() {
        return -errno::ENOEXEC;
    }
    let apres_entetes_ns = crate::kernel::timer::monotonic_ns();

    let mut space = match vmm::AddressSpace::new() {
        Some(space) => space,
        None => return -errno::ENOMEM,
    };

    let mut new_promises = Vec::new();
    let image = match elf::load_node_lazy(node, vmm::user_load_base(), &mut new_promises) {
        Ok(image) => image,
        Err(_) => return -errno::ENOEXEC,
    };
    let (entry, interp_base) = match image.interp.as_deref() {
        None => (image.entry, 0),
        Some(interp_path) => {
            let fs = crate::fs::ramfs::fs();
            let interp_node = match fs.resolve(interp_path, cwd) {
                Some(node) => node,
                None => return -errno::ENOENT,
            };
            match elf::load_node_lazy(interp_node, vmm::user_interp_base(), &mut new_promises) {
                Ok(interp) => (interp.entry, interp.base),
                Err(_) => return -errno::ENOEXEC,
            }
        }
    };

    let (uid, gid) = {
        let metadata = process.metadata.lock();
        (metadata.uid, metadata.gid)
    };
    let argv = if argv.is_empty() {
        alloc::vec![path.clone()]
    } else {
        argv
    };
    let layout = elf::StackLayout {
        argv: &argv,
        envp: &envp,
        image: &image,
        interp_base,
        uid,
        gid,
    };
    let apres_projections_ns = crate::kernel::timer::monotonic_ns();
    let stack = match elf::build_stack(&mut space, &mut new_promises, &layout) {
        Ok(stack) => stack,
        Err(_) => return -errno::ENOMEM,
    };
    let apres_pile_ns = crate::kernel::timer::monotonic_ns();

    // All fallible image construction is complete. Only now terminate sibling
    // tasks and retire the old identity: failed execve must leave both intact.
    task::terminate_sibling_threads();
    // Stop every sibling on the old CR3 before replacement.
    let old_identity = process.mm.lock().space.identity();
    old_identity.begin_retire();
    let self_cpu = smp::cpu_index();
    let active = old_identity.active_cpus() & !(1u64 << self_cpu);
    let depth = crate::kernel::smp_lock::suspend_for_schedule();
    for cpu in 0..smp::MAX_CPUS.min(64) {
        if active & (1u64 << cpu) != 0 {
            smp::reschedule_cpu(cpu);
        }
    }
    let wait_start = crate::kernel::timer::monotonic_ns();
    old_identity.wait_remote_quiescent(self_cpu);
    let waited = crate::kernel::timer::monotonic_ns().saturating_sub(wait_start);
    EXEC_QUIESCE_WAIT_NS.fetch_add(waited, Ordering::Relaxed);
    EXEC_QUIESCE_MAX_NS.fetch_max(waited, Ordering::Relaxed);
    crate::kernel::smp_lock::resume_after_schedule(depth);
    let apres_quiescence_ns = crate::kernel::timer::monotonic_ns();

    // BOUCHAUD_C8_SUPERVISION_SUR_EXECVE_V1
    //
    // Les enfants du navigateur -- WebContent, RequestServer, ImageDecoder --
    // ne naissent pas par `lance_detache` : le courtier les cree par `fork` +
    // `execve`, donc par ICI. Sans ce point, la supervision ne voyait aucun
    // d'eux, et le registre restait vide quel que soit le nombre d'onglets.
    //
    // Le pid ne change pas a l'execve ; c'est l'IMAGE qui change. Le
    // reenregistrement remplace donc l'entree precedente, ce que
    // `note_lancement` fait deja pour un pid recycle -- un processus qui passe
    // de courtier a moteur de rendu ne doit pas garder l'ancien role.
    if let Some(role) = crate::kernel::navigateur::supervision::Role::depuis_image(&path) {
        let courtier = process.parent;
        crate::kernel::navigateur::supervision::note_lancement(
            process.pid, role, courtier, process.pid,
            crate::kernel::timer::monotonic_ns(),
        );
    }
    let nom_journal = path.clone();
    // Releve TOT : `process` n'est plus empruntable au point no-return.
    let pid_journal = process.pid;
    process.metadata.lock().name = path;
    process.files.lock().close_on_exec();
    process.signals.lock().reset_for_exec();
    process.lifecycle.lock().threads = 1;
    let (old, old_clean, old_shared) = {
        let mut mm = process.mm.lock();
        mm.brk_start = (image.end + 0x10_0000) & !0xFFF;
        mm.brk = mm.brk_start;
        mm.mmap_next = vmm::user_mmap_base();
        // L'ancien espace disparait avec ses mappages : les references qu'il
        // tenait sur le cache partage doivent partir avec lui.
        let old_shared = core::mem::take(&mut mm.partages);
        let old_clean = core::mem::take(&mut mm.clean_pages);
        mm.promesses = new_promises;
        // L'ancien espace est detruit ici. Son `Drop` remet CR3 sur la table du
        // noyau s'il etait actif : c'est pour cela qu'on active le nouveau
        // juste apres, avant de retourner en ring 3.
        let old = core::mem::replace(&mut mm.space, space);
        let new_identity = mm.space.identity();
        drop(mm);
        process.mm.replace_activation(new_identity);
        process.mm.activate();
        old_identity.mark_inactive(smp::cpu_index());
        (old, old_clean, old_shared)
    };
    let apres_bascule_ns = crate::kernel::timer::monotonic_ns();
    task::forget_fault_space(old.pml4());
    for mapping in old_clean {
        crate::kernel::clean_page_cache::release(mapping.key);
    }
    for mapping in old_shared {
        crate::kernel::partage::demappe(mapping.node);
    }
    // LIBERER COUTE AUSSI CHER QUE COPIER, ET C'EST LE MEME `fork` QUI PAIE.
    //
    // `Drop` rend une a une toutes les frames que la duplication venait
    // d'allouer. Un `fork` suivi d'un `execve` parcourt donc DEUX FOIS la
    // taille residente du pere : une fois pour la recopier, une fois pour la
    // rendre. Le second passage etait invisible.
    let pages_rendues = old.mapped_pages() as u64;
    drop(old);
    let fin_ns = crate::kernel::timer::monotonic_ns();
    crate::kernel::dmesg::log_fmt(format_args!(
        "PERF_EXECVE t={} image={} pid={} duree_us={} ouverture_us={} entetes_us={} projections_us={} pile_us={} quiescence_us={} bascule_us={} liberation_us={} pages_rendues={}",
        fin_ns / 1_000_000,
        nom_journal,
        process.pid,
        fin_ns.saturating_sub(debut_ns) / 1_000,
        apres_ouverture_ns.saturating_sub(debut_ns) / 1_000,
        apres_entetes_ns.saturating_sub(apres_ouverture_ns) / 1_000,
        apres_projections_ns.saturating_sub(apres_entetes_ns) / 1_000,
        apres_pile_ns.saturating_sub(apres_projections_ns) / 1_000,
        apres_quiescence_ns.saturating_sub(apres_pile_ns) / 1_000,
        apres_bascule_ns.saturating_sub(apres_quiescence_ns) / 1_000,
        fin_ns.saturating_sub(apres_bascule_ns) / 1_000,
        pages_rendues,
    ));

    // BOUCHAUD_C45_LA_TENUE_SE_MESURE
    //
    // La liberation de l'ancien espace fait l'essentiel de l'execve, et elle
    // passe par `free_frames_lot`. Publier la TENUE du verrou ici, au moment ou
    // elle vient d'etre payee, evite d'avoir a la deduire d'un releve
    // periodique qui melangerait plusieurs processus.
    {
        let (tranches, frames_rendues, tenue_ns, pire_ns, attente_ns, contentions) =
            crate::kernel::vmm::stats_liberation_lot();
        crate::kernel::dmesg::log_fmt(format_args!(
            "PERF_FRAME_FREE_BATCH t={} pid={} tranche={} tranches={} frames={} \
duration_us={} lock_hold_us={} max_lock_hold_us={} attente_us={} contentions={}",
            crate::kernel::timer::monotonic_ms(),
            pid_journal,
            crate::kernel::vmm::tranche_liberation(),
            tranches,
            frames_rendues,
            (tenue_ns + attente_ns) / 1_000,
            tenue_ns / 1_000,
            pire_ns / 1_000,
            attente_ns / 1_000,
            contentions,
        ));
    }

    // La tache repart de zero : nouvelle trame, pile noyau reinitialisee. La
    // pile noyau courante (celle de cet appel systeme) est abandonnee telle
    // quelle — c'est sans consequence, elle repart du sommet a la prochaine
    // entree.
    // Contenu PRIVE de la tache : acces exclusif pris explicitement.
    let mut task = task::current_exclusif();
    task.fs_base = 0;
    task.clear_child_tid = 0;
    usermode::set_fs_base(0);
    usermode::set_kernel_stack(task.kstack_top);
    let frame = TrapFrame::new_user(entry, stack);
    task.frame = frame;
    // Le garde est rendu ICI : la reprise en ring 3 ne revient jamais, et le
    // conserver laisserait l'emplacement marque exclusif pour toujours --
    // aucun recyclage ne pourrait plus le reprendre.
    drop(task);

    // BOUCHAUD_EXECVE_NORETURN_BKL_FIX_V1
    //
    // `sys_execve` ne retourne jamais vers `syscall_dispatch`: il saute
    // directement vers le nouveau ring3 via `resume_usermode`. Le RAII
    // `KernelGuard` cree dans syscall_dispatch est donc abandonne sur l'ancienne
    // pile noyau et son Drop ne peut jamais liberer le BKL.
    //
    // Avant ce fix, un execve reussi laissait OWNER=CPU courant et DEPTH=1
    // indefiniment. Le nouveau programme pouvait etre preempte, mais toute la
    // machine continuait avec un BKL fantome, ce qui gelait Ladybird.
    //
    // Comme cette pile ne reprendra jamais, on libere explicitement toute la
    // profondeur BKL et on ferme aussi la sonde syscall avant l'iretq.
    task::stall_site_clear();
    task::stall_syscall_exit();
    task::account_resume_user_noreturn();

    // BOUCHAUD_C39_DEUX_RAII_ABANDONNES_PAS_UN
    //
    // `syscall_dispatch` ouvre DEUX gardes RAII avant d'appeler l'appel
    // systeme, et le chemin no-return les abandonne tous les deux :
    //
    //     let _domaine = sync::portee(sync::Domaine::Syscall);   // (2)
    //     let kernel   = smp_lock::enter();                      // (1)
    //
    // (1) est compense juste en dessous par `suspend_for_schedule`. (2) ne
    // l'etait PAS : son `Drop` appelle `DOMAINES.sort(cpu)`, qui n'est jamais
    // execute. `sommet[cpu]` monte donc d'un cran a chaque execve REUSSI et ne
    // redescend jamais.
    //
    // La sonde mesure les deux profondeurs de part et d'autre de la bascule,
    // pour que la fuite soit un CHIFFRE et non une lecture de code. `domaines_`
    // est ce qui doit rester stable d'un execve au suivant ; s'il monte, la
    // fuite est la.
    let cpu_courant = crate::arch::x86_64::smp::cpu_index();
    let registre = crate::kernel::sync::registre_domaines();
    let domaines_avant = registre.profondeur(cpu_courant);
    let depth_avant = crate::kernel::smp_lock::profondeur_locale();

    let abandoned_depth = crate::kernel::smp_lock::suspend_for_schedule();
    debug_assert!(
        abandoned_depth > 0,
        "execve: chemin no-return sans BKL syscall actif"
    );

    let portees_refermees = crate::kernel::sync::referme_portees_abandonnees(cpu_courant);
    let domaines_apres = registre.profondeur(cpu_courant);
    let owner_apres = crate::kernel::smp_lock::owner_cpu();
    crate::kernel::dmesg::log_fmt(format_args!(
        "PERF_EXECVE_BKL t={} pid={} cpu={} depth_avant={} depth_abandonne={} \
owner_apres={} domaines_avant={} domaines_apres={} portees_refermees={} \
debordements={} reperes_perimes={}",
        crate::kernel::timer::monotonic_ms(),
        pid_journal,
        cpu_courant,
        depth_avant,
        abandoned_depth,
        match owner_apres {
            Some(c) => c as i64,
            None => -1,
        },
        domaines_avant,
        domaines_apres,
        portees_refermees,
        registre.debordements(),
        registre.reperes_perimes(),
    ));

    unsafe { usermode::resume_usermode(&frame) }
}

/// `wait4` : attend la fin d'un processus fils.
pub fn sys_wait4(pid: i64, status_addr: u64, options: u32, _rusage: u64) -> i64 {
    const WNOHANG: u32 = 1;
    let parent_pid = task::current_process().pid;

    loop {
        let zombies = task::zombie_children(parent_pid);
        let found = zombies
            .iter()
            .find(|(child, _)| pid <= 0 || *child == pid as u32)
            .copied();

        if let Some((child_pid, code)) = found {
            if status_addr != 0 {
                // Encodage `wait` : octet de poids faible = cause, octet
                // suivant = code de sortie. Un code >= 128 signale une mort par
                // signal, comme le fait `kill_faulting_task`.
                let status = if code >= 128 {
                    (code as u32 - 128) & 0x7F
                } else {
                    ((code as u32) & 0xFF) << 8
                };
                user_write_u32(status_addr, status);
            }
            task::collect_child(child_pid);
            return child_pid as i64;
        }

        if !task::has_children(parent_pid) {
            return -errno::ECHILD;
        }
        if options & WNOHANG != 0 {
            return 0;
        }

        // BOUCHAUD_C70_SE_DECLARER_EN_ATTENTE_AVANT_DE_REVERIFIER
        //
        // Blocage jusqu'a ce qu'un fils se termine : c'est `exit_current` qui
        // remettra cette tache en etat pret. L'ORDRE compte, et il a coute une
        // integration entiere.
        //
        // Le reveilleur, `notify_parent_of_exit`, exige DEUX conditions :
        //
        //     waiting_for_child.compare_exchange(true, false).is_ok()
        //  && state.echange(Blocked, Ready)
        //
        // La version precedente cherchait les zombies, ne trouvait rien, PUIS
        // se declarait en attente. Un fils qui mourait dans cet intervalle
        // trouvait `waiting_for_child` encore a faux : son reveil tombait dans
        // le vide, et le parent s'endormait ensuite pour toujours. C'est le
        // blocage de `qemu / os primitives` apres `SESSION_PERE_SORT fils=4` :
        // machine vivante, shell jamais repris, `pretes=2` avec `au_repos=4`.
        //
        // `Blocked` AVANT le drapeau, et non l'inverse : le reveilleur qui
        // gagne le `compare_exchange` trouve alors forcement l'etat `Blocked`
        // a echanger. Pose dans l'autre sens, il consommait le drapeau puis
        // echouait sur l'etat, et perdait le reveil aussi surement.
        //
        // Puis on REVERIFIE. Un reveil emis avant notre declaration n'a pas pu
        // nous atteindre, mais le zombie qu'il annoncait, lui, est visible.
        {
            let task = task::current();
            task.state.range(task::TaskState::Blocked);
            task.waiting_for_child.range(true);
        }
        if !task::zombie_children(parent_pid).is_empty() {
            let task = task::current();
            task.waiting_for_child.range(false);
            task.state.range(task::TaskState::Ready);
            continue;
        }
        // schedule() effectue deja HLT avec le BKL suspendu si necessaire.
        let _ = task::schedule();
        let task = task::current();
        task.waiting_for_child.range(false);
        task.state.range(task::TaskState::Ready);
    }
}

// --- Signaux -----------------------------------------------------------------

/// `rt_sigaction`.
pub fn sys_rt_sigaction(signal: u32, act: u64, oldact: u64) -> i64 {
    if signal == 0 || signal as usize > signal::NSIG {
        return -errno::EINVAL;
    }
    // SIGKILL et SIGSTOP ne peuvent etre ni interceptes ni ignores.
    if act != 0 && (signal == signal::SIGKILL || signal == signal::SIGSTOP) {
        return -errno::EINVAL;
    }
    let process = task::current_process();
    let index = signal as usize - 1;

    if oldact != 0 {
        let previous = process.signals.lock().actions[index];
        if !signal::write_sigaction(oldact, &previous) {
            return -errno::EFAULT;
        }
    }
    if act != 0 {
        let action = match signal::read_sigaction(act) {
            Some(action) => action,
            None => return -errno::EFAULT,
        };
        process.signals.lock().actions[index] = action;
    }
    0
}

/// `rt_sigprocmask`.
pub fn sys_rt_sigprocmask(how: i32, set: u64, oldset: u64) -> i64 {
    let process = task::current_process();
    let current_mask = process.signals.lock().blocked;
    if oldset != 0 && !user_write(oldset, &current_mask.to_le_bytes()) {
        return -errno::EFAULT;
    }
    if set != 0 {
        let new = match user_read_u64(set) {
            Some(value) => value,
            None => return -errno::EFAULT,
        };
        let mut signals = process.signals.lock();
        signals.blocked = match how {
            signal::SIG_BLOCK => current_mask | new,
            signal::SIG_UNBLOCK => current_mask & !new,
            signal::SIG_SETMASK => new,
            _ => return -errno::EINVAL,
        };
        // Ces deux-la ne se bloquent pas, quoi qu'on demande.
        signals.blocked &= !(1 << (signal::SIGKILL - 1));
        signals.blocked &= !(1 << (signal::SIGSTOP - 1));
    }
    0
}

/// `rt_sigreturn` : retour d'un gestionnaire de signal.
///
/// Ne renvoie pas de valeur : la trame entiere est ecrasee par l'etat
/// sauvegarde au moment de la livraison.
pub fn sys_rt_sigreturn(frame: &mut TrapFrame) -> i64 {
    match signal::restore(frame) {
        Some(mask) => {
            task::current_process().signals.lock().blocked = mask;
            frame.rax as i64
        }
        None => {
            // Trame illisible : le gestionnaire a corrompu sa pile, il n'y a
            // aucun etat coherent ou revenir.
            task::exit_group(128 + signal::SIGSEGV as i32)
        }
    }
}

/// Envoie un signal a un processus (`kill`) ou a un thread (`tkill`).
pub fn send_signal(pid: u32, signal: u32) -> i64 {
    if signal == 0 {
        // Signal 0 : simple test d'existence du processus.
        return if task::process_by_pid(pid).is_some() {
            0
        } else {
            -errno::ESRCH
        };
    }
    if signal as usize > signal::NSIG {
        return -errno::EINVAL;
    }
    match task::process_by_pid(pid) {
        Some(process) => {
            process.signals.lock().raise(signal);
            // Un signal doit reveiller une tache endormie, sinon un `SIGTERM`
            // sur un processus bloque resterait sans effet.
            task::wake_for_signal(pid);
            0
        }
        None => -errno::ESRCH,
    }
}

/// `kill(pid, sig)`.
pub fn sys_kill(pid: i64, signal: u32) -> i64 {
    if pid <= 0 {
        // Groupes de processus : on traite comme « moi-meme ».
        let self_pid = task::current_process().pid;
        return send_signal(self_pid, signal);
    }
    send_signal(pid as u32, signal)
}

/// `tkill(tid, sig)` : le premier argument est un **identifiant de thread**,
/// pas de processus.
///
/// La distinction n'est pas theorique : c'est par cet appel que `raise()` de
/// musl s'envoie un signal a lui-meme. Le confondre avec un pid fait echouer
/// tout `raise` en `ESRCH`, donc `abort()`, `assert()` et tout gestionnaire
/// declenche depuis le programme lui-meme.
pub fn sys_tkill(tid: u32, signal: u32) -> i64 {
    match task::process_of_tid(tid) {
        Some(process) => {
            let pid = process.pid;
            send_signal(pid, signal)
        }
        None => -errno::ESRCH,
    }
}

/// Livre au plus un signal en attente avant le retour en ring 3.
///
/// Appele depuis le dispatch d'appel systeme, seul endroit ou la trame ring 3
/// est a la fois accessible et modifiable.
pub fn deliver_pending(frame: &mut TrapFrame) {
    if !task::in_user_task() {
        return;
    }
    loop {
        // `deliver_pending` s'execute a la fin de CHAQUE appel systeme, et
        // `task::current_process()` prend le gros verrou parce qu'il passe par
        // la table des taches. C'etait donc une acquisition par appel systeme,
        // AVANT meme de regarder s'il y a un signal a livrer -- et sur un appel
        // libere, elle annulait a elle seule tout le benefice de la
        // liberation : mesure, 20 123 acquisitions pour 20 000 `getpid`.
        //
        // Le domaine CPU-local rend le meme `Arc` sans rien verrouiller, et
        // `signals` a son propre verrou sur le `Process`.
        let process = super::processus_courant();
        let (signal, action, blocked) = {
            let signals = process.signals.lock();
            match signals.next_deliverable() {
                None => return,
                Some(signal) => (
                    signal,
                    signals.actions[signal as usize - 1],
                    signals.blocked,
                ),
            }
        };
        process.signals.lock().clear(signal);

        if action.handler == SIG_IGN {
            continue;
        }
        if action.handler == SIG_DFL {
            match signal::default_action(signal) {
                signal::DefaultAction::Ignore => continue,
                signal::DefaultAction::Terminate => {
                    crate::serial_println!(
                        "[signal] {} non intercepte : processus termine",
                        signal::name(signal)
                    );
                    task::exit_group(128 + signal as i32);
                }
            }
        }
        // SIGKILL et SIGSTOP ne sont jamais detournables, meme avec un
        // gestionnaire installe (rt_sigaction le refuse deja, ceinture et
        // bretelles).
        if signal == signal::SIGKILL || signal == signal::SIGSTOP {
            task::exit_group(128 + signal as i32);
        }

        if !signal::deliver(frame, signal, &action, blocked) {
            crate::serial_println!("[signal] pile utilisateur inaccessible : processus termine");
            task::exit_group(128 + signal::SIGSEGV as i32);
        }

        {
            let mut signals = process.signals.lock();
            // Le signal livre est bloque pendant l'execution de son
            // gestionnaire (sauf SA_NODEFER), plus ceux demandes par sa_mask :
            // c'est ce qui evite qu'il se reentre indefiniment.
            signals.blocked |= action.mask;
            if action.flags & signal::SA_NODEFER == 0 {
                signals.blocked |= 1 << (signal - 1);
            }
            if action.flags & signal::SA_RESETHAND != 0 {
                signals.actions[signal as usize - 1] = SigAction::default();
            }
        }
        // Un seul signal par retour : le suivant sera livre au retour du
        // gestionnaire courant.
        return;
    }
}
