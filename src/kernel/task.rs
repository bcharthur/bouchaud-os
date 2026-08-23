// BOUCHAUD_SMP4_DEADLOCK_FIX
//! Taches utilisateur : threads, changement de contexte, futex.
//!
//! Un **processus** ([`Process`]) possede un espace d'adressage, une table de
//! descripteurs, un `brk` et une zone `mmap`. Une **tache** ([`Task`]) est un
//! fil d'execution : c'est l'unite ordonnancee. `clone(CLONE_THREAD)` cree une
//! tache de plus dans le meme processus, exactement comme sous Linux — c'est ce
//! dont `pthread_create` (donc Qt, donc Python) a besoin.
//!
//! ## Deux piles par tache
//!
//! - la **pile utilisateur**, dans l'espace d'adressage du processus ;
//! - la **pile noyau**, privee, sur laquelle s'executent ses appels systeme.
//!   C'est elle qui rend le blocage possible : quand une tache s'endort dans un
//!   `futex`, son etat noyau reste sur sa propre pile pendant qu'une autre tache
//!   utilise la sienne.
//!
//! ## Ou l'ordonnanceur peut-il commuter ?
//!
//! - a un point de blocage volontaire (`futex`, `nanosleep`, `sched_yield`,
//!   lecture bloquante) ;
//! - sur IRQ0 **uniquement si le timer a interrompu du code ring 3**
//!   ([`preempt_from_irq`]).
//!
//! Le noyau lui-meme n'est jamais preempte : il n'est pas reentrant (son
//! allocateur et ses pilotes prennent des verrous tournants), et le preempter
//! provoquerait des interblocages sur un CPU unique. Une tache utilisateur, en
//! revanche, ne detient aucun verrou noyau : la preempter est sans risque.
//!
//! ## Ce qu'une commutation doit emporter
//!
//! Ces deux chemins n'arrivent pas dans le meme etat de processeur : le premier
//! interruptions actives, le second interruptions coupees par la porte d'IRQ.
//! RFLAGS fait donc partie du contexte a sauvegarder au meme titre que les
//! registres callee-saved — voir [`switch_context`], qui explique ce que coutait
//! son oubli. L'invariant qui en decoule, verifie par [`schedule`] en
//! compilation de debogage : **on ne commute jamais interruptions coupees**, et
//! toute attente passe par [`cpu::wait_for_interrupt`] plutot que par un `hlt`
//! nu.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::arch::x86_64::{cpu, smp};
use crate::arch::x86_64::usermode::{self, TrapFrame};
use crate::kernel::fd::FdTable;
use crate::kernel::smp_lock;
use crate::kernel::vmm::AddressSpace;
pub use crate::kernel::vma::{Backing as PromesseBacking, Vma as Promesse};

/// Taille de la pile noyau d'une tache (64 KiB).
const KSTACK_SIZE: usize = 64 * 1024;

/// Classe d'ordonnancement d'une tache.
///
/// Deux, pas davantage. L'audit OS avait identifie l'absence de priorites comme
/// le dernier manque avant un processus de rendu separe : sur un cœur unique et
/// un tourniquet strict, sortir le rendu d'un processus n'empeche pas une page
/// lourde de rendre l'interface lente, parce que rien ne favorise l'interface.
///
/// Ce qu'il fallait n'etait pas un ordonnanceur different — le tourniquet
/// convient — mais un moyen de dire lequel des deux compte quand les deux sont
/// prets. Deux classes suffisent a le dire, et une troisieme n'ajouterait que
/// des questions sans reponse.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Priorite {
    /// Ce qui repond a l'utilisateur : l'interface du navigateur, le serveur
    /// graphique. Servi en premier quand plusieurs taches sont pretes.
    Interactive,
    /// Tout le reste : calcul, rendu, travail de fond. Jamais affame.
    Normale,
}

/// Nombre maximal de tours consecutifs accordes aux taches interactives.
///
/// Sans cette borne, une tache interactive qui calcule sans jamais se bloquer
/// affamerait tout le reste — et « l'interface reste fluide » deviendrait
/// « rien d'autre ne tourne ». Au-dela du compte, le tourniquet reprend ses
/// droits pour un tour, ce qui garantit une progression a toute tache prete.
///
/// Quatre : l'interface conserve des rafales courtes, mais une tache normale
/// recupere au moins un tour sur cinq sous pression interactive continue.
/// C'est volontairement plus favorable a WebContent et aux workers CPU.
const TOURS_INTERACTIFS_MAX: u32 = 4;


/// Etat d'ordonnancement d'une tache.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum TaskState {
    /// Prete a s'executer (ou en cours).
    Ready,
    /// En attente d'un evenement (futex, sommeil, entree).
    Blocked,
    /// Terminee, en attente de nettoyage.
    Zombie,
}

/// Contexte noyau sauvegarde lors d'un changement de tache.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Context {
    /// Sommet de pile noyau sauvegarde (tout le reste y est empile).
    pub rsp: u64,
}

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
static mut FAULTS_ZERO: u64 = 0;
static mut FAULTS_FILE: u64 = 0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum FaultPageState {
    Missing,
    Loading,
    Present,
    Failed,
}

struct FaultPage {
    pml4: u64,
    page: u64,
    state: FaultPageState,
    waiters: crate::kernel::sync::WaitQueue,
}

static mut FAULT_PAGES: Option<Vec<Box<FaultPage>>> = None;

fn fault_page(pml4: u64, page: u64) -> &'static mut FaultPage {
    debug_assert!(smp_lock::held_by_current_cpu());
    unsafe {
        let pages = FAULT_PAGES.get_or_insert_with(Vec::new);
        if let Some(index) = pages
            .iter()
            .position(|entry| entry.pml4 == pml4 && entry.page == page)
        {
            return &mut *pages[index];
        }
        pages.push(Box::new(FaultPage {
            pml4,
            page,
            state: FaultPageState::Missing,
            waiters: crate::kernel::sync::WaitQueue::new(),
        }));
        &mut *pages.last_mut().unwrap()
    }
}

/// Peuple a la demande une page promise.
///
/// La derniere promesse couvrant la page fixe les droits. Toutes les promesses
/// file-backed couvrant cette meme page contribuent ensuite leurs octets : deux
/// PT_LOAD ELF peuvent legalement partager une page de bordure.
pub fn peuple_a_la_demande(adresse: u64, protection_fault: bool) -> bool {
    if !crate::kernel::vmm::is_user_addr(adresse) || !in_user_task() {
        return false;
    }

    // Une faute de protection n'est jamais transformee en demand paging.
    if protection_fault {
        return false;
    }

    let page = adresse & !(crate::kernel::vmm::PAGE_SIZE - 1);
    loop {
        let record = fault_page(crate::kernel::vmm::current_pml4(), page);
        match record.state {
            FaultPageState::Loading => {
                let ticket = record.waiters.ticket();
                record.waiters.wait(ticket);
                continue;
            }
            FaultPageState::Present => {
                if current_process().borrow_mut().space.translate(page).is_some() {
                    return true;
                }
                // munmap a pu retirer la PTE depuis le dernier fault.
                record.state = FaultPageState::Loading;
                break;
            }
            FaultPageState::Missing => {
                record.state = FaultPageState::Loading;
                break;
            }
            FaultPageState::Failed => return false,
        }
    }

    let result = peuple_page_loader(adresse);
    let record = fault_page(crate::kernel::vmm::current_pml4(), page);
    record.state = if result {
        FaultPageState::Present
    } else {
        FaultPageState::Failed
    };
    record.waiters.wake_all();
    result
}

fn peuple_page_loader(adresse: u64) -> bool {

    stall_pf_phase(210, adresse);
    let processus = current_process();
    stall_pf_phase(211, adresse);
    let mut p = processus.borrow_mut();
    let page = adresse & !(crate::kernel::vmm::PAGE_SIZE - 1);
    stall_pf_phase(212, page);

    let effective = match crate::kernel::vma::trouve(&p.promesses, page) {
        Some(region) => region,
        None => return false,
    };

    if effective.drapeaux & crate::kernel::vmm::PTE_USER == 0 {
        return false;
    }

    if p.space.translate(page).is_some() {
        // Un autre CPU du meme processus a pu materialiser la page pendant que
        // cette faute attendait le BKL. Dans ce cas le fault est deja resolu.
        // Une faute de protection, elle, reste fatale et ne doit pas boucler.
        return true;
    }

    match effective.backing {
        PromesseBacking::Zero => {
            stall_pf_phase(220, page);
            if !p.space.map_alloc(
                page,
                crate::kernel::vmm::PAGE_SIZE,
                effective.drapeaux,
            ) {
                return false;
            }
            unsafe {
                FAULTS_ZERO = FAULTS_ZERO.saturating_add(1);
            }
            stall_pf_phase(229, page);
            true
        }

        PromesseBacking::Framebuffer {
            phys_base,
            mapping_start,
            phys_offset,
        } => {
            let delta = page.saturating_sub(mapping_start);
            let phys = phys_base
                .saturating_add(phys_offset)
                .saturating_add(delta);
            p.space.map_foreign(page, phys, effective.drapeaux)
        }

        PromesseBacking::SharedFile {
            node,
            mapping_start,
            file_offset,
            ..
        } => {
            stall_pf_phase(230, page);
            let source =
                file_offset.saturating_add(page.saturating_sub(mapping_start));
            let numero = source / crate::kernel::vmm::PAGE_SIZE;
            let frame = match crate::kernel::partage::page(node, numero) {
                Some(frame) => frame,
                None => return false,
            };
            if !p.space.map_foreign(page, frame, effective.drapeaux) {
                return false;
            }
            unsafe {
                FAULTS_FILE = FAULTS_FILE.saturating_add(1);
            }
            stall_pf_phase(239, page);
            true
        }

        PromesseBacking::File { .. } => {
            stall_pf_phase(240, page);
            if !p.space.map_alloc(
                page,
                crate::kernel::vmm::PAGE_SIZE,
                effective.drapeaux,
            ) {
                return false;
            }

            // Plusieurs PT_LOAD peuvent partager une page. On compose tous les
            // fragments file-backed qui la couvrent.
            stall_pf_phase(241, page);
            let count = p.promesses.len();
            for index in 0..count {
                let region = p.promesses[index];
                if page < region.debut || page >= region.fin {
                    continue;
                }

                let (node, mapping_start, file_offset, file_size) =
                    match region.backing {
                        PromesseBacking::File {
                            node,
                            mapping_start,
                            file_offset,
                            file_size,
                        } => (node, mapping_start, file_offset, file_size),
                        _ => continue,
                    };

                let page_end = page + crate::kernel::vmm::PAGE_SIZE;
                let data_start = mapping_start;
                let data_end = mapping_start.saturating_add(file_size);
                let start = core::cmp::max(page, data_start);
                let end = core::cmp::min(page_end, data_end);
                if end <= start {
                    continue;
                }

                let wanted = (end - start) as usize;
                let mut buffer =
                    [0u8; crate::kernel::vmm::PAGE_SIZE as usize];
                let source_offset = file_offset
                    .saturating_add(start.saturating_sub(mapping_start));
                stall_pf_file_begin(source_offset);
                let got = crate::fs::backing::read_at(
                    node,
                    source_offset as usize,
                    &mut buffer[..wanted],
                );
                stall_pf_file_done(got, wanted);

                stall_pf_phase(244, start);
                if got != wanted || !p.space.write(start, &buffer[..got]) {
                    let retirement = p
                        .space
                        .prepare_unmap(page, crate::kernel::vmm::PAGE_SIZE);
                    drop(p);
                    debug_assert!(
                        processus.try_borrow_mut().is_ok(),
                        "page fault cleanup: Process encore emprunte"
                    );
                    let depth = smp_lock::suspend_for_schedule();
                    retirement.invalidation().execute();
                    smp_lock::resume_after_schedule(depth);
                    processus.borrow_mut().space.finish_unmap(retirement);
                    return false;
                }
            }

            stall_pf_phase(249, page);
            unsafe {
                FAULTS_FILE = FAULTS_FILE.saturating_add(1);
                if FAULTS_FILE == 1 {
                    crate::kernel::dmesg::log_fmt(format_args!(
                        "memfabric: premier fault fichier page={:#x}",
                        page
                    ));
                }
            }
            true
        }
    }
}

pub fn demand_fault_stats() -> (u64, u64) {
    unsafe { (FAULTS_ZERO, FAULTS_FILE) }
}

