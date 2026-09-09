//! Preuve hote du BACKING du tas noyau.
//!
//! Les modules de production `heap.rs`, `pages_tas.rs`, `magasin.rs` et
//! `compagnon.rs` sont inclus TELS QUELS : ce qui est mis a l'epreuve ici est
//! le code qui tourne dans le noyau, pas une copie qui lui ressemble.
//!
//! # La question a laquelle ce fichier repond
//!
//! Le tas avait trois etages, et le troisieme -- `LockedHeap`, une liste
//! chainee sous un verrou global -- etait le backing NORMAL : chaque premiere
//! chauffe d'une classe et CHAQUE allocation de plus de 1024 octets y
//! descendaient. Le chantier remplace ce backing par les pages du compagnon.
//!
//! Une affirmation pareille ne se lit pas dans un diff : elle se MESURE. Les
//! compteurs `backing_allocs` / `backing_frees` comptent exactement les
//! descentes dans le `LockedHeap`, et la preuve centrale est qu'apres
//! `switch_arena`, une charge de plusieurs milliers d'allocations ne les fait
//! plus bouger DU TOUT.
//!
//! # Ce que les cas negatifs protegent
//!
//! `compagnon::ordre_pour` SATURE au dernier ordre : il rend dix pour
//! n'importe quelle demande superieure a quatre mebioctets, sans le dire. Un
//! garde ecrit `ordre >= ORDRES` -- la forme naturelle, et la mauvaise -- ne se
//! declenche donc jamais, et un tampon de trame 1920x1080x32, qui fait huit
//! mebioctets, recevrait un bloc de quatre. `le_compagnon_refuse_ce_qu_il_ne_
//! peut_pas_couvrir` echoue si ce garde est reecrit sous cette forme.
//!
//! # Ce que ce fichier ne prouve pas
//!
//! Le comportement SMP reel : l'hote a un seul index de CPU. Les listes
//! per-CPU sont donc exercees sur une seule, ce qui met a l'epreuve leur
//! logique et pas leur concurrence. La contention est le domaine de
//! `test_magasin_depot.rs`.

// ---------------------------------------------------------------------------
// Le decor : ce que le noyau fournit et que l'hote n'a pas.
// ---------------------------------------------------------------------------

// `use x86_64::instructions::interrupts;` et `use linked_list_allocator::...`
// sont des chemins de CRATE : depuis Rust 2018, un `use` qui commence par un
// identifiant designe une crate externe, jamais un module local. Aliaser la
// crate de test sous ces deux noms est ce qui permet d'inclure `heap.rs` TEL
// QUEL au lieu d'en modifier les imports pour la commodite d'un test.
extern crate self as x86_64;
extern crate self as linked_list_allocator;

// Le tas publie sur COM1. L'hote n'a pas de port serie ; la macro est rendue
// muette pour que les modules de PRODUCTION soient inclus tels quels.
macro_rules! serial_println {
    ($($arg:tt)*) => {{}};
}
pub(crate) use serial_println;

/// Le repli. Le vrai est `linked_list_allocator::LockedHeap` ; celui-ci a la
/// meme surface et la meme semantique observable -- il sert, il rend, il dit ce
/// qu'il utilise et ce qu'il lui reste.
///
/// Il n'a pas a etre le meme allocateur : ce qui est mesure ici est le NOMBRE
/// de fois qu'on descend dedans, pas ce qu'il fait une fois qu'on y est.
mod repli {
    use core::alloc::{GlobalAlloc, Layout};
    use std::sync::{Mutex, MutexGuard};

    /// Un premier-ajustement avec FUSION, sur une liste d'intervalles libres.
    ///
    /// Ce n'est pas `linked_list_allocator`, et ce n'est pas ce qui est mis a
    /// l'epreuve : ce qui est mesure est le NOMBRE de descentes dans le repli.
    /// La fusion est neanmoins indispensable -- sans elle, une suite de
    /// grandes allocations et liberations epuiserait le repli et ferait echouer
    /// des cas qui parlent d'autre chose.
    pub struct Interne {
        debut: usize,
        fin: usize,
        utilise: usize,
        libres: Vec<(usize, usize)>,
    }

