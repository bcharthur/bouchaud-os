//! Pilote NVM Express : le disque interne de la machine de reference.
//!
//! Le TRIGKEY Speed S5 porte son systeme sur un M.2 NVMe. Jusqu'ici le noyau
//! ne savait parler qu'a l'ATA : `hardware_probe` DETECTAIT le controleur et
//! ecrivait « nvme: detected, runtime driver missing ». C'est la raison exacte
//! pour laquelle le systeme ne pouvait tourner qu'en live, depuis une cle USB,
//! avec un runtime en RAM -- il n'y avait aucun peripherique inscriptible qui
//! survive a une coupure.
//!
//! # Ce que ce pilote fait, et ce qu'il ne fait pas
//!
//! Il prend un controleur, le remet a zero, cree une file admin et UNE file
//! d'entree-sortie, identifie le controleur puis le namespace, et s'enregistre
//! dans `api::bloc` comme un volume ordinaire. A partir de la, le systeme de
//! fichiers, le commit A/B et l'installateur y accedent sans savoir que c'est
//! du NVMe.
//!
//! Il SCRUTE ses achevements. Il n'arme ni MSI-X ni interruption, et le dit :
//! `IEN` reste a zero dans la creation de file d'achevement. Une interruption
//! armee qu'aucun vecteur ne recoit ne rend pas le pilote plus rapide, elle
//! fait monter un IRQ non gere. Le passage aux interruptions ne changera pas
//! le contrat vu de `api::bloc`, qui a deja la forme d'une soumission suivie
//! d'un achevement.
//!
//! # Pourquoi un tampon de rebond
//!
//! `api::bloc` remet des tranches d'octets. Une tranche peut venir du tas
//! noyau, d'une pile, d'un statique -- et RIEN dans sa signature ne dit ou
//! elle vit physiquement. Le pilote pourrait deduire l'adresse physique du tas
//! noyau, qui est bien une region contigue mappee a decalage fixe ; il ne peut
//! pas le deduire des autres.
//!
//! Un pilote qui fait cette deduction pour tout le monde marche tant que tous
//! les appelants passent par le tas, et ecrit du DMA a une adresse physique
//! arbitraire le jour ou l'un d'eux n'y passe pas. Cette faute-la ne produit
//! pas une erreur : elle produit une corruption ailleurs en memoire, et le
//! symptome apparait dans un tout autre sous-systeme.
//!
//! Le pilote copie donc par une region qu'il a lui-meme allouee dans l'arene
//! DMA, dont il connait l'adresse physique parce qu'il l'a demandee. Le cout
//! est une recopie ; le benefice est qu'aucun appelant ne peut se tromper. Le
//! jour ou un chemin veut du zero-copie, il devra passer une adresse physique
//! explicite -- c'est-a-dire le dire.
//!
//! # Le decodage vit a cote
//!
//! Tout ce qui est de l'arithmetique -- foulee des sonnettes, compte de blocs
//! decale de un, plan PRP, format de bloc -- est dans `nvme/decodage.rs` et
//! mis a l'epreuve par `tools/platform/test_nvme_decodage.rs`. Ce fichier-ci
//! ne contient que ce qui touche vraiment le materiel.

use alloc::vec;
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{fence, AtomicBool, AtomicU32, AtomicU64, Ordering};

use crate::arch::x86_64::pci::{self, PciDevice};
use crate::drivers::bloc::{self, Achevement, Descripteur, Genre, PiloteBloc, Requete, Volume};
use crate::kernel::memory;
use crate::kernel::sync::SpinLockIrq;

include!("nvme/decodage.rs");

// Le cycle de vie d'une commande -- attribution, quarantaine sur echeance,
// rejet d'un achevement qui n'appartient a personne, arithmetique des deux
// anneaux -- vit dans `nvme/suivi.rs`, sans un seul acces materiel, et est
// mis a l'epreuve par `tools/platform/test_nvme_suivi.rs` avec les sequences
// que le materiel ne produit qu'une fois.
include!("nvme/suivi.rs");

/// Taille de page programmee dans `CC.MPS`. On reste sur 4 Kio : c'est la
/// taille minimale que tout controleur accepte, et celle de la pagination.
const PAGE: usize = 4096;

/// Entrees des files admin. Seize suffisent : l'initialisation emet une
/// poignee de commandes, et une file admin surdimensionnee ne sert a rien.
const ENTREES_ADMIN: u32 = 16;

/// Entrees de la file d'entree-sortie.
const ENTREES_ES: u32 = 64;

/// Identifiant de la file d'entree-sortie. La file 0 est la file admin.
const FILE_ES: u16 = 1;

/// Taille du tampon de rebond, en octets. 128 Kio couvre trente-deux pages,
/// c'est-a-dire un transfert de 256 blocs de 512 octets d'un seul coup.
const REBOND_OCTETS: usize = 128 * 1024;

/// Delai d'une commande d'entree-sortie ordinaire, en millisecondes.
///
/// # Pourquoi deux secondes, et non trente
///
/// Ce delai n'est pas une patience : c'est la duree pendant laquelle la
/// machine ENTIERE est arretee. `ETAT` masque les interruptions, donc pendant
/// l'attente il n'y a ni tick, ni scrutation xHCI, ni clavier, ni souris,
/// ni rafraichissement de l'ecran. Trente secondes par commande faisaient
/// d'un disque muet un gel indiscernable d'un plantage -- et le probe GPT
/// emet des dizaines de commandes.
///
/// Un NVMe sain acheve une lecture de secteur en moins d'une milliseconde.
/// Deux secondes couvrent trois ordres de grandeur de marge ; au-dela, le
/// disque ne repond pas, et l'attendre plus longtemps n'apprend rien.
const LIMITE_ES_MS: u64 = 2_000;

/// Delai d'une vidange de cache. Une vidange reelle peut etre longue.
const LIMITE_VIDANGE_MS: u64 = 5_000;

/// Nombre de delais consecutifs apres lequel le disque est mis hors service.
const DELAIS_AVANT_HORS_SERVICE: u64 = 2;

/// Attente maximale du jeton d'entree-sortie, en millisecondes.
///
/// Elle est prise interruptions ACTIVES : la machine vit pendant ce temps.
/// La borner reste necessaire -- un pilote bloque ne doit pas bloquer le
/// systeme de fichiers pour toujours.
const LIMITE_JETON_MS: u64 = 4_000;

/// Combien de temps laisser a un achevement tardif avant de refuser l'entree.
///
/// Court : au-dela, l'achevement n'arrivera probablement plus, et faire
/// patienter l'appelant ne fait que reporter le meme refus.
const LIMITE_QUARANTAINE_MS: u64 = 500;

/// Volume attribue au disque interne.
pub const VOLUME_INTERNE: Volume = Volume(2);

static PRESENT: AtomicBool = AtomicBool::new(false);
static LECTURES: AtomicU64 = AtomicU64::new(0);
static ECRITURES: AtomicU64 = AtomicU64::new(0);
static VIDANGES: AtomicU64 = AtomicU64::new(0);
static ERREURS: AtomicU64 = AtomicU64::new(0);
static DELAIS: AtomicU64 = AtomicU64::new(0);
/// Delais consecutifs sur le chemin d'entree-sortie.
static DELAIS_SUITE: AtomicU64 = AtomicU64::new(0);
/// Le disque a-t-il ete retire du service apres des delais repetes ?
static HORS_SERVICE: AtomicBool = AtomicBool::new(false);
/// Requetes refusees parce que le pilote etait deja occupe trop longtemps.
static OCCUPES: AtomicU64 = AtomicU64::new(0);

/// Une file, vue du pilote.
struct File {
    /// Adresse virtuelle de la zone DMA.
    virt: *mut u8,
    /// Adresse physique de la meme zone.
    phys: u64,
    entrees: u32,
    /// L'arithmetique de l'anneau d'achevement : index et phase.
    anneau: AnneauAchevement,
    /// Celle de l'anneau de soumission : queue publiee et tete du controleur.
    ///
    /// Les deux vivent dans la meme structure parce que les files admin et
    /// d'entree-sortie partagent ce type ; seul le champ pertinent sert.
    soumission: AnneauSoumission,
    /// Decalage de la sonnette dans le BAR0.
    sonnette: usize,
}

// Les pointeurs sont des adresses de l'arene DMA, jamais partagees ; l'etat
// entier est derriere un verrou.
unsafe impl Send for File {}

struct Etat {
    base: usize,
    caps: Capacites,
    admin_sq: File,
    admin_cq: File,
    /// Tampon de rebond : virtuel, physique.
    rebond_virt: *mut u8,
    rebond_phys: u64,
    /// Page de liste PRP, pour les transferts de plus de deux pages.
    liste_virt: *mut u8,
    liste_phys: u64,
    nsid: u32,
    format: FormatBloc,
    blocs: u64,
    /// Octets qu'une seule commande peut porter, tout plafond confondu.
    transfert_max: usize,
    /// Prochain identifiant de commande.
    prochain_id: u16,
    /// Le controleur sait-il vraiment vider son cache ?
    vidange_reelle: bool,
    modele: [u8; 40],
}

unsafe impl Send for Etat {}

static ETAT: SpinLockIrq<Option<Etat>> = SpinLockIrq::new(None);