/// Diagnostic d'une faute que la Memory Fabric n'a pas pu servir.
pub fn log_fault_mapping(adresse: u64) {
    if !in_user_task() {
        return;
    }

    let process = current_process();
    let mut p = process.borrow_mut();
    let page = adresse & !(crate::kernel::vmm::PAGE_SIZE - 1);
    let present = p.space.translate(page).is_some();
    let writable = p.space.writable(page);

    if let Some(region) = crate::kernel::vma::trouve(&p.promesses, page) {
        crate::println!(
            "[memfabric] FAULT_FATAL pid={} app={} addr={:#x} page={:#x} vma={:#x}..{:#x} backing={} flags={:#x} present={} writable={}",
            p.pid,
            p.name,
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
            p.pid,
            p.name,
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




pub struct Process {
    pub pid: u32,
    /// PID du parent (0 pour le processus lance depuis le shell).
    pub parent: u32,
    pub name: String,
    pub space: AddressSpace,
    pub files: FdTable,
    /// Debut et sommet courant du tas `brk`.
    pub brk_start: u64,
    pub brk: u64,
    /// Prochaine adresse libre pour `mmap`.
    pub mmap_next: u64,
    /// Repertoire courant (index de nœud RAMFS).
    pub cwd: usize,
    /// Code de sortie renseigne par `exit_group`.
    pub exit_code: i32,
    /// Le processus est termine et attend d'etre recolte par son parent.
    pub zombie: bool,
    /// Nombre de threads encore vivants.
    pub threads: usize,
    /// uid/gid vus par le programme.
    pub uid: u32,
    pub gid: u32,
    /// Gestionnaires et masques de signaux.
    pub signals: crate::kernel::signal::SignalState,
    /// Plages `MAP_SHARED` vivantes, chacune tenant une reference sur le cache
    /// de pages partage.
    pub partages: Vec<Partage>,
    /// Taille maximale de l'espace d'adressage (`RLIMIT_AS`), 0 = illimite.
    pub limite_as: u64,
    /// Plages promises mais pas encore peuplees : la pagination a la demande.
    ///
    /// Un `mmap` en `MAP_NORESERVE` demande une plage utilisable **sans**
    /// engager la memoire d'avance. C'est ainsi que mimalloc — l'allocateur
    /// d'AK, donc de tout Ladybird — prend ses arenes : **un gibioctet a la
    /// fois**, en lecture-ecriture, dont il ne touchera qu'une fraction.
    ///
    /// Les peupler a l'appel epuisait la machine en deux arenes. Les refuser
    /// aurait fait echouer l'allocateur. La seule reponse juste est celle de
    /// Linux : noter la promesse, et n'allouer chaque page qu'a son premier
    /// acces — c'est ce que fait `crate::kernel::vmm::peuple_a_la_demande`,
    /// appele depuis le gestionnaire de faute de page.
    pub promesses: Vec<Promesse>,
    /// Ecran virtuel : `/dev/fb0` de ce processus designe cette surface
    /// partagee, et non le framebuffer physique.
    ///
    /// C'est ce qui fait du navigateur une application parmi d'autres plutot
    /// qu'un programme qui prend l'ecran. Il ouvre `/dev/fb0`, l'interroge, le
    /// projette — tout son code reste celui d'un client framebuffer Linux
    /// ordinaire — et ce qu'il obtient est la surface que le gestionnaire de
    /// fenetres compose dans sa fenetre. La regle « le WM est seul proprietaire
    /// du framebuffer physique » est donc tenue par le noyau, pas par la bonne
    /// volonte du client.
    pub ecran: Option<EcranVirtuel>,
}

/// Redirection de `/dev/fb0` vers une surface partagee.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EcranVirtuel {
    /// Nœud RAMFS anonyme qui porte les pixels.
    pub node: usize,
    pub largeur: u32,
    pub hauteur: u32,
    /// Octets par ligne.
    pub pas: u32,
}

/// Une plage `MAP_SHARED` projetee dans un processus.
///
/// Elle existe pour une seule raison : savoir quelle reference rendre au cache
/// de pages quand la plage disparait — par `munmap`, par `execve` ou avec le
/// processus. Sans cette trace, `munmap` ne saurait pas quel nœud il vient de
/// lacher, et les frames resteraient allouees pour toujours.
#[derive(Clone, Copy)]
pub struct Partage {
    pub base: u64,
    pub length: u64,
    pub node: usize,
}

impl Process {
    /// Le nœud partage qui couvre cette adresse, s'il y en a un.
    pub fn partage_a(&self, addr: u64) -> Option<usize> {
        self.partages
            .iter()
            .find(|p| addr >= p.base && addr < p.base + p.length)
            .map(|p| p.node)
    }

    /// Retire les plages entierement couvertes par `[addr, addr+len)` et rend
    /// les nœuds dont la reference est a relacher.
    ///
    /// Un recouvrement **partiel** ne relache rien : la plage reste inscrite
    /// telle quelle. C'est deliberement conservateur. Un `munmap` de la moitie
    /// d'une surface partagee est assez rare pour qu'une fuite bornee soit
    /// preferable a la seule autre erreur possible ici — liberer une frame
    /// qu'un mappage vivant designe encore.
    pub fn retire_partages(&mut self, addr: u64, len: u64) -> Vec<usize> {
        let fin = addr + len;
        let mut rendus = Vec::new();
        self.partages.retain(|p| {
            if addr <= p.base && p.base + p.length <= fin {
                rendus.push(p.node);
                false
            } else {
                true
            }
        });
        rendus
    }

    /// Relache toutes les plages partagees (`execve`, fin du processus).
    pub fn relache_partages(&mut self) {
        let nœuds: Vec<usize> = self.partages.iter().map(|p| p.node).collect();
        self.partages.clear();
        for node in nœuds {
            crate::kernel::partage::demappe(node);
        }
    }

    /// Taille actuelle de l'espace d'adressage, en octets.
    ///
    /// Ce noyau n'a pas d'allocation paresseuse : `mmap` et `brk` mappent
    /// immediatement ce qu'ils promettent. Taille virtuelle et taille residente
    /// coincident donc, et compter les pages possedees est une mesure exacte de
    /// ce que `RLIMIT_AS` borne.
pub fn taille_as(&self) -> u64 {
        let virtuel =
            crate::kernel::vma::octets_virtuels(&self.promesses);
        let resident = self.space.mapped_pages() as u64
            * crate::kernel::vmm::PAGE_SIZE;
        core::cmp::max(virtuel, resident)
    }

    /// Cette croissance tiendrait-elle sous `RLIMIT_AS` ?
    pub fn tient_sous_limite(&self, croissance: u64) -> bool {
        self.limite_as == 0 || self.taille_as() + croissance <= self.limite_as
    }
}

impl Drop for Process {
    /// Un processus qui disparait rend ses references sur le cache partage.
    ///
    /// C'est le filet : `munmap` couvre le cas ordinaire, celui-ci couvre la
    /// mort brutale — un `SIGKILL`, une faute de page fatale, un `exit` sans
    /// menage. Sans lui, tuer un renderer suffirait a faire fuir ses surfaces.
    fn drop(&mut self) {
        self.relache_partages();
    }
}

/// Un fil d'execution utilisateur.
pub struct Task {
    pub tid: u32,
    pub process: Rc<RefCell<Process>>,
    pub state: TaskState,
    /// Classe d'ordonnancement. Voir [`Priorite`].
    pub priorite: Priorite,
    // BOUCHAUD_SMP_NG2_THREAD_BALANCER_TLB_V1
    /// Masque d'affinite par THREAD. Les taches user naissent sur tous les CPU
    /// online; les fils noyau restent CPU0.
    pub affinity_mask: u64,
    /// Proprietaire logique de la runqueue quand la tache est Ready.
    pub runq_cpu: u8,
    /// Dernier CPU sur lequel la tache a reellement execute.
    pub last_cpu: u8,
    /// CPU qui execute actuellement cette tache, -1 si elle est en runqueue.
    pub on_cpu: i8,
    /// Derniere migration effective, pour imposer une residence cache minimale.
    pub last_migration_ns: u64,
    /// Runtime recent lisse, utilise comme estimation du poids de la tache.
    pub recent_runtime_ns: u64,
    /// Debut de la tranche courante.
    pub slice_start_ns: u64,
    pub last_account_ns: u64,
    pub user_cpu_ns: u64,
    pub kernel_cpu_ns: u64,
    pub cpu_ns: [u64; MAX_CPUS],
    pub in_kernel: bool,
    pub context_switches: u64,
    pub migrations: u64,
    /// Etat ring 3 quand la tache n'est pas en cours d'execution.
    pub frame: TrapFrame,
    /// Contexte noyau (pile) pour le changement de tache.
    pub ctx: Context,
    /// Pile noyau privee.
    kstack: Vec<u8>,
    pub kstack_top: u64,
    /// Zone `fxsave` (512 octets alignes 16) pour l'etat FPU/SSE.
    fpu: Vec<u8>,
    fpu_area: u64,
    /// Base FS (TLS de la libc) propre au thread.
    pub fs_base: u64,
    /// Adresse ecrite a la mort du thread (`set_tid_address`), pour pthread_join.
    pub clear_child_tid: u64,
    /// Cle du futex attendu, si la tache est bloquee dessus.
    pub futex_key: u64,
    /// Identite d'une WaitQueue noyau, 0 lorsqu'aucune attente n'est armee.
    pub wait_queue_key: usize,
    /// Deadline monotone en nanosecondes (0 = pas de sommeil).
    pub wake_deadline_ns: u64,
    /// La tache attend la fin d'un processus fils (`wait4`).
    pub waiting_for_child: bool,
    /// La tache n'a pas encore rejoint le ring 3.
    pub fresh: bool,
    /// Ticks du timer pendant lesquels cette tache avait la main.
    ///
    /// C'est un profileur par echantillonnage, et le plus simple qui soit : a
    /// chaque IRQ0 — mille fois par seconde — on incremente le compteur de la
    /// tache courante. Sur mille echantillons, la proportion est le temps
    /// processeur, a la precision du tick pres. Cela ne coute qu'une addition
    /// dans un gestionnaire d'interruption qui existe deja, et cela repond a la
    /// seule question qu'on se pose devant une machine lente : qui consomme.
    pub ticks_cpu: u64,
    /// Fil noyau : ne part jamais en ring 3, garde l'espace d'adressage du
    /// noyau, et n'est jamais preempte (l'IRQ0 ne commute que depuis ring 3).
    ///
    /// Le gestionnaire de fenetres en est un. Il pouvait rester sur le fil du
    /// shell tant qu'il lancait ses programmes de facon synchrone ; des lors
    /// qu'il doit **composer pendant** que le navigateur tourne, il lui faut
    /// une place dans l'ordonnanceur comme a tout le monde.
    pub noyau: bool,
    /// Fonction du fil noyau. Elle ne rend jamais la main : elle se termine par
    /// [`exit_current`].
    entree_noyau: Option<fn() -> !>,
}

static mut TASKS: Option<Vec<Box<Task>>> = None;
/// Tous les processus vivants ou zombies.
static mut PROCESSES: Option<Vec<Rc<RefCell<Process>>>> = None;

/// Assertion de frontière SMP: suspendre le BKL autorise un autre CPU à
/// installer un sibling et donc à consulter son Process. Aucun Ref/RefMut ne
/// peut rester vivant à cet instant.
#[cfg(debug_assertions)]
pub(crate) fn debug_assert_no_process_borrows() {
    unsafe {
        if let Some(processes) = PROCESSES.as_ref() {
            for process in processes.iter() {
                debug_assert!(
                    process.try_borrow_mut().is_ok(),
                    "suspension BKL avec un Process encore emprunte"
                );
            }
        }
    }
}

const NO_TASK: usize = usize::MAX;
const MAX_CPUS: usize = smp::MAX_CPUS;
static CURRENT: [AtomicUsize; MAX_CPUS] = [const { AtomicUsize::new(NO_TASK) }; MAX_CPUS];
/// Zombie qui vient de quitter physiquement la pile de ce CPU. Le contexte
/// entrant le rend recyclable une fois le switch assembleur effectivement fini.
static RETIRED: [AtomicUsize; MAX_CPUS] = [const { AtomicUsize::new(NO_TASK) }; MAX_CPUS];
static NEXT_TID: AtomicU32 = AtomicU32::new(100);
static mut KERNEL_CTX: [Context; MAX_CPUS] = [Context { rsp: 0 }; MAX_CPUS];
static NEED_RESCHED: [AtomicBool; MAX_CPUS] = [const { AtomicBool::new(false) }; MAX_CPUS];
static CURRENT_IS_KERNEL: [AtomicBool; MAX_CPUS] = [const { AtomicBool::new(false) }; MAX_CPUS];
static TOURS_INTERACTIFS: [AtomicU32; MAX_CPUS] = [const { AtomicU32::new(0) }; MAX_CPUS];
static RUNQ_STEALS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static CPU_MIGRATIONS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static STEAL_ATTEMPTS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static STEAL_REJECT_BALANCE: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static STEAL_REJECT_AFFINITY: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];

static CONTEXT_SWITCHES: AtomicU64 = AtomicU64::new(0);
static IRQ_PREEMPTIONS: AtomicU64 = AtomicU64::new(0);
static DEFERRED_PREEMPTIONS: AtomicU64 = AtomicU64::new(0);
static WM_HEARTBEAT_TICK: AtomicU64 = AtomicU64::new(0);
static WM_WATCHDOG_ARMED: AtomicBool = AtomicBool::new(false);
static WM_LAST_WARNING_TICK: AtomicU64 = AtomicU64::new(0);

// BOUCHAUD_SMP4_STALL_PROBE_V1
// phase: 0=hors syscall, 1=attente BKL, 2=dans ABI avec BKL.
const STALL_NO_SYSCALL: u64 = u64::MAX;
static STALL_SYSCALL_NR: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(STALL_NO_SYSCALL) }; MAX_CPUS];
static STALL_SYSCALL_PHASE: [AtomicU32; MAX_CPUS] =
    [const { AtomicU32::new(0) }; MAX_CPUS];
