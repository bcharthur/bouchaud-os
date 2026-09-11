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
//     generation ; un ancien handle est donc refuse apres reattribution ;
//   * une generation ne peut pas invalider une reference Rust deja rendue.
//     Toute lecture conserve donc un garde, et le recycleur attend la
//     quiescence de tous les lecteurs avant d'ecraser le contenu ;
//   * la lecture partagee ne rend QUE `&Task` sous ce garde. Les champs
//     disputes sont atomiques ; le reste ne s'atteint plus par cette voie ;
//   * l'acces exclusif au contenu non atomique passe par un DRAPEAU par
//     emplacement, pris en exclusion mutuelle. Le `&mut` n'existe que derriere
//     ce drapeau, et il est alors reellement exclusif ;
//   * le recyclage prend le rendez-vous d'ecriture, attend lecteurs ET gardes
//     exclusifs, puis incremente la generation AVANT d'ecrire. Aucun lecteur
//     ne peut donc observer un contenu remplace sous sa reference.
//
// # Ce que cette version n'essaie pas de faire
//
// Elle ne separe pas physiquement les champs atomiques des autres dans le type
// `Task`. Ce serait plus fort, et cela demanderait de reecrire les quatre-vingt
// dix sites qui manipulent une tache. Le rendez-vous de quiescence et le
// drapeau d'exclusivite rendent la transition verifiable sans ce grand saut.

use core::sync::atomic::AtomicPtr;

/// Nombre maximal de taches simultanement enregistrees.
pub const MAX_TACHES: usize = 1024;

// Les bitmaps des runqueues indexent les EMPLACEMENTS de ce registre. Un
// emplacement au-dela de leur couverture ne serait jamais mis en file : la
// tache serait prete et ne s'executerait jamais. Le desaccord se voit ici, a la
// compilation, et pas au bout de mille processus.
const _: () = assert!(
    MAX_TACHES == crate::kernel::scheduler::runqueue::EMPLACEMENTS,
    "runqueue::EMPLACEMENTS doit couvrir exactement MAX_TACHES",
);

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

// Une generation invalide un HANDLE, pas une reference Rust deja obtenue.
// Ce rendez-vous empeche donc le recycleur d'ecraser une `Task` tant qu'une
// reference partagee ou exclusive existe encore.
static LECTEURS: AtomicUsize = AtomicUsize::new(0);
static ECRIVAIN: AtomicBool = AtomicBool::new(false);

/// PROFONDEUR DE LECTURE PAR COEUR.
///
/// # L'interblocage que ce compteur ferme
///
/// Le rendez-vous de quiescence est un verrou lecteur/ecrivain a PRIORITE
/// ECRIVAIN : l'ecrivain publie son drapeau, puis attend que le compte de
/// lecteurs retombe a zero ; un lecteur qui voit le drapeau attend, sans se
/// compter.
///
/// Cette discipline est correcte tant qu'un lecteur n'en prend qu'un a la
/// fois. Elle se referme sur elle-meme des qu'un lecteur en prend un SECOND
/// alors qu'il tient deja le premier : son propre garde exterieur maintient le
/// compte a un, l'ecrivain ne repart donc jamais, et le garde interieur attend
/// un drapeau qui ne tombera plus. Les deux coeurs s'immobilisent, et rien ne
/// le dit -- la machine se tait, simplement.
///
/// Ce n'etait pas une hypothese. Sur la sonde `nvme-parallele`, le troisieme
/// passage recyclait l'emplacement du fil de montage, qui venait de mourir :
/// `registre_ajoute` prenait le rendez-vous d'ecriture pendant que les autres
/// coeurs etaient dans l'ordonnanceur. Les registres pris au moniteur QEMU au
/// moment du gel le montraient sans ambiguite -- coeur 0 dans
/// `RegistreEcriture::acquire`, IRQ masquees ; coeurs 1, 2 et 3 dans
/// `RegistreLecture::acquire`, tous les trois.
///
/// Et l'imbrication n'est pas une maladresse isolee : `wake_sleepers` tient
/// une vue du registre et appelle `publish_ready`, qui en reprend une. La
/// fermer site par site serait a refaire a chaque nouvel appelant. Elle se
/// ferme donc ICI, dans la structure.
///
/// # Pourquoi c'est sur
///
/// Un garde imbrique ne retouche pas le compte global : le coeur y figure deja
/// pour un. L'exclusion vis-a-vis de l'ecrivain est donc intacte -- il attend
/// toujours zero, et zero veut toujours dire « plus aucune reference ».
///
/// Et sauter l'attente du drapeau est legitime au niveau imbrique : si ce coeur
/// tient un garde, le compte n'est pas nul, donc aucun ecrivain n'est ENTRE
/// dans sa section critique. Il ne peut qu'attendre -- et c'est precisement ce
/// qu'on le laisse faire, jusqu'a ce que le dernier garde de ce coeur tombe.
///
/// # Ce que ce compteur suppose
///
/// Qu'un garde de lecture ne traverse pas un changement de contexte. C'est le
/// cas : le noyau n'est pas preemptible en son sein -- `preempt_from_irq` ne
/// commute que si le timer a interrompu du code ring 3 --, et les points de
/// commutation volontaires n'en tiennent aucun. `RegistreEcriture::acquire`
/// verifie d'ailleurs qu'aucun garde local n'est tenu : un appelant qui
/// recyclerait un emplacement en tenant une lecture s'interbloquerait avec
/// lui-meme, et il vaut mieux qu'il l'apprenne par un panic que par un silence.

