//! Decodage clavier PS/2 : la partie qui ne touche a aucun materiel.
//!
//! # Pourquoi ce fichier existe
//!
//! Tout ce qui est ici est une fonction du seul flux d'octets : la disposition
//! AZERTY, l'etat des modificateurs, et la transition appui/relachement. Rien
//! n'y lit un port, ne masque une IRQ ni n'ecrit a l'ecran.
//!
//! C'est ce qui permet de l'exercer sur la machine de developpement, sans QEMU
//! -- voir `tools/gui/test_clavier.rs`. Un decodeur clavier est exactement le
//! genre de code ou une faute ne se voit pas : une touche manquante se prend
//! pour une frappe ratee, un relachement perdu pour une bizarrerie de la page.
//!
//! `ps2_keyboard.rs` garde ce qui parle au controleur 8042 et n'a plus qu'un
//! etat a tenir.

fn ascii_letter(ch: u8, shift: bool) -> char {
    if shift && ch >= b'a' && ch <= b'z' {
        (ch - 32) as char
    } else {
        ch as char
    }
}

/// Traduit un scancode en caractere selon la disposition AZERTY-FR.
///
/// `altgr` active la 3e couche (AltGr) qui fournit les symboles indispensables
/// au shell ( | < > { } [ ] \ @ # ~ ` ^ ), utile notamment sur les claviers
/// portables depourvus de la touche ISO `<>` (ex. Dell a pave numerique).
///
/// Les caracteres accentues sont translitteres tant que l'affichage reste en
/// ASCII pur (ex. la touche `é` produit `e`).
fn scancode_to_char(sc: u8, shift: bool, altgr: bool) -> Option<char> {
    if altgr {
        // Couche AltGr (FR) + raccourcis Bouchaud OS pour < et > sans touche ISO.
        return match sc {
            0x03 => Some('~'),   // AltGr+2
            0x04 => Some('#'),   // AltGr+3
            0x05 => Some('{'),   // AltGr+4
            0x06 => Some('['),   // AltGr+5
            0x07 => Some('|'),   // AltGr+6
            0x08 => Some('`'),   // AltGr+7
            0x09 => Some('\\'),  // AltGr+8
            0x0a => Some('^'),   // AltGr+9
            0x0b => Some('@'),   // AltGr+0
            0x0c => Some(']'),   // AltGr+)
            0x0d => Some('}'),   // AltGr+=
            0x32 => Some('<'),   // AltGr+, (touche virgule)
            0x33 => Some('>'),   // AltGr+; (touche point-virgule)
            _ => None,
        };
    }
    match sc {
        0x01 => Some('\x1b'),
        0x0e => Some('\x08'),
        0x0f => Some('\t'),
        0x1c => Some('\n'),
        0x39 => Some(' '),

        // Ligne numerique AZERTY. Les accents sont translitteres pour l'instant.
        0x02 => Some(if shift { '1' } else { '&' }),
        0x03 => Some(if shift { '2' } else { 'e' }),
        0x04 => Some(if shift { '3' } else { '"' }),
        0x05 => Some(if shift { '4' } else { '\'' }),
        0x06 => Some(if shift { '5' } else { '(' }),
        0x07 => Some(if shift { '6' } else { '-' }),
        0x08 => Some(if shift { '7' } else { 'e' }),
        0x09 => Some(if shift { '8' } else { '_' }),
        0x0a => Some(if shift { '9' } else { 'c' }),
        0x0b => Some(if shift { '0' } else { 'a' }),
        0x0c => Some(if shift { ')' } else { ')' }),
        0x0d => Some(if shift { '+' } else { '=' }),

        // AZERTY lettres principales
        0x10 => Some(ascii_letter(b'a', shift)),
        0x11 => Some(ascii_letter(b'z', shift)),
        0x12 => Some(ascii_letter(b'e', shift)),
        0x13 => Some(ascii_letter(b'r', shift)),
        0x14 => Some(ascii_letter(b't', shift)),
        0x15 => Some(ascii_letter(b'y', shift)),
        0x16 => Some(ascii_letter(b'u', shift)),
        0x17 => Some(ascii_letter(b'i', shift)),
        0x18 => Some(ascii_letter(b'o', shift)),
        0x19 => Some(ascii_letter(b'p', shift)),
        0x1a => Some(if shift { '^' } else { '^' }),
        0x1b => Some(if shift { '*' } else { '$' }),

        0x1e => Some(ascii_letter(b'q', shift)),
        0x1f => Some(ascii_letter(b's', shift)),
        0x20 => Some(ascii_letter(b'd', shift)),
        0x21 => Some(ascii_letter(b'f', shift)),
        0x22 => Some(ascii_letter(b'g', shift)),
        0x23 => Some(ascii_letter(b'h', shift)),
        0x24 => Some(ascii_letter(b'j', shift)),
        0x25 => Some(ascii_letter(b'k', shift)),
        0x26 => Some(ascii_letter(b'l', shift)),
        0x27 => Some(ascii_letter(b'm', shift)),
        0x28 => Some(if shift { '%' } else { 'u' }),
        0x2b => Some(if shift { '|' } else { '*' }),

        0x2c => Some(ascii_letter(b'w', shift)),
        0x2d => Some(ascii_letter(b'x', shift)),
        0x2e => Some(ascii_letter(b'c', shift)),
        0x2f => Some(ascii_letter(b'v', shift)),
        0x30 => Some(ascii_letter(b'b', shift)),
        0x31 => Some(ascii_letter(b'n', shift)),
        0x32 => Some(if shift { '?' } else { ',' }),
        0x33 => Some(if shift { '.' } else { ';' }),
        0x34 => Some(if shift { '/' } else { ':' }),
        0x35 => Some(if shift { '/' } else { '!' }),

        // Touche ISO "<>" (a gauche de W) presente sur la plupart des AZERTY.
        0x56 => Some(if shift { '>' } else { '<' }),
        _ => None,
    }
}

