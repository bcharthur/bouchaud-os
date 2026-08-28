//! Comptabilite du gros verrou : ce qui est mesure, et ce qui ne peut pas mentir.
//!
//! # Le releve qui a rendu tout le reste inutilisable
//!
//! ```text
//! [BKL-SYSCALL] window_ns=5000000000 poll=[hold_delta_ns=9166000000 hold_pct=183 ...]
//! [BKL-MAX-HOLD] ns=29562372510 origine=resume_after_schedule
//! ```
//!
//! 183 % du temps ecoule passe a detenir un verrou EXCLUSIF. C'est
//! arithmetiquement impossible : a tout instant il y a au plus un proprietaire,
//! donc la somme des tenues est majoree par le temps qui passe. Un chiffre
//! impossible ne se corrige pas en le divisant par deux -- il retire toute
//! valeur aux autres chiffres du meme releve, y compris a la pointe de 29
//! secondes qui, elle, designait un vrai figement.
//!
//! # Pourquoi l'ancien schema pouvait depasser 100 %
//!
//! L'horodatage d'acquisition vivait dans un tableau INDEXE PAR CPU :
//!
//! ```text
//! ACQUIRED_AT_NS[cpu].store(maintenant)   a l'acquisition
//! let debut = ACQUIRED_AT_NS[cpu].swap(0) a la liberation
//! TOTAL_HOLD_NS.fetch_add(maintenant - debut)   <- HORS du test `debut != 0`
//! ```
//!
//! Deux fautes, et la seconde est la plus couteuse :
//!
//!   1. l'addition avait lieu meme quand `debut` valait zero. `maintenant - 0`
//!      vaut alors le TEMPS DEPUIS LE DEMARRAGE. Une seule liberation orpheline
//!      ajoutait des dizaines de secondes au cumul ;
//!   2. l'intervalle etait attribue a un CPU, alors qu'il appartient au VERROU.
//!      Une case laissee non nulle par une acquisition dont la liberation n'a
//!      pas eu lieu sur le meme CPU reste en place, et c'est une liberation
//!      SANS RAPPORT, bien plus tard, qui la consomme : la duree publiee couvre
//!      alors tout ce qui s'est passe entre deux, avec la provenance de la
//!      premiere. Une tenue de quelques microsecondes se lit « 29 secondes dans
//!      resume_after_schedule ».
//!
//! # Ce que ce module garantit
//!
//! L'intervalle de tenue appartient au verrou, en un seul exemplaire :
//!
//! ```text
//! acquisition : debut <- maintenant       (swap : l'ancien est visible)
//! liberation  : debut -> 0, cumul += maintenant - debut
//! ```
//!
//! Le verrou etant exclusif, et la sonde de liberation s'executant AVANT que
//! `OWNER` ne repasse a `FREE`, les intervalles factures sont deux a deux
//! disjoints. Avec une horloge monotone, la somme est donc majoree par le temps
//! ecoule : `hold_pct <= 100` devient une propriete de la structure, pas un
//! resultat a esperer. `tools/smp/test_bkl_comptes.rs` en fait un test.
//!
//! Rien n'est absorbe en silence. Une liberation sans debut, une acquisition
//! par-dessus une tenue ouverte, une horloge qui recule : chacune INCREMENTE UN
//! COMPTEUR et ne facture rien. Un compteur d'anomalie non nul dit tout de
//! suite que le modele ne decrit plus la machine -- c'est exactement ce que
//! l'ancien code cachait en ajoutant l'uptime au cumul.
//!
//! # Les grandeurs sont separees, et n'ont pas le meme sens
//!
//! Les melanger est ce qui a produit un pourcentage impossible.
//!
//!   * `tenue_ns` -- temps REELLEMENT proprietaire. Du temps de MURAILLE :
//!     majore par la fenetre, donc comparable a elle.
//!   * `attente_ns` -- temps passe a attendre avant d'acquerir. Du temps de
//!     CPU : quatre coeurs qui attendent une seconde en cumulent quatre.
//!     Depasser la fenetre est ici NORMAL, et ne signale rien.
//!   * `reprise_ns` -- la part de cette attente qui est subie par une pile
//!     reprise apres un changement de contexte, `resume_after_schedule`. Sous
//!     ensemble d'`attente_ns`, isole parce que c'est lui qu'on soupconne.
//!
//! Le reste compte des EVENEMENTS, jamais du temps : spins, parkings, IPI de
//! reveil, reveils improductifs, et la ventilation par CPU qui dit sur QUI on
//! attend et QUI recoit les reveils.

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Assez pour le `MAX_CPUS` du noyau ; le module reste independant de lui pour
/// pouvoir se compiler sur l'hote.
pub const MAX_CPUS: usize = 16;