    impl Interne {
        const fn neuf() -> Self {
            Self { debut: 0, fin: 0, utilise: 0, libres: Vec::new() }
        }

        pub fn init(&mut self, debut: *mut u8, taille: usize) {
            self.debut = debut as usize;
            self.fin = debut as usize + taille;
            self.utilise = 0;
            self.libres.clear();
            self.libres.push((self.debut, self.fin));
        }

        pub fn used(&self) -> usize {
            self.utilise
        }

        pub fn free(&self) -> usize {
            self.libres.iter().map(|(d, f)| f - d).sum()
        }

        /// Rend l'intervalle et le recolle a ses voisins.
        fn rend(&mut self, debut: usize, fin: usize) {
            let place = self.libres.partition_point(|(d, _)| *d < debut);
            self.libres.insert(place, (debut, fin));
            let mut i = 0;
            while i + 1 < self.libres.len() {
                if self.libres[i].1 >= self.libres[i + 1].0 {
                    let suivant = self.libres.remove(i + 1);
                    self.libres[i].1 = self.libres[i].1.max(suivant.1);
                } else {
                    i += 1;
                }
            }
        }
    }

    pub struct LockedHeap(Mutex<Interne>);

    impl LockedHeap {
        pub const fn empty() -> Self {
            Self(Mutex::new(Interne::neuf()))
        }
        pub fn lock(&self) -> MutexGuard<'_, Interne> {
            self.0.lock().expect("repli empoisonne")
        }
    }

    unsafe impl GlobalAlloc for LockedHeap {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let mut etat = self.lock();
            for indice in 0..etat.libres.len() {
                let (debut, fin) = etat.libres[indice];
                let aligne = (debut + layout.align() - 1) & !(layout.align() - 1);
                let bout = match aligne.checked_add(layout.size()) {
                    Some(bout) => bout,
                    None => continue,
                };
                if bout > fin {
                    continue;
                }
                etat.libres.remove(indice);
                // Ce qui precede et ce qui suit le bloc servi redeviennent
                // libres : ne pas les rendre transformerait chaque alignement
                // en perte definitive.
                if aligne > debut {
                    etat.rend(debut, aligne);
                }
                if bout < fin {
                    etat.rend(bout, fin);
                }
                etat.utilise += layout.size();
                return aligne as *mut u8;
            }
            core::ptr::null_mut()
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            let mut etat = self.lock();
            assert!(
                ptr as usize >= etat.debut && (ptr as usize) < etat.fin,
                "le repli a recu un bloc qui ne vient pas de lui : {:#x} hors de {:#x}..{:#x}",
                ptr as usize,
                etat.debut,
                etat.fin
            );
            etat.utilise = etat.utilise.saturating_sub(layout.size());
            etat.rend(ptr as usize, ptr as usize + layout.size());
        }
    }
}

pub use repli::LockedHeap;

/// Le masquage d'interruptions. L'hote n'en a pas ; la section critique est
/// exercee telle quelle, sur un seul fil.
pub mod instructions {
    pub mod interrupts {
        pub fn without_interrupts<F: FnOnce() -> R, R>(f: F) -> R {
            f()
        }
    }
}

pub mod arch {
    pub mod x86_64 {
        pub mod smp {
            pub const MAX_CPUS: usize = 16;
            pub fn cpu_index() -> usize {
                0
            }
        }
        pub mod cpu_local {
            pub struct CpuId(usize);
            impl CpuId {
                pub fn from_index(index: usize) -> Option<Self> {
                    Some(CpuId(index))
                }
            }
            pub struct Local;
            impl Local {
                pub fn irq_depth(&self) -> usize {
                    0
                }
            }
            pub fn local(_: CpuId) -> Local {
                Local
            }
        }
    }
}

