// Le decodage HID : traduire ce qu'un clavier ou une souris USB envoie.
//
// # Pourquoi ce code doit vivre a part
//
// La table des touches fait cent lignes de correspondances. Une seule entree
// fausse ne fait rien planter : elle fait qu'UNE touche ne marche pas, sur une
// machine physique, ou l'on ne peut la decouvrir qu'en appuyant dessus. Il n'y
// a pas de pire endroit pour une faute -- elle est invisible a la relecture,
// invisible a la compilation, et elle se manifeste chez l'utilisateur.
//
// Ce fichier ne touche donc rien : ni registre, ni file de transfert, ni pile
// d'entree. Il transforme des octets en evenements. C'est ce qui permet a
// `tools/platform/test_hid.rs` de lui donner les rapports que produisent un
// clavier azerty, une souris a molette, un adaptateur qui prefixe ses rapports
// d'un identifiant -- et de verifier chaque touche.
//
// # Ce qu'un rapport de clavier ne dit pas
//
// Il ne dit pas « cette touche vient d'etre appuyee ». Il dit « voici les six
// touches actuellement enfoncees ». Les appuis et les relachements sont la
// DIFFERENCE entre deux rapports, et c'est a nous de la faire.
//
// Une consequence qu'on oublie : un rapport identique au precedent n'est pas
// une repetition, c'est une absence de changement. Le traiter comme un nouvel
// appui ferait doubler chaque caractere.
//
// Une autre : quand une touche disparait du rapport, il faut emettre son
// relachement -- meme si le rapport suivant est vide. Sans cela, une touche
// modificatrice restee « enfoncee » transforme tout ce qui suit.

#![allow(dead_code)]

/// Ce qu'un rapport produit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Evenement {
    /// Code PS/2 de la touche.
    pub code: u8,
    /// La touche a-t-elle un prefixe 0xE0 ?
    pub etendu: bool,
    /// Appui (vrai) ou relachement (faux).
    pub appui: bool,
}

/// Evenements qu'un seul rapport peut produire.
///
/// Huit modificateurs, six touches relachees, six touches appuyees.
pub const EVENEMENTS_MAX: usize = 20;

/// L'etat d'un clavier entre deux rapports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct EtatClavier {
    pub modificateurs: u8,
    pub touches: [u8; 6],
}

/// Retire l'identifiant de rapport, quand le peripherique en met un.
///
/// Un peripherique qui declare plusieurs rapports prefixe chacun de son
/// identifiant. Ne pas le retirer decale TOUT d'un octet : les modificateurs
/// deviennent le premier code de touche, et le clavier tape n'importe quoi.
///
/// Un rapport dont l'identifiant n'est pas le notre n'est pas une erreur : il
/// appartient a un autre rapport du meme peripherique, et il faut l'ignorer,
/// pas le decoder.
pub fn charge_utile<'a>(identifiant: u8, donnees: &'a [u8]) -> Option<&'a [u8]> {
    if identifiant == 0 {
        return Some(donnees);
    }
    if donnees.first().copied()? != identifiant {
        return None;
    }
    Some(&donnees[1..])
}

/// Traduit un rapport de clavier en evenements, et met l'etat a jour.
///
/// Rend le nombre d'evenements ecrits, ou `None` quand le rapport n'est pas
/// exploitable -- trop court, ou destine a un autre rapport du peripherique.
///
/// L'ORDRE compte. Les relachements sortent avant les appuis : quand une
/// touche en remplace une autre dans le meme rapport -- ce qui arrive des
/// qu'on tape vite --, emettre l'appui d'abord donnerait deux touches
/// simultanement enfoncees pour l'application, puis un relachement qui arrive
/// apres coup.
pub fn evenements_clavier(
    etat: &mut EtatClavier,
    identifiant: u8,
    donnees: &[u8],
    sortie: &mut [Evenement; EVENEMENTS_MAX],
) -> Option<usize> {
    let donnees = charge_utile(identifiant, donnees)?;
    if donnees.len() < 8 {
        return None;
    }
    let modificateurs = donnees[0];
    let mut touches = [0u8; 6];
    touches.copy_from_slice(&donnees[2..8]);
    let mut n = 0usize;

    for bit in 0..8u8 {
        let masque = 1u8 << bit;
        let avant = etat.modificateurs & masque != 0;
        let apres = modificateurs & masque != 0;
        if avant != apres {
            if let Some((code, etendu)) = modificateur_ps2(bit) {
                sortie[n] = Evenement { code, etendu, appui: apres };
                n += 1;
            }
        }
    }

    // Les relachements d'abord.
    for ancienne in etat.touches.iter().copied() {
        if ancienne >= 4 && !touches.contains(&ancienne) {
            if let Some((code, etendu)) = usage_ps2(ancienne) {
                sortie[n] = Evenement { code, etendu, appui: false };
                n += 1;
            }
        }
    }
    for touche in touches.iter().copied() {
        if touche >= 4 && !etat.touches.contains(&touche) {
            if let Some((code, etendu)) = usage_ps2(touche) {
                sortie[n] = Evenement { code, etendu, appui: true };
                n += 1;
            }
        }
    }

    etat.modificateurs = modificateurs;
    etat.touches = touches;
    Some(n)
}

/// Ce qu'un rapport de souris dit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Souris {
    /// Gauche, droite, milieu -- les trois bits bas.
    pub boutons: u8,
    pub dx: i8,
    pub dy: i8,
    pub roue: i8,
}