// ---------------------------------------------------------------------------
// Acces registres
// ---------------------------------------------------------------------------

#[inline]
unsafe fn lit32(base: usize, decalage: usize) -> u32 {
    read_volatile((base + decalage) as *const u32)
}

#[inline]
unsafe fn ecrit32(base: usize, decalage: usize, valeur: u32) {
    write_volatile((base + decalage) as *mut u32, valeur);
}

#[inline]
unsafe fn lit64(base: usize, decalage: usize) -> u64 {
    // Deux lectures de 32 bits : certains ponts ne relaient pas un acces de
    // 64 bits vers l'espace memoire d'un peripherique, et rendent alors des
    // moities incoherentes plutot qu'une erreur.
    let bas = lit32(base, decalage) as u64;
    let haut = lit32(base, decalage + 4) as u64;
    bas | (haut << 32)
}

#[inline]
unsafe fn ecrit64(base: usize, decalage: usize, valeur: u64) {
    ecrit32(base, decalage, valeur as u32);
    ecrit32(base, decalage + 4, (valeur >> 32) as u32);
}

/// Attend qu'un predicat devienne vrai, au plus `limite_ms`.
///
/// # L'horloge de ce pilote n'est pas celle des IRQ
///
/// Tout ce fichier tourne sous `ETAT`, un `SpinLockIrq` : les interruptions
/// sont MASQUEES pendant l'attente. Une borne qui compterait des ticks PIT
/// compterait donc une horloge que l'attente elle-meme a arretee, et la
/// boucle ne sortirait jamais. `attente_bornee` porte une seconde borne, en
/// cycles `rdtsc`, qui ne depend ni des IRQ ni d'une calibration reussie.
fn attend(limite_ms: u64, predicat: impl FnMut() -> bool) -> bool {
    crate::kernel::timer::attente_bornee(limite_ms, predicat)
}

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

fn alloue_file(entrees: u32, taille_entree: usize, sonnette: usize) -> Option<File> {
    let octets = (entrees as usize * taille_entree).max(PAGE);
    let (phys, virt) = memory::alloc_dma(octets)?;
    Some(File {
        virt,
        phys,
        entrees,
        anneau: AnneauAchevement::neuf(entrees),
        soumission: AnneauSoumission::neuf(entrees),
        sonnette,
    })
}

impl File {
    /// Ecrit une commande dans la file de soumission et avance la queue.
    ///
    /// Rend `false` si le controleur n'a pas encore consomme assez d'entrees :
    /// ecrire quand meme ecraserait une commande qu'il n'a pas lue. Le cas ne
    /// se presente pas a profondeur un, et c'est precisement pourquoi il faut
    /// le traiter maintenant plutot que le decouvrir plus tard.
    unsafe fn pose(&mut self, sqe: &Sqe) -> bool {
        let Some(index) = self.soumission.pose() else { return false };
        let emplacement = self.virt.add(index as usize * TAILLE_SQE) as *mut u32;
        for (i, mot) in sqe.iter().enumerate() {
            write_volatile(emplacement.add(i), *mot);
        }
        true
    }

    /// La valeur a publier a la sonnette de soumission.
    fn queue_sq(&self) -> u32 {
        self.soumission.queue
    }

    /// Lit l'entree d'achevement courante, si sa phase est celle attendue.
    unsafe fn achevement(&self) -> Option<EntreeAchevement> {
        let emplacement = self.virt.add(self.anneau.tete as usize * TAILLE_CQE) as *const u32;
        let mut brut = [0u32; 4];
        // Le mot 3 porte la phase : le lire EN PREMIER, puis relire le reste,
        // garantit qu'on ne decode pas une entree a moitie ecrite.
        brut[3] = read_volatile(emplacement.add(3));
        let decode = decode_achevement(brut);
        if !self.anneau.est_neuve(decode.phase) {
            return None;
        }
        fence(Ordering::Acquire);
        brut[0] = read_volatile(emplacement);
        brut[1] = read_volatile(emplacement.add(1));
        brut[2] = read_volatile(emplacement.add(2));
        Some(decode_achevement(brut))
    }

    /// Avance la tete d'une file d'achevement, en basculant la phase au tour.
    fn avance(&mut self) {
        self.anneau.avance();
    }
}

unsafe fn sonne(base: usize, decalage: usize, valeur: u32) {
    fence(Ordering::SeqCst);
    ecrit32(base, decalage, valeur);
}

// ---------------------------------------------------------------------------
// Emission d'une commande
// ---------------------------------------------------------------------------

/// Emet une commande sur une paire de files et attend son achevement.
///
/// Rend `Err` sur delai depasse comme sur statut non nul. Un pilote qui
/// confondrait les deux ne pourrait pas distinguer un disque lent d'un disque
/// qui refuse -- et le second est le seul des deux qui doive arreter une
/// installation.
unsafe fn emet(
    base: usize,
    sq: &mut File,
    cq: &mut File,
    sqe: &Sqe,
    limite_ms: u64,
) -> Result<EntreeAchevement, &'static str> {
    let attendu = ((sqe[0] >> 16) & 0xFFFF) as u16;
    if !sq.pose(sqe) {
        return Err("soumission-pleine");
    }
    sonne(base, sq.sonnette, sq.queue_sq());

    let mut resultat: Option<EntreeAchevement> = None;
    let obtenu = attend(limite_ms, || {
        if let Some(a) = cq.achevement() {
            sq.soumission.tete_vue(a.tete_sq);
            if a.identifiant == attendu {
                resultat = Some(a);
                return true;
            }
            // Un achevement qui n'est pas le notre appartient a une commande
            // precedente dont on n'attendait plus rien. L'avaler evite que la
            // file se bloque dessus.
            cq.avance();
            ecrit32(base, cq.sonnette, cq.anneau.tete);
        }
        false
    });

    if !obtenu {
        DELAIS.fetch_add(1, Ordering::Relaxed);
        return Err("delai");
    }
    cq.avance();
    sonne(base, cq.sonnette, cq.anneau.tete);

    let a = resultat.ok_or("achevement-absent")?;
    if !a.reussi() {
        ERREURS.fetch_add(1, Ordering::Relaxed);
        crate::serial_println!(
            "BOUCHAUD_NVME_STATUT type={} code={} cid={}",
            a.type_statut,
            a.code_statut,
            a.identifiant
        );
        return Err("statut");
    }
    Ok(a)
}

impl Etat {
    fn identifiant(&mut self) -> u16 {
        // Zero reste libre : un identifiant nul dans un achevement inattendu
        // est le signe d'une entree jamais ecrite, et le distinguer aide.
        self.prochain_id = self.prochain_id.wrapping_add(1).max(1);
        self.prochain_id
    }

    unsafe fn admin(&mut self, sqe: &Sqe, limite_ms: u64) -> Result<EntreeAchevement, &'static str> {
        emet(self.base, &mut self.admin_sq, &mut self.admin_cq, sqe, limite_ms)
    }

}

// ---------------------------------------------------------------------------
// Le chemin d'entree-sortie : hors du gros verrou, interruptions ACTIVES
// ---------------------------------------------------------------------------
//
// # Ce que l'ancien chemin faisait, et pourquoi c'etait une panne
//
// `PiloteNvme::soumet` prenait `ETAT` -- un `SpinLockIrq` -- et le gardait
// pendant TOUT le transfert, attente comprise. Les interruptions restaient
// donc masquees jusqu'a deux secondes par commande : pas de tick, pas de
// scrutation xHCI, pas de clavier, pas de souris, pas de trame. Un disque un
// peu lent devenait un gel indiscernable d'un plantage, et le probe GPT emet
// des dizaines de commandes.
//
// Pire : pendant ce temps, les autres coeurs qui attendent un renvoi de TLB ou
// le meme verrou attendent aussi. Une seule commande lente arretait la machine
// entiere.
//
// # Les trois portees, separees
//
//   * `ETAT` protege la CONFIGURATION -- geometrie, format, namespace. Elle est
//     lue une fois par transfert, en quelques instructions.
//   * `FILES_ES` protege les FILES -- poser une entree de soumission, sonner,
//     drainer les achevements. Chaque prise dure quelques dizaines
//     d'instructions et ne contient AUCUNE attente.
//   * `EMPLACEMENTS_LIBRES` distribue les ressources DMA. Chaque emplacement
//     porte SON tampon de rebond et SA page de liste PRP : deux commandes
//     simultanees ne partagent rien. Ce n'est pas un verrou a interruptions
//     masquees : celui qui attend un emplacement le fait interruptions
//     ACTIVES, et draine les achevements pendant ce temps.
//
// L'attente de l'achevement se fait donc hors de tout verrou a interruptions
// masquees. Le systeme continue de vivre pendant qu'un disque reflechit.
//
// # Pourquoi un tableau d'achevements
//
// Celui qui attend draine la file d'achevement lui-meme, et peut y trouver
// l'achevement d'une AUTRE commande -- une commande abandonnee sur delai, dont
// la reponse arrive en retard. La jeter ferait boucler son proprietaire pour
// toujours ; le ranger par identifiant le lui rend, et permet demain plusieurs
// commandes en vol.

