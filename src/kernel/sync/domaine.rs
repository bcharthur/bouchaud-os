//! A quel chemin appartient chaque prise du gros verrou, et lesquels ont promis
//! de ne plus la prendre.
//!
//! # Pourquoi un compteur global ne suffit pas
//!
//! `[BKL-STATS] acquisitions=387000` dit qu'il y en a beaucoup. Il ne dit pas
//! LESQUELLES restent legitimes. Or la sortie du gros verrou ne se fait pas en
//! une fois : elle se fait chemin par chemin, et chaque chemin migre doit
//! ensuite le RESTER. Sans attribution, une regression -- un appelant qui
//! reprend le verrou dans un sous-systeme deja sorti -- est indiscernable du
//! bruit de fond des chemins non encore migres.
//!
//! # Le contrat
//!
//! Chaque domaine porte un contrat, et le contrat est verifiable a l'execution :
//!
//!   * `Migre`      -- ce chemin ne prend PLUS le gros verrou. Toute
//!                     acquisition attribuee ici est une REGRESSION, comptee
//!                     comme telle et attribuable au domaine.
//!   * `EnMigration` -- le travail est commence ; les acquisitions sont comptees
//!                     pour mesurer ce qu'il reste, sans etre des fautes.
//!   * `Legacy`     -- pas encore commence. Compte, ne juge pas.
//!   * `Exempte`    -- boot tres precoce et panique. Le gros verrou y reste
//!                     legitime : il n'y a pas de concurrence a proteger au
//!                     boot, et une panique n'a pas a etre elegante.
//!
//! La valeur attendue de `violations` est ZERO, pour toujours. Ce n'est pas une
//! esperance : c'est ce que le garde-fou et le journal verifient.
//!
//! # Comment l'attribution fonctionne
//!
//! Chaque chemin de production ouvre une portee (`Portee`), qui empile son
//! domaine sur une pile PAR CPU. Le gros verrou, a chaque acquisition, lit le
//! sommet de cette pile. C'est le domaine le plus INTERIEUR qui attribue : si
//! un appel systeme entre dans le domaine `Fd`, qui appelle le systeme de
//! fichiers, c'est `Fs` qui paie -- c'est bien lui qui a eu besoin du verrou.
//!
//! Tout est en atomiques par CPU, sans allocation ni verrou : le chemin
//! d'acquisition du gros verrou s'execute interruptions masquees, et un
//! diagnostic qui aurait besoin de dormir y serait fatal.

use core::sync::atomic::{AtomicU64, AtomicU8, AtomicUsize, Ordering};

/// Assez pour le `MAX_CPUS` du noyau ; le module reste independant de lui pour
/// pouvoir se compiler et se tester sur l'hote.
pub const MAX_CPUS: usize = 16;

/// Profondeur d'imbrication des portees suivie exactement. Au-dela, on compte
/// un debordement et on conserve le domaine le plus profond connu : perdre la
/// precision est acceptable, mentir ne l'est pas.
pub const PROFONDEUR: usize = 8;

/// Les chemins du noyau, tels que le chantier « sortie du gros verrou » les
/// decoupe.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Domaine {
    /// Aucune portee ouverte : du code qui n'a pas encore ete rattache.
    Indetermine = 0,
    Syscall = 1,
    Vm = 2,
    Ordonnanceur = 3,
    Processus = 4,
    Fd = 5,
    Readiness = 6,
    Futex = 7,
    Reseau = 8,
    Vfs = 9,
    Fs = 10,
    Pilote = 11,
    /// Registre global des processus. Sorti du gros verrou : `RankedSpinLock`
    /// de classe `ProcessTable`.
    RegistreProcessus = 12,
    /// Verrous d'enregistrement POSIX (`fcntl`). Sorti du gros verrou :
    /// `RankedSpinLock` de classe `PosixRecord`.
    VerrouEnregistrement = 13,
    /// Avant que le SMP ne soit demarre : un seul coeur, rien a serialiser.
    BootPrecoce = 14,
    /// Faute fatale ou panique : la tache est demontee, ou la machine
    /// s'arrete. Il n'y a plus de concurrence a preserver, et la coherence du
    /// diagnostic prime.
    Panique = 15,
}

pub const NOMBRE: usize = 16;

impl Domaine {
    pub const fn code(self) -> u8 { self as u8 }