pub mod kernel {
    pub mod dmesg {
        pub fn log(_: &str) {}
    }

    /// Le verrou du compagnon. Le vrai masque les interruptions ; l'hote n'en a
    /// pas, et la propriete mise a l'epreuve -- l'exclusion mutuelle -- est la
    /// meme.
    pub mod sync {
        use std::sync::{Mutex, MutexGuard};
        pub struct SpinLockIrq<T>(Mutex<T>);
        impl<T> SpinLockIrq<T> {
            pub const fn new(valeur: T) -> Self {
                Self(Mutex::new(valeur))
            }
            pub fn lock(&self) -> MutexGuard<'_, T> {
                self.0.lock().expect("verrou du compagnon empoisonne")
            }
        }
    }

    // Les modules de production sont declares a la RACINE du fichier -- voir
    // plus bas -- puis re-exportes ici. Un `#[path]` depuis un module en ligne
    // se resout a travers un repertoire `tools/smp/kernel/` qui n'existe pas,
    // et le systeme de fichiers refuse de traverser ce qui n'existe pas.
    pub use crate::{
        compagnon_prod as compagnon, dalles_tas_prod as dalles_tas,
        heap_prod as heap, magasin_prod as magasin, pages_tas_prod as pages_tas,
    };
}

#[path = "../../src/kernel/memory/compagnon.rs"]
pub mod compagnon_prod;
#[path = "../../src/kernel/memory/magasin.rs"]
pub mod magasin_prod;
#[path = "../../src/kernel/memory/pages_tas.rs"]
pub mod pages_tas_prod;
#[path = "../../src/kernel/memory/dalles_tas.rs"]
pub mod dalles_tas_prod;
#[path = "../../src/kernel/memory/heap.rs"]
pub mod heap_prod;

// ---------------------------------------------------------------------------
// La preuve
// ---------------------------------------------------------------------------

use core::alloc::{GlobalAlloc, Layout};
use kernel::{heap, pages_tas};
use std::collections::HashSet;
use std::sync::Once;

/// L'arene donnee au tas. Assez grande pour que le compagnon la porte et que
/// le repli en garde une part utile : `switch_arena` reserve `size / 8`, borne
/// a huit mebioctets au minimum.
const ARENE: usize = 64 * 1024 * 1024;

/// Le plus grand bloc contigu du compagnon : 2^10 pages.
const PLUS_GRAND_BLOC: usize = 1024 * pages_tas::PAGE;

static PRET: Once = Once::new();
/// Deux blocs alloues AVANT le basculement, donc dans le tableau d'amorcage.
static mut BLOC_AMORCAGE_PETIT: (usize, usize) = (0, 0);
static mut BLOC_AMORCAGE_GRAND: (usize, usize) = (0, 0);

/// Amorce le tas puis le bascule, une seule fois pour toute la suite.
///
/// Le basculement n'est pas repetable -- `pages_tas::configure` refuse une
/// seconde configuration --, et chaque cas doit donc rendre ce qu'il prend :
/// les assertions portent sur des DELTAS, jamais sur des compteurs absolus.
fn tas_pret() {
    PRET.call_once(|| unsafe {
        heap::init();

        // Deux allocations d'amorcage, gardees pour prouver plus loin que leur
        // liberation ne contamine pas le repli.
        let petit = Layout::from_size_align(48, 8).unwrap();
        BLOC_AMORCAGE_PETIT =
            (GlobalAlloc::alloc(&heap::ALLOCATOR, petit) as usize, 48);
        let grand = Layout::from_size_align(8192, 8).unwrap();
        BLOC_AMORCAGE_GRAND =
            (GlobalAlloc::alloc(&heap::ALLOCATOR, grand) as usize, 8192);
        assert_ne!(BLOC_AMORCAGE_PETIT.0, 0);
        assert_ne!(BLOC_AMORCAGE_GRAND.0, 0);

        // L'arene est une reservation de l'hote, alignee sur la page comme
        // celle que `physical.rs` decoupe dans la plus grande region RAM.
        let brut: Vec<u8> = Vec::with_capacity(ARENE + 2 * pages_tas::PAGE);
        let base = brut.as_ptr() as usize;
        std::mem::forget(brut);
        let aligne = (base + pages_tas::PAGE - 1) & !(pages_tas::PAGE - 1);
        heap::switch_arena(aligne as *mut u8, ARENE);
    });
}