/// Identifiants de commande d'entree-sortie. Le tableau d'achevements est
/// indexe par cet identifiant, et il doit tenir en memoire statique.
const CID_ES_MAX: usize = 256;

/// Les files d'entree-sortie et le registre de base.
///
/// Elles vivent HORS d'`Etat` pour que leur verrou puisse etre pris et rendu
/// sans toucher a la configuration -- et surtout sans englober une attente.
struct FilesEs {
    base: usize,
    sq: File,
    cq: File,
}

unsafe impl Send for FilesEs {}

static FILES_ES: SpinLockIrq<Option<FilesEs>> = SpinLockIrq::new(None);

/// Combien de commandes d'entree-sortie peuvent voler ensemble.
///
/// # Pourquoi ce nombre existe, et pourquoi il n'etait pas la
///
/// La premiere version serialisait TOUTES les entrees-sorties derriere un
/// jeton unique, parce que le tampon de rebond etait unique. La profondeur
/// effective valait donc un, quoi qu'en dise la taille des files : soixante-
/// quatre entrees de soumission pour une seule commande utile.
///
/// Annoncer une profondeur superieure en partageant un tampon non partitionne
/// aurait ete pire que de ne rien annoncer : deux commandes se seraient
/// ecrasees l'une l'autre. Chaque emplacement porte donc SES ressources --
/// son tampon de rebond, sa page de liste PRP -- et rien n'est partage.
const EMPLACEMENTS_ES: usize = 4;

/// Les ressources DMA d'un emplacement. Rien n'est partage avec un autre.
#[derive(Clone, Copy)]
struct Emplacement {
    rebond_phys: u64,
    rebond_virt: *mut u8,
    liste_phys: u64,
    liste_virt: *mut u8,
}

unsafe impl Send for Emplacement {}

static EMPLACEMENTS: SpinLockIrq<Option<[Emplacement; EMPLACEMENTS_ES]>> =
    SpinLockIrq::new(None);

/// Masque des emplacements LIBRES. Bit a un = disponible.
///
/// Un masque atomique plutot qu'un verrou : la prise et le rendu sont deux
/// instructions, sur le chemin le plus chaud du pilote, et rien d'autre n'a
/// besoin d'etre serialise avec eux.
static EMPLACEMENTS_LIBRES: AtomicU32 = AtomicU32::new(0);

/// Entrees-sorties refusees faute d'emplacement libre.
static ATTENTES_EMPLACEMENT: AtomicU64 = AtomicU64::new(0);

/// Profondeur maximale reellement atteinte. Une profondeur annoncee que la
/// machine n'atteint jamais est une profondeur imaginaire.
static PROFONDEUR_MAX: AtomicU32 = AtomicU32::new(0);

/// Alloue les ressources DMA de tous les emplacements.
///
/// Chaque emplacement recoit SON tampon de rebond et SA page de liste PRP.
/// Rien n'est partage : c'est ce qui rend la profondeur superieure a un
/// legitime plutot qu'annoncee.
fn alloue_les_emplacements() -> Option<[Emplacement; EMPLACEMENTS_ES]> {
    let mut places = [Emplacement {
        rebond_phys: 0,
        rebond_virt: core::ptr::null_mut(),
        liste_phys: 0,
        liste_virt: core::ptr::null_mut(),
    }; EMPLACEMENTS_ES];
    for place in places.iter_mut() {
        let (rebond_phys, rebond_virt) = memory::alloc_dma(REBOND_OCTETS)?;
        let (liste_phys, liste_virt) = memory::alloc_dma(PAGE)?;
        *place = Emplacement { rebond_phys, rebond_virt, liste_phys, liste_virt };
    }
    Some(places)
}

