//! Format de fil du protocole GUI userland, version 1.
//!
//! Ce module est **pur** : il ne connait ni le noyau, ni le framebuffer, ni les
//! taches. Il ne fait qu'encoder, decoder et decouper des rectangles. C'est
//! deliberement le cas, pour deux raisons.
//!
//! D'abord parce que le meme format doit exister a l'identique de l'autre cote
//! du fil, dans `tools/userland/navigateur/hote.cpp`. Un desaccord d'un seul
//! octet sur la taille d'un en-tete ne se voit pas a la compilation : il se voit
//! six mois plus tard sous la forme d'une fenetre qui ne s'ouvre pas. Les tailles
//! sont donc verifiees a la compilation ici (voir les `const _`), et decrites
//! une seule fois dans `docs/GUI_USERLAND_PROTOCOL.md`.
//!
//! Ensuite parce qu'un module sans dependance noyau se **teste sur l'hote**.
//! `tools/gui/test_protocole.rs` inclut ce fichier tel quel et l'exerce avec
//! `rustc --test` : le rognage des rectangles de degat et la conversion des
//! coordonnees ecran vers fenetre sont exactement le genre d'arithmetique ou une
//! erreur d'un pixel se paye en corruption d'affichage, et exactement le genre de
//! code qu'aucune sonde QEMU ne saura isoler.
//!
//! ## Ordre des octets
//!
//! Tout est en petit-boutiste explicite. Les deux cotes tournent sur x86-64, mais
//! ecrire l'encodage a la main plutot que de recopier une structure `repr(C)`
//! coute quelques lignes et supprime la question de l'alignement — que le C++ et
//! le Rust ne resolvent pas forcement pareil des qu'un champ change de type.

use alloc::vec::Vec;

/// "BOGU" en ASCII, pour rejeter immediatement un flux qui n'est pas le notre.
pub const MAGIC: u32 = 0x5547_4F42;
pub const PROTOCOL_VERSION: u16 = 1;

/// Taille de l'en-tete fixe qui precede chaque charge utile.
pub const TAILLE_ENTETE: usize = 16;

/// Identifiant de la seule fenetre du jalon 2.
///
/// Le champ existe dans tous les messages parce que le multi-fenetre viendra
/// (un onglet detache, une popup) et qu'ajouter un champ a un protocole deja
/// deploye coute plus cher que de le prevoir. Il vaut 1 partout aujourd'hui.
pub const FENETRE_PRINCIPALE: u32 = 1;

#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Genre {
    // --- client -> gestionnaire de fenetres ---
    Hello = 1,
    CreateWindow = 2,
    SetTitle = 3,
    Damage = 4,
    Close = 5,
    FrameReady = 6,
    // --- gestionnaire de fenetres -> client ---
    Surface = 0x100,
    Configure = 0x101,
    Focus = 0x102,
    Key = 0x103,
    Pointer = 0x104,
    Wheel = 0x105,
    CloseRequest = 0x106,
}

impl Genre {
    pub const fn depuis_u16(valeur: u16) -> Option<Genre> {
        Some(match valeur {
            1 => Genre::Hello,
            2 => Genre::CreateWindow,
            3 => Genre::SetTitle,
            4 => Genre::Damage,
            5 => Genre::Close,
            6 => Genre::FrameReady,
            0x100 => Genre::Surface,
            0x101 => Genre::Configure,
            0x102 => Genre::Focus,
            0x103 => Genre::Key,
            0x104 => Genre::Pointer,
            0x105 => Genre::Wheel,
            0x106 => Genre::CloseRequest,
            _ => return None,
        })
    }
}

/// En-tete fixe de chaque message. La charge utile suit immediatement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entete {
    pub magic: u32,
    pub version: u16,
    pub genre: u16,
    pub taille_charge: u32,
    pub serie: u32,
}

impl Entete {
    pub const fn neuf(genre: Genre, taille_charge: u32, serie: u32) -> Self {
        Self {
            magic: MAGIC,
            version: PROTOCOL_VERSION,
            genre: genre as u16,
            taille_charge,
            serie,
        }
    }

    pub const fn valide(&self) -> bool {
        self.magic == MAGIC && self.version == PROTOCOL_VERSION
    }

