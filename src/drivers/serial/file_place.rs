//! BOUCHAUD_CONSOLE_CONTRE_PRESSION_V1 -- la file des producteurs qui
//! attendent de la place dans l'anneau de la console asynchrone.
//!
//! KVM SMP4, flot qui sature le port (run 39602362503, 3adcea3c) : 586 lignes
//! intactes sur 720, 135 perdues. Reproduit sous TCG (port ralenti a
//! 5 us/octet) : 494/720, 323 perdues, AUCUN repli. Un producteur qui trouvait
//! l'anneau plein vidait lui-meme un morceau puis retentait -- mais la place
//! qu'il liberait etait prise par un autre producteur entre-temps. Au bout
//! d'une seconde il renoncait et rangeait sa ligne de 1 800 octets dans
//! l'anneau de son coeur (4 Kio : deux lignes), puis la perdait.
//!
//! Desormais un producteur qui PEUT CEDER prend un ticket ; la place liberee
//! revient au ticket de tete, dans l'ordre d'arrivee. Pas d'echeance tant que
//! la console avance (le fil, ou la tete qui aide a vider) : la ligne attend,
//! elle ne se perd pas. Elle ne sort de la file sans etre rangee que si la
//! console cesse d'etre asynchrone (repli, arret, panique) -- le chemin
//! synchrone l'emet alors.
//!
//! Invariants (tools/serie/test_file_place_property.rs) :
//!   * `servi <= prochain` : on ne sert jamais un ticket non distribue ;
//!   * `servi` ne recule jamais ;
//!   * toute sortie -- rangee, abandon -- avance `servi` au-dela de son
//!     ticket : un ticket servi ne bloque jamais les suivants ;
//!   * sans saut, les tickets sont servis dans l'ordre, un seul a la fois ;
//!   * un saut (tete figee) ne depasse jamais le ticket de celui qui saute.
//!
//! Module pur (atomiques seulement).

use core::sync::atomic::{AtomicU64, Ordering};

/// Un enregistrement de `total` octets (plus ses 2 octets de longueur) tient-il
/// dans `libre` en laissant `reserve` octets libres ? La reserve est celle des
/// contextes qui ne peuvent pas attendre (interruption, verrou tenu, fil
/// noyau) : un producteur qui peut ceder ne la consomme jamais.
pub fn tient(libre: usize, total: usize, reserve: usize) -> bool {
    libre >= total.saturating_add(2).saturating_add(reserve)
}

pub struct FilePlace {
    /// Prochain ticket distribue.
    prochain: AtomicU64,
    /// Le plus petit ticket encore servable : `t` a son tour des que
    /// `servi >= t`.
    servi: AtomicU64,
}

impl FilePlace {
    pub const fn neuve() -> Self {
        Self { prochain: AtomicU64::new(0), servi: AtomicU64::new(0) }
    }

    /// Personne n'attend : un producteur peut ranger directement, sans
    /// doubler personne.
    pub fn vide(&self) -> bool {
        self.servi.load(Ordering::Acquire) >= self.prochain.load(Ordering::Acquire)
    }

    /// Un ticket, dans l'ordre d'arrivee.
    pub fn prend(&self) -> u64 {
        self.prochain.fetch_add(1, Ordering::AcqRel)
    }

    pub fn servi(&self) -> u64 {
        self.servi.load(Ordering::Acquire)
    }

    pub fn a_son_tour(&self, ticket: u64) -> bool {
        self.servi() >= ticket
    }

    /// Le ticket sort de la file (ligne rangee, ou console devenue
    /// synchrone) : les suivants avancent. Jamais de recul.
    pub fn sort(&self, ticket: u64) {
        self.servi.fetch_max(ticket.saturating_add(1), Ordering::AcqRel);
    }

    /// La tete `vu` n'a pas avance depuis trop longtemps (son producteur a
    /// disparu sans sortir) : `moi`, plus loin dans la file, la passe. Rend
    /// `true` si c'est lui qui a saute. Jamais au-dela de son propre ticket :
    /// `servi` reste un ticket distribue.
    pub fn saute(&self, vu: u64, moi: u64) -> bool {
        vu < moi
            && self
                .servi
                .compare_exchange(vu, vu + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
    }
}
