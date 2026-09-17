//! Ou passe le temps entre deux tours de scrutation HID.
//!
//! # La question que ce module existe pour trancher
//!
//! La session physique du 17 septembre montre UN pic de 638 ms sur
//! `hid_poll_gap_max_ms`, en 259 secondes. Trois causes possibles, et le
//! chiffre ne les separe pas :
//!
//!   * l'ordonnanceur -- la tache redevient prete et n'est pas elue ;
//!   * la famine du verrou xHCI -- elle est elue mais ne peut pas prendre le
//!     pilote ;
//!   * le corps de la scrutation lui-meme -- elle l'a pris et n'en sort pas.
//!
//! `note_poll_servi()` n'etait appele qu'APRES l'acquisition du verrou : un
//! ecart de 638 ms pouvait venir de n'importe laquelle des trois, et aucune
//! mesure existante ne disait laquelle. Trois hypotheses pour un chiffre,
//! c'est zero diagnostic.
//!
//! # Les quatre instants, et les trois intervalles qu'ils separent
//!
//! ```text
//!   T0  l'echeance du sommeil expire        -- la tache DOIT etre prete
//!   T1  premiere instruction utile du tour  -- la tache est ELUE
//!   T2  le jeton du pilote xHCI est pris    -- elle a le materiel
//!   T3  sortie de la scrutation             -- le tour est fini
//!
//!   T1-T0  wake_to_run   ordonnanceur
//!   T2-T1  run_to_lock   verrou xHCI
//!   T3-T2  poll_body     chemin HID
//! ```
//!
//! Chacun accuse EXACTEMENT un responsable. Leur somme est l'ecart observe,
//! donc aucun des trois ne peut se cacher derriere les autres.
//!
//! # Pourquoi des maxima et rien d'autre
//!
//! Le defaut est un PIC unique en quatre minutes. Une moyenne le noie, un
//! journal par tour le noie dans mille lignes par seconde -- et ferait payer
//! a la scrutation le cout de sa propre observation, ce qui est la faute
//! qu'on vient de passer trois sessions a corriger ailleurs.
//!
//! Module PUR : ni `use crate::`, ni `unsafe`, ni horloge. Les instants sont
//! des arguments.

use core::sync::atomic::{AtomicU64, Ordering};

/// Nombre de proprietaires possibles du verrou du pilote. Voir
/// `proprietaire_runtime::Proprietaire`, dont les codes vont de zero a six.
pub const PROPRIETAIRES: usize = 7;

/// Ce qu'un releve doit montrer du temps perdu par la scrutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Releve {
    /// Retard maximal entre l'echeance du sommeil et la reprise du tour.
    /// C'est la part de l'ORDONNANCEUR.
    pub wake_to_run_max_us: u64,
    /// Duree maximale entre la reprise et l'obtention du pilote, sur un tour
    /// qui l'a OBTENU.
    pub run_to_lock_max_us: u64,
    /// Duree maximale d'une FAMINE : du premier refus du verrou jusqu'au tour
    /// qui l'obtient enfin.
    ///
    /// # Pourquoi `run_to_lock` ne suffisait pas
    ///
    /// Il ne se mesure que sur les tours qui obtiennent le pilote. Un fil
    /// refuse trente-cinq fois de suite redort trente-cinq fois, et chacun de
    /// ces tours-la est court : le maximum reste petit pendant que l'ecart
    /// reel atteint cent quarante millisecondes.
    ///
    /// Au premier banc, les trois intervalles totalisaient trente et une
    /// millisecondes pour un ecart mesure de cent quarante-quatre. Les cent
    /// treize manquantes etaient ici, et nulle part ailleurs.
    pub lock_starve_max_us: u64,
    /// Duree maximale du corps de la scrutation, pilote en main. C'est la
    /// part du CHEMIN HID.
    pub poll_body_max_us: u64,
    /// Refus consecutifs maximaux du verrou.
    pub lock_fail_streak_max: u64,
    /// Refus cumules.
    pub lock_fail_total: u64,
    /// Refus par proprietaire du verrou au moment du refus.
    pub lock_fail_owner: [u64; PROPRIETAIRES],
    /// Tours mesures. Sans lui, un maximum ne dit pas s'il a ete rencontre
    /// une fois ou jamais.
    pub tours: u64,
}

impl Releve {
    /// Le responsable du pire ecart, en un mot.
    ///
    /// Trois intervalles, un seul coupable : celui qui domine. La regle est
    /// explicite plutot que laissee a la lecture, parce que c'est elle qui
    /// decide du correctif, et qu'un correctif choisi sur la mauvaise moitie
    /// d'un tableau coute une session physique.
    pub fn responsable(&self) -> &'static str {
        let w = self.wake_to_run_max_us;
        // LA FAMINE COMPTE COMME DU TEMPS PASSE A ATTENDRE LE VERROU.
        //
        // C'est la meme attente, vue de l'autre cote : `run_to_lock` mesure
        // celle des tours qui obtiennent le pilote, `lock_starve` celle des
        // series qui n'y arrivent pas. Prendre le plus grand des deux est ce
        // qui empeche une famine de cent quarante millisecondes de se cacher
        // derriere un `run_to_lock` de quatre.
        let r = self.run_to_lock_max_us.max(self.lock_starve_max_us);
        let p = self.poll_body_max_us;
        if w >= r && w >= p {
            "ordonnanceur"
        } else if r >= p {
            "verrou"
        } else {
            "chemin-hid"
        }
    }
}

