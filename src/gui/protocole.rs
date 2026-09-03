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

// --- Echelle et coordonnees logiques -----------------------------------------
//
// BOUCHAUD_C12_ECHELLE_V1
//
// POURQUOI DEUX REPERES, ET POURQUOI MAINTENANT
//
// Le protocole ne connaissait qu'une unite : le pixel de la dalle. Cela tient
// tant que l'echelle vaut un. Des qu'elle ne la vaut plus -- un ecran dense, un
// bureau agrandi --, tout ce qui est exprime en pixels devient ambigu : une
// fenetre de 800 est-elle 800 pixels de dalle, ou 800 unites de mise en page
// que le compositeur multipliera ?
//
// Les deux reperes sont donc nommes :
//
//   * PHYSIQUE  -- pixels de la surface partagee, ce que le compositeur copie ;
//   * LOGIQUE   -- unites de mise en page, ce dont un client raisonne.
//
// L'echelle relie les deux. Elle est FRACTIONNAIRE, exprimee en cent-vingtiemes
// comme le fait Wayland (`wp_fractional_scale_v1`) : 120 = 1,0 ; 180 = 1,5 ;
// 240 = 2,0. Les cent-vingtiemes ne sont pas un caprice -- 120 se divise par 2,
// 3, 4, 5, 6 et 8, donc les echelles usuelles (1,25 / 1,5 / 2 / 3) tombent
// juste et n'accumulent aucune derive d'arrondi.
//
// # Retrocompatibilite
//
// Aucun genre de message n'est ajoute, et aucun champ n'est deplace. `Surface`
// et `Configure` s'ALLONGENT, et leurs decodeurs acceptent l'ancienne longueur
// en supposant l'echelle unite. Un client d'avant ce commit lit les champs
// qu'il connait et ignore la queue ; un client d'apres, face a un vieux
// compositeur, se comporte comme si l'echelle valait un -- ce qui etait
// exactement le cas.
//
// # La regle d'arrondi
//
// Une conversion de RECTANGLE arrondit toujours VERS L'EXTERIEUR. Un degat
// converti trop petit laisse a l'ecran une bande de pixels perimee jusqu'a la
// trame suivante ; un degat converti trop grand recopie quelques pixels
// inchanges. Les deux erreurs ne se valent pas, et le sens de l'arrondi est le
// seul endroit ou cela se decide.

/// Valeur de l'echelle qui signifie « un pixel logique = un pixel physique ».
pub const ECHELLE_UNITE: u32 = 120;

/// Echelle minimale acceptee. Zero ferait une division par zero, et une echelle
/// plus petite que 1/4 rendrait une fenetre illisible avant de rendre un bogue
/// visible.
pub const ECHELLE_MIN: u32 = 30;

/// Echelle maximale acceptee (8,0). Au-dela, une fenetre logique de taille
/// ordinaire demanderait une surface que l'arene ne sait pas donner.
pub const ECHELLE_MAX: u32 = 960;

/// Ramene une echelle recue dans les bornes, en repliant sur l'unite ce qui n'a
/// aucun sens.
///
/// Un client ou un compositeur peuvent se tromper ; les deux se traitent
/// pareil. Zero est le cas qui compte : il vient d'un champ absent d'un vieux
/// message, et il doit se lire « echelle unite », jamais « division par zero ».
pub const fn echelle_valide(echelle: u32) -> u32 {
    if echelle < ECHELLE_MIN || echelle > ECHELLE_MAX {
        ECHELLE_UNITE
    } else {
        echelle
    }
}

/// Longueur logique -> longueur physique, arrondie vers le HAUT.
///
/// Vers le haut : une surface d'un pixel trop courte laisse la derniere colonne
/// de la mise en page hors de la fenetre.
pub const fn longueur_physique(logique: u32, echelle: u32) -> u32 {
    let e = echelle_valide(echelle) as u64;
    let produit = logique as u64 * e + (ECHELLE_UNITE as u64 - 1);
    let valeur = produit / ECHELLE_UNITE as u64;
    if valeur > u32::MAX as u64 { u32::MAX } else { valeur as u32 }
}