    pub fn encode(&self) -> [u8; TAILLE_ENTETE] {
        let mut octets = [0u8; TAILLE_ENTETE];
        octets[0..4].copy_from_slice(&self.magic.to_le_bytes());
        octets[4..6].copy_from_slice(&self.version.to_le_bytes());
        octets[6..8].copy_from_slice(&self.genre.to_le_bytes());
        octets[8..12].copy_from_slice(&self.taille_charge.to_le_bytes());
        octets[12..16].copy_from_slice(&self.serie.to_le_bytes());
        octets
    }

    pub fn decode(octets: &[u8]) -> Option<Entete> {
        if octets.len() < TAILLE_ENTETE {
            return None;
        }
        Some(Entete {
            magic: u32::from_le_bytes([octets[0], octets[1], octets[2], octets[3]]),
            version: u16::from_le_bytes([octets[4], octets[5]]),
            genre: u16::from_le_bytes([octets[6], octets[7]]),
            taille_charge: u32::from_le_bytes([octets[8], octets[9], octets[10], octets[11]]),
            serie: u32::from_le_bytes([octets[12], octets[13], octets[14], octets[15]]),
        })
    }
}

/// Taille maximale d'une charge utile acceptee.
///
/// Un client qui annonce 4 Gio de titre ne doit pas faire allouer 4 Gio au
/// gestionnaire de fenetres avant d'etre rejete. Le plafond est genereux pour ce
/// que le protocole transporte reellement (le plus gros message est un titre) et
/// minuscule devant la memoire de la machine.
pub const CHARGE_MAX: u32 = 4096;

// --- Rectangles --------------------------------------------------------------

/// Rectangle en coordonnees entieres, largeur/hauteur non signees.
///
/// Le degat est exprime dans le repere **de la surface** du client, origine en
/// haut a gauche de sa zone utile. Le gestionnaire de fenetres y ajoute la
/// position de la fenetre : c'est lui, et lui seul, qui sait ou la fenetre se
/// trouve a l'ecran.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub largeur: u32,
    pub hauteur: u32,
}

impl Rect {
    pub const fn neuf(x: i32, y: i32, largeur: u32, hauteur: u32) -> Rect {
        Rect { x, y, largeur, hauteur }
    }

    pub const fn vide(&self) -> bool {
        self.largeur == 0 || self.hauteur == 0
    }

    /// Bord droit / bas, en `i64` pour qu'un client hostile ne puisse pas faire
    /// deborder l'addition et obtenir un rectangle qui « contient » l'ecran.
    pub const fn droite(&self) -> i64 {
        self.x as i64 + self.largeur as i64
    }
    pub const fn bas(&self) -> i64 {
        self.y as i64 + self.hauteur as i64
    }

    /// Intersection de deux rectangles ; rectangle vide s'ils sont disjoints.
    pub fn intersecte(&self, autre: &Rect) -> Rect {
        let x0 = self.x.max(autre.x) as i64;
        let y0 = self.y.max(autre.y) as i64;
        let x1 = self.droite().min(autre.droite());
        let y1 = self.bas().min(autre.bas());
        if x1 <= x0 || y1 <= y0 {
            return Rect::default();
        }
        Rect {
            x: x0 as i32,
            y: y0 as i32,
            largeur: (x1 - x0) as u32,
            hauteur: (y1 - y0) as u32,
        }
    }

    /// Plus petit rectangle contenant les deux. Un rectangle vide est neutre.
    ///
    /// C'est le repli assume du jalon 2 : on accumule les degats d'une trame en
    /// une seule boite plutot qu'en une liste de regions. Une union grossiere
    /// recopie parfois des pixels inchanges ; une liste de regions mal fusionnee
    /// en oublie — et un pixel oublie reste faux jusqu'a la trame suivante.
    pub fn union(&self, autre: &Rect) -> Rect {
        if self.vide() {
            return *autre;
        }
        if autre.vide() {
            return *self;
        }
        let x0 = self.x.min(autre.x) as i64;
        let y0 = self.y.min(autre.y) as i64;
        let x1 = self.droite().max(autre.droite());
        let y1 = self.bas().max(autre.bas());
        Rect {
            x: x0 as i32,
            y: y0 as i32,
            largeur: (x1 - x0) as u32,
            hauteur: (y1 - y0) as u32,
        }
    }

