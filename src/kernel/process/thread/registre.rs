// BOUCHAUD_C1_REGISTRE_TACHES_V1
//
// LE REGISTRE DES TACHES, SANS GROS VERROU
// ========================================
//
// La table etait `static mut TASKS: Option<Vec<Box<Task>>>`, et `tasks()`
// rendait un `&'static mut Vec<Box<Task>>` a qui le demandait. Rien ne la
// protegeait : c'est le gros verrou, pris par tous les appelants, qui rendait
// l'ensemble sur. Tant que la table a cette forme, aucun chemin qui la lit ne
// peut sortir du verrou global.
//
// Deux dangers, et un seul est un vrai danger.
//
//   1. La REALLOCATION du `Vec`. `push` peut deplacer le tableau de pointeurs
//      sous les pieds d'un lecteur. C'est une course franche.
//   2. Le RECYCLAGE d'un emplacement. `tasks()[index] = task` DETRUIT
//      l'ancienne `Box<Task>` ; un lecteur qui en tenait une reference lirait
//      de la memoire liberee.
//
// Ce que le code garantissait deja, et qu'on garde : en SMP la table n'est
// JAMAIS compactee. `CURRENT` est un indice, et le compactage le rendrait
// faux. Les indices sont donc stables a vie, et les emplacements se recyclent.
//
// # La forme retenue
//
// Un tableau d'emplacements de taille fixe, chacun un `AtomicPtr<Task>` :
//
//   * la lecture par indice est SANS VERROU -- un seul `load` atomique ;
//   * l'ajout n'alloue aucun tableau, il ecrit dans l'emplacement suivant :
//     plus de reallocation, donc plus de danger 1 ;
//   * le recyclage REUTILISE L'ALLOCATION au lieu de la liberer : l'adresse
//     d'une `Task` ne change jamais et n'est jamais rendue, donc plus de
//     danger 2.
//
// Ne jamais liberer coute une `Task` par emplacement recycle -- quelques
// kilo-octets, borne par `MAX_TACHES`. C'est le prix d'une lecture sans
// verrou, et il est petit : l'alternative demanderait une reclamation
// differee (epoques, RCU) qu'on ne peut pas s'offrir tant que
// l'ordonnanceur lui-meme est en cours de migration.
//
// # Ce que ce module ne pretend pas resoudre
//
// Il rend l'ACCES a la table sur sans verrou global. Il ne rend pas sur, a lui
// seul, la modification concurrente du CONTENU d'une tache : c'est le role des
// champs atomiques de `modeles.rs`, poses au lot precedent. Les deux ensemble
// permettent de retirer le verrou des chemins de reveil ; ni l'un ni l'autre
// n'y suffit.

use core::sync::atomic::AtomicPtr;

/// Nombre maximal de taches simultanement enregistrees.
///
/// Le `Vec` etait sans borne. Une borne est le prix d'emplacements a adresse
/// stable -- et elle est de toute facon atteinte bien avant par la memoire :
/// chaque tache porte une pile noyau et une zone FPU.
pub const MAX_TACHES: usize = 1024;

static EMPLACEMENTS: [AtomicPtr<Task>; MAX_TACHES] =
    [const { AtomicPtr::new(core::ptr::null_mut()) }; MAX_TACHES];

/// Nombre d'emplacements jamais occupes. Ne DECROIT jamais : les indices sont
/// stables a vie, et un emplacement libere reste comptabilise pour que
/// `CURRENT` conserve son sens.
static LONGUEUR: AtomicUsize = AtomicUsize::new(0);

/// Serialise UNIQUEMENT l'ajout et le recyclage.
///
/// Ni la lecture, ni la modification d'une tache ne passent par la. C'est tout
/// l'ecart avec le gros verrou : celui-ci etait pris pour lire un champ, celui-la
/// ne l'est que pour choisir un emplacement.
static STRUCTURE: crate::kernel::sync::SpinLock<()> =
    crate::kernel::sync::SpinLock::new(());


/// Nombre d'emplacements enregistres.
#[inline]
pub fn registre_longueur() -> usize {
    LONGUEUR.load(Ordering::Acquire)
}

