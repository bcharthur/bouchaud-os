//! L'arene DMA : ce que les pilotes obtiennent, et ce qu'ils peuvent enfin
//! rendre.
//!
//! # Ce qu'etait l'arene
//!
//! Trois `static mut` et un pointeur qui monte :
//!
//!     let base = (DMA_NEXT + 0xFFF) & !0xFFF;
//!     DMA_NEXT = base + taille;
//!
//! Trois defauts, dans l'ordre de gravite :
//!
//!   1. RIEN NE SE REND. Un pilote retire, un anneau redimensionne, une pile
//!      reseau reinitialisee : la memoire est perdue jusqu'au redemarrage. Avec
//!      32 Mio d'arene, une reinitialisation d'anneau par erreur reseau suffit
//!      a l'epuiser -- et l'echec se manifeste beaucoup plus tard, sur un
//!      pilote qui n'a rien fait de mal ;
//!   2. C'ETAIT UNE COURSE. `static mut` sans verrou, lu-modifie-ecrit depuis
//!      n'importe quel coeur. Deux pilotes qui s'initialisent en parallele
//!      pouvaient recevoir LA MEME adresse physique, et se marcher dessus dans
//!      un tampon que le materiel lit ;
//!   3. rien n'etait mesure au-dela d'un compteur d'allocations : ni le pic,
//!      ni la fragmentation, ni ce qui a ete reutilise.
//!
//! # Ce que c'est maintenant
//!
//! Une frontiere -- ce qui n'a jamais ete distribue -- plus une LISTE de
//! regions rendues, bornee et fusionnante :
//!
//!   * `alloue` sert d'abord une region rendue (meilleur ajustement, pour ne
//!     pas hacher une grande region pour une petite demande), sinon avance la
//!     frontiere ;
//!   * `libere` fusionne avec les regions adjacentes, et REPLIE la frontiere
//!     quand la region rendue la touche. Une arene qui alloue puis rend tout
//!     revient donc exactement a son etat initial ;
//!   * la liste est bornee. Une region qui n'y tient pas n'est pas perdue en
//!     silence : elle est comptee (`debordements`), et le compteur dit qu'il
//!     faut agrandir la liste plutot que de chercher une fuite ailleurs.
//!
//! Ce n'est pas encore un allocateur par ordres. C'est l'ABSTRACTION qui
//! permettra de le devenir : les pilotes appellent `alloue`/`libere`, pas un
//! pointeur qui monte.
//!
//! # Interruptions
//!
//! Le verrou est un verrou tournant simple. L'appelant doit masquer les
//! interruptions : l'arene est atteignable depuis l'initialisation d'un pilote
//! comme depuis un gestionnaire. La regle appartient a l'appelant, ce module
//! restant sans dependance d'architecture pour rester testable sur l'hote.

use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

pub const PAGE: u64 = 4096;

/// Regions rendues suivies simultanement.
///
/// Chaque entree est une region CONTIGUE ; la fusion les recolle, donc ce
/// nombre borne la FRAGMENTATION, pas le nombre de liberations.
pub const REGIONS_MAX: usize = 64;

pub struct AreneDma {
    verrou: AtomicBool,
    debut: AtomicU64,
    /// Premiere adresse jamais distribuee.
    frontiere: AtomicU64,
    fin: AtomicU64,

    bases: [AtomicU64; REGIONS_MAX],
    tailles: [AtomicU64; REGIONS_MAX],
    regions: AtomicUsize,

    allocations: AtomicU64,
    liberations: AtomicU64,
    /// Allocations servies par une region rendue au lieu de la frontiere.
    reutilisations: AtomicU64,
    /// Liberations recollees a une region voisine ou a la frontiere.
    fusions: AtomicU64,
    /// Liberations que la liste n'a pas pu enregistrer.
    debordements: AtomicU64,
    echecs: AtomicU64,
    /// Plus grande occupation atteinte, frontiere moins ce qui est rendu.
    pic: AtomicU64,
}