    pub fn contient(&self, x: i32, y: i32) -> bool {
        (x as i64) >= self.x as i64
            && (x as i64) < self.droite()
            && (y as i64) >= self.y as i64
            && (y as i64) < self.bas()
    }

    pub fn encode(&self) -> [u8; 16] {
        let mut octets = [0u8; 16];
        octets[0..4].copy_from_slice(&self.x.to_le_bytes());
        octets[4..8].copy_from_slice(&self.y.to_le_bytes());
        octets[8..12].copy_from_slice(&self.largeur.to_le_bytes());
        octets[12..16].copy_from_slice(&self.hauteur.to_le_bytes());
        octets
    }

    pub fn decode(octets: &[u8]) -> Option<Rect> {
        if octets.len() < 16 {
            return None;
        }
        Some(Rect {
            x: lit_i32(octets, 0),
            y: lit_i32(octets, 4),
            largeur: lit_u32(octets, 8),
            hauteur: lit_u32(octets, 12),
        })
    }
}

/// Rognage d'un degat annonce par un client a la surface qu'il possede.
///
/// Un client peut se tromper, ou mentir. Les deux se traitent pareil : le degat
/// est ramene a l'intersection avec la surface avant d'etre utilise comme
/// intervalle de copie. Sans ce rognage, un `Damage { x: -1 }` ferait lire le
/// compositeur avant le debut du tampon, et un `largeur: u32::MAX` ferait lire
/// bien au-dela — dans les deux cas depuis le noyau, avec les pages du noyau.
pub fn rogne_degat(degat: Rect, largeur_surface: u32, hauteur_surface: u32) -> Rect {
    degat.intersecte(&Rect::neuf(0, 0, largeur_surface, hauteur_surface))
}

/// Coordonnees ecran -> coordonnees locales a la zone utile d'une fenetre.
///
/// Rend `None` si le point tombe hors de la zone utile : c'est ce qui evite
/// d'envoyer au client un clic qui a eu lieu sur sa barre de titre, ou pire, un
/// clic negatif que Qt interpreterait comme un point valide tres loin.
pub fn vers_local(zone: &Rect, x_ecran: i32, y_ecran: i32) -> Option<(i32, i32)> {
    if !zone.contient(x_ecran, y_ecran) {
        return None;
    }
    Some((x_ecran - zone.x, y_ecran - zone.y))
}

/// Delta de molette PS/2 brut -> delta du protocole GUI.
///
/// Les deux comptent a l'envers l'un de l'autre, et rien dans le typage ne le
/// dit : ce sont deux `i32`. Le quatrieme octet d'un paquet IntelliMouse est
/// **negatif quand la molette tourne vers le haut** (loin de l'utilisateur) —
/// c'est ce que produit QEMU (`hw/input/ps2.c` : `WHEEL_UP` fait `mouse_dz--`)
/// et c'est ce que suppose Linux, qui publie `REL_WHEEL = -(signed char)
/// packet[3]` dans `drivers/input/mouse/psmouse-base.c`. Notre propre couche
/// evdev fait deja cette negation (`kernel::input::read_mouse`).
///
/// Le protocole GUI, lui, compte **positif vers le haut** (convention Qt, voir
/// [`Molette`]). Les trois consommateurs le lisent ainsi : `apps::wheel_to_app`
/// et `rustpad::on_wheel` font `scroll - delta`, et le pont M11 fait
/// `wheel_delta_y = -delta` pour retrouver la convention du DOM (positif vers
/// le bas). Passer l'octet brut sur le fil inversait donc tout defilement du
/// systeme : sur une page en haut de course, le geste « vers le bas » demandait
/// de remonter, et ne bougeait rien.
pub fn molette_depuis_ps2(brut: i32) -> i32 {
    -brut
}

// --- Charges utiles ----------------------------------------------------------