/// Numero d'une touche de fonction, ou `None`.
///
/// F1 a F10 se suivent (0x3b..0x44), puis F11 et F12 ont ete ajoutees plus tard
/// a la fin du jeu 1 (0x57, 0x58) : c'est de l'histoire du materiel, pas une
/// regle, et l'ecrire en table plutot qu'en arithmetique evite d'avoir a s'en
/// souvenir.
fn touche_de_fonction(sc: u8) -> Option<u8> {
    match sc {
        0x3b..=0x44 => Some(sc - 0x3b + 1),
        0x57 => Some(11),
        0x58 => Some(12),
        _ => None,
    }
}

/// Touche logique, apres application de la disposition.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Char(u8),
    Enter,
    Backspace,
    Tab,
    Up,
    Down,
    Left,
    Right,
    /// Le pave de navigation, et la touche Suppr.
    ///
    /// # Pourquoi elles arrivent si tard
    ///
    /// Le decodeur ne reconnaissait que les quatre fleches parmi les sequences
    /// etendues, et rendait `None` pour tout le reste. Origine, Fin, Page
    /// precedente et Page suivante etaient donc PERDUES entre le controleur et
    /// le client : sur le bureau, aucune consequence visible ; dans un
    /// navigateur, l'impossibilite de faire defiler une page sans molette.
    ///
    /// Suppr etait pire que perdue. `0xE0 0x53` etait traduit en
    /// [`Key::Backspace`], si bien que la touche effacait le caractere de
    /// GAUCHE. Ce n'est pas une touche manquante, c'est une touche qui fait
    /// autre chose que ce qu'elle annonce.
    Home,
    End,
    PageUp,
    PageDown,
    Delete,
    Insert,
    /// F1 a F12. Le numero, pas le scancode : c'est ce que tout le reste de la
    /// chaine manipule, et le traduire une seule fois evite que chaque
    /// consommateur refasse la table.
    Fonction(u8),
    Other,
}

/// Etat des modificateurs a l'instant d'une transition.
///
/// Des booleens, et non un masque : l'assignation des bits appartient au
/// protocole GUI, pas au pilote PS/2. Elle est faite une seule fois, dans
/// `window_manager::modificateur`, ou la barriere du protocole sait la lire.
/// Un pilote qui inventerait ses propres bits en ferait une seconde source de
/// verite, et le desaccord ne se verrait qu'a l'execution.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Modificateurs {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub altgr: bool,
}

/// Une transition physique de touche, telle que le controleur l'a produite.
///
/// # Pourquoi ce type existe
///
/// [`Key`] est une touche DEJA interpretee : elle ne dit pas si elle est un
/// appui ou un relachement, et l'ancien decodeur jetait purement et simplement
/// les codes de relachement. Le protocole GUI transportait pourtant un champ
/// `appui` depuis le debut, et le pont Ladybird fabriquait un `KeyUp`
/// synthetique juste apres chaque `KeyDown` faute de recevoir le vrai. Une page
/// qui compte les deux -- un jeu, un raccourci maintenu, une saisie qui suit
/// `keyup` -- voyait donc toutes ses touches relachees dans l'instant.
///
/// L'information existe des l'IRQ. Ce type la conserve jusqu'au client.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KeyEvent {
    /// Scancode du jeu 1, bit de relachement retire.
    pub scancode: u8,
    /// La sequence portait le prefixe 0xE0.
    pub etendue: bool,
    /// Touche logique apres application de la disposition.
    pub logique: Key,
    /// Point de code produit, 0 si la touche n'en produit aucun.
    pub unicode: u32,
    /// Etat des modificateurs au moment de la transition.
    pub modificateurs: Modificateurs,
    /// Appui (`true`) ou relachement (`false`).
    pub appui: bool,
    /// Repetition automatique du controleur, et non une nouvelle frappe.
    pub repeat: bool,
}