pub struct ChronoHid {
    wake_to_run_max_ns: AtomicU64,
    run_to_lock_max_ns: AtomicU64,
    poll_body_max_ns: AtomicU64,
    serie: AtomicU64,
    serie_max: AtomicU64,
    /// Instant du PREMIER refus de la serie en cours, zero hors famine.
    famine_depuis_ns: AtomicU64,
    famine_max_ns: AtomicU64,
    total: AtomicU64,
    par_proprietaire: [AtomicU64; PROPRIETAIRES],
    tours: AtomicU64,
}

impl ChronoHid {
    pub const fn neuf() -> Self {
        Self {
            wake_to_run_max_ns: AtomicU64::new(0),
            run_to_lock_max_ns: AtomicU64::new(0),
            poll_body_max_ns: AtomicU64::new(0),
            serie: AtomicU64::new(0),
            serie_max: AtomicU64::new(0),
            famine_depuis_ns: AtomicU64::new(0),
            famine_max_ns: AtomicU64::new(0),
            total: AtomicU64::new(0),
            par_proprietaire: [const { AtomicU64::new(0) }; PROPRIETAIRES],
            tours: AtomicU64::new(0),
        }
    }

    /// T1 - T0 : le tour a repris `t1_ns`, il devait reprendre a `echeance_ns`.
    ///
    /// Une reprise EN AVANCE rend zero et non un nombre negatif replie sur
    /// soixante-quatre bits : `sleep_ticks` arrondit au tick, et une avance
    /// d'une microseconde deviendrait sinon le maximum de la session, pour
    /// toujours.
    pub fn note_reveil(&self, echeance_ns: u64, t1_ns: u64) {
        self.tours.fetch_add(1, Ordering::Relaxed);
        let retard = t1_ns.saturating_sub(echeance_ns);
        self.wake_to_run_max_ns.fetch_max(retard, Ordering::Relaxed);
    }

    /// T2 - T1 : le tour a mis ce temps a obtenir le pilote.
    ///
    /// Ferme aussi la famine en cours, s'il y en avait une : c'est le seul
    /// instant ou sa duree est connue.
    pub fn note_verrou_pris(&self, t1_ns: u64, t2_ns: u64) {
        self.run_to_lock_max_ns
            .fetch_max(t2_ns.saturating_sub(t1_ns), Ordering::Relaxed);
        self.serie.store(0, Ordering::Relaxed);
        let depuis = self.famine_depuis_ns.swap(0, Ordering::AcqRel);
        if depuis != 0 {
            self.famine_max_ns
                .fetch_max(t2_ns.saturating_sub(depuis), Ordering::Relaxed);
        }
    }

    /// T3 - T2 : le tour a tenu le pilote ce temps-la.
    pub fn note_corps(&self, t2_ns: u64, t3_ns: u64) {
        self.poll_body_max_ns
            .fetch_max(t3_ns.saturating_sub(t2_ns), Ordering::Relaxed);
    }

    /// Le verrou a ete refuse, et voici qui le tenait.
    ///
    /// Le proprietaire est lu APRES le refus : il a pu changer entre les deux.
    /// C'est une indication, pas une preuve -- mais sur une serie, celui qui
    /// revient est bien celui qui tient.
    pub fn note_echec_verrou(&self, proprietaire: u8, maintenant_ns: u64) {
        self.total.fetch_add(1, Ordering::Relaxed);
        // LE PREMIER REFUS DATE LA FAMINE, PAS LE DERNIER.
        //
        // Zero sert de « pas de famine en cours ». Un refus qui tomberait a
        // l'instant zero de l'horloge rouvrirait donc une famine a chaque
        // tour ; c'est sans consequence ici, puisque la scrutation ne demarre
        // pas avant l'enumeration du controleur.
        let _ = self.famine_depuis_ns.compare_exchange(
            0,
            maintenant_ns.max(1),
            Ordering::AcqRel,
            Ordering::Relaxed,
        );
        let serie = self.serie.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
        self.serie_max.fetch_max(serie, Ordering::Relaxed);
        let index = (proprietaire as usize).min(PROPRIETAIRES - 1);
        self.par_proprietaire[index].fetch_add(1, Ordering::Relaxed);
    }

    pub fn releve(&self) -> Releve {
        let mut par = [0u64; PROPRIETAIRES];
        for (i, compteur) in self.par_proprietaire.iter().enumerate() {
            par[i] = compteur.load(Ordering::Relaxed);
        }
        Releve {
            wake_to_run_max_us: self.wake_to_run_max_ns.load(Ordering::Relaxed) / 1_000,
            run_to_lock_max_us: self.run_to_lock_max_ns.load(Ordering::Relaxed) / 1_000,
            lock_starve_max_us: self.famine_max_ns.load(Ordering::Relaxed) / 1_000,
            poll_body_max_us: self.poll_body_max_ns.load(Ordering::Relaxed) / 1_000,
            lock_fail_streak_max: self.serie_max.load(Ordering::Relaxed),
            lock_fail_total: self.total.load(Ordering::Relaxed),
            lock_fail_owner: par,
            tours: self.tours.load(Ordering::Relaxed),
        }
    }
}