/// # Ce que cela coute, dit franchement
///
/// Chaque prise et chaque relachement ajoutent un masquage d'interruptions et
/// une lecture de l'identite du coeur, laquelle passe par `GS_BASE`. Sur le
/// chemin d'election, qui prend plusieurs gardes par commutation, c'est un
/// cout reel et non nul.
///
/// Il est paye volontiers : la version sans lui immobilisait quatre coeurs une
/// fois sur deux, et une machine qui se tait n'a pas de latence a defendre.
static PROFONDEUR_LECTURE: [AtomicUsize; MAX_CPUS] =
    [const { AtomicUsize::new(0) }; MAX_CPUS];

/// Profondeur de lecture tenue par le coeur courant.
#[inline]
fn profondeur_lecture_locale() -> usize {
    PROFONDEUR_LECTURE[local_cpu()].load(Ordering::Relaxed)
}

struct RegistreLecture;

impl RegistreLecture {
    #[inline]
    fn acquire() -> Self {
        loop {
            // LES DEUX COMPTES BOUGENT ENSEMBLE, IRQ MASQUEES.
            //
            // Sans ce masquage, un handler tombant entre l'incrementation
            // globale et la pose de la profondeur locale se croirait au
            // premier niveau et reprendrait le protocole complet : il
            // attendrait le drapeau que le compte de l'interrompu retient.
            // C'est le meme interblocage, reduit a quelques instructions.
            //
            // L'ATTENTE, elle, reste hors du masquage : la boucle rend la main
            // aux interruptions a chaque tour. Un rendez-vous d'ecriture dure
            // le temps d'un remplacement de tache, mais il n'y a aucune raison
            // de l'attendre IRQ fermees.
            let irq = interrupts::are_enabled();
            interrupts::disable();
            let cpu = local_cpu();
            let profondeur = PROFONDEUR_LECTURE[cpu].load(Ordering::Relaxed);
            let pris = if profondeur > 0 {
                PROFONDEUR_LECTURE[cpu].store(profondeur + 1, Ordering::Relaxed);
                true
            } else if ECRIVAIN.load(Ordering::Acquire) {
                false
            } else {
                LECTEURS.fetch_add(1, Ordering::AcqRel);
                if ECRIVAIN.load(Ordering::Acquire) {
                    LECTEURS.fetch_sub(1, Ordering::Release);
                    false
                } else {
                    PROFONDEUR_LECTURE[cpu].store(1, Ordering::Relaxed);
                    true
                }
            };
            if irq {
                interrupts::enable();
            }
            if pris {
                return Self;
            }
            core::hint::spin_loop();
        }
    }
}

impl Drop for RegistreLecture {
    #[inline]
    fn drop(&mut self) {
        let irq = interrupts::are_enabled();
        interrupts::disable();
        let cpu = local_cpu();
        let profondeur = PROFONDEUR_LECTURE[cpu].load(Ordering::Relaxed);
        debug_assert!(
            profondeur > 0,
            "registre: relachement d'une lecture que ce coeur ne tient pas",
        );
        PROFONDEUR_LECTURE[cpu].store(profondeur.saturating_sub(1), Ordering::Relaxed);
        // Le compte global ne retombe qu'au DERNIER garde du coeur : c'est lui
        // qui rend la quiescence a l'ecrivain.
        if profondeur <= 1 {
            LECTEURS.fetch_sub(1, Ordering::Release);
        }
        if irq {
            interrupts::enable();
        }
    }
}

struct RegistreEcriture {
    restaure_irq: bool,
}

