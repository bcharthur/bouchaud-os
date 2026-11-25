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

/// Ce que `clone` demande au-dela de la creation du fils.
///
/// `fork` (57) et `vfork` (58) n'en portent aucune ; `clone` et `clone3` les
/// transmettent telles quelles.
#[derive(Clone, Copy, Default)]
pub struct DemandeClone {
    pub flags: u64,
    /// Sommet de la pile du fils (0 : celle du pere, ce que fait `vfork`).
    pub pile: u64,
    pub child_tid: u64,
    pub tls: u64,
}

pub const CLONE_VM: u64 = 0x100;
pub const CLONE_VFORK: u64 = 0x4000;
pub const CLONE_SETTLS: u64 = 0x80000;
pub const CLONE_CHILD_CLEARTID: u64 = 0x200000;
pub const CLONE_CHILD_SETTID: u64 = 0x1000000;

/// `fork` : duplique le processus courant.
///
/// La copie de l'espace d'adressage est immediate (voir
/// `AddressSpace::duplicate`). La table de descripteurs est clonee, ce qui
/// partage les objets sous-jacents — tubes, sockets, `eventfd` : c'est
/// exactement ce que POSIX demande, et ce sur quoi repose le chainage
/// `cmd1 | cmd2`.
pub fn sys_fork(frame: &TrapFrame) -> i64 {
    sys_fork_demande(frame, DemandeClone::default())
}

/// `fork` avec les options de `clone` (pile, TLS, `CHILD_SETTID`...).
pub fn sys_fork_demande(frame: &TrapFrame, demande: DemandeClone) -> i64 {
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

    let (space, compte, brk_start, brk, mmap_next, partages, limite_as, promesses, clean_pages, invalidation, regions_natives) = {
        let mut mm = parent.mm.lock();
        let (space, compte, invalidation) = match mm.space.duplicate() {
            Some(resultat) => resultat,
            None => return -errno::ENOMEM,
        };
        (space, compte, mm.brk_start, mm.brk, mm.mmap_next, mm.partages.clone(),
            mm.limite_as, mm.promesses.clone(), mm.clean_pages.clone(), invalidation,
            // Le fils herite des projections natives (memes frames, partagees) :
            // il doit tenir les regions lui aussi.
            mm.regions_natives.clone())
    };
    // BOUCHAUD_COW_V1 : les pages du pere viennent de passer en lecture seule.
    // Un autre fil du pere peut encore en tenir une traduction inscriptible
    // sur un autre coeur : elle doit disparaitre AVANT que le fils ne coure,
    // sinon ses ecritures atterriraient dans une frame desormais partagee.
    if let Some(invalidation) = invalidation {
        invalidation.execute();
    }
    let apres_espace_ns = crate::kernel::timer::monotonic_ns();

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

    let mm = Arc::new(Mm::new(MmState { space, regions_natives, brk_start, brk, mmap_next,
        partages, limite_as, promesses, clean_pages }));
    let (pid, nom_journal) = construit_fils(&parent, mm, None, frame, &demande);

    let fin_ns = crate::kernel::timer::monotonic_ns();
    let kio_copies = compte.pages_copiees.saturating_mul(crate::kernel::vmm::PAGE_SIZE) / 1024;
    FORK_TOTAL_NS.fetch_add(fin_ns.saturating_sub(debut_ns), Ordering::Relaxed);
    FORK_COPIE_NS.fetch_add(apres_espace_ns.saturating_sub(debut_ns), Ordering::Relaxed);
    FORK_PAGES_COPIEES.fetch_add(compte.pages_copiees, Ordering::Relaxed);
    FORK_APPELS.fetch_add(1, Ordering::Relaxed);
    FORK_PIRE_NS.fetch_max(fin_ns.saturating_sub(debut_ns), Ordering::Relaxed);
    crate::kernel::dmesg::log_fmt(format_args!(
        "PERF_FORK t={} pere={} enfant={} image={} duree_us={} copie_us={} references_us={} reste_us={} pages_copiees={} pages_empruntees={} kio_copies={} pages_partagees_cow={}",
        fin_ns / 1_000_000,
        parent.pid,
        pid,
        nom_journal,
        fin_ns.saturating_sub(debut_ns) / 1_000,
        apres_espace_ns.saturating_sub(debut_ns) / 1_000,
        apres_references_ns.saturating_sub(apres_espace_ns) / 1_000,
        fin_ns.saturating_sub(apres_references_ns) / 1_000,
        compte.pages_copiees,
        compte.pages_empruntees,
        kio_copies,
        compte.pages_partagees,
    ));

    pid as i64
}

