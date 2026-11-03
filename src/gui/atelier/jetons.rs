//! Jetons Atelier (BOUCHAUD_ATELIER_JETONS_V1).
//!
//! Pur : aucune dependance au noyau, inclus tel quel par les tests d'hote.
//!
//! Une couleur est `0xRRGGBB` ; une opacite est un `u8` (0 transparent, 255
//! opaque). Une matiere translucide est le couple des deux : elle se MELANGE
//! sur ce qui est deja peint, elle ne le remplace pas.

// ─── Encre (texte et pictogrammes sur matiere claire) ─────────────────────

/// Texte principal : un ardoise profond, jamais un noir pur.
pub const ENCRE: u32 = 0x1d2130;
/// Texte secondaire : libelles, metadonnees.
pub const ENCRE_DOUCE: u32 = 0x585e73;
/// Texte tertiaire : etats inactifs, pictogrammes au repos.
pub const ENCRE_TENUE: u32 = 0x8b90a3;
/// Texte pose directement sur le fond d'ecran.
pub const ENCRE_SUR_FOND: u32 = 0xffffff;

// ─── Matieres ─────────────────────────────────────────────────────────────

/// Verre : blanc translucide pose sur le fond d'ecran.
pub const VERRE: u32 = 0xffffff;
/// Opacite du verre de la barre du haut.
pub const VERRE_BARRE: u8 = 0x9e;
/// Opacite du verre du dock : un peu plus dense, il porte des icones.
pub const VERRE_DOCK: u8 = 0xb4;
/// Liseré lumineux en haut d'une plaque de verre.
pub const VERRE_LISERE: u8 = 0xd8;
/// Filet sombre qui detache une plaque de verre du fond.
pub const FILET: u32 = 0x1d2130;
pub const FILET_OPACITE: u8 = 0x1c;

/// Surface d'une fenetre : un blanc casse tres legerement froid.
pub const SURFACE: u32 = 0xf8f8fb;
/// Barre de titre d'une fenetre active.
pub const TITRE_ACTIF: u32 = 0xf1f1f6;
/// Barre de titre d'une fenetre inactive : plus proche de la surface.
pub const TITRE_INACTIF: u32 = 0xf6f6f9;
/// Filet de bordure d'une fenetre.
pub const BORDURE_FENETRE: u32 = 0xd5d6e0;
/// Separateur entre barre de titre et contenu.
pub const SEPARATEUR: u32 = 0xe4e5ec;

/// Survol d'un bouton de fenetre ou d'une cellule de dock.
pub const SURVOL: u32 = 0x1d2130;
pub const SURVOL_OPACITE: u8 = 0x14;

// ─── Accent et etats ──────────────────────────────────────────────────────

/// Accent Atelier : un indigo doux, celui de la maquette retenue.
pub const ACCENT: u32 = 0x5b5bd6;
/// Danger : la fermeture survolee.
pub const DANGER: u32 = 0xe5484d;

// ─── Ombre ────────────────────────────────────────────────────────────────

/// Teinte de l'ombre : un bleu nuit, pas un noir -- une ombre noire sur un
/// fond chaud se lit comme de la salete.
pub const OMBRE: u32 = 0x161a33;
/// Opacite maximale de l'ombre d'une fenetre active, au contact du cadre.
pub const OMBRE_FENETRE_ACTIVE: u8 = 0x30;
/// Idem, fenetre inactive : elle recule.
pub const OMBRE_FENETRE_INACTIVE: u8 = 0x1a;
/// Ombre du dock et du menu.
pub const OMBRE_FLOTTANT: u8 = 0x26;

// ─── Rayons ───────────────────────────────────────────────────────────────

/// Rayon d'une fenetre. Egal a `windowing::WINDOW_RADIUS` : la geometrie de
/// rendu et le test d'impact en dependent.
pub const RAYON_FENETRE: u32 = 10;
/// Rayon du dock : une pastille, pas une barre.
pub const RAYON_DOCK: u32 = 20;
/// Rayon d'une cellule du dock et d'un bouton.
pub const RAYON_CELLULE: u32 = 12;
/// Rayon d'une pastille (indicateur, petit bouton).
pub const RAYON_PASTILLE: u32 = 6;

// ─── Espacements (echelle de 4) ───────────────────────────────────────────

pub const ESPACE_1: i32 = 4;
pub const ESPACE_2: i32 = 8;
pub const ESPACE_3: i32 = 12;
pub const ESPACE_4: i32 = 16;
pub const ESPACE_6: i32 = 24;

// ─── Typographie (DejaVu Sans, corps en pixels) ───────────────────────────

/// Barre du haut.
pub const CORPS_BARRE: f32 = 13.0;
/// Titre de fenetre.
pub const CORPS_TITRE: f32 = 12.0;
/// Libelle d'icone et texte courant des widgets.
pub const CORPS_LIBELLE: f32 = 12.0;
/// Titre du bureau (marque au centre du fond).
pub const CORPS_MARQUE: f32 = 40.0;
/// Sous-titre du bureau.
pub const CORPS_SOUS_MARQUE: f32 = 18.0;

// ─── Mouvement ────────────────────────────────────────────────────────────

/// Retour d'etat immediat : survol, appui.
pub const DUREE_RAPIDE_MS: u32 = 120;
/// Transition standard : ouverture d'un menu, apparition d'un indicateur.
pub const DUREE_STANDARD_MS: u32 = 200;
/// Entree d'un element de grande taille : fenetre, panneau.
pub const DUREE_ENTREE_MS: u32 = 240;
/// Au-dela, une animation fait attendre : aucune n'a le droit de durer plus.
pub const DUREE_MAX_MS: u32 = 320;