/// Prend un emplacement libre, interruptions ACTIVES.
///
/// Rend son indice, ou `None` sur echeance. Un emplacement retenu par une
/// commande abandonnee n'est PAS dans le masque : la quarantaine est par
/// emplacement, et une commande lente n'arrete plus les autres.
fn prend_un_emplacement(limite_ms: u64) -> Option<usize> {
    let mut pris = usize::MAX;
    let obtenu = crate::kernel::timer::attente_bornee(limite_ms, || {
        let libres = EMPLACEMENTS_LIBRES.load(Ordering::Acquire);
        if libres == 0 {
            // Personne ne rendra son emplacement si les achevements ne sont
            // pas draines : c'est ici que les commandes en vol se terminent.
            draine_les_achevements();
            return false;
        }
        let indice = libres.trailing_zeros() as usize;
        let bit = 1u32 << indice;
        if EMPLACEMENTS_LIBRES
            .compare_exchange_weak(libres, libres & !bit, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            pris = indice;
            return true;
        }
        false
    });
    if !obtenu {
        ATTENTES_EMPLACEMENT.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    // Profondeur atteinte : le nombre d'emplacements pris a cet instant.
    let occupes = EMPLACEMENTS_ES as u32
        - EMPLACEMENTS_LIBRES.load(Ordering::Relaxed).count_ones();
    PROFONDEUR_MAX.fetch_max(occupes, Ordering::Relaxed);
    Some(pris)
}

/// Rend un emplacement au pot.
#[inline]
fn rend_un_emplacement(indice: usize) {
    EMPLACEMENTS_LIBRES.fetch_or(1u32 << indice, Ordering::Release);
}

/// Le cycle de vie des commandes d'entree-sortie.
///
/// Il a remplace un tableau d'atomiques indexe par `identifiant % 256`. Ce
/// tableau avait deux trous que rien n'attrapait au banc :
///
///   * une echeance depassee rendait l'identifiant au pot alors que le
///     controleur, lui, n'avait rien annule et pouvait encore ecrire dans le
///     tampon de rebond ;
///   * un achevement portant un identifiant jamais emis etait plie par modulo
///     sur le domaine suivi, ou il satisfaisait un attendant.
///
/// Le verrou n'englobe JAMAIS une attente : il est pris pour attribuer, pour
/// ranger, pour recolter, chaque fois quelques dizaines d'instructions.
static SUIVI: SpinLockIrq<Suivi> = SpinLockIrq::new(Suivi::neuf());

/// Achevements ranges pour un identifiant que plus personne n'attend.
static TARDIFS: AtomicU64 = AtomicU64::new(0);
/// Achevements refuses : inconnu, perime, double, hors domaine.
static REJETES: AtomicU64 = AtomicU64::new(0);
/// Entrees-sorties refusees parce qu'une quarantaine tenait encore le tampon.
static REFUS_QUARANTAINE: AtomicU64 = AtomicU64::new(0);

/// Commandes d'entree-sortie dont le releve detaille a ete publie.
///
/// Le releve coute cher : huit lignes sur un port serie a 115200 bauds font
/// quelques millisecondes. Les premieres commandes sont celles qui echouent
/// quand quelque chose ne va pas -- c'est la PREMIERE lecture reelle qui a
/// double-faute sur la machine de reference --, et les suivantes n'apprennent
/// rien de plus. Au-dela du plafond, seules les erreurs parlent.
static RELEVES_ES: AtomicU64 = AtomicU64::new(0);
const RELEVES_ES_MAX: u64 = 8;

/// Ce releve doit-il etre publie ?
#[inline]
fn releve_detaille() -> bool {
    RELEVES_ES.load(Ordering::Relaxed) < RELEVES_ES_MAX
}

/// Ce qu'une commande d'entree-sortie a besoin de savoir de la configuration.
///
/// Copie UNE fois sous `ETAT`, puis utilisee sans lui. Les adresses des
/// tampons ne changent pas de toute la vie du pilote.
#[derive(Clone, Copy)]
struct ContexteEs {
    nsid: u32,
    taille_bloc: usize,
    blocs: u64,
    transfert_max: usize,
    /// L'emplacement dont proviennent les adresses ci-dessous.
    emplacement: u8,
    rebond_phys: u64,
    rebond_virt: *mut u8,
    liste_phys: u64,
    liste_virt: *mut u8,
}

/// Lit la configuration, verrou pris quelques instructions.
fn contexte_es(emplacement: usize) -> Option<ContexteEs> {
    // Les ressources DMA d'abord : elles appartiennent a l'emplacement, pas a
    // la configuration. Les lire ici garantit qu'un contexte ne peut pas
    // porter les adresses d'un autre emplacement.
    let place = {
        let garde = EMPLACEMENTS.lock();
        *garde.as_ref()?.get(emplacement)?
    };
    let garde = ETAT.lock();
    let e = garde.as_ref()?;
    if e.nsid == 0 || e.format.taille_bloc == 0 {
        return None;
    }
    Some(ContexteEs {
        nsid: e.nsid,
        taille_bloc: e.format.taille_bloc,
        blocs: e.blocs,
        transfert_max: e.transfert_max,
        emplacement: emplacement as u8,
        rebond_phys: place.rebond_phys,
        rebond_virt: place.rebond_virt,
        liste_phys: place.liste_phys,
        liste_virt: place.liste_virt,
    })
}

/// Prend le jeton d'entree-sortie, interruptions ACTIVES.
///
/// Rend `false` sur echeance : mieux vaut une erreur rendue a l'appelant qu'une
/// attente sans fin sur un pilote bloque.
fn prend_le_jeton(limite_ms: u64) -> Option<usize> {
    let emplacement = prend_un_emplacement(limite_ms)?;
    // L'emplacement ne suffit pas. Une commande abandonnee sur echeance est
    // toujours la, du point de vue du controleur : il peut ecrire dans le
    // tampon de CET emplacement a n'importe quel moment. Le rendre a une
    // nouvelle entree-sortie serait autoriser une corruption silencieuse --
    // et pour une ecriture, ce serait autoriser le controleur a relire un
    // tampon qu'on aurait deja remplace, donc a poser sur le disque des
    // octets qui n'appartiennent pas au bloc demande.
    //
    // On laisse une derniere chance a l'achevement tardif d'arriver : c'est le
    // seul evenement qui leve une quarantaine sans reinitialiser le
    // controleur.
    if !tampon_disponible(emplacement) {
        let libere = crate::kernel::timer::attente_bornee(LIMITE_QUARANTAINE_MS, || {
            draine_les_achevements();
            tampon_disponible(emplacement)
        });
        if !libere {
            REFUS_QUARANTAINE.fetch_add(1, Ordering::Relaxed);
            crate::serial_println!(
                "NVME_IO_QUARANTAINE emplacement={} commandes={} consequence=entree-sortie-refusee",
                emplacement,
                quarantaine_es()
            );
            // L'emplacement N'EST PAS rendu : il reste retenu par la commande
            // abandonnee. Le rendre au masque le proposerait a la requete
            // suivante, qui se ferait ecraser par le meme achevement tardif.
            return None;
        }
    }
    Some(emplacement)
}

#[inline]
fn rend_le_jeton(emplacement: usize) {
    rend_un_emplacement(emplacement);
}

/// Recolte l'achevement d'un identifiant, verrou pris le temps d'un appel.
///
/// # Pourquoi une fonction pour une ligne
///
/// Le point d'attente ne doit tenir aucun verrou. En Rust, une garde placee
/// dans le scrutateur d'un `if let` vit jusqu'a la FIN du corps -- ce qui
/// suffirait a faire vivre ce verrou a travers une decision. La sortir dans
/// une fonction rend la duree de vie de la garde impossible a rallonger par
/// accident, et laisse `emet_es` litteralement sans `.lock()`.
#[inline]
fn recolte_es(cid: u16) -> Option<(u8, u8)> {
    SUIVI.lock().recolte(cid)
}

/// Met un identifiant en quarantaine. Rend l'emplacement retenu, ou `None` si
/// la commande s'etait achevee entre-temps.
#[inline]
fn expire_es(cid: u16) -> Option<u8> {
    SUIVI.lock().expire(cid)
}

/// Combien de commandes abandonnees retiennent encore le tampon de rebond.
#[inline]
fn quarantaine_es() -> u16 {
    SUIVI.lock().quarantaine()
}

/// Le tampon de CET emplacement est-il libre de toute commande abandonnee ?
///
/// La question est posee par emplacement, et non pour le pilote entier : une
/// commande lente sur un emplacement ne doit pas arreter les trois autres.
#[inline]
fn tampon_disponible(emplacement: usize) -> bool {
    !SUIVI.lock().emplacement_en_quarantaine(emplacement as u8)
}

/// Un identifiant d'entree-sortie libre, ou `None`.
///
/// Rendre `None` quand le pot est vide -- parce que des quarantaines le
/// retiennent -- est le comportement correct. Une entree-sortie refusee se
/// voit et se compte ; une entree-sortie qui reutilise un identifiant encore
/// vivant rend a son appelant un tampon que personne n'a rempli.
fn cid_es(emplacement: u8) -> Option<u16> {
    SUIVI.lock().alloue(emplacement)
}


/// Achevements lus en un seul passage sur la file.
///
/// Borne volontaire : le tableau vit sur la pile noyau, dont le budget est
/// verifie par `tools/verifie-pile-noyau-profondeur.py`. Seize entrees de
/// vingt octets font trois cent vingt octets, et il suffit de reboucler pour
/// vider une file plus pleine.
const LOT_ACHEVEMENTS: usize = 16;

/// Vide la file d'achevement, puis range ce qu'on en a tire.
///
/// # Pourquoi deux temps
///
/// Le verrou des files et celui du suivi ne sont JAMAIS tenus ensemble. Les
/// imbriquer creerait une arete d'ordre entre deux verrous que le reste du
/// pilote prend separement, et c'est exactement le genre d'arete qu'un
/// interblocage attend. Les achevements passent donc par la pile.
///
/// La sonnette n'est touchee QU'UNE fois, avec la tete finale : sonner par
/// achevement multiplierait les ecritures registre sans rien apprendre au
/// controleur.
fn draine_les_achevements() -> usize {
    let mut lot = [(0u16, 0u8, 0u8); LOT_ACHEVEMENTS];
    let mut lus = 0usize;
    let mut tete_sq: Option<u16> = None;
    {
        let mut garde = FILES_ES.lock();
        let Some(files) = garde.as_mut() else { return 0 };
        while lus < LOT_ACHEVEMENTS {
            let Some(a) = (unsafe { files.cq.achevement() }) else { break };
            lot[lus] = (a.identifiant, a.type_statut, a.code_statut);
            // La tete de soumission que le controleur publie dans CHAQUE
            // achevement est la seule facon de savoir combien de places
            // restent dans la file de soumission.
            tete_sq = Some(a.tete_sq);
            files.cq.avance();
            lus += 1;
        }
        if lus != 0 {
            if let Some(tete) = tete_sq {
                files.sq.soumission.tete_vue(tete);
            }
            unsafe { sonne(files.base, files.cq.sonnette, files.cq.anneau.tete) };
        }
    }

    if lus == 0 {
        return 0;
    }
    let mut suivi = SUIVI.lock();
    for &(cid, type_statut, code_statut) in &lot[..lus] {
        match suivi.range(cid, type_statut, code_statut) {
            Verdict::Attendu => {}
            Verdict::Tardif { emplacement } => {
                // Le controleur vient de prouver qu'il en a fini avec le
                // tampon de cet emplacement : il retourne au pot ICI, et
                // nulle part ailleurs.
                TARDIFS.fetch_add(1, Ordering::Relaxed);
                rend_un_emplacement(emplacement as usize);
                crate::serial_println!(
                    "NVME_IO_TARDIF cid={} emplacement={} consequence=emplacement-rendu",
                    cid, emplacement,
                );
            }
            verdict => {
                REJETES.fetch_add(1, Ordering::Relaxed);
                crate::serial_println!(
                    "NVME_IO_CQE_REJETE cid={} type={} code={} raison={}",
                    cid,
                    type_statut,
                    code_statut,
                    match verdict {
                        Verdict::Inconnu => "inconnu",
                        Verdict::Perime => "perime",
                        Verdict::Double => "double",
                        Verdict::HorsDomaine => "hors-domaine",
                        _ => "?",
                    }
                );
            }
        }
    }
    lus
}

/// Pose une commande d'entree-sortie et sonne. Verrou pris quelques
/// instructions, JAMAIS pendant une attente.
fn soumet_es(sqe: &Sqe) -> Result<(), &'static str> {
    let mut garde = FILES_ES.lock();
    let Some(files) = garde.as_mut() else { return Err("files-absentes") };
    unsafe {
        if !files.sq.pose(sqe) {
            // Le controleur n'a pas consomme assez d'entrees : poser quand
            // meme ecraserait une commande qu'il n'a pas encore lue.
            return Err("soumission-pleine");
        }
        if releve_detaille() {
            crate::serial_println!(
                "NVME_IO_DOORBELL sonnette={:#x} valeur={} base={:#x} sq_phys={:#x}",
                files.sq.sonnette, files.sq.queue_sq(), files.base, files.sq.phys,
            );
        }
        sonne(files.base, files.sq.sonnette, files.sq.queue_sq());
    }
    Ok(())
}

