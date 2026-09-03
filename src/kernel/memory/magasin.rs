//! Le depot de magasins : ce qui evite au tas per-CPU de retomber sur le
//! backing global a chaque debordement.
//!
//! # Ce que les caches per-CPU ne reglaient pas
//!
//! Le tas NG garde par CPU une liste libre bornee (`MAX_CACHED_PER_CLASS`) par
//! classe de taille. Tant qu'on reste dedans, allouer et liberer ne prennent
//! aucun verrou global. Mais aux DEUX bords, on retombe sur le backing :
//!
//!   * liste vide  -> une allocation backing, verrou global, PAR OBJET ;
//!   * liste pleine -> une liberation backing, verrou global, PAR OBJET.
//!
//! Un fil qui oscille autour du plafond -- exactement ce que fait un
//! compositeur qui alloue et rend un tampon par trame, ou un analyseur HTML qui
//! construit et jette des noeuds -- paie donc le verrou global a CHAQUE
//! operation. Le cache ne l'a pas supprime : il l'a deplace.
//!
//! # Le depot
//!
//! Entre le cache per-CPU et le backing s'intercale un DEPOT par classe : une
//! pile de MAGASINS, chacun une chaine de `LOT` blocs deja decoupes.
//!
//!   * liste vide  -> on prend un magasin entier, UN verrou, `LOT` objets ;
//!   * liste pleine -> on rend un magasin entier, UN verrou, `LOT` objets.
//!
//! Le backing global n'est atteint que lorsque le depot est vide (premiere
//! chauffe) ou plein (memoire reellement rendue). Le trafic de verrou global
//! est divise par `LOT` sur le regime etabli, et c'est ce que mesurent
//! `servis`/`deposes` face a `vides`/`pleins`.
//!
//! # Ce que ce module ne fait PAS
//!
//! Il ne connait ni l'allocateur backing, ni le CPU courant, ni les
//! interruptions. Les blocs sont des ADRESSES ; leur premier mot sert de lien,
//! exactement comme dans la liste per-CPU. C'est ce qui permet de le mettre a
//! l'epreuve sur l'hote, sur un tampon ordinaire, sans noyau.
//!
//! # Interruptions
//!
//! Le verrou du depot est un verrou tournant simple. Il est correct parce que
//! l'appelant masque les interruptions -- le tas est atteignable depuis un
//! gestionnaire d'interruption, et une reprise sur le meme CPU serait un
//! interblocage. La regle appartient a l'appelant : ce module ne peut pas la
//! faire respecter sans dependre de l'architecture, et le noyau la tient dans
//! `heap.rs`.

use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

/// Objets par magasin. Le trafic vers le backing global est divise par ce
/// nombre sur le regime etabli.
pub const LOT: usize = 16;

/// Magasins gardes par classe. Au-dela, la memoire est reellement rendue au
/// backing : un depot sans plafond serait une fuite deguisee en cache.
pub const MAGASINS_MAX: usize = 8;

/// Le lien vers le bloc suivant vit dans le PREMIER MOT du bloc.
///
/// C'est la meme convention que la liste libre per-CPU, et c'est ce qui rend
/// un magasin gratuit : il n'y a aucune structure a allouer pour decrire une
/// chaine de blocs libres.
///
/// # Securite
/// `bloc` doit etre une adresse de bloc LIBRE, alignee sur `usize`, dont la
/// taille de classe est au moins `size_of::<usize>()`.
#[inline]
pub unsafe fn lien_lit(bloc: usize) -> usize {
    *(bloc as *const usize)
}

/// # Securite
/// Voir [`lien_lit`].
#[inline]
pub unsafe fn lien_ecrit(bloc: usize, suivant: usize) {
    *(bloc as *mut usize) = suivant;
}

/// Une chaine de blocs libres, et sa longueur.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Magasin {
    pub tete: usize,
    pub compte: usize,
}

/// Le depot d'une classe de taille.
pub struct Depot {
    verrou: AtomicBool,
    tetes: [AtomicUsize; MAGASINS_MAX],
    comptes: [AtomicUsize; MAGASINS_MAX],
    longueur: AtomicUsize,
    /// Magasins servis a un CPU dont la liste etait vide.
    servis: AtomicU64,
    /// Magasins rendus par un CPU dont la liste debordait.
    deposes: AtomicU64,
    /// Demandes auxquelles le depot n'a pas pu repondre : le backing est
    /// atteint.
    vides: AtomicU64,
    /// Depots refuses faute de place : le backing recupere reellement.
    pleins: AtomicU64,
    /// Longueur maximale atteinte, pour dimensionner `MAGASINS_MAX`.
    pic: AtomicUsize,
}

