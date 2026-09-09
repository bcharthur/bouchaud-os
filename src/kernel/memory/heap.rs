//! Le tas noyau : listes per-CPU, depot de magasins, PAGES DU COMPAGNON.
//!
//! # Les trois etages, et celui qui manquait
//!
//! Le tas avait trois etages -- liste libre par CPU, depot de magasins, puis
//! `LockedHeap`. Les deux premiers suppriment le verrou global AU MILIEU. Le
//! troisieme etait un allocateur a LISTE CHAINEE sous un verrou GLOBAL, et il
//! restait le backing NORMAL :
//!
//!   * chaque premiere chauffe d'une classe y descendait chercher UN objet ;
//!   * chaque grande allocation -- au-dela de 1024 octets -- y descendait
//!     TOUJOURS, sans passer par aucun cache ;
//!   * sa recherche est lineaire dans une liste que la fragmentation allonge,
//!     sous un verrou que les seize coeurs du Reference Device partagent.
//!
//! `memory::pages_tas` -- le compagnon, deja eprouve sur le DMA, instancie une
//! seconde fois sur l'arene du tas -- fournit desormais des PAGES :
//!
//!   * une classe qui chauffe prend une DALLE (au moins `LOT` blocs d'un coup)
//!     et la decoupe entierement, au lieu d'un objet par descente ;
//!   * une grande allocation prend exactement les pages qu'il lui faut, avec
//!     une fusion en OU EXCLUSIF au retour au lieu d'une liste a parcourir.
//!
//! # Ce qui reste au `LockedHeap`
//!
//! L'amorcage et la reprise, rien d'autre. Avant `switch_arena` il est le seul
//! tas qui existe. Apres, il ne garde qu'une RESERVE bornee en queue d'arene,
//! pour les seules demandes que le compagnon refuse : un alignement superieur a
//! la page, une taille superieure a son plus grand bloc, ou l'epuisement.
//!
//! `[MEM-NG-HEAP] backing_allocs=` mesure exactement ce qui lui reste : sur le
//! regime etabli, ce compteur doit cesser de croitre.
//!
//! # Le routage des liberations se fait sur l'ADRESSE
//!
//! Il ne se devine pas. Apres le basculement, des blocs des DEUX origines
//! coexistent -- ceux d'avant viennent du tableau statique d'amorcage, ceux
//! d'apres du compagnon -- et seule l'adresse le dit. C'est la seule regle qui
//! survive au changement d'arene, et c'est pourquoi `dealloc` interroge
//! `pages_tas::nous_appartient` avant tout le reste.
//!
//! # Ce que les dalles ne rendent pas, et pourquoi
//!
//! Une dalle decoupee en blocs de 32 octets ne peut pas retourner au compagnon
//! tant qu'UN seul de ses cent-vingt-huit blocs vit encore, et ces blocs sont
//! disperses dans les listes de seize CPU et dans les magasins du depot :
//! aucune structure ne permet de les y retrouver. Les blocs qui debordent du
//! depot vont donc dans une RESERVE PAR CLASSE au lieu de repartir au
//! compagnon.
//!
//! Ce n'est pas une fuite, et la difference est mesurable : ce qu'une classe
//! retient est borne par son PIC de demande simultanee -- on ne retient que de
//! la memoire qui a reellement ete vivante --, et cette memoire reste servable
//! par cette classe. `[MEM-NG-RESERVE]` publie ce que chaque classe retient,
//! afin que la borne soit une mesure et pas une esperance. Rendre une dalle
//! partielle demanderait une liste libre PAR DALLE, ce qui est un autre lot.

use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use linked_list_allocator::LockedHeap;
use crate::arch::x86_64::smp;
use crate::kernel::magasin::{self, Depot, Magasin, LOT};
use crate::kernel::pages_tas;
use x86_64::instructions::interrupts;

pub const BOOTSTRAP_SIZE: usize = 8 * 1024 * 1024;
const CLASS_SIZES: [usize; 6] = [32, 64, 128, 256, 512, 1024];
const CLASS_COUNT: usize = CLASS_SIZES.len();
const MAX_CACHED_PER_CLASS: usize = 64;

static mut HEAP_SPACE: [u8; BOOTSTRAP_SIZE] = [0; BOOTSTRAP_SIZE];
static HEAP_TOTAL: AtomicUsize = AtomicUsize::new(BOOTSTRAP_SIZE);
static CACHE_READY: AtomicBool = AtomicBool::new(false);

/// Bornes de l'arene du tas. Elles servent a valider les liens de la liste
/// libre AVANT de les dereferencer -- voir `magasin::lien_plausible`.
static ARENE_DEBUT: AtomicUsize = AtomicUsize::new(0);
static ARENE_FIN: AtomicUsize = AtomicUsize::new(0);
/// Liens refuses parce qu'ils ne pouvaient pas designer un bloc.
static LIENS_REFUSES: AtomicU64 = AtomicU64::new(0);
/// La corruption a-t-elle deja ete signalee ? Une fois suffit.
static CORRUPTION_DITE: AtomicBool = AtomicBool::new(false);

/// Les bornes de l'arene du tas, ou `(0, 0)` si elle n'est pas connue.
///
/// Le chemin de faute s'en sert pour dire si `RSP` designe encore le tas :
/// toutes les piles noyau y sont allouees, donc un `RSP` hors de ces bornes
/// n'est pas une pile un peu trop pleine, c'est une pile qui n'existe pas.
pub fn arene_bornes() -> (usize, usize) {
    (ARENE_DEBUT.load(Ordering::Acquire), ARENE_FIN.load(Ordering::Acquire))
}

/// Compteur de liens refuses, pour le diagnostic.
pub fn liens_refuses() -> u64 {
    LIENS_REFUSES.load(Ordering::Relaxed)
}