static STALL_SYSCALL_TICK: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];

// BOUCHAUD_SMP4_OWNER_SITE_PROBE_V2
// Site noyau courant par CPU, uniquement pour diagnostic.
// 0=aucun/user, 21=page fault+BKL, 31=IPI+BKL, 41=preempt+BKL,
// 50=AP loop+BKL, 52=AP retour switch avant reacquire, 53=AP post-reacquire,
// 54=complete_retired, 55=activate_kernel, 61=timer+BKL.
static STALL_KERNEL_SITE: [AtomicU32; MAX_CPUS] =
    [const { AtomicU32::new(0) }; MAX_CPUS];
static STALL_KERNEL_AUX: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];

// BOUCHAUD_SMP4_OWNER_PROVENANCE_PROBE_V3
// Heartbeat IPI : capture avant toute tentative de BKL. Si l'age IPI du
// CPU proprietaire explose, il ne prend plus ses interruptions.
static STALL_IPI_COUNT: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];
static STALL_IPI_TICK: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];
static STALL_IPI_RIP: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];
static STALL_IPI_USER: [AtomicU32; MAX_CPUS] =
    [const { AtomicU32::new(0) }; MAX_CPUS];
static STALL_IPI_BKL_HIT: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];
static STALL_IPI_BKL_MISS: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];

// Compteurs page-fault. begin/done mesurent le handler ; file begin/done
// encadrent exactement fs::backing::read_at dans le demand paging.
static STALL_PF_BEGIN: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];
static STALL_PF_DONE: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];
static STALL_PF_FAIL: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];
static STALL_PF_FILE_BEGIN: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];
static STALL_PF_FILE_DONE: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];

#[inline]
pub fn stall_site_set(site: u32, aux: u64) {
    let cpu = local_cpu();
    STALL_KERNEL_AUX[cpu].store(aux, Ordering::Release);
    STALL_KERNEL_SITE[cpu].store(site, Ordering::Release);
}

#[inline]
pub fn stall_site_clear() {
    STALL_KERNEL_SITE[local_cpu()].store(0, Ordering::Release);
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
/// devient proprietaire. Aucun Rc/RefCell n'est touche ici.
pub fn stall_probe_local_context() -> (usize, u64, u32, u32, u64) {
    let cpu = local_cpu();
    (
        CURRENT[cpu].load(Ordering::Acquire),
        STALL_SYSCALL_NR[cpu].load(Ordering::Acquire),
        STALL_SYSCALL_PHASE[cpu].load(Ordering::Acquire),
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


// --- Sonde de stall SMP : aucun acces Rc/RefCell, uniquement atomiques. ---
pub fn stall_syscall_enter(nr: u64) {
    let cpu = local_cpu();
    STALL_SYSCALL_NR[cpu].store(nr, Ordering::Release);
    STALL_SYSCALL_TICK[cpu].store(crate::kernel::timer::ticks(), Ordering::Release);
    STALL_SYSCALL_PHASE[cpu].store(1, Ordering::Release);
}

pub fn stall_syscall_bkl_acquired() {
    STALL_SYSCALL_PHASE[local_cpu()].store(2, Ordering::Release);
}

pub fn stall_syscall_exit() {
    let cpu = local_cpu();
    STALL_SYSCALL_PHASE[cpu].store(0, Ordering::Release);
    STALL_SYSCALL_NR[cpu].store(STALL_NO_SYSCALL, Ordering::Release);
}

/// Appelee par le PIT BSP AVANT tout try_enter(BKL). Si les logs normaux
/// meurent parce qu'un AP garde le BKL, cette ligne continue donc a sortir.
pub fn stall_probe_from_timer() {
    let now = crate::kernel::timer::ticks();
    if now == 0 || now % crate::kernel::timer::TICKS_PER_SECOND != 0 {
        return;
    }

    let nr0 = STALL_SYSCALL_NR[0].load(Ordering::Acquire);
    let nr1 = STALL_SYSCALL_NR[1].load(Ordering::Acquire);
    let nr2 = STALL_SYSCALL_NR[2].load(Ordering::Acquire);
    let nr3 = STALL_SYSCALL_NR[3].load(Ordering::Acquire);
    let ph0 = STALL_SYSCALL_PHASE[0].load(Ordering::Acquire);
    let ph1 = STALL_SYSCALL_PHASE[1].load(Ordering::Acquire);
    let ph2 = STALL_SYSCALL_PHASE[2].load(Ordering::Acquire);
    let ph3 = STALL_SYSCALL_PHASE[3].load(Ordering::Acquire);
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

    crate::serial_println!(
        "[SMP-STALL] t={} owner={} depth=[{},{},{},{}] cur=[{},{},{},{}] site=[{}:{:#x} {}:{:#x} {}:{:#x} {}:{:#x}] syscall=[{}:{}/{} {}:{}/{} {}:{}/{} {}:{}/{}]",
        now,
        crate::kernel::smp_lock::stall_probe_owner_token(),
        crate::kernel::smp_lock::stall_probe_depth(0),
        crate::kernel::smp_lock::stall_probe_depth(1),
        crate::kernel::smp_lock::stall_probe_depth(2),
        crate::kernel::smp_lock::stall_probe_depth(3),
        CURRENT[0].load(Ordering::Acquire),
        CURRENT[1].load(Ordering::Acquire),
        CURRENT[2].load(Ordering::Acquire),
        CURRENT[3].load(Ordering::Acquire),
        site0, aux0, site1, aux1, site2, aux2, site3, aux3,
        nr0, ph0, age0,
        nr1, ph1, age1,
        nr2, ph2, age2,
        nr3, ph3, age3,
    );

    let prov = crate::kernel::smp_lock::stall_probe_provenance();
    let owner_cpu = if prov.owner_token == 0 { 255usize } else { prov.owner_token - 1 };
    let held = if prov.owner_token == 0 || prov.generation == 0 {
        0
    } else {
        now.wrapping_sub(prov.since_tick)
    };
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

#[inline]
fn set_current_index(index: usize) {
    CURRENT[local_cpu()].store(index, Ordering::Release);
}

#[inline]
fn set_current_is_kernel(value: bool) {
    CURRENT_IS_KERNEL[local_cpu()].store(value, Ordering::Release);
}

#[inline]
fn kernel_ctx() -> &'static mut Context {
    unsafe { &mut KERNEL_CTX[local_cpu()] }
}

/// A appeler dans le contexte qui vient de PRENDRE le CPU, BKL tenu. Le zombie
/// note avant le switch ne peut plus utiliser sa pile : son slot devient donc
/// recyclable sans use-after-free.
fn complete_retired() {
    let cpu = local_cpu();
    let retired = RETIRED[cpu].swap(NO_TASK, Ordering::AcqRel);
    if retired == NO_TASK { return; }
    if let Some(task) = tasks().get_mut(retired) {
        if task.state == TaskState::Zombie { task.on_cpu = -1; }
    }
}

/// PID du programme lance au premier plan par [`run`], 0 si aucun.
///
/// Un `exec` synchrone doit rendre la main quand CE programme se termine. Sans
/// ce reperage, `exit_current` n'avait qu'un seul critere -- « plus aucune
/// tache executable » -- et un programme qui laisse des fils vivants ne rendait
/// donc JAMAIS la main : les fils tournent, l'ordonnanceur a toujours quelqu'un
/// a servir, et l'invite ne revient pas.
///
/// C'est ce qui est arrive au run 32427953935. `BouchaudBrowserHost` quitte
/// proprement sur `window.close()`, mais ses services -- WebContent,
/// RequestServer, ImageDecoder, Compositor -- restent dans leur boucle
/// d'evenements. L'autorun ne reprenait pas, donc `power::shutdown` n'etait
/// jamais appele, donc /persist n'etait jamais ecrit a l'extinction.
static RACINE_PREMIER_PLAN: AtomicU32 = AtomicU32::new(0);

/// Le processus `pid` descend-il de `racine` (ou est-il `racine`) ?
fn descend_de(pid: u32, racine: u32) -> bool {
    let mut courant = pid;
    // La table est finie et un cycle de filiation serait une corruption : la
    // borne evite d'y tourner sans fin.
    for _ in 0..processes().len() + 1 {
        if courant == racine {
            return true;
        }
        if courant == 0 {
            return false;
        }
        let parent = processes()
            .iter()
            .find(|p| p.borrow().pid == courant)
            .map(|p| p.borrow().parent);
        match parent {
            Some(suivant) => courant = suivant,
            None => return false,
        }
    }
    false
}

fn tasks() -> &'static mut Vec<Box<Task>> {
    unsafe {
        if TASKS.is_none() {
            TASKS = Some(Vec::new());
        }
        TASKS.as_mut().unwrap()
    }
}

/// Table des processus.
pub fn processes() -> &'static mut Vec<Rc<RefCell<Process>>> {
    unsafe {
        if PROCESSES.is_none() {
            PROCESSES = Some(Vec::new());
        }
        PROCESSES.as_mut().unwrap()
    }
}

/// Retrouve un processus par son pid.
pub fn process_by_pid(pid: u32) -> Option<Rc<RefCell<Process>>> {
    processes().iter().find(|p| p.borrow().pid == pid).cloned()
}

/// Retrouve le processus auquel appartient un thread donne.
pub fn process_of_tid(tid: u32) -> Option<Rc<RefCell<Process>>> {
    tasks()
        .iter()
        .find(|t| t.tid == tid)
        .map(|t| t.process.clone())
}

/// Alloue un identifiant de tache.
pub fn alloc_tid() -> u32 {
    NEXT_TID.fetch_add(1, Ordering::Relaxed)
}

/// Y a-t-il une tache utilisateur en cours ?
pub fn in_user_task() -> bool {
    current_index_raw() != NO_TASK
}

/// Tache courante du CPU local.
pub fn current() -> &'static mut Task {
    let index = current_index_raw();
    assert!(index != NO_TASK, "task: aucune tache active sur ce CPU");
    unsafe { &mut *(&mut **tasks().get_mut(index).unwrap() as *mut Task) }
}

/// Tache courante, si elle existe.
pub fn try_current() -> Option<&'static mut Task> {
    if in_user_task() {
        Some(current())
    } else {
        None
    }
}

/// Processus de la tache courante.
pub fn current_process() -> Rc<RefCell<Process>> {
    current().process.clone()
}

/// Temps processeur consomme par un processus, en millisecondes.
///
/// Le profileur par echantillonnage incremente `ticks_cpu` de la tache courante
/// a chaque IRQ0. Le PIT battant a `TICKS_PER_SECOND` = 1000 Hz, un tick vaut
/// donc exactement une milliseconde de processeur — et la somme sur les taches
/// du processus est son temps CPU.
///
/// Sert a `clock_gettime(CLOCK_PROCESS_CPUTIME_ID)`. Sans cette horloge, un
/// programme ne peut pas distinguer une attente qui dort d'une attente qui
/// brule un cœur : les deux durent le meme temps au mur. C'est exactement la
/// question que posait la reparation de la couche UDP.
pub fn cpu_time_ms(pid: u32) -> u64 {
    let mut total = 0u64;
    for task in tasks().iter() {
        if task.process.borrow().pid == pid {
            total = total.saturating_add(task.ticks_cpu);
        }
    }
    total * (1000 / crate::kernel::timer::TICKS_PER_SECOND).max(1)
}