/// Emet une commande d'entree-sortie et attend son achevement, interruptions
/// ACTIVES pendant l'attente.
///
/// L'appelant doit tenir un emplacement.
fn emet_es(sqe: &Sqe, limite_ms: u64) -> Result<(), &'static str> {
    let cid = ((sqe[0] >> 16) & 0xFFFF) as u16;
    soumet_es(sqe)?;

    let mut statut = (0u8, 0u8);
    let obtenu = crate::kernel::timer::attente_bornee(limite_ms, || {
        // Le suivi d'abord : un autre attendant a pu ranger notre achevement.
        if let Some(vu) = recolte_es(cid) {
            statut = vu;
            return true;
        }
        draine_les_achevements();
        if let Some(vu) = recolte_es(cid) {
            statut = vu;
            return true;
        }
        // Rendre la main. Attendre un disque en tournant sur un coeur pendant
        // deux secondes prive la machine d'un seizieme de sa capacite pour
        // n'apprendre strictement rien de plus.
        //
        // La condition n'est pas une precaution de style : ce chemin est aussi
        // emprunte pendant la mise en service, avant que l'ordonnanceur ne
        // tourne et avant que les interruptions ne soient armees. Y appeler
        // `schedule()` reviendrait a commuter vers une tache qui n'existe pas.
        if peut_ceder_le_processeur() {
            crate::kernel::task::yield_now();
        }
        false
    });

    if !obtenu {
        DELAIS.fetch_add(1, Ordering::Relaxed);
        // L'identifiant passe en QUARANTAINE, il ne retourne pas au pot. La
        // commande n'est pas annulee : le controleur peut encore ecrire dans
        // son tampon. `expire` rend faux si l'achevement s'est range pendant
        // qu'on decidait d'abandonner -- la course existe, et la perdre
        // couterait un resultat valide.
        if expire_es(cid).is_none() {
            if let Some(vu) = recolte_es(cid) {
                statut = vu;
                crate::serial_println!("NVME_IO_DELAI_RATTRAPE cid={}", cid);
                return verdict_statut(cid, statut);
            }
        }
        let fatal = controleur_en_panne();
        crate::serial_println!(
            "NVME_IO_DELAI cid={} limite_ms={} quarantaine={} csts_fatal={}",
            cid,
            limite_ms,
            quarantaine_es(),
            fatal as u8,
        );
        if fatal {
            // Un controleur qui a leve CFS ne repondra plus a rien. Continuer
            // a lui parler ne produit que des echeances de deux secondes, une
            // par appel, jusqu'a ce que la machine paraisse figee.
            HORS_SERVICE.store(true, Ordering::Release);
            crate::serial_println!(
                "BOUCHAUD_NVME_HORS_SERVICE raison=csts-cfs cid={}",
                cid
            );
        }
        return Err("delai");
    }

    verdict_statut(cid, statut)
}

/// Peut-on rendre la main a l'ordonnanceur depuis ce point d'attente ?
///
/// Une seule condition a verifier ici : les interruptions. Sans elles, aucun
/// timer ne nous reveillera -- commuter serait s'endormir pour toujours -- et
/// l'ordonnanceur le refuse d'ailleurs par assertion. Le cas « pas encore de
/// tache courante », lui, est deja traite par `schedule()`, qui rend la main
/// sans rien commuter : ce chemin est emprunte pendant la mise en service,
/// avant que l'ordonnanceur ne tourne.
#[inline]
fn peut_ceder_le_processeur() -> bool {
    crate::arch::x86_64::cpu::interrupts_enabled()
}

/// Le controleur a-t-il leve son bit d'etat fatal ?
///
/// Question posee au MATERIEL, pas a un compteur d'echeances. Un disque lent
/// et un disque mort produisent le meme delai vu de l'appelant ; seul `CSTS`
/// les distingue.
fn controleur_en_panne() -> bool {
    let garde = FILES_ES.lock();
    let Some(files) = garde.as_ref() else { return false };
    panne_fatale(unsafe { lit32(files.base, registre::CSTS) })
}

/// Transforme un statut recolte en resultat.
fn verdict_statut(cid: u16, statut: (u8, u8)) -> Result<(), &'static str> {
    let (type_statut, code_statut) = statut;
    if releve_detaille() {
        crate::serial_println!(
            "NVME_IO_CQE_OK cid={} type={} code={}",
            cid, type_statut, code_statut,
        );
    }
    if type_statut != 0 || code_statut != 0 {
        ERREURS.fetch_add(1, Ordering::Relaxed);
        crate::serial_println!(
            "BOUCHAUD_NVME_STATUT type={} code={} cid={}",
            type_statut, code_statut, cid,
        );
        return Err("statut");
    }
    Ok(())
}

/// Prepare `PRP1`/`PRP2` pour un transfert depuis le tampon de rebond.
///
/// Hors d'`Etat` : le contexte porte deja les deux adresses, et prendre le
/// verrou de configuration pour lire deux nombres qu'on a deja serait un
/// verrou de plus sur le chemin chaud.
unsafe fn prp_es(ctx: &ContexteEs, octets: usize) -> (u64, u64, &'static str) {
    match plan_prp(ctx.rebond_phys, octets, PAGE) {
        Prp::UnePage { prp1 } => (prp1, 0, "une-page"),
        Prp::DeuxPages { prp1, prp2 } => (prp1, prp2, "deux-pages"),
        Prp::Liste { prp1, entrees } => {
            let liste = ctx.liste_virt as *mut u64;
            let maximum = PAGE / 8;
            let n = entrees.min(maximum);
            for i in 0..n {
                write_volatile(liste.add(i), page_de_la_liste(prp1, i, PAGE));
            }
            (prp1, ctx.liste_phys, "liste")
        }
    }
}

/// Un transfert d'un seul lot, emplacement tenu.
unsafe fn transfert_es(
    ctx: &ContexteEs,
    ecriture: bool,
    lba: u64,
    blocs: u32,
) -> Result<(), &'static str> {
    let octets = blocs as usize * ctx.taille_bloc;
    let (prp1, prp2, plan) = prp_es(ctx, octets);
    // Pas d'identifiant libre : le pot est retenu par des quarantaines. Refuser
    // est le seul comportement correct -- reutiliser un identifiant encore
    // vivant ferait rendre a l'appelant l'achevement d'une autre commande.
    let Some(cid) = cid_es(ctx.emplacement) else { return Err("identifiants-epuises") };
    if releve_detaille() {
        crate::serial_println!(
            "NVME_IO_PRP_READY cid={} plan={} prp1={:#x} prp2={:#x} octets={} rebond_phys={:#x} rebond_virt={:#x}",
            cid, plan, prp1, prp2, octets, ctx.rebond_phys, ctx.rebond_virt as usize,
        );
    }
    let sqe = commande_transfert(ecriture, cid, ctx.nsid, lba, blocs, prp1, prp2);
    if releve_detaille() {
        crate::serial_println!(
            "NVME_IO_SQE_READY cid={} nsid={} lba={} nlb={} opcode={} dw10={:#x} dw11={:#x} dw12={:#x}",
            cid, ctx.nsid, lba, blocs.saturating_sub(1), sqe[0] & 0xFF,
            sqe[10], sqe[11], sqe[12],
        );
        RELEVES_ES.fetch_add(1, Ordering::Relaxed);
    }
    emet_es(&sqe, LIMITE_ES_MS)
}

// ---------------------------------------------------------------------------
// Initialisation
// ---------------------------------------------------------------------------

/// Remet le controleur a zero puis le redemarre avec ses files admin.
unsafe fn reinitialise(base: usize, caps: &Capacites, sq: &File, cq: &File) -> Result<(), &'static str> {
    // Desarmer AVANT de toucher quoi que ce soit. Un controleur laisse actif
    // par le micrologiciel a deja des files a lui, dont les adresses ne
    // veulent plus rien dire une fois qu'on aura ecrit les notres.
    let cc = lit32(base, registre::CC);
    if cc & 1 != 0 {
        ecrit32(base, registre::CC, cc & !1);
    }
    if !attend(caps.delai_pret_ms as u64 + 500, || !pret(lit32(base, registre::CSTS))) {
        return Err("reset-bloque");
    }

    ecrit32(base, registre::AQA, valeur_aqa(sq.entrees, cq.entrees));
    ecrit64(base, registre::ASQ, sq.phys);
    ecrit64(base, registre::ACQ, cq.phys);

    // MPS : le logarithme de la taille de page, moins 12. On demande 4 Kio,
    // ce qui vaut zero -- mais seulement si le controleur l'accepte.
    if caps.mps_min > 0 {
        return Err("page-minimale-trop-grande");
    }
    ecrit32(base, registre::CC, valeur_cc_demarrage(0));

    if !attend(caps.delai_pret_ms as u64 + 500, || {
        let csts = lit32(base, registre::CSTS);
        pret(csts) || panne_fatale(csts)
    }) {
        return Err("demarrage-bloque");
    }
    if panne_fatale(lit32(base, registre::CSTS)) {
        return Err("panne-fatale");
    }
    Ok(())
}

/// Copie un champ ASCII d'`Identify Controller`, espaces de queue retires.
fn champ_ascii(source: &[u8], decalage: usize, taille: usize) -> [u8; 40] {
    let mut sortie = [b' '; 40];
    let n = taille.min(40);
    for i in 0..n {
        sortie[i] = source.get(decalage + i).copied().unwrap_or(b' ');
    }
    sortie
}

