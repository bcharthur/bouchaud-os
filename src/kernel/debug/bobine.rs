//! Le tambour RAM de l'enregistreur de vol.
//!
//! # Le defaut, releve sur la TRIGKEY
//!
//! `blackbox::append()` ecrivait PHYSIQUEMENT un enregistrement de quatre
//! kibioctets sur la cle USB a chaque appel : prise du verrou du pilote xHCI,
//! trois transferts Bulk synchrones, deux attentes d'evenement. Ce chemin est
//! celui du DIAGNOSTIC, et il passait par le peripherique meme dont on
//! cherchait a comprendre la defaillance.
//!
//! Les trois archives physiques disent la meme chose : l'enregistrement
//! s'arrete entre 7,3 s et 8,2 s, exactement quand les services du navigateur
//! commencent a lire la cle d'amorcage. Aucune des causes candidates n'a
//! survecu a la mesure -- tambour plein, echec d'ecriture, famine du verrou,
//! support retire, ecrasement, extracteur : toutes eliminees, chiffres a
//! l'appui. Ce qui restait etait la FORME du chemin : un diagnostic qui
//! depend du peripherique qu'il observe ne peut pas rapporter la panne de ce
//! peripherique.
//!
//! # Ce que ce module garantit
//!
//! Poser un enregistrement, c'est desormais : deux increments atomiques, une
//! copie memoire, une publication. Aucune allocation, aucun verrou, aucun
//! acces xHCI, aucune attente. Le support USB n'intervient plus qu'a
//! l'extinction volontaire, ou une panne du support ne coute que les octets
//! qu'on n'a pas pu poser -- plus jamais la trace elle-meme.
//!
//! # L'invariant qui rend la perte EXACTE
//!
//! Deux anneaux cohabitent : les descripteurs (taille fixe) et les octets
//! (taille variable). Un enregistrement disparait quand l'un OU l'autre est
//! rattrape par l'ecriture, et deux causes de perte demandent deux comptes --
//! dont l'un, celui des octets, ne se calcule pas sans parcourir l'anneau.
//!
//! `OCTETS = DESCRIPTEURS * PAYLOAD_MAX` supprime le second cas. Une charge
//! utile vaut au plus `PAYLOAD_MAX`, donc `DESCRIPTEURS` enregistrements
//! consecutifs occupent au plus `OCTETS` octets : l'anneau d'octets ne peut
//! JAMAIS laper avant l'anneau de descripteurs. La perte se reduit alors au
//! recyclage d'un descripteur, qui se compte exactement -- et se verifie
//! contre une formule fermee, ce que fait la suite hote.
//!
//! Module PUR : ni `use crate::`, ni `unsafe`, ni horloge. Il calcule des
//! positions ; l'appelant copie les octets.

use core::sync::atomic::{AtomicU16, AtomicU64, AtomicUsize, Ordering};

/// Charge utile maximale d'un enregistrement, en octets.
///
/// C'est la valeur du format persistant (4096 - 64 d'en-tete) : le tambour
/// RAM porte EXACTEMENT ce que le support portera, sans conversion.
pub const PAYLOAD_MAX: usize = 4032;

/// Nombre d'enregistrements que le tambour retient.
///
/// # Pourquoi huit mille et non deux mille
///
/// Le banc pose environ seize enregistrements par seconde sous charge : deux
/// mille couvraient deux minutes, et l'essai physique doit en durer
/// plusieurs. Au-dela de la capacite, le tambour ne cesse pas -- il ecrase
/// les plus anciens, et le compte le dit --, mais une session qui ecrase a
/// perdu son debut, c'est-a-dire l'amorcage.
///
/// Huit mille descripteurs donnent environ trente et un mebioctets de
/// tambour et de l'ordre de huit minutes de charge soutenue. C'est de la
/// memoire `.bss` : jamais allouee, jamais liberee, disponible avant le
/// premier `println!`.
pub const DESCRIPTEURS: usize = 8192;

/// Taille de l'anneau d'octets. Voir l'invariant ci-dessus : cette valeur
/// n'est pas un reglage, elle est DERIVEE, et la depasser vers le bas
/// reintroduirait une seconde cause de perte.
pub const OCTETS: usize = DESCRIPTEURS * PAYLOAD_MAX;

/// Une place reservee dans le tambour, avant que les octets n'y soient.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reservation {
    /// Numero logique de l'enregistrement, a partir de un.
    pub seq: u64,
    /// Descripteur qui le portera.
    pub index: usize,
    /// Position ABSOLUE du premier octet, avant repliement.
    pub debut: u64,
    /// Longueur de la charge utile.
    pub longueur: usize,
}

impl Reservation {
    /// Les deux tranches a copier : `(depart, premiere, seconde)`.
    ///
    /// Un enregistrement peut chevaucher la fin de l'anneau. Le refuser
    /// gaspillerait jusqu'a quatre kibioctets par tour et, surtout, ferait
    /// dependre la capacite reelle de l'alignement des tailles -- une capacite
    /// qu'on ne saurait plus annoncer.
    pub fn tranches(&self) -> (usize, usize, usize) {
        let depart = (self.debut % OCTETS as u64) as usize;
        let premiere = self.longueur.min(OCTETS - depart);
        (depart, premiere, self.longueur - premiere)
    }
}