impl Task {
    /// Cree une tache prete a demarrer en ring 3 avec la trame donnee.
    pub fn new(process: Rc<RefCell<Process>>, frame: TrapFrame) -> Box<Task> {
        let kstack = vec![0u8; KSTACK_SIZE];
        // Sommet aligne 16 : l'ABI System V l'exige avant chaque `call`, et le
        // stub d'entree syscall empile un nombre pair de quadmots.
        let kstack_top = (kstack.as_ptr() as u64 + KSTACK_SIZE as u64) & !0xF;
        // Le tampon vit sur le tas : son adresse ne bouge pas quand la `Task`
        // est deplacee dans son `Box`.
        let fpu = vec![0u8; 512 + 16];
        let fpu_area = (fpu.as_ptr() as u64 + 15) & !0xF;
        // Etat FPU initial valide : une zone `fxsave` toute a zero donnerait
        // MXCSR = 0, c'est-a-dire toutes les exceptions SSE demasquees — la
        // premiere division flottante du programme leverait alors #XF.
        unsafe {
            let area = fpu_area as *mut u8;
            core::ptr::copy_nonoverlapping(0x037Fu16.to_le_bytes().as_ptr(), area, 2); // FCW
            core::ptr::copy_nonoverlapping(0x1F80u32.to_le_bytes().as_ptr(), area.add(24), 4); // MXCSR
            core::ptr::copy_nonoverlapping(0x0000_FFBFu32.to_le_bytes().as_ptr(), area.add(28), 4);
            // MXCSR_MASK
        }

        let mut task = Box::new(Task {
            tid: alloc_tid(),
            process,
            state: TaskState::Ready,
            // Toute tache nait normale. C'est a elle de se declarer
            // interactive — un programme qui ne demande rien ne doit pas
            // pouvoir prendre le pas sur l'interface par accident.
            priorite: Priorite::Normale,
            affinity_mask: 0,
            runq_cpu: u8::MAX,
            last_cpu: u8::MAX,
            on_cpu: -1,
            last_migration_ns: 0,
            recent_runtime_ns: 0,
            slice_start_ns: 0,
            last_account_ns: 0,
            user_cpu_ns: 0,
            kernel_cpu_ns: 0,
            cpu_ns: [0; MAX_CPUS],
            in_kernel: false,
            context_switches: 0,
            migrations: 0,
            frame,
            ctx: Context::default(),
            kstack,
            kstack_top,
            fpu,
            fpu_area,
            fs_base: 0,
            clear_child_tid: 0,
            futex_key: 0,
            wait_queue_key: 0,
            wake_deadline_ns: 0,
            waiting_for_child: false,
            fresh: true,
            ticks_cpu: 0,
            noyau: false,
            entree_noyau: None,
        });

        // RFLAGS de depart : bit 1 reserve a 1, `IF` a 0. Le trampoline n'a
        // pas besoin des interruptions — `resume_usermode` commence par un
        // `cli` — et l'`iretq` qui l'acheve rendra a la tache le RFLAGS de
        // sa trame ring 3.
        amorce_pile(&mut task, task_trampoline, 0x0000_0002);
        task
    }

    /// Cree un fil noyau : meme ordonnancement, mais il execute `entree` en
    /// ring 0 au lieu de partir en ring 3.
    ///
    /// `process` n'est la que pour les champs que tout le noyau consulte sans
    /// se demander qui les porte (pid, table de descripteurs). Son espace
    /// d'adressage n'est jamais active : [`install`] bascule sur celui du noyau
    /// pour un fil noyau.
    pub fn new_kernel(process: Rc<RefCell<Process>>, entree: fn() -> !) -> Box<Task> {
        // La trame ring 3 n'a aucun sens ici ; elle reste a zero et n'est jamais
        // restauree, puisque le trampoline noyau ne fait pas d'`iretq`.
        let mut task = Task::new(process, TrapFrame::new_user(0, 0));
        task.noyau = true;
        task.affinity_mask = 1;
        task.runq_cpu = 0;
        task.last_cpu = 0;
        task.entree_noyau = Some(entree);
        // Un fil noyau demarre **interruptions actives** : rien ne les
        // retablira pour lui plus tard. Sans `IF`, sa premiere attente
        // s'arreterait sur un `hlt` que plus aucun tick ne pourrait lever.
        amorce_pile(&mut task, kernel_task_trampoline, 0x0000_0202);
        task
    }

    /// Adresse de la zone `fxsave` (alignee 16, dans `self.fpu`).
    fn fpu_ptr(&self) -> u64 {
        self.fpu_area
    }
}

/// Amorce de pile noyau : le premier `switch_context` vers cette tache depile
/// six registres callee-saved et un RFLAGS, puis fait `ret` sur `trampoline`.
/// La disposition doit etre le miroir exact des `push` de `switch_context`.
fn amorce_pile(task: &mut Task, trampoline: extern "C" fn() -> !, rflags: u64) {
    unsafe {
        let mut sp = task.kstack_top as *mut u64;
        sp = sp.sub(1);
        *sp = trampoline as *const () as usize as u64; // adresse de retour
        sp = sp.sub(1);
        *sp = rflags;
        for _ in 0..6 {
            sp = sp.sub(1);
            *sp = 0; // rbp, rbx, r12, r13, r14, r15
        }
        task.ctx.rsp = sp as u64;
    }
}

/// Masque des CPU logiques actuellement utilisables.
fn online_affinity_mask() -> u64 {
    let online = smp::schedulable_cpus().max(1).min(MAX_CPUS).min(64);
    if online >= 64 { u64::MAX } else { (1u64 << online) - 1 }
}

#[inline]
fn allowed_on(task: &Task, cpu: usize) -> bool {
    cpu < 64 && task.affinity_mask & (1u64 << cpu) != 0
}

fn running_count_cpu(cpu: usize) -> usize {
    tasks().iter()
        .filter(|t| t.state != TaskState::Zombie && t.on_cpu == cpu as i8)
        .count()
}

fn queue_pressure(cpu_id: usize) -> usize {
    tasks().iter()
        .filter(|t| {
            t.state == TaskState::Ready
                && t.on_cpu < 0
                && t.runq_cpu as usize == cpu_id
                && allowed_on(t, cpu_id)
        })
        .count()
}

/// Placement initial d'un THREAD. Le score combine pression de runqueue,
/// nombre de taches deja running et charge mesuree. CPU0 recoit une petite
/// penalite car il porte le desktop/PIC, sans devenir interdit au userland.
fn choose_runq_cpu(mask: u64) -> u8 {
    let online = smp::schedulable_cpus().max(1).min(MAX_CPUS);
    let mut best_cpu = 0usize;
    let mut best_score = usize::MAX;

    for cpu_id in 0..online {
        if cpu_id >= 64 || mask & (1u64 << cpu_id) == 0 {
            continue;
        }
        let rq = queue_pressure(cpu_id);
        let running = running_count_cpu(cpu_id);
        let measured = cpu::load_percent_cpu(cpu_id) as usize;
        let bsp_penalty = if cpu_id == 0 && online > 1 { 24 } else { 0 };
        let score = rq.saturating_mul(32)
            .saturating_add(running.saturating_mul(16))
            .saturating_add(measured)
            .saturating_add(bsp_penalty);
        if score < best_score {
            best_score = score;
            best_cpu = cpu_id;
        }
    }
    best_cpu as u8
}

/// Ajoute une tache a la table et renvoie son indice. En SMP les indices ne
/// bougent jamais : un slot zombie est recycle au lieu de compacter le Vec.
pub fn register(mut task: Box<Task>) -> usize {
    let _kernel = smp_lock::enter();

    if task.noyau {
        task.affinity_mask = 1;
        task.runq_cpu = 0;
        task.last_cpu = 0;
    } else {
        if task.affinity_mask == 0 {
            task.affinity_mask = online_affinity_mask();
        } else {
            task.affinity_mask &= online_affinity_mask();
            if task.affinity_mask == 0 {
                task.affinity_mask = online_affinity_mask();
            }
        }
        if task.runq_cpu == u8::MAX || !allowed_on(&task, task.runq_cpu as usize) {
            task.runq_cpu = choose_runq_cpu(task.affinity_mask);
        }
    }
    task.on_cpu = -1;

    let reuse = tasks().iter().position(|old| {
        old.state == TaskState::Zombie && old.on_cpu < 0
    });
    let index = if let Some(index) = reuse {
        tasks()[index] = task;
        index
    } else {
        let list = tasks();
        list.push(task);
        list.len() - 1
    };

    {
        let registered = &tasks()[index];
        let process = registered.process.borrow();
        crate::serial_println!(
            "[SMP-TASK] idx={} tid={} pid={} rq={} last={} aff={:#x} on={} kernel={} prio={:?} name={}",
            index,
            registered.tid,
            process.pid,
            registered.runq_cpu,
            registered.last_cpu,
            registered.affinity_mask,
            registered.on_cpu,
            registered.noyau,
            registered.priorite,
            process.name.as_str(),
        );
    }

    smp::broadcast_reschedule();
    if let Some(id) = crate::arch::x86_64::cpu_local::CpuId::from_index(
        tasks()[index].runq_cpu as usize,
    ) {
        crate::arch::x86_64::cpu_local::local(id).enqueue(index);
    }
    index
}

/// Indice d'une tache par son tid.
fn index_of(tid: u32) -> Option<usize> {
    tasks().iter().position(|t| t.tid == tid)
}

/// Tache par tid.
pub fn by_tid(tid: u32) -> Option<&'static mut Task> {
    let index = index_of(tid)?;
    Some(unsafe { &mut *(&mut **tasks().get_mut(index).unwrap() as *mut Task) })
}

/// Nombre de taches vivantes (non zombies).
pub fn live_count() -> usize {
    tasks()
        .iter()
        .filter(|t| t.state != TaskState::Zombie)
        .count()
}

/// Nombre de taches pretes.
fn ready_count() -> usize {
    tasks().iter().filter(|t| t.state == TaskState::Ready).count()
}

fn ready_count_cpu(cpu: usize) -> usize {
    tasks().iter().filter(|t| {
        t.state == TaskState::Ready
            && t.on_cpu < 0
            && t.runq_cpu as usize == cpu
            && allowed_on(t, cpu)
    }).count()
}

fn stealable_count_cpu(cpu: usize) -> usize {
    tasks().iter().filter(|t| {
        t.state == TaskState::Ready
            && t.on_cpu < 0
            && !t.noyau
            && t.runq_cpu as usize != cpu
            && allowed_on(t, cpu)
    }).count()
}

fn running_count() -> usize {
    tasks().iter().filter(|t| t.state != TaskState::Zombie && t.on_cpu >= 0).count()
}

// --- Changement de contexte --------------------------------------------------

/// Sauvegarde RFLAGS et les registres callee-saved sur la pile courante,
/// bascule sur la pile `to`, et y restaure le tout. Le retour se fait sur
/// l'adresse empilee par la sauvegarde symetrique (ou par l'amorce de
/// [`Task::new`]).
///
/// ## Pourquoi RFLAGS fait partie du contexte
///
/// `IF` — le drapeau d'interruption — est un etat du CPU, pas de la pile : sans
/// ce `pushfq`/`popfq`, il traverse la commutation et suit la **nouvelle**
/// tache. Or les deux appelants n'ont pas le meme etat : [`schedule`] commute
/// depuis un appel systeme, interruptions actives, tandis que
/// [`preempt_from_irq`] commute depuis le gestionnaire du timer, ou le CPU les a
/// coupees en franchissant la porte d'interruption. La preemption d'une tache
/// livrait donc son `IF=0` a celle qui reprenait la main, au beau milieu de son
/// appel systeme. La suite dependait de ce qu'elle y faisait : le plus souvent
/// rien de visible — elle rendait la main en ring 3, ou `sysretq` remet un
/// RFLAGS correct —, mais si elle attendait dans un `poll`, un `futex` ou un
/// sommeil, son `hlt` arretait le CPU alors que plus aucune interruption ne
/// pouvait le reveiller. Machine gelee, sans faute ni message.
///
/// Sauvegarder RFLAGS rend chaque tache a l'etat d'interruption qui etait le
/// sien : la tache preemptee reprend dans son gestionnaire d'IRQ avec `IF=0`
/// (et c'est `iretq` qui le retablira), celle qui dormait dans un appel systeme
/// reprend avec `IF=1`.
///
/// # Securite
/// `from` doit pointer sur un `Context` valide et `to` sur une pile noyau
/// preparee par cette meme fonction ou par `Task::new`.
#[unsafe(naked)]
unsafe extern "C" fn switch_context(from: *mut u64, to: u64) {
    core::arch::naked_asm!(
        "pushfq",
        "push r15",
        "push r14",
        "push r13",
        "push r12",
        "push rbx",
        "push rbp",
        "mov [rdi], rsp",
        "mov rsp, rsi",
        "pop rbp",
        "pop rbx",
        "pop r12",
        "pop r13",
        "pop r14",
        "pop r15",
        "popfq",
        "ret",
    )
}

/// Point d'entree d'une tache neuve : le scheduler a relache le BKL avant le
/// switch. On le reprend uniquement le temps d'installer l'etat materiel, puis
/// on le rend avant l'iretq utilisateur.
extern "C" fn task_trampoline() -> ! {
    let frame = {
        let _kernel = smp_lock::enter();
        complete_retired();
        let task = current();
        install(task);
        task.frame
    };
    unsafe { usermode::resume_usermode(&frame) }
}

/// Les fils noyau sont pin CPU0 et gardent le BKL pendant leur travail. Chaque
/// `schedule()` le suspend autour du changement de contexte.
extern "C" fn kernel_task_trampoline() -> ! {
    let _kernel = smp_lock::enter();
    complete_retired();
    let task = current();
    task.fresh = false;
    let entree = task.entree_noyau.expect("task: fil noyau sans point d'entree");
    entree()
}

/// Installe le contexte materiel d'une tache : espace d'adressage, pile noyau,
/// base FS (TLS) et etat FPU.
fn install(task: &mut Task) {
    unsafe {
        set_current_is_kernel(task.noyau);
        // Un fil noyau n'a pas d'espace utilisateur a activer, et surtout ne
        // doit pas activer celui d'un programme : il lirait alors, sous les
        // memes adresses, la memoire du dernier processus installe.
        if task.noyau {
            crate::kernel::vmm::activate_kernel();
        } else {
            task.process.borrow().space.activate();
        }
        usermode::set_kernel_stack(task.kstack_top);
        usermode::set_fs_base(task.fs_base);
        usermode::per_cpu().current = task.tid as u64;
        // La zone est initialisee a un etat FPU valide des `Task::new` : on peut
        // restaurer inconditionnellement, y compris au premier passage.
        usermode::fxrstor(task.fpu_ptr() as *const u8);
        task.fresh = false;
    }
}