/// Longueur physique -> longueur logique, arrondie vers le BAS.
///
/// Vers le bas : la taille logique annoncee a un client doit tenir dans la
/// surface qu'on lui donne. L'arrondir vers le haut lui ferait dessiner une
/// colonne qui n'existe pas.
pub const fn longueur_logique(physique: u32, echelle: u32) -> u32 {
    let e = echelle_valide(echelle) as u64;
    ((physique as u64 * ECHELLE_UNITE as u64) / e) as u32
}

/// Coordonnee logique -> physique, arrondie vers zero pour un POINT.
///
/// Un point n'a pas d'epaisseur : l'arrondir vers l'exterieur le deplacerait.
/// C'est la conversion des evenements d'entree, pas celle des rectangles.
pub const fn point_physique(logique: i32, echelle: u32) -> i32 {
    let e = echelle_valide(echelle) as i64;
    ((logique as i64 * e) / ECHELLE_UNITE as i64) as i32
}

/// Coordonnee physique -> logique, arrondie vers zero.
pub const fn point_logique(physique: i32, echelle: u32) -> i32 {
    let e = echelle_valide(echelle) as i64;
    ((physique as i64 * ECHELLE_UNITE as i64) / e) as i32
}

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

    /// Rectangle logique -> rectangle physique, arrondi VERS L'EXTERIEUR.
    ///
    /// Le bord gauche descend, le bord droit monte. Un degat converti trop
    /// petit laisse a l'ecran une bande de pixels perimee jusqu'a la trame
    /// suivante ; converti trop grand, il recopie quelques pixels inchanges.
    /// Les deux erreurs ne se valent pas.
    pub fn vers_physique(&self, echelle: u32) -> Rect {
        if self.vide() {
            return Rect::default();
        }
        let e = echelle_valide(echelle) as i64;
        let u = ECHELLE_UNITE as i64;
        let x0 = div_plancher(self.x as i64 * e, u);
        let y0 = div_plancher(self.y as i64 * e, u);
        let x1 = div_plafond(self.droite() * e, u);
        let y1 = div_plafond(self.bas() * e, u);
        Rect {
            x: borne_i32(x0),
            y: borne_i32(y0),
            largeur: borne_u32(x1 - x0),
            hauteur: borne_u32(y1 - y0),
        }
    }

    /// Rectangle physique -> rectangle logique, arrondi VERS L'EXTERIEUR.
    ///
    /// Meme raison, sens inverse : un client qui apprend qu'un rectangle
    /// logique est sale doit redessiner au moins ce qui l'est.
    pub fn vers_logique(&self, echelle: u32) -> Rect {
        if self.vide() {
            return Rect::default();
        }
        let e = echelle_valide(echelle) as i64;
        let u = ECHELLE_UNITE as i64;
        let x0 = div_plancher(self.x as i64 * u, e);
        let y0 = div_plancher(self.y as i64 * u, e);
        let x1 = div_plafond(self.droite() * u, e);
        let y1 = div_plafond(self.bas() * u, e);
        Rect {
            x: borne_i32(x0),
            y: borne_i32(y0),
            largeur: borne_u32(x1 - x0),
            hauteur: borne_u32(y1 - y0),
        }
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface {
    pub fenetre: u32,
    pub tampon: u32,
    /// Largeur PHYSIQUE, en pixels de la surface partagee.
    pub largeur: u32,
    /// Hauteur PHYSIQUE.
    pub hauteur: u32,
    pub pas: u32,
    /// 0 = XRGB8888 (le seul format du jalon 2).
    pub format: u32,
    /// Echelle, en cent-vingtiemes (120 = 1,0). Voir [`ECHELLE_UNITE`].
    ///
    /// Le champ est en QUEUE de charge utile : un decodeur d'avant le
    /// chantier 12 lit les vingt-quatre premiers octets et ignore le reste,
    /// ce qui est exactement le comportement voulu -- il travaillait deja a
    /// l'echelle unite.
    pub echelle: u32,
    /// Reserve, zero. Garde la charge sur un multiple de huit octets et evite
    /// d'avoir a rallonger de nouveau pour le prochain champ.
    pub reserve: u32,
}

/// Taille de la charge `Surface` avant le chantier 12.
pub const TAILLE_SURFACE_V1: usize = 24;
/// Taille de la charge `Surface` avec l'echelle.
pub const TAILLE_SURFACE: usize = 32;

impl Default for Surface {
    /// L'echelle par defaut est l'UNITE, pas zero : une structure construite
    /// par defaut doit decrire une surface utilisable.
    fn default() -> Self {
        Self {
            fenetre: 0,
            tampon: 0,
            largeur: 0,
            hauteur: 0,
            pas: 0,
            format: 0,
            echelle: ECHELLE_UNITE,
            reserve: 0,
        }
    }
}

impl Surface {
    pub fn encode(&self) -> [u8; TAILLE_SURFACE] {
        let mut o = [0u8; TAILLE_SURFACE];
        o[0..4].copy_from_slice(&self.fenetre.to_le_bytes());
        o[4..8].copy_from_slice(&self.tampon.to_le_bytes());
        o[8..12].copy_from_slice(&self.largeur.to_le_bytes());
        o[12..16].copy_from_slice(&self.hauteur.to_le_bytes());
        o[16..20].copy_from_slice(&self.pas.to_le_bytes());
        o[20..24].copy_from_slice(&self.format.to_le_bytes());
        o[24..28].copy_from_slice(&echelle_valide(self.echelle).to_le_bytes());
        o[28..32].copy_from_slice(&self.reserve.to_le_bytes());
        o
    }

    /// Decode une charge `Surface`, ancienne longueur comprise.
    ///
    /// Une charge de vingt-quatre octets vient d'un compositeur d'avant le
    /// chantier 12 : elle decrit une surface a l'echelle unite, et c'est ce
    /// qu'on en conclut. Refuser cette longueur casserait le seul client C++
    /// qui existe.
    pub fn decode(o: &[u8]) -> Option<Surface> {
        if o.len() < TAILLE_SURFACE_V1 {
            return None;
        }
        let echelle = if o.len() >= TAILLE_SURFACE {
            echelle_valide(lit_u32(o, 24))
        } else {
            ECHELLE_UNITE
        };
        Some(Surface {
            fenetre: lit_u32(o, 0),
            tampon: lit_u32(o, 4),
            largeur: lit_u32(o, 8),
            hauteur: lit_u32(o, 12),
            pas: lit_u32(o, 16),
            format: lit_u32(o, 20),
            echelle,
            reserve: if o.len() >= TAILLE_SURFACE { lit_u32(o, 28) } else { 0 },
        })
    }

    /// La taille LOGIQUE de cette surface : ce dont le client raisonne.
    pub fn taille_logique(&self) -> (u32, u32) {
        (
            longueur_logique(self.largeur, self.echelle),
            longueur_logique(self.hauteur, self.echelle),
        )
    }
}

/// `Configure` : nouvelle geometrie de la zone utile.
///
/// `largeur`/`hauteur` restent la geometrie PHYSIQUE -- c'est ce que le champ
/// voulait dire avant le chantier 12, et le changer de sens aurait casse le
/// client C++ en silence. L'echelle et la geometrie LOGIQUE s'ajoutent en
/// queue, la ou un vieux decodeur ne les lit pas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Configure {
    pub fenetre: u32,
    /// Largeur PHYSIQUE de la zone utile.
    pub largeur: u32,
    /// Hauteur PHYSIQUE.
    pub hauteur: u32,
    /// 1 = la fenetre a le focus clavier, 0 sinon.
    pub focus: u32,
    /// Echelle en cent-vingtiemes (120 = 1,0).
    pub echelle: u32,
    /// Reserve, zero.
    pub reserve: u32,
}

