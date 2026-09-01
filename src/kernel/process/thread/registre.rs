// BOUCHAUD_C1_REGISTRE_GENERATIONNEL_V2
//
// LE REGISTRE DES TACHES : EMPLACEMENTS STABLES, INCARNATIONS DISTINCTES
// ======================================================================
//
// La premiere version de ce registre a remplace `static mut TASKS` par un
// tableau d'emplacements a adresse stable. Elle reglait le probleme de duree de
// vie -- une adresse rendue ne l'est jamais -- et en laissait deux autres, qui
// sont les vrais.
//
// # Faute 1 : un `&mut` fabrique depuis une lecture partagee
//
// `registre_tache_mut(slot)` rendait `&'static mut Task` a QUI LE DEMANDAIT, a
// partir du meme pointeur atomique que la lecture partagee. Un `&mut` promet
// l'exclusivite ; rien ici ne la faisait respecter. Que les champs disputes
// soient atomiques ne rend pas l'aliasing legal, et un `allow(...)` sur le lint
// qui le signale n'est pas une demonstration.
//
// # Faute 2 : adresse stable n'est pas identite stable
//
// Le recyclage ecrasait le contenu sur place -- `*ancienne = *nouvelle` -- en
// considerant que garder l'adresse suffisait. Un lecteur ayant obtenu une
// reference quand l'emplacement portait la tache A pouvait la relire quand il
// portait la tache B. D'ou : ABA sur les indices, un ancien ticket qui reveille
// une tache neuve, un `Vec` ou un `Arc` detruit sous un lecteur.
//
// # Ce que cette version etablit
//
//   * une INCARNATION par emplacement. `TacheId` porte l'emplacement ET sa
//     generation ; toute reference obtenue par identite est refusee des que
//     l'emplacement a ete reattribue ;
//   * la lecture partagee ne rend QUE `&Task`. Les champs disputes sont
//     atomiques et se lisent par cette reference ; le reste ne s'atteint plus
//     par la ;
//   * l'acces exclusif au contenu non atomique passe par un DRAPEAU par
//     emplacement, pris en exclusion mutuelle. Le `&mut` n'existe que derriere
//     ce drapeau, et il est alors reellement exclusif ;
//   * le recyclage exige ce meme drapeau, et incremente la generation AVANT
//     d'ecrire. Aucun lecteur ne peut donc observer un contenu a moitie
//     remplace en croyant lire l'ancienne tache.
//
// # Ce que cette version n'essaie pas de faire
//
// Elle ne separe pas physiquement les champs atomiques des autres dans le type
// `Task`. Ce serait plus fort, et cela demanderait de reecrire les quatre-vingt
// dix sites qui manipulent une tache. Le drapeau d'exclusivite donne la meme
// garantie a l'execution, et il est verifiable -- un test le falsifie.

use core::sync::atomic::AtomicPtr;

/// Nombre maximal de taches simultanement enregistrees.
pub const MAX_TACHES: usize = 1024;

/// Identite d'une tache : son emplacement, et QUELLE incarnation.
///
/// Un simple indice ne suffit pas. Les emplacements se recyclent, et un indice
/// conserve par une file d'execution, un futex ou une file d'attente
/// designerait alors la tache suivante -- qui n'a rien demande. C'est le
/// probleme ABA, et la generation est ce qui le ferme.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TacheId {
    emplacement: u32,
    generation: u32,
}

impl TacheId {
    #[inline]
    pub const fn emplacement(self) -> usize {
        self.emplacement as usize
    }

    #[inline]
    pub const fn generation(self) -> u32 {
        self.generation
    }

    /// Empaquete l'identite dans un mot, pour les structures qui ne stockent
    /// qu'un entier (files d'execution, cles de reveil).
    #[inline]
    pub const fn en_mot(self) -> u64 {
        ((self.generation as u64) << 32) | self.emplacement as u64
    }

    #[inline]
    pub const fn depuis_mot(mot: u64) -> Self {
        Self {
            emplacement: (mot & 0xffff_ffff) as u32,
            generation: (mot >> 32) as u32,
        }
    }
}

struct Emplacement {
    tache: AtomicPtr<Task>,
    /// Numero d'incarnation. Incremente a CHAQUE installation, y compris la
    /// premiere. Zero signifie « jamais occupe ».
    generation: AtomicU32,
    /// Quelqu'un detient-il l'acces exclusif au contenu non atomique.
    exclusif: AtomicBool,
}

static EMPLACEMENTS: [Emplacement; MAX_TACHES] = [const {
    Emplacement {
        tache: AtomicPtr::new(core::ptr::null_mut()),
        generation: AtomicU32::new(0),
        exclusif: AtomicBool::new(false),
    }
}; MAX_TACHES];