#[inline]
fn deactivate_task_space(task: &Task, cpu_id: usize) {
    if !task.noyau {
        task.process.borrow().space.mark_inactive(cpu_id);
    }
}

#[inline]
fn mark_task_running(task: &mut Task, cpu_id: usize) {
    let now = crate::kernel::timer::monotonic_ns();
    if task.last_cpu != u8::MAX && task.last_cpu as usize != cpu_id {
        CPU_MIGRATIONS[cpu_id].fetch_add(1, Ordering::Relaxed);
        task.migrations = task.migrations.saturating_add(1);
        task.last_migration_ns = now;
        if let Some(id) = crate::arch::x86_64::cpu_local::CpuId::from_index(cpu_id) {
            crate::arch::x86_64::cpu_local::local(id).note_migration();
        }
    }
    task.last_cpu = cpu_id as u8;
    task.runq_cpu = cpu_id as u8;
    task.on_cpu = cpu_id as i8;
    task.slice_start_ns = now;
    task.context_switches = task.context_switches.saturating_add(1);
}

fn account_slice_end(task: &mut Task) {
    if task.slice_start_ns == 0 {
        return;
    }
    let elapsed = crate::kernel::timer::monotonic_ns()
        .saturating_sub(task.slice_start_ns);
    if task.in_kernel {
        task.kernel_cpu_ns = task.kernel_cpu_ns.saturating_add(elapsed);
    } else {
        task.user_cpu_ns = task.user_cpu_ns.saturating_add(elapsed);
    }
    let cpu = local_cpu();
    task.cpu_ns[cpu] = task.cpu_ns[cpu].saturating_add(elapsed);
    // EWMA 7/8 historique + 1/8 dernière tranche: stable mais réactif en
    // quelques quanta, sans utiliser les ticks comme unité.
    task.recent_runtime_ns = task
        .recent_runtime_ns
        .saturating_mul(7)
        .saturating_add(elapsed)
        / 8;
    task.slice_start_ns = 0;
}

/// Frontières syscall utilisées pour séparer user/kernel sans dépendre du PIT.
pub fn account_kernel_enter() {
    let task = current();
    account_slice_end(task);
    task.in_kernel = true;
    task.slice_start_ns = crate::kernel::timer::monotonic_ns();
}

pub fn account_kernel_exit() {
    let task = current();
    account_slice_end(task);
    task.in_kernel = false;
    task.slice_start_ns = crate::kernel::timer::monotonic_ns();
}

pub fn account_resume_user_noreturn() {
    account_kernel_exit();
}

/// Choisit la prochaine tache prete apres `after`.
///
/// Un tourniquet a deux etages. On cherche d'abord une tache **interactive**
/// prete, en repartant de `after` pour que plusieurs taches interactives se
/// relaient equitablement entre elles. A defaut, on prend la premiere prete,
/// quelle qu'elle soit.
///
/// La borne `TOURS_INTERACTIFS_MAX` est ce qui separe une priorite d'une
/// famine : passe ce nombre de tours consecutifs, l'etage interactif est
/// ignore pour un tour et le tourniquet ordinaire reprend. Une tache normale
/// avance donc toujours, meme face a une tache interactive qui ne se bloque
/// jamais.
fn runnable_local(task: &Task, cpu: usize) -> bool {
    task.state == TaskState::Ready
        && task.on_cpu < 0
        && task.runq_cpu as usize == cpu
        && allowed_on(task, cpu)
}

fn runnable_steal(task: &Task, cpu: usize) -> bool {
    task.state == TaskState::Ready
        && task.on_cpu < 0
        && !task.noyau
        && task.runq_cpu as usize != cpu
        && allowed_on(task, cpu)
}

fn pick_next(after: usize, cpu: usize) -> Option<usize> {
    let len = tasks().len();
    if len == 0 { return None; }

    if let Some(id) = crate::arch::x86_64::cpu_local::CpuId::from_index(cpu) {
        let local = crate::arch::x86_64::cpu_local::local(id);
        while let Some(index) = local.dequeue() {
            if index < len && runnable_local(&tasks()[index], cpu) {
                return Some(index);
            }
        }
        // Transition NG3: les anciens callsites de wake ne connaissent pas
        // encore tous RunQueue. On reconcilie sous BKL, sans doublon, puis les
        // prochains choix consomment exclusivement la queue locale.
        for index in 0..len {
            if runnable_local(&tasks()[index], cpu) {
                local.enqueue(index);
            }
        }
        while let Some(index) = local.dequeue() {
            if index < len && runnable_local(&tasks()[index], cpu) {
                return Some(index);
            }
        }
    }

    let start = if after == NO_TASK { 0 } else { after % len };
    let tours = TOURS_INTERACTIFS[cpu].load(Ordering::Relaxed);
    let force_partage = tours >= TOURS_INTERACTIFS_MAX;

    if !force_partage {
        for offset in 1..=len {
            let index = (start.wrapping_add(offset)) % len;
            let candidate = &tasks()[index];
            if runnable_local(candidate, cpu)
                && candidate.priorite == Priorite::Interactive
            {
                TOURS_INTERACTIFS[cpu].fetch_add(1, Ordering::Relaxed);
                return Some(index);
            }
        }
    }

    for offset in 1..=len {
        let index = (start.wrapping_add(offset)) % len;
        if runnable_local(&tasks()[index], cpu) {
            if tasks()[index].priorite == Priorite::Normale {
                TOURS_INTERACTIFS[cpu].store(0, Ordering::Relaxed);
            } else {
                TOURS_INTERACTIFS[cpu].fetch_add(1, Ordering::Relaxed);
            }
            return Some(index);
        }
    }

    let mut pressure = [0usize; MAX_CPUS];
    for task in tasks().iter() {
        if task.state == TaskState::Ready && task.on_cpu < 0 {
            let owner = task.runq_cpu as usize;
            if owner < MAX_CPUS {
                pressure[owner] = pressure[owner].saturating_add(1);
            }
        }
    }

    STEAL_ATTEMPTS[cpu].fetch_add(1, Ordering::Relaxed);
    let donor = (0..smp::schedulable_cpus().min(MAX_CPUS))
        .filter(|&candidate| candidate != cpu)
        .max_by_key(|&candidate| pressure[candidate]);
    let Some(donor) = donor else { return None; };

    // Ne jamais voler la dernière tâche Ready d'un CPU: NG3 le faisait dans
    // le fallback ci-dessous, puis un autre CPU la revolait quelques ms plus
    // tard. C'était la source directe des milliers de migrations observées.
    if pressure[donor] <= pressure[cpu].saturating_add(1) {
        STEAL_REJECT_BALANCE[cpu].fetch_add(1, Ordering::Relaxed);
        return None;
    }

    const MIN_MIGRATION_RESIDENCY_NS: u64 = 20_000_000;
    let now = crate::kernel::timer::monotonic_ns();
    let mut best: Option<(usize, u64)> = None;
    for offset in 1..=len {
        let index = (start.wrapping_add(offset)) % len;
        let candidate = &tasks()[index];
        if !runnable_steal(candidate, cpu) || candidate.runq_cpu as usize != donor {
            continue;
        }
        if candidate.last_migration_ns != 0
            && now.saturating_sub(candidate.last_migration_ns) < MIN_MIGRATION_RESIDENCY_NS
        {
            STEAL_REJECT_AFFINITY[cpu].fetch_add(1, Ordering::Relaxed);
            continue;
        }
        // Préférer une tâche qui a réellement consommé du CPU: déplacer un
        // waiter qui se rendort immédiatement ne corrige aucune imbalance.
        if best.map_or(true, |(_, old_weight)| candidate.recent_runtime_ns > old_weight) {
            best = Some((index, candidate.recent_runtime_ns));
        }
    }

    if let Some((index, _)) = best {
        tasks()[index].runq_cpu = cpu as u8;
        RUNQ_STEALS[cpu].fetch_add(1, Ordering::Relaxed);
        return Some(index);
    }

    None
}

/// Change la classe d'ordonnancement du processus courant.
///
/// Rend l'ancienne. Toutes les taches du processus suivent : une priorite
/// s'applique a un programme, pas a l'un de ses fils — un navigateur dont
/// seule la moitie des fils serait prioritaire aurait une interface qui saccade
/// une fois sur deux.
pub fn pose_priorite(priorite: Priorite) -> Priorite {
    let pid = current().process.borrow().pid;
    let ancienne = current().priorite;
    for task in tasks().iter_mut() {
        if task.process.borrow().pid == pid {
            task.priorite = priorite;
        }
    }
    ancienne
}

/// La classe d'ordonnancement du processus courant.
pub fn priorite() -> Priorite {
    current().priorite
}

/// Verifie l'invariant de non-reentrance avant de commuter.
///
/// La regle est enoncee en tete de [`crate::kernel::abi`] : aucun emprunt du
/// `Process` ne doit survivre a un point de commutation. Elle n'est pas
/// verifiable a la compilation, `RefCell` comptant ses emprunts a l'execution ;
/// mais elle l'est ici, et c'est le seul endroit qui compte, puisque toutes les
/// attentes du noyau — `yield_now`, futex, lecture bloquante — passent par
/// [`schedule`].
///
/// Sans ce controle, un emprunt oublie ne se manifeste qu'au moment ou une
/// **autre** tache du meme processus tente d'emprunter a son tour : le
/// `BorrowMutError` designe alors la victime, jamais le coupable, et rien dans
/// la trace ne mene a l'appel systeme fautif. Le cout est d'un essai d'emprunt
/// par commutation, uniquement en compilation de debogage.
#[inline]
fn debug_assert_borrows_released() {
    #[cfg(debug_assertions)]
    {
        if let Some(task) = try_current() {
            debug_assert!(
                task.process.try_borrow_mut().is_ok(),
                "task: un emprunt du Process est encore actif au moment de commuter \
                 — relacher le borrow avant toute attente (invariant : voir kernel::abi)"
            );
        }
    }
}

/// Second invariant du meme point de passage : **on ne commute jamais
/// interruptions coupees**.
///
/// [`schedule`] n'est appelee que depuis du code de tache — un appel systeme,
/// une attente volontaire —, jamais depuis un gestionnaire d'interruption (la
/// preemption sur IRQ0 passe par [`preempt_from_irq`], qui ne vient pas ici).
/// Dans ce contexte `IF` vaut toujours 1, et il le faut : la tache qui attend
/// s'arrete sur un `hlt` dont seul le tick du timer la tirera.
///
/// Un `IF=0` a cet endroit signalerait une fuite du drapeau — un `cli` sans
/// `sti`, ou une commutation qui ne rendrait pas son RFLAGS a la tache reprise
/// (voir [`switch_context`]). La panique arrive alors dans le coupable, au lieu
/// du gel silencieux qu'on constaterait autrement plusieurs instructions plus
/// loin, sur le `hlt` de la victime.
#[inline]
fn debug_assert_interrupts_enabled() {
    #[cfg(debug_assertions)]
    debug_assert!(
        cpu::interrupts_enabled(),
        "task: commutation demandee interruptions coupees — le `hlt` d'attente \
         figerait la machine (invariant : voir switch_context)"
    );
}

/// Dort jusqu'a la prochaine interruption en garantissant que le Big Kernel
/// Lock n'est jamais conserve pendant HLT.
///
/// Cette primitive est la seule autorisee depuis les attentes ABI qui dorment
/// directement (sigsuspend/pause, WASI clock poll). `syscall_dispatch` garde
/// un BKL externe pendant `abi::handle`; la suspension explicite est donc
/// obligatoire avant HLT.
pub fn wait_for_interrupt_releasing_bkl() {
    debug_assert_interrupts_enabled();
    let depth = smp_lock::suspend_for_schedule();

    #[cfg(debug_assertions)]
    debug_assert!(
        !smp_lock::held_by_current_cpu(),
        "task: HLT interdit tant que le BKL est detenu"
    );

    cpu::wait_for_interrupt();
    smp_lock::resume_after_schedule(depth);
}

/// Rend la main : bascule sur une autre tache prete s'il y en a une.
///
/// Renvoie `true` si un changement de tache a eu lieu. Si la tache courante est
/// la seule prete, la fonction attend une interruption (`hlt`) et rend la main a
/// l'appelant, qui doit reevaluer sa condition d'attente.
pub fn schedule() -> bool {
    let _kernel = smp_lock::enter();
    let cur = current_index_raw();
    if cur == NO_TASK { return false; }
    debug_assert_borrows_released();
    debug_assert_interrupts_enabled();
    wake_sleepers();
    let cpu_id = local_cpu();
    let next = match pick_next(cur, cpu_id) {
        Some(next) if next != cur => next,
        _ => {
            if tasks()[cur].state != TaskState::Ready {
                // Ne jamais dormir en tenant le BKL : les autres CPU doivent
                // pouvoir entrer dans leurs syscalls pendant notre HLT.
                let depth = smp_lock::suspend_for_schedule();
                cpu::wait_for_interrupt();
                smp_lock::resume_after_schedule(depth);
            }
            return false;
        }
    };
    switch_to(cur, next);
    true
}

