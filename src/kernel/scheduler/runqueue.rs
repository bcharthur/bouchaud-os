//! La file d'execution par CPU : deux bandes, des bitmaps, aucune allocation.
//!
//! # Ce qui etait la, et ce que cela coutait
//!
//! La file etait un `SpinLockIrq<Vec<u64>>`, et les quatre operations du
//! chemin chaud etaient exactement les quatre plus mauvaises que ce type
//! offre :
//!
//!   * `contains()` -- lineaire, a chaque mise en file, pour dedupliquer ;
//!   * `remove(0)` -- lineaire, et il DEPLACE tout le vecteur, a chaque
//!     election ;
//!   * `push()` -- peut REALLOUER, donc allouer, interruptions masquees, sous
//!     un verrou, depuis un gestionnaire d'interruption ;
//!   * `len()` -- prend le verrou, alors que le choix du CPU d'accueil
//!     (`choose_runq_cpu`) l'appelle une fois PAR CPU a chaque reveil.
//!
//! Et la priorite `Interactive` n'existait nulle part dans la structure : le
//! commentaire promettait un tourniquet a deux etages, la file etait une seule
//! FIFO. Une tache interactive attendait derriere le rendu.
//!
//! # La structure
//!
//! Deux BANDES par CPU -- interactive, normale --, chacune un BITMAP
//! d'emplacements du registre des taches. Un emplacement est un numero borne
//! (`EMPLACEMENTS`), donc un bit ; l'appartenance est un test de bit, la mise
//! en file un `fetch_or`, le retrait un `fetch_and`.
//!
//!   * mise en file    : O(1), un `fetch_or`, jamais d'allocation ;
//!   * retrait         : O(1), un mot de resume puis un `trailing_zeros` ;
//!   * appartenance    : O(1), un test de bit ;
//!   * longueur        : O(1), un compteur atomique, SANS VERROU.
//!
//! Il n'y a plus de verrou du tout. C'est ce qui ferme, par construction, la
//! reentrance par interruption que `SpinLockIrq` ne faisait que masquer : un
//! gestionnaire d'interruption qui met une tache en file execute un `fetch_or`
//! sur un mot, pas une section critique.
//!
//! # L'exactement-une-fois
//!
//! Deux CPU peuvent viser le meme bit. Le protocole est celui de l'echange
//! atomique, pas celui de l'exclusion :
//!
//!   * mise en file : `fetch_or` ; celui qui observe le bit A ZERO avant lui
//!     est le seul a compter une entree. Les autres sont des DOUBLONS, comptes
//!     comme tels, et ne creent pas de seconde entree ;
//!   * retrait : `fetch_and` ; celui qui observe le bit A UN avant lui est le
//!     seul a servir la tache. Les autres recommencent.
//!
//! Une tache ne peut donc etre ni perdue ni servie deux fois, y compris quand
//! deux coeurs elisent en meme temps.
//!
//! # La generation, et pourquoi elle ne tient pas dans le bit
//!
//! Un emplacement se recycle. Un bit seul designerait alors la tache SUIVANTE,
//! qui n'a rien demande -- c'est le probleme ABA, et c'est pour lui que la file
//! portait des identites empaquetees et non des indices.
//!
//! L'identite complete reste donc portee, mais dans une table PAR EMPLACEMENT
//! et non par file : un emplacement n'est en file que dans une seule bande d'un
//! seul CPU a la fois, une seule case suffit donc pour toute la machine. Elle
//! est ecrite AVANT le bit et relue APRES son retrait ; le consommateur refuse
//! ce que le registre ne reconnait plus.
//!
//! # Le mot de resume, et pourquoi il ne peut pas mentir dangereusement
//!
//! `resume` dit quels mots du bitmap sont non vides, pour que le retrait soit
//! un `trailing_zeros` et non un balayage. Un bit de resume peut etre en retard
//! -- deux coeurs qui vident et remplissent le meme mot --, et un bit de resume
//! en retard PERDRAIT une tache prete, ce qui fige une machine.
//!
//! Deux protections, et la seconde suffit seule :
//!
//!   1. qui vide un mot efface son bit de resume PUIS RELIT le mot ; s'il n'est
//!      plus vide, il remet le bit. En ordre `SeqCst`, l'ordre total place la
//!      relecture apres toute pose de bit deja publiee ;
//!   2. `longueur` est l'autorite. Si elle est non nulle et que le resume ne
//!      donne rien, le retrait BALAYE les mots. Le balayage est borne par
//!      `MOTS` -- seize, quel que soit le nombre de taches --, et la
//!      correction ne depend donc jamais du protocole du resume.

