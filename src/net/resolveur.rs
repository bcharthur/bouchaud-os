//! Quel resolveur DNS remettre au navigateur, et d'ou il vient.
//!
//! # Le defaut, releve le 16 septembre 2026
//!
//! Le journal physique dit, a une seconde d'intervalle :
//!
//!     17:00:38  BOUCHAUD_NET_LIEN etat=UP ancien_verdict=lien-bas
//!     17:00:38  net: eth0 lien UP 1000 Mb/s duplex complet
//!     17:00:39  BOUCHAUD_NAVIGATEUR_RESEAU dns=10.0.2.3 verdict=lien-bas
//!               lien=0 resolveur=NON-CONFIGURE
//!     17:00:39  Setting DNS server to 10.0.2.3:53
//!
//! `10.0.2.3` est le resolveur du NAT de QEMU. Sur la TRIGKEY, branchee a un
//! vrai reseau, cette adresse ne mene nulle part : toutes les pages repondent
//! « Unable to resolve host », et la panne parait venir du navigateur.
//!
//! Le resolveur est lu UNE FOIS, a l'exec, et le navigateur le garde pour la
//! vie. Le creneau est donc etroit et decisif.
//!
//! # La regle
//!
//! Le bail DHCP d'abord. A defaut, LA PASSERELLE : sur a peu pres toutes les
//! box domestiques elle resout, et c'est une adresse qui existe reellement
//! sur ce reseau-ci -- contrairement a la valeur compilee, qui n'est juste
//! que sous QEMU.
//!
//! # Pourquoi ce module est PUR
//!
//! Aucun `use crate::`, aucun `unsafe`. La regle se verifie sur machine hote,
//! adresse par adresse, au lieu d'etre constatee une fois par session
//! physique -- c'est-a-dire une fois par jour.

/// D'ou vient le resolveur remis au navigateur.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// Le bail DHCP. Le cas nominal.
    Bail = 0,
    /// La passerelle, faute de bail. Elle resout presque toujours.
    Passerelle = 1,
    /// La valeur compilee. Juste sous QEMU, fausse a peu pres partout ailleurs.
    Compile = 2,
    /// Rien d'utilisable.
    Aucun = 3,
}

impl Source {
    pub fn nom(self) -> &'static str {
        match self {
            Source::Bail => "bail-dhcp",
            Source::Passerelle => "passerelle",
            Source::Compile => "compile",
            Source::Aucun => "aucun",
        }
    }
}

/// Une adresse peut-elle servir de resolveur ?
///
/// Zero n'est pas une adresse. La boucle locale ne resout rien ici : aucun
/// resolveur n'ecoute dans ce noyau, et la pointer ferait echouer chaque
/// requete apres un delai d'attente au lieu d'echouer tout de suite. La
/// diffusion et le multicast ne sont pas des interlocuteurs.
pub fn utilisable(adresse: [u8; 4]) -> bool {
    if adresse == [0, 0, 0, 0] {
        return false;
    }
    if adresse[0] == 127 {
        return false;
    }
    if adresse == [255, 255, 255, 255] {
        return false;
    }
    // 224.0.0.0/4 : multicast. 240.0.0.0/4 : reserve.
    if adresse[0] >= 224 {
        return false;
    }
    true
}

/// Choisit le resolveur a transmettre, et dit d'ou il vient.
pub fn choisis(
    bail: [u8; 4],
    passerelle: [u8; 4],
    compile: [u8; 4],
) -> ([u8; 4], Source) {
    if utilisable(bail) {
        return (bail, Source::Bail);
    }
    if utilisable(passerelle) {
        return (passerelle, Source::Passerelle);
    }
    if utilisable(compile) {
        return (compile, Source::Compile);
    }
    ([0, 0, 0, 0], Source::Aucun)
}