fn alloue(taille: usize, align: usize) -> (*mut u8, Layout) {
    let layout = Layout::from_size_align(taille, align).unwrap();
    let bloc = unsafe { GlobalAlloc::alloc(&heap::ALLOCATOR, layout) };
    assert!(!bloc.is_null(), "allocation refusee : {taille} octets, align {align}");
    (bloc, layout)
}

fn libere(bloc: *mut u8, layout: Layout) {
    unsafe { GlobalAlloc::dealloc(&heap::ALLOCATOR, bloc, layout) };
}

// ---------------------------------------------------------------------------
// La preuve centrale
// ---------------------------------------------------------------------------

/// Apres le basculement, une charge soutenue ne descend PLUS DU TOUT dans le
/// `LockedHeap` -- ni pour les classes de taille, ni pour les grandes demandes.
///
/// C'est l'affirmation entiere du chantier, et elle se mesure a un compteur
/// qui ne bouge pas. Le meme test avant le chantier aurait vu
/// `backing_allocs` croitre d'au moins une unite par premiere chauffe et
/// d'UNE PAR grande allocation, soit plus de deux cents.
#[test]
fn apres_le_basculement_le_chemin_normal_ne_descend_plus_dans_le_repli() {
    tas_pret();
    // Une premiere passe absorbe les premieres chauffes de chaque classe : ce
    // qui est mesure est le REGIME ETABLI, pas le demarrage.
    let mut chauffe: Vec<(*mut u8, Layout)> = Vec::new();
    for tour in 0..1024 {
        chauffe.push(alloue([24, 40, 96, 200, 500, 1000][tour % 6], 8));
    }
    for (bloc, layout) in chauffe.drain(..) {
        libere(bloc, layout);
    }

    let avant = heap::ng_stats();
    let mut vivants: Vec<(*mut u8, Layout)> = Vec::new();
    for tour in 0..8192 {
        let taille = [24, 40, 96, 200, 500, 1000][tour % 6];
        let (bloc, layout) = alloue(taille, 8);
        unsafe { core::ptr::write_bytes(bloc, 0xA5, taille) };
        vivants.push((bloc, layout));
    }
    // Les grandes demandes : celles qui descendaient TOUJOURS dans le repli.
    for _ in 0..128 {
        let (bloc, layout) = alloue(64 * 1024, 8);
        unsafe { core::ptr::write_bytes(bloc, 0x5A, 64 * 1024) };
        vivants.push((bloc, layout));
    }
    for (bloc, layout) in vivants.drain(..) {
        libere(bloc, layout);
    }
    let apres = heap::ng_stats();

    assert_eq!(
        apres.backing_allocs, avant.backing_allocs,
        "le chemin normal descend encore dans le LockedHeap : {} allocations backing",
        apres.backing_allocs - avant.backing_allocs
    );
    assert_eq!(
        apres.backing_frees, avant.backing_frees,
        "le chemin normal rend encore au LockedHeap : {} liberations backing",
        apres.backing_frees - avant.backing_frees
    );
    assert!(
        apres.grandes_par_pages >= avant.grandes_par_pages + 128,
        "les grandes demandes ne sont pas servies par des pages"
    );
    assert_eq!(
        apres.grandes_par_repli, avant.grandes_par_repli,
        "une grande demande est partie au repli alors que le compagnon pouvait la servir"
    );
}

