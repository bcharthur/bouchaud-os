//! Jusqu'ou un appelant attend le jeton serie (BOUCHAUD_SERIE_ATTENTE_PROGRES_V1).
//!
//! # Le defaut
//!
//! os-primitives `748322af`, KVM SMP4 (run 38603003639) : 719 lignes intactes
//! sur 720, `emissions_a_la_borne=1`. Six ecrivains de lignes de 1 800 octets
//! (environ 5 ms chacune sous KVM, une sortie de VM par octet) et cent vingt
//! processus fautifs dont le noyau journalise la mort. Un appelant a attendu
//! le jeton plus d'une seconde, puis a emis SANS lui, au milieu de la ligne
//! d'un autre : `SERIE_LIGNE e=3 n=0066 ...` coupee avant son `FIN_0066`.
//!
//! La borne d'une seconde portait sur l'attente TOTALE de l'appelant. Or elle
//! n'existe que pour ne pas rester pris derriere un detenteur qui ne rendra
//! jamais le jeton. Un appelant qui fait la queue derriere des emetteurs qui
//! PROGRESSENT -- ou dont le processeur virtuel a ete retire par l'hote le
//! temps que d'autres emettent -- n'est pas dans ce cas : l'emission sans
//! jeton y ecrivait une ligne entrelacee pour rien.
//!
//! # La regle
//!
//! Pour un appelant INTERRUPTIBLE (il ne tient rien, il peut attendre) :
//!   * la borne d'une seconde est une borne SANS PROGRES : chaque emission
//!     faite par un autre depuis la derniere echeance la renouvelle ;
//!   * un plafond ABSOLU (dix secondes) garde l'attente finie quoi qu'il
//!     arrive.
//! Les contextes masques (interruptions, panique) gardent leur borne courte :
//! ils ne peuvent pas attendre.
//!
//! Module pur : `tools/serie/test_attente_jeton.rs`.

/// Une seconde sans qu'aucune ligne ne sorte : le detenteur est perdu.
pub const BORNE_SANS_PROGRES_NS: u64 = 1_000_000_000;
/// Quoi qu'il arrive, l'attente d'un appelant interruptible finit.
pub const PLAFOND_ABSOLU_NS: u64 = 10_000_000_000;

/// Pourquoi un appelant renonce au jeton et emet sans lui.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Renonce {
    /// Aucune emission depuis une seconde : detenteur perdu.
    SansProgres,
    /// L'attente a atteint le plafond absolu malgre des emissions.
    Plafond,
}

/// Ce que fait l'appelant quand son echeance tombe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Echeance {
    /// D'autres ont emis : nouvelle echeance, l'appelant refait la queue.
    Renouvelee { echeance_ns: u64, emissions_vues: u64 },
    /// L'appelant emet sans le jeton (compte, et la cause est gardee).
    Renonce(Renonce),
}

/// L'echeance d'un appelant interruptible est passee : la renouveler ou
/// renoncer. `debut_ns` : l'entree dans l'attente ; `emissions_vues` : le
/// compteur d'emissions a la derniere echeance ; `emissions` : le compteur
/// maintenant. Pure.
pub fn a_l_echeance(maintenant_ns: u64, debut_ns: u64, emissions_vues: u64, emissions: u64) -> Echeance {
    if maintenant_ns.saturating_sub(debut_ns) >= PLAFOND_ABSOLU_NS {
        return Echeance::Renonce(Renonce::Plafond);
    }
    if emissions == emissions_vues {
        return Echeance::Renonce(Renonce::SansProgres);
    }
    Echeance::Renouvelee {
        echeance_ns: maintenant_ns.saturating_add(BORNE_SANS_PROGRES_NS),
        emissions_vues: emissions,
    }
}

/// BOUCHAUD_SERIE_JAMAIS_SANS_JETON_V1 -- ce que fait un appelant qui n'a pas
/// obtenu le jeton : contexte masque a sa borne courte, appelant
/// interruptible qui renonce (`a_l_echeance`), reentrance sur le coeur qui
/// tient deja le jeton.
///
/// os-primitives 1413289b, KVM SMP4 (run 38608429652) : 717 lignes sur 720,
/// `emissions_a_la_borne=3` -- la borne renouvelable de V1 ne couvrait pas
/// les contextes MASQUES (le noyau qui journalise une faute de processus),
/// dont la borne courte (quelques millisecondes) tombe sous la duree d'une
/// ligne de 1 800 octets sous KVM. Emettre sans le jeton a la borne, c'etait
/// casser la ligne d'un autre pour sauver la sienne.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repli {
    /// Fonctionnement normal : l'integrite des lignes prime. La ligne est
    /// rangee ENTIERE dans l'anneau du coeur (`differees`) ; le prochain
    /// detenteur du jeton vide les anneaux avant sa propre ligne. Un anneau
    /// plein perd la ligne -- comptee, jamais entrelacee.
    Differe,
    /// Panique : la disponibilite du journal prime. Le detenteur est peut-
    /// etre mort ; la ligne sort sans le jeton (comptee).
    EmetSansJeton,
}

/// Le repli d'un appelant sans jeton. Pure.
pub fn repli(panique: bool) -> Repli {
    if panique {
        Repli::EmetSansJeton
    } else {
        Repli::Differe
    }
}