/// Taille de la charge `Configure` avant le chantier 12.
pub const TAILLE_CONFIGURE_V1: usize = 16;
/// Taille de la charge `Configure` avec l'echelle.
pub const TAILLE_CONFIGURE: usize = 24;

impl Default for Configure {
    fn default() -> Self {
        Self {
            fenetre: 0,
            largeur: 0,
            hauteur: 0,
            focus: 0,
            echelle: ECHELLE_UNITE,
            reserve: 0,
        }
    }
}

impl Configure {
    pub fn encode(&self) -> [u8; TAILLE_CONFIGURE] {
        let mut o = [0u8; TAILLE_CONFIGURE];
        o[0..4].copy_from_slice(&self.fenetre.to_le_bytes());
        o[4..8].copy_from_slice(&self.largeur.to_le_bytes());
        o[8..12].copy_from_slice(&self.hauteur.to_le_bytes());
        o[12..16].copy_from_slice(&self.focus.to_le_bytes());
        o[16..20].copy_from_slice(&echelle_valide(self.echelle).to_le_bytes());
        o[20..24].copy_from_slice(&self.reserve.to_le_bytes());
        o
    }
    pub fn decode(o: &[u8]) -> Option<Configure> {
        if o.len() < TAILLE_CONFIGURE_V1 {
            return None;
        }
        Some(Configure {
            fenetre: lit_u32(o, 0),
            largeur: lit_u32(o, 4),
            hauteur: lit_u32(o, 8),
            focus: lit_u32(o, 12),
            echelle: if o.len() >= TAILLE_CONFIGURE {
                echelle_valide(lit_u32(o, 16))
            } else {
                ECHELLE_UNITE
            },
            reserve: if o.len() >= TAILLE_CONFIGURE { lit_u32(o, 20) } else { 0 },
        })
    }