impl RegistreEcriture {
    fn acquire() -> Self {
        // UN RECYCLEUR QUI TIENT UNE LECTURE S'ATTEND LUI-MEME.
        //
        // L'attente ci-dessous porte sur le compte GLOBAL des lecteurs, celui
        // de ce coeur compris. Prendre le rendez-vous d'ecriture en tenant un
        // garde de lecture est donc un interblocage certain -- et muet, faute
        // de quoi que ce soit pour le dire. Il se dit ici.
        debug_assert_eq!(
            profondeur_lecture_locale(),
            0,
            "registre: rendez-vous d'ecriture pris en tenant une lecture",
        );
        // Masquer AVANT de publier l'ecrivain : un handler local ne peut pas
        // interrompre le recycleur puis attendre ce meme recycleur.
        let restaure_irq = interrupts::are_enabled();
        interrupts::disable();
        while ECRIVAIN
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            core::hint::spin_loop();
        }
        while LECTEURS.load(Ordering::Acquire) != 0 {
            core::hint::spin_loop();
        }
        Self { restaure_irq }
    }
}

impl Drop for RegistreEcriture {
    fn drop(&mut self) {
        ECRIVAIN.store(false, Ordering::Release);
        if self.restaure_irq {
            interrupts::enable();
        }
    }
}

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

#[inline]
fn registre_tache_sous_lecture<'a>(
    _lecture: &'a RegistreLecture,
    emplacement: usize,
) -> Option<&'a Task> {
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

/// Reference partagee dont le garde interdit le recyclage de son allocation.
pub struct GardeLectureTache {
    lecture: RegistreLecture,
    tache: *const Task,
}

impl core::ops::Deref for GardeLectureTache {
    type Target = Task;

    #[inline]
    fn deref(&self) -> &Task {
        let _ = &self.lecture;
        unsafe { &*self.tache }
    }
}

/// Lecture partagee d'une tache. La reference ne peut pas survivre a son garde.
#[inline]
pub fn registre_tache(emplacement: usize) -> Option<GardeLectureTache> {
    let lecture = RegistreLecture::acquire();
    let tache = registre_tache_sous_lecture(&lecture, emplacement)? as *const Task;
    Some(GardeLectureTache { lecture, tache })
}

/// Lecture partagee PAR IDENTITE.
///
/// Rend `None` si l'emplacement a ete reattribue depuis que l'identite a ete
/// obtenue. C'est ce qui empeche un ancien ticket d'agir sur une tache neuve.
#[inline]
pub fn registre_tache_id(id: TacheId) -> Option<GardeLectureTache> {
    let emplacement = id.emplacement();
    if emplacement >= MAX_TACHES {
        return None;
    }
    let lecture = RegistreLecture::acquire();
    // La generation et le pointeur restent la meme incarnation jusqu'au Drop.
    if EMPLACEMENTS[emplacement].generation.load(Ordering::Acquire) != id.generation {
        return None;
    }
    let tache = registre_tache_sous_lecture(&lecture, emplacement)? as *const Task;
    Some(GardeLectureTache { lecture, tache })
}

/// Acces EXCLUSIF au contenu d'une tache.
///
/// Le `&mut` n'existe qu'ici, derriere un drapeau pris en exclusion mutuelle.
/// Deux appelants ne peuvent pas l'obtenir en meme temps. Son garde de lecture
/// fait en plus attendre le recycleur jusqu'a la fin de cet acces exclusif.
pub struct GardeTache {
    lecture: RegistreLecture,
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
    let lecture = RegistreLecture::acquire();
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
    Some(GardeTache { lecture, emplacement, tache: pointeur })
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
    let lecture = RegistreLecture::acquire();
    let tache = registre_tache_sous_lecture(&lecture, emplacement)?;
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
/// remplace SUR PLACE seulement apres la quiescence de toutes les references.
/// La generation est ensuite incrementee AVANT l'ecriture, de sorte qu'aucune
/// identite ancienne ne designe l'incarnation installee.
///
/// Rend `None` si le registre est plein.
pub fn registre_ajoute(
    tache: alloc::boxed::Box<Task>,
    recyclable: impl Fn(&Task) -> bool,
) -> Option<TacheId> {
    let _structure = STRUCTURE.lock();
    let _ecriture = RegistreEcriture::acquire();
    let longueur = LONGUEUR.load(Ordering::Acquire);

    for emplacement in 0..longueur {
        let pointeur = EMPLACEMENTS[emplacement].tache.load(Ordering::Acquire);
        if pointeur.is_null() {
            continue;
        }
        // `_ecriture` a attendu tous les gardes. Il n'existe donc plus aucune
        // reference partagee ou exclusive vers le contenu remplace.
        let ancienne = unsafe { &mut *pointeur };
        if !recyclable(ancienne) {
            continue;
        }
        // La generation d'abord : a partir d'ici, toute identite ancienne est
        // refusee, et personne ne peut plus prendre cet emplacement pour
        // l'ancienne tache.
        let generation = prochaine_generation(emplacement);
        *ancienne = *tache;
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