/// Valeur d'un champ « CPU » sans proprietaire connu.
pub const AUCUN: usize = usize::MAX;

/// Une tenue close, telle que la liberation l'a mesuree.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tenue {
    /// Duree en nanosecondes.
    pub ns: u64,
    /// Seau d'appel systeme note a l'ACQUISITION -- c'est l'acquereur qui a
    /// tenu le verrou pendant tout l'intervalle, pas celui qui court maintenant.
    pub seau: usize,
    /// CPU qui a acquis.
    pub cpu_acquisition: usize,
    /// CPU qui libere. Different du precedent = la continuation a MIGRE entre
    /// les deux, et c'est precisement ce que l'ancien schema par CPU ne pouvait
    /// pas representer.
    pub cpu_liberation: usize,
    /// Origine notee a l'acquisition : 1 `enter`, 2 `try_enter`, 3 `resume`.
    pub origine: u32,
}

/// Etat de la comptabilite. Un seul exemplaire, celui du verrou.
pub struct Comptes {
    // --- l'intervalle en cours : un seul, parce que le verrou est exclusif ---
    debut_ns: AtomicU64,
    debut_seau: AtomicUsize,
    debut_cpu: AtomicUsize,
    debut_origine: AtomicU64,

    // --- du temps, et chaque ligne a son unite ---
    tenue_ns: AtomicU64,
    attente_ns: AtomicU64,
    reprise_ns: AtomicU64,
    reprise_max_ns: AtomicU64,

    // --- des evenements ---
    spins: AtomicU64,
    parks: AtomicU64,
    wake_ipis: AtomicU64,
    reveils_sans_acquisition: AtomicU64,
    /// Sur QUI on s'est arrete : index par le CPU proprietaire vu au parking,
    /// la derniere case pour « verrou deja libre ».
    parks_par_proprietaire: [AtomicU64; MAX_CPUS + 1],
    /// QUI recoit les reveils.
    wakes_par_cible: [AtomicU64; MAX_CPUS],
    /// Tenues closes par un CPU autre que l'acquereur.
    liberations_migrees: AtomicU64,

    // --- des anomalies : jamais absorbees, toujours comptees ---
    liberations_sans_debut: AtomicU64,
    acquisitions_sur_tenue: AtomicU64,
    horloge_a_rebours: AtomicU64,
}

impl Comptes {
    pub const fn neuf() -> Self {
        Self {
            debut_ns: AtomicU64::new(0),
            debut_seau: AtomicUsize::new(0),
            debut_cpu: AtomicUsize::new(AUCUN),
            debut_origine: AtomicU64::new(0),
            tenue_ns: AtomicU64::new(0),
            attente_ns: AtomicU64::new(0),
            reprise_ns: AtomicU64::new(0),
            reprise_max_ns: AtomicU64::new(0),
            spins: AtomicU64::new(0),
            parks: AtomicU64::new(0),
            wake_ipis: AtomicU64::new(0),
            reveils_sans_acquisition: AtomicU64::new(0),
            parks_par_proprietaire: [const { AtomicU64::new(0) }; MAX_CPUS + 1],
            wakes_par_cible: [const { AtomicU64::new(0) }; MAX_CPUS],
            liberations_migrees: AtomicU64::new(0),
            liberations_sans_debut: AtomicU64::new(0),
            acquisitions_sur_tenue: AtomicU64::new(0),
            horloge_a_rebours: AtomicU64::new(0),
        }
    }

