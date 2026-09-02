//! Le registre des taches peut-il rendre une reference perimee ?
//!
//! # Ce que la premiere version laissait passer
//!
//! Elle rendait `&'static mut Task` depuis la meme lecture partagee que
//! `&Task`, et recyclait un emplacement en ecrasant son contenu sur place.
//! Garder l'adresse reglait la duree de vie et rien d'autre : une adresse
//! stable n'est pas une identite stable. Un lecteur ayant obtenu une reference
//! quand l'emplacement portait la tache A pouvait la relire quand il portait
//! la tache B.
//!
//! Les consequences sont toutes silencieuses -- aucune ne fait echouer un test
//! qui ne les cherche pas :
//!
//!   * une file d'execution qui garde un indice ordonnance la tache suivante ;
//!   * un ticket d'attente reveille une incarnation qui n'a rien demande ;
//!   * deux coeurs mettent la meme tache deux fois en file.
//!
//! # Le modele
//!
//! `registre.rs` ne se compile pas sur l'hote : il touche le SMP, le verrou
//! noyau et le type `Task`. On rejoue donc ici sa STRUCTURE -- emplacements a
//! adresse stable, generation, rendez-vous lecteurs/ecrivain et drapeau
//! d'exclusivite -- avec les memes ordres memoire et les memes transitions.
//!
//! Un garde-fou source verifie separement que le vrai module garde ces
//! elements ; ce test verifie que ces elements suffisent.
//!
//! Lance par `tools/dev/validate-fast.ps1` et la barriere courte.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

const MAX_TACHES: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct TacheId {
    emplacement: u32,
    generation: u32,
}

impl TacheId {
    fn en_mot(self) -> u64 {
        ((self.generation as u64) << 32) | self.emplacement as u64
    }
    fn depuis_mot(mot: u64) -> Self {
        Self {
            emplacement: (mot & 0xffff_ffff) as u32,
            generation: (mot >> 32) as u32,
        }
    }
}

/// Le contenu d'une tache, reduit a ce que le registre doit proteger.
struct Tache {
    /// Etat d'ordonnancement : 0 pret, 1 bloque, 2 zombie.
    etat: AtomicU32,
    /// Identifiant de fil : sert a reconnaitre l'incarnation.
    tid: AtomicU32,
    /// Un champ NON atomique, que seul un acces exclusif peut toucher.
    prive: std::cell::UnsafeCell<u64>,
}

unsafe impl Sync for Tache {}

const PRET: u32 = 0;
const BLOQUE: u32 = 1;
const ZOMBIE: u32 = 2;

struct Emplacement {
    tache: std::sync::atomic::AtomicPtr<Tache>,
    generation: AtomicU32,
    exclusif: AtomicBool,
}

struct Registre {
    emplacements: Vec<Emplacement>,
    longueur: AtomicUsize,
    lecteurs: AtomicUsize,
    ecrivain: AtomicBool,
    /// Falsification : quand faux, le recyclage n'incremente plus la
    /// generation. Sert a prouver que les tests dependent bien d'elle.
    generations_actives: AtomicBool,
}

struct Garde<'a> {
    registre: &'a Registre,
    _lecture: Lecture<'a>,
    emplacement: usize,
    tache: *mut Tache,
}

struct Lecture<'a> {
    registre: &'a Registre,
}

impl Drop for Lecture<'_> {
    fn drop(&mut self) {
        self.registre.lecteurs.fetch_sub(1, Ordering::Release);
    }
}

struct LectureTache<'a> {
    _lecture: Lecture<'a>,
    tache: *const Tache,
}

impl std::ops::Deref for LectureTache<'_> {
    type Target = Tache;
    fn deref(&self) -> &Tache { unsafe { &*self.tache } }
}

struct Ecriture<'a> {
    registre: &'a Registre,
}

impl Drop for Ecriture<'_> {
    fn drop(&mut self) {
        self.registre.ecrivain.store(false, Ordering::Release);
    }
}

impl Garde<'_> {
    fn prive(&mut self) -> &mut u64 {
        unsafe { &mut *(*self.tache).prive.get() }
    }
    fn tache(&self) -> &Tache {
        unsafe { &*self.tache }
    }
}

impl Drop for Garde<'_> {
    fn drop(&mut self) {
        self.registre.emplacements[self.emplacement]
            .exclusif
            .store(false, Ordering::Release);
    }
}