/// `vfork`, et `clone(CLONE_VM|CLONE_VFORK)` -- donc `posix_spawn`.
///
/// BOUCHAUD_VFORK_REEL_V1. Le fils partage le `Mm` du pere : AUCUNE page
/// n'est recopiee, et l'`execve` qui suit n'a rien a rendre. Le fil appelant
/// du pere est suspendu jusqu'a ce que le fils ait fait `execve` ou soit mort
/// (`LienVfork`). Les autres fils du pere continuent, comme sous Linux.
///
/// Ce chemin remplace un `fork` a copie complete : 11 a 18 ms de recopie,
/// puis 7 a 10 ms de liberation, par processus du navigateur.
pub fn sys_vfork(frame: &TrapFrame, demande: DemandeClone) -> i64 {
    let parent = task::current_process();
    let debut_ns = crate::kernel::timer::monotonic_ns();
    let mm = parent.mm.get();
    let lien = task::LienVfork::nouveau(Arc::clone(&mm));
    let (pid, nom_journal) = construit_fils(&parent, mm, Some(Arc::clone(&lien)), frame, &demande);
    let cree_ns = crate::kernel::timer::monotonic_ns();
    // Le pere ne repart qu'une fois l'emprunt fini : sa pile (vfork) ou celle
    // qu'il a donnee au fils (posix_spawn) est encore en service.
    lien.attend();
    let fin_ns = crate::kernel::timer::monotonic_ns();
    crate::kernel::dmesg::log_fmt(format_args!(
        "PERF_VFORK t={} pere={} enfant={} image={} creation_us={} attente_fils_us={} pages_copiees=0",
        fin_ns / 1_000_000,
        parent.pid,
        pid,
        nom_journal,
        cree_ns.saturating_sub(debut_ns) / 1_000,
        fin_ns.saturating_sub(cree_ns) / 1_000,
    ));
    pid as i64
}