use core::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

/// Nombre d'emplacements du registre des taches couverts par les bitmaps.
///
/// Doit valoir exactement `kernel::task::MAX_TACHES` : le noyau le verifie a la
/// compilation (voir `cpu_local.rs`).
pub const EMPLACEMENTS: usize = 1024;

/// Mots de 64 bits necessaires pour couvrir tous les emplacements.
pub const MOTS: usize = EMPLACEMENTS / 64;

const _: () = assert!(EMPLACEMENTS % 64 == 0);
const _: () = assert!(MOTS <= 64, "le mot de resume ne couvre plus tous les mots");

/// Nombre de tours interactifs consecutifs avant qu'un tour soit rendu a la
/// bande normale.
///
/// C'est la SEULE chose qui separe une priorite d'une famine. Une tache
/// interactive qui ne se bloque jamais laisse malgre tout passer une tache
/// normale un tour sur `TOURS_INTERACTIFS_MAX + 1`.
pub const TOURS_INTERACTIFS_MAX: u32 = 8;

/// Retraits infructueux consecutifs tolerees avant d'abandonner le tour.
///
/// Perdre la course sur un bit signifie qu'un autre coeur a servi la tache :
/// il y a donc du progres global. Abandonner apres quelques tentatives ne perd
/// rien -- les bits restants sont toujours la -- et borne le temps passe dans
/// l'election.
const TENTATIVES_MAX: usize = 8;

/// Les deux bandes d'une file. L'ordre est la priorite.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bande {
    Interactive = 0,
    Normale = 1,
}

pub const NB_BANDES: usize = 2;

impl Bande {
    #[inline]
    pub const fn indice(self) -> usize {
        self as usize
    }

    #[inline]
    pub const fn depuis_indice(indice: usize) -> Self {
        match indice {
            0 => Self::Interactive,
            _ => Self::Normale,
        }
    }

    #[inline]
    pub const fn nom(self) -> &'static str {
        match self {
            Self::Interactive => "interactive",
            Self::Normale => "normale",
        }
    }
}

/// Une bande : un bitmap d'emplacements, son resume, sa longueur, son curseur.
pub struct BandeFile {
    mots: [AtomicU64; MOTS],
    /// Bit `i` : `mots[i]` est (peut-etre) non nul. Accelerateur, jamais
    /// autorite -- voir l'en-tete du module.
    resume: AtomicU64,
    longueur: AtomicUsize,
    /// Mot par lequel commencer la prochaine ELECTION, et par lequel commencer
    /// le prochain VOL. Deux curseurs, parce que les deux services partent de
    /// bouts opposes : un curseur commun ferait deplacer l'election par un vol
    /// venu d'un autre coeur.
    ///
    /// Le curseur AVANCE apres chaque service. Sans cela, un mot du bitmap qui
    /// ne se vide jamais -- une tache qui se remet prete aussitot servie --
    /// retiendrait le service et affamerait tous les mots suivants. Avec, le
    /// balayage repasse par tous les mots en au plus `MOTS` services.
    curseur_bas: AtomicUsize,
    curseur_haut: AtomicUsize,
}

impl BandeFile {
    pub const fn neuve() -> Self {
        Self {
            mots: [const { AtomicU64::new(0) }; MOTS],
            resume: AtomicU64::new(0),
            longueur: AtomicUsize::new(0),
            curseur_bas: AtomicUsize::new(0),
            curseur_haut: AtomicUsize::new(0),
        }
    }