impl Registre {
    fn neuf() -> Self {
        Self {
            emplacements: (0..MAX_TACHES)
                .map(|_| Emplacement {
                    tache: std::sync::atomic::AtomicPtr::new(core::ptr::null_mut()),
                    generation: AtomicU32::new(0),
                    exclusif: AtomicBool::new(false),
                })
                .collect(),
            longueur: AtomicUsize::new(0),
            lecteurs: AtomicUsize::new(0),
            ecrivain: AtomicBool::new(false),
            generations_actives: AtomicBool::new(true),
        }
    }

    fn lecture(&self) -> Lecture<'_> {
        loop {
            while self.ecrivain.load(Ordering::Acquire) { std::hint::spin_loop(); }
            self.lecteurs.fetch_add(1, Ordering::AcqRel);
            if !self.ecrivain.load(Ordering::Acquire) {
                return Lecture { registre: self };
            }
            self.lecteurs.fetch_sub(1, Ordering::Release);
        }
    }

    fn ecriture(&self) -> Ecriture<'_> {
        while self.ecrivain
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            std::hint::spin_loop();
        }
        while self.lecteurs.load(Ordering::Acquire) != 0 { std::hint::spin_loop(); }
        Ecriture { registre: self }
    }

    fn longueur(&self) -> usize {
        self.longueur.load(Ordering::Acquire)
    }

    fn tache(&self, emplacement: usize) -> Option<LectureTache<'_>> {
        let lecture = self.lecture();
        if emplacement >= MAX_TACHES {
            return None;
        }
        let p = self.emplacements[emplacement].tache.load(Ordering::Acquire);
        if p.is_null() { None } else { Some(LectureTache { _lecture: lecture, tache: p }) }
    }

    /// Lecture PAR IDENTITE : refuse une incarnation perimee.
    fn tache_id(&self, id: TacheId) -> Option<LectureTache<'_>> {
        let lecture = self.lecture();
        let emplacement = id.emplacement as usize;
        if emplacement >= MAX_TACHES {
            return None;
        }
        if self.emplacements[emplacement].generation.load(Ordering::Acquire) != id.generation {
            return None;
        }
        let p = self.emplacements[emplacement].tache.load(Ordering::Acquire);
        if p.is_null() { None } else { Some(LectureTache { _lecture: lecture, tache: p }) }
    }

    fn id(&self, emplacement: usize) -> Option<TacheId> {
        let generation = self.emplacements[emplacement].generation.load(Ordering::Acquire);
        if generation == 0 {
            return None;
        }
        Some(TacheId { emplacement: emplacement as u32, generation })
    }

    fn exclusif(&self, emplacement: usize) -> Option<Garde<'_>> {
        let lecture = self.lecture();
        if emplacement >= MAX_TACHES {
            return None;
        }
        if self.emplacements[emplacement]
            .exclusif
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return None;
        }
        let p = self.emplacements[emplacement].tache.load(Ordering::Acquire);
        if p.is_null() {
            self.emplacements[emplacement].exclusif.store(false, Ordering::Release);
            return None;
        }
        Some(Garde { registre: self, _lecture: lecture, emplacement, tache: p })
    }

    fn prochaine_generation(&self, emplacement: usize) -> u32 {
        if !self.generations_actives.load(Ordering::Relaxed) {
            // Falsification : la premiere installation numerote, mais le
            // RECYCLAGE ne renumerote plus. C'est exactement la version qui
            // laissait passer l'ABA.
            let actuelle = self.emplacements[emplacement].generation.load(Ordering::Relaxed);
            if actuelle != 0 {
                return actuelle;
            }
            self.emplacements[emplacement].generation.store(1, Ordering::Release);
            return 1;
        }
        let suivante = self.emplacements[emplacement]
            .generation
            .load(Ordering::Relaxed)
            .wrapping_add(1);
        let suivante = if suivante == 0 { 1 } else { suivante };
        self.emplacements[emplacement].generation.store(suivante, Ordering::Release);
        suivante
    }

    fn ajoute(&self, tid: u32) -> Option<TacheId> {
        let _ecriture = self.ecriture();
        let longueur = self.longueur();
        for emplacement in 0..longueur {
            let p = self.emplacements[emplacement].tache.load(Ordering::Acquire);
            if p.is_null() { continue; }
            let tache = unsafe { &mut *p };
            if tache.etat.load(Ordering::Acquire) != ZOMBIE {
                continue;
            }
            let generation = self.prochaine_generation(emplacement);
            tache.etat.store(PRET, Ordering::Release);
            tache.tid.store(tid, Ordering::Release);
            unsafe { *tache.prive.get() = 0; }
            return Some(TacheId { emplacement: emplacement as u32, generation });
        }
        if longueur >= MAX_TACHES {
            return None;
        }
        let generation = self.prochaine_generation(longueur);
        let tache = Box::into_raw(Box::new(Tache {
            etat: AtomicU32::new(PRET),
            tid: AtomicU32::new(tid),
            prive: std::cell::UnsafeCell::new(0),
        }));
        self.emplacements[longueur].tache.store(tache, Ordering::Release);
        self.longueur.store(longueur + 1, Ordering::Release);
        Some(TacheId { emplacement: longueur as u32, generation })
    }

    /// Reveille par identite : exactement un gagnant, et jamais une autre
    /// incarnation.
    fn reveille(&self, id: TacheId) -> bool {
        let Some(tache) = self.tache_id(id) else { return false };
        tache
            .etat
            .compare_exchange(BLOQUE, PRET, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
}

// ---------------------------------------------------------------------------
// A. Lecture concurrente + modification d'etat
// ---------------------------------------------------------------------------

#[test]
fn a_lecture_concurrente_et_modification_d_etat() {
    let registre = Arc::new(Registre::neuf());
    let id = registre.ajoute(1).unwrap();

    let mut fils = Vec::new();
    for _ in 0..4 {
        let registre = Arc::clone(&registre);
        fils.push(std::thread::spawn(move || {
            for _ in 0..5_000 {
                let tache = registre.tache_id(id).expect("l'incarnation est vivante");
                let etat = tache.etat.load(Ordering::Acquire);
                assert!(etat <= ZOMBIE, "etat corrompu : {etat}");
                assert_eq!(tache.tid.load(Ordering::Acquire), 1);
                tache.etat.store(BLOQUE, Ordering::Release);
                tache.etat.store(PRET, Ordering::Release);
            }
        }));
    }
    for f in fils { f.join().unwrap(); }
    assert_eq!(registre.tache_id(id).unwrap().tid.load(Ordering::Acquire), 1);
}

// ---------------------------------------------------------------------------
// B / F / G. Reveil concurrent : exactement un gagnant, aucun double enqueue
// ---------------------------------------------------------------------------

#[test]
fn b_f_g_deux_coeurs_ne_reveillent_qu_une_fois() {
    for _ in 0..2_000 {
        let registre = Arc::new(Registre::neuf());
        let id = registre.ajoute(7).unwrap();
        registre.tache_id(id).unwrap().etat.store(BLOQUE, Ordering::Release);

        let gagnants = Arc::new(AtomicU64::new(0));
        let mut fils = Vec::new();
        for _ in 0..4 {
            let registre = Arc::clone(&registre);
            let gagnants = Arc::clone(&gagnants);
            fils.push(std::thread::spawn(move || {
                if registre.reveille(id) {
                    // Un seul reveilleur a le droit de mettre en file :
                    // c'est ce qui interdit le double enqueue.
                    gagnants.fetch_add(1, Ordering::Relaxed);
                }
            }));
        }
        for f in fils { f.join().unwrap(); }
        assert_eq!(
            gagnants.load(Ordering::Relaxed), 1,
            "exactement un coeur doit gagner la transition bloque -> pret",
        );
    }
}

// ---------------------------------------------------------------------------
// C / E. Ancien handle apres sortie et recyclage
// ---------------------------------------------------------------------------

#[test]
fn c_e_un_ancien_handle_ne_designe_plus_la_nouvelle_incarnation() {
    let registre = Registre::neuf();
    let ancien = registre.ajoute(11).unwrap();
    assert!(registre.tache_id(ancien).is_some());

    // La tache sort.
    registre.tache_id(ancien).unwrap().etat.store(ZOMBIE, Ordering::Release);

    // L'emplacement est recycle par une AUTRE tache.
    let neuf = registre.ajoute(22).unwrap();
    assert_eq!(
        neuf.emplacement, ancien.emplacement,
        "le test n'a de sens que si l'emplacement est bien reutilise",
    );
    assert_ne!(neuf.generation, ancien.generation);

    assert!(
        registre.tache_id(ancien).is_none(),
        "l'ancien handle designe encore l'emplacement : c'est l'ABA",
    );
    assert_eq!(registre.tache_id(neuf).unwrap().tid.load(Ordering::Acquire), 22);
}

// ---------------------------------------------------------------------------
// D / H. Un ancien reveil ne touche pas la nouvelle incarnation
// ---------------------------------------------------------------------------

#[test]
fn d_h_un_ancien_reveil_n_atteint_pas_la_nouvelle_incarnation() {
    let registre = Registre::neuf();
    let ancien = registre.ajoute(31).unwrap();
    registre.tache_id(ancien).unwrap().etat.store(ZOMBIE, Ordering::Release);

    let neuf = registre.ajoute(32).unwrap();
    // La nouvelle incarnation se bloque, comme le ferait n'importe quelle
    // tache neuve entrant dans une file d'attente.
    registre.tache_id(neuf).unwrap().etat.store(BLOQUE, Ordering::Release);

    assert!(
        !registre.reveille(ancien),
        "un reveil vise par l'ancien ticket a atteint la nouvelle tache",
    );
    assert_eq!(
        registre.tache_id(neuf).unwrap().etat.load(Ordering::Acquire), BLOQUE,
        "la nouvelle incarnation a ete reveillee par un ticket qui ne la visait pas",
    );
    assert!(registre.reveille(neuf), "son propre ticket doit fonctionner");
}

// ---------------------------------------------------------------------------
// Files d'execution : une entree perimee ne se sert pas
// ---------------------------------------------------------------------------

#[test]
fn une_entree_de_file_perimee_est_rejetee() {
    // C'est le cas qui produisait une « tache fantome » prenant des quantums :
    // la file gardait un indice, l'emplacement etait recycle, et le
    // consommateur servait la tache suivante.
    let registre = Registre::neuf();
    let ancien = registre.ajoute(41).unwrap();
    let file = vec![ancien.en_mot()];

    registre.tache_id(ancien).unwrap().etat.store(ZOMBIE, Ordering::Release);
    let neuf = registre.ajoute(42).unwrap();

    let servies: Vec<u32> = file
        .iter()
        .filter_map(|mot| registre.tache_id(TacheId::depuis_mot(*mot)))
        .map(|t| t.tid.load(Ordering::Acquire))
        .collect();
    assert!(servies.is_empty(), "une entree perimee a ete servie : {servies:?}");
    let _ = neuf;
}

#[test]
fn l_empaquetage_d_une_identite_est_bijectif() {
    for emplacement in 0..MAX_TACHES as u32 {
        for generation in [1u32, 2, 0xffff, 0xffff_ffff] {
            let id = TacheId { emplacement, generation };
            assert_eq!(TacheId::depuis_mot(id.en_mot()), id);
        }
    }
}

// ---------------------------------------------------------------------------
// Exclusivite : le contenu non atomique n'a jamais deux ecrivains
// ---------------------------------------------------------------------------

#[test]
fn l_acces_exclusif_est_reellement_exclusif() {
    let registre = Registre::neuf();
    let id = registre.ajoute(51).unwrap();
    let emplacement = id.emplacement as usize;

    let premier = registre.exclusif(emplacement).expect("libre au depart");
    assert!(
        registre.exclusif(emplacement).is_none(),
        "deux gardes exclusifs coexistent : le `&mut` ne prouve plus rien",
    );
    drop(premier);
    assert!(registre.exclusif(emplacement).is_some(), "rendu au Drop");
}

#[test]
fn le_champ_prive_ne_perd_aucune_ecriture() {
    // Sans exclusion, deux fils faisant « lire, ajouter, ecrire » sur un champ
    // NON atomique en perdent. Le garde doit les serialiser.
    let registre = Arc::new(Registre::neuf());
    let id = registre.ajoute(61).unwrap();
    let emplacement = id.emplacement as usize;

    let mut fils = Vec::new();
    for _ in 0..4 {
        let registre = Arc::clone(&registre);
        fils.push(std::thread::spawn(move || {
            for _ in 0..2_000 {
                loop {
                    if let Some(mut garde) = registre.exclusif(emplacement) {
                        *garde.prive() += 1;
                        break;
                    }
                    std::hint::spin_loop();
                }
            }
        }));
    }
    for f in fils { f.join().unwrap(); }
    let garde = registre.exclusif(emplacement).unwrap();
    let valeur = unsafe { *(*garde.tache).prive.get() };
    assert_eq!(valeur, 8_000, "des ecritures ont ete perdues");
}

#[test]
fn le_recyclage_attend_tous_les_lecteurs() {
    // La generation ne peut rien pour une reference DEJA rendue. L'ecrivain
    // doit rester bloque jusqu'au Drop du lecteur, puis seulement remplacer.
    let registre = Arc::new(Registre::neuf());
    let id = registre.ajoute(71).unwrap();
    let emplacement = id.emplacement as usize;
    registre.tache_id(id).unwrap().etat.store(ZOMBIE, Ordering::Release);

    let lecture = registre.tache_id(id).unwrap();
    let fini = Arc::new(AtomicBool::new(false));
    let registre_fils = Arc::clone(&registre);
    let fini_fils = Arc::clone(&fini);
    let fils = std::thread::spawn(move || {
        let autre = registre_fils.ajoute(72).unwrap();
        fini_fils.store(true, Ordering::Release);
        autre
    });

    for _ in 0..1_000_000 {
        if registre.ecrivain.load(Ordering::Acquire) { break; }
        std::hint::spin_loop();
    }
    assert!(registre.ecrivain.load(Ordering::Acquire), "le recycleur ne s'est pas publie");
    assert!(!fini.load(Ordering::Acquire), "le recycleur a depasse un lecteur vivant");
    assert_eq!(lecture.tid.load(Ordering::Acquire), 71,
        "une reference existante a vu l'incarnation suivante");
    drop(lecture);

    let autre = fils.join().unwrap();
    assert_eq!(autre.emplacement as usize, emplacement);
    assert_ne!(autre.generation, id.generation);
}

// ---------------------------------------------------------------------------
// I / J. Epuisement propre, et reutilisation sans reference pendante
// ---------------------------------------------------------------------------

#[test]
fn i_l_epuisement_est_propre() {
    let registre = Registre::neuf();
    for numero in 0..MAX_TACHES as u32 {
        assert!(registre.ajoute(numero).is_some(), "place {numero} refusee a tort");
    }
    assert!(
        registre.ajoute(999).is_none(),
        "le registre plein doit REFUSER, pas deborder ni ecraser",
    );
    assert_eq!(registre.longueur(), MAX_TACHES);
    // Et il redevient utilisable des qu'une place se libere.
    registre.tache(0).unwrap().etat.store(ZOMBIE, Ordering::Release);
    let recyclee = registre.ajoute(999).expect("une place s'est liberee");
    assert_eq!(recyclee.emplacement, 0);
}

#[test]
fn j_la_reutilisation_ne_laisse_aucune_reference_pendante() {
    // L'adresse de l'allocation ne change pas, mais le contenu n'est remplace
    // qu'apres le Drop de tous les lecteurs. La generation, elle, garantit
    // qu'un ancien handle n'est pas pris pour l'incarnation suivante.
    let registre = Registre::neuf();
    let premier = registre.ajoute(81).unwrap();
    let adresse_premiere = {
        let lecture = registre.tache(0).unwrap();
        let adresse = &*lecture as *const Tache;
        drop(lecture);
        adresse
    };

    for tour in 0..50u32 {
        let identite = registre.id(0).unwrap();
        registre.tache_id(identite).unwrap().etat.store(ZOMBIE, Ordering::Release);
        let suivante = registre.ajoute(100 + tour).unwrap();
        assert_eq!(suivante.emplacement, 0);
        let adresse_courante = {
            let lecture = registre.tache(0).unwrap();
            let adresse = &*lecture as *const Tache;
            drop(lecture);
            adresse
        };
        assert_eq!(
            adresse_courante, adresse_premiere,
            "l'adresse de l'emplacement a change : une reference pendrait",
        );
        assert!(
            registre.tache_id(premier).is_none() || tour == 0 && false,
            "l'identite d'origine survit a {} recyclages", tour + 1,
        );
    }
}

// ---------------------------------------------------------------------------
// La falsification : sans generation, les protections tombent
// ---------------------------------------------------------------------------

#[test]
fn sans_generation_l_aba_reapparait() {
    // Ce test prouve que les precedents dependent REELLEMENT de la generation,
    // et ne passent pas pour une autre raison.
    let registre = Registre::neuf();
    registre.generations_actives.store(false, Ordering::Relaxed);

    let ancien = registre.ajoute(91).unwrap();
    registre.tache_id(ancien).unwrap().etat.store(ZOMBIE, Ordering::Release);
    let neuf = registre.ajoute(92).unwrap();
    assert_eq!(neuf.emplacement, ancien.emplacement);

    // Sans incrementation, l'ancien handle designe encore l'emplacement...
    let vue = registre.tache_id(ancien).expect("la protection est desactivee");
    assert_eq!(
        vue.tid.load(Ordering::Acquire), 92,
        "l'ancien handle lit la NOUVELLE tache : c'est exactement l'ABA que la \
         generation ferme",
    );

    // ... et un ancien reveil atteint la nouvelle incarnation.
    vue.etat.store(BLOQUE, Ordering::Release);
    assert!(
        registre.reveille(ancien),
        "un ancien ticket a reveille une tache qui ne l'attendait pas",
    );
}