/// Traduit un rapport de souris.
///
/// La disposition d'amorcage -- boutons, X, Y, puis molette optionnelle -- est
/// celle que `SET_PROTOCOL(boot)` garantit, et celle de l'immense majorite des
/// souris de bureau meme sans elle.
///
/// La molette est OPTIONNELLE et non nulle par defaut : une souris a trois
/// octets n'a pas une molette immobile, elle n'en a pas. Lire un quatrieme
/// octet qui n'existe pas ferait defiler au hasard.
pub fn decode_souris(identifiant: u8, donnees: &[u8]) -> Option<Souris> {
    let donnees = charge_utile(identifiant, donnees)?;
    if donnees.len() < 3 {
        return None;
    }
    Some(Souris {
        boutons: donnees[0] & 0x07,
        dx: donnees[1] as i8,
        dy: donnees[2] as i8,
        roue: if donnees.len() >= 4 { donnees[3] as i8 } else { 0 },
    })
}

/// Le genre d'une interface HID : 1 clavier, 2 souris, 0 autre.
///
/// Seule la sous-classe « amorcage » (1) porte un protocole qui veut dire
/// quelque chose. Une interface de sous-classe zero declare son role dans son
/// descripteur de rapport, qu'il faut alors analyser -- et deviner « clavier »
/// sur un protocole qui ne le dit pas ferait injecter des touches depuis un
/// volant de course.
pub const fn genre_interface(sous_classe: u8, protocole: u8) -> u8 {
    match (sous_classe, protocole) {
        (1, 1) => 1,
        (1, 2) => 2,
        _ => 0,
    }
}

pub fn modificateur_ps2(bit: u8) -> Option<(u8, bool)> {
    Some(match bit {
        0 => (0x1d, false), // Left Ctrl
        1 => (0x2a, false), // Left Shift
        2 => (0x38, false), // Left Alt
        3 => (0x5b, true),  // Left GUI
        4 => (0x1d, true),  // Right Ctrl
        5 => (0x36, false), // Right Shift
        6 => (0x38, true),  // Right Alt / AltGr
        7 => (0x5c, true),  // Right GUI
        _ => return None,
    })
}

pub fn usage_ps2(usage: u8) -> Option<(u8, bool)> {
    let result = match usage {
        0x04 => (0x1e, false), 0x05 => (0x30, false), 0x06 => (0x2e, false),
        0x07 => (0x20, false), 0x08 => (0x12, false), 0x09 => (0x21, false),
        0x0a => (0x22, false), 0x0b => (0x23, false), 0x0c => (0x17, false),
        0x0d => (0x24, false), 0x0e => (0x25, false), 0x0f => (0x26, false),
        0x10 => (0x32, false), 0x11 => (0x31, false), 0x12 => (0x18, false),
        0x13 => (0x19, false), 0x14 => (0x10, false), 0x15 => (0x13, false),
        0x16 => (0x1f, false), 0x17 => (0x14, false), 0x18 => (0x16, false),
        0x19 => (0x2f, false), 0x1a => (0x11, false), 0x1b => (0x2d, false),
        0x1c => (0x15, false), 0x1d => (0x2c, false),
        0x1e => (0x02, false), 0x1f => (0x03, false), 0x20 => (0x04, false),
        0x21 => (0x05, false), 0x22 => (0x06, false), 0x23 => (0x07, false),
        0x24 => (0x08, false), 0x25 => (0x09, false), 0x26 => (0x0a, false),
        0x27 => (0x0b, false),
        0x28 => (0x1c, false), 0x29 => (0x01, false), 0x2a => (0x0e, false),
        0x2b => (0x0f, false), 0x2c => (0x39, false), 0x2d => (0x0c, false),
        0x2e => (0x0d, false), 0x2f => (0x1a, false), 0x30 => (0x1b, false),
        0x31 | 0x32 => (0x2b, false), 0x33 => (0x27, false), 0x34 => (0x28, false),
        0x35 => (0x29, false), 0x36 => (0x33, false), 0x37 => (0x34, false),
        0x38 => (0x35, false), 0x39 => (0x3a, false),
        0x3a => (0x3b, false), 0x3b => (0x3c, false), 0x3c => (0x3d, false),
        0x3d => (0x3e, false), 0x3e => (0x3f, false), 0x3f => (0x40, false),
        0x40 => (0x41, false), 0x41 => (0x42, false), 0x42 => (0x43, false),
        0x43 => (0x44, false), 0x44 => (0x57, false), 0x45 => (0x58, false),
        0x47 => (0x46, false),
        0x49 => (0x52, true), 0x4a => (0x47, true), 0x4b => (0x49, true),
        0x4c => (0x53, true), 0x4d => (0x4f, true), 0x4e => (0x51, true),
        0x4f => (0x4d, true), 0x50 => (0x4b, true), 0x51 => (0x50, true),
        0x52 => (0x48, true),
        0x53 => (0x45, false), 0x54 => (0x35, true), 0x55 => (0x37, false),
        0x56 => (0x4a, false), 0x57 => (0x4e, false), 0x58 => (0x1c, true),
        0x59 => (0x4f, false), 0x5a => (0x50, false), 0x5b => (0x51, false),
        0x5c => (0x4b, false), 0x5d => (0x4c, false), 0x5e => (0x4d, false),
        0x5f => (0x47, false), 0x60 => (0x48, false), 0x61 => (0x49, false),
        0x62 => (0x52, false), 0x63 => (0x53, false),
        // 0x64 : la touche « inferieur / superieur », a gauche du W sur un
        // clavier francais et absente d'un clavier americain. Sans cette
        // ligne, elle ne fait RIEN -- et c'est exactement le genre de manque
        // qu'on ne decouvre qu'en appuyant dessus, sur la machine, apres
        // l'avoir livree.
        0x64 => (0x56, false),
        // 0x65 : la touche « menu contextuel », entre AltGr et Ctrl droit.
        0x65 => (0x5d, true),
        _ => return None,
    };
    Some(result)
}