    #[inline]
    pub fn longueur(&self) -> usize {
        self.longueur.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn est_vide(&self) -> bool {
        self.longueur() == 0
    }

    #[inline]
    pub fn contient(&self, emplacement: usize) -> bool {
        if emplacement >= EMPLACEMENTS {
            return false;
        }
        let (mot, bit) = position(emplacement);
        self.mots[mot].load(Ordering::Acquire) & bit != 0
    }

    /// Pose le bit. Rend `true` si l'entree est NOUVELLE.
    ///
    /// `false` n'est pas un echec : c'est un doublon, et la deduplication est
    /// precisement ce que ce chemin doit garantir sans parcourir la file.
    #[inline]
    pub fn insere(&self, emplacement: usize) -> bool {
        if emplacement >= EMPLACEMENTS {
            return false;
        }
        let (mot, bit) = position(emplacement);
        let avant = self.mots[mot].fetch_or(bit, Ordering::SeqCst);
        // Toujours poser le bit de resume, doublon ou non : un resume en trop
        // ne coute qu'un mot relu, un resume manquant perd une tache.
        self.resume.fetch_or(1u64 << mot, Ordering::SeqCst);
        if avant & bit != 0 {
            return false;
        }
        self.longueur.fetch_add(1, Ordering::Relaxed);
        true
    }

    /// Retire un emplacement precis. Rend `true` si c'est nous qui l'avons
    /// retire.
    #[inline]
    pub fn retire(&self, emplacement: usize) -> bool {
        if emplacement >= EMPLACEMENTS {
            return false;
        }
        let (mot, bit) = position(emplacement);
        let avant = self.mots[mot].fetch_and(!bit, Ordering::SeqCst);
        if avant & bit == 0 {
            return false;
        }
        self.longueur.fetch_sub(1, Ordering::Relaxed);
        self.rafraichit_resume(mot);
        true
    }

    /// Sert le plus petit emplacement a partir du curseur (tourniquet).
    pub fn retire_bas(&self) -> Option<usize> {
        self.sert(false)
    }

    /// Sert le plus grand emplacement (bout oppose).
    ///
    /// Le vol prend par ce bout : deux coeurs qui piochent aux deux extremites
    /// de la meme bande se disputent le meme bit seulement quand il n'en reste
    /// qu'un.
    pub fn retire_haut(&self) -> Option<usize> {
        self.sert(true)
    }

    fn sert(&self, par_le_haut: bool) -> Option<usize> {
        for _ in 0..TENTATIVES_MAX {
            if self.est_vide() {
                return None;
            }
            let Some(emplacement) = self.candidat(par_le_haut) else {
                // `longueur` non nulle et aucun bit trouve : soit un retrait
                // concurrent vient d'aboutir, soit le resume ET le balayage
                // ont croise une pose en cours. Reessayer, sans conclure.
                core::hint::spin_loop();
                continue;
            };
            if self.retire(emplacement) {
                return Some(emplacement);
            }
            // Un autre coeur a servi cette tache. Il y a eu du progres global ;
            // on recommence sur ce qui reste.
        }
        None
    }

    /// Un emplacement pose, s'il y en a un. Resume d'abord, balayage ensuite.
    fn candidat(&self, par_le_haut: bool) -> Option<usize> {
        let curseur = if par_le_haut { &self.curseur_haut } else { &self.curseur_bas };
        let depart = curseur.load(Ordering::Relaxed) % MOTS;
        let resume = self.resume.load(Ordering::SeqCst);

        if resume != 0 {
            for pas in 0..MOTS {
                let mot = mot_visite(depart, pas, par_le_haut);
                if resume & (1u64 << mot) == 0 {
                    continue;
                }
                let valeur = self.mots[mot].load(Ordering::Acquire);
                if valeur == 0 {
                    continue;
                }
                curseur.store(mot_suivant(mot, par_le_haut), Ordering::Relaxed);
                return Some(mot * 64 + bit_choisi(valeur, par_le_haut));
            }
        }

        // Balayage integral : `MOTS` lectures, independamment du nombre de
        // taches. C'est ce qui rend la correction independante du resume.
        for pas in 0..MOTS {
            let mot = mot_visite(depart, pas, par_le_haut);
            let valeur = self.mots[mot].load(Ordering::Acquire);
            if valeur == 0 {
                continue;
            }
            // Le resume avait tort : le corriger ici evite que le balayage
            // devienne le chemin normal.
            self.resume.fetch_or(1u64 << mot, Ordering::SeqCst);
            curseur.store(mot_suivant(mot, par_le_haut), Ordering::Relaxed);
            return Some(mot * 64 + bit_choisi(valeur, par_le_haut));
        }
        None
    }

    /// Efface le bit de resume d'un mot devenu vide, PUIS relit le mot.
    #[inline]
    fn rafraichit_resume(&self, mot: usize) {
        if self.mots[mot].load(Ordering::SeqCst) != 0 {
            return;
        }
        self.resume.fetch_and(!(1u64 << mot), Ordering::SeqCst);
        // Une pose concurrente publiee AVANT cet effacement doit survivre : en
        // ordre total `SeqCst`, si elle a eu lieu, cette relecture la voit.
        if self.mots[mot].load(Ordering::SeqCst) != 0 {
            self.resume.fetch_or(1u64 << mot, Ordering::SeqCst);
        }
    }
}

/// Le mot visite au `pas`-ieme cran du balayage, dans le sens demande.
#[inline]
const fn mot_visite(depart: usize, pas: usize, par_le_haut: bool) -> usize {
    if par_le_haut {
        (depart + MOTS - pas % MOTS) % MOTS
    } else {
        (depart + pas) % MOTS
    }
}

/// Ou reprendre au prochain service : le mot D'APRES celui qu'on vient de
/// servir. C'est ce qui empeche un mot qui ne se vide jamais de retenir le
/// service.
#[inline]
const fn mot_suivant(mot: usize, par_le_haut: bool) -> usize {
    if par_le_haut {
        (mot + MOTS - 1) % MOTS
    } else {
        (mot + 1) % MOTS
    }
}

#[inline]
const fn position(emplacement: usize) -> (usize, u64) {
    (emplacement / 64, 1u64 << (emplacement % 64))
}

#[inline]
const fn bit_choisi(valeur: u64, par_le_haut: bool) -> usize {
    if par_le_haut {
        63 - valeur.leading_zeros() as usize
    } else {
        valeur.trailing_zeros() as usize
    }
}

/// L'identite (emplacement + generation) associee a chaque emplacement en file.
///
/// Une case par emplacement pour toute la machine : un emplacement n'est en
/// file que dans une seule bande d'un seul CPU a la fois.
pub struct TableIdentites {
    mots: [AtomicU64; EMPLACEMENTS],
}

impl TableIdentites {
    pub const fn neuve() -> Self {
        Self { mots: [const { AtomicU64::new(0) }; EMPLACEMENTS] }
    }

