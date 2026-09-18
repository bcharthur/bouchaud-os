//! QUATRE FAITS DISTINCTS SUR UNE CARTE RESEAU, et non un booleen.
//!
//! # Le defaut, releve en photo le 18 septembre 2026
//!
//! Apres une reprise de degre quatre, l'ecran de la TRIGKEY affichait :
//!
//! ```text
//! net.nic.e1000     Erreur          carte non pilotee
//! net.nic.rtl8168   Indisponible    absente de ce materiel
//! ```
//!
//! Les deux lignes etaient fausses, et de deux facons differentes :
//!
//! - le RTL8168 est SOUDE sur cette machine. Il ne peut pas etre « absent ».
//!   Il venait d'etre desactive par notre propre reprise ;
//! - la e1000 n'a jamais ete presente sur cette machine. Elle ne peut pas
//!   etre « en erreur, carte non pilotee ».
//!
//! La cause tient en une ligne : le modele de carte etait choisi d'apres
//! `rtl8168::is_ready()`. Un pilote qui se desactive faisait donc disparaitre
//! le materiel du rapport, et apparaitre a sa place une carte qui n'existe
//! pas. Un seul booleen portait quatre questions :
//!
//! ```text
//!   la puce est-elle sur le bus PCI ?        <- le materiel
//!   un pilote lui est-il attache ?           <- le logiciel
//!   ce pilote est-il en etat de servir ?     <- l'etat courant
//!   le cable porte-t-il ?                    <- le lien
//! ```
//!
//! Les quatre sont independantes. Une carte presente peut n'avoir aucun
//! pilote ; un pilote attache peut etre en cours de reprise ; un pilote pret
//! peut avoir le cable debranche. Les confondre a produit un rapport qui
//! designait le mauvais composant au pire moment.
//!
//! Ce module est PUR : aucun `use crate::`, aucun `unsafe`, aucun acces
//! materiel. Il ne decrit que la machine a etats et ce qu'elle autorise.

/// L'etat d'un pilote de carte reseau.
///
/// L'ordre n'est pas anodin : il va de « rien » a « en service », et la
/// reprise n'est PAS un retour en arriere vers l'absence.
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
#[repr(u8)]
pub enum Etat {
    /// La puce n'est pas sur le bus. Rien a piloter, et ce n'est pas un
    /// defaut : la e1000 de QEMU n'existe pas sur la TRIGKEY.
    Absent = 0,
    /// La puce est sur le bus, aucun pilote ne s'y est attache.
    Detecte = 1,
    /// Un pilote s'y est attache : identite PCI retenue, anneaux alloues.
    Lie = 2,
    /// Le pilote sert : les moteurs tournent.
    Pret = 3,
    /// Une reprise est en cours. LE PILOTE RESTE ATTACHE.
    Reprise = 4,
    /// La reprise a echoue. La puce est toujours la, le pilote toujours
    /// attache -- il ne rend simplement plus de service.
    ///
    /// JAMAIS `Absent` : une carte soudee ne disparait pas parce que notre
    /// code a renonce.
    Echec = 5,
}

impl Etat {
    pub fn nom(self) -> &'static str {
        match self {
            Etat::Absent => "absent",
            Etat::Detecte => "detecte",
            Etat::Lie => "lie",
            Etat::Pret => "pret",
            Etat::Reprise => "reprise",
            Etat::Echec => "echec",
        }
    }

    /// La puce est-elle physiquement la ?
    ///
    /// Vraie des qu'on l'a vue sur le bus, et elle ne redevient jamais
    /// fausse : une carte soudee ne s'en va pas.
    pub fn presente(self) -> bool {
        self != Etat::Absent
    }

    /// Un pilote lui est-il attache ?
    ///
    /// Vraie meme en reprise et meme en echec : c'est precisement ce que le
    /// rapport du 18 septembre ne savait pas dire.
    pub fn attache(self) -> bool {
        matches!(self, Etat::Lie | Etat::Pret | Etat::Reprise | Etat::Echec)
    }

    /// Le pilote rend-il service MAINTENANT ?
    ///
    /// Seul `Pret` l'autorise. Emettre pendant une reprise ecrirait dans des
    /// anneaux qu'on est en train de reprogrammer.
    pub fn en_service(self) -> bool {
        self == Etat::Pret
    }

    /// Une reprise a-t-elle le droit de commencer ?
    pub fn reprise_possible(self) -> bool {
        matches!(self, Etat::Pret | Etat::Echec)
    }
}

/// La transition demandee est-elle legitime ?
///
/// La regle qui compte : RIEN ne ramene a `Absent`. C'est la transition que
/// l'ancienne reprise de degre quatre effectuait en posant `READY = false` et
/// `MMIO = 0`, et c'est elle qui faisait disparaitre la carte du rapport.
pub fn transition_permise(depuis: Etat, vers: Etat) -> bool {
    if vers == Etat::Absent {
        // Seul le neant mene au neant.
        return depuis == Etat::Absent;
    }
    match (depuis, vers) {
        // La detection, puis l'attachement.
        (Etat::Absent, Etat::Detecte) => true,
        (Etat::Detecte, Etat::Lie) => true,
        // Une fois lie, on programme le materiel.
        (Etat::Lie, Etat::Pret) => true,
        (Etat::Lie, Etat::Echec) => true,
        // La vie courante.
        (Etat::Pret, Etat::Reprise) => true,
        (Etat::Reprise, Etat::Pret) => true,
        (Etat::Reprise, Etat::Echec) => true,
        // Un echec n'est pas definitif : on a le droit de retenter.
        (Etat::Echec, Etat::Reprise) => true,
        (Etat::Echec, Etat::Pret) => true,
        // Rester sur place est toujours permis.
        (a, b) if a == b => true,
        _ => false,
    }
}

/// Ce qu'une couche superieure doit afficher pour cette carte.
///
/// Rend le couple (etat lisible, raison). La raison n'est jamais « absente de
/// ce materiel » pour une puce qu'on a vue sur le bus.
pub fn resume(etat: Etat, lien: bool) -> (&'static str, &'static str) {
    match etat {
        Etat::Absent => ("indisponible", "absente de ce materiel"),
        Etat::Detecte => ("attente", "aucun pilote attache"),
        Etat::Lie => ("demarrage", "pilote attache"),
        Etat::Pret if lien => ("actif", "pilote en service"),
        Etat::Pret => ("attente", "lien bas"),
        Etat::Reprise => ("reprise", "reinitialisation en cours"),
        Etat::Echec => ("panne", "reinitialisation echouee"),
    }
}