impl Depot {
    pub const fn neuf() -> Self {
        Self {
            verrou: AtomicBool::new(false),
            tetes: [const { AtomicUsize::new(0) }; MAGASINS_MAX],
            comptes: [const { AtomicUsize::new(0) }; MAGASINS_MAX],
            longueur: AtomicUsize::new(0),
            servis: AtomicU64::new(0),
            deposes: AtomicU64::new(0),
            vides: AtomicU64::new(0),
            pleins: AtomicU64::new(0),
            pic: AtomicUsize::new(0),
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

    /// Retire un magasin, s'il y en a un.
    ///
    /// # Interruptions
    /// L'appelant doit les avoir masquees : voir l'en-tete du module.
    pub fn retire(&self) -> Option<Magasin> {
        self.prend();
        let n = self.longueur.load(Ordering::Relaxed);
        let magasin = if n == 0 {
            None
        } else {
            let indice = n - 1;
            self.longueur.store(indice, Ordering::Relaxed);
            Some(Magasin {
                tete: self.tetes[indice].load(Ordering::Relaxed),
                compte: self.comptes[indice].load(Ordering::Relaxed),
            })
        };
        self.rend();
        match magasin {
            Some(_) => { self.servis.fetch_add(1, Ordering::Relaxed); }
            None => { self.vides.fetch_add(1, Ordering::Relaxed); }
        }
        magasin
    }

    /// Depose un magasin. Rend `false` si le depot est plein : l'appelant doit
    /// alors rendre les blocs au backing, un par un.
    ///
    /// # Interruptions
    /// L'appelant doit les avoir masquees : voir l'en-tete du module.
    pub fn depose(&self, magasin: Magasin) -> bool {
        if magasin.compte == 0 {
            return true;
        }
        self.prend();
        let n = self.longueur.load(Ordering::Relaxed);
        let accepte = n < MAGASINS_MAX;
        if accepte {
            self.tetes[n].store(magasin.tete, Ordering::Relaxed);
            self.comptes[n].store(magasin.compte, Ordering::Relaxed);
            self.longueur.store(n + 1, Ordering::Relaxed);
        }
        self.rend();
        if accepte {
            self.deposes.fetch_add(1, Ordering::Relaxed);
            self.pic.fetch_max(n + 1, Ordering::Relaxed);
        } else {
            self.pleins.fetch_add(1, Ordering::Relaxed);
        }
        accepte
    }

    #[inline]
    pub fn longueur(&self) -> usize {
        self.longueur.load(Ordering::Relaxed)
    }

    pub fn compteurs(&self) -> CompteursDepot {
        CompteursDepot {
            magasins: self.longueur.load(Ordering::Relaxed) as u64,
            servis: self.servis.load(Ordering::Relaxed),
            deposes: self.deposes.load(Ordering::Relaxed),
            vides: self.vides.load(Ordering::Relaxed),
            pleins: self.pleins.load(Ordering::Relaxed),
            pic: self.pic.load(Ordering::Relaxed) as u64,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompteursDepot {
    pub magasins: u64,
    pub servis: u64,
    pub deposes: u64,
    pub vides: u64,
    pub pleins: u64,
    pub pic: u64,
}

/// Detache au plus `lot` blocs en tete d'une chaine.
///
/// Rend le magasin detache et la nouvelle tete de la chaine restante. La
/// marche est bornee par `lot` -- seize -- et se fait sur des blocs qui
/// viennent d'etre touches : c'est du parcours de cache chaud, pas un
/// parcours de liste.
///
/// # Securite
/// `tete` doit etre une chaine de blocs libres d'au moins `lot` elements, liee
/// par [`lien_ecrit`].
pub unsafe fn detache(tete: usize, lot: usize) -> (Magasin, usize) {
    if tete == 0 || lot == 0 {
        return (Magasin { tete: 0, compte: 0 }, tete);
    }
    let mut dernier = tete;
    let mut compte = 1usize;
    while compte < lot {
        let suivant = lien_lit(dernier);
        if suivant == 0 {
            break;
        }
        dernier = suivant;
        compte += 1;
    }
    let reste = lien_lit(dernier);
    lien_ecrit(dernier, 0);
    (Magasin { tete, compte }, reste)
}

/// Compte les blocs d'une chaine, jusqu'a `borne`.
///
/// Reserve au diagnostic et aux tests : le chemin chaud ne parcourt jamais une
/// chaine complete.
///
/// # Securite
/// Voir [`lien_lit`].
pub unsafe fn longueur_chaine(tete: usize, borne: usize) -> usize {
    let mut courant = tete;
    let mut n = 0usize;
    while courant != 0 && n < borne {
        courant = lien_lit(courant);
        n += 1;
    }
    n
}