    pub const fn depuis_code(code: u8) -> Self {
        match code {
            1 => Self::Syscall,
            2 => Self::Vm,
            3 => Self::Ordonnanceur,
            4 => Self::Processus,
            5 => Self::Fd,
            6 => Self::Readiness,
            7 => Self::Futex,
            8 => Self::Reseau,
            9 => Self::Vfs,
            10 => Self::Fs,
            11 => Self::Pilote,
            12 => Self::RegistreProcessus,
            13 => Self::VerrouEnregistrement,
            14 => Self::BootPrecoce,
            15 => Self::Panique,
            _ => Self::Indetermine,
        }
    }

    pub const fn nom(self) -> &'static str {
        match self {
            Self::Indetermine => "indetermine",
            Self::Syscall => "syscall",
            Self::Vm => "vm",
            Self::Ordonnanceur => "ordonnanceur",
            Self::Processus => "processus",
            Self::Fd => "fd",
            Self::Readiness => "readiness",
            Self::Futex => "futex",
            Self::Reseau => "reseau",
            Self::Vfs => "vfs",
            Self::Fs => "fs",
            Self::Pilote => "pilote",
            Self::RegistreProcessus => "registre-processus",
            Self::VerrouEnregistrement => "verrou-enregistrement",
            Self::BootPrecoce => "boot-precoce",
            Self::Panique => "panique",
        }
    }

    /// Le contrat de ce domaine. Une constante, relue a chaque migration : c'est
    /// la SEULE chose a changer pour declarer un chemin sorti, et le journal
    /// dira aussitot si la declaration est vraie.
    pub const fn contrat(self) -> Contrat {
        match self {
            // Sortis, et verifies comme tels.
            Self::RegistreProcessus | Self::VerrouEnregistrement => Contrat::Migre,
            // Legitimes : pas de concurrence a proteger.
            Self::BootPrecoce | Self::Panique => Contrat::Exempte,
            // Le chantier en cours.
            Self::Ordonnanceur | Self::Processus | Self::Readiness | Self::Futex => {
                Contrat::EnMigration
            }
            _ => Contrat::Legacy,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Contrat {
    Migre,
    EnMigration,
    Legacy,
    Exempte,
}

impl Contrat {
    pub const fn nom(self) -> &'static str {
        match self {
            Self::Migre => "migre",
            Self::EnMigration => "en-migration",
            Self::Legacy => "legacy",
            Self::Exempte => "exempte",
        }
    }
}

/// L'etat de l'attribution. Un seul exemplaire dans le noyau ; le type reste
/// instanciable pour que les tests hote en fabriquent autant qu'ils veulent.
pub struct Registre {
    pile: [[AtomicU8; PROFONDEUR]; MAX_CPUS],
    sommet: [AtomicUsize; MAX_CPUS],
    acquisitions: [AtomicU64; NOMBRE],
    violations: [AtomicU64; NOMBRE],
    debordements: AtomicU64,
    /// Premier domaine migre a avoir repris le verrou, code + 1 (0 = aucun).
    /// Le PREMIER, pas le dernier : c'est celui-la qui a introduit la
    /// regression, les suivants peuvent n'en etre que la consequence.
    premiere_regression: AtomicU8,
}

impl Registre {
    pub const fn neuf() -> Self {
        Self {
            pile: [const { [const { AtomicU8::new(0) }; PROFONDEUR] }; MAX_CPUS],
            sommet: [const { AtomicUsize::new(0) }; MAX_CPUS],
            acquisitions: [const { AtomicU64::new(0) }; NOMBRE],
            violations: [const { AtomicU64::new(0) }; NOMBRE],
            debordements: AtomicU64::new(0),
            premiere_regression: AtomicU8::new(0),
        }
    }

    /// Ouvre une portee sur `cpu`.
    ///
    /// Le domaine est ecrit AVANT que le sommet ne monte : une interruption qui
    /// arrive entre les deux verrait sinon une case non encore ecrite, et
    /// attribuerait son acquisition au domaine du tour precedent.
    #[inline]
    pub fn entre(&self, cpu: usize, domaine: Domaine) {
        if cpu >= MAX_CPUS {
            return;
        }
        let n = self.sommet[cpu].load(Ordering::Relaxed);
        if n < PROFONDEUR {
            self.pile[cpu][n].store(domaine.code(), Ordering::Relaxed);
        } else {
            self.debordements.fetch_add(1, Ordering::Relaxed);
        }
        self.sommet[cpu].store(n + 1, Ordering::Relaxed);
    }

    /// Referme la portee la plus interieure de `cpu`.
    #[inline]
    pub fn sort(&self, cpu: usize) {
        if cpu >= MAX_CPUS {
            return;
        }
        let n = self.sommet[cpu].load(Ordering::Relaxed);
        // Une pile qui remonterait au-dessus de zero signalerait un `sort` sans
        // `entre` : impossible avec la RAII, et le taire vaut mieux que boucler.
        self.sommet[cpu].store(n.saturating_sub(1), Ordering::Relaxed);
    }

    /// Le domaine le plus interieur ouvert sur `cpu`.
    #[inline]
    pub fn courant(&self, cpu: usize) -> Domaine {
        if cpu >= MAX_CPUS {
            return Domaine::Indetermine;
        }
        let n = self.sommet[cpu].load(Ordering::Relaxed);
        if n == 0 {
            return Domaine::Indetermine;
        }
        // Au-dela de la profondeur suivie, on rend le plus profond connu.
        let index = if n > PROFONDEUR { PROFONDEUR - 1 } else { n - 1 };
        Domaine::depuis_code(self.pile[cpu][index].load(Ordering::Relaxed))
    }

    /// Attribue une acquisition du gros verrou au domaine courant de `cpu`.
    ///
    /// Rend `Some(domaine)` lorsque c'est une REGRESSION : un chemin declare
    /// sorti vient de reprendre le verrou. L'appelant decide quoi en faire ;
    /// ce module ne fait que compter, parce qu'il s'execute interruptions
    /// masquees sur le chemin d'acquisition et n'a le droit de rien de plus.
    #[inline]
    pub fn note_acquisition(&self, cpu: usize) -> Option<Domaine> {
        let domaine = self.courant(cpu);
        self.acquisitions[domaine.code() as usize].fetch_add(1, Ordering::Relaxed);
        if !matches!(domaine.contrat(), Contrat::Migre) {
            return None;
        }
        self.violations[domaine.code() as usize].fetch_add(1, Ordering::Relaxed);
        // `compare_exchange` et non `store` : on garde la PREMIERE.
        let _ = self.premiere_regression.compare_exchange(
            0,
            domaine.code() + 1,
            Ordering::Relaxed,
            Ordering::Relaxed,
        );
        Some(domaine)
    }

    // --- lecture -----------------------------------------------------------

    #[inline]
    pub fn acquisitions(&self, domaine: Domaine) -> u64 {
        self.acquisitions[domaine.code() as usize].load(Ordering::Relaxed)
    }

    #[inline]
    pub fn violations(&self, domaine: Domaine) -> u64 {
        self.violations[domaine.code() as usize].load(Ordering::Relaxed)
    }

    /// Somme des regressions, tous domaines confondus. Doit valoir zero.
    pub fn total_violations(&self) -> u64 {
        let mut total = 0;
        for index in 0..NOMBRE {
            total += self.violations[index].load(Ordering::Relaxed);
        }
        total
    }

    /// Acquisitions attribuees a un chemin NORMAL, c'est-a-dire ni boot
    /// precoce, ni panique, ni non rattache.
    ///
    /// C'est le chiffre que le chantier fait baisser, et le seul qui ait un
    /// sens comme objectif : le total inclut le boot, qu'on ne migrera jamais.
    pub fn acquisitions_chemins_normaux(&self) -> u64 {
        let mut total = 0;
        for index in 0..NOMBRE {
            let domaine = Domaine::depuis_code(index as u8);
            if matches!(domaine.contrat(), Contrat::Exempte)
                || matches!(domaine, Domaine::Indetermine)
            {
                continue;
            }
            total += self.acquisitions[index].load(Ordering::Relaxed);
        }
        total
    }

    pub fn debordements(&self) -> u64 {
        self.debordements.load(Ordering::Relaxed)
    }

    /// Le premier domaine migre a avoir repris le verrou, s'il y en a un.
    pub fn premiere_regression(&self) -> Option<Domaine> {
        match self.premiere_regression.load(Ordering::Relaxed) {
            0 => None,
            code => Some(Domaine::depuis_code(code - 1)),
        }
    }

    /// Profondeur de portees ouverte sur `cpu`. Sert aux post-conditions : un
    /// chemin qui rend la main doit avoir referme ce qu'il a ouvert.
    pub fn profondeur(&self, cpu: usize) -> usize {
        if cpu >= MAX_CPUS {
            return 0;
        }
        self.sommet[cpu].load(Ordering::Relaxed)
    }
}
