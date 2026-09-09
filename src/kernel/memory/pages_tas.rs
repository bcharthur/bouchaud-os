//! Les pages du tas noyau, servies par l'allocateur compagnon.
//!
//! # Ce que le `LockedHeap` etait, et pourquoi il ne pouvait pas rester
//!
//! Le tas noyau avait trois etages -- liste libre par CPU, depot de magasins,
//! puis `LockedHeap` -- et le troisieme etait un allocateur a LISTE CHAINEE
//! sous un verrou GLOBAL. Tant que les deux premiers suffisent, il ne coute
//! rien. Aux bords, il coute tout :
//!
//!   * chaque premiere chauffe d'une classe descendait chercher UN objet ;
//!   * chaque grande allocation -- au-dela de la plus grande classe -- y
//!     descendait TOUJOURS, sans passer par aucun cache ;
//!   * et sa recherche est lineaire dans une liste que la fragmentation
//!     allonge, sous un verrou que tous les coeurs partagent.
//!
//! Ce module donne au tas un fournisseur de PAGES : le compagnon, deja utilise
//! pour le DMA, instancie une seconde fois sur l'arene du tas. Une classe qui
//! chauffe prend une page et la decoupe en un magasin entier ; une grande
//! allocation prend directement le nombre de pages qu'il lui faut.
//!
//! # Ce qui reste au `LockedHeap`
//!
//! L'amorcage, et le repli. Il garde le tableau statique de huit mebioctets sur
//! lequel il a ete initialise au demarrage : avant que l'arene physique
//! n'existe, c'est le seul tas qui existe. Apres, il ne sert plus qu'aux
//! demandes que le compagnon refuse -- alignement superieur a la page, ou
//! compagnon epuise.
//!
//! Le routage de la LIBERATION ne se devine donc pas : il se decide sur
//! l'ADRESSE. Un bloc rendu qui tombe dans la plage du compagnon lui revient ;
//! tout le reste retourne au `LockedHeap`. C'est la seule regle qui survive au
//! basculement d'arene, ou des blocs des deux origines coexistent.
//!
//! # Ou vit le bitmap
//!
//! En tete de l'arene elle-meme, comme pour le DMA : c'est la seule memoire
//! dont on soit sur qu'elle existe au moment de la configuration -- le tas
//! qu'on est en train de construire ne peut pas s'allouer a lui-meme.

use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use crate::kernel::compagnon::{self, Compagnon, Terrain, ORDRES};
use crate::kernel::sync::SpinLockIrq;

/// Taille d'un bloc elementaire.
pub const PAGE: usize = 4096;

/// Le terrain : l'arene du tas est deja VIRTUELLE.
///
/// C'est la seule difference avec le terrain du DMA, qui indexe des adresses
/// physiques et les traduit. Ici l'appelant nous a donne un pointeur utilisable
/// tel quel, et le traduire une seconde fois designerait n'importe quoi.
struct TerrainVirtuel {
    base: usize,
}

impl Terrain for TerrainVirtuel {
    fn lien(&self, bloc: usize) -> usize {
        unsafe { core::ptr::read_volatile((self.base + bloc * PAGE) as *const usize) }
    }
    fn pose_lien(&mut self, bloc: usize, suivant: usize) {
        unsafe { core::ptr::write_volatile((self.base + bloc * PAGE) as *mut usize, suivant) }
    }
}

struct Etat {
    compagnon: Compagnon<'static>,
    terrain: TerrainVirtuel,
    /// Premiere adresse distribuable, apres le bitmap.
    base: usize,
    blocs: usize,
}

// Le bitmap et les pages vivent dans l'arene du tas, atteints par ce seul
// verrou.
unsafe impl Send for Etat {}

static ETAT: SpinLockIrq<Option<Etat>> = SpinLockIrq::new(None);
static CONFIGURE: AtomicBool = AtomicBool::new(false);
static BASE: AtomicUsize = AtomicUsize::new(0);
static FIN: AtomicUsize = AtomicUsize::new(0);

static PAGES_SERVIES: AtomicU64 = AtomicU64::new(0);
static PAGES_RENDUES: AtomicU64 = AtomicU64::new(0);
static REFUS: AtomicU64 = AtomicU64::new(0);
static PIC_PAGES: AtomicU64 = AtomicU64::new(0);
static EN_SERVICE: AtomicU64 = AtomicU64::new(0);