/// Un lien de liste libre peut-il etre suivi ?
///
/// Le refus est COMPTE et signale une fois. Une liste libre corrompue est un
/// usage-apres-liberation quelque part ; le taire reviendrait a transformer un
/// bug reperable en corruption silencieuse.
#[inline]
fn lien_suivable(bloc: usize, taille: usize) -> bool {
    let (debut, fin) = arene_bornes();
    if magasin::lien_plausible(bloc, taille, debut, fin) {
        return true;
    }
    LIENS_REFUSES.fetch_add(1, Ordering::Relaxed);
    if !CORRUPTION_DITE.swap(true, Ordering::Release) {
        crate::serial_println!(
            "BOUCHAUD_HEAP_LISTE_LIBRE_CORROMPUE bloc={:#x} taille={} arene={:#x}..{:#x}",
            bloc, taille, debut, fin
        );
    }
    false
}

struct CacheClass {
    head: AtomicUsize,
    count: AtomicUsize,
}
impl CacheClass {
    const fn new() -> Self {
        Self { head: AtomicUsize::new(0), count: AtomicUsize::new(0) }
    }
}
struct CpuCache { classes: [CacheClass; CLASS_COUNT] }
impl CpuCache {
    const fn new() -> Self {
        Self { classes: [const { CacheClass::new() }; CLASS_COUNT] }
    }
}
static CACHES: [CpuCache; smp::MAX_CPUS] =
    [const { CpuCache::new() }; smp::MAX_CPUS];

/// Un depot par classe de taille. Partage par tous les CPU, pris pour la duree
/// d'un transfert de magasin -- donc O(1), interruptions masquees.
static DEPOTS: [Depot; CLASS_COUNT] = [const { Depot::neuf() }; CLASS_COUNT];

/// Allocations demandees alors que ce CPU etait dans un gestionnaire
/// d'interruption.
///
/// Ce n'est pas une faute en soi -- le reveil d'une file d'attente depuis
/// l'IRQ 8042 en fait --, mais c'est le chemin ou une descente dans le backing
/// global coute le plus cher, et il n'etait mesure par rien.
static ALLOCS_EN_IRQ: AtomicU64 = AtomicU64::new(0);
static BACKING_FREES: AtomicU64 = AtomicU64::new(0);

static CACHE_BYTES: AtomicUsize = AtomicUsize::new(0);
static DEPOT_HITS: AtomicU64 = AtomicU64::new(0);
static DEPOT_SPILLS: AtomicU64 = AtomicU64::new(0);
static CACHE_HITS: AtomicU64 = AtomicU64::new(0);
static CACHE_MISSES: AtomicU64 = AtomicU64::new(0);
static CACHE_RETURNS: AtomicU64 = AtomicU64::new(0);
static BACKING_ALLOCS: AtomicU64 = AtomicU64::new(0);

/// Bornes du tableau statique d'amorcage.
///
/// Elles ne servent pas a allouer : elles servent a RECONNAITRE, apres le
/// basculement, un bloc qui vient de l'amorcage. Le rendre au `LockedHeap`
/// -- qui gere alors une TOUTE AUTRE region -- ajouterait a sa liste libre de
/// la memoire qui ne lui appartient pas, et il la redistribuerait ensuite.
static BOOTSTRAP_DEBUT: AtomicUsize = AtomicUsize::new(0);
static BOOTSTRAP_FIN: AtomicUsize = AtomicUsize::new(0);
/// L'arene a-t-elle bascule ? Avant, l'amorcage EST le tas.
static ARENE_BASCULEE: AtomicBool = AtomicBool::new(false);
/// Liberations de blocs d'amorcage arrivees apres le basculement.
///
/// L'invariant de `switch_arena` dit qu'il ne doit pas y en avoir. Un compteur
/// non nul dit que l'invariant est faux, ce qu'aucune assertion ne disait.
static ABANDONS_AMORCAGE: AtomicU64 = AtomicU64::new(0);

/// Dalles decoupees, et octets qu'elles representent.
static DALLES: AtomicU64 = AtomicU64::new(0);
static DALLES_OCTETS: AtomicUsize = AtomicUsize::new(0);
/// Grandes allocations servies par les pages du compagnon, et par le repli.
static GRANDES_PAR_PAGES: AtomicU64 = AtomicU64::new(0);
static GRANDES_PAR_REPLI: AtomicU64 = AtomicU64::new(0);
/// Demandes auxquelles plus rien n'a pu repondre.
static OOM: AtomicU64 = AtomicU64::new(0);
static OOM_DIT: AtomicBool = AtomicBool::new(false);

/// Une adresse tombe-t-elle dans le tableau statique d'amorcage ?
#[inline]
fn dans_amorcage(adresse: usize) -> bool {
    let debut = BOOTSTRAP_DEBUT.load(Ordering::Acquire);
    debut != 0 && adresse >= debut && adresse < BOOTSTRAP_FIN.load(Ordering::Acquire)
}

/// La reserve d'une classe : les blocs du compagnon que le depot ne garde plus.
///
/// # Pourquoi ces blocs ne repartent pas au compagnon
///
/// Ils sont des morceaux de dalle. Rendre une page au compagnon alors qu'un
/// seul de ses blocs vit encore la ferait redistribuer sous les pieds de son
/// occupant. Savoir qu'aucun ne vit plus demanderait de retrouver TOUS les
/// blocs d'une dalle, et ils sont disperses dans les listes de seize CPU et
/// dans les magasins du depot.
///
/// La reserve est donc sans plafond -- mais pas sans borne : elle ne peut
/// contenir que des blocs qui ont ete alloues, donc au plus le pic de demande
/// simultanee de sa classe. `blocs` et `pic` publient cette borne.
struct Reserve {
    verrou: AtomicBool,
    tete: AtomicUsize,
    blocs: AtomicUsize,
    pic: AtomicUsize,
    servis: AtomicU64,
    recus: AtomicU64,
}

impl Reserve {
    const fn neuve() -> Self {
        Self {
            verrou: AtomicBool::new(false),
            tete: AtomicUsize::new(0),
            blocs: AtomicUsize::new(0),
            pic: AtomicUsize::new(0),
            servis: AtomicU64::new(0),
            recus: AtomicU64::new(0),
        }
    }

    #[inline]
    fn prend(&self) {
        while self
            .verrou
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
    }

    #[inline]
    fn rend(&self) {
        self.verrou.store(false, Ordering::Release);
    }