    /// Publie l'identite AVANT que le bit ne soit pose.
    ///
    /// Ecrite meme quand le bit est deja pose : un emplacement recycle pendant
    /// qu'il etait en file porterait sinon l'identite de son occupant
    /// precedent, et la nouvelle tache ne serait jamais servie.
    #[inline]
    pub fn publie(&self, emplacement: usize, identite: u64) {
        if emplacement < EMPLACEMENTS {
            self.mots[emplacement].store(identite, Ordering::Release);
        }
    }

    #[inline]
    pub fn lit(&self, emplacement: usize) -> u64 {
        if emplacement < EMPLACEMENTS {
            self.mots[emplacement].load(Ordering::Acquire)
        } else {
            0
        }
    }
}

/// La file d'execution d'un CPU.
pub struct FileCpu {
    bandes: [BandeFile; NB_BANDES],
    /// Tours interactifs consecutifs deja servis.
    tours_interactifs: AtomicU32,
    enfilees: AtomicU64,
    doublons: AtomicU64,
    defilees: AtomicU64,
    volees: AtomicU64,
    /// Tours rendus a la bande normale par la borne anti-famine.
    anti_famine: AtomicU64,
}

impl FileCpu {
    pub const fn neuve() -> Self {
        Self {
            bandes: [const { BandeFile::neuve() }; NB_BANDES],
            tours_interactifs: AtomicU32::new(0),
            enfilees: AtomicU64::new(0),
            doublons: AtomicU64::new(0),
            defilees: AtomicU64::new(0),
            volees: AtomicU64::new(0),
            anti_famine: AtomicU64::new(0),
        }
    }

    #[inline]
    pub fn bande(&self, bande: Bande) -> &BandeFile {
        &self.bandes[bande.indice()]
    }