/// `Surface` : le gestionnaire de fenetres decrit au client la memoire qu'il
/// vient de lui donner. Envoye une fois, avant la premiere trame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Surface {
    pub fenetre: u32,
    pub tampon: u32,
    pub largeur: u32,
    pub hauteur: u32,
    pub pas: u32,
    /// 0 = XRGB8888 (le seul format du jalon 2).
    pub format: u32,
}

impl Surface {
    pub fn encode(&self) -> [u8; 24] {
        let mut o = [0u8; 24];
        o[0..4].copy_from_slice(&self.fenetre.to_le_bytes());
        o[4..8].copy_from_slice(&self.tampon.to_le_bytes());
        o[8..12].copy_from_slice(&self.largeur.to_le_bytes());
        o[12..16].copy_from_slice(&self.hauteur.to_le_bytes());
        o[16..20].copy_from_slice(&self.pas.to_le_bytes());
        o[20..24].copy_from_slice(&self.format.to_le_bytes());
        o
    }
    pub fn decode(o: &[u8]) -> Option<Surface> {
        if o.len() < 24 {
            return None;
        }
        Some(Surface {
            fenetre: lit_u32(o, 0),
            tampon: lit_u32(o, 4),
            largeur: lit_u32(o, 8),
            hauteur: lit_u32(o, 12),
            pas: lit_u32(o, 16),
            format: lit_u32(o, 20),
        })
    }
}

/// `Configure` : nouvelle geometrie de la zone utile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Configure {
    pub fenetre: u32,
    pub largeur: u32,
    pub hauteur: u32,
    /// 1 = la fenetre a le focus clavier, 0 sinon.
    pub focus: u32,
}

impl Configure {
    pub fn encode(&self) -> [u8; 16] {
        let mut o = [0u8; 16];
        o[0..4].copy_from_slice(&self.fenetre.to_le_bytes());
        o[4..8].copy_from_slice(&self.largeur.to_le_bytes());
        o[8..12].copy_from_slice(&self.hauteur.to_le_bytes());
        o[12..16].copy_from_slice(&self.focus.to_le_bytes());
        o
    }
    pub fn decode(o: &[u8]) -> Option<Configure> {
        if o.len() < 16 {
            return None;
        }
        Some(Configure {
            fenetre: lit_u32(o, 0),
            largeur: lit_u32(o, 4),
            hauteur: lit_u32(o, 8),
            focus: lit_u32(o, 12),
        })
    }
}

/// `Pointer` : position du curseur **dans la zone utile** et etat des boutons.
///
/// Il n'y a volontairement pas de delta : le gestionnaire de fenetres possede le
/// curseur, le client n'a pas a le deplacer lui-meme. Envoyer une position
/// absolue rend aussi le message idempotent — en perdre un ne desynchronise
/// rien, la position suivante corrige tout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pointeur {
    pub fenetre: u32,
    pub x: i32,
    pub y: i32,
    /// Masque : bit 0 gauche, bit 1 droit, bit 2 milieu.
    pub boutons: u32,
}

impl Pointeur {
    pub fn encode(&self) -> [u8; 16] {
        let mut o = [0u8; 16];
        o[0..4].copy_from_slice(&self.fenetre.to_le_bytes());
        o[4..8].copy_from_slice(&self.x.to_le_bytes());
        o[8..12].copy_from_slice(&self.y.to_le_bytes());
        o[12..16].copy_from_slice(&self.boutons.to_le_bytes());
        o
    }
    pub fn decode(o: &[u8]) -> Option<Pointeur> {
        if o.len() < 16 {
            return None;
        }
        Some(Pointeur {
            fenetre: lit_u32(o, 0),
            x: lit_i32(o, 4),
            y: lit_i32(o, 8),
            boutons: lit_u32(o, 12),
        })
    }
}

/// `Wheel` : cran de molette, positif vers le haut (convention Qt).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Molette {
    pub fenetre: u32,
    pub delta: i32,
    pub x: i32,
    pub y: i32,
}