fn switch_to(from: usize, to: usize) {
    let _kernel = smp_lock::enter();
    let cpu_id = local_cpu();
    let (from_ptr, to_ptr) = unsafe {
        let list = tasks();
        let from_ptr = &mut **list.get_mut(from).unwrap() as *mut Task;
        let to_ptr = &mut **list.get_mut(to).unwrap() as *mut Task;

        CONTEXT_SWITCHES.fetch_add(1, Ordering::Relaxed);
        account_slice_end(&mut *from_ptr);
        usermode::fxsave((*from_ptr).fpu_ptr() as *mut u8);
        (*from_ptr).fs_base = usermode::fs_base();
        deactivate_task_space(&*from_ptr, cpu_id);
        if (*from_ptr).state == TaskState::Zombie {
            RETIRED[cpu_id].store(from, Ordering::Release);
            // Reste marque running jusqu'a ce que le nouveau contexte confirme
            // que le switch assembleur a effectivement quitte cette pile.
        } else {
            (*from_ptr).on_cpu = -1;
            (*from_ptr).last_cpu = cpu_id as u8;
            (*from_ptr).runq_cpu = cpu_id as u8;
        }
        mark_task_running(&mut *to_ptr, cpu_id);

        set_current_index(to);
        install(&mut *to_ptr);
        (from_ptr, to_ptr)
    };

    let depth = smp_lock::suspend_for_schedule();
    unsafe { switch_context(&mut (*from_ptr).ctx.rsp, (*to_ptr).ctx.rsp); }
    smp_lock::resume_after_schedule(depth);
    complete_retired();
}

/// Retour definitif au fil noyau appelant (la tache courante est terminee).
fn switch_to_kernel() -> ! {
    let _kernel = smp_lock::enter();
    let cur = current_index_raw();
    let from_ptr = unsafe {
        let list = tasks();
        let ptr = &mut **list.get_mut(cur).unwrap() as *mut Task;
        deactivate_task_space(&*ptr, local_cpu());
        if (*ptr).state == TaskState::Zombie {
            RETIRED[local_cpu()].store(cur, Ordering::Release);
        } else {
            (*ptr).on_cpu = -1;
        }
        ptr
    };
    set_current_index(NO_TASK);
    set_current_is_kernel(false);
    usermode::per_cpu().current = 0;
    crate::kernel::vmm::activate_kernel();
    let target_rsp = kernel_ctx().rsp;
    let depth = smp_lock::suspend_for_schedule();
    unsafe { switch_context(&mut (*from_ptr).ctx.rsp, target_rsp); }
    smp_lock::resume_after_schedule(depth);
    unreachable!("task: reprise d'une tache terminee")
}

/// Boucle idle/scheduler des AP. Le contexte `KERNEL_CTX[cpu]` est la pile de
/// cette boucle ; `switch_to_kernel` y revient lorsqu'un processus local n'a
/// plus de tache executable.
pub fn secondary_cpu_loop() -> ! {
    let cpu_id = local_cpu();
    assert!(cpu_id != 0, "task: secondary_cpu_loop sur BSP");
    set_current_index(NO_TASK);
    set_current_is_kernel(false);
    usermode::per_cpu().current = 0;

    loop {
        let _kernel = smp_lock::enter();
        stall_site_set(50, current_index_raw() as u64);
        // Avant le premier register() du BSP, ne meme pas materialiser TASKS :
        // cela permet d'activer les AP juste avant l'autorun sans mettre le boot
        // historique en concurrence avec une allocation secondaire.
        let aucune_tache = unsafe { TASKS.as_ref().map_or(true, |list| list.is_empty()) };
        if aucune_tache {
            let depth = smp_lock::suspend_for_schedule();
            stall_site_clear();
            cpu::wait_for_interrupt();
            stall_site_set(52, current_index_raw() as u64);
            smp_lock::resume_after_schedule(depth);
            stall_site_set(53, current_index_raw() as u64);
            continue;
        }
        wake_sleepers();
        if let Some(next) = pick_next(NO_TASK, cpu_id) {
            let to_ptr = unsafe {
                let list = tasks();
                let ptr = &mut **list.get_mut(next).unwrap() as *mut Task;
                mark_task_running(&mut *ptr, cpu_id);
                ptr
            };
            set_current_index(next);
            unsafe { install(&mut *to_ptr); }
            let kernel_rsp = &mut kernel_ctx().rsp as *mut u64;
            let depth = smp_lock::suspend_for_schedule();
            stall_site_clear();
            unsafe { switch_context(kernel_rsp, (*to_ptr).ctx.rsp); }
            stall_site_set(52, current_index_raw() as u64);
            smp_lock::resume_after_schedule(depth);
            stall_site_set(53, current_index_raw() as u64);
            stall_site_set(54, RETIRED[cpu_id].load(Ordering::Acquire) as u64);
            complete_retired();
            stall_site_set(50, current_index_raw() as u64);
            set_current_index(NO_TASK);
            set_current_is_kernel(false);
            usermode::per_cpu().current = 0;
            stall_site_set(55, 0);
            crate::kernel::vmm::activate_kernel();
            stall_site_set(50, NO_TASK as u64);
        } else {
            let depth = smp_lock::suspend_for_schedule();
            stall_site_clear();
            cpu::wait_for_interrupt();
            stall_site_set(52, current_index_raw() as u64);
            smp_lock::resume_after_schedule(depth);
            stall_site_set(53, current_index_raw() as u64);
        }
    }
}