/// Un enregistrement relu du tambour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Descripteur {
    pub seq: u64,
    pub genre: u16,
    pub ts_ns: u64,
    pub fin_trace: u64,
    pub debut: u64,
    pub longueur: usize,
}

impl Descripteur {
    /// Les deux tranches a relire, dans le meme ordre qu'a l'ecriture.
    pub fn tranches(&self) -> (usize, usize, usize) {
        let depart = (self.debut % OCTETS as u64) as usize;
        let premiere = self.longueur.min(OCTETS - depart);
        (depart, premiere, self.longueur - premiere)
    }
}

struct Case {
    /// Numero publie ici, zero tant qu'aucun. C'est la SEULE barriere : elle
    /// est posee en dernier a l'ecriture et lue en premier a la relecture.
    publie: AtomicU64,
    genre: AtomicU16,
    ts_ns: AtomicU64,
    fin_trace: AtomicU64,
    debut: AtomicU64,
    longueur: AtomicUsize,
}

impl Case {
    const fn neuve() -> Self {
        Self {
            publie: AtomicU64::new(0),
            genre: AtomicU16::new(0),
            ts_ns: AtomicU64::new(0),
            fin_trace: AtomicU64::new(0),
            debut: AtomicU64::new(0),
            longueur: AtomicUsize::new(0),
        }
    }
}

/// Etat du tambour, tel qu'un releve doit le montrer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Etat {
    /// Enregistrements reserves depuis le demarrage.
    pub reserves: u64,
    /// Enregistrements effectivement publies.
    pub poses: u64,
    /// Enregistrements dont le descripteur a ete RECYCLE. Ce n'est pas une
    /// perte en soi : un enregistrement deja sorti vers le support peut etre
    /// recycle sans que rien ne manque.
    pub ecrases: u64,
    /// Enregistrements recycles AVANT d'avoir ete sortis. Ceux-la sont
    /// perdus pour de bon, et zero est la seule valeur qui autorise a lire la
    /// trace comme un recit complet.
    ///
    /// # Pourquoi ce compteur a du etre separe d'`ecrases`
    ///
    /// Le vidage final convertit le journal serie et les evenements de vol en
    /// enregistrements, et les fait passer par le tambour. Ceux-ci recyclent
    /// les descripteurs des plus anciens -- qui sont DEJA sur le support,
    /// puisque le tambour est vide en premier. Le banc affichait donc
    /// `ecrases=223` sur une archive parfaitement complete : un chiffre
    /// alarmant qui ne designait aucune perte.
    pub perdus: u64,
    /// Charges utiles refusees parce que trop grandes. Un refus est un defaut
    /// de l'appelant, pas une perte du tambour : il se compte a part.
    pub refuses: u64,
    /// Enregistrements deja sortis vers le support.
    pub vidanges: u64,
    /// Octets occupes par les enregistrements encore lisibles.
    pub octets_vifs: u64,
}

/// Le tambour. Plusieurs producteurs, un consommateur.
pub struct Bobine {
    cases: [Case; DESCRIPTEURS],
    reserve: AtomicU64,
    curseur: AtomicU64,
    poses: AtomicU64,
    ecrases: AtomicU64,
    perdus: AtomicU64,
    refuses: AtomicU64,
    /// Dernier numero sorti vers le support. C'est lui qui separe un
    /// recyclage anodin d'une perte.
    vidange_jusqua: AtomicU64,
    vidanges: AtomicU64,
}

impl Bobine {
    pub const fn neuve() -> Self {
        Self {
            cases: [const { Case::neuve() }; DESCRIPTEURS],
            reserve: AtomicU64::new(0),
            curseur: AtomicU64::new(0),
            poses: AtomicU64::new(0),
            ecrases: AtomicU64::new(0),
            perdus: AtomicU64::new(0),
            refuses: AtomicU64::new(0),
            vidange_jusqua: AtomicU64::new(0),
            vidanges: AtomicU64::new(0),
        }
    }