impl Molette {
    pub fn encode(&self) -> [u8; 16] {
        let mut o = [0u8; 16];
        o[0..4].copy_from_slice(&self.fenetre.to_le_bytes());
        o[4..8].copy_from_slice(&self.delta.to_le_bytes());
        o[8..12].copy_from_slice(&self.x.to_le_bytes());
        o[12..16].copy_from_slice(&self.y.to_le_bytes());
        o
    }
    pub fn decode(o: &[u8]) -> Option<Molette> {
        if o.len() < 16 {
            return None;
        }
        Some(Molette {
            fenetre: lit_u32(o, 0),
            delta: lit_i32(o, 4),
            x: lit_i32(o, 8),
            y: lit_i32(o, 12),
        })
    }
}

/// `Key` : une touche, deja traduite par le gestionnaire de fenetres.
///
/// `unicode` vaut 0 pour les touches sans caractere (fleches, F1...). Le client
/// n'a donc pas a refaire de table de disposition clavier : il n'en a pas les
/// moyens, puisqu'il ne voit plus evdev.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Touche {
    pub fenetre: u32,
    /// Code de touche Linux (`KEY_*`), tel que le noyau le produit deja.
    pub code: u32,
    /// Masque : bit 0 majuscule, bit 1 controle, bit 2 alt.
    pub modificateurs: u32,
    /// Point de code Unicode, ou 0.
    pub unicode: u32,
    /// 1 = appui, 0 = relachement.
    pub appui: u32,
}

impl Touche {
    pub fn encode(&self) -> [u8; 20] {
        let mut o = [0u8; 20];
        o[0..4].copy_from_slice(&self.fenetre.to_le_bytes());
        o[4..8].copy_from_slice(&self.code.to_le_bytes());
        o[8..12].copy_from_slice(&self.modificateurs.to_le_bytes());
        o[12..16].copy_from_slice(&self.unicode.to_le_bytes());
        o[16..20].copy_from_slice(&self.appui.to_le_bytes());
        o
    }
    pub fn decode(o: &[u8]) -> Option<Touche> {
        if o.len() < 20 {
            return None;
        }
        Some(Touche {
            fenetre: lit_u32(o, 0),
            code: lit_u32(o, 4),
            modificateurs: lit_u32(o, 8),
            unicode: lit_u32(o, 12),
            appui: lit_u32(o, 16),
        })
    }
}

/// `FrameReady` : le client a fini d'ecrire dans la surface.
///
/// C'est le seul message qui autorise le compositeur a lire les pixels. Le
/// `tampon` designera le tampon arriere quand il y en aura deux ; aujourd'hui il
/// vaut toujours 0, et le champ existe pour que le passage au double tampon ne
/// change pas le format du fil.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Trame {
    pub fenetre: u32,
    pub tampon: u32,
    pub degat: Rect,
}

impl Trame {
    pub fn encode(&self) -> [u8; 24] {
        let mut o = [0u8; 24];
        o[0..4].copy_from_slice(&self.fenetre.to_le_bytes());
        o[4..8].copy_from_slice(&self.tampon.to_le_bytes());
        o[8..24].copy_from_slice(&self.degat.encode());
        o
    }
    pub fn decode(o: &[u8]) -> Option<Trame> {
        if o.len() < 24 {
            return None;
        }
        Some(Trame {
            fenetre: lit_u32(o, 0),
            tampon: lit_u32(o, 4),
            degat: Rect::decode(&o[8..24])?,
        })
    }
}

// --- Assemblage --------------------------------------------------------------

/// Prefixe une charge utile de son en-tete.
pub fn message(genre: Genre, serie: u32, charge: &[u8]) -> Vec<u8> {
    let mut octets = Vec::with_capacity(TAILLE_ENTETE + charge.len());
    octets.extend_from_slice(&Entete::neuf(genre, charge.len() as u32, serie).encode());
    octets.extend_from_slice(charge);
    octets
}

/// Ce qu'un decodeur peut conclure d'un tampon de reception.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lecture {
    /// Pas encore assez d'octets : rappeler apres la prochaine reception.
    Incomplet,
    /// Un message complet : (genre, debut de la charge, taille de la charge).
    Message { genre: Genre, debut: usize, taille: usize, total: usize },
    /// Flux invalide : la seule reponse saine est de fermer le client.
    Invalide,
}