/// Les blocs des classes de taille viennent des PAGES du compagnon.
///
/// Le compteur `backing_allocs` dit qu'on ne descend plus ; ceci dit d'ou l'on
/// remonte. Les deux sont necessaires : un allocateur qui ne servirait plus
/// rien du tout satisferait le premier.
#[test]
fn les_blocs_de_classe_sortent_des_pages_du_compagnon() {
    tas_pret();
    let avant = heap::ng_stats();
    let mut vivants = Vec::new();
    for tour in 0..512 {
        let (bloc, layout) = alloue([32, 64, 128, 256, 512, 1024][tour % 6], 8);
        assert!(
            pages_tas::nous_appartient(bloc as usize),
            "un bloc de classe ne vient pas du compagnon : {:#x}",
            bloc as usize
        );
        vivants.push((bloc, layout));
    }
    for (bloc, layout) in vivants.drain(..) {
        libere(bloc, layout);
    }
    let apres = heap::ng_stats();
    assert!(
        apres.dalles > avant.dalles || apres.cache_hits > avant.cache_hits,
        "aucune dalle prise et aucun cache servi : d'ou viendraient ces blocs ?"
    );
}

// ---------------------------------------------------------------------------
// Ce qui reste au repli, et qui doit y rester
// ---------------------------------------------------------------------------

/// Le compagnon REFUSE ce qu'il ne peut pas couvrir.
///
/// `ordre_pour` sature au dernier ordre sans le dire : pour deux mille pages il
/// rend dix, qui n'en couvre que mille vingt-quatre. Un garde ecrit
/// `ordre >= ORDRES` -- la forme naturelle -- ne se declencherait jamais, et la
/// demande recevrait un bloc DEUX FOIS TROP PETIT dont l'appelant deborderait.
///
/// Ce cas echoue si le garde est reecrit sous cette forme.
#[test]
fn le_compagnon_refuse_ce_qu_il_ne_peut_pas_couvrir() {
    tas_pret();
    assert!(
        pages_tas::alloue(PLUS_GRAND_BLOC + 1).is_none(),
        "le compagnon a servi une demande d'une page de plus que son plus grand bloc"
    );
    assert!(
        pages_tas::alloue(8 * 1024 * 1024).is_none(),
        "le compagnon a servi huit mebioctets -- la taille d'un tampon 1920x1080x32"
    );
    // Ce qu'il peut couvrir, il le sert et le reprend.
    let bloc = pages_tas::alloue(PLUS_GRAND_BLOC)
        .expect("le compagnon doit servir son plus grand bloc");
    unsafe { core::ptr::write_bytes(bloc as *mut u8, 0x11, PLUS_GRAND_BLOC) };
    pages_tas::libere(bloc, PLUS_GRAND_BLOC);
}

/// Une demande plus grande que le plus grand bloc part au repli, et le repli
/// la sert entierement.
///
/// C'est le role qui reste au `LockedHeap` : la RECUPERATION. Le supprimer
/// rendrait ces demandes impossibles.
#[test]
fn une_demande_trop_grande_pour_le_compagnon_est_servie_par_le_repli() {
    tas_pret();
    let avant = heap::ng_stats();
    let taille = PLUS_GRAND_BLOC + 64 * 1024;
    let (bloc, layout) = alloue(taille, 8);
    assert!(
        !pages_tas::nous_appartient(bloc as usize),
        "le compagnon a servi une demande qu'il ne peut pas couvrir"
    );
    // Ecrire TOUTE la demande : si un bloc trop petit avait ete rendu, cette
    // ecriture depasserait, et le compagnon rendrait ensuite des pages dont le
    // chainage a ete ecrase.
    unsafe { core::ptr::write_bytes(bloc, 0x77, taille) };
    let apres = heap::ng_stats();
    assert_eq!(apres.grandes_par_repli, avant.grandes_par_repli + 1);
    assert_eq!(apres.backing_allocs, avant.backing_allocs + 1);
    libere(bloc, layout);
    assert_eq!(heap::ng_stats().backing_frees, avant.backing_frees + 1);
}