    /// Accepte un bloc. O(1) : la reserve est une pile, pas une file.
    ///
    /// # Interruptions
    /// L'appelant doit les avoir masquees : le tas est atteignable depuis un
    /// gestionnaire d'interruption, et une reprise sur ce CPU serait un
    /// interblocage sur `verrou`.
    unsafe fn depose(&self, bloc: usize) {
        self.prend();
        let ancienne = self.tete.load(Ordering::Relaxed);
        magasin::lien_ecrit(bloc, ancienne);
        self.tete.store(bloc, Ordering::Relaxed);
        let n = self.blocs.load(Ordering::Relaxed) + 1;
        self.blocs.store(n, Ordering::Relaxed);
        self.rend();
        self.recus.fetch_add(1, Ordering::Relaxed);
        self.pic.fetch_max(n, Ordering::Relaxed);
    }

    /// Detache au plus `LOT` blocs. Rend `None` si la reserve est vide.
    ///
    /// La marche est bornee par `LOT` et chaque lien est valide avant d'etre
    /// suivi : la reserve est de la memoire LIBEREE, donc de la memoire qu'un
    /// usage-apres-liberation a pu reecrire.
    ///
    /// # Interruptions
    /// Voir [`Reserve::depose`].
    unsafe fn retire_lot(&self, taille: usize) -> Option<Magasin> {
        self.prend();
        let tete = self.tete.load(Ordering::Relaxed);
        if tete == 0 || !lien_suivable(tete, taille) {
            // Une tete invraisemblable veut dire que la chaine ne decrit plus
            // rien. L'abandonner coute la reserve d'une classe ; la suivre
            // ecrirait a une adresse arbitraire.
            if tete != 0 {
                self.tete.store(0, Ordering::Relaxed);
                self.blocs.store(0, Ordering::Relaxed);
            }
            self.rend();
            return None;
        }
        let mut dernier = tete;
        let mut compte = 1usize;
        while compte < LOT {
            let suivant = magasin::lien_lit(dernier);
            if suivant == 0 || !lien_suivable(suivant, taille) {
                break;
            }
            dernier = suivant;
            compte += 1;
        }
        let reste = magasin::lien_lit(dernier);
        magasin::lien_ecrit(dernier, 0);
        let reste = if reste != 0 && !lien_suivable(reste, taille) { 0 } else { reste };
        self.tete.store(reste, Ordering::Relaxed);
        self.blocs
            .store(self.blocs.load(Ordering::Relaxed).saturating_sub(compte), Ordering::Relaxed);
        self.rend();
        self.servis.fetch_add(1, Ordering::Relaxed);
        Some(Magasin { tete, compte })
    }

    fn vide(&self) {
        self.prend();
        self.tete.store(0, Ordering::Relaxed);
        self.blocs.store(0, Ordering::Relaxed);
        self.rend();
    }
}

static RESERVES: [Reserve; CLASS_COUNT] = [const { Reserve::neuve() }; CLASS_COUNT];

pub(crate) struct NgHeap { inner: LockedHeap }
impl NgHeap { const fn empty() -> Self { Self { inner: LockedHeap::empty() } } }

/// L'allocateur du noyau.
///
/// `pub(crate)` et non prive : `tools/smp/test_tas_pages.rs` inclut CE module
/// et l'exerce par `GlobalAlloc::alloc` directement. Mesurer ce que le tas fait
/// reellement demande de tenir le tas reel, pas une copie qui lui ressemble.
///
/// L'attribut `#[global_allocator]` ne vaut que pour le noyau. Une suite hote
/// est compilee par `rustc --test`, ou `cfg(test)` est vrai pour tout le
/// module : y installer cet allocateur remplacerait celui de `std` par un tas
/// qui n'est pas encore initialise, et le binaire de test ne demarrerait pas.
/// Le noyau, lui, n'est jamais compile avec `--test`.
#[cfg_attr(not(test), global_allocator)]
pub(crate) static ALLOCATOR: NgHeap = NgHeap::empty();

fn class_for(layout: Layout) -> Option<(usize, usize)> {
    let need = layout.size().max(layout.align()).max(core::mem::size_of::<usize>());
    CLASS_SIZES.iter().copied().enumerate().find(|(_, size)| *size >= need)
}

fn cpu_index() -> usize { smp::cpu_index().min(smp::MAX_CPUS - 1) }

/// Compte les allocations demandees depuis un gestionnaire d'interruption.
///
/// Le noyau en fait -- le reveil d'une file d'attente depuis l'IRQ 8042 en
/// est une --, et les interdire serait une regle qu'on ne pourrait pas tenir.
/// Les MESURER est ce qui manquait : c'est le chemin ou une descente dans le
/// backing global coute le plus cher, et un compteur dit s'il est rare ou s'il
/// est le regime normal.
#[inline]
fn note_contexte_alloc() {
    let index = smp::cpu_index();
    if let Some(id) = crate::arch::x86_64::cpu_local::CpuId::from_index(index) {
        if crate::arch::x86_64::cpu_local::local(id).irq_depth() != 0 {
            ALLOCS_EN_IRQ.fetch_add(1, Ordering::Relaxed);
        }
    }
}

unsafe fn cache_pop(index: usize, size: usize) -> *mut u8 {
    interrupts::without_interrupts(|| {
        let cache = &CACHES[cpu_index()].classes[index];
        let head = cache.head.load(Ordering::Acquire);
        if head == 0 { return core::ptr::null_mut(); }
        if !lien_suivable(head, size) {
            // La liste de ce CPU ne veut plus rien dire. L'abandonner coute au
            // pire quelques blocs ; la suivre ecrirait n'importe ou.
            cache.head.store(0, Ordering::Release);
            cache.count.store(0, Ordering::Relaxed);
            return core::ptr::null_mut();
        }
        let next = *(head as *const usize);
        cache.head.store(next, Ordering::Release);
        cache.count.fetch_sub(1, Ordering::Relaxed);
        CACHE_BYTES.fetch_sub(size, Ordering::Relaxed);
        CACHE_HITS.fetch_add(1, Ordering::Relaxed);
        head as *mut u8
    })
}

