//! Manifeste des polices : la seule source de verite.
//!
//! # Pourquoi il existe
//!
//! Trois endroits decrivaient les polices, et ils se contredisaient :
//!
//!   * `src/assets/fonts/` contenait QUATRE fichiers ;
//!   * `kernel::sysroot` en installait TROIS -- `DejaVuSansMono-Bold.ttf`
//!     etait embarque dans le binaire et n'arrivait jamais dans le systeme de
//!     fichiers ;
//!   * la configuration fontconfig du portage Ladybird designait une famille
//!     « DejaVu Serif » dont aucun fichier n'existe.
//!
//! Aucune de ces contradictions ne produit d'erreur. Elles produisent du texte
//! qui tombe silencieusement sur une autre police -- c'est-a-dire exactement le
//! genre de defaut qu'on met des mois a remarquer et cinq minutes a corriger
//! quand on sait ou regarder.
//!
//! Le manifeste ci-dessous est lu par l'installation ET par
//! `tools/gui/verifie-polices.py`, qui refuse qu'un fichier existe sans etre
//! declare, ou l'inverse.

/// Graisse d'une police. Deux suffisent aujourd'hui ; en ajouter une demande
/// d'ajouter le fichier, pas seulement la valeur.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Graisse {
    Normale,
    Grasse,
}

/// A quoi sert une famille, du point de vue de celui qui dessine.
///
/// Le role est ce que demande un widget ; la famille est ce que le systeme a.
/// Les separer permet de changer de police sans toucher au code de rendu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Famille {
    /// Texte d'interface et de contenu : la police par defaut.
    Sans,
    /// Chasse fixe : terminal, code, colonnes alignees.
    Mono,
}

/// Une police reellement embarquee ET installee.
pub struct Police {
    pub famille: Famille,
    pub graisse: Graisse,
    /// Nom du fichier dans `/usr/share/fonts/truetype/dejavu`.
    pub fichier: &'static str,
    pub octets: &'static [u8],
}

/// Les polices du systeme. Ajouter une ligne suppose d'ajouter le fichier :
/// `include_bytes!` refuse de compiler sinon, et la barriere refuse un fichier
/// qui ne serait pas declare ici.
pub fn manifeste() -> [Police; 4] {
    [
        Police {
            famille: Famille::Sans,
            graisse: Graisse::Normale,
            fichier: "DejaVuSans.ttf",
            octets: crate::gui::font::FONT_DATA,
        },
        Police {
            famille: Famille::Sans,
            graisse: Graisse::Grasse,
            fichier: "DejaVuSans-Bold.ttf",
            octets: include_bytes!("../assets/fonts/DejaVuSans-Bold.ttf"),
        },
        Police {
            famille: Famille::Mono,
            graisse: Graisse::Normale,
            fichier: "DejaVuSansMono.ttf",
            octets: include_bytes!("../assets/fonts/DejaVuSansMono.ttf"),
        },
        Police {
            famille: Famille::Mono,
            graisse: Graisse::Grasse,
            fichier: "DejaVuSansMono-Bold.ttf",
            octets: include_bytes!("../assets/fonts/DejaVuSansMono-Bold.ttf"),
        },
    ]
}

/// Repertoire d'installation, tel que fontconfig et Ladybird le cherchent.
pub const REPERTOIRE: &str = "/usr/share/fonts/truetype/dejavu";

/// Familles fontconfig reellement servies par ce manifeste.
///
/// Sert a la barriere : une configuration qui prefere une famille absente
/// n'echoue pas, elle tombe en silence sur la suivante. Autant le dire.
pub const FAMILLES_FOURNIES: [&str; 2] = ["DejaVu Sans", "DejaVu Sans Mono"];