/// Marque la tache courante terminee et rend la main.
///
/// Si d'autres threads du programme tournent encore, on bascule sur eux ;
/// sinon, retour au fil noyau qui a lance le programme.
pub fn exit_current(code: i32) -> ! {
    let cur = current_index_raw();
    {
        let task = current();
        task.state = TaskState::Zombie;
        // pthread_join s'appuie sur cette ecriture suivie d'un futex_wake.
        let clear = task.clear_child_tid;
        if clear != 0 {
            let process = task.process.clone();
            let mut process = process.borrow_mut();
            process.space.write(clear, &0u32.to_le_bytes());
            drop(process);
            futex_wake(clear, 1);
        }
        let process = task.process.clone();
        let mut process = process.borrow_mut();
        if process.threads > 0 {
            process.threads -= 1;
        }
        process.exit_code = code;
        if process.threads == 0 {
            // Dernier thread : le processus devient zombie jusqu'a ce que son
            // parent le recolte par `wait4`. C'est ce qui permet au parent de
            // recuperer le code de sortie apres coup.
            process.zombie = true;
            // Les verrous d'enregistrement POSIX meurent avec leur detenteur.
            // Un WebContent qui plante ne doit pas laisser la base SQL du
            // navigateur verrouillee pour le reste de la session.
            crate::kernel::abi::verrous::libere_processus(process.pid);
        }
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
            let process = current().process.borrow();
            process.zombie && process.pid == racine
        };
        if fini {
            let mut emportes = 0usize;
            for index in 0..tasks().len() {
                if tasks()[index].state == TaskState::Zombie {
                    continue;
                }
                let pid = tasks()[index].process.borrow().pid;
                if descend_de(pid, racine) {
                    tasks()[index].state = TaskState::Zombie;
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

    // Sur un AP, le contexte noyau appelant est la boucle idle : si ce CPU
    // n'a plus rien de runnable, on y revient immediatement. Les autres CPU
    // continuent independamment.
    let cpu_id = local_cpu();
    if cpu_id != 0 {
        let cur = current_index_raw();
        wake_sleepers();
        if let Some(next) = pick_next(cur, cpu_id) {
            switch_to(cur, next);
            unreachable!("task: reprise d'une tache terminee sur AP");
        }
        switch_to_kernel();
    }

    // BSP : conserve la semantique historique des lancements synchrones et du
    // desktop, mais ne choisit que des taches affinees CPU0.
    let cur = current_index_raw();
    let patience = 30 * crate::kernel::timer::TICKS_PER_SECOND;
    let mut idle_since = crate::kernel::timer::ticks();
    loop {
        wake_sleepers();
        if let Some(next) = pick_next(cur, 0) {
            if next != cur {
                switch_to(cur, next);
                unreachable!("task: reprise d'une tache terminee");
            }
        }
        if tasks().iter().all(|t| t.state == TaskState::Zombie) { break; }
        if crate::kernel::timer::ticks().wrapping_sub(idle_since) > patience {
            crate::kernel::dmesg::log("task: aucune tache executable CPU0 depuis 30 s, interblocage suppose");
            for task in tasks().iter_mut() {
                if task.runq_cpu == 0 && allowed_on(task, 0) { task.state = TaskState::Zombie; }
            }
            break;
        }
        let depth = smp_lock::suspend_for_schedule();
        cpu::wait_for_interrupt();
        smp_lock::resume_after_schedule(depth);
        if tasks().iter().any(|t| runnable_local(t, 0) || runnable_steal(t, 0)) {
            idle_since = crate::kernel::timer::ticks();
        }
    }
    switch_to_kernel()
}

/// Signale au parent qu'un de ses fils vient de se terminer.
///
/// Deux effets distincts, tous deux necessaires : `SIGCHLD` (que le parent
/// peut avoir choisi d'intercepter) et le reveil d'un `wait4` bloquant.
fn notify_parent_of_exit() {
    let (parent_pid, is_zombie) = {
        let process = current().process.borrow();
        (process.parent, process.zombie)
    };
    if !is_zombie || parent_pid == 0 {
        return;
    }
    for task in tasks().iter_mut() {
        if task.state == TaskState::Zombie {
            continue;
        }
        let matches = {
            let mut process = task.process.borrow_mut();
            if process.pid == parent_pid {
                process.signals.raise(crate::kernel::signal::SIGCHLD);
                true
            } else {
                false
            }
        };
        if matches && task.waiting_for_child {
            task.waiting_for_child = false;
            task.state = TaskState::Ready;
        }
    }
}

/// Recense les processus fils zombies d'un pid donne.
pub fn zombie_children(parent_pid: u32) -> Vec<(u32, i32)> {
    let mut out = Vec::new();
    for process in processes().iter() {
        let borrowed = process.borrow();
        if borrowed.parent == parent_pid && borrowed.zombie {
            out.push((borrowed.pid, borrowed.exit_code));
        }
    }
    out
}

/// Ce pid a-t-il encore des fils (zombies ou vivants) ?
pub fn has_children(parent_pid: u32) -> bool {
    processes().iter().any(|p| p.borrow().parent == parent_pid)
}

/// Retire un processus zombie de la table (il a ete recolte).
pub fn collect_child(pid: u32) {
    processes().retain(|p| p.borrow().pid != pid);
    crate::kernel::process::kill(pid);
}

/// Termine tous les threads du processus courant (`exit_group`).
pub fn exit_group(code: i32) -> ! {
    let (pid, tid, process) = {
        let task = current();
        (task.process.borrow().pid, task.tid, task.process.clone())
    };
    for task in tasks().iter_mut() {
        if task.tid != tid && task.process.borrow().pid == pid {
            task.state = TaskState::Zombie;
        }
    }

    // `exit_group` termine tous les autres threads du processus. Le thread
    // courant est donc le seul encore vivant; `exit_current` le decrementera
    // de 1 a 0 et rendra le processus zombie/recoltable par `wait4`.
    process.borrow_mut().threads = 1;

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
    let _kernel = smp_lock::enter();
    // Le thread racine d'un lancement synchrone doit revenir sur la pile
    // noyau de son CPU appelant. Lui seul est pince; les pthreads qu'il cree
    // naissent avec une affinite machine complete et peuvent etre balances.
    let caller_cpu = local_cpu();
    first.affinity_mask = 1u64 << caller_cpu;
    first.runq_cpu = caller_cpu as u8;
    first.last_cpu = caller_cpu as u8;
    let process = first.process.clone();
    let racine = process.borrow().pid;
    let index = register(first);
    let cpu_id = local_cpu();
    let to_ptr = unsafe {
        RACINE_PREMIER_PLAN.store(racine, Ordering::Release);
        let list = tasks();
        let ptr = &mut **list.get_mut(index).unwrap() as *mut Task;
        mark_task_running(&mut *ptr, cpu_id);
        ptr
    };
    set_current_index(index);
    unsafe { install(&mut *to_ptr); }
    let kernel_rsp = &mut kernel_ctx().rsp as *mut u64;
    let depth = smp_lock::suspend_for_schedule();
    unsafe { switch_context(kernel_rsp, (*to_ptr).ctx.rsp); }
    smp_lock::resume_after_schedule(depth);
    complete_retired();

    crate::kernel::vmm::activate_kernel();
    set_current_index(NO_TASK);
    RACINE_PREMIER_PLAN.store(0, Ordering::Release);
    let (code, pid) = {
        let borrowed = process.borrow();
        (borrowed.exit_code, borrowed.pid)
    };
    reap();
    for stale in processes().iter() {
        crate::kernel::process::kill(stale.borrow().pid);
    }
    processes().clear();
    crate::kernel::process::kill(pid);
    code
}

pub fn run_noyau(entree: fn() -> !, nom: &str) -> i32 {
    let _kernel = smp_lock::enter();
    if in_user_task() {
        crate::kernel::dmesg::log("task: run_noyau imbrique refuse");
        return -1;
    }
    let process = match new_process(nom, 0) {
        Some(process) => process,
        None => return -1,
    };
    let mut task = Task::new_kernel(process.clone(), entree);
    task.priorite = Priorite::Interactive;
    task.affinity_mask = 1;
    task.runq_cpu = 0;
    task.last_cpu = 0;
    let index = register(task);
    let to_ptr = unsafe {
        let list = tasks();
        let ptr = &mut **list.get_mut(index).unwrap() as *mut Task;
        mark_task_running(&mut *ptr, 0);
        ptr
    };
    set_current_index(index);
    unsafe { install(&mut *to_ptr); }
    let kernel_rsp = &mut kernel_ctx().rsp as *mut u64;
    let depth = smp_lock::suspend_for_schedule();
    unsafe { switch_context(kernel_rsp, (*to_ptr).ctx.rsp); }
    smp_lock::resume_after_schedule(depth);
    complete_retired();

    crate::kernel::vmm::activate_kernel();
    set_current_index(NO_TASK);
    let (code, pid) = {
        let borrowed = process.borrow();
        (borrowed.exit_code, borrowed.pid)
    };
    reap();
    for stale in processes().iter() {
        crate::kernel::process::kill(stale.borrow().pid);
    }
    processes().clear();
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
    // Les CURRENT per-CPU sont des indices stables. En SMP on ne compacte donc
    // jamais le Vec ; `register` recycle les slots zombies. En UP, conserver le
    // comportement historique est sans risque.
    if smp::schedulable_cpus() <= 1 {
        tasks().retain(|t| t.state != TaskState::Zombie);
    }
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
    for task in tasks().iter_mut() {
        if task.process.borrow().pid == pid {
            task.priorite = priorite;
        }
    }
}

/// Le processus est-il termine, et avec quel code ?
pub fn code_de_sortie(pid: u32) -> Option<i32> {
    processes().iter().find_map(|p| {
        let borrowed = p.borrow();
        if borrowed.pid == pid && borrowed.zombie {
            Some(borrowed.exit_code)
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
            let enfant = process.borrow();
            if enfant.parent == parent && !cibles.contains(&enfant.pid) {
                cibles.push(enfant.pid);
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
    let courant = try_current().map(|t| t.tid);
    for task in tasks().iter_mut() {
        if Some(task.tid) == courant {
            continue;
        }
        if task.process.borrow().pid == pid {
            task.state = TaskState::Zombie;
        }
    }
    if let Some(process) = process_by_pid(pid) {
        let mut borrowed = process.borrow_mut();
        borrowed.threads = 0;
        borrowed.exit_code = code;
        borrowed.zombie = true;
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
        (task.process.borrow().pid, task.tid)
    };
    for task in tasks().iter_mut() {
        if task.tid != tid && task.process.borrow().pid == pid {
            task.state = TaskState::Zombie;
        }
    }
}

/// Reveille les taches d'un processus qui dorment, pour qu'elles constatent
/// un signal en attente.
pub fn wake_for_signal(pid: u32) {
    for task in tasks().iter_mut() {
        if task.state == TaskState::Blocked && task.process.borrow().pid == pid {
            task.futex_key = 0;
            task.wait_queue_key = 0;
            task.wake_deadline_ns = 0;
            task.waiting_for_child = false;
            task.state = TaskState::Ready;
        }
    }
}

/// Endort la tache courante sur une WaitQueue. L'appelant doit avoir valide la
/// generation sous le BKL juste avant cet appel pour fermer le lost wakeup.
pub(crate) fn park_current_on(wait_queue_key: usize) {
    {
        let task = current();
        task.wait_queue_key = wait_queue_key;
        task.state = TaskState::Blocked;
    }
    while current().state == TaskState::Blocked {
        schedule();
    }
    current().wait_queue_key = 0;
}

/// Reveille au plus `limit` taches inscrites sur la queue.
pub(crate) fn wake_wait_queue(wait_queue_key: usize, limit: usize) -> usize {
    let _kernel = smp_lock::enter();
    let mut woke = 0;
    for task in tasks().iter_mut() {
        if woke == limit {
            break;
        }
        if task.state == TaskState::Blocked && task.wait_queue_key == wait_queue_key {
            task.wait_queue_key = 0;
            task.state = TaskState::Ready;
            woke += 1;
        }
    }
    if woke != 0 {
        smp::broadcast_reschedule();
    }
    woke
}

/// Y a-t-il un signal livrable pour la tache courante ?
///
/// Consulte par les attentes bloquantes (`poll`, `wait4`, futex) : une attente
/// sans limite de temps doit pouvoir etre interrompue par un signal.
pub fn signal_pending() -> bool {
    match try_current() {
        Some(task) => task.process.borrow().signals.next_deliverable().is_some(),
        None => false,
    }
}

/// Termine de force toutes les taches (utilise apres une faute fatale).
pub fn kill_all(code: i32) {
    for task in tasks().iter_mut() {
        task.state = TaskState::Zombie;
        task.process.borrow_mut().exit_code = code;
    }
}

// --- Preemption --------------------------------------------------------------

/// Appele par IRQ0 quand le timer a interrompu du code ring 3.
///
/// On ne commute que si une autre tache est prete : sinon on economise deux
/// changements de contexte par tick.

pub fn preempt_from_irq() {
    // BOUCHAUD_SMP4_DEADLOCK_FIX
    //
    // Une IRQ ne doit jamais attendre le BKL avec IF=0. Si le verrou est
    // occupe, on differe simplement la preemption.
    debug_assert!(
        !cpu::interrupts_enabled(),
        "task: preempt_from_irq appelee hors contexte IRQ"
    );

    stall_site_set(40, 0);
    let Some(kernel) = smp_lock::try_enter() else {
        stall_site_clear();
        request_deferred_preempt();
        return;
    };
    stall_site_set(41, 0);

    let cur = current_index_raw();
    if cur == NO_TASK {
        return;
    }

    complete_retired();
    wake_sleepers();

    let cpu_id = local_cpu();
    if ready_count_cpu(cpu_id) == 0 && stealable_count_cpu(cpu_id) == 0 {
        return;
    }

    let Some(next) = pick_next(cur, cpu_id) else {
        return;
    };
    if next == cur {
        return;
    }

    let (from_ptr, to_ptr) = unsafe {
        let list = tasks();
        let from_ptr = &mut **list.get_mut(cur).unwrap() as *mut Task;
        let to_ptr = &mut **list.get_mut(next).unwrap() as *mut Task;

        IRQ_PREEMPTIONS.fetch_add(1, Ordering::Relaxed);
        CONTEXT_SWITCHES.fetch_add(1, Ordering::Relaxed);
        account_slice_end(&mut *from_ptr);

        usermode::fxsave((*from_ptr).fpu_ptr() as *mut u8);
        (*from_ptr).fs_base = usermode::fs_base();
        deactivate_task_space(&*from_ptr, cpu_id);
        (*from_ptr).on_cpu = -1;
        (*from_ptr).last_cpu = cpu_id as u8;
        (*from_ptr).runq_cpu = cpu_id as u8;
        mark_task_running(&mut *to_ptr, cpu_id);

        set_current_index(next);
        install(&mut *to_ptr);
        (from_ptr, to_ptr)
    };

    // Le BKL est libere AVANT le switch IRQ. Quand cette pile IRQ sera reprise
    // plus tard avec IF=0, elle n'aura aucun BKL a reacquerir avant IRETQ.
    drop(kernel);
    // Le nouveau contexte ne doit pas heriter d'un tag "preempt kernel".
    stall_site_clear();
    unsafe { switch_context(&mut (*from_ptr).ctx.rsp, (*to_ptr).ctx.rsp); }

    // Ne jamais bloquer ici. Nettoyage opportuniste uniquement.
    stall_site_set(42, 0);
    if let Some(_kernel) = smp_lock::try_enter() {
        stall_site_set(43, 0);
        complete_retired();
    }
    stall_site_clear();
}

fn add_current_ticks(delta: u64) {
    let index = current_index_raw();
    if index == NO_TASK { return; }
    if let Some(task) = tasks().get_mut(index) {
        task.ticks_cpu = task.ticks_cpu.wrapping_add(delta);
    }
}

pub fn echantillonne(interrupted_user: bool) {
    if cpu::account_timer_tick(interrupted_user) { return; }
    add_current_ticks(1);
}

/// Compte uniquement la tache BSP apres que l'accounting machine a deja ete
/// fait hors BKL dans l'IRQ PIT.
pub fn echantillonne_tache_bsp() {
    add_current_ticks(1);
}

/// Echantillon des AP, cadence par IPI de quantum. Le PIT reste l'unique
/// horloge murale ; ici on ne fait que comptabiliser le temps CPU de la tache.
pub fn echantillonne_quantum(_interrupted_user: bool, ticks: u64) {
    add_current_ticks(ticks.max(1));
}

/// Snapshot SMP-NG2: charge physique, pression de runqueue, tache courante,
/// steals et migrations par CPU.
pub fn log_smp_load() {
    let _kernel = smp_lock::enter();
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
            (t.tid, t.process.borrow().pid)
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
    crate::kernel::dmesg::log_fmt(format_args!(
        "[BKL-STATS] wait_ns={} hold_ns={} acquisitions={}",
        bkl_wait, bkl_hold, bkl_acq,
    ));
}

/// Instantane d'un processus pour le journal : (pid, nom, ticks, octets).
pub struct Mesure {
    pub pid: u32,
    pub nom: String,
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
}

/// Compteurs de la derniere mesure, pour rendre un delta plutot qu'un cumul.
///
/// Un cumul depuis le demarrage ne dit rien d'utile : au bout d'une minute,
/// tout le monde a « beaucoup » de ticks. Ce qu'on veut lire, c'est ce qui s'est
/// passe depuis la ligne precedente du journal.
static mut MESURE_PRECEDENTE: Option<Vec<(u32, u64, [u64; MAX_CPUS])>> = None;
static mut MESURE_NS_PRECEDENT: u64 = 0;

/// Mesure tous les processus vivants et remet les compteurs a la reference.
///
/// Rend aussi le nombre total de ticks ecoules sur la periode, seul denominateur
/// honnete d'un pourcentage : compter sur l'horloge murale donnerait des totaux
/// qui depassent 100 % des que la machine dort.
pub fn mesure_processus() -> (Vec<Mesure>, u64) {
    let mut cumuls: Vec<(u32, u64, [u64; MAX_CPUS])> = Vec::new();
    let mut mesures: Vec<Mesure> = Vec::new();

    for task in tasks().iter() {
        if task.state == TaskState::Zombie {
            continue;
        }
        let (pid, nom, rss_octets, vss_octets) = {
            let process = task.process.borrow();
            let usage = crate::kernel::resource::memory_usage(&process);
            (process.pid, process.name.clone(), usage.rss, usage.vss)
        };
        let runtime = task.user_cpu_ns.saturating_add(task.kernel_cpu_ns);
        match cumuls.iter_mut().find(|(autre, _, _)| *autre == pid) {
            Some((_, total, cpu_map)) => {
                *total = total.saturating_add(runtime);
                for cpu in 0..MAX_CPUS {
                    cpu_map[cpu] = cpu_map[cpu].saturating_add(task.cpu_ns[cpu]);
                }
            }
            None => {
                cumuls.push((pid, runtime, task.cpu_ns));
                mesures.push(Mesure {
                    pid,
                    nom,
                    ticks: 0,
                    octets: rss_octets,
                    rss_octets,
                    vss_octets,
                    taches: 0,
                    cpu_map_ns: [0; MAX_CPUS],
                    migrations: 0,
                    context_switches: 0,
                });
            }
        }
        if let Some(mesure) = mesures.iter_mut().find(|m| m.pid == pid) {
            mesure.taches += 1;
            mesure.migrations = mesure.migrations.saturating_add(task.migrations);
            mesure.context_switches = mesure.context_switches.saturating_add(task.context_switches);
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
            .find(|(pid, _, _)| *pid == mesure.pid)
            .map_or(0, |(_, total, _)| *total);
        let avant = precedents
            .iter()
            .find(|(pid, _, _)| *pid == mesure.pid)
            .map_or(0, |(_, total, _)| *total);
        mesure.ticks = cumul.saturating_sub(avant);
        let current_map = cumuls
            .iter()
            .find(|(pid, _, _)| *pid == mesure.pid)
            .map_or([0; MAX_CPUS], |(_, _, map)| *map);
        let previous_map = precedents
            .iter()
            .find(|(pid, _, _)| *pid == mesure.pid)
            .map_or([0; MAX_CPUS], |(_, _, map)| *map);
        for cpu in 0..MAX_CPUS {
            mesure.cpu_map_ns[cpu] = current_map[cpu].saturating_sub(previous_map[cpu]);
        }
    }

    let now = crate::kernel::timer::monotonic_ns();
    let previous_ns = unsafe { MESURE_NS_PRECEDENT };
    let window = if previous_ns == 0 {
        now.max(1)
    } else {
        now.saturating_sub(previous_ns).max(1)
    };
    unsafe {
        MESURE_PRECEDENTE = Some(cumuls);
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

pub fn watchdog_from_timer() {
    if !WM_WATCHDOG_ARMED.load(Ordering::Acquire) { return; }
    let now = crate::kernel::timer::ticks();
    let heartbeat = WM_HEARTBEAT_TICK.load(Ordering::Relaxed);
    let last_warning = WM_LAST_WARNING_TICK.load(Ordering::Relaxed);
    let silence = now.wrapping_sub(heartbeat);
    let seuil = 2 * crate::kernel::timer::TICKS_PER_SECOND;
    if silence >= seuil && now.wrapping_sub(last_warning) >= seuil {
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
        wm_age_ms: now.saturating_sub(heartbeat).saturating_mul(1000)
            / crate::kernel::timer::TICKS_PER_SECOND,
        ready: ready_count(),
        live: live_count(),
    }
}

// --- Sommeil -----------------------------------------------------------------

/// Endort la tache courante pendant `ticks` ticks du timer.

/// Attente d'une boucle `poll`/`select`/`epoll_wait` qui n'a rien trouve de pret.
///
/// ## Le defaut que cette fonction corrige
///
/// Les trois boucles faisaient, chacune a sa facon :
///
/// ```text
/// if task::schedule() { continue; }        // une autre tache est prete
/// cpu::wait_for_interrupt();               // personne d'autre : on dort
/// ```
///
/// Le raisonnement est juste tache par tache, et faux pour la machine. Un
/// bureau Bouchaud avec le navigateur ouvert compte quatre processus — le
/// compositeur, le bootstrap, WebContent, RequestServer — et quand ils
/// attendent tous, ils attendent **tous dans `poll`**. Chacun reste alors
/// `Ready` du point de vue de l'ordonnanceur : `schedule()` trouve toujours un
/// autre candidat, rend `true`, et personne n'atteint jamais le `hlt`. Les
/// quatre se relaient a plein regime pour ne rien faire.
///
/// C'est ce qui affichait 100 % de processeur des le demarrage, avant meme
/// qu'une page soit demandee, et ce que le releve par processus montrait sans
/// qu'on le lise ainsi : la somme des parts valait le cœur entier alors
/// qu'aucun des quatre ne progressait.
///
/// ## Ce qu'elle fait
///
/// Elle marque la tache **bloquee** pour un tick, ce que `sleep_ticks` sait
/// deja faire. Les autres taches reellement pretes continuent d'etre elues ;
/// quand plus aucune ne l'est, `schedule()` tombe sur son propre
/// `wait_for_interrupt()` et le processeur s'arrete pour de bon — ce que la
/// mesure de charge compte enfin comme du repos.
///
/// La latence ne change pas : le tick vaut une milliseconde, exactement le
/// delai qu'imposait deja le `hlt` reveille par l'horloge. Le reveil logiciel
/// n'est pas perdu non plus — une tache qui rend un descripteur pret pendant
/// notre tick le trouvera pret a notre reveil, un tour de boucle plus tard.
// BOUCHAUD_CPU_OPT_ADAPTIVE_IO: attente adaptative pour poll/select/epoll.
//
// La pile reseau Bouchaud reste aujourd'hui pilotee par interrogation. Bloquer
// sans limite ici serait donc incorrect : personne ne pomperait les paquets.
// On garde une phase tres reactive a 1 ms, puis on espace progressivement les
// tours vides jusqu'a 8 ms. Une I/O qui repart quitte son syscall et le compteur
// local repart automatiquement de zero a l'appel suivant.
pub fn attends_io_adaptatif(tours_vides: &mut u32) {
    let ticks = match *tours_vides {
        0..=2 => 1,
        3..=7 => 2,
        8..=15 => 4,
        _ => 8,
    };
    *tours_vides = tours_vides.saturating_add(1).min(64);
    sleep_ticks(ticks);
}

/// Attente historique stricte d'un tick. Conservee pour les chemins ou la
/// latence minimale prime (console, audio, petites attentes protocolaires).
pub fn attends_un_tick() {
    sleep_ticks(1);
}

pub fn sleep_ticks(ticks: u64) {
    let duration_ns = ticks.max(1)
        .saturating_mul(1_000_000_000 / crate::kernel::timer::TICKS_PER_SECOND);
    let deadline = crate::kernel::timer::monotonic_ns().saturating_add(duration_ns);
    {
        let task = current();
        task.wake_deadline_ns = deadline;
        task.state = TaskState::Blocked;
    }
    while crate::kernel::timer::monotonic_ns() < deadline {
        // schedule() fait deja HLT si la tache est bloquee et seule.
        schedule();
        if current().state == TaskState::Ready {
            break;
        }
    }
    let task = current();
    task.wake_deadline_ns = 0;
    task.state = TaskState::Ready;
}

/// Reveille les taches dont le sommeil est echu, et declenche les `SIGALRM`.
fn wake_sleepers() {
    let now = crate::kernel::timer::monotonic_ns();
    let mut woke = false;
    for task in tasks().iter_mut() {
        if task.state == TaskState::Blocked
            && task.wake_deadline_ns != 0
            && now >= task.wake_deadline_ns
        {
            task.wake_deadline_ns = 0;
            task.futex_key = 0;
            task.state = TaskState::Ready;
            woke = true;
        }
    }
    fire_alarms(now);
    if woke { smp::broadcast_reschedule(); }
}

/// Echeance du prochain `SIGALRM` par processus : (pid, tick).
static mut ALARMS: Option<Vec<(u32, u64)>> = None;

fn alarms() -> &'static mut Vec<(u32, u64)> {
    unsafe {
        if ALARMS.is_none() {
            ALARMS = Some(Vec::new());
        }
        ALARMS.as_mut().unwrap()
    }
}

/// Programme (ou annule, avec 0) l'alarme du processus courant.
/// Renvoie l'echeance precedente, 0 s'il n'y en avait pas.
pub fn set_alarm(deadline: u64) -> u64 {
    let pid = current().process.borrow().pid;
    let list = alarms();
    let previous = list
        .iter()
        .find(|(p, _)| *p == pid)
        .map(|(_, t)| *t)
        .unwrap_or(0);
    list.retain(|(p, _)| *p != pid);
    if deadline != 0 {
        list.push((pid, deadline));
    }
    previous
}

/// Echeance de l'alarme du processus courant (0 s'il n'y en a pas).
pub fn peek_alarm() -> u64 {
    let pid = current().process.borrow().pid;
    alarms()
        .iter()
        .find(|(p, _)| *p == pid)
        .map(|(_, t)| *t)
        .unwrap_or(0)
}

/// Leve les `SIGALRM` dont l'echeance est atteinte.
fn fire_alarms(now: u64) {
    let expired: Vec<u32> = alarms()
        .iter()
        .filter(|(_, deadline)| now >= *deadline)
        .map(|(pid, _)| *pid)
        .collect();
    if expired.is_empty() {
        return;
    }
    alarms().retain(|(_, deadline)| now < *deadline);
    for pid in expired {
        if let Some(process) = process_by_pid(pid) {
            process
                .borrow_mut()
                .signals
                .raise(crate::kernel::signal::SIGALRM);
        }
        wake_for_signal(pid);
    }
}

/// Cede le CPU une fois (`sched_yield`).
pub fn yield_now() {
    schedule();
}

// --- Futex -------------------------------------------------------------------

/// Cle d'attente d'un futex : adresse physique du mot surveille, pour que deux
/// threads partageant la page s'accordent meme via des adresses virtuelles
/// differentes.
fn futex_key(uaddr: u64) -> u64 {
    let process = current().process.clone();
    let mut process = process.borrow_mut();
    process.space.translate(uaddr).unwrap_or(uaddr)
}

/// `FUTEX_WAIT` : endort la tache si `*uaddr == expected`.
///
/// `timeout_ms` a 0 signifie « sans limite ». Renvoie `true` si la tache a
/// ete reveillee par un `FUTEX_WAKE`, `false` sur delai expire.
pub fn futex_wait(uaddr: u64, expected: u32, timeout_ms: u64) -> bool {
    let key = futex_key(uaddr);
    // Verification atomique vis-a-vis des autres taches : le noyau n'est pas
    // preemptible ici, donc lire puis dormir est indivisible.
    let mut value = [0u8; 4];
    {
        let process = current().process.clone();
        let mut process = process.borrow_mut();
        if !process.space.read(uaddr, &mut value) {
            return false;
        }
    }
    if u32::from_le_bytes(value) != expected {
        return true; // EAGAIN cote appelant
    }

    let deadline_ns = if timeout_ms == 0 {
        0
    } else {
        crate::kernel::timer::monotonic_ns()
            .saturating_add(timeout_ms.saturating_mul(1_000_000))
    };
    {
        let task = current();
        task.futex_key = key;
        task.wake_deadline_ns = 0;
        task.state = TaskState::Blocked;
    }

    loop {
        if !schedule() {
            // schedule() a deja dormi si cette tache etait la seule runnable.
            // Ne jamais refaire HLT apres reprise du BKL de syscall.
            wake_sleepers();
        }
        // L'ordre des deux tests compte. `wake_sleepers` remet la tache en
        // `Ready` des que son echeance est atteinte, exactement comme le ferait
        // un `FUTEX_WAKE` : tester l'etat en premier ferait passer tout delai
        // expire pour un reveil. La libc croirait alors avoir ete signalee, se
        // rendormirait pour la meme duree, et `pthread_cond_timedwait`
        // attendrait un multiple de ce qu'on lui a demande.
        let expired = deadline_ns != 0
            && crate::kernel::timer::monotonic_ns() >= deadline_ns;
        let task = current();
        if expired {
            task.futex_key = 0;
            task.wake_deadline_ns = 0;
            task.state = TaskState::Ready;
            return false;
        }
        if task.state == TaskState::Ready {
            task.futex_key = 0;
            task.wake_deadline_ns = 0;
            return true;
        }
    }
}

/// `FUTEX_WAKE` : reveille jusqu'a `count` taches en attente sur `uaddr`.
/// Renvoie le nombre de taches reveillees.
pub fn futex_wake(uaddr: u64, count: u32) -> u32 {
    let key = futex_key(uaddr);
    let mut woken = 0;
    for task in tasks().iter_mut() {
        if woken >= count {
            break;
        }
        if task.state == TaskState::Blocked && task.futex_key == key {
            task.futex_key = 0;
            task.wake_deadline_ns = 0;
            task.state = TaskState::Ready;
            woken += 1;
        }
    }
    if woken > 0 {
        smp::broadcast_reschedule();
    }
    woken
}

// --- Diagnostic --------------------------------------------------------------

/// Affiche la table des taches utilisateur (commande `tasks`).
pub fn print_table() {
    let list = tasks();
    if list.is_empty() {
        crate::println!("aucune tache utilisateur (ring 3) active");
        return;
    }
    crate::println!("  TID  PID  ETAT      PAGES  NOM");
    for task in list.iter() {
        let process = task.process.borrow();
        let state = match task.state {
            TaskState::Ready => "ready",
            TaskState::Blocked => "blocked",
            TaskState::Zombie => "zombie",
        };
        crate::println!(
            "  {:>3}  {:>3}  {:<8}  {:>5}  {}",
            task.tid,
            process.pid,
            state,
            process.space.mapped_pages(),
            process.name
        );
    }
}

/// Cree un processus vide (espace d'adressage neuf, descripteurs standards).
pub fn new_process(name: &str, cwd: usize) -> Option<Rc<RefCell<Process>>> {
    let space = AddressSpace::new()?;
    let pid = crate::kernel::process::spawn(name, crate::users::session().uid());
    let process = Rc::new(RefCell::new(Process {
        pid,
        parent: 0,
        name: name.to_string(),
        space,
        files: FdTable::new(),
        brk_start: 0,
        brk: 0,
        mmap_next: crate::kernel::vmm::user_mmap_base(),
        cwd,
        exit_code: 0,
        zombie: false,
        threads: 1,
        uid: crate::users::session().uid() as u32,
        gid: crate::users::session().uid() as u32,
        signals: crate::kernel::signal::SignalState::default(),
        partages: Vec::new(),
        limite_as: 0,
        promesses: Vec::new(),
        ecran: None,
    }));
    processes().push(process.clone());
    Some(process)
}

/// Enregistre un processus cree par `fork` (espace deja duplique).
pub fn register_process(process: Rc<RefCell<Process>>) {
    processes().push(process);
}