unsafe fn cache_push(index: usize, size: usize, ptr: *mut u8) -> bool {
    interrupts::without_interrupts(|| {
        let cache = &CACHES[cpu_index()].classes[index];
        if cache.count.load(Ordering::Relaxed) >= MAX_CACHED_PER_CLASS {
            return false;
        }
        let head = cache.head.load(Ordering::Acquire);
        *(ptr as *mut usize) = head;
        cache.head.store(ptr as usize, Ordering::Release);
        cache.count.fetch_add(1, Ordering::Relaxed);
        CACHE_BYTES.fetch_add(size, Ordering::Relaxed);
        CACHE_RETURNS.fetch_add(1, Ordering::Relaxed);
        true
    })
}

/// Remplit la liste per-CPU vide avec un magasin entier du depot.
///
/// Rend un bloc pret a servir, ou nul si le depot n'avait rien : c'est
/// seulement alors qu'on descend dans le backing global.
unsafe fn recharge_depuis_depot(index: usize, size: usize) -> *mut u8 {
    interrupts::without_interrupts(|| {
        let Some(magasin) = DEPOTS[index].retire() else {
            return core::ptr::null_mut();
        };
        let cache = &CACHES[cpu_index()].classes[index];
        let servi = magasin.tete;
        // LE MAGASIN VIENT DU DEPOT, PAS D'UNE PREUVE.
        //
        // Sa tete est une adresse lue dans une structure partagee, et le mot
        // qu'on s'apprete a lire vit dans de la memoire LIBEREE -- donc dans de
        // la memoire qu'un usage-apres-liberation a pu reecrire. La suivre sans
        // la valider etait le chemin par lequel une corruption d'un octet
        // devenait une ecriture a une adresse arbitraire, plus bas.
        if !lien_suivable(servi, size) {
            return core::ptr::null_mut();
        }
        let reste = magasin::lien_lit(servi);
        // `compte` vaut au moins un -- `Depot::depose` refuse zero -- mais le
        // soustraire sans garde ferait de la seule violation possible un
        // `usize::MAX`, c'est-a-dire une marche de liste sans fin.
        let restants = magasin.compte.saturating_sub(1);

        // La liste etait vide quand `cache_pop` a echoue -- mais elle a pu se
        // remplir depuis. `cache_pop` ne masque les interruptions QUE pour sa
        // propre section critique : entre son echec et ce rechargement, une
        // liberation depuis un gestionnaire d'interruption de ce CPU a pu
        // pousser des blocs ici. Ecraser la tete les perdrait -- une fuite
        // silencieuse, proportionnelle au trafic d'interruption.
        //
        // Le magasin se RACCORDE donc a ce qui est la. La marche jusqu'a sa
        // queue est bornee par `LOT`, sur des blocs qui viennent d'etre lus.
        // Le reste de la chaine doit exister avant qu'on pretende le raccorder.
        // `restants != 0` avec un `reste` nul est une incoherence du depot, et
        // la traiter comme « chaine vide » vaut mieux que d'ecrire a zero.
        let restants = if restants != 0 && !lien_suivable(reste, size) {
            0
        } else {
            restants
        };

        let ancienne = cache.head.load(Ordering::Acquire);
        if ancienne != 0 && restants != 0 {
            let mut queue = reste;
            // La marche est bornee par `LOT` : un magasin n'en contient jamais
            // plus, et un `compte` menteur ne doit pas pouvoir faire tourner
            // cette boucle plus longtemps que la structure ne l'autorise.
            for _ in 1..restants.min(magasin::LOT) {
                let suivant = magasin::lien_lit(queue);
                if suivant == 0 { break; }
                if !lien_suivable(suivant, size) {
                    break;
                }
                queue = suivant;
            }
            magasin::lien_ecrit(queue, ancienne);
            cache.head.store(reste, Ordering::Release);
            cache.count.fetch_add(restants, Ordering::Relaxed);
        } else if restants != 0 {
            cache.head.store(reste, Ordering::Release);
            cache.count.fetch_add(restants, Ordering::Relaxed);
        }
        CACHE_BYTES.fetch_add(size.saturating_mul(restants), Ordering::Relaxed);
        DEPOT_HITS.fetch_add(1, Ordering::Relaxed);
        servi as *mut u8
    })
}

