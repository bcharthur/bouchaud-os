//! Ce que TCP doit savoir : quel octet est parti sans avoir ete acquitte, et
//! quand le renvoyer.
//!
//! # Ce que la pile n'avait pas
//!
//! L'en-tete de `tcp.rs` le disait sans detour : « Pas de retransmission ni de
//! controle de congestion ». Concretement, un segment perdu n'etait JAMAIS
//! renvoye par nous. La connexion ne tenait que parce que SLIRP, en face,
//! retransmet pour son propre compte et parce qu'un `GET` tient dans un seul
//! segment. Sur un reseau qui perd, la requete disparaissait et la connexion
//! attendait son delai d'inactivite complet avant d'abandonner -- trente
//! secondes pour un paquet perdu.
//!
//! Et l'absence de mesure de RTT avait une consequence plus insidieuse :
//! l'attente etait un BUSY-POLL de huit secondes, choisi parce qu'on ne savait
//! pas combien de temps attendre. Un coeur entier occupe a interroger l'anneau,
//! sur un systeme qui compte ses cycles.
//!
//! # Ce que ce module etablit
//!
//!   * une file d'emission bornee des segments ENVOYES ET NON ACQUITTES ;
//!   * la mesure du RTT, avec l'algorithme de Karn -- un segment retransmis ne
//!     donne jamais d'echantillon, parce qu'on ne sait pas laquelle des deux
//!     copies a ete acquittee, et se tromper divise le RTO par deux a chaque
//!     perte ;
//!   * le calcul du RTO de la RFC 6298 : `SRTT`, `RTTVAR`, plancher, plafond,
//!     et doublement a chaque expiration ;
//!   * le comptage des ACK dupliques, donc la RETRANSMISSION RAPIDE : trois
//!     doublons valent une perte, et attendre le RTO complet pour s'en rendre
//!     compte est ce qui fait passer un transfert de une seconde a trente.
//!
//! # Ce que ce module ne fait pas
//!
//! Pas de controle de congestion. `cwnd` et `ssthresh` demandent de decider
//! quand REDUIRE le debit, et cette decision se prend sur des mesures qu'on n'a
//! pas encore. La file d'emission est ce qui rend cette suite possible ; la
//! faire sans elle serait un reglage, pas un algorithme.
//!
//! # Sans allocation
//!
//! La file est un tableau de taille fixe. Une pile reseau qui alloue par
//! paquet alloue depuis un gestionnaire d'interruption, et une allocation qui
//! descend dans le backing global y coute plus cher que le paquet ne rapporte.

use core::sync::atomic::{AtomicU64, Ordering};

/// Segments en vol suivis simultanement.
///
/// Borne la memoire ET la fenetre d'emission : au-dela, l'emetteur attend un
/// acquittement plutot que de gonfler une file que personne ne draine.
pub const SEGMENTS_MAX: usize = 64;

/// Octets d'un segment retenus pour une eventuelle retransmission.
pub const CHARGE_MAX: usize = 1460;

/// RTO plancher, en millisecondes (RFC 6298 recommande 1 s ; 200 ms convient a
/// un lien local ou SLIRP, ou attendre une seconde est une eternite).
pub const RTO_MIN_MS: u64 = 200;
/// RTO plafond. Au-dela, la connexion est morte, pas lente.
pub const RTO_MAX_MS: u64 = 60_000;
/// RTO initial, avant tout echantillon de RTT (RFC 6298, section 2.1).
pub const RTO_INITIAL_MS: u64 = 1_000;
/// ACK dupliques valant une perte.
pub const DOUBLONS_POUR_RETRANSMISSION: u32 = 3;
/// Retransmissions consecutives d'un meme segment avant abandon.
pub const RETRANSMISSIONS_MAX: u32 = 8;

/// Un segment envoye et pas encore acquitte.
#[derive(Clone, Copy)]
pub struct SegmentEnVol {
    pub seq: u32,
    pub longueur: u16,
    /// Consomme un numero de sequence sans porter d'octet (SYN, FIN).
    pub controle: bool,
    /// Instant d'emission, en millisecondes.
    pub envoye_ms: u64,
    /// Prochaine expiration.
    pub echeance_ms: u64,
    pub retransmissions: u32,
    pub occupe: bool,
    pub octets: [u8; CHARGE_MAX],
}

impl SegmentEnVol {
    const fn vide() -> Self {
        Self {
            seq: 0,
            longueur: 0,
            controle: false,
            envoye_ms: 0,
            echeance_ms: 0,
            retransmissions: 0,
            occupe: false,
            octets: [0u8; CHARGE_MAX],
        }
    }