impl AreneDma {
    pub const fn neuve() -> Self {
        Self {
            verrou: AtomicBool::new(false),
            debut: AtomicU64::new(0),
            frontiere: AtomicU64::new(0),
            fin: AtomicU64::new(0),
            bases: [const { AtomicU64::new(0) }; REGIONS_MAX],
            tailles: [const { AtomicU64::new(0) }; REGIONS_MAX],
            regions: AtomicUsize::new(0),
            allocations: AtomicU64::new(0),
            liberations: AtomicU64::new(0),
            reutilisations: AtomicU64::new(0),
            fusions: AtomicU64::new(0),
            debordements: AtomicU64::new(0),
            echecs: AtomicU64::new(0),
            pic: AtomicU64::new(0),
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

    /// Installe l'arene. `debut` est arrondi vers le haut, `fin` vers le bas.
    pub fn configure(&self, debut: u64, fin: u64) {
        self.prend();
        let debut = (debut + PAGE - 1) & !(PAGE - 1);
        let fin = fin & !(PAGE - 1);
        self.debut.store(debut, Ordering::Relaxed);
        self.frontiere.store(debut, Ordering::Relaxed);
        self.fin.store(if fin > debut { fin } else { debut }, Ordering::Relaxed);
        self.regions.store(0, Ordering::Relaxed);
        self.rend();
    }

    #[inline]
    pub fn configuree(&self) -> bool {
        self.fin.load(Ordering::Relaxed) > self.debut.load(Ordering::Relaxed)
    }

    /// Alloue `taille` octets, arrondis a la page.
    ///
    /// # Interruptions
    /// L'appelant doit les avoir masquees : voir l'en-tete du module.
    pub fn alloue(&self, taille: usize) -> Option<u64> {
        if taille == 0 {
            return None;
        }
        let besoin = ((taille as u64) + PAGE - 1) & !(PAGE - 1);
        self.prend();
        let resultat = self.alloue_verrouille(besoin);
        self.rend();
        match resultat {
            Some(_) => { self.allocations.fetch_add(1, Ordering::Relaxed); }
            None => { self.echecs.fetch_add(1, Ordering::Relaxed); }
        }
        resultat
    }

    fn alloue_verrouille(&self, besoin: u64) -> Option<u64> {
        // MEILLEUR AJUSTEMENT, pas premier ajustement : les tampons DMA sont
        // peu nombreux et de tailles tres inegales -- un anneau de descripteurs
        // de 4 Kio face a un tampon de reception de plusieurs centaines. Servir
        // la petite demande depuis la grande region hacherait la seule region
        // capable de reloger un anneau.
        let n = self.regions.load(Ordering::Relaxed);
        let mut choisi: Option<usize> = None;
        let mut meilleure = u64::MAX;
        for index in 0..n {
            let taille = self.tailles[index].load(Ordering::Relaxed);
            if taille >= besoin && taille < meilleure {
                meilleure = taille;
                choisi = Some(index);
            }
        }
        if let Some(index) = choisi {
            let base = self.bases[index].load(Ordering::Relaxed);
            let reste = meilleure - besoin;
            if reste == 0 {
                self.retire_region(index, n);
            } else {
                self.bases[index].store(base + besoin, Ordering::Relaxed);
                self.tailles[index].store(reste, Ordering::Relaxed);
            }
            self.reutilisations.fetch_add(1, Ordering::Relaxed);
            self.note_pic();
            return Some(base);
        }

        let base = self.frontiere.load(Ordering::Relaxed);
        let fin = base.checked_add(besoin)?;
        if fin > self.fin.load(Ordering::Relaxed) {
            return None;
        }
        self.frontiere.store(fin, Ordering::Relaxed);
        self.note_pic();
        Some(base)
    }

    /// Rend une region. `base` et `taille` doivent etre ceux d'une allocation.
    ///
    /// # Interruptions
    /// L'appelant doit les avoir masquees : voir l'en-tete du module.
    pub fn libere(&self, base: u64, taille: usize) {
        if taille == 0 || !self.configuree() {
            return;
        }
        let taille = ((taille as u64) + PAGE - 1) & !(PAGE - 1);
        let base = base & !(PAGE - 1);
        self.prend();
        self.libere_verrouille(base, taille);
        self.rend();
        self.liberations.fetch_add(1, Ordering::Relaxed);
    }

    fn libere_verrouille(&self, mut base: u64, mut taille: u64) {
        let debut = self.debut.load(Ordering::Relaxed);
        let fin = self.fin.load(Ordering::Relaxed);
        if base < debut || base + taille > fin {
            // Hors arene : ce n'est pas a nous. Le compter plutot que de
            // l'ajouter a la liste, ou il corromprait les allocations
            // suivantes.
            self.debordements.fetch_add(1, Ordering::Relaxed);
            return;
        }

        // Recoller aux voisines, autant de fois qu'il le faut : rendre la
        // region du milieu de trois doit produire UNE region, pas deux
        // adjacentes que la prochaine allocation ne saurait pas reunir.
        let mut fusionne = false;
        loop {
            let n = self.regions.load(Ordering::Relaxed);
            let mut voisine = None;
            for index in 0..n {
                let vbase = self.bases[index].load(Ordering::Relaxed);
                let vtaille = self.tailles[index].load(Ordering::Relaxed);
                if vbase + vtaille == base || base + taille == vbase {
                    voisine = Some((index, vbase, vtaille));
                    break;
                }
            }
            let Some((index, vbase, vtaille)) = voisine else { break };
            base = base.min(vbase);
            taille += vtaille;
            self.retire_region(index, n);
            fusionne = true;
        }

        // La region touche la frontiere : la REPLIER rend la memoire au
        // « jamais distribue » plutot que de la garder dans une liste bornee.
        // C'est ce qui fait qu'une arene qui alloue puis rend tout revient
        // exactement a son etat initial.
        if base + taille == self.frontiere.load(Ordering::Relaxed) {
            self.frontiere.store(base, Ordering::Relaxed);
            self.fusions.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if fusionne {
            self.fusions.fetch_add(1, Ordering::Relaxed);
        }

        let n = self.regions.load(Ordering::Relaxed);
        if n >= REGIONS_MAX {
            // La liste est pleine. La region n'est pas rendue -- mais elle est
            // COMPTEE : sans ce compteur, la fuite se lirait comme une fuite de
            // pilote, et on la chercherait pendant des jours au mauvais
            // endroit.
            self.debordements.fetch_add(1, Ordering::Relaxed);
            return;
        }
        self.bases[n].store(base, Ordering::Relaxed);
        self.tailles[n].store(taille, Ordering::Relaxed);
        self.regions.store(n + 1, Ordering::Relaxed);
    }

    #[inline]
    fn retire_region(&self, index: usize, n: usize) {
        let dernier = n - 1;
        if index != dernier {
            self.bases[index].store(self.bases[dernier].load(Ordering::Relaxed), Ordering::Relaxed);
            self.tailles[index]
                .store(self.tailles[dernier].load(Ordering::Relaxed), Ordering::Relaxed);
        }
        self.regions.store(dernier, Ordering::Relaxed);
    }

    #[inline]
    fn note_pic(&self) {
        let utilise = self
            .frontiere
            .load(Ordering::Relaxed)
            .saturating_sub(self.debut.load(Ordering::Relaxed))
            .saturating_sub(self.libre_en_liste());
        self.pic.fetch_max(utilise, Ordering::Relaxed);
    }

    fn libre_en_liste(&self) -> u64 {
        let n = self.regions.load(Ordering::Relaxed);
        let mut total = 0u64;
        for index in 0..n {
            total = total.saturating_add(self.tailles[index].load(Ordering::Relaxed));
        }
        total
    }

    pub fn etat(&self) -> EtatDma {
        let debut = self.debut.load(Ordering::Relaxed);
        let frontiere = self.frontiere.load(Ordering::Relaxed);
        let fin = self.fin.load(Ordering::Relaxed);
        let total = fin.saturating_sub(debut);
        let rendu = self.libre_en_liste();
        let utilise = frontiere.saturating_sub(debut).saturating_sub(rendu);
        EtatDma {
            total,
            utilise,
            libre: total.saturating_sub(utilise),
            rendu,
            regions: self.regions.load(Ordering::Relaxed) as u64,
            allocations: self.allocations.load(Ordering::Relaxed),
            liberations: self.liberations.load(Ordering::Relaxed),
            reutilisations: self.reutilisations.load(Ordering::Relaxed),
            fusions: self.fusions.load(Ordering::Relaxed),
            debordements: self.debordements.load(Ordering::Relaxed),
            echecs: self.echecs.load(Ordering::Relaxed),
            pic: self.pic.load(Ordering::Relaxed),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EtatDma {
    pub total: u64,
    pub utilise: u64,
    pub libre: u64,
    pub rendu: u64,
    pub regions: u64,
    pub allocations: u64,
    pub liberations: u64,
    pub reutilisations: u64,
    pub fusions: u64,
    pub debordements: u64,
    pub echecs: u64,
    pub pic: u64,
}