/// Vide `LOT` blocs de la liste per-CPU vers le depot.
///
/// Rend `false` si le depot est plein : l'appelant rend alors les blocs au
/// backing, un par un et avec exactement la disposition qui les a alloues.
unsafe fn deverse_vers_depot(index: usize, size: usize) -> Option<Magasin> {
    interrupts::without_interrupts(|| {
        let cache = &CACHES[cpu_index()].classes[index];
        let tete = cache.head.load(Ordering::Acquire);
        if tete == 0 {
            return None;
        }
        let (magasin, reste) = magasin::detache(tete, LOT);
        if magasin.compte == 0 {
            return None;
        }
        cache.head.store(reste, Ordering::Release);
        cache
            .count
            .store(cache.count.load(Ordering::Relaxed).saturating_sub(magasin.compte), Ordering::Relaxed);
        CACHE_BYTES.fetch_sub(size.saturating_mul(magasin.compte), Ordering::Relaxed);
        if DEPOTS[index].depose(magasin) {
            DEPOT_SPILLS.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        // Depot plein : ces blocs reviennent a l'appelant, qui les rendra au
        // backing. Ne jamais les perdre serait tentant a oublier : un magasin
        // detache et non replace est une fuite silencieuse.
        Some(magasin)
    })
}

/// Taille d'une dalle : au moins `LOT` blocs, arrondie a la page.
///
/// C'est ce qui rend la descente au compagnon amortie. Prendre UNE page pour
/// la classe 1024 ne donnerait que quatre blocs, soit une descente tous les
/// quatre objets ; `LOT` blocs en donnent seize, comme un magasin.
const fn taille_dalle(taille: usize) -> usize {
    let brut = taille * LOT;
    let pages = (brut + pages_tas::PAGE - 1) / pages_tas::PAGE;
    if pages == 0 { pages_tas::PAGE } else { pages * pages_tas::PAGE }
}

/// Prend une dalle au compagnon et la decoupe entierement dans la classe.
///
/// Rend le premier bloc ; tous les autres sont chaines dans la liste de ce CPU.
/// Rend un pointeur nul quand le compagnon n'a plus rien : l'appelant retombe
/// alors sur le `LockedHeap` de reprise.
///
/// # Alignement
/// La base d'une dalle est alignee sur la page et toutes les classes sont des
/// puissances de deux qui divisent la page : chaque bloc `base + k * taille`
/// est donc aligne sur `taille`, ce que `magasin::lien_plausible` exige.
unsafe fn decoupe_une_dalle(index: usize, taille: usize) -> *mut u8 {
    let octets = taille_dalle(taille);
    // Le compagnon est pris AVANT de masquer les interruptions : son verrou les
    // masque deja, et les imbriquer sans raison allongerait la fenetre.
    let Some(base) = pages_tas::alloue(octets) else {
        return core::ptr::null_mut();
    };
    DALLES.fetch_add(1, Ordering::Relaxed);
    DALLES_OCTETS.fetch_add(octets, Ordering::Relaxed);
    let blocs = octets / taille;
    interrupts::without_interrupts(|| {
        let cache = &CACHES[cpu_index()].classes[index];
        // La chaine se construit A REBOURS et se RACCORDE a ce qui est deja la.
        // Ecraser la tete perdrait les blocs qu'un gestionnaire d'interruption
        // de ce CPU a pu y pousser entre l'echec du cache et cette descente.
        let mut precedent = cache.head.load(Ordering::Acquire);
        let mut k = blocs;
        while k > 1 {
            k -= 1;
            let bloc = base + k * taille;
            magasin::lien_ecrit(bloc, precedent);
            precedent = bloc;
        }
        cache.head.store(precedent, Ordering::Release);
        cache.count.fetch_add(blocs - 1, Ordering::Relaxed);
        CACHE_BYTES.fetch_add(taille * (blocs - 1), Ordering::Relaxed);
        base as *mut u8
    })
}

/// Remplit la liste de ce CPU depuis la reserve de la classe.
///
/// Rend un bloc pret a servir, ou nul si la reserve etait vide.
unsafe fn recharge_depuis_reserve(index: usize, taille: usize) -> *mut u8 {
    interrupts::without_interrupts(|| {
        let Some(magasin) = RESERVES[index].retire_lot(taille) else {
            return core::ptr::null_mut();
        };
        let servi = magasin.tete;
        let reste = magasin::lien_lit(servi);
        let restants = magasin.compte.saturating_sub(1);
        let restants = if restants != 0 && !lien_suivable(reste, taille) { 0 } else { restants };
        if restants != 0 {
            let cache = &CACHES[cpu_index()].classes[index];
            let ancienne = cache.head.load(Ordering::Acquire);
            if ancienne != 0 {
                let mut queue = reste;
                for _ in 1..restants.min(LOT) {
                    let suivant = magasin::lien_lit(queue);
                    if suivant == 0 || !lien_suivable(suivant, taille) { break; }
                    queue = suivant;
                }
                magasin::lien_ecrit(queue, ancienne);
            }
            cache.head.store(reste, Ordering::Release);
            cache.count.fetch_add(restants, Ordering::Relaxed);
            CACHE_BYTES.fetch_add(taille.saturating_mul(restants), Ordering::Relaxed);
        }
        servi as *mut u8
    })
}

/// Rend UN bloc de classe a son origine, decidee par son ADRESSE.
///
/// Trois origines possibles, et une seule est deduite du contexte :
///
///   * le compagnon -- le bloc va dans la reserve de sa classe ;
///   * le tableau d'amorcage, apres le basculement -- le bloc est ABANDONNE,
///     parce que le `LockedHeap` gere alors une autre region et l'y ajouter la
///     corromprait ;
///   * le `LockedHeap` -- le bloc lui revient.
unsafe fn rend_un_bloc(index: usize, taille: usize, bloc: usize) {
    if pages_tas::nous_appartient(bloc) {
        interrupts::without_interrupts(|| RESERVES[index].depose(bloc));
        return;
    }
    if dans_amorcage(bloc) && ARENE_BASCULEE.load(Ordering::Acquire) {
        ABANDONS_AMORCAGE.fetch_add(1, Ordering::Relaxed);
        return;
    }
    BACKING_FREES.fetch_add(1, Ordering::Relaxed);
    GlobalAlloc::dealloc(
        &ALLOCATOR.inner,
        bloc as *mut u8,
        Layout::from_size_align_unchecked(taille, taille),
    );
}

/// Les octets qu'une grande demande occupe reellement.
///
/// L'alignement entre dans le calcul parce que le compagnon aligne sur l'ordre
/// qu'il sert : demander `align` octets garantit un bloc aligne sur `align`.
/// La MEME formule doit servir a la liberation, sinon le compagnon fusionnerait
/// sur un jumeau qui n'existe pas.
#[inline]
fn octets_grande(layout: Layout) -> usize {
    layout.size().max(layout.align())
}

/// Signale la premiere impossibilite d'allouer, avec de quoi la comprendre.
///
/// Une seule fois : un tas epuise echoue en rafale, et noyer le port serie
/// ferait perdre la premiere trace, qui est la seule utile.
fn signale_oom(layout: Layout) {
    OOM.fetch_add(1, Ordering::Relaxed);
    if OOM_DIT.swap(true, Ordering::Release) {
        return;
    }
    let (servies, rendues, en_service, pic, refus, libres) = pages_tas::stats();
    crate::serial_println!(
        "BOUCHAUD_TAS_OOM taille={} align={} pages_servies={} pages_rendues={} pages_en_service={} pic={} refus={} pages_libres={} plus_grand_contigu={} dalles={} repli_octets_libres={}",
        layout.size(), layout.align(), servies, rendues, en_service, pic, refus,
        libres, pages_tas::plus_grand_contigu(), DALLES.load(Ordering::Relaxed),
        ALLOCATOR.inner.lock().free(),
    );
}

impl NgHeap {
    /// Le chemin d'une classe de taille : cache, depot, reserve, DALLE.
    ///
    /// Le `LockedHeap` n'est atteint qu'apres l'echec du compagnon. Sur le
    /// regime etabli, `backing_allocs` doit cesser de croitre : c'est la mesure
    /// qui dit que le backing normal a change.
    unsafe fn alloue_classe(&self, index: usize, taille: usize) -> *mut u8 {
        let cache = cache_pop(index, taille);
        if !cache.is_null() { return cache; }
        CACHE_MISSES.fetch_add(1, Ordering::Relaxed);

        let depot = recharge_depuis_depot(index, taille);
        if !depot.is_null() { return depot; }

        let reserve = recharge_depuis_reserve(index, taille);
        if !reserve.is_null() { return reserve; }

        let dalle = decoupe_une_dalle(index, taille);
        if !dalle.is_null() { return dalle; }

        BACKING_ALLOCS.fetch_add(1, Ordering::Relaxed);
        GlobalAlloc::alloc(&self.inner, Layout::from_size_align_unchecked(taille, taille))
    }

    /// Le chemin des grandes demandes : des PAGES, pas une liste chainee.
    ///
    /// Le compagnon ne sait aligner que sur l'ordre qu'il sert, et sa base
    /// n'est alignee que sur la page : une demande d'alignement superieur a la
    /// page part donc au repli, ou elle est correcte, plutot qu'a un compagnon
    /// qui ne peut pas la garantir.
    unsafe fn alloue_grande(&self, layout: Layout) -> *mut u8 {
        if layout.align() <= pages_tas::PAGE {
            if let Some(adresse) = pages_tas::alloue(octets_grande(layout)) {
                GRANDES_PAR_PAGES.fetch_add(1, Ordering::Relaxed);
                return adresse as *mut u8;
            }
        }
        GRANDES_PAR_REPLI.fetch_add(1, Ordering::Relaxed);
        BACKING_ALLOCS.fetch_add(1, Ordering::Relaxed);
        GlobalAlloc::alloc(&self.inner, layout)
    }

    unsafe fn libere_classe(&self, index: usize, taille: usize, ptr: *mut u8) {
        if cache_push(index, taille, ptr) { return; }
        // La liste deborde. On la vide d'un LOT vers le depot ; c'est le seul
        // chemin qui rend au cache la place d'accepter ce bloc sans descendre.
        if let Some(magasin) = deverse_vers_depot(index, taille) {
            // Depot plein : chaque bloc repart a SON origine, decidee par son
            // adresse. Un magasin peut en melanger -- le basculement d'arene ne
            // trie pas ce qui etait deja dans les listes.
            let mut courant = magasin.tete;
            while courant != 0 {
                let suivant = magasin::lien_lit(courant);
                rend_un_bloc(index, taille, courant);
                courant = suivant;
            }
        }
        if cache_push(index, taille, ptr) { return; }
        rend_un_bloc(index, taille, ptr as usize);
    }

    unsafe fn libere_grande(&self, ptr: *mut u8, layout: Layout) {
        let adresse = ptr as usize;
        if pages_tas::nous_appartient(adresse) {
            pages_tas::libere(adresse, octets_grande(layout));
            return;
        }
        if dans_amorcage(adresse) && ARENE_BASCULEE.load(Ordering::Acquire) {
            ABANDONS_AMORCAGE.fetch_add(1, Ordering::Relaxed);
            return;
        }
        BACKING_FREES.fetch_add(1, Ordering::Relaxed);
        GlobalAlloc::dealloc(&self.inner, ptr, layout);
    }
}

unsafe impl GlobalAlloc for NgHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note_contexte_alloc();
        let bloc = match class_for(layout) {
            Some((index, taille)) if CACHE_READY.load(Ordering::Acquire) => {
                self.alloue_classe(index, taille)
            }
            Some(_) => {
                // Caches desarmes : l'amorcage, et la fenetre du basculement.
                BACKING_ALLOCS.fetch_add(1, Ordering::Relaxed);
                GlobalAlloc::alloc(&self.inner, layout)
            }
            None => self.alloue_grande(layout),
        };
        if bloc.is_null() {
            signale_oom(layout);
        }
        bloc
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // UN BLOC D'AMORCAGE RENDU APRES LE BASCULEMENT N'APPARTIENT A PERSONNE.
        //
        // Ni au compagnon, ni au repli -- qui gere alors une toute autre
        // region. Le laisser suivre le chemin ordinaire le ferait entrer dans
        // la liste libre de sa classe, ou il devient la TETE : le prochain
        // `cache_pop` demanderait a `lien_suivable` si cette adresse peut etre
        // un bloc du tas, elle serait refusee -- elle est hors de l'arene --,
        // la liste entiere serait abandonnee et le noyau signalerait une
        // corruption qui n'en est pas une.
        //
        // Il est donc ABANDONNE ici, et compte. Sa memoire vit dans un statique
        // de l'image, qui n'est ni rendu ni reutilise : le cout est nul, et le
        // compteur dit si l'invariant de `switch_arena` tient vraiment.
        if dans_amorcage(ptr as usize) && ARENE_BASCULEE.load(Ordering::Acquire) {
            ABANDONS_AMORCAGE.fetch_add(1, Ordering::Relaxed);
            return;
        }
        match class_for(layout) {
            Some((index, taille)) => {
                if CACHE_READY.load(Ordering::Acquire) {
                    self.libere_classe(index, taille, ptr);
                } else {
                    // Caches desarmes, mais le ROUTAGE reste sur l'adresse :
                    // un bloc du compagnon ne doit jamais partir au repli, meme
                    // pendant la fenetre du basculement.
                    rend_un_bloc(index, taille, ptr as usize);
                }
            }
            None => self.libere_grande(ptr, layout),
        }
    }
}