/// Examine le debut d'un tampon de reception sans rien consommer.
///
/// Separer « analyser » de « consommer » est ce qui rend la contre-pression
/// simple a ecrire : l'appelant ne retire du tampon que ce qu'il a reellement
/// traite, et un message a cheval sur deux lectures ne se perd pas. C'est
/// exactement la lecon du canal du renderer, ou une trame reemise en entier
/// apres un `EAGAIN` corrompait le cadrage.
pub fn examine(tampon: &[u8]) -> Lecture {
    if tampon.len() < TAILLE_ENTETE {
        return Lecture::Incomplet;
    }
    let entete = match Entete::decode(tampon) {
        Some(entete) => entete,
        None => return Lecture::Incomplet,
    };
    if !entete.valide() || entete.taille_charge > CHARGE_MAX {
        return Lecture::Invalide;
    }
    let genre = match Genre::depuis_u16(entete.genre) {
        Some(genre) => genre,
        None => return Lecture::Invalide,
    };
    let taille = entete.taille_charge as usize;
    let total = TAILLE_ENTETE + taille;
    if tampon.len() < total {
        return Lecture::Incomplet;
    }
    Lecture::Message { genre, debut: TAILLE_ENTETE, taille, total }
}

// --- Petits lecteurs ---------------------------------------------------------

fn lit_u32(octets: &[u8], decalage: usize) -> u32 {
    u32::from_le_bytes([
        octets[decalage],
        octets[decalage + 1],
        octets[decalage + 2],
        octets[decalage + 3],
    ])
}

fn lit_i32(octets: &[u8], decalage: usize) -> i32 {
    lit_u32(octets, decalage) as i32
}

// --- Contrat de taille -------------------------------------------------------
//
// Ces assertions sont evaluees a la compilation : `cargo build` echoue si l'une
// d'elles cesse d'etre vraie. C'est la barriere qui empeche un champ ajoute d'un
// cote de partir en production sans son equivalent dans `hote.cpp`.