/// Un alignement superieur a la page part au repli, parce que le compagnon ne
/// peut pas le garantir : sa base n'est alignee que sur la page.
#[test]
fn un_alignement_superieur_a_la_page_part_au_repli() {
    tas_pret();
    let avant = heap::ng_stats();
    let (bloc, layout) = alloue(8192, 16384);
    assert_eq!(bloc as usize % 16384, 0, "l'alignement demande n'est pas tenu");
    assert!(
        !pages_tas::nous_appartient(bloc as usize),
        "le compagnon a pris une demande dont il ne peut pas garantir l'alignement"
    );
    assert_eq!(heap::ng_stats().grandes_par_repli, avant.grandes_par_repli + 1);
    libere(bloc, layout);
}

// ---------------------------------------------------------------------------
// Le routage des liberations
// ---------------------------------------------------------------------------

/// Un bloc d'AMORCAGE rendu apres le basculement est ABANDONNE, pas donne au
/// repli.
///
/// Le repli gere alors une TOUTE AUTRE region. Lui ajouter un bloc du tableau
/// statique d'amorcage mettrait dans sa liste libre de la memoire qui ne lui
/// appartient pas, et il la redistribuerait ensuite -- deux allocations
/// distinctes rendraient la meme adresse.
#[test]
fn un_bloc_d_amorcage_rendu_apres_le_basculement_est_abandonne() {
    tas_pret();
    let avant = heap::ng_stats();
    unsafe {
        let (petit, taille_petit) = BLOC_AMORCAGE_PETIT;
        let (grand, taille_grand) = BLOC_AMORCAGE_GRAND;
        libere(petit as *mut u8, Layout::from_size_align(taille_petit, 8).unwrap());
        libere(grand as *mut u8, Layout::from_size_align(taille_grand, 8).unwrap());
    }
    let apres = heap::ng_stats();
    assert_eq!(
        apres.abandons_amorcage,
        avant.abandons_amorcage + 2,
        "les blocs d'amorcage n'ont pas ete reconnus comme tels"
    );
    assert_eq!(
        apres.backing_frees, avant.backing_frees,
        "un bloc d'amorcage a ete rendu au repli, qui gere une autre region"
    );
}

/// Deux allocations vivantes ne partagent jamais une adresse, et ce qu'on ecrit
/// dans l'une se relit dans l'une.
///
/// C'est la propriete que toute la mecanique -- dalles, magasins, reserves,
/// pages -- doit preserver, et la seule dont la violation soit silencieuse.
#[test]
fn deux_allocations_vivantes_ne_se_recouvrent_jamais() {
    tas_pret();
    let mut vivants: Vec<(*mut u8, Layout, u8)> = Vec::new();
    let mut adresses = HashSet::new();
    for tour in 0..6000usize {
        let taille = [24, 40, 96, 200, 500, 1000, 3000, 9000][tour % 8];
        let (bloc, layout) = alloue(taille, 8);
        assert!(
            adresses.insert(bloc as usize),
            "deux allocations vivantes a la meme adresse : {:#x}",
            bloc as usize
        );
        let marque = (tour % 251) as u8;
        unsafe { core::ptr::write_bytes(bloc, marque, taille) };
        vivants.push((bloc, layout, marque));
    }
    for (bloc, layout, marque) in vivants.iter() {
        let taille = layout.size();
        let vu = unsafe { core::slice::from_raw_parts(*bloc as *const u8, taille) };
        assert!(
            vu.iter().all(|octet| *octet == *marque),
            "le contenu d'une allocation vivante a ete reecrit par une autre"
        );
    }
    for (bloc, layout, _) in vivants.drain(..) {
        libere(bloc, layout);
    }
}