    /// Numeros de sequence consommes par ce segment.
    #[inline]
    pub const fn consomme(&self) -> u32 {
        if self.controle { self.longueur as u32 + 1 } else { self.longueur as u32 }
    }

    /// Premier numero de sequence APRES ce segment.
    #[inline]
    pub const fn fin(&self) -> u32 {
        self.seq.wrapping_add(self.consomme())
    }

    pub fn charge(&self) -> &[u8] {
        &self.octets[..self.longueur as usize]
    }
}

/// Ce qu'une expiration demande a l'appelant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expiration {
    /// Rien a faire pour l'instant.
    Rien,
    /// Renvoyer ce segment (indice dans la file).
    Retransmettre(usize),
    /// Ce segment a ete renvoye trop de fois : la connexion est perdue.
    Abandon(usize),
}

/// L'etat d'emission d'une connexion.
pub struct Emission {
    file: [SegmentEnVol; SEGMENTS_MAX],
    /// Plus petit numero de sequence non acquitte.
    pub snd_una: u32,
    /// Prochain numero de sequence a emettre.
    pub snd_nxt: u32,

    /// RTT lisse, en millisecondes. Zero = aucun echantillon.
    srtt_ms: u64,
    /// Variation du RTT.
    rttvar_ms: u64,
    /// Delai de retransmission courant.
    rto_ms: u64,

    /// Dernier ACK recu, pour compter les doublons.
    dernier_ack: u32,
    doublons: u32,

    // --- mesures ---
    pub segments_emis: u64,
    pub segments_retransmis: u64,
    pub retransmissions_rapides: u64,
    pub expirations: u64,
    pub echantillons_rtt: u64,
    pub echantillons_ecartes_karn: u64,
    pub refus_file_pleine: u64,
}

impl Default for Emission {
    fn default() -> Self { Self::neuve(0) }
}

impl Emission {
    pub const fn neuve(isn: u32) -> Self {
        Self {
            file: [SegmentEnVol::vide(); SEGMENTS_MAX],
            snd_una: isn,
            snd_nxt: isn,
            srtt_ms: 0,
            rttvar_ms: 0,
            rto_ms: RTO_INITIAL_MS,
            dernier_ack: isn,
            doublons: 0,
            segments_emis: 0,
            segments_retransmis: 0,
            retransmissions_rapides: 0,
            expirations: 0,
            echantillons_rtt: 0,
            echantillons_ecartes_karn: 0,
            refus_file_pleine: 0,
        }
    }

    #[inline]
    pub const fn rto_ms(&self) -> u64 { self.rto_ms }
    #[inline]
    pub const fn srtt_ms(&self) -> u64 { self.srtt_ms }
    #[inline]
    pub const fn rttvar_ms(&self) -> u64 { self.rttvar_ms }

    /// Segments encore en vol.
    pub fn en_vol(&self) -> usize {
        self.file.iter().filter(|s| s.occupe).count()
    }

    pub fn vide(&self) -> bool { self.en_vol() == 0 }

    pub fn segment(&self, index: usize) -> Option<&SegmentEnVol> {
        self.file.get(index).filter(|s| s.occupe)
    }

    /// Enregistre un segment qui vient de partir.
    ///
    /// Rend l'indice dans la file, ou `None` si elle est pleine. Un `None` ne
    /// doit PAS etre traite comme un envoi reussi : le segment ne serait jamais
    /// retransmis, et la perte serait silencieuse.
    pub fn enregistre(
        &mut self,
        seq: u32,
        charge: &[u8],
        controle: bool,
        maintenant_ms: u64,
    ) -> Option<usize> {
        if charge.len() > CHARGE_MAX {
            return None;
        }
        let Some(index) = self.file.iter().position(|s| !s.occupe) else {
            self.refus_file_pleine += 1;
            return None;
        };
        let segment = &mut self.file[index];
        segment.seq = seq;
        segment.longueur = charge.len() as u16;
        segment.controle = controle;
        segment.envoye_ms = maintenant_ms;
        segment.echeance_ms = maintenant_ms.saturating_add(self.rto_ms);
        segment.retransmissions = 0;
        segment.occupe = true;
        segment.octets[..charge.len()].copy_from_slice(charge);
        let fin = segment.fin();
        if seq_apres_ou_egal(fin, self.snd_nxt) {
            self.snd_nxt = fin;
        }
        self.segments_emis += 1;
        Some(index)
    }

