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

use crate::arch::x86_64::cpu;
use crate::arch::x86_64::usermode::{self, TrapFrame};
use crate::kernel::fd::FdTable;
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
/// Huit : assez pour que l'interface enchaine plusieurs reveils courts sans
/// interruption, assez peu pour qu'une tache de fond conserve au moins un
/// neuvieme du temps dans le pire des cas.
const TOURS_INTERACTIFS_MAX: u32 = 8;

static mut TOURS_INTERACTIFS: u32 = 0;

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

/// Peuple a la demande une page promise.
///
/// La derniere promesse couvrant la page fixe les droits. Toutes les promesses
/// file-backed couvrant cette meme page contribuent ensuite leurs octets : deux
/// PT_LOAD ELF peuvent legalement partager une page de bordure.
pub fn peuple_a_la_demande(adresse: u64) -> bool {
    if !crate::kernel::vmm::is_user_addr(adresse) || !in_user_task() {
        return false;
    }

    let processus = current_process();
    let mut p = processus.borrow_mut();
    let page = adresse & !(crate::kernel::vmm::PAGE_SIZE - 1);

    let effective = match crate::kernel::vma::trouve(&p.promesses, page) {
        Some(region) => region,
        None => return false,
    };

    if effective.drapeaux & crate::kernel::vmm::PTE_USER == 0 {
        return false;
    }

    if p.space.translate(page).is_some() {
        return false;
    }

    match effective.backing {
        PromesseBacking::Zero => {
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
            true
        }

        PromesseBacking::File { .. } => {
            if !p.space.map_alloc(
                page,
                crate::kernel::vmm::PAGE_SIZE,
                effective.drapeaux,
            ) {
                return false;
            }

            // Plusieurs PT_LOAD peuvent partager une page. On compose tous les
            // fragments file-backed qui la couvrent.
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
                let got = crate::fs::backing::read_at(
                    node,
                    source_offset as usize,
                    &mut buffer[..wanted],
                );

                if got != wanted || !p.space.write(start, &buffer[..got]) {
                    p.space.unmap(page, crate::kernel::vmm::PAGE_SIZE);
                    return false;
                }
            }

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
    /// Tick a partir duquel un sommeil se termine (0 = pas de sommeil).
    pub wake_tick: u64,
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
/// Tous les processus vivants ou zombies, y compris ceux dont plus aucune
/// tache ne tourne : c'est ce qui permet a `wait4` de retrouver un fils
/// termine longtemps apres sa mort.
static mut PROCESSES: Option<Vec<Rc<RefCell<Process>>>> = None;
static mut CURRENT: usize = usize::MAX;
static mut NEXT_TID: u32 = 100;
/// Contexte du fil noyau (shell/bureau) qui a lance le programme.
static mut KERNEL_CTX: Context = Context { rsp: 0 };
/// Une tache utilisateur a-t-elle demande une preemption ?
static mut NEED_RESCHED: bool = false;

/// Miroir IRQ-safe du type de tache courante : evite de parcourir TASKS quand
/// IRQ0 interrompt un syscall en ring 0.
static mut CURRENT_IS_KERNEL: bool = false;

static mut CONTEXT_SWITCHES: u64 = 0;
static mut IRQ_PREEMPTIONS: u64 = 0;
static mut DEFERRED_PREEMPTIONS: u64 = 0;
static mut WM_HEARTBEAT_TICK: u64 = 0;
static mut WM_WATCHDOG_ARMED: bool = false;
static mut WM_LAST_WARNING_TICK: u64 = 0;

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
    unsafe {
        let tid = NEXT_TID;
        NEXT_TID += 1;
        tid
    }
}

/// Y a-t-il une tache utilisateur en cours ?
pub fn in_user_task() -> bool {
    unsafe { CURRENT != usize::MAX }
}

/// Tache courante.
///
/// # Panique
/// Si aucune tache utilisateur n'est active (appel depuis le shell).
pub fn current() -> &'static mut Task {
    let index = unsafe { CURRENT };
    assert!(index != usize::MAX, "task: aucune tache utilisateur active");
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
            frame,
            ctx: Context::default(),
            kstack,
            kstack_top,
            fpu,
            fpu_area,
            fs_base: 0,
            clear_child_tid: 0,
            futex_key: 0,
            wake_tick: 0,
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

/// Ajoute une tache a la table et renvoie son indice.
pub fn register(task: Box<Task>) -> usize {
    let list = tasks();
    list.push(task);
    list.len() - 1
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
    tasks()
        .iter()
        .filter(|t| t.state == TaskState::Ready)
        .count()
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

/// Point d'entree d'une tache neuve : installe son contexte puis part en ring 3.
extern "C" fn task_trampoline() -> ! {
    let task = current();
    install(task);
    let frame = task.frame;
    unsafe { usermode::resume_usermode(&frame) }
}

/// Point d'entree d'un fil noyau : appelle sa fonction, qui ne revient pas.
extern "C" fn kernel_task_trampoline() -> ! {
    let task = current();
    task.fresh = false;
    let entree = task
        .entree_noyau
        .expect("task: fil noyau sans point d'entree");
    entree()
}

/// Installe le contexte materiel d'une tache : espace d'adressage, pile noyau,
/// base FS (TLS) et etat FPU.
fn install(task: &mut Task) {
    unsafe {
        CURRENT_IS_KERNEL = task.noyau;
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
fn pick_next(after: usize) -> Option<usize> {
    let len = tasks().len();
    if len == 0 {
        return None;
    }

    let force_partage = unsafe { TOURS_INTERACTIFS >= TOURS_INTERACTIFS_MAX };
    if !force_partage {
        for offset in 1..=len {
            let index = (after.wrapping_add(offset)) % len;
            let candidate = &tasks()[index];
            if candidate.state == TaskState::Ready && candidate.priorite == Priorite::Interactive {
                unsafe { TOURS_INTERACTIFS += 1 };
                return Some(index);
            }
        }
    }

    for offset in 1..=len {
        let index = (after.wrapping_add(offset)) % len;
        if tasks()[index].state == TaskState::Ready {
            // Une tache normale a eu la main : le compte repart. C'est ce qui
            // rend la borne glissante plutot que definitive.
            if tasks()[index].priorite == Priorite::Normale {
                unsafe { TOURS_INTERACTIFS = 0 };
            } else {
                unsafe { TOURS_INTERACTIFS += 1 };
            }
            return Some(index);
        }
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

/// Rend la main : bascule sur une autre tache prete s'il y en a une.
///
/// Renvoie `true` si un changement de tache a eu lieu. Si la tache courante est
/// la seule prete, la fonction attend une interruption (`hlt`) et rend la main a
/// l'appelant, qui doit reevaluer sa condition d'attente.
pub fn schedule() -> bool {
    let cur = unsafe { CURRENT };
    if cur == usize::MAX {
        return false;
    }
    debug_assert_borrows_released();
    debug_assert_interrupts_enabled();
    wake_sleepers();
    let next = match pick_next(cur) {
        Some(n) if n != cur => n,
        _ => {
            // Personne d'autre : si la tache courante est bloquee, on attend une
            // interruption plutot que de bruler du CPU.
            if tasks()[cur].state != TaskState::Ready {
                cpu::wait_for_interrupt();
            }
            return false;
        }
    };
    switch_to(cur, next);
    true
}

/// Bascule de la tache `from` vers la tache `to`.
fn switch_to(from: usize, to: usize) {
    unsafe {
        CONTEXT_SWITCHES = CONTEXT_SWITCHES.wrapping_add(1);
        let list = tasks();
        let from_ptr = &mut **list.get_mut(from).unwrap() as *mut Task;
        let to_ptr = &mut **list.get_mut(to).unwrap() as *mut Task;

        usermode::fxsave((*from_ptr).fpu_ptr() as *mut u8);
        (*from_ptr).fs_base = usermode::fs_base();

        CURRENT = to;
        install(&mut *to_ptr);

        switch_context(&mut (*from_ptr).ctx.rsp, (*to_ptr).ctx.rsp);
    }
}

/// Retour definitif au fil noyau appelant (la tache courante est terminee).
fn switch_to_kernel() -> ! {
    unsafe {
        let cur = CURRENT;
        let list = tasks();
        let from_ptr = &mut **list.get_mut(cur).unwrap() as *mut Task;
        CURRENT = usize::MAX;
        CURRENT_IS_KERNEL = false;
        usermode::per_cpu().current = 0;
        crate::kernel::vmm::activate_kernel();
        switch_context(&mut (*from_ptr).ctx.rsp, KERNEL_CTX.rsp);
    }
    // `switch_context` ne revient jamais ici : la tache est zombie.
    unreachable!("task: reprise d'une tache terminee")
}

/// Marque la tache courante terminee et rend la main.
///
/// Si d'autres threads du programme tournent encore, on bascule sur eux ;
/// sinon, retour au fil noyau qui a lance le programme.
pub fn exit_current(code: i32) -> ! {
    let cur = unsafe { CURRENT };
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
        }
    }

    // Previent le parent : SIGCHLD, et reveil s'il attendait dans `wait4`.
    notify_parent_of_exit();

    // On ne rend la main au noyau que lorsque **plus aucune** tache ne peut
    // reprendre. Se contenter de « aucune tache prete a cet instant » serait
    // faux : au moment ou un processus fils se termine, son parent est souvent
    // endormi (il attend justement cet evenement). Abandonner la aurait
    // demonte tout le programme au lieu de reveiller le parent.
    //
    // Filet de securite : si rien ne redevient executable pendant une longue
    // duree, c'est un interblocage franc. On termine plutot que de figer le
    // systeme, faute de Ctrl-C a offrir a l'utilisateur.
    let patience = 30 * crate::kernel::timer::TICKS_PER_SECOND;
    let mut idle_since = crate::kernel::timer::ticks();
    loop {
        wake_sleepers();
        if let Some(next) = pick_next(cur) {
            if next != cur {
                // La pile noyau de la tache morte reste vivante jusqu'au
                // nettoyage fait par `reap` depuis le fil noyau.
                unsafe {
                    let list = tasks();
                    let from_ptr = &mut **list.get_mut(cur).unwrap() as *mut Task;
                    let to_ptr = &mut **list.get_mut(next).unwrap() as *mut Task;
                    CURRENT = next;
                    install(&mut *to_ptr);
                    switch_context(&mut (*from_ptr).ctx.rsp, (*to_ptr).ctx.rsp);
                }
                unreachable!("task: reprise d'une tache terminee")
            }
        }
        if tasks().iter().all(|t| t.state == TaskState::Zombie) {
            break;
        }
        if crate::kernel::timer::ticks().wrapping_sub(idle_since) > patience {
            crate::kernel::dmesg::log(
                "task: aucune tache executable depuis 30 s, interblocage suppose",
            );
            for task in tasks().iter_mut() {
                task.state = TaskState::Zombie;
            }
            break;
        }
        cpu::wait_for_interrupt();
        // Le compteur repart des qu'une tache redevient prete.
        if tasks().iter().any(|t| t.state == TaskState::Ready) {
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
/// contexte du fil qui attend deja, et `CURRENT = usize::MAX` a la sortie
/// effacerait l'identite de la tache appelante. Les appelants verifient
/// [`in_user_task`] avant d'arriver ici — voir `exec::exec_image`.
pub fn run(first: Box<Task>) -> i32 {
    let process = first.process.clone();
    let index = register(first);
    unsafe {
        CURRENT = index;
        let list = tasks();
        let to_ptr = &mut **list.get_mut(index).unwrap() as *mut Task;
        install(&mut *to_ptr);
        switch_context(&mut KERNEL_CTX.rsp, (*to_ptr).ctx.rsp);
    }
    // Retour ici quand plus aucune tache n'est prete.
    crate::kernel::vmm::activate_kernel();
    unsafe { CURRENT = usize::MAX };
    let (code, pid) = {
        let borrowed = process.borrow();
        (borrowed.exit_code, borrowed.pid)
    };
    reap();
    // Tout ce qui reste est orphelin : le programme de premier plan est fini,
    // ses eventuels fils non recoltes n'ont plus personne pour les attendre.
    for stale in processes().iter() {
        crate::kernel::process::kill(stale.borrow().pid);
    }
    processes().clear();
    crate::kernel::process::kill(pid);
    code
}

/// Lance un fil noyau depuis le fil appelant et attend qu'il ait fini.
///
/// C'est `run` pour du code ring 0. Le gestionnaire de fenetres passe par la :
/// il devient une tache comme les autres, l'ordonnanceur peut donc lui rendre la
/// main pendant qu'un programme ring 3 tourne, ce qu'un simple appel de fonction
/// depuis le shell ne permettait pas.
pub fn run_noyau(entree: fn() -> !, nom: &str) -> i32 {
    // Meme invariant que `run` : un seul `KERNEL_CTX`, donc un seul fil noyau
    // appelant a la fois.
    if in_user_task() {
        crate::kernel::dmesg::log("task: run_noyau imbrique refuse");
        return -1;
    }
    // Racine comme repertoire courant : un fil noyau n'ouvre pas de chemin
    // relatif, et la valeur n'existe que parce que `Process` la porte.
    let process = match new_process(nom, 0) {
        Some(process) => process,
        None => return -1,
    };
    let mut task = Task::new_kernel(process.clone(), entree);
    // Le serveur graphique est de l'UI : meme classe interactive que le navigateur.
    task.priorite = Priorite::Interactive;
    let index = register(task);
    unsafe {
        CURRENT = index;
        let list = tasks();
        let to_ptr = &mut **list.get_mut(index).unwrap() as *mut Task;
        install(&mut *to_ptr);
        switch_context(&mut KERNEL_CTX.rsp, (*to_ptr).ctx.rsp);
    }
    crate::kernel::vmm::activate_kernel();
    unsafe { CURRENT = usize::MAX };
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
    tasks().retain(|t| t.state != TaskState::Zombie);
}

/// Meme chose, appelable depuis une tache vivante.
///
/// Retirer une tache decale toutes les suivantes dans le `Vec` ; `CURRENT`,
/// qui est un indice, designerait alors quelqu'un d'autre — c'est-a-dire que la
/// prochaine commutation sauverait le contexte de la tache courante par-dessus
/// celui d'une autre. On retrouve donc l'indice par le tid, seule identite
/// stable.
pub fn nettoie_zombies() {
    let cur = unsafe { CURRENT };
    if cur == usize::MAX {
        reap();
        return;
    }
    let tid = tasks()[cur].tid;
    tasks().retain(|t| t.state != TaskState::Zombie);
    if let Some(index) = index_of(tid) {
        unsafe { CURRENT = index };
    }
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
            task.wake_tick = 0;
            task.waiting_for_child = false;
            task.state = TaskState::Ready;
        }
    }
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
    if unsafe { CURRENT } == usize::MAX {
        return;
    }
    wake_sleepers();
    if ready_count() < 2 {
        return;
    }
    let cur = unsafe { CURRENT };
    if let Some(next) = pick_next(cur) {
        if next != cur {
            unsafe {
                IRQ_PREEMPTIONS = IRQ_PREEMPTIONS.wrapping_add(1);
            }
            switch_to(cur, next);
        }
    }
}

/// Compte un tick du timer pour la tache courante.
///
/// Appele par IRQ0, y compris quand le timer interrompt du code noyau : le
/// bureau est une tache comme les autres, et le temps qu'il passe a composer
/// doit se voir au meme titre que celui du navigateur.
pub fn echantillonne(interrupted_user: bool) {
    if cpu::account_timer_tick(interrupted_user) {
        return;
    }

    let index = unsafe { CURRENT };
    if index == usize::MAX {
        return;
    }
    if let Some(task) = tasks().get_mut(index) {
        task.ticks_cpu = task.ticks_cpu.wrapping_add(1);
    }
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
}

/// Compteurs de la derniere mesure, pour rendre un delta plutot qu'un cumul.
///
/// Un cumul depuis le demarrage ne dit rien d'utile : au bout d'une minute,
/// tout le monde a « beaucoup » de ticks. Ce qu'on veut lire, c'est ce qui s'est
/// passe depuis la ligne precedente du journal.
static mut MESURE_PRECEDENTE: Option<Vec<(u32, u64)>> = None;
static mut MESURE_TICK_PRECEDENT: u64 = 0;

/// Mesure tous les processus vivants et remet les compteurs a la reference.
///
/// Rend aussi le nombre total de ticks ecoules sur la periode, seul denominateur
/// honnete d'un pourcentage : compter sur l'horloge murale donnerait des totaux
/// qui depassent 100 % des que la machine dort.
pub fn mesure_processus() -> (Vec<Mesure>, u64) {
    let mut cumuls: Vec<(u32, u64)> = Vec::new();
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
        match cumuls.iter_mut().find(|(autre, _)| *autre == pid) {
            Some((_, ticks)) => *ticks += task.ticks_cpu,
            None => {
                cumuls.push((pid, task.ticks_cpu));
                mesures.push(Mesure {
                    pid,
                    nom,
                    ticks: 0,
                    octets: rss_octets,
                    rss_octets,
                    vss_octets,
                    taches: 0,
                });
            }
        }
        if let Some(mesure) = mesures.iter_mut().find(|m| m.pid == pid) {
            mesure.taches += 1;
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
            .find(|(pid, _)| *pid == mesure.pid)
            .map_or(0, |(_, t)| *t);
        let avant = precedents
            .iter()
            .find(|(pid, _)| *pid == mesure.pid)
            .map_or(0, |(_, t)| *t);
        mesure.ticks = cumul.saturating_sub(avant);
    }

    let now = crate::kernel::timer::ticks();
    let previous_tick = unsafe { MESURE_TICK_PRECEDENT };
    let window = if previous_tick == 0 {
        now.max(1)
    } else {
        now.wrapping_sub(previous_tick).max(1)
    };
    unsafe {
        MESURE_PRECEDENTE = Some(cumuls);
        MESURE_TICK_PRECEDENT = now;
    }
    (mesures, window)
}

/// Signale qu'une commutation est souhaitable au prochain point sur.
pub fn set_need_resched() {
    unsafe { NEED_RESCHED = true };
}

/// Consomme le drapeau de preemption.
pub fn take_need_resched() -> bool {
    unsafe {
        let value = NEED_RESCHED;
        NEED_RESCHED = false;
        value
    }
}

/// Le timer a interrompu du ring 0 appartenant a une tache utilisateur.
/// On ne commute pas au milieu du noyau : `abi::handle` consommera la demande.
pub fn request_deferred_preempt() {
    unsafe {
        DEFERRED_PREEMPTIONS = DEFERRED_PREEMPTIONS.wrapping_add(1);
        NEED_RESCHED = true;
    }
}

/// Lecture IRQ-safe, sans parcourir TASKS.
pub fn current_is_kernel_task() -> bool {
    unsafe { core::ptr::read_volatile(&raw const CURRENT_IS_KERNEL) }
}

/// Heartbeat du compositor.
pub fn note_wm_heartbeat() {
    unsafe {
        WM_HEARTBEAT_TICK = crate::kernel::timer::ticks();
        WM_WATCHDOG_ARMED = true;
    }
}

/// Watchdog IRQ0 : si le timer vit mais pas le WM, le journal serie continue.
pub fn watchdog_from_timer() {
    let now = crate::kernel::timer::ticks();
    let (armed, heartbeat, last_warning, switches, irq, deferred) = unsafe {
        (
            core::ptr::read_volatile(&raw const WM_WATCHDOG_ARMED),
            core::ptr::read_volatile(&raw const WM_HEARTBEAT_TICK),
            core::ptr::read_volatile(&raw const WM_LAST_WARNING_TICK),
            core::ptr::read_volatile(&raw const CONTEXT_SWITCHES),
            core::ptr::read_volatile(&raw const IRQ_PREEMPTIONS),
            core::ptr::read_volatile(&raw const DEFERRED_PREEMPTIONS),
        )
    };
    if !armed {
        return;
    }
    let silence = now.wrapping_sub(heartbeat);
    let seuil = 2 * crate::kernel::timer::TICKS_PER_SECOND;
    if silence >= seuil && now.wrapping_sub(last_warning) >= seuil {
        unsafe {
            WM_LAST_WARNING_TICK = now;
        }
        crate::serial_println!(
            "[sched-watchdog] desktop sans heartbeat depuis {} ms ; switches={} irq-preempt={} deferred={}",
            silence.saturating_mul(1000) / crate::kernel::timer::TICKS_PER_SECOND,
            switches, irq, deferred
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
    let (switches, irq, deferred, heartbeat) = unsafe {
        (
            CONTEXT_SWITCHES,
            IRQ_PREEMPTIONS,
            DEFERRED_PREEMPTIONS,
            WM_HEARTBEAT_TICK,
        )
    };
    OrdonnanceurStats {
        switches,
        irq_preemptions: irq,
        deferred_preemptions: deferred,
        wm_age_ms: now.saturating_sub(heartbeat).saturating_mul(1000)
            / crate::kernel::timer::TICKS_PER_SECOND,
        ready: ready_count(),
        live: live_count(),
    }
}

// --- Sommeil -----------------------------------------------------------------

/// Endort la tache courante pendant `ticks` ticks du timer.

pub fn sleep_ticks(ticks: u64) {
    let deadline = crate::kernel::timer::ticks() + ticks.max(1);
    {
        let task = current();
        task.wake_tick = deadline;
        task.state = TaskState::Blocked;
    }
    while crate::kernel::timer::ticks() < deadline {
        // schedule() fait deja HLT si la tache est bloquee et seule.
        schedule();
        if current().state == TaskState::Ready {
            break;
        }
    }
    let task = current();
    task.wake_tick = 0;
    task.state = TaskState::Ready;
}

/// Reveille les taches dont le sommeil est echu, et declenche les `SIGALRM`.
fn wake_sleepers() {
    let now = crate::kernel::timer::ticks();
    for task in tasks().iter_mut() {
        if task.state == TaskState::Blocked && task.wake_tick != 0 && now >= task.wake_tick {
            task.wake_tick = 0;
            task.futex_key = 0;
            task.state = TaskState::Ready;
        }
    }
    fire_alarms(now);
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
/// `timeout_ticks` a 0 signifie « sans limite ». Renvoie `true` si la tache a
/// ete reveillee par un `FUTEX_WAKE`, `false` sur delai expire.
pub fn futex_wait(uaddr: u64, expected: u32, timeout_ticks: u64) -> bool {
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

    let deadline = if timeout_ticks == 0 {
        0
    } else {
        crate::kernel::timer::ticks() + timeout_ticks
    };
    {
        let task = current();
        task.futex_key = key;
        task.wake_tick = deadline;
        task.state = TaskState::Blocked;
    }

    loop {
        if !schedule() {
            cpu::wait_for_interrupt();
            wake_sleepers();
        }
        // L'ordre des deux tests compte. `wake_sleepers` remet la tache en
        // `Ready` des que son echeance est atteinte, exactement comme le ferait
        // un `FUTEX_WAKE` : tester l'etat en premier ferait passer tout delai
        // expire pour un reveil. La libc croirait alors avoir ete signalee, se
        // rendormirait pour la meme duree, et `pthread_cond_timedwait`
        // attendrait un multiple de ce qu'on lui a demande.
        let expired = deadline != 0 && crate::kernel::timer::ticks() >= deadline;
        let task = current();
        if expired {
            task.futex_key = 0;
            task.wake_tick = 0;
            task.state = TaskState::Ready;
            return false;
        }
        if task.state == TaskState::Ready {
            task.futex_key = 0;
            task.wake_tick = 0;
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
            task.wake_tick = 0;
            task.state = TaskState::Ready;
            woken += 1;
        }
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