fn initialise(dev: PciDevice) -> Result<(Etat, FilesEs), &'static str> {
    let bar = pci::bar_decode(&dev, 0);
    let physique = bar.adresse();
    if physique == 0 {
        return Err("bar0-absent");
    }
    pci::enable_bus_master(&dev);
    let base = memory::phys_to_virt(physique) as usize;

    let caps = unsafe { decode_cap(lit64(base, registre::CAP)) };
    if !caps.jeu_nvm {
        return Err("jeu-nvm-absent");
    }
    let entrees_admin = ENTREES_ADMIN.min(caps.entrees_max);
    let entrees_es = ENTREES_ES.min(caps.entrees_max);

    let admin_sq = alloue_file(
        entrees_admin,
        TAILLE_SQE,
        decalage_sonnette(0, false, caps.foulee_sonnette),
    )
    .ok_or("dma-admin-sq")?;
    let admin_cq = alloue_file(
        entrees_admin,
        TAILLE_CQE,
        decalage_sonnette(0, true, caps.foulee_sonnette),
    )
    .ok_or("dma-admin-cq")?;

    unsafe { reinitialise(base, &caps, &admin_sq, &admin_cq)? };

    let es_sq = alloue_file(
        entrees_es,
        TAILLE_SQE,
        decalage_sonnette(FILE_ES, false, caps.foulee_sonnette),
    )
    .ok_or("dma-es-sq")?;
    let es_cq = alloue_file(
        entrees_es,
        TAILLE_CQE,
        decalage_sonnette(FILE_ES, true, caps.foulee_sonnette),
    )
    .ok_or("dma-es-cq")?;

    let (rebond_phys, rebond_virt) = memory::alloc_dma(REBOND_OCTETS).ok_or("dma-rebond")?;
    let (liste_phys, liste_virt) = memory::alloc_dma(PAGE).ok_or("dma-liste")?;

    let mut etat = Etat {
        base,
        caps,
        admin_sq,
        admin_cq,
        rebond_virt,
        rebond_phys,
        liste_virt,
        liste_phys,
        nsid: 0,
        format: FormatBloc { taille_bloc: 512, metadonnees: 0 },
        blocs: 0,
        transfert_max: REBOND_OCTETS,
        prochain_id: 0,
        vidange_reelle: false,
        modele: [b' '; 40],
    };

    // --- Identify Controller ------------------------------------------------
    let mut tampon = vec![0u8; PAGE];
    unsafe {
        let id = etat.identifiant();
        let sqe = commande_identifie(id, 0, cns::CONTROLEUR, etat.rebond_phys);
        etat.admin(&sqe, 5_000)?;
        core::ptr::copy_nonoverlapping(etat.rebond_virt, tampon.as_mut_ptr(), PAGE);
    }
    etat.modele = champ_ascii(&tampon, 24, 40);
    // `VWC` (octet 525), bit 0 : le controleur a-t-il un cache d'ecriture
    // volatil ? S'il n'en a pas, une vidange n'a rien a vider et la durabilite
    // est acquise des l'achevement de l'ecriture. S'il en a un, la vidange est
    // reelle et le commit A/B en depend.
    etat.vidange_reelle = tampon.get(525).map(|v| v & 1 != 0).unwrap_or(false);
    if let Some(max) = transfert_max_octets(&tampon, PAGE) {
        etat.transfert_max = etat.transfert_max.min(max);
    }

    // --- La file d'entree-sortie -------------------------------------------
    unsafe {
        let id = etat.identifiant();
        let sqe = commande_nombre_de_files(id, 1);
        // Un controleur qui refuse cet attribut n'est pas perdu : il garde son
        // nombre de files par defaut, qui est au moins un.
        let _ = etat.admin(&sqe, 5_000);

        let id = etat.identifiant();
        let sqe = commande_cree_cq(id, FILE_ES, es_cq.entrees, es_cq.phys);
        etat.admin(&sqe, 5_000)?;

        let id = etat.identifiant();
        let sqe = commande_cree_sq(id, FILE_ES, es_sq.entrees, es_sq.phys, FILE_ES);
        etat.admin(&sqe, 5_000)?;
    }

    // --- Le namespace -------------------------------------------------------
    // La liste plutot que « le namespace 1 » : un disque peut n'exposer que le
    // namespace 7, et supposer le premier echouerait sans rien dire.
    let mut identifiants = [0u32; 16];
    let trouves = unsafe {
        let id = etat.identifiant();
        let sqe = commande_identifie(id, 0, cns::LISTE_NAMESPACES, etat.rebond_phys);
        match etat.admin(&sqe, 5_000) {
            Ok(_) => {
                core::ptr::copy_nonoverlapping(etat.rebond_virt, tampon.as_mut_ptr(), PAGE);
                namespaces_actifs(&tampon, &mut identifiants)
            }
            Err(_) => 0,
        }
    };
    let candidats: &[u32] = if trouves == 0 { &[1] } else { &identifiants[..trouves] };

    for &nsid in candidats {
        let lu = unsafe {
            let id = etat.identifiant();
            let sqe = commande_identifie(id, nsid, cns::NAMESPACE, etat.rebond_phys);
            match etat.admin(&sqe, 5_000) {
                Ok(_) => {
                    core::ptr::copy_nonoverlapping(etat.rebond_virt, tampon.as_mut_ptr(), PAGE);
                    true
                }
                Err(_) => false,
            }
        };
        if !lu {
            continue;
        }
        let blocs = blocs_du_namespace(&tampon);
        let Some(format) = format_bloc(&tampon) else { continue };
        if blocs == 0 {
            continue;
        }
        etat.nsid = nsid;
        etat.blocs = blocs;
        etat.format = format;
        break;
    }

    if etat.nsid == 0 {
        return Err("aucun-namespace-utilisable");
    }
    // Le tampon de rebond borne le transfert autant que MDTS.
    etat.transfert_max = etat.transfert_max.min(REBOND_OCTETS);
    Ok((etat, FilesEs { base, sq: es_sq, cq: es_cq }))
}

// ---------------------------------------------------------------------------
// Le pilote vu de la couche bloc
// ---------------------------------------------------------------------------

pub struct PiloteNvme;

static PILOTE: PiloteNvme = PiloteNvme;

impl PiloteBloc for PiloteNvme {
    fn descripteur(&self) -> Descripteur {
        if hors_service() {
            return Descripteur::absent();
        }
        let garde = ETAT.lock();
        match garde.as_ref() {
            Some(e) => Descripteur {
                taille_bloc: e.format.taille_bloc,
                blocs: e.blocs,
                // La profondeur ANNONCEE est celle qu'on peut reellement
                // servir : un emplacement porte ses propres ressources DMA,
                // et rien n'est partage entre deux commandes en vol.
                profondeur_file: EMPLACEMENTS_ES,
                vidange_reelle: e.vidange_reelle,
                nom: "nvme0",
            },
            None => Descripteur::absent(),
        }
    }

    fn soumet(&self, requete: Requete, tampon: &mut [u8]) -> Achevement {
        if requete.genre != Genre::Lecture {
            return Achevement::Erreur;
        }
        self.transfere(false, requete, tampon, None)
    }

    fn soumet_ecriture(&self, requete: Requete, donnees: &[u8]) -> Achevement {
        match requete.genre {
            Genre::Vidange => self.vidange(),
            Genre::Ecriture => {
                let mut vide: [u8; 0] = [];
                self.transfere(true, requete, &mut vide, Some(donnees))
            }
            Genre::Lecture => Achevement::Erreur,
        }
    }
}

impl PiloteNvme {
    /// Un transfert complet, DECOUPE en lots et sans verrou pendant l'attente.
    ///
    /// # L'ordre des trois portees
    ///
    /// La configuration est lue d'abord, verrou rendu aussitot. Le jeton
    /// d'entree-sortie est pris ensuite, interruptions ACTIVES. Ce n'est
    /// qu'une fois le jeton tenu que les files sont touchees, et chaque prise
    /// de leur verrou dure quelques dizaines d'instructions.
    ///
    /// Aucune de ces trois portees ne contient d'attente. C'est la difference
    /// avec l'ancien chemin, qui gardait `ETAT` -- interruptions masquees --
    /// pendant les deux secondes que le disque pouvait prendre.
    fn transfere(
        &self,
        ecriture: bool,
        requete: Requete,
        tampon: &mut [u8],
        source: Option<&[u8]>,
    ) -> Achevement {
        if hors_service() {
            return Achevement::Absent;
        }
        // L'EMPLACEMENT D'ABORD, LE CONTEXTE ENSUITE.
        //
        // L'ordre n'est pas indifferent : le contexte porte les adresses DMA
        // de SON emplacement. Le construire avant de savoir lequel on aura
        // reviendrait a lire les tampons d'un emplacement qu'on ne tient pas.
        //
        // La prise se fait interruptions ACTIVES, et draine les achevements
        // pendant l'attente : celui qui patiente fait avancer les autres.
        let Some(emplacement) = prend_le_jeton(LIMITE_JETON_MS) else {
            OCCUPES.fetch_add(1, Ordering::Relaxed);
            return Achevement::Erreur;
        };
        let Some(ctx) = contexte_es(emplacement) else {
            rend_le_jeton(emplacement);
            return Achevement::Absent;
        };
        let octets_appelant = match source {
            Some(donnees) => donnees.len(),
            None => tampon.len(),
        };
        if !bornes_valides_ctx(&ctx, requete.lba, requete.blocs, octets_appelant) {
            rend_le_jeton(emplacement);
            return Achevement::Erreur;
        }
        if releve_detaille() {
            crate::serial_println!(
                "NVME_IO_READ_ENTER ecriture={} lba={} blocs={} taille_bloc={} nsid={} total_blocs={} transfert_max={} emplacement={}",
                ecriture as u8, requete.lba, requete.blocs, ctx.taille_bloc,
                ctx.nsid, ctx.blocs, ctx.transfert_max, emplacement,
            );
        }

        let taille = ctx.taille_bloc;
        let mut faits = 0usize;
        while faits < requete.blocs {
            let lot = ((ctx.transfert_max / taille).max(1)).min(requete.blocs - faits);
            let octets = lot * taille;
            let decalage = faits * taille;
            unsafe {
                if ecriture {
                    let Some(donnees) = source else { break };
                    if decalage + octets > donnees.len() {
                        break;
                    }
                    core::ptr::copy_nonoverlapping(
                        donnees.as_ptr().add(decalage),
                        ctx.rebond_virt,
                        octets,
                    );
                }
                if let Err(raison) =
                    transfert_es(&ctx, ecriture, requete.lba + faits as u64, lot as u32)
                {
                    // Sur echeance, l'emplacement N'EST PAS rendu : il est en
                    // quarantaine, et seul l'achevement tardif le liberera.
                    if raison == "delai" {
                        note_delai_es();
                    } else {
                        rend_le_jeton(emplacement);
                    }
                    return if faits == 0 {
                        Achevement::Erreur
                    } else {
                        Achevement::Fait(faits)
                    };
                }
                if !ecriture {
                    if decalage + octets > tampon.len() {
                        break;
                    }
                    if releve_detaille() {
                        crate::serial_println!(
                            "NVME_IO_COPY_BEGIN de={:#x} vers={:#x} octets={}",
                            ctx.rebond_virt as usize,
                            tampon.as_ptr() as usize + decalage,
                            octets,
                        );
                    }
                    core::ptr::copy_nonoverlapping(
                        ctx.rebond_virt,
                        tampon.as_mut_ptr().add(decalage),
                        octets,
                    );
                    if releve_detaille() {
                        crate::serial_println!("NVME_IO_COPY_END octets={}", octets);
                    }
                }
            }
            faits += lot;
        }
        rend_le_jeton(emplacement);
        if faits == 0 {
            return Achevement::Erreur;
        }
        note_reussite_es();
        if ecriture {
            ECRITURES.fetch_add(faits as u64, Ordering::Relaxed);
        } else {
            LECTURES.fetch_add(faits as u64, Ordering::Relaxed);
        }
        Achevement::Fait(faits)
    }