    /// Traite un acquittement cumulatif.
    ///
    /// Rend `true` si l'ACK a fait avancer la fenetre. Un ACK qui n'avance pas
    /// ET qui repete le precedent est un DOUBLON, et trois doublons valent une
    /// perte : c'est ce que `retransmission_rapide` rendra alors.
    pub fn acquitte(&mut self, ack: u32, maintenant_ms: u64) -> bool {
        // Un ACK qui remonte dans le passe vient d'un segment reordonne ou
        // d'un pair confus. Le suivre ferait reculer la fenetre.
        if seq_avant(ack, self.snd_una) {
            return false;
        }
        if ack == self.dernier_ack && ack == self.snd_una {
            self.doublons += 1;
            return false;
        }

        let avance = seq_avant(self.snd_una, ack);
        self.dernier_ack = ack;
        if !avance {
            return false;
        }
        self.doublons = 0;
        self.snd_una = ack;

        for index in 0..SEGMENTS_MAX {
            let segment = self.file[index];
            if !segment.occupe {
                continue;
            }
            if !seq_apres_ou_egal(ack, segment.fin()) {
                continue;
            }
            // ALGORITHME DE KARN : un segment retransmis ne donne aucun
            // echantillon. On ne sait pas laquelle des deux copies a ete
            // acquittee ; prendre la plus recente diviserait le RTO par deux a
            // chaque perte, exactement quand il faut l'augmenter.
            if segment.retransmissions == 0 {
                self.echantillon_rtt(maintenant_ms.saturating_sub(segment.envoye_ms));
            } else {
                self.echantillons_ecartes_karn += 1;
            }
            self.file[index].occupe = false;
        }
        true
    }

    /// Les ACK dupliques accumules depuis le dernier progres.
    #[inline]
    pub const fn doublons(&self) -> u32 { self.doublons }

    /// Faut-il retransmettre sans attendre le RTO ?
    ///
    /// Trois ACK dupliques disent que le pair recoit des segments mais qu'il
    /// lui manque celui qu'on attend. Attendre le RTO complet pour s'en rendre
    /// compte est ce qui fait passer un transfert d'une seconde a trente.
    pub fn retransmission_rapide(&mut self) -> Option<usize> {
        if self.doublons < DOUBLONS_POUR_RETRANSMISSION {
            return None;
        }
        let cible = self.snd_una;
        let index = self
            .file
            .iter()
            .position(|s| s.occupe && s.seq == cible)?;
        self.doublons = 0;
        self.retransmissions_rapides += 1;
        Some(index)
    }

    /// Un segment est-il expire ?
    ///
    /// Rend le PLUS ANCIEN expire : retransmettre dans le desordre ferait
    /// compter au pair des doublons qui n'en sont pas.
    pub fn expire(&mut self, maintenant_ms: u64) -> Expiration {
        let mut choisi: Option<usize> = None;
        let mut plus_ancienne = u64::MAX;
        for (index, segment) in self.file.iter().enumerate() {
            if !segment.occupe || maintenant_ms < segment.echeance_ms {
                continue;
            }
            if segment.echeance_ms < plus_ancienne {
                plus_ancienne = segment.echeance_ms;
                choisi = Some(index);
            }
        }
        let Some(index) = choisi else { return Expiration::Rien };
        if self.file[index].retransmissions >= RETRANSMISSIONS_MAX {
            return Expiration::Abandon(index);
        }
        Expiration::Retransmettre(index)
    }

    /// Note qu'un segment vient d'etre renvoye.
    ///
    /// Double le RTO -- le repli exponentiel de la RFC 6298 -- et rearme
    /// l'echeance. Le doublement est ce qui empeche une pile de participer a un
    /// effondrement de congestion en insistant.
    pub fn note_retransmission(&mut self, index: usize, maintenant_ms: u64) {
        if index >= SEGMENTS_MAX || !self.file[index].occupe {
            return;
        }
        self.rto_ms = (self.rto_ms.saturating_mul(2)).min(RTO_MAX_MS);
        let segment = &mut self.file[index];
        segment.retransmissions += 1;
        segment.echeance_ms = maintenant_ms.saturating_add(self.rto_ms);
        self.segments_retransmis += 1;
        self.expirations += 1;
    }