    /// Ouvre l'intervalle de tenue.
    ///
    /// A appeler APRES avoir obtenu la propriete, et sous interruptions
    /// masquees comme le reste de la transition : le `swap` ne serialise rien
    /// a lui seul, c'est l'exclusion mutuelle du verrou qui garantit qu'un
    /// seul appelant est ici a la fois.
    ///
    /// `maintenant_ns` valant zero serait indistinguable de « pas de tenue » ;
    /// on le remonte a 1. Perdre une nanoseconde a l'instant du demarrage
    /// coute moins qu'un cas particulier qui traverserait tout le module.
    #[inline]
    pub fn ouvre(&self, maintenant_ns: u64, seau: usize, cpu: usize, origine: u32) {
        let ancien = self.debut_ns.swap(maintenant_ns.max(1), Ordering::Relaxed);
        if ancien != 0 {
            // Une tenue etait deja ouverte : le modele ne decrit plus la
            // machine. On ABANDONNE l'intervalle precedent -- le facturer
            // reviendrait a compter deux fois le meme temps de muraille, ce
            // qui est exactement la faute qu'on repare.
            self.acquisitions_sur_tenue.fetch_add(1, Ordering::Relaxed);
        }
        self.debut_seau.store(seau, Ordering::Relaxed);
        self.debut_cpu.store(cpu, Ordering::Relaxed);
        self.debut_origine.store(origine as u64, Ordering::Relaxed);
    }