/// Construit et enregistre le processus fils sur `mm`. Rend (pid, nom).
///
/// Commun a `fork` (un `Mm` neuf, copie) et a `vfork` (le `Mm` du pere,
/// partage, avec le lien qui suspend le pere).
fn construit_fils(
    parent: &Arc<Process>,
    mm: Arc<Mm>,
    vfork: Option<Arc<task::LienVfork>>,
    frame: &TrapFrame,
    demande: &DemandeClone,
) -> (u32, String) {
    let files = parent.files.lock().clone();
    let (cwd, uid, gid, name, ecran) = {
        let metadata = parent.metadata.lock();
        (metadata.cwd, metadata.uid, metadata.gid, metadata.name.clone(), metadata.ecran)
    };
    let signals = parent.signals.lock().clone();
    let parent_pid = parent.pid;
    let resource_group_id = parent.resource_group_id;
    let resource_group_name = parent.resource_group_name.clone();

    let pid = crate::kernel::process::spawn(&name, uid as u16);
    let nom_journal = name.clone();
    let mut child_signals = signals;
    // Les signaux en attente ne sont pas herites : ils appartenaient au parent.
    child_signals.pending = 0;
    child_signals.leve_ns = 0;

    let child = Arc::new(Process {
        pid,
        parent: parent_pid,
        resource_group_id,
        resource_group_name,
        mm: task::MmCourant::new(mm),
        vfork: SpinLock::new(vfork),
        files: Arc::new(FileTable::new(files)),
        metadata: SpinLock::new(ProcessMetadata { name, cwd, uid, gid, ecran }),
        espace_rendu: core::sync::atomic::AtomicBool::new(false),
        lifecycle: SpinLock::new(ProcessLifecycle { exit_code: 0, par_signal: false, zombie: false, threads: 1, groupe_en_sortie: false }),
        signals: SpinLock::new(child_signals),
        temps_recycle: task::TempsRecycle::neuf(),
        appels: task::AppelsSyscall::neuf(),
    });
    task::register_process(child.clone());

    // L'enfant reprend a l'instruction suivant le `syscall`, avec 0 dans rax :
    // c'est la seule chose qui distingue les deux retours de `fork`.
    let mut child_frame = *frame;
    child_frame.rax = 0;
    // `clone` avec une pile : le fils y demarre (posix_spawn lui en donne une,
    // puisqu'il partage la memoire du pere ; `vfork` garde celle du pere).
    if demande.pile != 0 {
        child_frame.rsp = demande.pile;
    }
    let mut child_task = Task::new(child.clone(), child_frame);
    // BOUCHAUD_CLONE_CHILD_SETTID_V1 : l'identifiant du FIL fils s'ecrit DANS
    // SON espace, avant qu'il ne tourne. `fork()` de la glibc demande
    // `CLONE_CHILD_SETTID` sur `THREAD_SELF->tid` : sans cette ecriture, le
    // fils gardait le tid copie du pere, et tout ce qui lit ce champ sans
    // appel systeme -- `pthread_kill` depuis un autre fil, proprietaire d'un
    // mutex robuste ou a controle d'erreur -- designait un fil du PERE. C'est
    // le tid (`gettid`), pas le pid : ici les deux espaces sont distincts.
    // Pour un fils vfork, son espace EST celui du pere : c'est aussi ce
    // qu'ecrit Linux. Page absente : rien d'ecrit (le champ de la glibc vit
    // dans une page que le pere a deja touchee, donc copiee).
    if demande.flags & CLONE_CHILD_SETTID != 0 && demande.child_tid != 0 {
        let tid = child_task.tid as u32;
        child.mm.lock().space.write_utilisateur(demande.child_tid, &tid.to_le_bytes());
    }
    // BOUCHAUD_SIGMASQUE_PAR_FIL_V1 : le fils herite du masque du fil qui
    // appelle `fork` (posix_spawn de la glibc : tout bloque, le fils le
    // restaure avant `execve`).
    child_task.masque_signaux.range(task::current().masque_signaux.charge());
    // La base FS doit suivre : c'est le TLS, et la premiere chose que fait la
    // libc dans l'enfant est de lire `%fs:0` pour retrouver sa structure de
    // thread. Une base nulle la ferait dereferencer l'adresse 0.
    child_task.fs_base = if demande.flags & CLONE_SETTLS != 0 {
        demande.tls
    } else {
        usermode::fs_base()
    };
    // `pthread_join` et `fork` de la glibc : a la mort du fils, le noyau
    // remet ce mot a zero et reveille ses attendants.
    if demande.flags & CLONE_CHILD_CLEARTID != 0 {
        child_task.clear_child_tid = demande.child_tid;
    }
    task::register(child_task);
    (pid, nom_journal)
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

/// Tout le travail d'`execve`, jusqu'a la trame de la nouvelle image comprise.
/// REVIENT toujours : voir `sys_execve`.
fn prepare_execve(path_addr: u64, argv_addr: u64, envp_addr: u64) -> Result<TrapFrame, i64> {
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
        None => return Err(-errno::EFAULT),
    };
    let argv = match read_string_array(argv_addr) {
        Some(list) => list,
        None => return Err(-errno::EFAULT),
    };
    let envp = match read_string_array(envp_addr) {
        Some(list) => list,
        None => return Err(-errno::EFAULT),
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
            Some(_) => return Err(-errno::EISDIR),
            None => return Err(-errno::ENOENT),
        }
    };
    let apres_ouverture_ns = crate::kernel::timer::monotonic_ns();
    if elf::parse_node(node).is_err() {
        return Err(-errno::ENOEXEC);
    }
    let apres_entetes_ns = crate::kernel::timer::monotonic_ns();

    let mut space = match vmm::AddressSpace::new() {
        Some(space) => space,
        None => return Err(-errno::ENOMEM),
    };

    let mut new_promises = Vec::new();
    let image = match elf::load_node_lazy(node, vmm::user_load_base(), &mut new_promises) {
        Ok(image) => image,
        Err(_) => return Err(-errno::ENOEXEC),
    };
    let (entry, interp_base) = match image.interp.as_deref() {
        None => (image.entry, 0),
        Some(interp_path) => {
            // load_node_lazy reprend le verrou VFS pour lire l'ELF.
            let interp_node = {
                let fs = crate::fs::ramfs::fs();
                match fs.resolve(interp_path, cwd) {
                    Some(node) if fs.nodes[node].kind == crate::fs::ramfs::NodeKind::File => node,
                    Some(_) => return Err(-errno::ENOEXEC),
                    None => return Err(-errno::ENOENT),
                }
            };
            match elf::load_node_lazy(interp_node, vmm::user_interp_base(), &mut new_promises) {
                Ok(interp) => (interp.entry, interp.base),
                Err(_) => return Err(-errno::ENOEXEC),
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
        Err(_) => return Err(-errno::ENOMEM),
    };
    let apres_pile_ns = crate::kernel::timer::monotonic_ns();

    // All fallible image construction is complete. Only now terminate sibling
    // tasks and retire the old identity: failed execve must leave both intact.
    // BOUCHAUD_GROUPE_RECLAME_V1 : point de non-retour. Un autre fil qui a
    // deja reclame le groupe (exit_group, execve) gagne ; celui-ci se retire.
    if !task::reclame_groupe() {
        task::retire_perdant_du_groupe();
    }
    task::terminate_sibling_threads();
    // BOUCHAUD_CYCLE_DE_VIE_V1 : les freres condamnes meurent d'eux-memes ;
    // l'image n'est remplacee qu'une fois le dernier mort.
    task::attend_extinction_freres();
    // BOUCHAUD_MM_REMPLACABLE_V1 : un `Mm` PARTAGE -- ce processus est un
    // fils vfork, ou un fils vfork s'execute encore sur le `Mm` de ce pere --
    // n'est ni retire ni remplace en place : l'autre processus s'en sert. On
    // installe un `Mm` neuf, et l'ancien vit tant que l'autre le tient.
    let mm_ancien = process.mm.get();
    let partage = process.emprunte_mm() || mm_ancien.est_emprunte();
    let old_identity = mm_ancien.lock().space.identity();
    let self_cpu = smp::cpu_index();
    if !partage {
        // Stop every sibling on the old CR3 before replacement.
        old_identity.begin_retire();
        let active = old_identity.active_cpus() & !(1u64 << self_cpu);
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
    }
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
    // BOUCHAUD_EOF_HORS_CLOSE_V1 : le pere de `posix_spawn` attend la fin de
    // fichier de ce tube CLOEXEC pour savoir que l'exec a reussi.
    let fermes = process.files.lock().close_on_exec();
    crate::kernel::fd::ferme_hors_verrou(fermes);
    process.signals.lock().reset_for_exec();
    {
        // Nouvelle image, un seul fil : le groupe est rendu.
        let mut lifecycle = process.lifecycle.lock();
        lifecycle.threads = 1;
        lifecycle.groupe_en_sortie = false;
    }
    let brk_start = (image.end + 0x10_0000) & !0xFFF;
    let (pages_rendues, apres_bascule_ns) = if partage {
        // BOUCHAUD_VFORK_REEL_V1 : rien a rendre, rien a attendre. Le nouvel
        // espace va dans un `Mm` neuf ; l'ancien reste celui du pere.
        let limite_as = mm_ancien.lock().limite_as;
        let neuf = Arc::new(Mm::new(MmState {
            space,
            regions_natives: Vec::new(),
            brk_start,
            brk: brk_start,
            mmap_next: vmm::user_mmap_base(),
            partages: Vec::new(),
            limite_as,
            promesses: new_promises,
            clean_pages: Vec::new(),
        }));
        // Case, CR3 et marque d'activite basculent sans qu'une commutation
        // puisse s'intercaler : une preemption entre `remplace` et `activate`
        // marquerait inactif le NOUVEL espace et laisserait l'ancien marque
        // actif sur ce coeur.
        x86_64::instructions::interrupts::without_interrupts(|| {
            drop(process.mm.remplace(Arc::clone(&neuf)));
            neuf.activate();
            old_identity.mark_inactive(self_cpu);
        });
        // Le pere peut repartir : le fils ne touche plus a sa memoire.
        process.libere_vfork(task::FinEmprunt::Execve);
        (0, crate::kernel::timer::monotonic_ns())
    } else {
        let mut regions_anciennes = Vec::new();
        let (old, old_clean, old_shared) = {
            let mut mm = process.mm.lock();
            mm.brk_start = brk_start;
            mm.brk = mm.brk_start;
            mm.mmap_next = vmm::user_mmap_base();
            // L'ancien espace disparait avec ses mappages : les references qu'il
            // tenait sur le cache partage doivent partir avec lui.
            let old_shared = core::mem::take(&mut mm.partages);
            let old_clean = core::mem::take(&mut mm.clean_pages);
            // Les regions natives de l'ancienne image sont rendues APRES la
            // destruction de l'ancien espace (plus aucune PTE ne les vise).
            regions_anciennes = core::mem::take(&mut mm.regions_natives);
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
        // BOUCHAUD_BARRIERE_CR3_V1 : meme fenetre que pour un mort -- un
        // frere qui vient d'effacer son bit peut encore executer sous
        // l'ancienne PML4 jusqu'a son prochain CR3.
        old_identity.barriere_cr3();
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
        drop(regions_anciennes);
        (pages_rendues, apres_bascule_ns)
    };
    drop(mm_ancien);
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
    drop(task);
    Ok(frame)
}

/// `execve` : remplace l'image du processus courant.
///
/// Ne revient jamais en cas de succes -- le processus repart au point d'entree
/// du nouveau programme.
///
/// BOUCHAUD_SORTIE_SANS_REFERENCE_V1 : `resume_usermode` ne revient JAMAIS. La
/// pile de cet appel est abandonnee sans executer un seul destructeur : tout ce
/// qui y vivait encore etait perdu a chaque `execve` reussi -- l'`Arc<Process>`
/// du processus (donc, a sa mort, TOUT son espace d'adressage), l'identite de
/// l'ancien espace, le chemin, `argv` et `envp`. Mesure (tempete-exec-probe,
/// TCG SMP4) : 7 700 processus crees, 3 544 detruits -- exactement ceux qui
/// n'avaient pas fait `execve` ; la RAM pleine en 2 000 cycles fork+exec.
/// Tout le travail se fait donc dans `prepare_execve`, qui REVIENT et rend
/// tout ; ici ne vit plus qu'une trame `Copy` au moment de sauter.
pub fn sys_execve(path_addr: u64, argv_addr: u64, envp_addr: u64) -> i64 {
    let frame = match prepare_execve(path_addr, argv_addr, envp_addr) {
        Ok(frame) => frame,
        Err(erreur) => return erreur,
    };
    // Le garde de tache est deja rendu (`prepare_execve`) : le conserver
    // laisserait l'emplacement marque exclusif pour toujours.
    //
    // Cette pile ne reprendra jamais : la sonde syscall est fermee ici, avant
    // l'iretq. Le chemin no-return abandonnait aussi le gros verrou et une
    // portee de domaine ; ni l'un ni l'autre n'existent plus
    // (BOUCHAUD_BKL_SUPPRIME_V1, qui remplace BOUCHAUD_EXECVE_SANS_BKL_V1).
    task::stall_site_clear();
    task::stall_syscall_exit();
    task::account_resume_user_noreturn();

    unsafe { usermode::resume_usermode(&frame) }
}

/// Le statut que rend `wait4` (BOUCHAUD_STATUT_SORTIE_V1) : `code << 8` pour
/// une sortie, le signal seul pour une mort par signal (`code` = 128 + signal).
pub fn statut_wait(code: i32, par_signal: bool) -> u32 {
    if par_signal {
        (code as u32).wrapping_sub(128) & 0x7F
    } else {
        ((code as u32) & 0xFF) << 8
    }
}

/// `wait4` : attend la fin d'un processus fils.
pub fn sys_wait4(pid: i64, status_addr: u64, options: u32, _rusage: u64) -> i64 {
    const WNOHANG: u32 = 1;
    let parent_pid = task::current_process().pid;

    loop {
        let zombies = task::zombie_children(parent_pid);
        let found = zombies
            .iter()
            .find(|(child, _, _)| pid <= 0 || *child == pid as u32)
            .copied();

        if let Some((child_pid, code, par_signal)) = found {
            // BOUCHAUD_WAIT4_RECOLTE_UNIQUE_V1 : seul le fil qui retire le zombie
            // le rend ; un autre fil du meme parent l'a peut-etre deja recolte.
            if !task::collect_child(child_pid) {
                continue;
            }
            if status_addr != 0 {
                // Encodage `wait` : octet de poids faible = signal, octet
                // suivant = code de sortie. BOUCHAUD_STATUT_SORTIE_V1 : la
                // mort par signal se lit sur `par_signal`, pas sur le code --
                // un `exit(130)` n'est pas un SIGINT, ni `exit(128)` un succes.
                let status = statut_wait(code, par_signal);
                user_write_u32(status_addr, status);
            }
            return child_pid as i64;
        }

        if !task::has_children(parent_pid) {
            return -errno::ECHILD;
        }
        if options & WNOHANG != 0 {
            return 0;
        }

        // BOUCHAUD_WAIT4_PUBLIE_PUIS_RELIT_V1
        //
        // L'ordre etait : chercher les zombies (au-dessus), PUIS se declarer en
        // attente. Un fils qui mourait entre les deux trouvait
        // `waiting_for_child` a faux, ne reveillait personne, et le parent
        // dormait pour toujours sur un zombie (MESURE_DEMARRAGE_A_FROID §15).
        // Tant que `wait4` et `exit` passaient tous deux par le gros verrou,
        // la fenetre etait fermee par serialisation ; le lot B8 l'a rouverte,
        // et `wait4-course-probe` l'a attrapee (B11 : un parent fige,
        // `zombies=4`, coeurs au repos).
        //
        // Meme motif croise que la WaitQueue : on PUBLIE l'attente, barriere,
        // puis on RELIT les zombies. Le fils, lui, pose `zombie` sous
        // `lifecycle` puis fait un CAS sur `waiting_for_child` (barriere
        // pleine). Au moins l'un des deux voit l'autre : ou bien le fils
        // trouve l'attente et nous remet pret, ou bien nous trouvons son
        // zombie et annulons le parking.
        task::current().waiting_for_child.range(true);
        // BOUCHAUD_CYCLE_DE_VIE_V1 : attente interruptible. `Ready -> Blocked`
        // par CAS ; une tache condamnee y meurt au lieu d'y dormir.
        if !task::publie_attente_interruptible() {
            let _ = task::current().waiting_for_child.compare_exchange(true, false);
            return -errno::EINTR;
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        let deja_la = task::zombie_children(parent_pid)
            .iter()
            .any(|(child, _, _)| pid <= 0 || *child == pid as u32);
        if deja_la || !task::has_children(parent_pid) {
            // Annule le parking. Si le fils a gagne le CAS entre-temps, il
            // nous remet pret lui-meme ; dans les deux cas on reboucle.
            let _ = task::current().waiting_for_child.compare_exchange(true, false);
            task::termine_attente();
            continue;
        }
        let _ = task::schedule();
        task::current().waiting_for_child.range(false);
        task::termine_attente();
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

    // BOUCHAUD_SIGNAUX_UNE_SEULE_PRISE_V1 : l'ancienne action rendue doit etre
    // celle que la nouvelle REMPLACE. Lecture et remplacement sous une seule
    // prise de `signals` ; lecture et ecriture utilisateur hors du verrou.
    let action = if act != 0 {
        match signal::read_sigaction(act) {
            Some(action) => Some(action),
            None => return -errno::EFAULT,
        }
    } else {
        None
    };
    let previous = {
        let mut signals = process.signals.lock();
        let previous = signals.actions[index];
        if let Some(action) = action {
            signals.actions[index] = action;
        }
        previous
    };
    if oldact != 0 && !signal::write_sigaction(oldact, &previous) {
        return -errno::EFAULT;
    }
    0
}

/// `rt_sigprocmask`.
pub fn sys_rt_sigprocmask(how: i32, set: u64, oldset: u64) -> i64 {
    // BOUCHAUD_SIGNAUX_UNE_SEULE_PRISE_V1 : l'ancien masque etait lu sous une
    // prise de `signals`, le nouveau ecrit sous une autre, calcule a partir de
    // la copie perimee. Deux fils du processus bloquant des signaux differents
    // en meme temps perdaient une des deux mises a jour -- le gros verrou de
    // l'aiguilleur masquait la fenetre. Lecture utilisateur AVANT, calcul et
    // ecriture sous UNE prise, ecriture utilisateur APRES (aucune faute de
    // page sous le verrou).
    //
    // BOUCHAUD_SIGMASQUE_PAR_FIL_V1 : le masque est celui du FIL appelant.
    // Un masque de processus faisait d'un « bloquer tout puis restaurer »
    // de la glibc (pthread_create, posix_spawn) une ecriture sur le masque
    // des AUTRES fils : restaurer le masque transitoire d'un voisin laissait
    // SIGCHLD bloque pour toujours (sigmasque-fil-probe, cas course). Seul
    // ce fil ecrit son masque : aucun verrou.
    let fil = task::current();
    let new = if set != 0 {
        match user_read_u64(set) {
            Some(value) => Some(value),
            None => return -errno::EFAULT,
        }
    } else {
        None
    };
    let current_mask = fil.masque_signaux.charge();
    if let Some(new) = new {
        let masque = match how {
            signal::SIG_BLOCK => current_mask | new,
            signal::SIG_UNBLOCK => current_mask & !new,
            signal::SIG_SETMASK => new,
            _ => return -errno::EINVAL,
        };
        fil.masque_signaux.range(signal::SignalState::masque_permis(masque));
    }
    if oldset != 0 && !user_write(oldset, &current_mask.to_le_bytes()) {
        return -errno::EFAULT;
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
            task::current().masque_signaux.range(signal::SignalState::masque_permis(mask));
            frame.rax as i64
        }
        None => {
            // Trame illisible : le gestionnaire a corrompu sa pile, il n'y a
            // aucun etat coherent ou revenir.
            task::exit_group_par_signal(signal::SIGSEGV as i32)
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
            // sur un processus bloque resterait sans effet -- y compris en
            // `poll` (BOUCHAUD_SIGNAL_INTERROMPT_POLL_V1).
            task::reveille_pour_signal(pid);
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
        // `task::current_process()` prenait alors le gros verrou parce qu'il
        // passait par la table des taches. C'etait donc une acquisition par appel systeme,
        // AVANT meme de regarder s'il y a un signal a livrer -- et sur un appel
        // libere, elle annulait a elle seule tout le benefice de la
        // liberation : mesure, 20 123 acquisitions pour 20 000 `getpid`.
        //
        // Le domaine CPU-local rend le meme `Arc` sans rien verrouiller, et
        // `signals` a son propre verrou sur le `Process`.
        let process = super::processus_courant();
        // BOUCHAUD_SIGNAL_FIL_PRINCIPAL_V1 : comme Linux, un signal de
        // processus revient au fil principal quand il peut le prendre.
        // Le verrou `signals` est rendu AVANT de parcourir le registre des
        // taches (`signal_a_effet_en_attente` le rend en sortant) : ailleurs
        // l'ordre est registre puis signals (`notify_parent_of_exit_for`).
        // Un signal IGNORE n'a rien a attendre : n'importe quel fil le
        // consomme. Seul celui qui fera quelque chose est laisse.
        if task::signal_a_effet_en_attente() && task::laisse_le_signal_au_fil_principal() {
            return;
        }
        // BOUCHAUD_SIGMASQUE_PAR_FIL_V1 : le masque du fil qui livre.
        let fil = task::current();
        let blocked = fil.masque_signaux.charge();
        let (signal, action) = {
            let signals = process.signals.lock();
            match signals.next_deliverable(blocked) {
                None => return,
                Some(signal) => (signal, signals.actions[signal as usize - 1]),
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
                    // BOUCHAUD_SORTIE_SANS_REFERENCE_V1 : `exit_group_par_signal`
                    // ne revient pas ; `process` ne serait jamais rendu.
                    drop(process);
                    task::exit_group_par_signal(signal as i32);
                }
            }
        }
        // SIGKILL et SIGSTOP ne sont jamais detournables, meme avec un
        // gestionnaire installe (rt_sigaction le refuse deja, ceinture et
        // bretelles).
        if signal == signal::SIGKILL || signal == signal::SIGSTOP {
            drop(process);
            task::exit_group_par_signal(signal as i32);
        }

        if !signal::deliver(frame, signal, &action, blocked) {
            crate::serial_println!("[signal] pile utilisateur inaccessible : processus termine");
            drop(process);
            task::exit_group_par_signal(signal::SIGSEGV as i32);
        }

        {
            // Le signal livre est bloque pendant l'execution de son
            // gestionnaire (sauf SA_NODEFER), plus ceux demandes par sa_mask :
            // c'est ce qui evite qu'il se reentre indefiniment. Sur CE fil
            // seulement : les autres fils peuvent prendre le suivant.
            let mut masque = blocked | action.mask;
            if action.flags & signal::SA_NODEFER == 0 {
                masque |= 1 << (signal - 1);
            }
            fil.masque_signaux.range(signal::SignalState::masque_permis(masque));
            if action.flags & signal::SA_RESETHAND != 0 {
                process.signals.lock().actions[signal as usize - 1] = SigAction::default();
            }
        }
        // Un seul signal par retour : le suivant sera livre au retour du
        // gestionnaire courant.
        return;
    }
}