    /// Une vidange de cache.
    ///
    /// Un controleur sans cache volatil n'a rien a vider : emettre la commande
    /// quand meme serait correct, mais rendre `Fait` sans elle serait mentir a
    /// `api::bloc`, qui croise ce resultat avec `vidange_reelle` pour dire a
    /// l'appelant s'il a une barriere.
    fn vidange(&self) -> Achevement {
        if hors_service() {
            return Achevement::Absent;
        }
        let Some(emplacement) = prend_le_jeton(LIMITE_JETON_MS) else {
            OCCUPES.fetch_add(1, Ordering::Relaxed);
            return Achevement::Erreur;
        };
        let Some(ctx) = contexte_es(emplacement) else {
            rend_le_jeton(emplacement);
            return Achevement::Absent;
        };
        let Some(cid) = cid_es(ctx.emplacement) else {
            rend_le_jeton(emplacement);
            return Achevement::Erreur;
        };
        let sqe = commande_vidange(cid, ctx.nsid);
        let resultat = emet_es(&sqe, LIMITE_VIDANGE_MS);
        // Une vidange qui expire laisse elle aussi son emplacement en
        // quarantaine : le controleur n'a rien annule.
        if resultat.as_ref().err().copied() != Some("delai") {
            rend_le_jeton(emplacement);
        }
        match resultat {
            Ok(_) => {
                note_reussite_es();
                VIDANGES.fetch_add(1, Ordering::Relaxed);
                Achevement::Fait(0)
            }
            Err(raison) => {
                if raison == "delai" {
                    note_delai_es();
                }
                Achevement::Erreur
            }
        }
    }
}

/// La requete tient-elle dans le volume, et le tampon dans la requete ?
fn bornes_valides_ctx(ctx: &ContexteEs, lba: u64, blocs: usize, octets: usize) -> bool {
    if blocs == 0 || ctx.taille_bloc == 0 {
        return false;
    }
    let Some(fin) = lba.checked_add(blocs as u64) else { return false };
    if fin > ctx.blocs {
        return false;
    }
    match blocs.checked_mul(ctx.taille_bloc) {
        Some(besoin) => besoin <= octets,
        None => false,
    }
}


// ---------------------------------------------------------------------------
// Entree publique
// ---------------------------------------------------------------------------

/// Cherche un controleur NVMe, l'initialise, et l'enregistre comme volume.
///
/// Rend `true` quand un disque est utilisable. Le journal porte le detail :
/// une machine sans NVMe et une machine dont le NVMe refuse de demarrer ne
/// doivent pas se ressembler dans les traces.
pub fn bring_up() -> bool {
    if PRESENT.load(Ordering::Acquire) {
        return true;
    }
    let Some(dev) = pci::find_nvme() else {
        crate::serial_println!("BOUCHAUD_NVME_ABSENT");
        return false;
    };
    match initialise(dev) {
        Ok((etat, files)) => {
            let taille = etat.format.taille_bloc;
            let blocs = etat.blocs;
            let modele = etat.modele;
            let vidange = etat.vidange_reelle;
            let transfert = etat.transfert_max;
            // LES FILES AVANT LA CONFIGURATION, ET LA CONFIGURATION AVANT
            // L'ENREGISTREMENT.
            //
            // Un appelant qui verrait le volume enregistre avant que les files
            // existent emettrait une commande dans le vide. L'ordre inverse ne
            // coute rien et ferme la fenetre.
            // LES EMPLACEMENTS AVANT TOUT LE RESTE.
            //
            // Sans eux, `prend_le_jeton` trouve un masque vide et toute
            // entree-sortie echoue sur echeance. Leur echec est donc FATAL
            // pour la mise en service, et non silencieux : mieux vaut un
            // disque declare absent qu'un disque declare pret qui refuse
            // chaque lecture au bout de quatre secondes.
            let Some(places) = alloue_les_emplacements() else {
                crate::serial_println!(
                    "BOUCHAUD_NVME_HORS_SERVICE raison=dma-emplacements emplacements={}",
                    EMPLACEMENTS_ES
                );
                HORS_SERVICE.store(true, Ordering::Release);
                return false;
            };
            *EMPLACEMENTS.lock() = Some(places);
            EMPLACEMENTS_LIBRES.store((1u32 << EMPLACEMENTS_ES) - 1, Ordering::Release);
            *FILES_ES.lock() = Some(files);
            *ETAT.lock() = Some(etat);
            bloc::enregistre(VOLUME_INTERNE, &PILOTE);
            PRESENT.store(true, Ordering::Release);
            let mio = blocs.saturating_mul(taille as u64) / (1024 * 1024);
            crate::serial_println!(
                "BOUCHAUD_NVME_GREEN bdf={:02x}:{:02x}.{} blocs={} taille_bloc={} mio={} vidange={} transfert_max={} modele={}",
                dev.bus,
                dev.slot,
                dev.func,
                blocs,
                taille,
                mio,
                vidange as u8,
                transfert,
                core::str::from_utf8(&modele).unwrap_or("?").trim_end()
            );
            crate::serial_println!(
                "BOUCHAUD_NVME_EMPLACEMENTS nombre={} rebond_octets={} total_dma={}",
                EMPLACEMENTS_ES,
                REBOND_OCTETS,
                EMPLACEMENTS_ES * (REBOND_OCTETS + PAGE),
            );
            crate::kernel::dmesg::log("nvme: disque interne pret");
            true
        }
        Err(raison) => {
            crate::serial_println!(
                "BOUCHAUD_NVME_FAIL bdf={:02x}:{:02x}.{} raison={}",
                dev.bus,
                dev.slot,
                dev.func,
                raison
            );
            false
        }
    }
}

/// Le disque interne est-il utilisable ?
pub fn present() -> bool {
    PRESENT.load(Ordering::Acquire) && !hors_service()
}

/// Le disque a-t-il ete retire du service ?
///
/// # Pourquoi un disque muet doit cesser d'etre interroge
///
/// Un controleur qui ne repond pas ne repond pas UNE fois : il ne repond a
/// aucune des commandes suivantes. Sans cet etat, chaque lecture repayait le
/// delai entier, interruptions masquees -- et un probe de table de partitions,
/// qui lit trente-quatre blocs, multipliait ce delai par trente-quatre.
///
/// Deux delais consecutifs suffisent a conclure. Le disque est alors declare
/// absent, immediatement et pour toutes les requetes suivantes ; le systeme
/// continue sans persistance au lieu de s'arreter dessus.
pub fn hors_service() -> bool {
    HORS_SERVICE.load(Ordering::Acquire)
}

/// Compte un delai sur le chemin d'entree-sortie, et retire le disque du
/// service quand ils s'enchainent.
fn note_delai_es() {
    let suite = DELAIS_SUITE.fetch_add(1, Ordering::AcqRel) + 1;
    if suite >= DELAIS_AVANT_HORS_SERVICE && !HORS_SERVICE.swap(true, Ordering::AcqRel) {
        crate::serial_println!(
            "BOUCHAUD_NVME_HORS_SERVICE delais_consecutifs={} raison=aucun-achevement",
            suite
        );
        crate::kernel::dmesg::log("nvme: disque interne muet, retire du service");
    }
}

/// Une commande d'entree-sortie a abouti : la serie de delais est rompue.
fn note_reussite_es() {
    DELAIS_SUITE.store(0, Ordering::Release);
}