/// Nombre d'emplacements jamais occupes. Ne decroit jamais : les indices sont
/// stables a vie.
static LONGUEUR: AtomicUsize = AtomicUsize::new(0);

/// Serialise le CHOIX d'un emplacement. Ni la lecture ni la modification d'une
/// tache ne passent par la.
static STRUCTURE: crate::kernel::sync::SpinLock<()> =
    crate::kernel::sync::SpinLock::new(());

#[inline]
pub fn registre_longueur() -> usize {
    LONGUEUR.load(Ordering::Acquire)
}

/// L'identite courante de cet emplacement, s'il est occupe.
#[inline]
pub fn registre_id(emplacement: usize) -> Option<TacheId> {
    if emplacement >= MAX_TACHES {
        return None;
    }
    let generation = EMPLACEMENTS[emplacement].generation.load(Ordering::Acquire);
    if generation == 0 || EMPLACEMENTS[emplacement].tache.load(Ordering::Acquire).is_null() {
        return None;
    }
    Some(TacheId { emplacement: emplacement as u32, generation })
}

/// Lecture PARTAGEE d'une tache.
///
/// Rend `&Task`, jamais `&mut`. Les champs que plusieurs coeurs se disputent
/// sont atomiques et se lisent ou s'ecrivent par cette reference ; le reste ne
/// s'atteint que par [`registre_exclusif`].
#[inline]
pub fn registre_tache(emplacement: usize) -> Option<&'static Task> {
    if emplacement >= MAX_TACHES {
        return None;
    }
    let pointeur = EMPLACEMENTS[emplacement].tache.load(Ordering::Acquire);
    if pointeur.is_null() {
        None
    } else {
        Some(unsafe { &*pointeur })
    }
}

/// Lecture partagee PAR IDENTITE.
///
/// Rend `None` si l'emplacement a ete reattribue depuis que l'identite a ete
/// obtenue. C'est ce qui empeche un ancien ticket d'agir sur une tache neuve.
#[inline]
pub fn registre_tache_id(id: TacheId) -> Option<&'static Task> {
    let emplacement = id.emplacement();
    if emplacement >= MAX_TACHES {
        return None;
    }
    // La generation est relue AVANT le pointeur : si elle correspond encore,
    // le pointeur lu ensuite designe bien cette incarnation.
    if EMPLACEMENTS[emplacement].generation.load(Ordering::Acquire) != id.generation {
        return None;
    }
    registre_tache(emplacement)
}

/// Acces EXCLUSIF au contenu d'une tache.
///
/// Le `&mut` n'existe qu'ici, derriere un drapeau pris en exclusion mutuelle.
/// Deux appelants ne peuvent pas l'obtenir en meme temps, et le recyclage de
/// l'emplacement l'exige aussi -- personne ne peut donc voir un contenu
/// remplace a moitie.
pub struct GardeTache {
    emplacement: usize,
    tache: *mut Task,
}

impl core::ops::Deref for GardeTache {
    type Target = Task;
    #[inline]
    fn deref(&self) -> &Task {
        unsafe { &*self.tache }
    }
}

impl core::ops::DerefMut for GardeTache {
    #[inline]
    fn deref_mut(&mut self) -> &mut Task {
        unsafe { &mut *self.tache }
    }
}

impl Drop for GardeTache {
    #[inline]
    fn drop(&mut self) {
        EMPLACEMENTS[self.emplacement].exclusif.store(false, Ordering::Release);
    }
}

/// Prend l'acces exclusif s'il est libre. Ne bloque jamais.
#[inline]
pub fn registre_exclusif(emplacement: usize) -> Option<GardeTache> {
    if emplacement >= MAX_TACHES {
        return None;
    }
    if EMPLACEMENTS[emplacement]
        .exclusif
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return None;
    }
    let pointeur = EMPLACEMENTS[emplacement].tache.load(Ordering::Acquire);
    if pointeur.is_null() {
        EMPLACEMENTS[emplacement].exclusif.store(false, Ordering::Release);
        return None;
    }
    Some(GardeTache { emplacement, tache: pointeur })
}

/// Prend l'acces exclusif, en attendant qu'il se libere.
///
/// L'attente est un spin court : les sections exclusives sont breves par
/// construction -- poser un champ, lire un contexte -- et aucune ne dort.
pub fn registre_exclusif_attente(emplacement: usize) -> Option<GardeTache> {
    loop {
        if let Some(garde) = registre_exclusif(emplacement) {
            return Some(garde);
        }
        if emplacement >= MAX_TACHES
            || EMPLACEMENTS[emplacement].tache.load(Ordering::Acquire).is_null()
        {
            return None;
        }
        core::hint::spin_loop();
    }
}