/// Configure l'allocateur de pages sur `[debut, debut + octets)`.
///
/// Rend `false` si la region est trop petite pour porter son bitmap et
/// quelques pages : l'appelant garde alors le `LockedHeap` comme backing, ce
/// qui est moins bon mais reste correct.
///
/// # Securite
/// `debut` doit designer une region virtuelle mappee d'au moins `octets`, dont
/// personne d'autre ne dispose.
pub unsafe fn configure(debut: usize, octets: usize) -> bool {
    if CONFIGURE.load(Ordering::Acquire) {
        return false;
    }
    let debut_aligne = (debut + PAGE - 1) & !(PAGE - 1);
    let fin = (debut + octets) & !(PAGE - 1);
    if fin <= debut_aligne {
        return false;
    }
    let pages_totales = (fin - debut_aligne) / PAGE;
    if pages_totales < 16 {
        return false;
    }

    // Le bitmap est dimensionne sur le total puis la base recalculee : majorer
    // reserve quelques mots inutiles, ce qui est sans consequence, tandis que
    // minorer ferait lire l'etat d'un bloc hors zone.
    let mots = compagnon::mots_bitmap(pages_totales);
    let pages_bitmap = (mots * 8 + PAGE - 1) / PAGE;
    if pages_totales <= pages_bitmap + 4 {
        return false;
    }
    let base = debut_aligne + pages_bitmap * PAGE;
    let blocs = (fin - base) / PAGE;

    let bitmap: &'static mut [u64] = {
        let pointeur = debut_aligne as *mut u64;
        core::ptr::write_bytes(pointeur as *mut u8, 0, pages_bitmap * PAGE);
        core::slice::from_raw_parts_mut(pointeur, mots)
    };

    let Some(mut allocateur) = Compagnon::neuf(blocs, bitmap) else {
        return false;
    };
    let mut terrain = TerrainVirtuel { base };

    // Alimente par les plus gros blocs alignes : rendre page par page donnerait
    // le meme resultat -- la fusion s'en charge -- pour beaucoup plus
    // d'operations.
    let mut position = 0usize;
    while position < blocs {
        let mut ordre = ORDRES - 1;
        loop {
            if compagnon::aligne(position, ordre) && position + (1usize << ordre) <= blocs {
                break;
            }
            if ordre == 0 {
                break;
            }
            ordre -= 1;
        }
        if !allocateur.rend(&mut terrain, position, ordre) {
            return false;
        }
        position += 1usize << ordre;
    }

    let plus_grand = allocateur.plus_grand_ordre().unwrap_or(0);
    *ETAT.lock() = Some(Etat { compagnon: allocateur, terrain, base, blocs });
    BASE.store(base, Ordering::Release);
    FIN.store(base + blocs * PAGE, Ordering::Release);
    CONFIGURE.store(true, Ordering::Release);
    crate::serial_println!(
        "BOUCHAUD_TAS_PAGES_PRET base={:#x} pages={} bitmap_pages={} plus_grand_ordre={} max_contigu={}",
        base, blocs, pages_bitmap, plus_grand, (1usize << (ORDRES - 1)) * PAGE,
    );
    true
}

/// L'allocateur de pages est-il en service ?
pub fn configure_ok() -> bool {
    CONFIGURE.load(Ordering::Acquire)
}

/// Cette adresse a-t-elle ete servie par CE module ?
///
/// C'est la regle qui route les liberations. Un bloc rendu apres le
/// basculement d'arene peut venir du `LockedHeap` d'amorcage comme du
/// compagnon ; seule son adresse le dit.
#[inline]
pub fn nous_appartient(adresse: usize) -> bool {
    let base = BASE.load(Ordering::Acquire);
    base != 0 && adresse >= base && adresse < FIN.load(Ordering::Acquire)
}