/// Compteurs, pour le diagnostic.
pub fn stats() -> (u64, u64, u64, u64, u64) {
    (
        LECTURES.load(Ordering::Relaxed),
        ECRITURES.load(Ordering::Relaxed),
        VIDANGES.load(Ordering::Relaxed),
        ERREURS.load(Ordering::Relaxed),
        DELAIS.load(Ordering::Relaxed),
    )
}

/// Publie l'etat du disque interne sur le port serie.
///
/// Muet tant qu'aucun controleur n'a ete trouve : une ligne vide a chaque
/// rapport rendrait la trace illisible sur une machine sans NVMe.
pub fn log_stats() {
    if !PRESENT.load(Ordering::Acquire) {
        return;
    }
    let (lectures, ecritures, vidanges, erreurs, delais) = stats();
    crate::serial_println!(
        "[NVME] lectures={} ecritures={} vidanges={} erreurs={} delais={} occupes={} hors_service={}",
        lectures, ecritures, vidanges, erreurs, delais,
        OCCUPES.load(Ordering::Relaxed),
        HORS_SERVICE.load(Ordering::Acquire) as u8,
    );
    // Ce qui manquait a la ligne precedente : ce que le pilote a REFUSE. Un
    // compteur d'erreurs a zero pendant que la quarantaine retient le tampon
    // decrit une machine saine qui ne lit plus rien.
    let (en_vol, quarantaine, echeances, tardifs_suivi, rejets) = {
        let g = SUIVI.lock();
        (
            g.en_vol(),
            g.quarantaine(),
            g.echeances,
            g.tardifs,
            g.rejets_inconnu + g.rejets_perime + g.rejets_double + g.rejets_hors_domaine,
        )
    };
    crate::serial_println!(
        "[NVME-SUIVI] en_vol={} quarantaine={} echeances={} tardifs={} rejets={} \
refus_quarantaine={} cqe_rejetes={} cqe_tardifs={}",
        en_vol,
        quarantaine,
        echeances,
        tardifs_suivi,
        rejets,
        REFUS_QUARANTAINE.load(Ordering::Relaxed),
        REJETES.load(Ordering::Relaxed),
        TARDIFS.load(Ordering::Relaxed),
    );
    // LA PROFONDEUR ATTEINTE, PAS LA PROFONDEUR ANNONCEE.
    //
    // Un pilote qui declare quatre emplacements et n'en occupe jamais deux a
    // une profondeur effective de un. La seule facon de le savoir est de
    // mesurer le maximum reellement observe.
    crate::serial_println!(
        "[NVME-FILE] emplacements={} libres={} profondeur_max={} attentes={}",
        EMPLACEMENTS_ES,
        EMPLACEMENTS_LIBRES.load(Ordering::Relaxed).count_ones(),
        PROFONDEUR_MAX.load(Ordering::Relaxed),
        ATTENTES_EMPLACEMENT.load(Ordering::Relaxed),
    );
}

// ---------------------------------------------------------------------------
// Sonde de concurrence
// ---------------------------------------------------------------------------

/// Combien de lecteurs la sonde lance.
const SONDE_LECTEURS: usize = 4;
/// Lectures par lecteur.
const SONDE_LECTURES: u64 = 8;

/// Lecteurs encore vivants.
static SONDE_RESTANTS: AtomicU32 = AtomicU32::new(0);
/// Lectures reussies, tous lecteurs confondus.
static SONDE_REUSSIES: AtomicU64 = AtomicU64::new(0);
/// Lectures echouees.
static SONDE_ECHECS: AtomicU64 = AtomicU64::new(0);
/// Prochain bloc a lire. Chaque lecteur en prend un different.
static SONDE_PROCHAIN: AtomicU64 = AtomicU64::new(0);

fn sonde_lecteur() -> ! {
    let mut tampon = [0u8; 512];
    for _ in 0..SONDE_LECTURES {
        // Des blocs DISTINCTS : deux lecteurs qui liraient le meme bloc
        // pourraient reussir en se partageant un cache imaginaire. Ceux-ci
        // n'en ont pas, mais la question ne doit pas se poser.
        let lba = SONDE_PROCHAIN.fetch_add(1, Ordering::Relaxed) % 1024;
        let requete = Requete::lecture(lba, 1);
        match PILOTE.soumet(requete, &mut tampon) {
            Achevement::Fait(_) => {
                SONDE_REUSSIES.fetch_add(1, Ordering::Relaxed);
            }
            _ => {
                SONDE_ECHECS.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    SONDE_RESTANTS.fetch_sub(1, Ordering::Release);
    crate::kernel::task::exit_current(0)
}

/// Emet des lectures depuis PLUSIEURS taches, et mesure la profondeur atteinte.
///
/// # Pourquoi cette sonde existe
///
/// Partitionner les ressources DMA par emplacement rend une profondeur
/// superieure a un POSSIBLE et SURE. Elle ne la rend pas ATTEINTE : le seul
/// appelant du pilote, le sondage GPT, est sequentiel, et le journal le
/// montrait sans ambiguite -- huit commandes, toutes sur `emplacement=0`.
///
/// Une profondeur annoncee que rien n'exerce est une profondeur imaginaire.
/// Cette sonde lance de vrais lecteurs, sur de vrais blocs distincts, et
/// publie le maximum reellement observe. Si le pilote ne sait pas servir deux
/// commandes ensemble, `profondeur_max` restera a un et le dira.
pub fn sonde_parallele() {
    if !present() || hors_service() {
        crate::serial_println!("NVME_PARALLELE_ABSENT");
        return;
    }
    PROFONDEUR_MAX.store(0, Ordering::Relaxed);
    SONDE_REUSSIES.store(0, Ordering::Relaxed);
    SONDE_ECHECS.store(0, Ordering::Relaxed);
    SONDE_RESTANTS.store(SONDE_LECTEURS as u32, Ordering::Release);

    let mut lances = 0usize;
    for _ in 0..SONDE_LECTEURS {
        if crate::kernel::task::spawn_noyau(sonde_lecteur, "nvme-sonde") {
            lances += 1;
        } else {
            SONDE_RESTANTS.fetch_sub(1, Ordering::Release);
        }
    }
    if lances == 0 {
        crate::serial_println!("NVME_PARALLELE_ECHEC raison=aucune-tache-creee");
        return;
    }

    let fini = crate::kernel::timer::attente_bornee(30_000, || {
        SONDE_RESTANTS.load(Ordering::Acquire) == 0
    });

    let (emplacements, libres, profondeur, attentes) = file_stats();
    crate::serial_println!(
        "NVME_PARALLELE lecteurs={} fini={} reussies={} echecs={} \
emplacements={} libres={} profondeur_max={} attentes={}",
        lances,
        fini as u8,
        SONDE_REUSSIES.load(Ordering::Relaxed),
        SONDE_ECHECS.load(Ordering::Relaxed),
        emplacements,
        libres,
        profondeur,
        attentes,
    );
    if !fini {
        crate::serial_println!("NVME_PARALLELE_ECHEC raison=echeance");
        return;
    }
    if SONDE_ECHECS.load(Ordering::Relaxed) != 0 {
        crate::serial_println!("NVME_PARALLELE_ECHEC raison=lecture-refusee");
        return;
    }
    if profondeur < 2 {
        // Ce n'est PAS une reussite. Les ressources sont partitionnees, mais
        // rien ne les a exercees ensemble : la profondeur effective vaut un.
        crate::serial_println!(
            "NVME_PARALLELE_SEQUENTIEL profondeur_max={} attendu=>=2",
            profondeur
        );
        return;
    }
    crate::serial_println!("NVME_PARALLELE_OK profondeur_max={}", profondeur);
}

/// Emplacements totaux, libres, profondeur maximale atteinte, attentes.
pub fn file_stats() -> (usize, u32, u32, u64) {
    (
        EMPLACEMENTS_ES,
        EMPLACEMENTS_LIBRES.load(Ordering::Relaxed).count_ones(),
        PROFONDEUR_MAX.load(Ordering::Relaxed),
        ATTENTES_EMPLACEMENT.load(Ordering::Relaxed),
    )
}

/// Commandes en vol, en quarantaine, et achevements refuses.
///
/// Publie pour que la BLACKBOX et les garde-fous puissent lire l'etat du
/// pilote sans passer par le port serie.
pub fn suivi_stats() -> (u16, u16, u32, u32, u32) {
    let g = SUIVI.lock();
    (
        g.en_vol(),
        g.quarantaine(),
        g.echeances,
        g.tardifs,
        g.rejets_inconnu + g.rejets_perime + g.rejets_double + g.rejets_hors_domaine,
    )
}

/// Requetes refusees faute d'avoir obtenu le jeton d'entree-sortie.
///
/// Non nul veut dire que le pilote est reste occupe plus longtemps que
/// `LIMITE_JETON_MS`. C'est la mesure de la contention du tampon de rebond
/// unique, et donc l'argument chiffre pour en avoir plusieurs.
pub fn occupes() -> u64 {
    OCCUPES.load(Ordering::Relaxed)
}

/// Capacite du disque interne en blocs et taille de bloc.
pub fn geometrie() -> Option<(u64, usize)> {
    if hors_service() {
        return None;
    }
    let garde = ETAT.lock();
    garde.as_ref().map(|e| (e.blocs, e.format.taille_bloc))
}