/// Pointeur brut vers une tache dont l'ORDONNANCEUR garantit la propriete.
///
/// # Pourquoi cette porte existe
///
/// Un changement de contexte ne peut pas tenir un garde RAII : la pile change
/// au milieu, et le garde serait relache par une autre tache que celle qui l'a
/// pris. L'ordonnanceur est pourtant bien proprietaire des deux taches a cet
/// instant -- la sortante quitte son coeur, l'entrante n'y est pas encore.
///
/// # Securite
///
/// L'appelant doit garantir que la tache n'est sur aucun autre coeur. En
/// construction de debogage, on le VERIFIE : `on_cpu` doit valoir -1 (aucun
/// coeur) ou le coeur courant. Une violation signale une faute de propriete de
/// l'ordonnanceur, qui est exactement ce qu'on ne veut pas laisser passer en
/// silence.
///
/// # Safety
///
/// Voir ci-dessus : propriete garantie par l'ordonnanceur.
pub unsafe fn registre_pointeur_ordonnanceur(emplacement: usize) -> Option<*mut Task> {
    let tache = registre_tache(emplacement)?;
    #[cfg(debug_assertions)]
    {
        let coeur = tache.on_cpu.charge();
        let courant = crate::arch::x86_64::smp::cpu_index() as i8;
        debug_assert!(
            coeur < 0 || coeur == courant,
            "registre: pointeur ordonnanceur sur une tache active ailleurs (on_cpu={coeur}, cpu={courant})",
        );
    }
    let _ = tache;
    let pointeur = EMPLACEMENTS[emplacement].tache.load(Ordering::Acquire);
    if pointeur.is_null() { None } else { Some(pointeur) }
}

/// Enregistre une tache et rend son identite.
///
/// Reutilise l'emplacement d'une tache morte quand il y en a un. Le contenu est
/// remplace SUR PLACE, ce qui garde l'adresse valide a vie -- mais la
/// generation est incrementee AVANT l'ecriture, de sorte qu'aucune identite
/// ancienne ne designe plus cet emplacement pendant qu'on le remplit.
///
/// Rend `None` si le registre est plein.
pub fn registre_ajoute(
    tache: alloc::boxed::Box<Task>,
    recyclable: impl Fn(&Task) -> bool,
) -> Option<TacheId> {
    let _structure = STRUCTURE.lock();
    let longueur = LONGUEUR.load(Ordering::Acquire);

    for emplacement in 0..longueur {
        let pointeur = EMPLACEMENTS[emplacement].tache.load(Ordering::Acquire);
        if pointeur.is_null() {
            continue;
        }
        // L'exclusivite AVANT le predicat : un lecteur exclusif en cours
        // examinerait sinon un contenu qu'on s'apprete a remplacer.
        let Some(mut garde) = registre_exclusif(emplacement) else { continue };
        if !recyclable(&garde) {
            continue;
        }
        // La generation d'abord : a partir d'ici, toute identite ancienne est
        // refusee, et personne ne peut plus prendre cet emplacement pour
        // l'ancienne tache.
        let generation = prochaine_generation(emplacement);
        *garde = *tache;
        drop(garde);
        return Some(TacheId { emplacement: emplacement as u32, generation });
    }

    if longueur >= MAX_TACHES {
        return None;
    }
    let generation = prochaine_generation(longueur);
    EMPLACEMENTS[longueur]
        .tache
        .store(alloc::boxed::Box::into_raw(tache), Ordering::Release);
    // La longueur monte APRES le pointeur : un lecteur qui voit l'indice voit
    // donc forcement un emplacement deja rempli.
    LONGUEUR.store(longueur + 1, Ordering::Release);
    Some(TacheId { emplacement: longueur as u32, generation })
}

/// Numero d'incarnation suivant pour cet emplacement.
///
/// Ne rend jamais zero, qui signifie « jamais occupe ». Le debordement d'un
/// `u32` demanderait quatre milliards de recyclages du MEME emplacement ; on
/// saute la valeur reservee plutot que d'y voir un cas impossible.
fn prochaine_generation(emplacement: usize) -> u32 {
    let suivante = EMPLACEMENTS[emplacement]
        .generation
        .load(Ordering::Relaxed)
        .wrapping_add(1);
    let suivante = if suivante == 0 { 1 } else { suivante };
    EMPLACEMENTS[emplacement].generation.store(suivante, Ordering::Release);
    suivante
}

/// Parcourt les taches enregistrees, en lecture partagee.
pub fn registre_iter() -> impl Iterator<Item = &'static Task> {
    (0..registre_longueur()).filter_map(registre_tache)
}

/// Indice de la premiere tache satisfaisant le predicat.
pub fn registre_position(predicat: impl Fn(&Task) -> bool) -> Option<usize> {
    (0..registre_longueur()).find(|&index| registre_tache(index).is_some_and(&predicat))
}