/// La tache a cet indice, sans verrou.
///
/// Rend `None` pour un indice hors bornes ou un emplacement jamais occupe.
/// L'adresse rendue reste valide pour toute la vie du systeme : un emplacement
/// recycle est REECRIT, jamais libere.
#[inline]
pub fn registre_tache(index: usize) -> Option<&'static Task> {
    if index >= MAX_TACHES {
        return None;
    }
    let pointeur = EMPLACEMENTS[index].load(Ordering::Acquire);
    if pointeur.is_null() {
        None
    } else {
        Some(unsafe { &*pointeur })
    }
}

/// La tache a cet indice, en acces exclusif.
///
/// # Ce que l'appelant doit garantir
///
/// Les champs que plusieurs coeurs se disputent sont ATOMIQUES : les toucher
/// par cette reference est sur. Les autres -- pile noyau, contexte, zone FPU --
/// n'appartiennent qu'au CPU qui execute la tache, et l'ordonnanceur ne les
/// touche qu'a des instants ou elle n'est sur aucun coeur.
///
/// C'est exactement le contrat qu'avait `&'static mut Vec<Box<Task>>`, en plus
/// etroit : il portait sur toute la table, celui-ci sur une seule tache.
#[inline]
#[allow(clippy::mut_from_ref)]
pub fn registre_tache_mut(index: usize) -> Option<&'static mut Task> {
    if index >= MAX_TACHES {
        return None;
    }
    let pointeur = EMPLACEMENTS[index].load(Ordering::Acquire);
    if pointeur.is_null() {
        None
    } else {
        Some(unsafe { &mut *pointeur })
    }
}

/// Enregistre une tache et rend son indice.
///
/// Reutilise l'emplacement d'une tache morte quand il y en a un. Le contenu
/// est ECRASE SUR PLACE : l'ancienne allocation reste, et son adresse aussi,
/// ce qui garantit qu'aucun lecteur ne tombe sur de la memoire liberee.
///
/// Rend `None` si le registre est plein.
pub fn registre_ajoute(tache: Box<Task>, recyclable: impl Fn(&Task) -> bool) -> Option<usize> {
    let _structure = STRUCTURE.lock();

    let longueur = LONGUEUR.load(Ordering::Acquire);
    for index in 0..longueur {
        let pointeur = EMPLACEMENTS[index].load(Ordering::Acquire);
        if pointeur.is_null() {
            continue;
        }
        // SECURITE : sous `STRUCTURE`, aucun autre ajout ne peut choisir le
        // meme emplacement. Le predicat verifie en plus que la tache n'est sur
        // aucun coeur -- personne n'en modifie donc le contenu.
        let ancienne = unsafe { &mut *pointeur };
        if recyclable(ancienne) {
            *ancienne = *tache;
            return Some(index);
        }
    }

    if longueur >= MAX_TACHES {
        return None;
    }
    // `Box::into_raw` : l'allocation n'est plus jamais rendue. C'est
    // deliberé, et c'est ce qui rend `tache()` sur sans verrou.
    EMPLACEMENTS[longueur].store(Box::into_raw(tache), Ordering::Release);
    // La longueur monte APRES le pointeur : un lecteur qui voit l'indice voit
    // donc forcement un emplacement deja rempli.
    LONGUEUR.store(longueur + 1, Ordering::Release);
    Some(longueur)
}

/// Parcourt les taches enregistrees.
pub fn registre_iter() -> impl Iterator<Item = &'static Task> {
    (0..registre_longueur()).filter_map(registre_tache)
}

/// Parcourt les taches enregistrees en acces exclusif. Meme contrat que
/// [`tache_mut`].
pub fn registre_iter_mut() -> impl Iterator<Item = &'static mut Task> {
    (0..registre_longueur()).filter_map(registre_tache_mut)
}

/// Indice de la premiere tache satisfaisant le predicat.
pub fn registre_position(predicat: impl Fn(&Task) -> bool) -> Option<usize> {
    (0..registre_longueur()).find(|&index| registre_tache(index).is_some_and(&predicat))
}
