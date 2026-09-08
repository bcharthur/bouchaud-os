//! Le DMA servi par l'allocateur compagnon.
//!
//! # Ce que l'arene ne pouvait pas faire
//!
//! `AreneDma` suit au plus **soixante-quatre** regions rendues. Au-dela, une
//! liberation est comptee dans `debordements` et **la memoire est perdue** --
//! pas corrompue, perdue : elle n'est plus dans la frontiere, plus dans la
//! liste, et rien ne la retrouvera avant le redemarrage.
//!
//! Ce plafond n'est pas atteint par le nombre de liberations : la fusion
//! recolle les voisines. Il est atteint par la FRAGMENTATION -- un pilote qui
//! alloue et rend des tampons de tailles differentes, ce que fait exactement
//! un pilote de stockage sous charge. Le defaut se manifeste alors comme une
//! machine qui, apres plusieurs heures, ne peut plus allouer de DMA, sans
//! qu'aucune erreur ne dise pourquoi.
//!
//! Le compagnon n'a pas de plafond de fragmentation : sa fusion est un ou
//! exclusif, et l'etat d'un bloc est un bit. Rendre de la memoire ne peut pas
//! echouer faute de place pour l'enregistrer.
//!
//! # Ou vit le bitmap
//!
//! Dans la region DMA elle-meme, en tete. C'est la seule memoire dont on est
//! sur qu'elle existe a ce moment : le tas noyau n'est pas encore bascule
//! quand la region est configuree, et une reservation statique dimensionnee
//! pour la plus grande machine imaginable serait payee par la plus petite.
//!
//! Onze kibioctets suffisent pour trente-deux mebioctets de DMA.
//!
//! # Le chainage vit dans les blocs libres
//!
//! Un bloc libre ne sert a personne : son premier mot porte le numero du bloc
//! libre suivant. C'est pour cela que `Terrain` n'ecrit que dans des blocs
//! libres, et pourquoi le test hote verifie qu'il n'ecrit jamais ailleurs.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::kernel::compagnon::{self, Compagnon, Terrain, ORDRES};
use crate::kernel::sync::SpinLockIrq;

/// Taille d'un bloc elementaire.
pub const PAGE: u64 = 4096;

/// Le terrain : les blocs vivent a `base + numero * PAGE`.
struct TerrainPhysique {
    base: u64,
}

impl Terrain for TerrainPhysique {
    fn lien(&self, bloc: usize) -> usize {
        let adresse = self.base + (bloc as u64) * PAGE;
        unsafe { core::ptr::read_volatile(crate::kernel::memory::phys_to_virt(adresse) as *const usize) }
    }
    fn pose_lien(&mut self, bloc: usize, suivant: usize) {
        let adresse = self.base + (bloc as u64) * PAGE;
        unsafe {
            core::ptr::write_volatile(crate::kernel::memory::phys_to_virt(adresse) as *mut usize, suivant);
        }
    }
}

struct Etat {
    compagnon: Compagnon<'static>,
    terrain: TerrainPhysique,
    /// Premiere adresse distribuable (apres le bitmap).
    base: u64,
    blocs: usize,
}

// Le bitmap et les blocs vivent dans la region DMA, jamais partages autrement
// que par ce verrou.
unsafe impl Send for Etat {}

static ETAT: SpinLockIrq<Option<Etat>> = SpinLockIrq::new(None);
static CONFIGURE: AtomicBool = AtomicBool::new(false);
static ECHECS: AtomicU64 = AtomicU64::new(0);
static PIC: AtomicU64 = AtomicU64::new(0);