// ---------------------------------------------------------------------------
// Fragmentation, epuisement, retention
// ---------------------------------------------------------------------------

/// Le compagnon epuise REFUSE, dit qu'il a refuse, et retrouve son plus grand
/// bloc quand tout lui est rendu.
///
/// La fusion est la seule raison de preferer un compagnon a une liste par
/// taille : sans elle, une arene entierement rendue resterait decoupee.
#[test]
fn le_compagnon_epuise_refuse_puis_retrouve_son_plus_grand_bloc() {
    tas_pret();
    let contigu_avant = pages_tas::plus_grand_contigu();
    assert!(contigu_avant > 0, "le compagnon n'a plus rien avant meme d'essayer");
    let oom_avant = heap::ng_stats().oom;

    let mut pris = Vec::new();
    for _ in 0..4096 {
        match pages_tas::alloue(PLUS_GRAND_BLOC) {
            Some(bloc) => pris.push(bloc),
            None => break,
        }
    }
    assert!(!pris.is_empty(), "le compagnon n'a servi aucun grand bloc");
    assert!(
        pages_tas::alloue(PLUS_GRAND_BLOC).is_none(),
        "le compagnon sert encore un plus grand bloc apres avoir dit non"
    );

    // Epuise, il refuse ; le tas doit alors le DIRE au lieu de rendre un
    // pointeur nul sans explication.
    let mut trop_grand = Vec::new();
    loop {
        let layout = Layout::from_size_align(PLUS_GRAND_BLOC, 8).unwrap();
        let bloc = unsafe { GlobalAlloc::alloc(&heap::ALLOCATOR, layout) };
        if bloc.is_null() {
            break;
        }
        trop_grand.push((bloc, layout));
        assert!(trop_grand.len() < 64, "le repli ne s'epuise jamais");
    }
    assert!(
        heap::ng_stats().oom > oom_avant,
        "une allocation impossible n'a pas ete comptee comme telle"
    );

    for (bloc, layout) in trop_grand.drain(..) {
        libere(bloc, layout);
    }
    for bloc in pris.drain(..) {
        pages_tas::libere(bloc, PLUS_GRAND_BLOC);
    }
    assert_eq!(
        pages_tas::plus_grand_contigu(),
        contigu_avant,
        "la fusion n'a pas recolle l'arene : le compagnon a fragmente definitivement"
    );
}

/// Ce qu'une classe RETIENT est borne par ce qui a ete vivant.
///
/// Les dalles ne repartent pas au compagnon -- un seul bloc encore vivant
/// suffirait a interdire la page --, donc les blocs qui debordent du depot
/// vont dans une reserve de classe. La borne annoncee est le PIC de demande
/// simultanee : ce cas la mesure au lieu de l'esperer.
#[test]
fn une_reserve_de_classe_ne_retient_pas_plus_que_ce_qui_a_ete_vivant() {
    tas_pret();
    let avant = heap::ng_stats();
    const BLOCS: usize = 4000;
    const TAILLE: usize = 128;

    let mut vivants = Vec::new();
    for _ in 0..BLOCS {
        vivants.push(alloue(TAILLE, 8));
    }
    for (bloc, layout) in vivants.drain(..) {
        libere(bloc, layout);
    }
    let apres = heap::ng_stats();

    let retenu = apres.reserve_octets.saturating_sub(avant.reserve_octets);
    assert!(
        retenu <= BLOCS * TAILLE,
        "la reserve retient {retenu} octets pour un pic de {} : ce n'est plus une borne",
        BLOCS * TAILLE
    );
    // Et ce qui est retenu est REUTILISE : la reserve est un cache, pas un
    // cimetiere. Une seconde passe ne doit prendre aucune dalle de plus.
    let dalles_avant = heap::ng_stats().dalles;
    let mut seconde = Vec::new();
    for _ in 0..BLOCS {
        seconde.push(alloue(TAILLE, 8));
    }
    for (bloc, layout) in seconde.drain(..) {
        libere(bloc, layout);
    }
    assert_eq!(
        heap::ng_stats().dalles,
        dalles_avant,
        "la seconde passe a repris des dalles au compagnon : la reserve ne ressert pas"
    );
}