/// La reserve laissee au `LockedHeap` en queue d'arene, apres le basculement.
///
/// Elle n'est pas le tas : elle est ce qui repond aux demandes que le compagnon
/// REFUSE -- alignement superieur a la page, taille superieure a son plus grand
/// bloc (quatre mebioctets), ou epuisement. La dimensionner a zero rendrait ces
/// demandes impossibles ; la dimensionner en fraction de l'arene rendrait au
/// verrou global une part proportionnelle du tas.
const REPLI_MAX: usize = 64 * 1024 * 1024;
const REPLI_MIN: usize = 8 * 1024 * 1024;

pub fn init() {
    unsafe {
        let debut = core::ptr::addr_of_mut!(HEAP_SPACE) as *mut u8;
        ALLOCATOR.inner.lock().init(debut, BOOTSTRAP_SIZE);
        ARENE_DEBUT.store(debut as usize, Ordering::Release);
        ARENE_FIN.store(debut as usize + BOOTSTRAP_SIZE, Ordering::Release);
        BOOTSTRAP_DEBUT.store(debut as usize, Ordering::Release);
        BOOTSTRAP_FIN.store(debut as usize + BOOTSTRAP_SIZE, Ordering::Release);
    }
    crate::kernel::dmesg::log("heap-ng: bootstrap 8 MiB initialise");
}