    /// Met un emplacement en file dans une bande. `true` si l'entree est neuve.
    ///
    /// Un emplacement deja present dans l'AUTRE bande y est d'abord retire :
    /// une tache dont la priorite change ne doit pas exister en double.
    pub fn enfile(&self, emplacement: usize, bande: Bande) -> bool {
        let autre = Bande::depuis_indice(1 - bande.indice());
        if self.bandes[autre.indice()].contient(emplacement) {
            self.bandes[autre.indice()].retire(emplacement);
        }
        let neuve = self.bandes[bande.indice()].insere(emplacement);
        if neuve {
            self.enfilees.fetch_add(1, Ordering::Relaxed);
        } else {
            self.doublons.fetch_add(1, Ordering::Relaxed);
        }
        neuve
    }

    /// Elit un emplacement : interactive d'abord, normale garantie.
    pub fn defile(&self) -> Option<usize> {
        let tours = self.tours_interactifs.load(Ordering::Relaxed);
        let normale_dabord =
            tours >= TOURS_INTERACTIFS_MAX && !self.bandes[1].est_vide();

        let ordre = if normale_dabord { [1usize, 0] } else { [0usize, 1] };
        for indice in ordre {
            let Some(emplacement) = self.bandes[indice].retire_bas() else {
                continue;
            };
            if indice == 0 {
                self.tours_interactifs.fetch_add(1, Ordering::Relaxed);
            } else {
                if normale_dabord {
                    self.anti_famine.fetch_add(1, Ordering::Relaxed);
                }
                self.tours_interactifs.store(0, Ordering::Relaxed);
            }
            self.defilees.fetch_add(1, Ordering::Relaxed);
            return Some(emplacement);
        }
        None
    }

    /// Prend du travail a voler : la bande NORMALE d'abord.
    ///
    /// Voler une tache interactive lui coute la migration -- cache froid, et
    /// une residence qui recommence -- au moment precis ou elle doit repondre.
    /// Le travail de fond est ce qui se deplace le mieux.
    pub fn vole(&self) -> Option<usize> {
        for indice in [1usize, 0] {
            if let Some(emplacement) = self.bandes[indice].retire_haut() {
                self.volees.fetch_add(1, Ordering::Relaxed);
                return Some(emplacement);
            }
        }
        None
    }

    /// Retire un emplacement de la file, quelle que soit sa bande.
    pub fn retire(&self, emplacement: usize) -> bool {
        self.bandes[0].retire(emplacement) || self.bandes[1].retire(emplacement)
    }

    #[inline]
    pub fn contient(&self, emplacement: usize) -> bool {
        self.bandes[0].contient(emplacement) || self.bandes[1].contient(emplacement)
    }

    /// Longueur totale. Deux lectures atomiques, AUCUN verrou.
    #[inline]
    pub fn longueur(&self) -> usize {
        self.bandes[0].longueur() + self.bandes[1].longueur()
    }

    /// Ce qu'un voleur peut esperer prendre sans nuire a l'interactivite.
    #[inline]
    pub fn pression_volable(&self) -> usize {
        self.bandes[1].longueur()
    }

    #[inline]
    pub fn est_vide(&self) -> bool {
        self.longueur() == 0
    }

    pub fn compteurs(&self) -> CompteursFile {
        CompteursFile {
            interactives: self.bandes[0].longueur() as u64,
            normales: self.bandes[1].longueur() as u64,
            enfilees: self.enfilees.load(Ordering::Relaxed),
            doublons: self.doublons.load(Ordering::Relaxed),
            defilees: self.defilees.load(Ordering::Relaxed),
            volees: self.volees.load(Ordering::Relaxed),
            anti_famine: self.anti_famine.load(Ordering::Relaxed),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompteursFile {
    pub interactives: u64,
    pub normales: u64,
    pub enfilees: u64,
    pub doublons: u64,
    pub defilees: u64,
    pub volees: u64,
    pub anti_famine: u64,
}

/// L'unique table d'identites de la machine.
static IDENTITES: TableIdentites = TableIdentites::neuve();

/// L'identite publiee pour cet emplacement, ou zero.
#[inline]
pub fn identite_en_file(emplacement: usize) -> u64 {
    IDENTITES.lit(emplacement)
}

/// Publie l'identite d'un emplacement avant sa mise en file.
#[inline]
pub fn publie_identite(emplacement: usize, identite: u64) {
    IDENTITES.publie(emplacement, identite);
}