/// Tout ce que le decodeur doit retenir entre deux octets.
///
/// Explicite plutot que global : c'est ce qui rend un scenario reproductible.
/// Un test peut derouler « Shift enfonce, A, A, Shift relache » sur un etat
/// neuf et affirmer ce qui sort, sans que l'ordre des tests compte.
#[derive(Clone, Copy, Default)]
pub struct EtatClavier {
    shift: bool,
    ctrl: bool,
    alt: bool,
    altgr: bool,
    /// Prefixe 0xE0 recu, second octet attendu.
    etendu_en_attente: bool,
    /// Touches physiquement enfoncees : 128 codes de base + 128 etendus.
    enfoncees: [u64; 4],
}

impl EtatClavier {
    pub const fn neuf() -> Self {
        Self {
            shift: false,
            ctrl: false,
            alt: false,
            altgr: false,
            etendu_en_attente: false,
            enfoncees: [0; 4],
        }
    }

    pub fn modificateurs(&self) -> Modificateurs {
        Modificateurs {
            shift: self.shift,
            ctrl: self.ctrl,
            alt: self.alt,
            altgr: self.altgr,
        }
    }

    /// Note l'etat physique et rend `true` si c'est une REPETITION.
    ///
    /// Le controleur PS/2 renvoie le meme code d'appui, sans relachement
    /// intermediaire, quand une touche reste enfoncee. Sans cette memoire, une
    /// touche maintenue produirait une rafale que rien ne distinguerait de
    /// frappes distinctes -- et le client compterait des caracteres que
    /// personne n'a tapes.
    fn marque(&mut self, base: u8, etendue: bool, appui: bool) -> bool {
        let index = base as usize | if etendue { 0x80 } else { 0 };
        let mot = index / 64;
        let bit = 1u64 << (index % 64);
        let etait = self.enfoncees[mot] & bit != 0;
        if appui {
            self.enfoncees[mot] |= bit;
        } else {
            self.enfoncees[mot] &= !bit;
        }
        appui && etait
    }

    /// Touche logique seule, relachements ecartes.
    ///
    /// C'est ce que veulent le shell, l'invite de connexion et l'editeur de
    /// ligne : un caractere, pas une transition.
    pub fn decode_touche(&mut self, sc: u8) -> Option<Key> {
        self.decode(sc).filter(|e| e.appui).map(|e| e.logique)
    }

    /// Transition complete. Rend `None` pour les octets qui ne portent qu'un
    /// etat : prefixe 0xE0, et les modificateurs eux-memes.
    pub fn decode(&mut self, sc: u8) -> Option<KeyEvent> {
        let etendue = core::mem::replace(&mut self.etendu_en_attente, false);

        if !etendue && sc == 0xe0 {
            self.etendu_en_attente = true;
            return None;
        }

        let appui = sc & 0x80 == 0;
        let base = sc & 0x7f;

        // Les modificateurs portent un etat, ils ne produisent pas
        // d'evenement. Leur transition est prise dans les DEUX sens : un Shift
        // dont on ignorerait le relachement resterait actif pour toujours.
        match (base, etendue) {
            (0x2a, false) | (0x36, false) => { self.shift = appui; return None; }
            (0x1d, _) => { self.ctrl = appui; return None; }
            (0x38, false) => { self.alt = appui; return None; }
            (0x38, true) => { self.altgr = appui; return None; }
            _ => {}
        }

        let repeat = self.marque(base, etendue, appui);

        let (logique, unicode) = if etendue {
            // Le pave de navigation d'un clavier 104 touches. Le pave numerique
            // porte les MEMES scancodes sans le prefixe 0xE0, mais sa
            // signification depend de Verr.Num, que ce decodeur ne suit pas :
            // le traiter ici ferait taper « 7 » a qui appuie sur Origine, ou
            // l'inverse, une fois sur deux. Le bloc etendu, lui, ne veut dire
            // qu'une chose.
            match base {
                0x47 => (Key::Home, 0),
                0x48 => (Key::Up, 0),
                0x49 => (Key::PageUp, 0),
                0x4b => (Key::Left, 0),
                0x4d => (Key::Right, 0),
                0x4f => (Key::End, 0),
                0x50 => (Key::Down, 0),
                0x51 => (Key::PageDown, 0),
                0x52 => (Key::Insert, 0),
                // Suppr, et non Retour arriere : voir [`Key::Delete`].
                0x53 => (Key::Delete, 0),
                // Entree du pave numerique.
                0x1c => (Key::Enter, 0),
                _ => return None,
            }
        } else if let Some(numero) = touche_de_fonction(base) {
            (Key::Fonction(numero), numero as u32)
        } else {
            match scancode_to_char(base, self.shift, self.altgr)? {
                '\n' => (Key::Enter, 0),
                '\x08' => (Key::Backspace, 0),
                '\t' => (Key::Tab, 0),
                '\x1b' => (Key::Other, 0),
                c => (Key::Char(c as u8), c as u32),
            }
        };

        Some(KeyEvent {
            scancode: base,
            etendue,
            logique,
            unicode,
            modificateurs: self.modificateurs(),
            appui,
            repeat,
        })
    }
}