const _: () = assert!(TAILLE_ENTETE == 16);
// Les charges utiles sont encodees dans des tableaux de taille fixe : leur
// longueur est donc deja verifiee par le typage. Ce qui ne l'est pas, c'est que
// les deux cotes se soient mis d'accord sur ces longueurs — d'ou le tableau du
// document de protocole, et ces bornes-ci pour ce que le fil ne peut pas dire.
const _: () = assert!(CHARGE_MAX >= 24);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entete_aller_retour() {
        let entete = Entete::neuf(Genre::FrameReady, 24, 7);
        let decode = Entete::decode(&entete.encode()).unwrap();
        assert_eq!(entete, decode);
        assert!(decode.valide());
        assert_eq!(Genre::depuis_u16(decode.genre), Some(Genre::FrameReady));
    }

    #[test]
    fn un_flux_etranger_est_rejete() {
        let mut octets = Entete::neuf(Genre::Hello, 0, 0).encode();
        octets[0] ^= 0xFF;
        assert_eq!(examine(&octets), Lecture::Invalide);
    }

    #[test]
    fn une_charge_demesuree_est_rejetee() {
        let entete = Entete {
            magic: MAGIC,
            version: PROTOCOL_VERSION,
            genre: Genre::SetTitle as u16,
            taille_charge: u32::MAX,
            serie: 0,
        };
        assert_eq!(examine(&entete.encode()), Lecture::Invalide);
    }

    #[test]
    fn un_message_coupe_en_deux_attend() {
        let complet = message(Genre::FrameReady, 1, &[0u8; 24]);
        for coupe in 0..complet.len() {
            assert_eq!(examine(&complet[..coupe]), Lecture::Incomplet, "coupe={}", coupe);
        }
        match examine(&complet) {
            Lecture::Message { taille, total, .. } => {
                assert_eq!(taille, 24);
                assert_eq!(total, TAILLE_ENTETE + 24);
            }
            autre => panic!("attendu un message complet, obtenu {:?}", autre),
        }
    }

    #[test]
    fn les_charges_font_un_aller_retour() {
        let surface = Surface { fenetre: 1, tampon: 0, largeur: 800, hauteur: 600, pas: 3200, format: 0 };
        assert_eq!(Surface::decode(&surface.encode()), Some(surface));

        let pointeur = Pointeur { fenetre: 1, x: -3, y: 42, boutons: 0b101 };
        assert_eq!(Pointeur::decode(&pointeur.encode()), Some(pointeur));

        let touche = Touche { fenetre: 1, code: 30, modificateurs: 2, unicode: 'a' as u32, appui: 1 };
        assert_eq!(Touche::decode(&touche.encode()), Some(touche));

        let molette = Molette { fenetre: 1, delta: -120, x: 320, y: 240 };
        assert_eq!(Molette::decode(&molette.encode()), Some(molette));

        let trame = Trame { fenetre: 1, tampon: 0, degat: Rect::neuf(10, 20, 30, 40) };
        assert_eq!(Trame::decode(&trame.encode()), Some(trame));

        let configure = Configure { fenetre: 1, largeur: 1100, hauteur: 604, focus: 1 };
        assert_eq!(Configure::decode(&configure.encode()), Some(configure));
    }

    #[test]
    fn la_molette_ps2_est_retournee_pour_le_protocole() {
        // Molette vers le haut : le paquet PS/2 porte un delta negatif, le
        // protocole GUI demande un positif. Le test fixe le sens dans les deux
        // directions, parce qu'une inversion se lit pareil dans le typage.
        assert_eq!(molette_depuis_ps2(-1), 1, "un cran vers le haut");
        assert_eq!(molette_depuis_ps2(1), -1, "un cran vers le bas");
        assert_eq!(molette_depuis_ps2(0), 0);
        // Plusieurs crans accumules entre deux tours du bureau.
        assert_eq!(molette_depuis_ps2(-3), 3);

        // Et le sens attendu par le consommateur : un cran vers le haut doit
        // faire *diminuer* la position de defilement, comme `wheel_to_app` et
        // `rustpad::on_wheel` la calculent (`scroll - delta`).
        let scroll = 10i32;
        assert_eq!(scroll - molette_depuis_ps2(-1), 9, "vers le haut remonte");
        assert_eq!(scroll - molette_depuis_ps2(1), 11, "vers le bas descend");
    }

    #[test]
    fn un_degat_deborde_est_ramene_a_la_surface() {
        // Coin superieur gauche negatif.
        let rogne = rogne_degat(Rect::neuf(-10, -10, 50, 50), 100, 100);
        assert_eq!(rogne, Rect::neuf(0, 0, 40, 40));

        // Largeur absurde : le rectangle ne peut pas depasser la surface.
        let rogne = rogne_degat(Rect::neuf(90, 90, u32::MAX, u32::MAX), 100, 100);
        assert_eq!(rogne, Rect::neuf(90, 90, 10, 10));

        // Entierement hors surface : rien a recopier.
        assert!(rogne_degat(Rect::neuf(200, 0, 10, 10), 100, 100).vide());
        assert!(rogne_degat(Rect::neuf(-50, 0, 10, 10), 100, 100).vide());
    }

    #[test]
    fn l_union_accumule_les_degats() {
        let a = Rect::neuf(10, 10, 10, 10);
        let b = Rect::neuf(50, 5, 10, 10);
        assert_eq!(a.union(&b), Rect::neuf(10, 5, 50, 15));
        // Le rectangle vide est neutre : c'est ce qui permet de partir de
        // `Rect::default()` et d'accumuler sans cas particulier.
        assert_eq!(Rect::default().union(&a), a);
        assert_eq!(a.union(&Rect::default()), a);
    }

    #[test]
    fn les_coordonnees_ecran_deviennent_locales() {
        let zone = Rect::neuf(100, 50, 200, 100);
        assert_eq!(vers_local(&zone, 100, 50), Some((0, 0)));
        assert_eq!(vers_local(&zone, 299, 149), Some((199, 99)));
        // Les bords exclusifs : un clic sur la premiere colonne hors zone ne
        // doit pas devenir un clic sur la derniere colonne de la fenetre.
        assert_eq!(vers_local(&zone, 300, 100), None);
        assert_eq!(vers_local(&zone, 150, 149), Some((50, 99)));
        assert_eq!(vers_local(&zone, 150, 150), None);
        assert_eq!(vers_local(&zone, 99, 100), None);
    }
}