    /// La geometrie LOGIQUE de la zone utile.
    pub fn taille_logique(&self) -> (u32, u32) {
        (
            longueur_logique(self.largeur, self.echelle),
            longueur_logique(self.hauteur, self.echelle),
        )
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

/// Division entiere arrondie vers moins l'infini.
///
/// `/` en Rust tronque VERS ZERO : `-1 / 2` vaut 0, pas -1. Pour un bord
/// gauche negatif -- une fenetre qui deborde a gauche de l'ecran --, tronquer
/// vers zero DECALE le rectangle vers la droite et laisse une colonne perimee.
const fn div_plancher(numerateur: i64, denominateur: i64) -> i64 {
    let quotient = numerateur / denominateur;
    if numerateur % denominateur != 0 && (numerateur < 0) != (denominateur < 0) {
        quotient - 1
    } else {
        quotient
    }
}

/// Division entiere arrondie vers plus l'infini.
const fn div_plafond(numerateur: i64, denominateur: i64) -> i64 {
    let quotient = numerateur / denominateur;
    if numerateur % denominateur != 0 && (numerateur < 0) == (denominateur < 0) {
        quotient + 1
    } else {
        quotient
    }
}

const fn borne_i32(valeur: i64) -> i32 {
    if valeur < i32::MIN as i64 {
        i32::MIN
    } else if valeur > i32::MAX as i64 {
        i32::MAX
    } else {
        valeur as i32
    }
}

const fn borne_u32(valeur: i64) -> u32 {
    if valeur < 0 {
        0
    } else if valeur > u32::MAX as i64 {
        u32::MAX
    } else {
        valeur as u32
    }
}

// --- Contrat de taille -------------------------------------------------------
//
// Ces assertions sont evaluees a la compilation : `cargo build` echoue si l'une
// d'elles cesse d'etre vraie. C'est la barriere qui empeche un champ ajoute d'un
// cote de partir en production sans son equivalent dans `hote.cpp`.

const _: () = assert!(TAILLE_ENTETE == 16);
// Les charges rallongees par le chantier 12. Leur PREFIXE doit rester celui de
// la version 1 : c'est ce qui permet a `hote.cpp`, qui ne lit pas la queue, de
// continuer a fonctionner sans etre recompile.
const _: () = assert!(TAILLE_SURFACE > TAILLE_SURFACE_V1);
const _: () = assert!(TAILLE_CONFIGURE > TAILLE_CONFIGURE_V1);
const _: () = assert!(TAILLE_SURFACE_V1 == 24);
const _: () = assert!(TAILLE_CONFIGURE_V1 == 16);
const _: () = assert!(ECHELLE_UNITE == 120);
// Les charges utiles sont encodees dans des tableaux de taille fixe : leur
// longueur est donc deja verifiee par le typage. Ce qui ne l'est pas, c'est que
// les deux cotes se soient mis d'accord sur ces longueurs — d'ou le tableau du
// document de protocole, et ces bornes-ci pour ce que le fil ne peut pas dire.
const _: () = assert!(CHARGE_MAX as usize >= TAILLE_SURFACE);

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
        let surface = Surface { fenetre: 1, tampon: 0, largeur: 800, hauteur: 600, pas: 3200, format: 0, echelle: ECHELLE_UNITE, reserve: 0 };
        assert_eq!(Surface::decode(&surface.encode()), Some(surface));

        let pointeur = Pointeur { fenetre: 1, x: -3, y: 42, boutons: 0b101 };
        assert_eq!(Pointeur::decode(&pointeur.encode()), Some(pointeur));

        let touche = Touche { fenetre: 1, code: 30, modificateurs: 2, unicode: 'a' as u32, appui: 1 };
        assert_eq!(Touche::decode(&touche.encode()), Some(touche));

        let molette = Molette { fenetre: 1, delta: -120, x: 320, y: 240 };
        assert_eq!(Molette::decode(&molette.encode()), Some(molette));

        let trame = Trame { fenetre: 1, tampon: 0, degat: Rect::neuf(10, 20, 30, 40) };
        assert_eq!(Trame::decode(&trame.encode()), Some(trame));

        let configure = Configure { fenetre: 1, largeur: 1100, hauteur: 604, focus: 1, echelle: ECHELLE_UNITE, reserve: 0 };
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

    // --- Chantier 12 : echelle et coordonnees logiques ----------------------

    #[test]
    fn une_echelle_absurde_se_replie_sur_l_unite() {
        // Zero vient d'un champ ABSENT d'un vieux message : il doit se lire
        // « echelle unite », jamais provoquer une division par zero.
        assert_eq!(echelle_valide(0), ECHELLE_UNITE);
        assert_eq!(echelle_valide(1), ECHELLE_UNITE);
        assert_eq!(echelle_valide(u32::MAX), ECHELLE_UNITE);
        // Les valeurs raisonnables passent telles quelles.
        assert_eq!(echelle_valide(ECHELLE_UNITE), ECHELLE_UNITE);
        assert_eq!(echelle_valide(180), 180);
        assert_eq!(echelle_valide(240), 240);
        assert_eq!(echelle_valide(ECHELLE_MIN), ECHELLE_MIN);
        assert_eq!(echelle_valide(ECHELLE_MAX), ECHELLE_MAX);
    }

    /// Les echelles usuelles tombent JUSTE en cent-vingtiemes. C'est la raison
    /// d'etre de cette unite : 120 se divise par 2, 3, 4, 5, 6 et 8.
    #[test]
    fn les_echelles_usuelles_ne_derivent_pas() {
        for (echelle, logique, physique) in [
            (120u32, 800u32, 800u32),   // 1,0
            (150, 800, 1000),           // 1,25
            (180, 800, 1200),           // 1,5
            (240, 800, 1600),           // 2,0
            (360, 800, 2400),           // 3,0
        ] {
            assert_eq!(
                longueur_physique(logique, echelle), physique,
                "echelle {echelle} : {logique} logiques devraient faire {physique} physiques"
            );
            assert_eq!(
                longueur_logique(physique, echelle), logique,
                "et le retour doit etre exact"
            );
        }
    }

    /// Une longueur logique arrondit vers le HAUT, une longueur physique vers le
    /// BAS. Ce n'est pas une symetrie ratee : c'est la seule paire qui garantit
    /// qu'une surface contient toujours la mise en page qu'on y dessine.
    #[test]
    fn les_longueurs_arrondissent_du_bon_cote() {
        // 100 logiques a 1,5 = 125 exactement.
        assert_eq!(longueur_physique(100, 180), 150);
        // 7 logiques a 1,25 = 8,75 -> 9 physiques, pas 8.
        assert_eq!(longueur_physique(7, 150), 9);
        // 9 physiques a 1,25 = 7,2 logiques -> 7, pas 8 : le client ne doit pas
        // croire qu'il dispose d'une colonne qui n'existe pas.
        assert_eq!(longueur_logique(9, 150), 7);
    }

    /// Un POINT n'a pas d'epaisseur : l'arrondir vers l'exterieur le
    /// deplacerait. C'est la conversion des evenements d'entree.
    #[test]
    fn un_point_ne_se_deplace_pas() {
        assert_eq!(point_physique(0, 240), 0);
        assert_eq!(point_physique(10, 240), 20);
        assert_eq!(point_logique(20, 240), 10);
        assert_eq!(point_logique(21, 240), 10);
        // Coordonnees negatives : un pointeur peut sortir de la zone utile,
        // et le message le dit avec un nombre negatif.
        assert_eq!(point_physique(-10, 240), -20);
        assert_eq!(point_logique(-20, 240), -10);
    }

    /// Un RECTANGLE arrondit vers l'exterieur, dans les deux sens.
    ///
    /// Un degat converti trop petit laisse a l'ecran une bande de pixels
    /// perimee jusqu'a la trame suivante ; converti trop grand, il recopie
    /// quelques pixels inchanges. Les deux erreurs ne se valent pas.
    #[test]
    fn un_rectangle_de_degat_arrondit_vers_l_exterieur() {
        // 1,25 : un rectangle logique dont les bords ne tombent pas juste.
        let logique = Rect::neuf(1, 1, 3, 3); // bords logiques 1..4
        let physique = logique.vers_physique(150);
        // 1 * 1,25 = 1,25 -> 1 (plancher) ; 4 * 1,25 = 5 -> 5 (plafond).
        assert_eq!(physique, Rect::neuf(1, 1, 4, 4));
        assert!(
            physique.droite() >= 5,
            "le bord droit physique doit couvrir tout le bord logique"
        );

        // Et le retour recouvre au moins le rectangle de depart.
        let retour = physique.vers_logique(150);
        assert!(retour.x <= logique.x);
        assert!(retour.droite() >= logique.droite());
        assert!(retour.y <= logique.y);
        assert!(retour.bas() >= logique.bas());
    }

    /// Un bord GAUCHE NEGATIF -- une fenetre qui deborde a gauche de l'ecran --
    /// doit descendre, pas remonter.
    ///
    /// `/` tronque vers zero en Rust : `-1 / 2` vaut 0, pas -1. Sans division
    /// plancher, le rectangle se decalerait vers la droite et laisserait une
    /// colonne perimee sur son bord gauche.
    #[test]
    fn un_bord_negatif_ne_se_decale_pas_vers_la_droite() {
        let logique = Rect::neuf(-1, -1, 3, 3);
        let physique = logique.vers_physique(150); // 1,25
        assert!(
            physique.x <= -2,
            "-1 logique a 1,25 vaut -1,25 : le bord physique doit etre -2, pas -1 ({physique:?})"
        );
        assert!(
            physique.droite() >= 3,
            "le bord droit doit couvrir 2 * 1,25 = 2,5 -> 3 ({physique:?})"
        );
    }

    /// Une conversion a l'echelle unite ne doit RIEN changer.
    #[test]
    fn l_echelle_unite_est_l_identite() {
        for rect in [
            Rect::neuf(0, 0, 100, 100),
            Rect::neuf(-5, 7, 13, 29),
            Rect::neuf(1920, 1080, 1, 1),
        ] {
            assert_eq!(rect.vers_physique(ECHELLE_UNITE), rect);
            assert_eq!(rect.vers_logique(ECHELLE_UNITE), rect);
            // Et l'echelle absente se comporte pareil.
            assert_eq!(rect.vers_physique(0), rect);
        }
    }

    /// Un rectangle vide reste vide : c'est ce qui permet d'accumuler des
    /// degats depuis `Rect::default()` sans cas particulier.
    #[test]
    fn un_rectangle_vide_reste_vide_a_toute_echelle() {
        for echelle in [0u32, ECHELLE_UNITE, 240, ECHELLE_MAX] {
            assert!(Rect::default().vers_physique(echelle).vide());
            assert!(Rect::default().vers_logique(echelle).vide());
        }
    }

    /// Le rognage doit s'appliquer APRES la conversion, et la conversion ne
    /// doit jamais fabriquer un rectangle qui deborde la surface.
    #[test]
    fn un_degat_logique_converti_puis_rogne_reste_dans_la_surface() {
        // Une surface physique de 1000x1000 a 1,25, donc 800x800 logiques.
        let echelle = 150u32;
        let (lw, lh) = (longueur_logique(1000, echelle), longueur_logique(1000, echelle));
        assert_eq!((lw, lh), (800, 800));

        // Un client qui salit tout son espace logique, et meme au-dela.
        let degat_logique = Rect::neuf(-10, -10, 5000, 5000);
        let physique = degat_logique.vers_physique(echelle);
        let rogne = rogne_degat(physique, 1000, 1000);
        assert_eq!(
            rogne, Rect::neuf(0, 0, 1000, 1000),
            "le degat doit couvrir toute la surface, et pas un pixel de plus"
        );
        assert!(rogne.droite() <= 1000);
        assert!(rogne.bas() <= 1000);
    }

    /// Un client hostile ne doit pas obtenir un rectangle qui deborde par
    /// l'arithmetique de l'echelle.
    #[test]
    fn une_echelle_maximale_ne_fait_pas_deborder() {
        let enorme = Rect::neuf(i32::MAX - 1, i32::MAX - 1, u32::MAX, u32::MAX);
        let physique = enorme.vers_physique(ECHELLE_MAX);
        // Bornage, pas debordement : le rectangle reste representable, et le
        // rognage le ramene ensuite a la surface.
        assert!(rogne_degat(physique, 800, 600).largeur <= 800);
        assert!(rogne_degat(physique, 800, 600).hauteur <= 600);
    }

    // --- Retrocompatibilite du fil ------------------------------------------

    /// Une charge `Surface` d'AVANT le chantier 12 doit se decoder, et se lire
    /// comme une surface a l'echelle unite.
    #[test]
    fn une_surface_de_l_ancien_format_se_decode_a_l_echelle_unite() {
        let mut ancienne = [0u8; TAILLE_SURFACE_V1];
        ancienne[0..4].copy_from_slice(&1u32.to_le_bytes());
        ancienne[8..12].copy_from_slice(&800u32.to_le_bytes());
        ancienne[12..16].copy_from_slice(&600u32.to_le_bytes());
        ancienne[16..20].copy_from_slice(&3200u32.to_le_bytes());

        let surface = Surface::decode(&ancienne).expect("l'ancien format doit rester lisible");
        assert_eq!(surface.largeur, 800);
        assert_eq!(surface.hauteur, 600);
        assert_eq!(
            surface.echelle, ECHELLE_UNITE,
            "sans champ d'echelle, la surface est a l'echelle unite"
        );
        assert_eq!(surface.taille_logique(), (800, 600));
    }

    /// Le PREFIXE de la nouvelle charge doit etre bit pour bit celui de
    /// l'ancienne : c'est ce qui permet a `hote.cpp`, qui lit les
    /// vingt-quatre premiers octets et ignore la queue, de continuer sans etre
    /// recompile.
    #[test]
    fn la_nouvelle_surface_commence_par_l_ancienne() {
        let surface = Surface {
            fenetre: 1, tampon: 0, largeur: 1100, hauteur: 604,
            pas: 4400, format: 0, echelle: 240, reserve: 0,
        };
        let encode = surface.encode();
        assert_eq!(encode.len(), TAILLE_SURFACE);

        // Ce qu'un vieux decodeur lit.
        let vieux = Surface::decode(&encode[..TAILLE_SURFACE_V1]).unwrap();
        assert_eq!(vieux.largeur, 1100);
        assert_eq!(vieux.hauteur, 604);
        assert_eq!(vieux.pas, 4400);
        assert_eq!(vieux.echelle, ECHELLE_UNITE);

        // Ce qu'un decodeur a jour lit.
        let neuf = Surface::decode(&encode).unwrap();
        assert_eq!(neuf, surface);
        assert_eq!(neuf.taille_logique(), (550, 302));
    }

    #[test]
    fn une_configure_de_l_ancien_format_se_decode_a_l_echelle_unite() {
        let mut ancienne = [0u8; TAILLE_CONFIGURE_V1];
        ancienne[0..4].copy_from_slice(&1u32.to_le_bytes());
        ancienne[4..8].copy_from_slice(&1100u32.to_le_bytes());
        ancienne[8..12].copy_from_slice(&604u32.to_le_bytes());
        ancienne[12..16].copy_from_slice(&1u32.to_le_bytes());

        let conf = Configure::decode(&ancienne).expect("l'ancien format doit rester lisible");
        assert_eq!(conf.largeur, 1100);
        assert_eq!(conf.focus, 1);
        assert_eq!(conf.echelle, ECHELLE_UNITE);
        assert_eq!(conf.taille_logique(), (1100, 604));
    }

    #[test]
    fn la_nouvelle_configure_commence_par_l_ancienne() {
        let conf = Configure {
            fenetre: 1, largeur: 1600, hauteur: 1200, focus: 1,
            echelle: 240, reserve: 0,
        };
        let encode = conf.encode();
        let vieux = Configure::decode(&encode[..TAILLE_CONFIGURE_V1]).unwrap();
        assert_eq!(vieux.largeur, 1600);
        assert_eq!(vieux.focus, 1);
        assert_eq!(vieux.echelle, ECHELLE_UNITE);
        assert_eq!(Configure::decode(&encode).unwrap(), conf);
        assert_eq!(conf.taille_logique(), (800, 600));
    }

    /// Une charge trop courte reste refusee : la tolerance porte sur la QUEUE,
    /// pas sur le prefixe.
    #[test]
    fn une_charge_tronquee_reste_refusee() {
        let surface = Surface::default().encode();
        for coupe in 0..TAILLE_SURFACE_V1 {
            assert!(
                Surface::decode(&surface[..coupe]).is_none(),
                "une charge de {coupe} octets ne decrit pas une surface"
            );
        }
        let conf = Configure::default().encode();
        for coupe in 0..TAILLE_CONFIGURE_V1 {
            assert!(Configure::decode(&conf[..coupe]).is_none());
        }
    }

    /// Une echelle absurde recue sur le FIL est repliee au decodage, pas
    /// propagee.
    #[test]
    fn une_echelle_absurde_recue_sur_le_fil_est_repliee() {
        let mut octets = Surface::default().encode();
        octets[24..28].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(Surface::decode(&octets).unwrap().echelle, ECHELLE_UNITE);

        let mut octets = Configure::default().encode();
        octets[16..20].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(Configure::decode(&octets).unwrap().echelle, ECHELLE_UNITE);
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