    /// Un echantillon de RTT. RFC 6298, section 2.
    fn echantillon_rtt(&mut self, mesure_ms: u64) {
        let mesure = mesure_ms.max(1);
        if self.srtt_ms == 0 {
            self.srtt_ms = mesure;
            self.rttvar_ms = mesure / 2;
        } else {
            // RTTVAR = 3/4 RTTVAR + 1/4 |SRTT - R|
            let ecart = self.srtt_ms.abs_diff(mesure);
            self.rttvar_ms = (self.rttvar_ms * 3 + ecart) / 4;
            // SRTT = 7/8 SRTT + 1/8 R
            self.srtt_ms = (self.srtt_ms * 7 + mesure) / 8;
        }
        self.echantillons_rtt += 1;
        self.recalcule_rto();
    }

    fn recalcule_rto(&mut self) {
        // RTO = SRTT + max(G, 4 * RTTVAR), borne.
        let rto = self.srtt_ms.saturating_add(self.rttvar_ms.saturating_mul(4).max(1));
        self.rto_ms = rto.clamp(RTO_MIN_MS, RTO_MAX_MS);
    }

    /// Combien de temps attendre avant la prochaine echeance.
    ///
    /// C'est ce qui remplace le busy-poll a duree fixe : on dort exactement
    /// jusqu'a ce qu'il y ait quelque chose a faire, et pas huit secondes
    /// choisies parce qu'on ne savait pas.
    pub fn attente_ms(&self, maintenant_ms: u64) -> Option<u64> {
        self.file
            .iter()
            .filter(|s| s.occupe)
            .map(|s| s.echeance_ms.saturating_sub(maintenant_ms))
            .min()
    }
}

/// `a` est-il strictement avant `b` dans l'espace circulaire des numeros de
/// sequence ?
///
/// La comparaison signee sur la difference est la seule correcte : les numeros
/// bouclent a 2^32, et `a < b` en non signe se trompe des qu'une connexion
/// traverse ce point.
#[inline]
pub fn seq_avant(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) < 0
}

#[inline]
pub fn seq_apres_ou_egal(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) >= 0
}

// --- Compteurs globaux -------------------------------------------------------

static TCP_RETRANSMISSIONS: AtomicU64 = AtomicU64::new(0);
static TCP_RETRANSMISSIONS_RAPIDES: AtomicU64 = AtomicU64::new(0);
static TCP_ECHANTILLONS_RTT: AtomicU64 = AtomicU64::new(0);
static TCP_SRTT_DERNIER_MS: AtomicU64 = AtomicU64::new(0);
static TCP_RTO_DERNIER_MS: AtomicU64 = AtomicU64::new(0);
/// Millisecondes passees a interroger l'anneau sans dormir.
///
/// C'est le chiffre que le chantier 9 doit faire baisser. Il n'existait pas :
/// le busy-poll etait une constante de huit secondes dans une boucle, et rien
/// ne disait combien de temps on y restait reellement.
static TCP_BUSY_POLL_MS: AtomicU64 = AtomicU64::new(0);
static TCP_ATTENTES_DORMIES: AtomicU64 = AtomicU64::new(0);

pub fn note_connexion(emission: &Emission, busy_poll_ms: u64, attentes_dormies: u64) {
    TCP_RETRANSMISSIONS.fetch_add(emission.segments_retransmis, Ordering::Relaxed);
    TCP_RETRANSMISSIONS_RAPIDES
        .fetch_add(emission.retransmissions_rapides, Ordering::Relaxed);
    TCP_ECHANTILLONS_RTT.fetch_add(emission.echantillons_rtt, Ordering::Relaxed);
    TCP_SRTT_DERNIER_MS.store(emission.srtt_ms, Ordering::Relaxed);
    TCP_RTO_DERNIER_MS.store(emission.rto_ms, Ordering::Relaxed);
    TCP_BUSY_POLL_MS.fetch_add(busy_poll_ms, Ordering::Relaxed);
    TCP_ATTENTES_DORMIES.fetch_add(attentes_dormies, Ordering::Relaxed);
}

/// retransmissions, rapides, echantillons RTT, SRTT, RTO, busy-poll ms, sommeils
pub fn stats() -> (u64, u64, u64, u64, u64, u64, u64) {
    (
        TCP_RETRANSMISSIONS.load(Ordering::Relaxed),
        TCP_RETRANSMISSIONS_RAPIDES.load(Ordering::Relaxed),
        TCP_ECHANTILLONS_RTT.load(Ordering::Relaxed),
        TCP_SRTT_DERNIER_MS.load(Ordering::Relaxed),
        TCP_RTO_DERNIER_MS.load(Ordering::Relaxed),
        TCP_BUSY_POLL_MS.load(Ordering::Relaxed),
        TCP_ATTENTES_DORMIES.load(Ordering::Relaxed),
    )
}