/// Alloue `octets` arrondis a la page. Rend l'adresse virtuelle, alignee page.
///
/// Rend `None` quand la demande depasse le plus gros bloc du compagnon ou que
/// celui-ci est epuise : l'appelant retombe alors sur le `LockedHeap`. Servir
/// une demande avec un bloc trop petit serait la pire des reponses, puisque
/// l'appelant ecrirait au-dela.
pub fn alloue(octets: usize) -> Option<usize> {
    if octets == 0 || !CONFIGURE.load(Ordering::Acquire) {
        return None;
    }
    let pages = (octets + PAGE - 1) / PAGE;
    let ordre = compagnon::ordre_pour(pages);
    // `ordre_pour` SATURE au dernier ordre : il rend dix pour n'importe quelle
    // demande superieure a quatre mebioctets, sans le dire. Servir cette
    // demande avec un bloc de dix serait rendre une adresse pour une taille
    // qu'on ne couvre pas, et l'appelant ecrirait au-dela -- un tampon de
    // trame 1920x1080x32 fait huit mebioctets et aurait touche ce cas au
    // premier rendu. Une demande qu'on ne peut pas porter ECHOUE.
    if (1usize << ordre) < pages {
        REFUS.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    let mut garde = ETAT.lock();
    let Some(etat) = garde.as_mut() else { return None };
    let Some(bloc) = etat.compagnon.prend(&mut etat.terrain, ordre) else {
        REFUS.fetch_add(1, Ordering::Relaxed);
        return None;
    };
    let servies = 1u64 << ordre;
    PAGES_SERVIES.fetch_add(servies, Ordering::Relaxed);
    let en_service = EN_SERVICE.fetch_add(servies, Ordering::AcqRel) + servies;
    PIC_PAGES.fetch_max(en_service, Ordering::Relaxed);
    Some(etat.base + bloc * PAGE)
}

/// Rend des pages allouees par [`alloue`], avec la MEME taille.
///
/// La taille doit etre celle de la demande : le compagnon retrouve l'ordre par
/// le calcul, pas par une table. Rendre une taille differente rendrait un bloc
/// d'un autre ordre, et la fusion travaillerait sur un jumeau qui n'existe pas.
pub fn libere(adresse: usize, octets: usize) {
    if octets == 0 || !nous_appartient(adresse) {
        REFUS.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let mut garde = ETAT.lock();
    let Some(etat) = garde.as_mut() else { return };
    if adresse < etat.base || (adresse - etat.base) % PAGE != 0 {
        REFUS.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let bloc = (adresse - etat.base) / PAGE;
    if bloc >= etat.blocs {
        REFUS.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let pages = (octets + PAGE - 1) / PAGE;
    let ordre = compagnon::ordre_pour(pages);
    // Symetrique du refus de `alloue` : une taille que le compagnon n'a pas pu
    // servir n'a pas pu venir de lui, et la rendre fabriquerait un bloc de
    // quatre mebioctets qu'il n'a jamais distribue.
    if (1usize << ordre) < pages {
        REFUS.fetch_add(1, Ordering::Relaxed);
        return;
    }
    if !etat.compagnon.rend(&mut etat.terrain, bloc, ordre) {
        REFUS.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let rendues = 1u64 << ordre;
    PAGES_RENDUES.fetch_add(rendues, Ordering::Relaxed);
    EN_SERVICE.fetch_sub(rendues.min(EN_SERVICE.load(Ordering::Acquire)), Ordering::AcqRel);
}

/// pages servies, pages rendues, pages en service, pic, refus, pages libres.
pub fn stats() -> (u64, u64, u64, u64, u64, usize) {
    let libres = ETAT.lock().as_ref().map(|e| e.compagnon.disponibles()).unwrap_or(0);
    (
        PAGES_SERVIES.load(Ordering::Relaxed),
        PAGES_RENDUES.load(Ordering::Relaxed),
        EN_SERVICE.load(Ordering::Relaxed),
        PIC_PAGES.load(Ordering::Relaxed),
        REFUS.load(Ordering::Relaxed),
        libres,
    )
}

/// La plus grande allocation contigue encore possible, en octets.
///
/// C'est la mesure de FRAGMENTATION qui compte : le nombre de pages libres ne
/// dit pas si l'on peut encore servir une demande contigue, et c'est cela qui
/// echoue en premier.
pub fn plus_grand_contigu() -> usize {
    ETAT.lock()
        .as_ref()
        .and_then(|e| e.compagnon.plus_grand_ordre())
        .map(|ordre| (1usize << ordre) * PAGE)
        .unwrap_or(0)
}

pub fn log_stats() {
    let (servies, rendues, en_service, pic, refus, libres) = stats();
    crate::serial_println!(
        "[MEM-NG-TAS-PAGES] servies={} rendues={} en_service={} pic={} refus={} libres={} plus_grand_contigu={}",
        servies, rendues, en_service, pic, refus, libres, plus_grand_contigu(),
    );
}