/// Configure l'allocateur sur `[debut, fin)`.
///
/// Rend `false` quand la region est trop petite pour porter son bitmap et au
/// moins quelques blocs. L'appelant retombe alors sur l'arene : mieux vaut
/// l'ancien allocateur que pas d'allocateur du tout.
pub fn configure(debut: u64, fin: u64) -> bool {
    if CONFIGURE.load(Ordering::Acquire) || fin <= debut {
        return false;
    }
    let debut = (debut + PAGE - 1) & !(PAGE - 1);
    let fin = fin & !(PAGE - 1);
    if fin <= debut {
        return false;
    }
    let pages_totales = ((fin - debut) / PAGE) as usize;
    if pages_totales < 16 {
        return false;
    }

    // Le bitmap est dimensionne pour les pages qui RESTERONT apres lui. On
    // majore d'abord, puis on recalcule : se dimensionner sur le total ferait
    // reserver quelques mots de trop, ce qui est sans consequence, tandis que
    // se dimensionner trop court ferait lire l'etat d'un bloc hors zone.
    let mots = compagnon::mots_bitmap(pages_totales);
    let octets_bitmap = (mots * 8) as u64;
    let pages_bitmap = (octets_bitmap + PAGE - 1) / PAGE;
    if pages_totales as u64 <= pages_bitmap + 4 {
        return false;
    }
    let base = debut + pages_bitmap * PAGE;
    let blocs = ((fin - base) / PAGE) as usize;

    let bitmap: &'static mut [u64] = unsafe {
        let pointeur = crate::kernel::memory::phys_to_virt(debut) as *mut u64;
        core::ptr::write_bytes(pointeur as *mut u8, 0, (pages_bitmap * PAGE) as usize);
        core::slice::from_raw_parts_mut(pointeur, mots)
    };

    let Some(mut allocateur) = Compagnon::neuf(blocs, bitmap) else {
        return false;
    };
    let mut terrain = TerrainPhysique { base };

    // Alimente en rendant les plus gros blocs alignes possibles. Rendre bloc
    // par bloc donnerait le meme resultat -- la fusion s'en charge -- pour
    // beaucoup plus d'operations.
    let mut position = 0usize;
    while position < blocs {
        let mut ordre = ORDRES - 1;
        loop {
            let taille = 1usize << ordre;
            if compagnon::aligne(position, ordre) && position + taille <= blocs {
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

    // On lit le plus grand ordre AVANT de poser l'etat : formater un message
    // en tenant le verrou de l'allocateur donnerait au port serie l'occasion
    // de le tenir a son tour, et l'ordre entre les deux verrous n'est ecrit
    // nulle part.
    let plus_grand = allocateur.plus_grand_ordre().unwrap_or(0);
    *ETAT.lock() = Some(Etat { compagnon: allocateur, terrain, base, blocs });
    CONFIGURE.store(true, Ordering::Release);
    crate::serial_println!(
        "[MEM-NG-COMPAGNON] base={:#x} blocs={} bitmap_pages={} plus_grand_ordre={} max_contigu={}",
        base,
        blocs,
        pages_bitmap,
        plus_grand,
        (1usize << (ORDRES - 1)) * PAGE as usize,
    );
    true
}

pub fn configure_ok() -> bool {
    CONFIGURE.load(Ordering::Acquire)
}

/// Alloue `taille` octets contigus, alignes sur une page.
///
/// Le plafond est le plus gros bloc de l'allocateur -- quatre mebioctets avec
/// onze ordres et des pages de quatre kibioctets. Une demande plus grande
/// ECHOUE, elle n'est pas servie par un bloc trop petit : rendre une adresse
/// pour une taille qu'on ne couvre pas serait la pire des reponses, puisque
/// l'appelant ecrirait au-dela.
pub fn alloue(taille: usize) -> Option<u64> {
    if taille == 0 {
        return None;
    }
    let pages = ((taille as u64 + PAGE - 1) / PAGE) as usize;
    let ordre = compagnon::ordre_pour(pages);
    // `ordre_pour` sature au dernier ordre : une demande plus grande que le
    // plus gros bloc doit ECHOUER, pas etre servie par un bloc trop petit.
    if (1usize << ordre) < pages {
        ECHECS.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    let mut garde = ETAT.lock();
    let etat = garde.as_mut()?;
    let bloc = match etat.compagnon.prend(&mut etat.terrain, ordre) {
        Some(bloc) => bloc,
        None => {
            ECHECS.fetch_add(1, Ordering::Relaxed);
            return None;
        }
    };
    let utilise = (etat.blocs - etat.compagnon.disponibles()) as u64 * PAGE;
    PIC.fetch_max(utilise, Ordering::Relaxed);
    Some(etat.base + (bloc as u64) * PAGE)
}

/// Rend une allocation.
///
/// `base` et `taille` doivent etre ceux d'un appel a [`alloue`]. Une adresse
/// hors zone est comptee et ignoree : l'ajouter fabriquerait un bloc qui
/// n'existe pas, et les allocations suivantes le distribueraient.
pub fn libere(base: u64, taille: usize) {
    if taille == 0 {
        return;
    }
    let mut garde = ETAT.lock();
    let Some(etat) = garde.as_mut() else { return };
    if base < etat.base {
        ECHECS.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let decalage = base - etat.base;
    if decalage % PAGE != 0 {
        ECHECS.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let bloc = (decalage / PAGE) as usize;
    if bloc >= etat.blocs {
        ECHECS.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let pages = ((taille as u64 + PAGE - 1) / PAGE) as usize;
    let ordre = compagnon::ordre_pour(pages);
    if !etat.compagnon.rend(&mut etat.terrain, bloc, ordre) {
        ECHECS.fetch_add(1, Ordering::Relaxed);
    }
}

/// L'etat, dans la meme forme que celui de l'arene.
///
/// Les diagnostics et le releve periodique lisent cette structure ; changer
/// d'allocateur ne doit pas leur demander de changer.
pub fn etat() -> crate::kernel::arene_dma::EtatDma {
    let garde = ETAT.lock();
    let Some(e) = garde.as_ref() else {
        return crate::kernel::arene_dma::EtatDma::default();
    };
    let stats = e.compagnon.statistiques();
    let libre = stats.disponibles as u64 * PAGE;
    let total = e.blocs as u64 * PAGE;
    crate::kernel::arene_dma::EtatDma {
        total,
        utilise: total - libre,
        libre,
        rendu: libre,
        // Le compagnon n'a pas de liste de regions bornee : le nombre
        // « regions » devient le plus grand ordre encore disponible, qui est
        // la mesure utile -- un allocateur dont le plus grand ordre tombe a
        // zero ne peut plus rien servir de contigu, meme s'il a de la place.
        regions: e.compagnon.plus_grand_ordre().unwrap_or(0) as u64,
        pic: PIC.load(Ordering::Relaxed),
        allocations: stats.allocations,
        liberations: stats.liberations,
        reutilisations: stats.fusions,
        fusions: stats.fusions,
        debordements: 0,
        echecs: stats.echecs + ECHECS.load(Ordering::Relaxed),
    }
}