/// Le tas publie un etat qui a un sens : le total ne bouge pas, et ce qui est
/// utilise plus ce qui est libre ne depasse jamais le total.
#[test]
fn l_etat_publie_reste_coherent() {
    tas_pret();
    let (_, _, total) = heap::stats();
    assert_eq!(total, ARENE, "le total annonce n'est pas l'arene donnee");
    let mut vivants = Vec::new();
    for _ in 0..256 {
        vivants.push(alloue(4096, 8));
    }
    let (utilise, libre, total) = heap::stats();
    // Les deux fournisseurs se partagent l'arene sans se recouvrir : la somme
    // ne peut pas la depasser. `cached` est ajoute a `libre` et retranche a
    // `utilise`, donc il ne compte qu'une fois.
    assert!(
        utilise + libre <= total,
        "utilise ({utilise}) + libre ({libre}) depasse le total ({total})"
    );
    assert!(utilise > 0, "256 pages vivantes et le tas se dit vide");
    for (bloc, layout) in vivants.drain(..) {
        libere(bloc, layout);
    }
}


// ---------------------------------------------------------------------------
// C9.1 : metadonnees des dalles
// ---------------------------------------------------------------------------

#[test]
fn c9_les_dalles_comptent_les_objets_vivants() {
    tas_pret();
    let avant = heap::ng_stats();
    let backing_avant = avant.backing_allocs;

    const N: usize = 2048;
    let mut blocs = Vec::new();
    for _ in 0..N {
        blocs.push(alloue(128, 8));
    }

    let pendant = heap::ng_stats();
    assert!(
        pendant.objets_dalles_vivants >= avant.objets_dalles_vivants + N,
        "C9 ne voit pas les objets vivants"
    );

    for (bloc, layout) in blocs.drain(..) {
        libere(bloc, layout);
    }

    let apres = heap::ng_stats();
    assert_eq!(
        apres.objets_dalles_vivants,
        avant.objets_dalles_vivants,
        "les objets rendus restent comptes vivants"
    );
    assert_eq!(
        apres.backing_allocs,
        backing_avant,
        "le suivi C9 a fait redescendre dans LockedHeap"
    );
    assert_eq!(
        apres.dalles_tracking_sous_flux, avant.dalles_tracking_sous_flux,
        "le comptage C9 est passe sous zero"
    );
    assert_eq!(
        apres.dalles_tracking_surallocations, avant.dalles_tracking_surallocations,
        "le comptage C9 depasse la capacite"
    );
}

#[test]
fn c9_une_dalle_vide_devient_candidate_sans_reclaim_premature() {
    tas_pret();
    let avant = heap::ng_stats();
    let (_, rendues_avant, _, _, _, _) = pages_tas::stats();

    let mut blocs = Vec::new();
    for _ in 0..4096 {
        blocs.push(alloue(256, 8));
    }
    for (bloc, layout) in blocs.drain(..) {
        libere(bloc, layout);
    }

    let apres = heap::ng_stats();
    let (_, rendues_apres, _, _, _, _) = pages_tas::stats();
    assert!(
        apres.candidats_dalles_vides > avant.candidats_dalles_vides,
        "aucun passage de dalle a zero n'a ete detecte"
    );
    assert!(apres.dalles_vides > 0, "aucune dalle vide visible");
    assert_eq!(
        rendues_apres, rendues_avant,
        "C9.1 a reclaim des pages avant le drainage des caches"
    );
}