/// Bascule le tas sur la grande arene physique.
///
/// L'arene est COUPEE EN DEUX : la plus grande part va au compagnon, qui
/// devient le fournisseur normal de pages ; une reserve bornee reste au
/// `LockedHeap` pour ce que le compagnon refuse. Si le compagnon ne peut pas
/// etre configure -- arene trop petite pour porter son bitmap --, toute l'arene
/// lui revient : mieux vaut l'ancien allocateur que pas d'allocateur.
///
/// # Securite
/// `start` doit designer `size` octets mappes dont personne d'autre ne dispose,
/// et l'invariant historique doit tenir : aucune allocation d'amorcage vivante
/// au moment de l'appel.
pub unsafe fn switch_arena(start: *mut u8, size: usize) {
    CACHE_READY.store(false, Ordering::Release);

    // LES CACHES SONT VIDES AVANT LE CHANGEMENT, ET C'EST OBLIGATOIRE.
    //
    // L'invariant historique -- « aucune allocation bootstrap persistante ne
    // doit exister ici » -- parlait des allocations VIVANTES. Il oubliait les
    // blocs LIBRES : ceux qui dorment dans les listes per-CPU et dans les
    // magasins du depot. Ces blocs-la pointent dans l'arene bootstrap, un
    // tableau statique de l'image noyau, et rien ne les suivait a travers le
    // changement.
    //
    // Le tas etait donc reinitialise sur une region entierement differente,
    // puis les caches reactives -- et le premier `cache_pop` rendait un
    // pointeur de l'ANCIENNE arene comme s'il venait de la nouvelle. Les deux
    // regions se melangeaient ensuite dans les memes listes libres, et toute
    // la comptabilite du tas portait sur un ensemble de blocs qui n'etaient
    // pas ceux qu'elle croyait decrire.
    //
    // Les blocs abandonnes ne fuient pas : ils vivent dans un statique de
    // l'image, qui n'est ni rendu ni reutilise. C'est le prix d'un unique
    // changement d'arene au demarrage, et il est nul.
    vide_les_caches();

    // Les bornes AVANT la configuration du compagnon : il ecrit le chainage de
    // ses blocs libres des `configure`, et `lien_suivable` doit deja pouvoir
    // reconnaitre ces adresses comme etant celles du tas.
    ARENE_DEBUT.store(start as usize, Ordering::Release);
    ARENE_FIN.store(start as usize + size, Ordering::Release);
    HEAP_TOTAL.store(size, Ordering::Release);

    let repli = REPLI_MAX.min(size / 8).max(REPLI_MIN).min(size);
    let pour_pages = size - repli;
    let compagnon_pret = pour_pages >= 64 * pages_tas::PAGE
        && pages_tas::configure(start as usize, pour_pages);

    if compagnon_pret {
        ALLOCATOR.inner.lock().init(start.add(pour_pages), repli);
    } else {
        // Le compagnon n'a pas pu etre configure. Toute l'arene revient au
        // repli : le tas reste correct, seulement plus lent.
        ALLOCATOR.inner.lock().init(start, size);
    }
    ARENE_BASCULEE.store(true, Ordering::Release);
    CACHE_READY.store(true, Ordering::Release);
    crate::serial_println!(
        "BOUCHAUD_TAS_BACKING compagnon={} arene={:#x}..{:#x} pages={} repli={}",
        if compagnon_pret { "pages" } else { "repli" },
        start as usize,
        start as usize + size,
        if compagnon_pret { pour_pages } else { 0 },
        if compagnon_pret { repli } else { size },
    );
    crate::kernel::dmesg::log("heap-ng: arene physique active + backing par pages");
}

/// Abandonne tout ce que les listes per-CPU, le depot et les reserves
/// retiennent.
///
/// A n'appeler que caches DESARMES (`CACHE_READY` faux) : sans cela, un autre
/// coeur pourrait pousser un bloc dans une liste qu'on vient de vider, et ce
/// bloc survivrait au changement d'arene -- exactement ce qu'on cherche a
/// empecher.
unsafe fn vide_les_caches() {
    for cpu in 0..smp::MAX_CPUS {
        for classe in 0..CLASS_COUNT {
            let cache = &CACHES[cpu].classes[classe];
            cache.head.store(0, Ordering::Release);
            cache.count.store(0, Ordering::Relaxed);
        }
    }
    let mut abandonnes = 0u64;
    for depot in DEPOTS.iter() {
        // `retire` depile un magasin a la fois ; la boucle s'arrete quand le
        // depot est vide, et `MAGASINS_MAX` la borne de toute facon.
        while let Some(magasin) = depot.retire() {
            abandonnes = abandonnes.saturating_add(magasin.compte as u64);
        }
    }
    for reserve in RESERVES.iter() {
        abandonnes = abandonnes.saturating_add(reserve.blocs.load(Ordering::Relaxed) as u64);
        reserve.vide();
    }
    CACHE_BYTES.store(0, Ordering::Relaxed);
    crate::serial_println!(
        "BOUCHAUD_HEAP_ARENE_CACHES_VIDES blocs_abandonnes={}",
        abandonnes
    );
}