    /// Ferme l'intervalle et le facture. Rend `None` si rien n'etait ouvert,
    /// ou si l'horloge a recule -- dans les deux cas, RIEN n'est ajoute.
    ///
    /// A appeler AVANT de rendre `OWNER` a `FREE` : c'est ce qui rend les
    /// intervalles disjoints, donc leur somme majoree par le temps ecoule.
    #[inline]
    pub fn ferme(&self, maintenant_ns: u64, cpu: usize) -> Option<Tenue> {
        let debut = self.debut_ns.swap(0, Ordering::Relaxed);
        if debut == 0 {
            self.liberations_sans_debut.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        if maintenant_ns < debut {
            self.horloge_a_rebours.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        let cpu_acquisition = self.debut_cpu.swap(AUCUN, Ordering::Relaxed);
        if cpu_acquisition != cpu {
            self.liberations_migrees.fetch_add(1, Ordering::Relaxed);
        }
        let ns = maintenant_ns - debut;
        self.tenue_ns.fetch_add(ns, Ordering::Relaxed);
        Some(Tenue {
            ns,
            seau: self.debut_seau.load(Ordering::Relaxed),
            cpu_acquisition,
            cpu_liberation: cpu,
            origine: self.debut_origine.load(Ordering::Relaxed) as u32,
        })
    }

    /// CPU proprietaire a cet instant, s'il y en a un.
    #[inline]
    pub fn proprietaire(&self) -> usize {
        if self.debut_ns.load(Ordering::Relaxed) == 0 {
            AUCUN
        } else {
            self.debut_cpu.load(Ordering::Relaxed)
        }
    }

    /// Temps d'attente avant une acquisition reussie. Du temps de CPU : le
    /// cumul de plusieurs coeurs peut depasser la fenetre, et c'est normal.
    #[inline]
    pub fn note_attente(&self, ns: u64) {
        self.attente_ns.fetch_add(ns, Ordering::Relaxed);
    }

    /// La part de l'attente subie par une pile reprise apres commutation.
    #[inline]
    pub fn note_reprise(&self, ns: u64) {
        self.reprise_ns.fetch_add(ns, Ordering::Relaxed);
        let mut record = self.reprise_max_ns.load(Ordering::Relaxed);
        while ns > record {
            match self.reprise_max_ns.compare_exchange_weak(
                record, ns, Ordering::Relaxed, Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(vu) => record = vu,
            }
        }
    }

    #[inline]
    pub fn note_spin(&self) {
        self.spins.fetch_add(1, Ordering::Relaxed);
    }

    /// Un CPU s'arrete. `proprietaire` est celui qu'il voyait detenir le
    /// verrou, ou [`AUCUN`] s'il etait deja libre -- ce second cas est une
    /// course benigne, mais nombreuse elle signale un parking inutile.
    #[inline]
    pub fn note_park(&self, proprietaire: usize) {
        self.parks.fetch_add(1, Ordering::Relaxed);
        let case = if proprietaire >= MAX_CPUS { MAX_CPUS } else { proprietaire };
        self.parks_par_proprietaire[case].fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn note_wake(&self, cible: usize) {
        self.wake_ipis.fetch_add(1, Ordering::Relaxed);
        if cible < MAX_CPUS {
            self.wakes_par_cible[cible].fetch_add(1, Ordering::Relaxed);
        }
    }

    /// `combien` reveils ont rendu la main sans que le verrou soit pris.
    ///
    /// C'est LA mesure du troupeau : `wake_parked_waiters` reveille tous les
    /// gares, mais un seul peut acquerir. Si ce compteur suit le nombre d'IPI
    /// de reveil, le reveil groupe coute plus qu'il ne rapporte.
    #[inline]
    pub fn note_reveils_improductifs(&self, combien: u64) {
        if combien != 0 {
            self.reveils_sans_acquisition.fetch_add(combien, Ordering::Relaxed);
        }
    }

    // --- lecture -----------------------------------------------------------

    #[inline]
    pub fn tenue_ns(&self) -> u64 { self.tenue_ns.load(Ordering::Relaxed) }
    #[inline]
    pub fn attente_ns(&self) -> u64 { self.attente_ns.load(Ordering::Relaxed) }
    #[inline]
    pub fn reprise_ns(&self) -> u64 { self.reprise_ns.load(Ordering::Relaxed) }
    #[inline]
    pub fn reprise_max_ns(&self) -> u64 { self.reprise_max_ns.load(Ordering::Relaxed) }
    #[inline]
    pub fn spins(&self) -> u64 { self.spins.load(Ordering::Relaxed) }
    #[inline]
    pub fn parks(&self) -> u64 { self.parks.load(Ordering::Relaxed) }
    #[inline]
    pub fn wake_ipis(&self) -> u64 { self.wake_ipis.load(Ordering::Relaxed) }
    #[inline]
    pub fn reveils_sans_acquisition(&self) -> u64 {
        self.reveils_sans_acquisition.load(Ordering::Relaxed)
    }
    #[inline]
    pub fn parks_sur(&self, cpu: usize) -> u64 {
        self.parks_par_proprietaire[cpu.min(MAX_CPUS)].load(Ordering::Relaxed)
    }
    #[inline]
    pub fn wakes_vers(&self, cpu: usize) -> u64 {
        self.wakes_par_cible[cpu.min(MAX_CPUS - 1)].load(Ordering::Relaxed)
    }
    #[inline]
    pub fn liberations_migrees(&self) -> u64 {
        self.liberations_migrees.load(Ordering::Relaxed)
    }

    /// Les trois anomalies, dans l'ordre : liberation sans debut, acquisition
    /// par-dessus une tenue ouverte, horloge a rebours.
    ///
    /// Toutes a zero est la CONDITION pour que les durees ci-dessus decrivent
    /// la machine. Non nulles, elles disent ou le modele s'est decroche, au
    /// lieu de laisser un cumul absurde le suggerer.
    #[inline]
    pub fn anomalies(&self) -> (u64, u64, u64) {
        (
            self.liberations_sans_debut.load(Ordering::Relaxed),
            self.acquisitions_sur_tenue.load(Ordering::Relaxed),
            self.horloge_a_rebours.load(Ordering::Relaxed),
        )
    }
}
