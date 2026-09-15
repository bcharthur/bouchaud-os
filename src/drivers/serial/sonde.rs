//! Verdict de presence d'un UART 16550 sur un port ISA, sans toucher au port.
//!
//! BOUCHAUD_SONDE_COM1_V1
//!
//! # Le defaut que ceci corrige
//!
//! `init()` posait `INITIALISED = true` sans jamais demander au materiel s'il
//! y avait quelqu'un. Sur un mini-PC sans Super I/O -- la TRIGKEY en est un --
//! chaque octet de journal part alors vers un port que personne ne decode :
//! une attente de THRE, puis seize `outb`, pour rien, et cela des la premiere
//! ligne du demarrage.
//!
//! Ce n'est pas seulement du temps perdu. C'est du temps perdu PROPORTIONNEL
//! a la verbosite, donc une taxe sur le diagnostic lui-meme : plus on veut
//! comprendre, plus on ralentit ce qu'on observe.
//!
//! # Pourquoi ce fichier est pur
//!
//! La sonde materielle tient en quatre acces au port. Ce qui est difficile,
//! ce n'est pas de les faire : c'est de LIRE leur resultat sans se tromper.
//! Cette decision-la est ici, sans `unsafe`, sans `use crate::`, et elle est
//! verifiee sur l'hote par `tools/platform/test_sonde_uart.rs`.

/// Motif ecrit dans le registre de donnees pendant le test de bouclage.
///
/// N'importe quelle valeur non triviale convient. `0xAE` est celle qu'utilise
/// la sonde 16550 de reference ; la garder evite d'avoir a justifier un choix
/// different a chaque relecture.
pub const MOTIF_BOUCLAGE: u8 = 0xAE;

/// Ce qu'une sonde a observe, sous une forme qui se raconte.
///
/// Les valeurs sont FIXEES : le pilote range le verdict dans un `AtomicU8` et
/// le relit ailleurs. Laisser le compilateur choisir les numeros ferait
/// dependre ce codage de l'ordre des variantes, donc d'une refonte sans
/// rapport.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Presence {
    /// Aucun peripherique ne decode le port : tous les registres rendent 0xFF.
    BusFlottant = 0,
    /// Le port est decode, mais le bouclage n'a pas rendu le motif.
    Muet = 1,
    /// Le bouclage a rendu le motif : il y a un 16550 derriere.
    Present = 2,
}

impl Presence {
    /// Faut-il ecrire sur le port ?
    pub fn ecrire(self) -> bool {
        matches!(self, Presence::Present)
    }

    /// Nom court pour le journal.
    pub fn nom(self) -> &'static str {
        match self {
            Presence::BusFlottant => "bus-flottant",
            Presence::Muet => "muet",
            Presence::Present => "present",
        }
    }
}

/// Verdict a partir des trois signaux qu'une sonde peut relever.
///
/// * `lsr` et `iir` sont lus AVANT toute ecriture. Un port non decode rend
///   0xFF sur tous ses registres -- c'est la signature du bus flottant, et
///   c'est le seul cas ou l'on peut conclure sans rien ecrire.
/// * `echo_bouclage` est ce que le registre de donnees rend apres avoir recu
///   `motif`, le controleur place en boucle locale.
///
/// L'ordre compte : sur un bus flottant, `echo_bouclage` vaut 0xFF lui aussi,
/// et `0xFF == 0xFF` conclurait « present » si l'on testait le bouclage en
/// premier avec un motif de 0xFF. Ecarter le bus flottant d'abord rend le
/// verdict insensible au motif choisi.
pub fn verdict(lsr: u8, iir: u8, echo_bouclage: u8, motif: u8) -> Presence {
    if lsr == 0xFF && iir == 0xFF {
        return Presence::BusFlottant;
    }
    if echo_bouclage == motif {
        Presence::Present
    } else {
        Presence::Muet
    }
}

/// Relit un verdict range dans un octet.
///
/// Toute valeur inconnue veut dire « ecris quand meme » : perdre le journal
/// parce qu'un octet a ete mal range serait le pire des deux echecs.
pub fn depuis_octet(code: u8) -> Presence {
    match code {
        0 => Presence::BusFlottant,
        1 => Presence::Muet,
        _ => Presence::Present,
    }
}
