//! Le presse-papiers du bureau.
//!
//! ## Ce qu'il est
//!
//! Un seul contenu, en octets, partage par tous les clients GUI. Copier dans le
//! navigateur et coller ailleurs -- ou l'inverse -- passe par ici, et c'est la
//! seule chose qui distingue un presse-papiers d'un tampon interne a une
//! application.
//!
//! ## Le modele de diffusion : pousser, pas laisser lire
//!
//! Un client ne DEMANDE jamais le contenu. C'est le gestionnaire de fenetres
//! qui le POUSSE, et seulement au client qui a le foyer, et seulement quand le
//! contenu a change depuis ce que ce client a deja recu -- d'ou le numero de
//! generation.
//!
//! Ce choix est un choix de securite, et c'est la faiblesse historique de
//! X11 qu'il ferme : la ou n'importe quel client peut lire la selection a tout
//! moment, un programme en arriere-plan n'a qu'a interroger le presse-papiers
//! en boucle pour recolter tout ce que l'utilisateur copie -- un mot de passe
//! sorti d'un gestionnaire, une phrase de recuperation, un jeton. Ici, un
//! client sans foyer ne recoit rien du tout, et n'a aucun message a envoyer
//! pour en obtenir : il n'existe pas de « demande de lecture » dans le
//! protocole, donc pas de chemin a oublier de garder.
//!
//! L'ECRITURE est bornee par la meme regle, et pour une raison symetrique :
//! un client d'arriere-plan qui pourrait ecrire remplacerait silencieusement
//! ce que l'utilisateur vient de copier -- l'adresse d'un virement, par
//! exemple, par une autre. C'est `client.rs` qui applique cette regle, la ou
//! le foyer est connu.
//!
//! ## Ce qu'il n'est pas
//!
//! Pas de formats multiples, pas de negociation de cible, pas de proprietaire
//! au sens X11 : le contenu est COPIE au moment ou on le donne. Un client qui
//! meurt ne fait donc pas disparaitre ce qu'il avait copie, ce qui est le
//! comportement que tout le monde attend et que X11 n'a jamais eu.
//!
//! ## Ce module est pur
//!
//! Il ne connait ni le framebuffer, ni les fenetres, ni les taches : il garde
//! des octets et un compteur. `tools/gui/test_presse_papiers.rs` l'inclut tel
//! quel et l'exerce sur l'hote.

use alloc::vec::Vec;

use crate::kernel::sync::SpinLock;

/// Taille maximale d'un contenu, en octets.
///
/// C'est le plafond de charge utile du protocole, et non un chiffre choisi
/// ici : le contenu voyage dans UN message, donc ce qui ne tient pas dans un
/// message ne pourrait pas etre remis. Deux bornes independantes pour une
/// seule contrainte finiraient par diverger.
pub const CAPACITE: usize = crate::gui::protocole::CHARGE_MAX as usize;

struct Contenu {
    octets: Vec<u8>,
    /// Numero de version, jamais decroissant.
    ///
    /// C'est lui qui permet de ne rien pousser quand rien n'a change, sans
    /// comparer des kibioctets a chaque tour de composition. Le comparer est
    /// aussi ce qui rend la poussee idempotente : un client qui vient
    /// d'ecrire connait deja la generation qu'il a produite.
    generation: u64,
}

static CONTENU: SpinLock<Contenu> = SpinLock::new(Contenu {
    octets: Vec::new(),
    generation: 0,
});

/// Remplace le contenu. Rend la generation ainsi produite.
///
/// Un contenu trop grand est TRONQUE et non refuse : ce qui arrive ici a deja
/// traverse le protocole, dont le decodeur borne la charge utile. Tronquer est
/// la defense en profondeur -- si un jour les deux bornes divergent, on perd
/// la fin d'un texte, pas la memoire du noyau.
pub fn ecrit(octets: &[u8]) -> u64 {
    let mut contenu = CONTENU.lock();
    contenu.octets.clear();
    let fin = core::cmp::min(octets.len(), CAPACITE);
    contenu.octets.extend_from_slice(&octets[..fin]);
    contenu.generation = contenu.generation.wrapping_add(1);
    contenu.generation
}

/// Le contenu et sa generation, copies.
pub fn lit() -> (Vec<u8>, u64) {
    let contenu = CONTENU.lock();
    (contenu.octets.clone(), contenu.generation)
}

/// La generation seule, sans copier le contenu.
///
/// C'est ce que consulte la boucle de composition, soixante fois par seconde
/// et par client : copier des kibioctets pour decouvrir qu'ils n'ont pas
/// change serait le genre de depense qui ne se voit que sur une machine lente,
/// c'est-a-dire sur celle-ci.
pub fn generation() -> u64 {
    CONTENU.lock().generation
}

/// Remet le presse-papiers a vide. Reservee aux tests.
#[cfg(test)]
pub fn reinitialise() {
    let mut contenu = CONTENU.lock();
    contenu.octets.clear();
    contenu.generation = 0;
}