    /// Reserve une place. Deux increments atomiques, rien d'autre.
    ///
    /// Le numero est consomme MEME si la publication n'a pas lieu : le
    /// tambour ne peut pas echouer autrement que par une charge utile trop
    /// grande, qui est refusee AVANT d'avoir rien consomme.
    pub fn reserve(&self, longueur: usize) -> Option<Reservation> {
        if longueur > PAYLOAD_MAX {
            self.refuses.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        let seq = self.reserve.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
        let debut = self.curseur.fetch_add(longueur as u64, Ordering::AcqRel);
        if seq > DESCRIPTEURS as u64 {
            // Le descripteur qu'on prend portait `seq - DESCRIPTEURS`. Cet
            // enregistrement-la n'existe plus en memoire.
            self.ecrases.fetch_add(1, Ordering::Relaxed);
            // MAIS IL N'EST PERDU QUE S'IL N'EST PAS DEJA SUR LE SUPPORT.
            //
            // Confondre les deux fait crier une archive complete : le vidage
            // final recycle par construction les descripteurs qu'il vient de
            // sortir.
            if seq - DESCRIPTEURS as u64 > self.vidange_jusqua.load(Ordering::Acquire) {
                self.perdus.fetch_add(1, Ordering::Relaxed);
            }
        }
        Some(Reservation {
            seq,
            index: ((seq - 1) % DESCRIPTEURS as u64) as usize,
            debut,
            longueur,
        })
    }

    /// Publie un enregistrement dont les octets sont DEJA copies.
    ///
    /// Appeler ceci avant la copie exposerait au lecteur des octets qui ne
    /// sont pas encore les siens, et rien ne le lui dirait.
    pub fn publie(&self, r: &Reservation, genre: u16, ts_ns: u64, fin_trace: u64) {
        let case = &self.cases[r.index];
        case.genre.store(genre, Ordering::Relaxed);
        case.ts_ns.store(ts_ns, Ordering::Relaxed);
        case.fin_trace.store(fin_trace, Ordering::Relaxed);
        case.debut.store(r.debut, Ordering::Relaxed);
        case.longueur.store(r.longueur, Ordering::Relaxed);
        case.publie.store(r.seq, Ordering::Release);
        self.poses.fetch_add(1, Ordering::Relaxed);
    }

    /// Relit l'enregistrement `seq`, s'il est encore la ET complet.
    ///
    /// La verification du numero est faite DEUX fois -- avant et apres avoir
    /// lu les champs. Entre les deux, un producteur a pu recycler le
    /// descripteur ; sans la seconde lecture, on rendrait un enregistrement
    /// dont les champs viennent de deux enregistrements differents.
    pub fn lis(&self, seq: u64) -> Option<Descripteur> {
        if seq == 0 || seq > self.reserve.load(Ordering::Acquire) {
            return None;
        }
        let index = ((seq - 1) % DESCRIPTEURS as u64) as usize;
        let case = &self.cases[index];
        if case.publie.load(Ordering::Acquire) != seq {
            return None;
        }
        let d = Descripteur {
            seq,
            genre: case.genre.load(Ordering::Relaxed),
            ts_ns: case.ts_ns.load(Ordering::Relaxed),
            fin_trace: case.fin_trace.load(Ordering::Relaxed),
            debut: case.debut.load(Ordering::Relaxed),
            longueur: case.longueur.load(Ordering::Relaxed),
        };
        if case.publie.load(Ordering::Acquire) != seq || d.longueur > PAYLOAD_MAX {
            return None;
        }
        Some(d)
    }

    /// Confirme que tout jusqu'a `jusqua` inclus a atteint le support.
    ///
    /// Le curseur ne RECULE jamais : deux vidages concurrents -- celui de
    /// l'extinction et celui d'un chemin fatal -- ne doivent pas pouvoir se
    /// contredire et transformer des enregistrements sauves en perdus.
    pub fn note_vidange(&self, combien: u64, jusqua: u64) {
        self.vidanges.fetch_add(combien, Ordering::Relaxed);
        self.vidange_jusqua.fetch_max(jusqua, Ordering::AcqRel);
    }

    /// Le plus petit numero qui puisse encore etre lisible.
    ///
    /// C'est une BORNE, pas une promesse : `lis` reste l'autorite, parce
    /// qu'un producteur peut recycler pendant le parcours.
    pub fn plus_ancien(&self) -> u64 {
        let dernier = self.reserve.load(Ordering::Acquire);
        dernier
            .saturating_sub(DESCRIPTEURS as u64)
            .saturating_add(1)
            .max(1)
    }

    /// Le plus grand numero reserve.
    pub fn dernier(&self) -> u64 {
        self.reserve.load(Ordering::Acquire)
    }

    pub fn etat(&self) -> Etat {
        let reserves = self.reserve.load(Ordering::Acquire);
        let ecrases = self.ecrases.load(Ordering::Relaxed);
        Etat {
            reserves,
            poses: self.poses.load(Ordering::Relaxed),
            ecrases,
            perdus: self.perdus.load(Ordering::Relaxed),
            refuses: self.refuses.load(Ordering::Relaxed),
            vidanges: self.vidanges.load(Ordering::Relaxed),
            octets_vifs: self.octets_vifs(),
        }
    }

    fn octets_vifs(&self) -> u64 {
        let fin = self.curseur.load(Ordering::Acquire);
        let plus_ancien = self.plus_ancien();
        let debut = self
            .lis(plus_ancien)
            .map(|d| d.debut)
            .unwrap_or(fin.saturating_sub(OCTETS as u64));
        fin.saturating_sub(debut).min(OCTETS as u64)
    }
}

/// Ce que la formule fermee predit comme nombre d'ecrasements.
///
/// Elle existe pour etre CONFRONTEE au compteur : deux facons de compter la
/// meme chose, dont une seule traverse le chemin chaud. Si elles divergent,
/// c'est le chemin chaud qui a tort, et la suite hote le dit.
pub fn ecrases_attendus(reserves: u64) -> u64 {
    reserves.saturating_sub(DESCRIPTEURS as u64)
}