/// (utilise, libre, total). Les blocs en cache sont comptes libres.
///
/// Les deux fournisseurs sont additionnes : le compagnon porte l'essentiel de
/// l'arene, le repli n'en garde qu'une reserve. Ne rendre que le second
/// ferait croire, apres le basculement, a un tas de soixante-quatre mebioctets.
pub fn stats() -> (usize, usize, usize) {
    // Le verrou du repli est relache AVANT d'interroger le compagnon : les deux
    // n'ont aucun ordre etabli entre eux, et les imbriquer en creerait un que
    // rien ne verifie.
    let (repli_utilise, repli_libre) = {
        let repli = ALLOCATOR.inner.lock();
        (repli.used(), repli.free())
    };
    let (_, _, en_service, _, _, pages_libres) = pages_tas::stats();
    let cached = CACHE_BYTES.load(Ordering::Relaxed);
    let total = HEAP_TOTAL.load(Ordering::Acquire);
    let utilise = repli_utilise.saturating_add(en_service as usize * pages_tas::PAGE);
    let libre = repli_libre.saturating_add(pages_libres * pages_tas::PAGE);
    (utilise.saturating_sub(cached), libre.saturating_add(cached), total)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NgStats {
    pub cached_bytes: usize,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_returns: u64,
    pub backing_allocs: u64,
    pub backing_frees: u64,
    pub depot_hits: u64,
    pub depot_spills: u64,
    pub allocs_en_irq: u64,
    /// Dalles prises au compagnon pour les classes de taille.
    pub dalles: u64,
    pub dalles_octets: usize,
    /// Grandes demandes servies par des pages, et par le repli.
    pub grandes_par_pages: u64,
    pub grandes_par_repli: u64,
    /// Blocs retenus par les reserves de classe.
    pub reserve_blocs: usize,
    pub reserve_octets: usize,
    /// Blocs d'amorcage rendus apres le basculement, donc abandonnes.
    pub abandons_amorcage: u64,
    /// Demandes auxquelles plus rien n'a pu repondre.
    pub oom: u64,
    /// La plus grande allocation contigue encore possible.
    pub plus_grand_contigu: usize,
}

pub fn ng_stats() -> NgStats {
    let mut reserve_blocs = 0usize;
    let mut reserve_octets = 0usize;
    for (index, taille) in CLASS_SIZES.iter().enumerate() {
        let n = RESERVES[index].blocs.load(Ordering::Relaxed);
        reserve_blocs += n;
        reserve_octets += n * taille;
    }
    NgStats {
        cached_bytes: CACHE_BYTES.load(Ordering::Relaxed),
        cache_hits: CACHE_HITS.load(Ordering::Relaxed),
        cache_misses: CACHE_MISSES.load(Ordering::Relaxed),
        cache_returns: CACHE_RETURNS.load(Ordering::Relaxed),
        backing_allocs: BACKING_ALLOCS.load(Ordering::Relaxed),
        backing_frees: BACKING_FREES.load(Ordering::Relaxed),
        depot_hits: DEPOT_HITS.load(Ordering::Relaxed),
        depot_spills: DEPOT_SPILLS.load(Ordering::Relaxed),
        allocs_en_irq: ALLOCS_EN_IRQ.load(Ordering::Relaxed),
        dalles: DALLES.load(Ordering::Relaxed),
        dalles_octets: DALLES_OCTETS.load(Ordering::Relaxed),
        grandes_par_pages: GRANDES_PAR_PAGES.load(Ordering::Relaxed),
        grandes_par_repli: GRANDES_PAR_REPLI.load(Ordering::Relaxed),
        reserve_blocs,
        reserve_octets,
        abandons_amorcage: ABANDONS_AMORCAGE.load(Ordering::Relaxed),
        oom: OOM.load(Ordering::Relaxed),
        plus_grand_contigu: pages_tas::plus_grand_contigu(),
    }
}

pub fn log_ng_stats() {
    let s = ng_stats();
    crate::serial_println!(
        "[MEM-NG-HEAP] cached_bytes={} hits={} misses={} returns={} backing_allocs={} backing_frees={} depot_hits={} depot_spills={} allocs_en_irq={}",
        s.cached_bytes, s.cache_hits, s.cache_misses, s.cache_returns,
        s.backing_allocs, s.backing_frees, s.depot_hits, s.depot_spills,
        s.allocs_en_irq
    );
    crate::serial_println!(
        "[MEM-NG-BACKING] dalles={} dalles_octets={} grandes_pages={} grandes_repli={} reserve_blocs={} reserve_octets={} abandons_amorcage={} oom={} plus_grand_contigu={}",
        s.dalles, s.dalles_octets, s.grandes_par_pages, s.grandes_par_repli,
        s.reserve_blocs, s.reserve_octets, s.abandons_amorcage, s.oom,
        s.plus_grand_contigu
    );
    pages_tas::log_stats();
    for (index, taille) in CLASS_SIZES.iter().enumerate() {
        let d = DEPOTS[index].compteurs();
        let r = &RESERVES[index];
        let blocs = r.blocs.load(Ordering::Relaxed);
        let recus = r.recus.load(Ordering::Relaxed);
        // Une classe jamais sollicitee n'a rien a dire : la taire garde la
        // trace lisible sur un port serie.
        if d.servis == 0 && d.deposes == 0 && d.vides == 0 && recus == 0 {
            continue;
        }
        crate::serial_println!(
            "[MEM-NG-DEPOT] classe={} magasins={} pic={} servis={} deposes={} vides={} pleins={}",
            taille, d.magasins, d.pic, d.servis, d.deposes, d.vides, d.pleins
        );
        if recus != 0 {
            crate::serial_println!(
                "[MEM-NG-RESERVE] classe={} blocs={} octets={} pic={} recus={} servis={}",
                taille, blocs, blocs * taille, r.pic.load(Ordering::Relaxed),
                recus, r.servis.load(Ordering::Relaxed)
            );
        }
    }
}
