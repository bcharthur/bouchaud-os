/*
 * Bouchaud OS — M11 : le chrome du navigateur natif Ladybird.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 *
 * ## Pourquoi ce fichier existe
 *
 * M9 a prouve la chaine « LibWeb -> RequestServer -> HTTP -> pixels dans une
 * fenetre Bouchaud ». Il l'a prouvee avec une URL passee en variable
 * d'environnement, une seule capture, et aucune entree : c'est une preuve, pas
 * un navigateur. M11 ajoute exactement ce qui manque pour s'en servir — une
 * barre d'adresse, un historique, des liens cliquables, du defilement — sans
 * introduire le processus Browser d'upstream, qui reste un jalon ulterieur.
 *
 * ## Ou il s'insere
 *
 *     Bouchaud WM  --- Key/Pointer/Wheel (protocole GUI v1) --->  ce fichier
 *          ^                                                          |
 *          |                                          Web::MouseEvent / KeyEvent
 *          |                                                          v
 *          +---- FrameReady + surface partagee <---- composition <-- WebContent
 *
 * Il est **entierement en-tete** et volontairement sans dependance nouvelle :
 * l'ajouter ne demande aucune modification de `Services/WebContent/CMakeLists.txt`,
 * donc aucune divergence de plus avec l'arbre upstream epingle.
 *
 * ## Ce qu'il ne fait pas
 *
 * Pas d'onglets (M13), pas de bac a sable (M14).
 *
 * Le texte, lui, n'est plus en police bitmap : `modernise-v15.py` remplace le
 * corps de `draw_ui_text` par le rendu Skia du meme moteur que la page, et
 * l'atlas 8x8 reste le secours et la MESURE. C'est pour cela que tout le texte
 * d'interface passe par cette seule fonction : une seconde voie de dessin
 * echapperait a la modernisation sans que rien ne le signale.
 */

#pragma once

// Atlas de glyphes DejaVu, genere par tools/ladybird/chrome/fabrique-atlas.py.
// C'est de la donnee : ce fichier ne gagne aucune dependance de dessin.
#include "BouchaudAtlas.h"

// BOUCHAUD_CHROME_V18_DEGAT_PARTIEL
//
// Quels pixels une nouvelle capture doit reecrire. Arithmetique entiere pure,
// sans dependance : `tools/ladybird/chrome/test_degat.cpp` l'execute sur l'hote
// a chaque CI, alors que le reste de ce fichier ne tourne que dans QEMU.
#include "BouchaudDegat.h"

// BOUCHAUD_CHROME_V18_ZOOM
//
// Les crans de zoom. Meme raison d'etre que BouchaudDegat.h : de l'arithmetique
// entiere sans dependance, donc verifiable sur l'hote par `test_zoom.cpp`.
#include "BouchaudZoom.h"

// BOUCHAUD_CHROME_V19_CALQUES
//
// Le degat des surfaces flottantes -- bulle de survol, barre de recherche,
// menu contextuel -- que le moteur ne connait pas et ne signalera donc jamais.
// Meme raison d'etre que les deux precedents : `test_calques.cpp` l'execute sur
// l'hote.
#include "BouchaudCalques.h"

#if defined(BOUCHAUD_PORT)

#    include <AK/ByteString.h>
#    include <AK/Format.h>
#    include <AK/Function.h>
#    include <AK/Optional.h>
#    include <AK/StringBuilder.h>
#    include <AK/StringView.h>
#    include <AK/Types.h>
#    include <AK/Vector.h>
#    include <LibGfx/Bitmap.h>
#    include <LibGfx/Point.h>
#    include <LibGfx/ShareableBitmap.h>
#    include <LibWeb/Page/InputEvent.h>
#    include <LibWeb/UIEvents/KeyCode.h>
#    include <LibWeb/UIEvents/MouseButton.h>

#    include <cerrno>
#    include <cstdint>
#    include <cstdlib>
#    include <cstring>
#    include <fcntl.h>
#    include <sys/mman.h>
#    include <unistd.h>

namespace WebContent::BouchaudChrome {

// ----------------------------------------------------------------------------
// Geometrie
// ----------------------------------------------------------------------------

/// Hauteur de la barre d'outils, en pixels de la surface.
///
/// Le viewport de la page vaut donc `hauteur_surface - toolbar_height`. Les deux
/// valeurs se lisent au meme endroit pour qu'un clic tombe la ou la page a ete
/// peinte : `page_origin_y()` est la seule conversion, et elle est utilisee
/// aussi bien par la composition que par le routage des entrees.
inline constexpr int toolbar_height = 36;

inline constexpr int button_width = 30;
inline constexpr int button_height = 24;
inline constexpr int button_top = 6;
inline constexpr int button_gap = 4;
inline constexpr int margin = 6;

inline constexpr int page_origin_y() { return toolbar_height; }

// Couleurs XRGB8888 (l'octet de poids fort est ignore par le compositeur).
inline constexpr u32 color_toolbar = 0x00'23'27'2b;
inline constexpr u32 color_toolbar_edge = 0x00'11'13'15;
inline constexpr u32 color_button = 0x00'3a'40'46;
inline constexpr u32 color_button_off = 0x00'2b'2f'34;
inline constexpr u32 color_field = 0x00'ff'ff'ff;
inline constexpr u32 color_field_idle = 0x00'e3'e6'ea;
inline constexpr u32 color_field_text = 0x00'16'1a'1e;
// Selection du champ d'adresse : bleu pale, texte inchange. C'est la convention
// des champs clairs, et cela evite de dependre de la couleur du texte -- que
// `tools/ladybird/chrome/modernise-v15.py` reecrit pour passer au rendu Skia.
inline constexpr u32 color_field_selection = 0x00'ac'ce'f7;
inline constexpr u32 color_glyph = 0x00'e8'ea'ed;
inline constexpr u32 color_glyph_off = 0x00'6b'71'78;
inline constexpr u32 color_secure = 0x00'1e'8e'3e;
inline constexpr u32 color_insecure = 0x00'c5'39'29;
inline constexpr u32 color_page_backdrop = 0x00'ff'ff'ff;

// BOUCHAUD_CHROME_V19_CALQUES : les surfaces flottantes.
inline constexpr u32 color_calque_fond = 0x00'23'27'2b;
inline constexpr u32 color_calque_bord = 0x00'11'13'15;
inline constexpr u32 color_calque_texte = 0x00'e8'ea'ed;

/// Hauteur d'une ligne de texte d'interface, celle que Skia recoit en V15.
inline constexpr int ui_text_height = 22;
/// Marge horizontale a l'interieur d'un calque.
inline constexpr int calque_marge = 8;
/// Hauteur de la bulle de survol : une ligne de texte et deux pixels d'air.
inline constexpr int survol_hauteur = ui_text_height + 2;
/// Barre de recherche : assez large pour une requete et son compteur.
inline constexpr int recherche_largeur = 320;
inline constexpr int recherche_hauteur = ui_text_height + 10;
/// Menu contextuel.
inline constexpr int menu_largeur = 232;
inline constexpr int menu_hauteur_entree = ui_text_height + 6;
inline constexpr int menu_marge_verticale = 4;

// ----------------------------------------------------------------------------
// Protocole GUI
//
// Troisieme implementation du meme format de fil, apres `src/gui/protocole.rs`
// (noyau) et `tools/userland/navigateur/hote.cpp` (client Qt). Rien dans la
// chaine de construction ne relie les trois : c'est
// `tools/verifie-protocole-gui.py` qui les compare, et il lit les constantes
// ci-dessous par leur nom. Une valeur ecrite en clair dans le code lui
// echapperait, et le desaccord ne se verrait qu'a l'execution, sous la forme
// d'une fenetre qui ne s'ouvre pas.
// ----------------------------------------------------------------------------

constexpr u32 MAGIC = 0x55474f42; // "BOGU"
constexpr u16 VERSION = 1;
constexpr u32 TAILLE_ENTETE = 16;
constexpr u32 CHARGE_MAX = 4096;
constexpr u32 FENETRE = 1;

enum Genre : u16 {
    Hello = 1,
    CreateWindow = 2,
    SetTitle = 3,
    Damage = 4,
    Close = 5,
    FrameReady = 6,
    /// Le chrome remplace le presse-papiers du bureau. Charge : les octets.
    PressePapiersEcrit = 7,
    Surface = 0x100,
    Configure = 0x101,
    Focus = 0x102,
    Key = 0x103,
    Pointer = 0x104,
    Wheel = 0x105,
    CloseRequest = 0x106,
    /// Le contenu courant du presse-papiers du bureau. Charge : les octets.
    ///
    /// Pousse par le gestionnaire de fenetres au seul client qui a le foyer, et
    /// seulement quand il a change. Il n'existe pas de message de LECTURE :
    /// voir `src/gui/presse_papiers.rs` pour ce que cette absence ferme.
    PressePapiers = 0x107,
};

// Bits du champ `modificateurs` d'un message `Key`. Definis cote noyau dans
// `window_manager::modificateur` ; `tools/verifie-protocole-gui.py` refuse un
// desaccord.
enum Modificateur : u32 {
    Shift = 1,
    Ctrl = 2,
    Alt = 4,
    AltGr = 8,
};

// Codes de touche du protocole (`docs/GUI_USERLAND_PROTOCOL.md` §4). Ce ne sont
// pas des codes evdev : le bureau produit une touche deja interpretee.
enum CodeTouche : u32 {
    ToucheCaractere = 0,
    ToucheEntree = 1,
    ToucheRetour = 2,
    ToucheTabulation = 3,
    ToucheHaut = 4,
    ToucheBas = 5,
    ToucheGauche = 6,
    ToucheDroite = 7,
    ToucheEchap = 8,
    // Le pave de navigation, arrive avec le navigateur.
    //
    // Ces touches etaient PERDUES entre le controleur et le client : le
    // decodeur clavier ne reconnaissait que les quatre fleches parmi les
    // sequences etendues. Sur le bureau, aucune consequence visible ; ici,
    // l'impossibilite de faire defiler une page sans molette. Suppr etait pire
    // que perdue -- elle arrivait comme Retour arriere, et effacait donc le
    // caractere de gauche.
    ToucheDebut = 9,
    ToucheFin = 10,
    TouchePageHaut = 11,
    TouchePageBas = 12,
    ToucheSupprimer = 13,
    ToucheInserer = 14,
    /// F1 a F12 : le NUMERO arrive dans le champ `unicode`, pas dans le code.
    ToucheFonction = 15,
};

// ----------------------------------------------------------------------------
// Police bitmap 8x8
//
// Copie de `src/drivers/gfx/font.rs` (« font8x8 basic », domaine public), ASCII
// 0x20..0x7E. Un octet par ligne, bit de poids faible = pixel de gauche. Le
// chrome partage ainsi le dessin des lettres avec le reste du systeme, et ne
// depend d'aucune police a charger ni d'aucune API de rendu d'upstream.
// ----------------------------------------------------------------------------

inline constexpr u8 font8x8[95][8] = {
    { 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00 }, // 0x20 ' '
    { 0x18, 0x3C, 0x3C, 0x18, 0x18, 0x00, 0x18, 0x00 }, // 0x21 !
    { 0x36, 0x36, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00 }, // 0x22 "
    { 0x36, 0x36, 0x7F, 0x36, 0x7F, 0x36, 0x36, 0x00 }, // 0x23 #
    { 0x0C, 0x3E, 0x03, 0x1E, 0x30, 0x1F, 0x0C, 0x00 }, // 0x24 $
    { 0x00, 0x63, 0x33, 0x18, 0x0C, 0x66, 0x63, 0x00 }, // 0x25 %
    { 0x1C, 0x36, 0x1C, 0x6E, 0x3B, 0x33, 0x6E, 0x00 }, // 0x26 &
    { 0x06, 0x06, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00 }, // 0x27 '
    { 0x18, 0x0C, 0x06, 0x06, 0x06, 0x0C, 0x18, 0x00 }, // 0x28 (
    { 0x06, 0x0C, 0x18, 0x18, 0x18, 0x0C, 0x06, 0x00 }, // 0x29 )
    { 0x00, 0x66, 0x3C, 0xFF, 0x3C, 0x66, 0x00, 0x00 }, // 0x2A *
    { 0x00, 0x0C, 0x0C, 0x3F, 0x0C, 0x0C, 0x00, 0x00 }, // 0x2B +
    { 0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C, 0x06 }, // 0x2C ,
    { 0x00, 0x00, 0x00, 0x3F, 0x00, 0x00, 0x00, 0x00 }, // 0x2D -
    { 0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C, 0x00 }, // 0x2E .
    { 0x60, 0x30, 0x18, 0x0C, 0x06, 0x03, 0x01, 0x00 }, // 0x2F /
    { 0x3E, 0x63, 0x73, 0x7B, 0x6F, 0x67, 0x3E, 0x00 }, // 0x30 0
    { 0x0C, 0x0E, 0x0C, 0x0C, 0x0C, 0x0C, 0x3F, 0x00 }, // 0x31 1
    { 0x1E, 0x33, 0x30, 0x1C, 0x06, 0x33, 0x3F, 0x00 }, // 0x32 2
    { 0x1E, 0x33, 0x30, 0x1C, 0x30, 0x33, 0x1E, 0x00 }, // 0x33 3
    { 0x38, 0x3C, 0x36, 0x33, 0x7F, 0x30, 0x78, 0x00 }, // 0x34 4
    { 0x3F, 0x03, 0x1F, 0x30, 0x30, 0x33, 0x1E, 0x00 }, // 0x35 5
    { 0x1C, 0x06, 0x03, 0x1F, 0x33, 0x33, 0x1E, 0x00 }, // 0x36 6
    { 0x3F, 0x33, 0x30, 0x18, 0x0C, 0x0C, 0x0C, 0x00 }, // 0x37 7
    { 0x1E, 0x33, 0x33, 0x1E, 0x33, 0x33, 0x1E, 0x00 }, // 0x38 8
    { 0x1E, 0x33, 0x33, 0x3E, 0x30, 0x18, 0x0E, 0x00 }, // 0x39 9
    { 0x00, 0x0C, 0x0C, 0x00, 0x00, 0x0C, 0x0C, 0x00 }, // 0x3A :
    { 0x00, 0x0C, 0x0C, 0x00, 0x00, 0x0C, 0x0C, 0x06 }, // 0x3B ;
    { 0x18, 0x0C, 0x06, 0x03, 0x06, 0x0C, 0x18, 0x00 }, // 0x3C <
    { 0x00, 0x00, 0x3F, 0x00, 0x00, 0x3F, 0x00, 0x00 }, // 0x3D =
    { 0x06, 0x0C, 0x18, 0x30, 0x18, 0x0C, 0x06, 0x00 }, // 0x3E >
    { 0x1E, 0x33, 0x30, 0x18, 0x0C, 0x00, 0x0C, 0x00 }, // 0x3F ?
    { 0x3E, 0x63, 0x7B, 0x7B, 0x7B, 0x03, 0x1E, 0x00 }, // 0x40 @
    { 0x0C, 0x1E, 0x33, 0x33, 0x3F, 0x33, 0x33, 0x00 }, // 0x41 A
    { 0x3F, 0x66, 0x66, 0x3E, 0x66, 0x66, 0x3F, 0x00 }, // 0x42 B
    { 0x3C, 0x66, 0x03, 0x03, 0x03, 0x66, 0x3C, 0x00 }, // 0x43 C
    { 0x1F, 0x36, 0x66, 0x66, 0x66, 0x36, 0x1F, 0x00 }, // 0x44 D
    { 0x7F, 0x46, 0x16, 0x1E, 0x16, 0x46, 0x7F, 0x00 }, // 0x45 E
    { 0x7F, 0x46, 0x16, 0x1E, 0x16, 0x06, 0x0F, 0x00 }, // 0x46 F
    { 0x3C, 0x66, 0x03, 0x03, 0x73, 0x66, 0x7C, 0x00 }, // 0x47 G
    { 0x33, 0x33, 0x33, 0x3F, 0x33, 0x33, 0x33, 0x00 }, // 0x48 H
    { 0x1E, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x1E, 0x00 }, // 0x49 I
    { 0x78, 0x30, 0x30, 0x30, 0x33, 0x33, 0x1E, 0x00 }, // 0x4A J
    { 0x67, 0x66, 0x36, 0x1E, 0x36, 0x66, 0x67, 0x00 }, // 0x4B K
    { 0x0F, 0x06, 0x06, 0x06, 0x46, 0x66, 0x7F, 0x00 }, // 0x4C L
    { 0x63, 0x77, 0x7F, 0x7F, 0x6B, 0x63, 0x63, 0x00 }, // 0x4D M
    { 0x63, 0x67, 0x6F, 0x7B, 0x73, 0x63, 0x63, 0x00 }, // 0x4E N
    { 0x1C, 0x36, 0x63, 0x63, 0x63, 0x36, 0x1C, 0x00 }, // 0x4F O
    { 0x3F, 0x66, 0x66, 0x3E, 0x06, 0x06, 0x0F, 0x00 }, // 0x50 P
    { 0x1E, 0x33, 0x33, 0x33, 0x3B, 0x1E, 0x38, 0x00 }, // 0x51 Q
    { 0x3F, 0x66, 0x66, 0x3E, 0x36, 0x66, 0x67, 0x00 }, // 0x52 R
    { 0x1E, 0x33, 0x07, 0x0E, 0x38, 0x33, 0x1E, 0x00 }, // 0x53 S
    { 0x3F, 0x2D, 0x0C, 0x0C, 0x0C, 0x0C, 0x1E, 0x00 }, // 0x54 T
    { 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x3F, 0x00 }, // 0x55 U
    { 0x33, 0x33, 0x33, 0x33, 0x33, 0x1E, 0x0C, 0x00 }, // 0x56 V
    { 0x63, 0x63, 0x63, 0x6B, 0x7F, 0x77, 0x63, 0x00 }, // 0x57 W
    { 0x63, 0x63, 0x36, 0x1C, 0x1C, 0x36, 0x63, 0x00 }, // 0x58 X
    { 0x33, 0x33, 0x33, 0x1E, 0x0C, 0x0C, 0x1E, 0x00 }, // 0x59 Y
    { 0x7F, 0x63, 0x31, 0x18, 0x4C, 0x66, 0x7F, 0x00 }, // 0x5A Z
    { 0x1E, 0x06, 0x06, 0x06, 0x06, 0x06, 0x1E, 0x00 }, // 0x5B [
    { 0x03, 0x06, 0x0C, 0x18, 0x30, 0x60, 0x40, 0x00 }, // 0x5C (barre oblique inverse)
    { 0x1E, 0x18, 0x18, 0x18, 0x18, 0x18, 0x1E, 0x00 }, // 0x5D ]
    { 0x08, 0x1C, 0x36, 0x63, 0x00, 0x00, 0x00, 0x00 }, // 0x5E ^
    { 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF }, // 0x5F _
    { 0x0C, 0x0C, 0x18, 0x00, 0x00, 0x00, 0x00, 0x00 }, // 0x60 `
    { 0x00, 0x00, 0x1E, 0x30, 0x3E, 0x33, 0x6E, 0x00 }, // 0x61 a
    { 0x07, 0x06, 0x06, 0x3E, 0x66, 0x66, 0x3B, 0x00 }, // 0x62 b
    { 0x00, 0x00, 0x1E, 0x33, 0x03, 0x33, 0x1E, 0x00 }, // 0x63 c
    { 0x38, 0x30, 0x30, 0x3E, 0x33, 0x33, 0x6E, 0x00 }, // 0x64 d
    { 0x00, 0x00, 0x1E, 0x33, 0x3F, 0x03, 0x1E, 0x00 }, // 0x65 e
    { 0x1C, 0x36, 0x06, 0x0F, 0x06, 0x06, 0x0F, 0x00 }, // 0x66 f
    { 0x00, 0x00, 0x6E, 0x33, 0x33, 0x3E, 0x30, 0x1F }, // 0x67 g
    { 0x07, 0x06, 0x36, 0x6E, 0x66, 0x66, 0x67, 0x00 }, // 0x68 h
    { 0x0C, 0x00, 0x0E, 0x0C, 0x0C, 0x0C, 0x1E, 0x00 }, // 0x69 i
    { 0x30, 0x00, 0x30, 0x30, 0x30, 0x33, 0x33, 0x1E }, // 0x6A j
    { 0x07, 0x06, 0x66, 0x36, 0x1E, 0x36, 0x67, 0x00 }, // 0x6B k
    { 0x0E, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x1E, 0x00 }, // 0x6C l
    { 0x00, 0x00, 0x33, 0x7F, 0x7F, 0x6B, 0x63, 0x00 }, // 0x6D m
    { 0x00, 0x00, 0x1F, 0x33, 0x33, 0x33, 0x33, 0x00 }, // 0x6E n
    { 0x00, 0x00, 0x1E, 0x33, 0x33, 0x33, 0x1E, 0x00 }, // 0x6F o
    { 0x00, 0x00, 0x3B, 0x66, 0x66, 0x3E, 0x06, 0x0F }, // 0x70 p
    { 0x00, 0x00, 0x6E, 0x33, 0x33, 0x3E, 0x30, 0x78 }, // 0x71 q
    { 0x00, 0x00, 0x3B, 0x6E, 0x66, 0x06, 0x0F, 0x00 }, // 0x72 r
    { 0x00, 0x00, 0x3E, 0x03, 0x1E, 0x30, 0x1F, 0x00 }, // 0x73 s
    { 0x08, 0x0C, 0x3E, 0x0C, 0x0C, 0x2C, 0x18, 0x00 }, // 0x74 t
    { 0x00, 0x00, 0x33, 0x33, 0x33, 0x33, 0x6E, 0x00 }, // 0x75 u
    { 0x00, 0x00, 0x33, 0x33, 0x33, 0x1E, 0x0C, 0x00 }, // 0x76 v
    { 0x00, 0x00, 0x63, 0x6B, 0x7F, 0x7F, 0x36, 0x00 }, // 0x77 w
    { 0x00, 0x00, 0x63, 0x36, 0x1C, 0x36, 0x63, 0x00 }, // 0x78 x
    { 0x00, 0x00, 0x33, 0x33, 0x33, 0x3E, 0x30, 0x1F }, // 0x79 y
    { 0x00, 0x00, 0x3F, 0x19, 0x0C, 0x26, 0x3F, 0x00 }, // 0x7A z
    { 0x38, 0x0C, 0x0C, 0x07, 0x0C, 0x0C, 0x38, 0x00 }, // 0x7B {
    { 0x18, 0x18, 0x18, 0x00, 0x18, 0x18, 0x18, 0x00 }, // 0x7C |
    { 0x07, 0x0C, 0x0C, 0x38, 0x0C, 0x0C, 0x07, 0x00 }, // 0x7D }
    { 0x6E, 0x3B, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00 }, // 0x7E ~
};

inline constexpr int glyph_width = 8;
inline constexpr int glyph_height = 8;

// ----------------------------------------------------------------------------
// Champ de saisie
// ----------------------------------------------------------------------------

/// Un champ de saisie a une ligne : le texte, le curseur, la selection totale.
///
/// # Pourquoi il existe plutot qu'une seconde copie de la meme table
///
/// Le chrome en a deux -- la barre d'adresse et la barre de recherche -- et il
/// en aura d'autres. Une table de touches de champ de saisie a l'air triviale
/// et ne l'est pas : la selection totale se defait a la premiere touche qui
/// deplace ou modifie, Suppr efface a DROITE quand Retour arriere efface a
/// gauche, et le curseur se borne au texte a chaque operation parce qu'un
/// `pose()` a pu le laisser derriere. Recopiee, elle diverge -- la copie qui
/// sert le plus gagne le collage de Ctrl+V, l'autre pas -- et rien ne le
/// signale avant qu'on essaie.
///
/// Le texte est conserve en octets ASCII : le pilote clavier du bureau n'expose
/// pas encore de disposition non latine, et pretendre le contraire ici ne
/// rendrait pas la saisie plus juste.
struct Champ {
    Vector<u8> texte;
    size_t caret { 0 };
    /// Tout le texte est selectionne : la premiere frappe le remplace.
    bool tout_selectionne { false };

    ByteString vers_chaine() const
    {
        StringBuilder builder;
        for (auto octet : texte)
            builder.append(static_cast<char>(octet));
        return builder.to_byte_string();
    }

    bool est_vide() const { return texte.is_empty(); }

    /// Remplace le contenu et pose le curseur a la fin.
    void pose(StringView valeur)
    {
        texte.clear_with_capacity();
        for (size_t index = 0; index < valeur.length(); ++index)
            texte.append(static_cast<u8>(valeur[index]));
        caret = texte.size();
        tout_selectionne = false;
    }

    void selectionne_tout()
    {
        tout_selectionne = true;
        caret = texte.size();
    }

    void deselectionne() { tout_selectionne = false; }

    /// Pose le curseur a la fin sans rien selectionner : ce que fait un clic.
    void pose_curseur_a_la_fin()
    {
        tout_selectionne = false;
        caret = texte.size();
    }

    /// Insere un texte a la position du curseur, en remplacant la selection.
    ///
    /// Les octets non imprimables sont IGNORES, et c'est la seule chose non
    /// evidente de cette fonction. Le presse-papiers vient du bureau, donc
    /// possiblement d'une autre application, donc d'un contenu que ce
    /// programme ne controle pas. Une chaine qui porte un saut de ligne puis
    /// une seconde adresse, collee dans une barre qui accepterait les deux,
    /// montre une adresse et en visite une autre : c'est un tour connu, et le
    /// filtre est ici plutot que chez l'appelant parce qu'un appelant, on
    /// l'oublie.
    void colle(StringView valeur)
    {
        if (tout_selectionne) {
            texte.clear_with_capacity();
            caret = 0;
            tout_selectionne = false;
        }
        if (caret > texte.size())
            caret = texte.size();
        for (size_t index = 0; index < valeur.length(); ++index) {
            auto const octet = static_cast<u8>(valeur[index]);
            if (octet < 0x20 || octet >= 0x7f)
                continue;
            texte.insert(caret, octet);
            ++caret;
        }
    }

    /// Applique UNE touche de saisie. Rend true si le champ l'a consommee.
    ///
    /// `ToucheEntree` et `ToucheEchap` ne sont volontairement PAS consommees :
    /// elles ne veulent pas dire la meme chose dans une barre d'adresse -- qui
    /// navigue et qui abandonne -- et dans une barre de recherche -- qui passe
    /// a la correspondance suivante et qui ferme. C'est a l'appelant de le
    /// decider, et lui seul le sait.
    bool applique(u32 code, u32 code_point);
};

inline bool Champ::applique(u32 code, u32 code_point)
{
    // La selection totale est resolue UNE fois, ici, avant la table. Chaque
    // branche ci-dessous suppose donc qu'elle n'existe plus -- ce qui evite
    // d'avoir a la reexaminer dans chacune, et d'en oublier une.
    auto const selectionne = tout_selectionne;
    auto const remplace_tout = [this] {
        texte.clear_with_capacity();
        caret = 0;
    };

    switch (code) {
    case ToucheCaractere:
        // Hors de l'ASCII imprimable, la touche n'est pas pour ce champ : la
        // rendre non consommee laisse l'appelant en faire autre chose.
        if (code_point < 0x20 || code_point >= 0x7f)
            return false;
        tout_selectionne = false;
        if (selectionne)
            remplace_tout();
        if (caret > texte.size())
            caret = texte.size();
        texte.insert(caret, static_cast<u8>(code_point));
        ++caret;
        return true;

    case ToucheRetour:
        tout_selectionne = false;
        if (selectionne)
            remplace_tout();
        else if (caret > 0 && !texte.is_empty()) {
            --caret;
            texte.remove(caret);
        }
        return true;

    case ToucheSupprimer:
        // Suppr efface a DROITE. La touche existait deja cote materiel, mais le
        // decodeur la traduisait en Retour arriere : elle effacait donc le
        // caractere de gauche, ce qui est la seule chose qu'elle ne doit pas
        // faire.
        tout_selectionne = false;
        if (selectionne)
            remplace_tout();
        else if (caret < texte.size())
            texte.remove(caret);
        return true;

    case ToucheGauche:
        tout_selectionne = false;
        if (selectionne)
            caret = 0;
        else if (caret > 0)
            --caret;
        return true;

    case ToucheDroite:
        tout_selectionne = false;
        if (selectionne)
            caret = texte.size();
        else if (caret < texte.size())
            ++caret;
        return true;

    case ToucheDebut:
        tout_selectionne = false;
        caret = 0;
        return true;

    case ToucheFin:
        tout_selectionne = false;
        caret = texte.size();
        return true;

    default:
        return false;
    }
}

// ----------------------------------------------------------------------------
// Etat
// ----------------------------------------------------------------------------

struct State {
    // Descripteurs et geometrie fournis par le gestionnaire de fenetres.
    int gui_fd { -1 };
    int surface_fd { -1 };
    // Taille LOGIQUE : la zone utile courante de la fenetre. Elle change quand
    // l'utilisateur maximise, restaure ou ancre la fenetre -- voir le
    // `Configure` de `handle_message`.
    int surface_width { 0 };
    int surface_height { 0 };
    int surface_stride { 0 };
    // Taille ALLOUEE : ce que le compositeur a reserve, une fois pour toutes,
    // pour la plus grande fenetre possible. C'est elle -- et jamais la taille
    // logique -- qui dimensionne le mappage.
    //
    // BOUCHAUD_CHROME_V17_SURFACE_REDIMENSIONNABLE : les deux etaient un seul
    // champ. `mapped_surface` comparait donc la taille du mappage a la taille
    // de la fenetre, et refusait de peindre des que celle-ci changeait
    // (M11_SURFACE_GEOMETRY_CHANGED). Les separer est ce qui rend le
    // redimensionnement possible sans remapper : le mappage ne bouge plus,
    // seule la partie qu'on en peint bouge.
    int surface_alloc_width { 0 };
    int surface_alloc_height { 0 };
    // La surface GUI garde le même fd pendant la vie du client. La mapper une
    // fois évite mmap/munmap et les shootdowns TLB à chaque frame chrome/page.
    u8* surface_mapping { nullptr };
    size_t surface_mapping_bytes { 0 };

    /// Barre d'adresse.
    ///
    /// Sa selection totale -- `address.tout_selectionne` -- est le seul etat de
    /// selection dont depende un raccourci : Ctrl+L veut dire « je vais taper
    /// une autre adresse », et laisser le curseur au bout du texte obligerait a
    /// effacer l'URL caractere par caractere avant de pouvoir s'en servir. La
    /// premiere frappe remplace, comme partout ailleurs ; Echap restaure l'URL
    /// commitee, donc rien n'est perdu.
    Champ address;
    bool address_focused { false };

    // BOUCHAUD_CHROME_V19_RECHERCHE
    //
    // La recherche dans la page. LibWeb la sait faire depuis toujours --
    // `Page::find_in_page()` cherche, surligne et compte -- et rien ne
    // l'appelait : sur un document long, la seule facon de trouver un mot
    // etait de le lire.
    Champ recherche;
    bool recherche_ouverte { false };
    /// La barre est ouverte ET recoit les frappes.
    ///
    /// Les deux etats sont distincts parce qu'ils le sont dans tout
    /// navigateur : un clic dans la page rend le foyer au document sans fermer
    /// la barre, et F3 continue d'y parcourir les correspondances. Les
    /// confondre ferait taper dans la barre de recherche a qui vient de
    /// cliquer dans un champ de la page.
    bool recherche_focus { false };
    /// Le rang de la correspondance courante, tel que le moteur le rend.
    size_t recherche_rang { 0 };
    size_t recherche_total { 0 };
    /// Le moteur ne connait pas toujours le total ; ne pas l'inventer.
    bool recherche_total_connu { false };

    // Ce que la page dit d'elle-meme.
    ByteString committed_url;
    ByteString title;
    ByteString status { "pret" };
    bool loading { false };
    bool secure { false };

    // Suivi du pointeur : le protocole GUI envoie un etat de boutons absolu,
    // pas des fronts. Les fronts se deduisent ici.
    unsigned last_buttons { 0 };
    int last_x { 0 };
    int last_y { -1 };
    bool pointer_in_page { false };

    // Cadrage du flux entrant : un message n'est analyse qu'entier.
    Vector<u8> incoming;

    /// Recompositions du chrome encore dues.
    ///
    /// Le *contenu de la page* n'entre pas dans ce compteur, et c'est tout le
    /// changement : LibWeb sait seul quand son rendu a change, et le dit par
    /// `request_frame()`. Ce compteur ne couvre que ce que le moteur ne peut
    /// pas savoir — une lettre tapee dans la barre d'adresse, un bouton
    /// enfonce, un libelle d'etat. Recomposer pour cela ne coute qu'une copie
    /// de surface : ni mise en page, ni liste d'affichage, ni rasterisation.
    int chrome_frames_pending { 0 };

    /// Derniere capture de page recue.
    ///
    /// La garder est ce qui permet de redessiner la barre d'outils sans
    /// redemander une page au moteur. `ShareableBitmap` porte un tampon
    /// anonyme partage et compte ses references : la retenir ne copie rien.
    Gfx::ShareableBitmap last_page;

    // Compte de series des messages sortants.
    u32 serial { 0 };
    bool handshake_done { false };
    bool frame_seen { false };
    bool frame_after_wheel_pending { false };
    bool wheel_input_pending { false };

    /// Ce que la surface partagee porte deja, et donc ce qu'une capture doit
    /// reecrire. Voir BouchaudDegat.h.
    BouchaudDegat::Suivi suivi_page;

    /// Le cran de zoom courant. Voir BouchaudZoom.h.
    int zoom_cran { BouchaudZoom::cran_neutre };

    /// Ce que la surface porte des calques. Voir BouchaudCalques.h.
    BouchaudCalques::Suivi calques;

    // BOUCHAUD_CHROME_V19_PRESSE_PAPIERS
    //
    // La copie locale du presse-papiers du bureau.
    //
    // Le coller doit etre SYNCHRONE -- Ctrl+V insere un texte a l'instant ou on
    // le tape -- et le contenu vit dans le gestionnaire de fenetres. Demander
    // puis attendre obligerait a suspendre une frappe le temps d'un aller-retour
    // de protocole, ou a coller « plus tard », ce qui n'est pas coller.
    //
    // Le bureau POUSSE donc le contenu a chaque prise de foyer, et cette copie
    // est toujours celle qui etait a jour la derniere fois que cette fenetre a
    // ete au premier plan -- c'est-a-dire la derniere fois ou l'utilisateur a
    // pu copier quoi que ce soit ailleurs.
    ByteString presse_papiers;

    // BOUCHAUD_CHROME_V19_MENU_CONTEXTUEL
    //
    // Le menu contextuel. Il ne s'ouvre pas sur le clic droit : il s'ouvre
    // quand LIBWEB le demande, apres avoir distribue l'evenement `contextmenu`
    // au document. C'est ce detour qui fait qu'une page qui appelle
    // `preventDefault()` -- un editeur de texte, une carte, un terminal web --
    // garde son propre menu, et c'est la seule facon correcte de le brancher.
    bool menu_ouvert { false };
    /// Position d'ouverture, en coordonnees de SURFACE.
    int menu_x { 0 };
    int menu_y { 0 };
    /// L'adresse du lien sous le pointeur au moment du clic, si c'en etait un.
    ByteString menu_lien;
    /// Le RANG de l'entree survolee dans la liste visible, -1 si aucune.
    int menu_survole { -1 };

    /// L'adresse du lien sous le pointeur, vide quand il n'y en a pas.
    ///
    /// C'est la seule chose qu'un navigateur montre avant qu'on clique, et
    /// c'est ce qui permet de voir ou mene un lien dont le texte ment. Sur un
    /// systeme dont le but est de tenir devant un attaquant, la retirer serait
    /// une regression de securite, pas un manque de confort.
    ByteString survol_url;

    // Rendu M11: compteurs cumulatifs, journalises par paquets de 16 trames.
    u64 chrome_full_frames { 0 };
    u64 chrome_partial_frames { 0 };
    u64 chrome_toolbar_frames { 0 };
    u64 page_frames { 0 };
    u64 chrome_pixels_written { 0 };
    u64 published_frames { 0 };
    /// Degats que le moteur a signales sans qu'aucun pixel ne soit a reecrire.
    ///
    /// Le compteur qui dit si le travail a disparu ou s'il a seulement change
    /// de place : une page qui invalide hors fenetre n'a plus a reveiller le
    /// compositeur.
    u64 page_frames_sans_effet { 0 };

    // Rappels vers WebContent. Poses par `ConnectionFromClient::bouchaud_m11_start`.
    Function<void(Web::MouseEvent)> on_mouse_event;
    Function<void(Web::KeyEvent)> on_key_event;
    Function<void(ByteString)> on_navigate;
    Function<void(int)> on_history_delta;
    Function<void()> on_reload;
    Function<void()> on_stop;
    Function<void()> on_repaint;
    /// La fenetre a change de taille : nouveau viewport de PAGE (largeur de la
    /// surface, hauteur sous la barre d'outils).
    Function<void(int, int)> on_resize;
    /// Le zoom a change : nouveau facteur, en POURCENTS.
    Function<void(int)> on_zoom;
    /// Nouvelle requete de recherche. Une chaine vide efface le surlignage.
    Function<void(ByteString)> on_find;
    Function<void()> on_find_next;
    Function<void()> on_find_previous;
    /// Selectionne tout le document.
    Function<void()> on_select_all;
    /// Le texte selectionne dans le document, vide s'il n'y en a pas.
    Function<ByteString()> on_copy;
    /// Le texte selectionne, retire du document au passage.
    Function<ByteString()> on_cut;
    /// Insere un texte a la place de la selection du document.
    Function<void(ByteString)> on_paste;
    Function<void()> on_close;
};

inline State& state()
{
    // Alloue une fois, jamais detruit. Un `static State` local imposerait un
    // destructeur a la sortie du processus (`-Wexit-time-destructors`, erreur
    // chez Ladybird), et le detruire n'aurait de toute facon aucun sens : cet
    // etat porte les rappels que le moteur peut encore appeler pendant
    // l'extinction. La duree de vie voulue est celle du processus, alors on
    // l'ecrit.
    static State* the_state = new State {};
    return *the_state;
}

/// Hauteur utile pour la page, une fois la barre d'outils retiree.
inline int viewport_height()
{
    auto height = state().surface_height - toolbar_height;
    return height > 0 ? height : state().surface_height;
}

/// M11 est actif seulement si le lanceur l'a demande. Sans la variable, le
/// comportement de M9 est conserve octet pour octet.
inline bool enabled()
{
    static bool const value = getenv("BOUCHAUD_M11") != nullptr;
    return value;
}

/// Demande une recomposition du chrome au prochain tic.
///
/// A n'appeler que pour un changement **du chrome**. Une page qui change se
/// signale toute seule : voir tools/ladybird/prepare-repaint.py.
inline void request_chrome_frame()
{
    state().chrome_frames_pending = 1;
}

inline int environment_int(char const* name, int fallback)
{
    auto* raw = getenv(name);
    if (!raw || !*raw)
        return fallback;
    auto parsed = atoi(raw);
    return parsed > 0 ? parsed : fallback;
}

// ----------------------------------------------------------------------------
// Protocole GUI — ecriture
// ----------------------------------------------------------------------------

/// Ecrit un message d'un seul bloc, en-tete et charge ensemble.
///
/// `docs/GUI_USERLAND_PROTOCOL.md` §5 : deux `write()` separes peuvent laisser
/// le compositeur devant un en-tete dont la charge n'arrive jamais. Le
/// descripteur etant passe en non bloquant pour la lecture, `EAGAIN` est
/// possible a l'ecriture ; on reessaie brievement puis on abandonne le message
/// plutot que de bloquer la boucle d'evenements du moteur.
inline bool send_message(u16 kind, void const* payload, u32 payload_size)
{
    auto& s = state();
    if (s.gui_fd < 0)
        return false;

    constexpr size_t header_size = TAILLE_ENTETE;
    constexpr u32 maximum_payload_size = CHARGE_MAX;
    if (payload_size > maximum_payload_size || (payload_size > 0 && payload == nullptr)) {
        errno = payload_size > maximum_payload_size ? EMSGSIZE : EINVAL;
        return false;
    }

    u8 message[header_size + maximum_payload_size] {};
    auto put16 = [](u8* out, u16 value) {
        out[0] = static_cast<u8>(value);
        out[1] = static_cast<u8>(value >> 8);
    };
    auto put32 = [](u8* out, u32 value) {
        out[0] = static_cast<u8>(value);
        out[1] = static_cast<u8>(value >> 8);
        out[2] = static_cast<u8>(value >> 16);
        out[3] = static_cast<u8>(value >> 24);
    };

    put32(message + 0, MAGIC);
    put16(message + 4, VERSION);
    put16(message + 6, kind);
    put32(message + 8, payload_size);
    put32(message + 12, ++s.serial);
    if (payload_size > 0)
        memcpy(message + header_size, payload, payload_size);

    auto message_size = header_size + static_cast<size_t>(payload_size);
    for (int attempt = 0; attempt < 64; ++attempt) {
        auto written = write(s.gui_fd, message, message_size);
        if (written < 0 && errno == EINTR)
            continue;
        if (written < 0 && errno == EAGAIN) {
            usleep(1000);
            continue;
        }
        if (written < 0)
            return false;
        if (static_cast<size_t>(written) != message_size) {
            errno = EIO;
            return false;
        }
        return true;
    }
    errno = EAGAIN;
    return false;
}

inline void send_handshake()
{
    auto& s = state();
    if (s.handshake_done)
        return;

    u8 hello[8] {};
    auto put32 = [](u8* out, u32 value) {
        out[0] = static_cast<u8>(value);
        out[1] = static_cast<u8>(value >> 8);
        out[2] = static_cast<u8>(value >> 16);
        out[3] = static_cast<u8>(value >> 24);
    };
    put32(hello + 0, 1);
    put32(hello + 4, static_cast<u32>(getpid()));
    if (!send_message(Genre::Hello, hello, sizeof(hello))) {
        warnln("[ladybird-bouchaud] M11_GUI_HELLO_FAILED errno={}", errno);
        return;
    }

    // Le nom du moteur qu'on execute, pas un nom maison : c'est ce que le
    // gestionnaire de fenetres affichera dans la barre de titre et dans la
    // barre des taches, a cote de l'icone qui porte la meme coccinelle.
    static constexpr char default_title[] = "Ladybird";
    send_message(Genre::SetTitle, default_title, sizeof(default_title) - 1);
    s.handshake_done = true;
    outln("[ladybird-bouchaud] M11_GUI_HANDSHAKE_OK");
}

inline void send_title()
{
    auto& s = state();
    if (!s.handshake_done)
        return;

    // « Titre de la page — Ladybird », la convention de tous les navigateurs :
    // ce que l'utilisateur cherche d'abord dans une barre des taches, c'est la
    // page ; le nom du navigateur sert a distinguer deux fenetres de deux
    // programmes. L'ordre inverse mettrait le meme mot au debut de chaque
    // fenetre, ce qui les rendrait indistinguables une fois tronquees.
    static constexpr StringView suffixe = " - Ladybird"sv;
    static constexpr size_t longueur_max = 96;

    // On tronque la **page**, jamais le suffixe : couper apres coup aurait
    // mange le nom du navigateur precisement sur les titres longs, c'est-a-dire
    // dans le seul cas ou la distinction sert a quelque chose.
    auto tete = s.title.is_empty() ? s.committed_url : s.title;
    if (tete.is_empty()) {
        auto seul = ByteString { "Ladybird" };
        send_message(Genre::SetTitle, seul.characters(), static_cast<u32>(seul.length()));
        return;
    }

    auto place = longueur_max - suffixe.length();
    if (tete.length() > place)
        tete = tete.substring(0, place);

    StringBuilder builder;
    builder.append(tete);
    builder.append(suffixe);

    auto text = builder.to_byte_string();
    send_message(Genre::SetTitle, text.characters(), static_cast<u32>(text.length()));
}

struct DamageRect {
    int x { 0 };
    int y { 0 };
    int width { 0 };
    int height { 0 };
};

inline void log_render_stats_if_due()
{
    auto& s = state();
    if (s.published_frames == 0 || (s.published_frames % 16) != 0)
        return;
    outln("[ladybird-bouchaud] M11_RENDER_STATS full={} partiel={} toolbar={} page={} "
          "sans_effet={} pixels={}",
        s.chrome_full_frames, s.chrome_partial_frames, s.chrome_toolbar_frames,
        s.page_frames, s.page_frames_sans_effet, s.chrome_pixels_written);
}

inline void send_frame_ready(DamageRect damage)
{
    auto& s = state();
    // Ne jamais publier un rectangle hors surface, meme si une future taille
    // de toolbar devient dynamique.
    damage.x = clamp(damage.x, 0, s.surface_width);
    damage.y = clamp(damage.y, 0, s.surface_height);
    damage.width = clamp(damage.width, 0, s.surface_width - damage.x);
    damage.height = clamp(damage.height, 0, s.surface_height - damage.y);
    if (damage.width == 0 || damage.height == 0)
        return;

    u8 frame[24] {};
    auto put32 = [](u8* out, u32 value) {
        out[0] = static_cast<u8>(value);
        out[1] = static_cast<u8>(value >> 8);
        out[2] = static_cast<u8>(value >> 16);
        out[3] = static_cast<u8>(value >> 24);
    };
    put32(frame + 0, FENETRE);
    put32(frame + 4, 0); // tampon
    put32(frame + 8, static_cast<u32>(damage.x));
    put32(frame + 12, static_cast<u32>(damage.y));
    put32(frame + 16, static_cast<u32>(damage.width));
    put32(frame + 20, static_cast<u32>(damage.height));
    if (!send_message(Genre::FrameReady, frame, sizeof(frame)))
        warnln("[ladybird-bouchaud] M11_FRAME_READY_FAILED errno={}", errno);
    ++s.published_frames;
    log_render_stats_if_due();
    // BOUCHAUD_CHROME_V18_DEGAT_PARTIEL
    //
    // Le temoin cherchait une trame couvrant toute la surface, ce qui etait la
    // seule sorte de trame qui existait. Une molette qui ne deplace que le bas
    // d'un cadre defilant produit maintenant un rectangle partiel, et le
    // temoin ne se serait plus jamais arme. Ce qu'il veut dire est « une trame
    // a suivi la molette » : c'est la trame qui compte, pas sa taille.
    if (s.frame_after_wheel_pending && damage.y + damage.height > page_origin_y()) {
        s.frame_after_wheel_pending = false;
        outln("[ladybird-bouchaud] M11_FRAME_AFTER_SCROLL");
    }
}

// ----------------------------------------------------------------------------
// Dessin
// ----------------------------------------------------------------------------

struct Canvas {
    u8* base { nullptr };
    int width { 0 };
    int height { 0 };
    int stride { 0 };

    u32* row(int y) const { return reinterpret_cast<u32*>(base + static_cast<size_t>(y) * stride); }
};

inline Optional<Canvas> mapped_surface()
{
    auto& s = state();
    if (s.surface_fd < 0 || s.gui_fd < 0 || s.surface_width <= 0 || s.surface_height <= 0)
        return {};
    auto stride_bytes = static_cast<size_t>(s.surface_stride);
    // L'ALLOCATION, pas la fenetre : une fenetre plus petite ne retrecit pas la
    // memoire partagee, et une fenetre agrandie ne doit pas declencher un
    // remappage que le compositeur n'a pas demande.
    auto height_rows = static_cast<size_t>(s.surface_alloc_height);
    if (height_rows != 0 && stride_bytes > SIZE_MAX / height_rows)
        return {};
    auto bytes = stride_bytes * height_rows;
    if (bytes == 0)
        return {};
    if (!s.surface_mapping) {
        auto* mapped = mmap(nullptr, bytes, PROT_READ | PROT_WRITE, MAP_SHARED, s.surface_fd, 0);
        if (mapped == MAP_FAILED) {
            warnln("[ladybird-bouchaud] M11_SURFACE_MMAP_FAILED errno={} bytes={}", errno, bytes);
            return {};
        }
        s.surface_mapping = static_cast<u8*>(mapped);
        s.surface_mapping_bytes = bytes;
        outln("[ladybird-bouchaud] M11_SURFACE_MAPPED bytes={}", bytes);
    }
    if (s.surface_mapping_bytes != bytes) {
        warnln("[ladybird-bouchaud] M11_SURFACE_GEOMETRY_CHANGED old={} new={}", s.surface_mapping_bytes, bytes);
        return {};
    }
    return Canvas { s.surface_mapping, s.surface_width, s.surface_height, s.surface_stride };
}

inline void fill_rect(Canvas const& canvas, int x, int y, int w, int h, u32 color)
{
    auto x0 = max(0, x);
    auto y0 = max(0, y);
    auto x1 = min(canvas.width, x + w);
    auto y1 = min(canvas.height, y + h);
    for (int row_index = y0; row_index < y1; ++row_index) {
        auto* pixels = canvas.row(row_index);
        for (int column = x0; column < x1; ++column)
            pixels[column] = color;
    }
}

// BOUCHAUD_CHROME_ATLAS_V1
//
// # Ce que le texte du chrome etait
//
// La bitmap `font8x8` ci-dessus : un bit par pixel, agrandie par un facteur
// entier. D'ou l'escalier a chaque diagonale sur les captures d'ecran, dans
// une fenetre dont tout le reste est antialiase -- et strictement aucune
// lettre accentuee, puisqu'elle s'arrete a `0x7e`.
//
// # Ce qu'il est
//
// Un atlas de DejaVu Sans, rasterise a la CONSTRUCTION par
// `tools/ladybird/chrome/fabrique-atlas.py` et embarque en octets de
// couverture. Le chrome n'a plus qu'a les melanger.
//
// La contrainte qui avait fait choisir la bitmap tient toujours : ce fichier
// ne depend d'aucune API de dessin, et reste donc utilisable quand la page a
// plante. Un atlas est de la donnee, pas une dependance.
//
// `font8x8` reste le repli exact pour ce que l'atlas ne couvre pas : mieux
// vaut un caractere en escalier qu'un caractere absent.

/// Melange un pixel de couverture. Les couleurs sont en 0x00RRGGBB.
inline void blend_pixel(Canvas const& canvas, int x, int y, u32 color, unsigned int alpha)
{
    if (alpha == 0 || x < 0 || y < 0 || x >= canvas.width || y >= canvas.height)
        return;
    auto* pixels = canvas.row(y);
    if (alpha >= 255) {
        pixels[x] = color;
        return;
    }
    auto destination = pixels[x];
    auto inverse = 255u - alpha;
    auto canal = [&](unsigned int decalage) {
        auto source = (color >> decalage) & 0xffu;
        auto fond = (destination >> decalage) & 0xffu;
        return ((source * alpha + fond * inverse) / 255u) << decalage;
    };
    pixels[x] = canal(16) | canal(8) | canal(0);
}

/// Le glyphe de l'atlas pour ce point de code, ou `nullptr`.
inline BouchaudAtlas::Glyphe const* atlas_glyphe(unsigned int point_de_code)
{
    for (int index = 0; index < BouchaudAtlas::nombre; ++index) {
        if (BouchaudAtlas::glyphes[index].point_de_code == point_de_code)
            return &BouchaudAtlas::glyphes[index];
    }
    return nullptr;
}

/// Decode un point de code UTF-8 a partir de `index`, qu'il avance.
///
/// Le champ d'adresse retient des octets ; une adresse ou un titre accentues y
/// arrivent donc en UTF-8. Les lire octet par octet afficherait deux
/// caracteres la ou il y en a un.
inline unsigned int decode_utf8(StringView text, size_t& index)
{
    auto premier = static_cast<unsigned char>(text[index]);
    auto reste = text.length() - index;
    auto continuation = [&](size_t rang) {
        return static_cast<unsigned char>(text[index + rang]) & 0x3fu;
    };
    if (premier < 0x80) {
        index += 1;
        return premier;
    }
    if ((premier & 0xe0) == 0xc0 && reste >= 2) {
        auto valeur = ((premier & 0x1fu) << 6) | continuation(1);
        index += 2;
        return valeur;
    }
    if ((premier & 0xf0) == 0xe0 && reste >= 3) {
        auto valeur = ((premier & 0x0fu) << 12) | (continuation(1) << 6) | continuation(2);
        index += 3;
        return valeur;
    }
    if ((premier & 0xf8) == 0xf0 && reste >= 4) {
        auto valeur = ((premier & 0x07u) << 18) | (continuation(1) << 12)
            | (continuation(2) << 6) | continuation(3);
        index += 4;
        return valeur;
    }
    index += 1;
    return premier;
}

/// L'avance d'un point de code, en pixels, a cette echelle.
inline int glyph_advance(unsigned int point_de_code, int scale)
{
    if (auto const* glyphe = atlas_glyphe(point_de_code)) {
        auto avance = scale >= 2 ? glyphe->avance * (scale / 2) : (glyphe->avance + 1) / 2;
        return avance > 0 ? avance : 1;
    }
    return glyph_width * scale;
}

inline void draw_glyph_point(Canvas const& canvas, int x, int y, unsigned int point_de_code,
    u32 color, int scale)
{
    auto const* glyphe = atlas_glyphe(point_de_code);
    if (glyphe == nullptr) {
        // Repli exact d'avant : la bitmap, agrandie.
        auto code = point_de_code;
        if (code < 0x20 || code > 0x7e)
            code = '?';
        auto const* bitmap = font8x8[code - 0x20];
        for (int row_index = 0; row_index < glyph_height; ++row_index) {
            auto bits = bitmap[row_index];
            for (int column = 0; column < glyph_width; ++column) {
                if ((bits & (1u << column)) == 0)
                    continue;
                fill_rect(canvas, x + column * scale, y + row_index * scale, scale, scale, color);
            }
        }
        return;
    }

    auto const* couverture = &BouchaudAtlas::couverture[glyphe->decalage];
    if (scale >= 2) {
        auto facteur = scale / 2;
        for (int row_index = 0; row_index < glyphe->hauteur; ++row_index) {
            for (int column = 0; column < glyphe->largeur; ++column) {
                auto alpha = couverture[row_index * glyphe->largeur + column];
                if (alpha == 0)
                    continue;
                for (int dy = 0; dy < facteur; ++dy) {
                    for (int dx = 0; dx < facteur; ++dx) {
                        blend_pixel(canvas,
                            x + (glyphe->gauche + column) * facteur + dx,
                            y + (glyphe->haut + row_index) * facteur + dy,
                            color, alpha);
                    }
                }
            }
        }
        return;
    }

    // Echelle 1 : moyenne de blocs 2x2. L'atlas est rasterise a quinze
    // pixels ; le reduire ici evite d'en embarquer un second.
    for (int row_index = 0; row_index < glyphe->hauteur; row_index += 2) {
        for (int column = 0; column < glyphe->largeur; column += 2) {
            unsigned int somme = 0;
            int compte = 0;
            for (int dy = 0; dy < 2 && row_index + dy < glyphe->hauteur; ++dy) {
                for (int dx = 0; dx < 2 && column + dx < glyphe->largeur; ++dx) {
                    somme += couverture[(row_index + dy) * glyphe->largeur + column + dx];
                    ++compte;
                }
            }
            if (compte == 0 || somme == 0)
                continue;
            blend_pixel(canvas, x + (glyphe->gauche + column) / 2,
                y + (glyphe->haut + row_index) / 2, color, somme / compte);
        }
    }
}

/// Dessine `text` a partir de (x, y), rogne a `max_width` pixels.
/// Rend l'abscisse suivant le dernier glyphe reellement dessine.
inline int draw_text(Canvas const& canvas, int x, int y, StringView text, u32 color, int scale, int max_width)
{
    auto cursor = x;
    size_t index = 0;
    while (index < text.length()) {
        auto point_de_code = decode_utf8(text, index);
        auto advance = glyph_advance(point_de_code, scale);
        if (cursor + advance > x + max_width)
            break;
        draw_glyph_point(canvas, cursor, y, point_de_code, color, scale);
        cursor += advance;
    }
    return cursor;
}

/// Largeur REELLE d'une chaine. Elle se comptait en caracteres, ce qui n'a de
/// sens que pour une police a chasse fixe : le curseur du champ d'adresse et
/// la troncature du titre auraient tous deux glisse.
inline int text_width(StringView text, int scale)
{
    auto total = 0;
    size_t index = 0;
    while (index < text.length())
        total += glyph_advance(decode_utf8(text, index), scale);
    return total;
}

struct Button {
    int x { 0 };
    int width { button_width };
    char const* glyph { "" };
};

inline Button back_button() { return { margin, button_width, "<" }; }
inline Button forward_button() { return { margin + button_width + button_gap, button_width, ">" }; }
inline Button reload_button() { return { margin + 2 * (button_width + button_gap), button_width, "@" }; }

inline int address_field_x() { return margin + 3 * (button_width + button_gap); }

inline int address_field_width()
{
    auto& s = state();
    auto width = s.surface_width - address_field_x() - margin;
    return max(0, width);
}

inline bool point_in_button(Button const& button, int x, int y)
{
    return x >= button.x && x < button.x + button.width
        && y >= button_top && y < button_top + button_height;
}

inline bool point_in_address_field(int x, int y)
{
    return x >= address_field_x() && x < address_field_x() + address_field_width()
        && y >= button_top && y < button_top + button_height;
}

/// Le texte d'interface du chrome, en UN SEUL point de passage.
///
/// `y` est le HAUT de la ligne, et sa hauteur est `ui_text_height`.
///
/// Ce point unique existe pour une raison precise :
/// `tools/ladybird/chrome/modernise-v15.py` remplace ce corps par le rendu
/// Skia et garde l'atlas en secours. Tant qu'il n'y avait qu'un seul texte
/// dans le chrome -- l'URL --, la substitution pouvait viser cette ligne-la ;
/// des qu'il y en a plusieurs, viser chacune serait un moyen sur d'en oublier
/// une, et une bulle de survol en police bitmap a cote d'une barre d'adresse
/// en DejaVu se verrait tout de suite.
///
/// La MESURE, elle, reste celle de l'atlas (`text_width(..., 2)`) dans les deux
/// cas : c'est deja ce que V15 fait pour le caret, et deux mesures differentes
/// pour un meme texte donneraient deux boites differentes.
inline void draw_ui_text(Canvas const& canvas, int x, int y, StringView texte, u32 couleur, int largeur_max)
{
    draw_text(canvas, x, y + 1, texte, couleur, 2, largeur_max);
}

inline void draw_toolbar(Canvas const& canvas)
{
    auto& s = state();

    fill_rect(canvas, 0, 0, canvas.width, toolbar_height, color_toolbar);
    fill_rect(canvas, 0, toolbar_height - 1, canvas.width, 1, color_toolbar_edge);

    auto draw_button = [&](Button const& button, bool active) {
        fill_rect(canvas, button.x, button_top, button.width, button_height,
            active ? color_button : color_button_off);
        auto glyph_x = button.x + (button.width - glyph_width * 2) / 2;
        auto glyph_y = button_top + (button_height - glyph_height * 2) / 2;
        draw_glyph(canvas, glyph_x, glyph_y, button.glyph[0],
            active ? color_glyph : color_glyph_off, 2);
    };

    // Les fleches restent dessinees en permanence : WebContent ne publie pas
    // d'etat « peut reculer » sans le processus Browser, et griser un bouton
    // sur une supposition serait mentir a l'utilisateur.
    draw_button(back_button(), true);
    draw_button(forward_button(), true);
    draw_button(reload_button(), true);

    auto field_x = address_field_x();
    auto field_w = address_field_width();
    fill_rect(canvas, field_x, button_top, field_w, button_height,
        s.address_focused ? color_field : color_field_idle);

    // Pastille de securite : verte pour https, rouge sinon. Elle ne prouve rien
    // de plus que le schema — la chaine, elle, est verifiee par RequestServer.
    fill_rect(canvas, field_x + 4, button_top + 6, 4, button_height - 12,
        s.secure ? color_secure : color_insecure);

    auto text_x = field_x + 14;
    auto text_y = button_top + 3;
    auto available = field_w - 20;

    StringBuilder builder;
    for (auto byte : s.address.texte)
        builder.append(static_cast<char>(byte));
    auto address_text = builder.to_byte_string();

    // Defilement de la saisie.
    //
    // La fenetre visible suit le **curseur**, pas la fin du texte. Les deux se
    // confondent tant qu'on tape a la fin, et divergent des qu'on revient en
    // arriere dans une URL longue : ancrer la vue sur la fin dessinerait alors
    // le curseur colle au bord gauche pendant que les caracteres modifies
    // resteraient hors champ. On ne peut pas corriger ce qu'on ne voit pas.
    // BOUCHAUD_CHROME_ATLAS_V1 : la fenetre visible se mesure en PIXELS.
    //
    // Elle se comptait en caracteres -- `available / (glyph_width * 2)` --, ce
    // qui n'a de sens que pour une chasse fixe. Avec une police
    // proportionnelle, « iii » et « WWW » n'occupent pas la meme place, et
    // l'ancre du defilement aurait glisse a chaque frappe.
    //
    // Le nombre de caracteres visibles est donc CHERCHE : on recule depuis la
    // fin tant que le texte tient. C'est au plus une passe sur la chaine, et
    // le champ d'adresse en compte quelques dizaines.
    auto tient = [&](size_t debut) {
        auto morceau = address_text.substring(debut, address_text.length() - debut);
        return text_width(morceau.view(), 2) <= available;
    };
    size_t first = 0;
    if (available > 0 && !tient(0)) {
        first = address_text.length();
        while (first > 0 && tient(first - 1))
            --first;

        if (s.address_focused) {
            auto caret = min(s.address.caret, address_text.length());
            if (caret < first) {
                first = caret;
            } else {
                // Le curseur doit rester visible : on avance l'ancre jusqu'a
                // ce que le texte du curseur tienne.
                while (first < caret) {
                    auto morceau = address_text.substring(first, caret - first);
                    if (text_width(morceau.view(), 2) <= available)
                        break;
                    ++first;
                }
            }
        }
    }

    auto visible = address_text.substring(first, address_text.length() - first);

    // La surbrillance se dessine AVANT le texte, et sur une ligne distincte de
    // celle qui le dessine : `tools/ladybird/chrome/modernise-v15.py` reecrit
    // l'appel a `draw_text` ci-dessous pour passer au rendu Skia, et une
    // surbrillance melee a cette ligne disparaitrait a la modernisation
    // suivante sans que rien ne le signale.
    if (s.address_focused && s.address.tout_selectionne && !visible.is_empty()) {
        auto largeur = min(text_width(visible.view(), 2), available);
        fill_rect(canvas, text_x - 1, button_top + 3, largeur + 2, button_height - 6,
            color_field_selection);
    }

    draw_ui_text(canvas, text_x, text_y, visible.view(), color_field_text, available);

    if (s.address_focused) {
        auto caret = min(s.address.caret, address_text.length());
        auto caret_offset = caret > first ? caret - first : 0;
        auto avant = address_text.substring(first, caret_offset);
        auto caret_x = text_x + text_width(avant.view(), 2);
        fill_rect(canvas, caret_x, button_top + 4, 2, button_height - 8, color_field_text);
    }

    // Etat du chargement, a droite, en petit.
    auto status_text = s.loading ? ByteString { "chargement..." } : s.status;
    auto status_width = text_width(status_text.view(), 1);
    auto status_x = canvas.width - margin - status_width;
    if (status_x > field_x + field_w + 4)
        draw_text(canvas, status_x, toolbar_height - glyph_height - 3, status_text.view(), color_glyph_off, 1, status_width);
}

// ----------------------------------------------------------------------------
// Calques — les surfaces flottantes au-dessus de la page
// ----------------------------------------------------------------------------
//
// BOUCHAUD_CHROME_V19_CALQUES
//
// Un calque est peint PAR-DESSUS les pixels de page, dans la meme surface
// partagee. Ces pixels-la n'appartiennent a personne : le moteur ne les connait
// pas, donc il ne les signalera jamais comme changes, et la surface les porte
// encore quand le calque disparait. BouchaudCalques.h decide ce qu'il faut
// reecrire ; ce qui suit decide ce qu'on y dessine.
//
// La discipline tient en trois appels, et compose_page() est le seul endroit
// qui les enchaine :
//
//     mesure_calques()      ou chaque calque doit etre a cette trame
//     ... plan, effacement, copie de la page ...
//     dessine_calques()     tout calque que le rectangle publie touche
//     acter la trame        la surface porte maintenant ce qui etait voulu

/// Les calques du chrome, nommes.
///
/// L'enumeration vit ICI et non dans BouchaudCalques.h : ce fichier-la ne sait
/// pas ce qu'est une bulle de survol, et le lui apprendre obligerait a modifier
/// son banc d'essai a chaque element d'interface ajoute. Elle grandit avec ce
/// qui est reellement dessine -- `tools/verifie-calques-chrome.py` refuse un
/// calque declare que `mesure_calques()` ne pose pas ou que
/// `dessine_calques()` ne peint pas.
enum Calque : int {
    /// Bulle d'adresse du lien survole, en bas a gauche.
    Survol = 0,
    /// Barre de recherche dans la page, en haut a droite.
    Recherche,
    /// Menu contextuel, la ou on a clique.
    Menu,
    Nombre,
};

// Un calque de plus que le suivi n'en porte serait silencieusement ignore :
// `place()` refuse un indice hors bornes, et rien ne le dirait avant que la
// bulle manquante se voie a l'ecran.
static_assert(Nombre <= BouchaudCalques::maximum,
    "plus de calques que BouchaudCalques::Suivi n'en porte");

/// La boite de la bulle de survol, en coordonnees de SURFACE.
///
/// En bas a gauche, contre le bord, comme dans tous les navigateurs de bureau :
/// c'est le coin ou elle recouvre le moins souvent ce qu'on est en train de
/// lire, et l'endroit ou l'oeil va la chercher.
inline BouchaudDegat::Rect boite_survol()
{
    auto& s = state();
    if (s.survol_url.is_empty() || s.surface_width <= 0 || s.surface_height <= 0)
        return {};

    auto const y = s.surface_height - survol_hauteur;
    // Une fenetre trop courte n'a pas de place sous la barre d'outils. Peindre
    // quand meme ecrirait la bulle DANS la barre, et le suivi de degat de page
    // ne restaurerait jamais ces pixels-la.
    if (y < page_origin_y())
        return {};

    auto largeur = text_width(s.survol_url.view(), 2) + 2 * calque_marge;
    largeur = min(largeur, max(0, s.surface_width * 3 / 4));
    if (largeur <= 0)
        return {};
    return { 0, y, largeur, survol_hauteur };
}

inline void dessine_survol(Canvas const& canvas)
{
    auto& s = state();
    auto const boite = s.calques.boite(Survol);
    if (boite.vide())
        return;

    fill_rect(canvas, boite.x, boite.y, boite.w, boite.h, color_calque_fond);
    // Deux aretes seulement : celles qui touchent la page. Les deux autres
    // touchent le bord de la fenetre, ou un trait ne separerait rien.
    fill_rect(canvas, boite.x, boite.y, boite.w, 1, color_calque_bord);
    fill_rect(canvas, boite.x + boite.w - 1, boite.y, 1, boite.h, color_calque_bord);
    draw_ui_text(canvas, boite.x + calque_marge, boite.y,
        s.survol_url.view(), color_calque_texte, boite.w - 2 * calque_marge);
}

/// Le compteur de correspondances, tel qu'on l'affiche.
///
/// Le moteur ne connait pas toujours le total -- `total_match_count` est un
/// `Optional` -- et l'inventer serait pire que de ne rien dire : un « 3/12 »
/// faux fait chercher neuf correspondances qui n'existent pas.
inline ByteString texte_compteur_recherche()
{
    auto& s = state();
    // Rien tant que le moteur n'a pas repondu : un compteur qui affiche « 0 »
    // avant la premiere reponse se lit comme « aucun resultat », et c'est faux.
    if (s.recherche.est_vide() || !s.recherche_total_connu)
        return {};
    if (s.recherche_total == 0)
        return "aucun";
    return ByteString::formatted("{}/{}", s.recherche_rang, s.recherche_total);
}

/// La boite de la barre de recherche, en coordonnees de SURFACE.
///
/// En haut a droite de la zone de page : le coin oppose a la bulle de survol,
/// pour que les deux ne se recouvrent jamais, et celui ou elle masque le moins
/// de texte sur une page alignee a gauche.
inline BouchaudDegat::Rect boite_recherche()
{
    auto& s = state();
    if (!s.recherche_ouverte || s.surface_width <= 0 || s.surface_height <= 0)
        return {};

    auto const y = page_origin_y() + calque_marge;
    if (y + recherche_hauteur > s.surface_height)
        return {};
    auto const largeur = min(recherche_largeur, s.surface_width - 2 * calque_marge);
    if (largeur <= 0)
        return {};
    return { s.surface_width - calque_marge - largeur, y, largeur, recherche_hauteur };
}

inline void dessine_recherche(Canvas const& canvas)
{
    auto& s = state();
    auto const boite = s.calques.boite(Recherche);
    if (boite.vide())
        return;

    fill_rect(canvas, boite.x, boite.y, boite.w, boite.h, color_calque_fond);
    // Un cadre complet, contrairement a la bulle de survol : ce calque flotte
    // au milieu de la page et se confondrait sans cela avec un bloc sombre du
    // document.
    fill_rect(canvas, boite.x, boite.y, boite.w, 1, color_calque_bord);
    fill_rect(canvas, boite.x, boite.y + boite.h - 1, boite.w, 1, color_calque_bord);
    fill_rect(canvas, boite.x, boite.y, 1, boite.h, color_calque_bord);
    fill_rect(canvas, boite.x + boite.w - 1, boite.y, 1, boite.h, color_calque_bord);

    auto const compteur = texte_compteur_recherche();
    auto const compteur_largeur = compteur.is_empty()
        ? 0
        : text_width(compteur.view(), 2) + calque_marge;

    auto const champ_x = boite.x + calque_marge;
    auto const champ_y = boite.y + (boite.h - ui_text_height) / 2;
    auto const champ_largeur = boite.w - 2 * calque_marge - compteur_largeur;
    if (champ_largeur <= 0)
        return;

    auto const requete = s.recherche.vers_chaine();

    if (s.recherche.tout_selectionne && !requete.is_empty()) {
        auto const largeur = min(text_width(requete.view(), 2), champ_largeur);
        fill_rect(canvas, champ_x - 1, boite.y + 4, largeur + 2, boite.h - 8, color_button);
    }

    // Une requete sans correspondance se dit par la COULEUR, pas seulement par
    // le compteur : c'est le retour le plus rapide, et celui qu'on lit sans
    // deplacer le regard vers le bord du calque.
    auto const introuvable = s.recherche_total_connu && s.recherche_total == 0
        && !requete.is_empty();
    draw_ui_text(canvas, champ_x, champ_y, requete.view(),
        introuvable ? color_insecure : color_calque_texte, champ_largeur);

    // Le curseur ne se dessine que si la barre a le foyer : sinon il clignote
    // dans un champ ou les frappes ne vont pas, ce qui est exactement le
    // contraire de ce qu'un curseur signifie.
    if (s.recherche_focus) {
        auto const caret = min(s.recherche.caret, requete.length());
        auto const avant = requete.substring(0, caret);
        auto const caret_x = champ_x + min(text_width(avant.view(), 2), champ_largeur);
        fill_rect(canvas, caret_x, boite.y + 5, 2, boite.h - 10, color_calque_texte);
    }

    if (!compteur.is_empty()) {
        auto const largeur = text_width(compteur.view(), 2);
        draw_ui_text(canvas, boite.x + boite.w - calque_marge - largeur, champ_y,
            compteur.view(), color_glyph_off, largeur);
    }
}

/// Les entrees du menu contextuel.
///
/// L'ordre de cette enumeration N'EST PAS celui du menu : `entrees_menu()`
/// decide de ce qui est visible et dans quel ordre, parce que cela depend du
/// contexte -- il n'y a pas d'entree « copier l'adresse du lien » quand on n'a
/// pas clique sur un lien. Un menu qui montre une entree inerte apprend a
/// l'utilisateur a s'en mefier.
enum EntreeMenu : int {
    MenuOuvrirLien = 0,
    MenuCopierLien,
    MenuReculer,
    MenuAvancer,
    MenuRecharger,
    MenuCopier,
    MenuColler,
    MenuToutSelectionner,
    MenuRechercher,
    MenuNombre,
};

inline StringView libelle_menu(int entree)
{
    switch (entree) {
    case MenuOuvrirLien:
        return "Ouvrir le lien"sv;
    case MenuCopierLien:
        return "Copier l'adresse du lien"sv;
    case MenuReculer:
        return "Reculer"sv;
    case MenuAvancer:
        return "Avancer"sv;
    case MenuRecharger:
        return "Recharger"sv;
    case MenuCopier:
        return "Copier"sv;
    case MenuColler:
        return "Coller"sv;
    case MenuToutSelectionner:
        return "Tout selectionner"sv;
    case MenuRechercher:
        return "Rechercher dans la page"sv;
    default:
        return ""sv;
    }
}

/// Remplit `sortie` des entrees visibles, dans l'ordre, et rend leur nombre.
inline int entrees_menu(int (&sortie)[MenuNombre])
{
    auto& s = state();
    int nombre = 0;
    auto ajoute = [&](int entree) { sortie[nombre++] = entree; };
    if (!s.menu_lien.is_empty()) {
        ajoute(MenuOuvrirLien);
        ajoute(MenuCopierLien);
    }
    ajoute(MenuReculer);
    ajoute(MenuAvancer);
    ajoute(MenuRecharger);
    ajoute(MenuCopier);
    ajoute(MenuColler);
    ajoute(MenuToutSelectionner);
    ajoute(MenuRechercher);
    return nombre;
}

inline BouchaudDegat::Rect boite_menu()
{
    auto& s = state();
    if (!s.menu_ouvert || s.surface_width <= 0 || s.surface_height <= 0)
        return {};

    int entrees[MenuNombre] {};
    auto const nombre = entrees_menu(entrees);
    if (nombre <= 0)
        return {};

    auto const hauteur = nombre * menu_hauteur_entree + 2 * menu_marge_verticale;
    auto const largeur = min(menu_largeur, s.surface_width);
    if (largeur <= 0 || page_origin_y() + hauteur > s.surface_height)
        return {};

    // Le menu s'ouvre au pointeur, puis RENTRE dans la fenetre. Un menu qui
    // deborde perd ses dernieres entrees, et un clic droit pres du bord bas
    // est exactement le cas ou cela arrive.
    auto const x = max(0, min(s.menu_x, s.surface_width - largeur));
    auto const y = max(page_origin_y(), min(s.menu_y, s.surface_height - hauteur));
    return { x, y, largeur, hauteur };
}

/// Le rang de l'entree que ce point survole, ou -1.
///
/// Elle interroge `boite_menu()` et non la boite deja posee dans le suivi :
/// celle-ci n'est mise a jour qu'a la composition suivante, et un clic arrive
/// entre les deux. DESSINER suit le suivi -- c'est de lui que le degat a ete
/// calcule -- mais VISER suit l'etat.
inline int entree_menu_au_point(int x, int y)
{
    auto const boite = boite_menu();
    if (!BouchaudCalques::contient(boite, x, y))
        return -1;
    auto const dans = y - boite.y - menu_marge_verticale;
    if (dans < 0)
        return -1;
    auto const rang = dans / menu_hauteur_entree;
    int entrees[MenuNombre] {};
    auto const nombre = entrees_menu(entrees);
    return rang < nombre ? rang : -1;
}

inline void dessine_menu(Canvas const& canvas)
{
    auto& s = state();
    auto const boite = s.calques.boite(Menu);
    if (boite.vide())
        return;

    fill_rect(canvas, boite.x, boite.y, boite.w, boite.h, color_calque_fond);
    fill_rect(canvas, boite.x, boite.y, boite.w, 1, color_calque_bord);
    fill_rect(canvas, boite.x, boite.y + boite.h - 1, boite.w, 1, color_calque_bord);
    fill_rect(canvas, boite.x, boite.y, 1, boite.h, color_calque_bord);
    fill_rect(canvas, boite.x + boite.w - 1, boite.y, 1, boite.h, color_calque_bord);

    int entrees[MenuNombre] {};
    auto const nombre = entrees_menu(entrees);
    for (int rang = 0; rang < nombre; ++rang) {
        auto const haut = boite.y + menu_marge_verticale + rang * menu_hauteur_entree;
        if (rang == s.menu_survole) {
            fill_rect(canvas, boite.x + 1, haut, boite.w - 2, menu_hauteur_entree,
                color_button);
        }
        draw_ui_text(canvas, boite.x + calque_marge,
            haut + (menu_hauteur_entree - ui_text_height) / 2,
            libelle_menu(entrees[rang]), color_calque_texte,
            boite.w - 2 * calque_marge);
    }
}

/// Ou chaque calque doit se trouver a la trame qui vient.
///
/// Un seul endroit calcule les boites, et il les calcule TOUTES : une boite
/// posee ailleurs, au moment ou l'etat change, se serait desynchronisee du
/// premier redimensionnement de fenetre venu -- la bulle est ancree au bas de
/// la surface, et ce bas bouge.
inline void mesure_calques()
{
    auto& s = state();
    s.calques.place(Survol, boite_survol());
    s.calques.place(Recherche, boite_recherche());
    s.calques.place(Menu, boite_menu());
}

/// Dessine les calques que `publie` recouvre, en coordonnees de SURFACE.
///
/// La question se pose pour tous les calques VISIBLES et pas seulement pour
/// ceux qui ont bouge : une capture de page qui repeint sous un calque immobile
/// vient d'effacer ses pixels.
inline void dessine_calques(Canvas const& canvas, BouchaudDegat::Rect publie)
{
    auto& s = state();
    if (BouchaudCalques::doit_redessiner(s.calques.boite(Survol), publie))
        dessine_survol(canvas);
    if (BouchaudCalques::doit_redessiner(s.calques.boite(Recherche), publie))
        dessine_recherche(canvas);
    // Le menu EN DERNIER : il s'ouvre par-dessus tout, y compris par-dessus la
    // barre de recherche, et l'ordre de dessin est ce qui le decide.
    if (BouchaudCalques::doit_redessiner(s.calques.boite(Menu), publie))
        dessine_menu(canvas);
}

// ----------------------------------------------------------------------------
// Composition
// ----------------------------------------------------------------------------

/// Compose la derniere page connue dans la surface partagee, en ne touchant
/// que ce que le moteur a signale comme change, puis annonce ce rectangle-la.
///
/// Ne demande rien au moteur : c'est une copie de pixels deja rasterises.
/// C'est ce qui permet a une lettre tapee dans la barre d'adresse de ne pas
/// declencher une mise en page complete du document.
///
/// BOUCHAUD_CHROME_V18_DEGAT_PARTIEL
///
/// `degat` arrive en coordonnees de PAGE et vient de
/// `LocalNavigable::paint_next_frame()`, qui le calcule en comparant la liste
/// d'affichage a celle de la trame precedente. Il etait calcule puis jete :
/// chaque capture recopiait 1 554 048 pixels et annoncait toute la surface,
/// meme pour un curseur qui clignote. Le choix des rectangles vit dans
/// BouchaudDegat.h, ou il est verifie sur l'hote.
inline bool compose_page(BouchaudDegat::Rect degat)
{
    auto& s = state();
    if (s.surface_fd < 0 || s.gui_fd < 0 || s.surface_width <= 0 || s.surface_height <= 0) {
        warnln("[ladybird-bouchaud] M11_SURFACE_ENV_INVALID surface_fd={} gui_fd={} {}x{}",
            s.surface_fd, s.gui_fd, s.surface_width, s.surface_height);
        return false;
    }

    auto canvas_or_error = mapped_surface();
    if (!canvas_or_error.has_value())
        return false;
    auto canvas = canvas_or_error.release_value();

    auto const* bitmap = (s.last_page.is_valid() && s.last_page.bitmap())
        ? s.last_page.bitmap()
        : nullptr;

    BouchaudDegat::Geometrie geometrie {
        s.surface_width,
        s.surface_height,
        page_origin_y(),
        bitmap ? bitmap->width() : 0,
        bitmap ? bitmap->height() : 0,
    };

    // BOUCHAUD_CHROME_V19_CALQUES
    //
    // Un calque qui bouge oblige a restaurer la page LA OU IL ETAIT, et cette
    // restauration n'existe qu'ici : c'est la seule fonction qui sait recopier
    // `last_page`. Son degat rejoint donc celui du moteur avant le plan, et
    // non apres -- apres, le plan aurait deja decide de ne rien recopier.
    //
    // Les deux reperes different : les calques flottent sur la SURFACE, le
    // degat du moteur porte sur la PAGE.
    mesure_calques();
    auto const degat_calques = s.calques.degat();
    if (!degat_calques.vide()) {
        degat = degat.englobe(BouchaudDegat::Rect {
            degat_calques.x,
            degat_calques.y - geometrie.page_haut,
            degat_calques.w,
            degat_calques.h,
        });
    }

    auto plan = s.suivi_page.planifie(geometrie, degat);

    auto page_top = geometrie.page_haut;
    auto page_height = geometrie.zone_page().h;

    if (plan.rien_a_faire()) {
        // Le moteur a repeint quelque chose qui ne touche pas cette fenetre.
        // Ne pas reveiller le compositeur pour rien est la moitie du gain :
        // l'autre moitie est de ne pas recopier ce qui n'a pas change.
        ++s.page_frames;
        ++s.page_frames_sans_effet;
        send_handshake();
        return true;
    }

    // Le rectangle reellement annonce au compositeur. Il part du plan de page
    // et grandit de ce que le chrome ajoute par-dessus : la barre d'outils si
    // elle est repeinte, la surface entiere sur une trame complete.
    auto publie = plan.publie;

    // La barre d'outils ne bouge pas quand la page bouge. Sur une trame
    // complete -- premiere trame, fenetre redimensionnee -- la surface a cesse
    // de la porter et il faut la repeindre.
    if (plan.complet) {
        draw_toolbar(canvas);
        // Elle vient d'etre repeinte : une recomposition de chrome en attente
        // serait redondante.
        s.chrome_frames_pending = 0;
        publie = BouchaudDegat::Rect { 0, 0, s.surface_width, s.surface_height };
    } else if (s.chrome_frames_pending > 0) {
        // Elle attend une recomposition et une trame partielle passe : la
        // peindre ici plutot que de laisser `tick()` publier un second message
        // pour les memes trente-six lignes. C'est aussi ce qui garantit qu'une
        // lettre tapee dans la barre pendant une capture n'attend pas la
        // frappe suivante pour s'afficher.
        draw_toolbar(canvas);
        s.chrome_frames_pending = 0;
        ++s.chrome_toolbar_frames;
        s.chrome_pixels_written += static_cast<u64>(s.surface_width)
            * static_cast<u64>(min(toolbar_height, s.surface_height));
        publie = publie.englobe(BouchaudDegat::Rect {
            0, 0, s.surface_width, min(toolbar_height, s.surface_height) });
    }

    if (plan.efface_necessaire) {
        fill_rect(canvas, plan.efface.x, page_top + plan.efface.y,
            plan.efface.w, plan.efface.h, color_page_backdrop);
    }

    size_t painted = 0;
    if (bitmap && !plan.copie.vide()) {
        for (int y = 0; y < plan.copie.h; ++y) {
            auto const* source = bitmap->scanline(plan.copie.y + y) + plan.copie.x;
            auto* destination = canvas.row(page_top + plan.copie.y + y) + plan.copie.x;
            for (int x = 0; x < plan.copie.w; ++x)
                destination[x] = source[x] & 0x00ffffffu;
        }
        painted = static_cast<size_t>(plan.copie.w) * static_cast<size_t>(plan.copie.h);
    }

    // Les calques passent APRES la copie : ils flottent au-dessus de la page,
    // et les dessiner avant reviendrait a les recouvrir de ce qu'ils cachent.
    dessine_calques(canvas, publie);
    // La trame va etre publiee : ce qui etait voulu est desormais porte par la
    // surface. Acter avant l'envoi et non apres n'a aucune importance ici --
    // `send_frame_ready` ne peut pas echouer a moitie -- mais acter sur une
    // trame ABANDONNEE en aurait : le suivi croirait la surface a jour.
    s.calques.acte();

    send_handshake();
    if (plan.complet)
        ++s.chrome_full_frames;
    else
        ++s.chrome_partial_frames;
    ++s.page_frames;
    if (plan.efface_necessaire) {
        s.chrome_pixels_written += static_cast<u64>(plan.efface.w) * static_cast<u64>(plan.efface.h);
    }
    s.chrome_pixels_written += static_cast<u64>(painted);

    // `publie` couvre deja tout ce que cette trame a reecrit : la page, la
    // barre d'outils si elle a ete repeinte, les calques. Un rectangle plus
    // petit laisserait le compositeur afficher l'ancien contenu.
    send_frame_ready({ publie.x, publie.y, publie.w, publie.h });

    // Le temoin dit « la premiere trame de PAGE », pas « la premiere trame ».
    // Depuis qu'un `Configure` recompose immediatement, la toute premiere trame
    // publiee peut ne porter que la barre d'outils et du fond : l'annoncer comme
    // premier rendu ferait mesurer a `tools/perf/analyse-fps-hz.py` un temps de
    // premier affichage qui n'affiche rien.
    if (!s.frame_seen && painted > 0) {
        s.frame_seen = true;
        outln("[ladybird-bouchaud] M11_FIRST_FRAME pixels={} viewport={}x{}",
            painted, s.surface_width, page_height);
    }
    return true;
}

/// Recompose toute la fenetre : barre d'outils et page entiere.
///
/// La surface a cesse de porter la trame precedente pour une raison que le
/// degat du moteur ne dit pas -- premiere trame, remappage, geometrie changee
/// par le gestionnaire de fenetres.
inline bool compose_full()
{
    state().suivi_page.invalide();
    return compose_page({});
}

/// Met a jour uniquement les pixels du chrome. Les pixels de page resident deja
/// dans la surface MAP_SHARED et ne sont ni effaces ni recopies.
inline bool compose_toolbar_only()
{
    auto& s = state();
    if (s.surface_fd < 0 || s.gui_fd < 0 || s.surface_width <= 0 || s.surface_height <= 0)
        return false;

    auto canvas_or_error = mapped_surface();
    if (!canvas_or_error.has_value())
        return false;
    auto canvas = canvas_or_error.release_value();
    draw_toolbar(canvas);

    send_handshake();
    ++s.chrome_toolbar_frames;
    auto damage_height = min(toolbar_height, s.surface_height);
    s.chrome_pixels_written += static_cast<u64>(s.surface_width) * static_cast<u64>(damage_height);
    send_frame_ready({ 0, 0, s.surface_width, damage_height });
    return true;
}

/// Recoit une nouvelle capture du moteur et l'affiche.
///
/// C'est le seul point ou `last_page` change. Une capture invalide n'ecrase
/// pas la precedente : mieux vaut reafficher la page d'avant qu'un rectangle
/// vide.
///
/// `degat_*` sont les coordonnees de PAGE du rectangle que le moteur a
/// recalcule, accumulees depuis la capture precedente. Voir BouchaudDegat.h et
/// tools/ladybird/prepare-repaint.py : l'accumulation est ce qui rend le
/// partiel sur, puisque le pump ne capture pas toutes les etapes de rendu.
inline bool present(Gfx::ShareableBitmap const& screenshot, int degat_x, int degat_y,
    int degat_largeur, int degat_hauteur)
{
    auto& s = state();
    auto const valid = screenshot.is_valid() && screenshot.bitmap();
    if (s.frame_after_wheel_pending)
        outln("[ladybird-bouchaud] WEB_SCREENSHOT_READY after_wheel=1 valid={}", valid ? 1 : 0);
    if (valid)
        s.last_page = screenshot;
    return compose_page({ degat_x, degat_y, degat_largeur, degat_hauteur });
}

/// Recoit une capture dont rien ne dit ce qui a change.
///
/// Le chemin M9+M11 -- chrome sans BrowserHost, donc sans processus Compositor
/// et sans calcul de degat -- n'a aucun rectangle a offrir. Chaque capture y
/// est une trame complete. Le dire explicitement vaut mieux que de passer un
/// degat par defaut, qui aurait l'air d'un vrai et recopierait un coin de page.
inline bool present_complet(Gfx::ShareableBitmap const& screenshot)
{
    state().suivi_page.invalide();
    return present(screenshot, 0, 0, 0, 0);
}

// ----------------------------------------------------------------------------
// Barre d'adresse
// ----------------------------------------------------------------------------

inline ByteString address_text()
{
    return state().address.vers_chaine();
}

/// Rend le foyer au document.
///
/// Une seule fonction pour les quatre endroits qui le faisaient, parce que la
/// selection totale doit disparaitre avec le foyer : une surbrillance survivant
/// a un clic dans la page reapparaitrait a la frappe suivante, et la premiere
/// lettre tapee effacerait une URL que plus rien ne montrait comme selectionnee.
inline void defocus_address()
{
    auto& s = state();
    s.address_focused = false;
    s.address.deselectionne();
}

inline void set_address_text(StringView text)
{
    state().address.pose(text);
}

/// Complete une saisie humaine en URL.
///
/// « example.com » n'est pas une URL, et le refuser serait exact mais inutile :
/// personne ne tape le schema. Une entree qui contient un espace ou aucun point
/// est traitee comme une recherche, parce que c'est ce qu'elle est.
inline ByteString normalize_input(ByteString const& raw)
{
    auto trimmed = raw.trim_whitespace();
    if (trimmed.is_empty())
        return {};

    if (trimmed.starts_with("http://"sv) || trimmed.starts_with("https://"sv)
        || trimmed.starts_with("about:"sv) || trimmed.starts_with("data:"sv)
        || trimmed.starts_with("file://"sv))
        return trimmed;

    auto looks_like_host = !trimmed.contains(' ') && trimmed.contains('.');
    if (looks_like_host)
        return ByteString::formatted("https://{}", trimmed);

    // `getenv` rend `char*`, pas `char const*` : sans le type explicite, `auto*`
    // deduit `char*` et l'affectation du litteral devient une conversion que
    // C++11 interdit (`-Wwritable-strings`).
    char const* engine = getenv("BOUCHAUD_SEARCH_URL");
    if (engine == nullptr || *engine == '\0')
        engine = "https://duckduckgo.com/?q=";

    StringBuilder builder;
    builder.append(StringView { engine, strlen(engine) });
    for (size_t index = 0; index < trimmed.length(); ++index) {
        auto character = static_cast<unsigned char>(trimmed[index]);
        auto unreserved = (character >= 'a' && character <= 'z')
            || (character >= 'A' && character <= 'Z')
            || (character >= '0' && character <= '9')
            || character == '-' || character == '_' || character == '.' || character == '~';
        if (unreserved)
            builder.append(static_cast<char>(character));
        else if (character == ' ')
            builder.append('+');
        else
            builder.appendff("%{:02X}", character);
    }
    return builder.to_byte_string();
}

inline void commit_address()
{
    auto& s = state();
    auto target = normalize_input(address_text());
    if (target.is_empty())
        return;

    defocus_address();
    s.loading = true;
    s.status = "chargement...";
    outln("[ladybird-bouchaud] M11_NAVIGATE url={}", target);
    if (s.on_navigate)
        s.on_navigate(target);
}

// ----------------------------------------------------------------------------
// Recherche dans la page
// ----------------------------------------------------------------------------
//
// BOUCHAUD_CHROME_V19_RECHERCHE
//
// LibWeb sait chercher dans un document depuis toujours : `Page::find_in_page()`
// parcourt le texte, deplace la selection sur la correspondance, la fait
// defiler a l'ecran et rend le rang et le total. Rien ne l'appelait. Sur un
// document long, la seule facon de trouver un mot etait de le lire.
//
// Tout est SYNCHRONE : les trois entrees du moteur rendent leur resultat, il
// n'y a donc aucun rappel a attendre et aucun etat a reconcilier. C'est ce qui
// permet au compteur de ne jamais afficher le resultat d'une requete
// precedente.

inline void lance_recherche()
{
    auto& s = state();
    s.calques.salit(Recherche);
    if (s.on_find)
        s.on_find(s.recherche.vers_chaine());
}

/// Ouvre la barre de recherche et lui donne le foyer, tout selectionne.
///
/// Ctrl+F sur une barre deja ouverte reselectionne : c'est ce que fait tout
/// navigateur, et c'est ce qu'on veut quand on cherche un second mot.
inline void ouvre_recherche()
{
    auto& s = state();
    // Le foyer est unique. Deux champs qui recoivent la meme frappe est le
    // genre de defaut qu'on ne decouvre qu'en tapant.
    defocus_address();
    s.recherche_ouverte = true;
    s.recherche_focus = true;
    s.recherche.selectionne_tout();
    s.calques.salit(Recherche);
    // La barre d'adresse vient de perdre le foyer : son cadre change.
    request_chrome_frame();
}

inline void ferme_recherche()
{
    auto& s = state();
    if (!s.recherche_ouverte)
        return;
    s.recherche_ouverte = false;
    s.recherche_focus = false;
    s.recherche_rang = 0;
    s.recherche_total = 0;
    s.recherche_total_connu = false;
    // Le surlignage appartient au DOCUMENT, pas au calque : fermer la barre
    // sans l'effacer laisserait la page marquee par une recherche qui n'existe
    // plus, et rien dans le chrome ne dirait pourquoi.
    if (s.on_find)
        s.on_find(ByteString {});
}

inline void recherche_suivante()
{
    auto& s = state();
    if (s.recherche.est_vide())
        return;
    if (s.on_find_next)
        s.on_find_next();
}

inline void recherche_precedente()
{
    auto& s = state();
    if (s.recherche.est_vide())
        return;
    if (s.on_find_previous)
        s.on_find_previous();
}

/// Ce que le moteur a trouve.
///
/// `total_connu` distingue « zero correspondance » de « je ne sais pas », que
/// `Optional<size_t>` distingue cote LibWeb et qu'un simple entier
/// confondrait : la seconde reponse arrive quand aucune requete n'est encore
/// enregistree, et l'afficher comme « aucun » serait mentir.
inline void set_resultat_recherche(size_t index, bool total_connu, size_t total)
{
    auto& s = state();
    // LibWeb compte a partir de zero -- `current_match_index` est un indice
    // dans son tableau de correspondances. Un humain compte a partir de un, et
    // « 0/12 » se lirait comme un echec.
    s.recherche_rang = total > 0 ? index + 1 : 0;
    s.recherche_total_connu = total_connu;
    s.recherche_total = total;
    s.calques.salit(Recherche);
}


// ----------------------------------------------------------------------------
// Presse-papiers
// ----------------------------------------------------------------------------
//
// BOUCHAUD_CHROME_V19_PRESSE_PAPIERS
//
// Le contenu vit dans le gestionnaire de fenetres, pas ici : copier dans le
// navigateur et coller dans une autre application est ce qui distingue un
// presse-papiers d'un tampon interne. Voir `src/gui/presse_papiers.rs` pour le
// modele -- pousse au client qui a le foyer, jamais lu a la demande -- et pour
// ce que ce choix ferme.
//
// Ce que le chrome garde est une COPIE, mise a jour par le bureau a chaque
// prise de foyer. C'est ce qui permet a Ctrl+V d'etre synchrone : demander
// puis attendre obligerait a suspendre une frappe le temps d'un aller-retour.

inline void copie_vers_le_presse_papiers(ByteString const& texte)
{
    auto& s = state();
    // Un message ne porte que `CHARGE_MAX` octets, et le decodeur du noyau
    // REJETTE le flux entier au-dela -- il ne tronque pas, il se resynchronise.
    // Une selection de six kibioctets couperait donc le canal GUI de la
    // fenetre, ce qui est un tres mauvais prix pour un Ctrl+C.
    auto const longueur = min(texte.length(), static_cast<size_t>(CHARGE_MAX));
    if (longueur < texte.length()) {
        outln("[ladybird-bouchaud] M11_CLIPBOARD_TRUNCATED {} -> {}",
            texte.length(), longueur);
    }
    // La copie locale ET le bureau. Le bureau seul obligerait a attendre la
    // poussee de retour pour coller ce qu'on vient de copier ; la copie locale
    // seule ferait un presse-papiers qui ne sort pas de la fenetre.
    s.presse_papiers = texte.substring(0, longueur);
    send_message(Genre::PressePapiersEcrit, s.presse_papiers.characters(),
        static_cast<u32>(longueur));
}

/// Ctrl+A : selectionne tout, la ou est le foyer.
inline void selectionne_tout_le_foyer()
{
    auto& s = state();
    if (s.recherche_focus) {
        s.recherche.selectionne_tout();
        s.calques.salit(Recherche);
        return;
    }
    if (s.address_focused) {
        s.address.selectionne_tout();
        request_chrome_frame();
        return;
    }
    if (s.on_select_all)
        s.on_select_all();
}

/// Ctrl+C et Ctrl+X, la ou est le foyer.
inline void copie_la_selection(bool coupe)
{
    auto& s = state();
    if (s.recherche_focus || s.address_focused) {
        auto& champ = s.recherche_focus ? s.recherche : s.address;
        // Un champ du chrome ne modelise qu'une selection : tout ou rien. Sans
        // selection, ne rien faire -- ecraser le presse-papiers avec le texte
        // entier serait une surprise, et l'ecraser avec du vide perdrait ce que
        // l'utilisateur avait copie.
        if (!champ.tout_selectionne)
            return;
        copie_vers_le_presse_papiers(champ.vers_chaine());
        if (!coupe)
            return;
        champ.pose({});
        if (s.recherche_focus)
            lance_recherche();
        else
            request_chrome_frame();
        return;
    }

    ByteString texte;
    if (coupe) {
        if (s.on_cut)
            texte = s.on_cut();
    } else if (s.on_copy) {
        texte = s.on_copy();
    }
    // Une selection vide n'efface pas le presse-papiers. Ctrl+C sans rien de
    // selectionne est presque toujours une frappe pour rien, et repondre en
    // effacant ce qu'on avait copie serait la pire reponse possible.
    if (!texte.is_empty())
        copie_vers_le_presse_papiers(texte);
}

/// Ctrl+V, la ou est le foyer.
inline void colle_le_presse_papiers()
{
    auto& s = state();
    if (s.presse_papiers.is_empty())
        return;
    if (s.recherche_focus) {
        s.recherche.colle(s.presse_papiers.view());
        lance_recherche();
        return;
    }
    if (s.address_focused) {
        s.address.colle(s.presse_papiers.view());
        request_chrome_frame();
        return;
    }
    if (s.on_paste)
        s.on_paste(s.presse_papiers);
}

/// Ce que le DOCUMENT vient de mettre dans le presse-papiers.
///
/// `navigator.clipboard.writeText()` et `document.execCommand('copy')`
/// aboutissent ici. LibWeb exige deja une activation transitoire de
/// l'utilisateur avant d'y arriver -- c'est la specification Clipboard qui le
/// demande, et c'est ce qui empeche une page d'ecrire le presse-papiers a
/// l'improviste. Ce que ce chrome ajoute est la borne de taille, appliquee
/// comme pour une copie humaine.
inline void set_presse_papiers_du_document(ByteString const& texte)
{
    copie_vers_le_presse_papiers(texte);
}


// ----------------------------------------------------------------------------
// Menu contextuel
// ----------------------------------------------------------------------------
//
// BOUCHAUD_CHROME_V19_MENU_CONTEXTUEL
//
// Il ne s'ouvre pas sur le clic droit. Il s'ouvre quand LIBWEB le demande,
// apres avoir distribue l'evenement `contextmenu` au document : c'est ce
// detour qui fait qu'une page qui appelle `preventDefault()` -- un editeur de
// texte, une carte, un terminal web -- garde son propre menu. L'ouvrir depuis
// le chrome, sur le bouton, aurait ete plus court et aurait casse ces pages-la
// sans qu'aucun test ne le dise.
//
// Aucune de ses entrees n'a de rappel a elle : elles appellent ce que les
// raccourcis clavier appellent deja. Un menu qui ferait les choses par un
// second chemin finirait par les faire differemment.

inline void ferme_menu()
{
    auto& s = state();
    if (!s.menu_ouvert)
        return;
    s.menu_ouvert = false;
    s.menu_lien = ByteString {};
    s.menu_survole = -1;
}

/// `page_x` et `page_y` sont dans le repere de PAGE : ils viennent du moteur.
inline void ouvre_menu_contextuel(int page_x, int page_y, ByteString const& lien)
{
    auto& s = state();
    s.menu_x = page_x;
    s.menu_y = page_y + page_origin_y();
    s.menu_lien = lien;
    s.menu_survole = -1;
    s.menu_ouvert = true;
    // Le menu prend le foyer : les fleches et Entree lui appartiennent tant
    // qu'il est ouvert, et deux widgets qui recoivent la meme frappe est le
    // genre de defaut qu'on ne voit qu'en tapant.
    defocus_address();
    if (s.recherche_focus) {
        s.recherche_focus = false;
        s.calques.salit(Recherche);
    }
    request_chrome_frame();
}

inline void active_entree_menu(int rang)
{
    auto& s = state();
    int entrees[MenuNombre] {};
    auto const nombre = entrees_menu(entrees);
    if (rang < 0 || rang >= nombre) {
        ferme_menu();
        return;
    }

    auto const entree = entrees[rang];
    // Le lien est copie AVANT la fermeture, qui l'efface.
    auto const lien = s.menu_lien;
    // Fermer avant d'agir : plusieurs entrees ouvrent autre chose -- la barre
    // de recherche -- ou naviguent, et un menu ferme apres coup effacerait ce
    // que l'action vient de mettre a l'ecran.
    ferme_menu();

    switch (entree) {
    case MenuOuvrirLien:
        if (!lien.is_empty() && s.on_navigate) {
            s.loading = true;
            s.status = "chargement...";
            s.on_navigate(lien);
        }
        break;
    case MenuCopierLien:
        if (!lien.is_empty())
            copie_vers_le_presse_papiers(lien);
        break;
    case MenuReculer:
        if (s.on_history_delta)
            s.on_history_delta(-1);
        break;
    case MenuAvancer:
        if (s.on_history_delta)
            s.on_history_delta(1);
        break;
    case MenuRecharger:
        s.loading = true;
        s.status = "chargement...";
        if (s.on_reload)
            s.on_reload();
        break;
    case MenuCopier:
        copie_la_selection(false);
        break;
    case MenuColler:
        colle_le_presse_papiers();
        break;
    case MenuToutSelectionner:
        selectionne_tout_le_foyer();
        break;
    case MenuRechercher:
        ouvre_recherche();
        break;
    default:
        break;
    }
    request_chrome_frame();
}

// ----------------------------------------------------------------------------
// Entrees
// ----------------------------------------------------------------------------

inline Web::UIEvents::MouseButton buttons_from_mask(unsigned mask)
{
    auto buttons = Web::UIEvents::MouseButton::None;
    if (mask & 1u)
        buttons |= Web::UIEvents::MouseButton::Primary;
    if (mask & 2u)
        buttons |= Web::UIEvents::MouseButton::Secondary;
    if (mask & 4u)
        buttons |= Web::UIEvents::MouseButton::Middle;
    return buttons;
}

inline void dispatch_mouse(Web::MouseEvent::Type type, int x, int y, unsigned button_mask, unsigned buttons_mask, double wheel_y)
{
    auto& s = state();
    if (!s.on_mouse_event)
        return;

    Web::MouseEvent event {};
    event.type = type;
    auto point = Gfx::IntPoint { x, y }.to_type<Web::DevicePixels>();
    event.position = point;
    event.screen_position = point;
    event.button = buttons_from_mask(button_mask);
    event.buttons = buttons_from_mask(buttons_mask);
    event.modifiers = Web::UIEvents::KeyModifier::Mod_None;
    event.wheel_delta_x = 0;
    event.wheel_delta_y = wheel_y;
    event.click_count = type == Web::MouseEvent::Type::MouseDown ? 1 : 0;
    s.on_mouse_event(move(event));
    if (type == Web::MouseEvent::Type::MouseWheel)
        outln("[ladybird-bouchaud] WEB_WHEEL_CALLBACK queued=1");
    // M11_PAGE_INPUT_NO_CHROME_COMPOSE:
    // Le pointeur appartient a la page. Si :hover/scroll/clic change le rendu,
    // LibWeb invalide et produit lui-meme une nouvelle frame. Recomposer ici
    // ne ferait que recopier l'ancienne capture sur toute la surface.
}

inline void handle_pointer(int x, int y, unsigned buttons)
{
    auto& s = state();
    auto previous = s.last_buttons;
    auto pressed = buttons & ~previous;
    auto released = previous & ~buttons;
    s.last_buttons = buttons;

    // BOUCHAUD_CHROME_V19_MENU_CONTEXTUEL
    //
    // Le menu prend TOUT le pointeur tant qu'il est ouvert, y compris au-dessus
    // de la barre d'outils. Un clic ailleurs le ferme et ne va pas plus loin :
    // c'est ce que fait tout menu, et laisser passer ce premier clic ferait
    // agir sur ce qu'on voulait seulement quitter des yeux.
    if (s.menu_ouvert) {
        auto const survole = entree_menu_au_point(x, y);
        if (survole != s.menu_survole) {
            s.menu_survole = survole;
            s.calques.salit(Menu);
        }
        if (pressed != 0) {
            if (survole >= 0)
                active_entree_menu(survole);
            else
                ferme_menu();
        }
        s.last_x = x;
        s.last_y = y;
        return;
    }

    auto in_toolbar = y < toolbar_height;

    if (in_toolbar) {
        // La page perd le pointeur des qu'il entre dans la barre : sans cela,
        // un survol reste allume sous une barre d'outils que la page ne voit pas.
        if (s.pointer_in_page && s.on_mouse_event) {
            Web::MouseEvent leave {};
            leave.type = Web::MouseEvent::Type::MouseLeave;
            leave.position = Gfx::IntPoint { s.last_x, 0 }.to_type<Web::DevicePixels>();
            leave.screen_position = leave.position;
            s.on_mouse_event(move(leave));
            s.pointer_in_page = false;
        }

        if (pressed & 1u) {
            // Le foyer suit le clic. Sans cela les touches suivantes iraient
            // encore dans la barre de recherche, restee ouverte sous la barre
            // d'outils qu'on vient d'utiliser.
            if (s.recherche_focus) {
                s.recherche_focus = false;
                s.calques.salit(Recherche);
            }
            if (point_in_button(back_button(), x, y)) {
                outln("[ladybird-bouchaud] M11_HISTORY delta=-1");
                if (s.on_history_delta)
                    s.on_history_delta(-1);
            } else if (point_in_button(forward_button(), x, y)) {
                outln("[ladybird-bouchaud] M11_HISTORY delta=+1");
                if (s.on_history_delta)
                    s.on_history_delta(1);
            } else if (point_in_button(reload_button(), x, y)) {
                outln("[ladybird-bouchaud] M11_RELOAD");
                s.loading = true;
                s.status = "chargement...";
                if (s.on_reload)
                    s.on_reload();
            } else if (point_in_address_field(x, y)) {
                // Un clic POSE un curseur ; il ne selectionne pas. Ctrl+L est
                // la pour cela, et confondre les deux ferait effacer l'URL a
                // qui voulait seulement corriger sa fin.
                s.address_focused = true;
                s.address.pose_curseur_a_la_fin();
            } else {
                defocus_address();
            }
            request_chrome_frame();
        }

        s.last_x = x;
        s.last_y = y;
        return;
    }

    // Un clic dans un calque appartient au calque. Sans cela il traverse la
    // barre de recherche et va selectionner du texte dans la page qu'elle
    // recouvre -- la faute la plus visible qu'une surface flottante puisse
    // commettre. L'evenement entier est consomme, appui comme relachement : ne
    // consommer que l'appui enverrait au document un `mouseup` sans
    // `mousedown`.
    //
    // La bulle de survol, elle, ne prend rien : elle n'a rien a cliquer, et
    // elle s'efface des que le pointeur quitte le lien.
    if (BouchaudCalques::contient(boite_recherche(), x, y)) {
        if (pressed != 0) {
            defocus_address();
            s.recherche_focus = true;
            s.recherche.pose_curseur_a_la_fin();
            s.calques.salit(Recherche);
            request_chrome_frame();
        }
        s.last_x = x;
        s.last_y = y;
        return;
    }

    // Clic dans la page : les barres du chrome rendent le foyer au document,
    // sinon les touches suivantes continueraient d'aller dans une barre. La
    // barre de recherche RESTE ouverte -- F3 doit continuer d'y parcourir les
    // correspondances pendant qu'on lit la page.
    if (pressed != 0) {
        if (s.address_focused) {
            defocus_address();
            request_chrome_frame();
        }
        if (s.recherche_focus) {
            s.recherche_focus = false;
            s.calques.salit(Recherche);
        }
    }

    auto page_x = x;
    auto page_y = y - page_origin_y();
    s.pointer_in_page = true;

    if (page_x != s.last_x || page_y != (s.last_y - page_origin_y()))
        dispatch_mouse(Web::MouseEvent::Type::MouseMove, page_x, page_y, 0, buttons, 0);

    if (pressed != 0)
        dispatch_mouse(Web::MouseEvent::Type::MouseDown, page_x, page_y, pressed, buttons, 0);
    if (released != 0)
        dispatch_mouse(Web::MouseEvent::Type::MouseUp, page_x, page_y, released, buttons, 0);

    s.last_x = x;
    s.last_y = y;
}

inline void handle_wheel(int delta, int x, int y)
{
    auto& s = state();
    // Un menu est ancre a un point de la PAGE, et la molette deplace la page
    // sous lui. Le laisser ouvert le ferait pointer autre chose que ce sur
    // quoi on a clique -- et « ouvrir le lien » ouvrirait alors un lien qui
    // n'est plus la.
    if (s.menu_ouvert)
        ferme_menu();
    outln("[ladybird-bouchaud] M11_WHEEL_RX dx=0 dy={} client_x={} client_y={}", delta, x, y);
    if (y < toolbar_height) {
        outln("[ladybird-bouchaud] WEB_WHEEL_DROP reason=toolbar client_x={} client_y={}", x, y);
        return;
    }

    // Le protocole GUI compte positif vers le haut (convention Qt) ; le DOM
    // compte positif vers le bas. Trois lignes de 18 pixels par cran, la meme
    // valeur que les portages de bureau d'upstream.
    auto page_y = y - page_origin_y();
    auto wheel_y = static_cast<double>(-delta) * 54.0;
    outln("[ladybird-bouchaud] WEB_WHEEL_DISPATCH viewport_x={} viewport_y={} delta_y={}", x, page_y, wheel_y);
    s.wheel_input_pending = true;
    s.frame_after_wheel_pending = true;
    dispatch_mouse(Web::MouseEvent::Type::MouseWheel, x, page_y, 0, s.last_buttons,
        wheel_y);
}

/// Traduit le masque de modificateurs du protocole GUI
/// (`window_manager::modificateur`) vers celui de LibWeb.
inline Web::UIEvents::KeyModifier modifiers_from_mask(u32 mask)
{
    auto modifiers = Web::UIEvents::KeyModifier::Mod_None;
    if (mask & Modificateur::Shift)
        modifiers |= Web::UIEvents::KeyModifier::Mod_Shift;
    if (mask & Modificateur::Ctrl)
        modifiers |= Web::UIEvents::KeyModifier::Mod_Ctrl;
    // LibWeb distingue la touche physique AltGr (`Key_AltGr`) mais son masque
    // de modificateurs ne possede qu'un bit Alt generique. Bouchaud a deja
    // compose le caractere de la couche AltGr avant ce pont ; conserver le bit
    // Alt transmet donc la meilleure metadonnee disponible sans inventer une
    // valeur que l'API amont ne sait pas representer.
    if (mask & (Modificateur::Alt | Modificateur::AltGr))
        modifiers |= Web::UIEvents::KeyModifier::Mod_Alt;
    return modifiers;
}

/// Transmet UNE transition de touche a la page : celle qui s'est reellement
/// produite.
///
/// Ce qui existait avant : le bureau n'envoyait que des appuis, et cette
/// fonction fabriquait le relachement dans la foulee. Une page qui ecoute
/// `keyup` voyait donc chaque touche relachee dans l'instant, une touche
/// maintenue n'existait pas, et la repetition automatique etait indiscernable
/// d'une rafale de frappes. Le pilote PS/2 connaissait pourtant les codes
/// make/break depuis toujours -- ils etaient jetes avant le protocole GUI.
inline void dispatch_key_to_page(
    Web::UIEvents::KeyCode code, u32 code_point, bool insert_text, u32 modifiers, bool pressed, bool repeat)
{
    auto& s = state();
    if (!s.on_key_event)
        return;

    Web::KeyEvent event {};
    event.type = pressed ? Web::KeyEvent::Type::KeyDown : Web::KeyEvent::Type::KeyUp;
    event.key = code;
    event.modifiers = modifiers_from_mask(modifiers);
    event.code_point = code_point;
    event.repeat = repeat;
    // Un relachement n'insere jamais de texte : c'est l'appui qui compose.
    event.should_insert_text = pressed && insert_text;
    s.on_key_event(move(event));
    // Même règle que pour la souris : une touche envoyee au document ne change
    // pas le chrome. Le moteur demandera un repaint si le DOM visuel change.
}

/// Donne le foyer a la barre d'adresse avec tout le texte selectionne.
inline void focus_address_bar()
{
    auto& s = state();
    s.address_focused = true;
    s.address.selectionne_tout();
    request_chrome_frame();
}

/// Les raccourcis du NAVIGATEUR, ceux qui appartiennent au chrome et jamais au
/// document. Rend `true` si la touche a ete consommee.
///
/// # Pourquoi ils arrivent apres le reste
///
/// Le chrome avait des boutons et une barre d'adresse, et rien pour les
/// atteindre au clavier. Ce n'est pas un manque de confort : les touches qui
/// les servent -- F5, Ctrl+L, Alt+fleche -- etaient PERDUES avant le protocole
/// (voir `src/drivers/input/clavier_decodeur.rs`), et il n'y avait donc rien a
/// brancher.
///
/// Ils sont examines avant la barre d'adresse comme avant la page : un
/// raccourci qui ne fonctionne que lorsque le foyer est au bon endroit n'est
/// pas un raccourci. Seul l'APPUI declenche ; le relachement d'un raccourci ne
/// veut rien dire, et le traiter le declencherait deux fois.
inline bool raccourci_navigateur(u32 code, u32 code_point, u32 modifiers, bool appui)
{
    auto& s = state();

    auto const ctrl = (modifiers & Modificateur::Ctrl) != 0;
    auto const alt = (modifiers & Modificateur::Alt) != 0;
    auto const lettre = [code_point](char attendue) {
        return code_point == static_cast<u32>(attendue)
            || code_point == static_cast<u32>(attendue - 32);
    };

    // Rechargement : F5 seule, et Ctrl+R.
    auto const rechargement = (code == ToucheFonction && code_point == 5u && !ctrl && !alt)
        || (code == ToucheCaractere && ctrl && lettre('r'));
    // Historique : Alt+fleche, la convention de tous les navigateurs de bureau.
    auto const historique = alt && (code == ToucheGauche || code == ToucheDroite);
    auto const barre_adresse = ctrl && code == ToucheCaractere && lettre('l');

    // BOUCHAUD_CHROME_V18_ZOOM
    //
    // Sur un clavier AZERTY, `+` et `=` sont la MEME touche -- l'une est
    // l'autre avec Maj -- et les deux doivent donc agrandir : demander a
    // l'utilisateur laquelle il a « vraiment » tapee n'aurait aucun sens.
    // Le chiffre 0, lui, exige deja Maj sur cette disposition ; c'est ce que
    // fait quiconque tape un zero, et rien de plus n'est a inventer.
    auto const caractere = code == ToucheCaractere;
    auto const zoom_plus = ctrl && caractere
        && (code_point == static_cast<u32>('+') || code_point == static_cast<u32>('='));
    auto const zoom_moins = ctrl && caractere && code_point == static_cast<u32>('-');
    auto const zoom_neutre = ctrl && caractere && code_point == static_cast<u32>('0');

    // BOUCHAUD_CHROME_V19_RECHERCHE
    //
    // Ctrl+F ouvre, F3 repete. Les deux existent partout, et la seconde vaut
    // la peine : elle permet de parcourir les correspondances sans garder le
    // foyer dans la barre, donc en continuant a faire defiler la page.
    auto const recherche = ctrl && caractere && lettre('f');
    auto const recherche_repetee = code == ToucheFonction && code_point == 3u && !ctrl && !alt;

    // BOUCHAUD_CHROME_V19_PRESSE_PAPIERS
    //
    // Les quatre raccourcis d'edition appartiennent au CHROME et non au
    // document, comme dans tout navigateur : c'est lui qui sait ou est le
    // foyer -- barre d'adresse, barre de recherche, ou page -- et lui seul qui
    // parle au presse-papiers du bureau. Les laisser au document ferait
    // fonctionner Ctrl+C dans la page et nulle part ailleurs.
    auto const tout_selectionner = ctrl && caractere && lettre('a');
    auto const copier = ctrl && caractere && lettre('c');
    auto const couper = ctrl && caractere && lettre('x');
    auto const coller = ctrl && caractere && lettre('v');

    if (!rechargement && !historique && !barre_adresse
        && !zoom_plus && !zoom_moins && !zoom_neutre
        && !recherche && !recherche_repetee
        && !tout_selectionner && !copier && !couper && !coller)
        return false;

    // Consomme dans les DEUX sens, agit sur l'appui seul.
    //
    // Laisser passer le relachement enverrait a la page un `keyup` sans
    // `keydown` : une page qui compte les deux -- un jeu, un raccourci maintenu
    // -- verrait une touche relachee qu'elle n'a jamais vue enfoncee. C'est
    // exactement le defaut que le pilote PS/2 avait deja corrige en cessant de
    // fabriquer des relachements synthetiques ; le reintroduire ici serait
    // dommage.
    if (!appui)
        return true;

    if (rechargement) {
        s.loading = true;
        s.status = "chargement...";
        if (s.on_reload)
            s.on_reload();
        request_chrome_frame();
        return true;
    }

    if (historique) {
        if (s.on_history_delta)
            s.on_history_delta(code == ToucheGauche ? -1 : 1);
        return true;
    }

    if (zoom_plus || zoom_moins || zoom_neutre) {
        auto const avant = s.zoom_cran;
        if (zoom_plus)
            s.zoom_cran = BouchaudZoom::agrandit(s.zoom_cran);
        else if (zoom_moins)
            s.zoom_cran = BouchaudZoom::reduit(s.zoom_cran);
        else
            s.zoom_cran = BouchaudZoom::cran_neutre;

        // Aux extremites de l'echelle, la touche ne change rien : refaire la
        // mise en page pour le meme facteur couterait une trame complete pour
        // afficher exactement la meme chose.
        if (s.zoom_cran != avant && s.on_zoom)
            s.on_zoom(BouchaudZoom::pourcent(s.zoom_cran));
        return true;
    }

    if (tout_selectionner) {
        selectionne_tout_le_foyer();
        return true;
    }

    if (copier || couper) {
        copie_la_selection(couper);
        return true;
    }

    if (coller) {
        colle_le_presse_papiers();
        return true;
    }

    if (recherche) {
        ouvre_recherche();
        return true;
    }

    if (recherche_repetee) {
        // F3 sur une barre fermee l'ouvre : c'est ce qu'attend quelqu'un qui
        // vient de la fermer par Echap et se ravise.
        if (!s.recherche_ouverte)
            ouvre_recherche();
        else if ((modifiers & Modificateur::Shift) != 0)
            recherche_precedente();
        else
            recherche_suivante();
        return true;
    }

    focus_address_bar();
    return true;
}

inline void handle_key(u32 code, u32 code_point, u32 modifiers, u32 pressed)
{
    auto& s = state();
    auto const appui = pressed != 0;

    if (raccourci_navigateur(code, code_point, modifiers, appui))
        return;

    // Le menu contextuel passe avant tout le reste : il est ouvert PAR-DESSUS,
    // et c'est lui que l'utilisateur regarde.
    if (s.menu_ouvert) {
        if (!appui)
            return;
        int entrees[MenuNombre] {};
        auto const nombre = entrees_menu(entrees);
        switch (code) {
        case ToucheBas:
            // Depuis « rien de survole », la premiere fleche vers le bas
            // designe la premiere entree : `-1 + 1 == 0`.
            s.menu_survole = nombre > 0 ? (s.menu_survole + 1) % nombre : -1;
            s.calques.salit(Menu);
            break;
        case ToucheHaut:
            s.menu_survole = nombre > 0
                ? (s.menu_survole <= 0 ? nombre - 1 : s.menu_survole - 1)
                : -1;
            s.calques.salit(Menu);
            break;
        case ToucheEntree:
            active_entree_menu(s.menu_survole);
            break;
        case ToucheEchap:
            ferme_menu();
            break;
        default:
            break;
        }
        return;
    }

    // BOUCHAUD_CHROME_V19_RECHERCHE
    //
    // La barre de recherche a le foyer avant la barre d'adresse : elle est
    // ouverte par-dessus, et c'est elle que l'utilisateur regarde. Comme la
    // barre d'adresse, c'est un widget du chrome et non un document : elle
    // n'agit que sur l'appui.
    if (s.recherche_focus) {
        if (!appui)
            return;
        if (s.recherche.applique(code, code_point)) {
            // La requete a change : relancer. LibWeb repart du debut du
            // document a chaque requete nouvelle, ce qui est ce qu'on veut --
            // « exampl » et « example » n'ont pas les memes correspondances.
            lance_recherche();
            return;
        }
        switch (code) {
        case ToucheEntree:
            // Entree passe a la correspondance suivante, Maj+Entree a la
            // precedente : la convention de tous les navigateurs.
            if ((modifiers & Modificateur::Shift) != 0)
                recherche_precedente();
            else
                recherche_suivante();
            break;
        case ToucheEchap:
            ferme_recherche();
            break;
        default:
            s.recherche.deselectionne();
            s.calques.salit(Recherche);
            break;
        }
        return;
    }

    // La barre d'adresse est un widget du chrome, pas un document : elle
    // n'agit que sur l'appui. La page, elle, recoit les deux transitions.
    if (s.address_focused) {
        if (!appui)
            return;
        if (!s.address.applique(code, code_point)) {
            switch (code) {
            case ToucheEntree:
                commit_address();
                break;
            case ToucheEchap:
                // Echap rend le foyer a la page et restaure l'URL affichee :
                // une saisie abandonnee ne doit pas laisser un texte qui ne
                // correspond plus a ce qui est a l'ecran.
                defocus_address();
                set_address_text(s.committed_url.view());
                break;
            case ToucheTabulation:
            default:
                // Une touche que le champ n'a pas prise defait quand meme la
                // selection totale : sinon une surbrillance survivrait a une
                // touche qui ne l'a pas remplacee, et la frappe suivante
                // effacerait l'URL sans prevenir.
                s.address.deselectionne();
                break;
            }
        }
        request_chrome_frame();
        return;
    }

    switch (code) {
    case ToucheCaractere:
        dispatch_key_to_page(Web::UIEvents::code_point_to_key_code(code_point), code_point, true, modifiers, appui, false);
        break;
    case ToucheEntree:
        dispatch_key_to_page(Web::UIEvents::KeyCode::Key_Return, '\n', true, modifiers, appui, false);
        break;
    case ToucheRetour:
        dispatch_key_to_page(Web::UIEvents::KeyCode::Key_Backspace, 0, false, modifiers, appui, false);
        break;
    case ToucheTabulation:
        dispatch_key_to_page(Web::UIEvents::KeyCode::Key_Tab, '\t', false, modifiers, appui, false);
        break;
    case ToucheHaut:
        dispatch_key_to_page(Web::UIEvents::KeyCode::Key_Up, 0, false, modifiers, appui, false);
        break;
    case ToucheBas:
        dispatch_key_to_page(Web::UIEvents::KeyCode::Key_Down, 0, false, modifiers, appui, false);
        break;
    case ToucheGauche:
        dispatch_key_to_page(Web::UIEvents::KeyCode::Key_Left, 0, false, modifiers, appui, false);
        break;
    case ToucheDroite:
        dispatch_key_to_page(Web::UIEvents::KeyCode::Key_Right, 0, false, modifiers, appui, false);
        break;
    // Le pave de navigation. Rien de special a faire : LibWeb sait deja faire
    // defiler un document, un cadre, une zone en `overflow:auto`, et respecter
    // un `preventDefault`. Ce qui manquait, c'etait que les touches arrivent.
    case TouchePageHaut:
        dispatch_key_to_page(Web::UIEvents::KeyCode::Key_PageUp, 0, false, modifiers, appui, false);
        break;
    case TouchePageBas:
        dispatch_key_to_page(Web::UIEvents::KeyCode::Key_PageDown, 0, false, modifiers, appui, false);
        break;
    case ToucheDebut:
        dispatch_key_to_page(Web::UIEvents::KeyCode::Key_Home, 0, false, modifiers, appui, false);
        break;
    case ToucheFin:
        dispatch_key_to_page(Web::UIEvents::KeyCode::Key_End, 0, false, modifiers, appui, false);
        break;
    case ToucheSupprimer:
        dispatch_key_to_page(Web::UIEvents::KeyCode::Key_Delete, 0, false, modifiers, appui, false);
        break;
    case ToucheInserer:
        dispatch_key_to_page(Web::UIEvents::KeyCode::Key_Insert, 0, false, modifiers, appui, false);
        break;
    case ToucheFonction:
        // F5 a deja ete consommee par `raccourci_navigateur`. Les autres
        // appartiennent au document : une page a le droit d'ecouter F1 ou F12,
        // et les avaler ici en ferait des touches mortes.
        if (code_point >= 1u && code_point <= 12u) {
            auto const touche = static_cast<Web::UIEvents::KeyCode>(
                static_cast<int>(Web::UIEvents::KeyCode::Key_F1) + static_cast<int>(code_point) - 1);
            dispatch_key_to_page(touche, 0, false, modifiers, appui, false);
        }
        break;
    case ToucheEchap:
        // Echap arrete le chargement en cours, comme dans tout navigateur. Le
        // gestionnaire de fenetres nous le laisse precisement pour cela.
        //
        // L'arret est declenche par l'APPUI seul : le faire aussi au
        // relachement le demanderait deux fois par frappe, et la seconde
        // porterait sur un chargement deja arrete.
        if (appui && s.loading) {
            s.loading = false;
            s.status = "arrete";
            if (s.on_stop)
                s.on_stop();
            // Ici le chrome change reellement ("arrete"), donc une seule
            // recomposition est legitime.
            request_chrome_frame();
        }
        dispatch_key_to_page(Web::UIEvents::KeyCode::Key_Escape, 0, false, modifiers, appui, false);
        break;
    default:
        break;
    }
    // Pas de request_chrome_frame() ici : hors barre d'adresse, ces touches
    // appartiennent au document et son invalidation pilote le rendu.
}

// ----------------------------------------------------------------------------
// Protocole GUI — lecture
// ----------------------------------------------------------------------------

inline u32 read_u32(u8 const* data, size_t offset)
{
    return static_cast<u32>(data[offset])
        | (static_cast<u32>(data[offset + 1]) << 8)
        | (static_cast<u32>(data[offset + 2]) << 16)
        | (static_cast<u32>(data[offset + 3]) << 24);
}

inline i32 read_i32(u8 const* data, size_t offset)
{
    return static_cast<i32>(read_u32(data, offset));
}

inline void handle_message(u16 kind, u8 const* payload, u32 size)
{
    auto& s = state();
    switch (kind) {
    case Genre::Configure:
        if (size >= 16) {
            auto width = static_cast<int>(read_u32(payload, 4));
            auto height = static_cast<int>(read_u32(payload, 8));
            // BOUCHAUD_CHROME_V17_SURFACE_REDIMENSIONNABLE
            //
            // La geometrie etait NOTEE et non adoptee : le chrome journalisait
            // la nouvelle taille puis continuait de peindre l'ancienne. Le
            // bouton plein ecran de la barre de titre etait donc inerte des
            // deux cotes a la fois.
            //
            // Elle est bornee a l'allocation. Le compositeur la borne deja,
            // mais un client ne doit pas dependre de la prudence de son
            // serveur pour ne pas ecrire hors de sa propre projection.
            width = min(width, s.surface_alloc_width);
            height = min(height, s.surface_alloc_height);
            if (width > 0 && height > 0 && (width != s.surface_width || height != s.surface_height)) {
                outln("[ladybird-bouchaud] M11_CONFIGURE {}x{} (alloue {}x{})",
                    width, height, s.surface_alloc_width, s.surface_alloc_height);
                s.surface_width = width;
                s.surface_height = height;

                // La zone nouvellement decouverte n'a jamais ete peinte : elle
                // porte ce que l'allocation contenait. Recomposer tout de suite
                // avec la derniere capture connue evite de montrer cela pendant
                // le temps -- une remise en page complete -- que le moteur met a
                // produire une trame a la nouvelle taille.
                compose_full();

                // BOUCHAUD_CHROME_V18_VIEWPORT_SUIT_LA_FENETRE
                //
                // La fenetre s'agrandissait, le chrome peignait plus grand, et
                // le moteur continuait de mettre en page a l'ancienne largeur :
                // la page restait coupee au meme endroit dans une fenetre plus
                // large. Le bouton plein ecran redimensionnait le cadre sans
                // rien changer a ce qu'il encadre.
                if (s.on_resize)
                    s.on_resize(s.surface_width, viewport_height());
                else if (s.on_repaint)
                    s.on_repaint();
            }
        }
        break;
    case Genre::Key:
        if (size >= 20)
            handle_key(read_u32(payload, 4), read_u32(payload, 12), read_u32(payload, 8), read_u32(payload, 16));
        break;
    case Genre::Pointer:
        if (size >= 16)
            handle_pointer(read_i32(payload, 4), read_i32(payload, 8), read_u32(payload, 12));
        break;
    case Genre::Wheel:
        if (size >= 16)
            handle_wheel(read_i32(payload, 4), read_i32(payload, 8), read_i32(payload, 12));
        break;
    case Genre::PressePapiers:
        // Le contenu arrive sans que rien ne l'ait demande : c'est le bureau
        // qui pousse, a la prise de foyer. Voir `src/gui/presse_papiers.rs`.
        {
            StringBuilder builder;
            for (u32 index = 0; index < size; ++index)
                builder.append(static_cast<char>(payload[index]));
            s.presse_papiers = builder.to_byte_string();
        }
        break;
    case Genre::CloseRequest:
        outln("[ladybird-bouchaud] M11_CLOSE_REQUEST");
        if (s.on_close)
            s.on_close();
        break;
    default:
        break;
    }
}

/// Vide le canal GUI et traite tous les messages entiers qu'il contient.
///
/// Appelee par un `Core::Notifier` **et** par un minuteur : la sonde M9 a montre
/// qu'un notificateur de lecture pouvait rester endormi sous Bouchaud alors que
/// la donnee etait deja la. Un navigateur dont la souris s'arrete par
/// intermittence est inutilisable ; le minuteur est la ceinture.
inline void drain()
{
    auto& s = state();
    if (s.gui_fd < 0)
        return;

    u8 buffer[4096];
    for (;;) {
        auto received = read(s.gui_fd, buffer, sizeof(buffer));
        if (received > 0) {
            s.incoming.append(buffer, static_cast<size_t>(received));
            continue;
        }
        if (received < 0 && errno == EINTR)
            continue;
        break;
    }

    constexpr size_t header_size = TAILLE_ENTETE;
    size_t offset = 0;
    while (s.incoming.size() - offset >= header_size) {
        auto const* header = s.incoming.data() + offset;
        auto magic = read_u32(header, 0);
        if (magic != MAGIC) {
            // Flux desynchronise : on ne devine pas, on jette. Le protocole est
            // idempotent (positions absolues), la prochaine trame corrige tout.
            warnln("[ladybird-bouchaud] M11_GUI_STREAM_DESYNC");
            offset = s.incoming.size();
            break;
        }
        auto kind = static_cast<u16>(header[6] | (header[7] << 8));
        auto payload_size = read_u32(header, 8);
        if (payload_size > CHARGE_MAX) {
            warnln("[ladybird-bouchaud] M11_GUI_PAYLOAD_TOO_LARGE {}", payload_size);
            offset = s.incoming.size();
            break;
        }
        if (s.incoming.size() - offset < header_size + payload_size)
            break;

        handle_message(kind, header + header_size, payload_size);
        offset += header_size + payload_size;
    }

    if (offset > 0)
        s.incoming.remove(0, offset);
}

inline bool wheel_input_pending()
{
    return state().wheel_input_pending;
}

inline void wheel_handled_and_capture_requested(int result)
{
    auto& s = state();
    if (!s.wheel_input_pending)
        return;
    s.wheel_input_pending = false;
    outln("[ladybird-bouchaud] WEB_WHEEL_HANDLED result={} capture=scheduled", result);
}

/// Un tic du minuteur : lire les entrees, puis recomposer si le chrome a change.
///
/// Ce que ce tic ne fait plus : reclamer une trame de page. Il le faisait a
/// chaque tic pendant un chargement, et `queue_screenshot_task()` appelle
/// `set_needs_repaint()` — donc soixante fois par seconde nous repondions
/// « oui, repeins » a la question que le moteur n'avait pas encore posee. Son
/// modele d'invalidation en devenait inoperant, et la machine passait 90 a 98 %
/// de son unique cœur a remettre en page un document inchange.
///
/// Le contenu de la page arrive maintenant par `present()`, quand LibWeb a
/// decide qu'il fallait repeindre. Voir tools/ladybird/prepare-repaint.py.
inline void tick()
{
    drain();

    auto& s = state();

    // BOUCHAUD_CHROME_V19_CALQUES
    //
    // Les calques d'abord. Un calque qui apparait, se deplace ou disparait
    // oblige a restaurer la page dessous, et `compose_toolbar_only()` ne sait
    // pas faire cela : il ne touche pas un pixel de page. La composition de
    // page peint aussi la barre d'outils si elle attend, donc le compteur est
    // traite au passage et rien n'est publie deux fois.
    mesure_calques();
    if (!s.calques.degat().vide()) {
        compose_page({});
        return;
    }

    if (s.chrome_frames_pending <= 0)
        return;

    --s.chrome_frames_pending;
    compose_toolbar_only();
}

// ----------------------------------------------------------------------------
// Initialisation
// ----------------------------------------------------------------------------

inline void initialize_from_environment()
{
    auto& s = state();
    auto* gui = getenv("BO_GUI_FD");
    auto* surface = getenv("BO_SURFACE_FD");
    s.gui_fd = gui ? atoi(gui) : -1;
    s.surface_fd = surface ? atoi(surface) : -1;
    // `BO_SURFACE_*` decrit l'ALLOCATION : c'est ce que le client projette.
    // La zone utile, elle, arrive par le premier `Configure`, que le
    // compositeur envoie des l'ouverture de la fenetre. Tant qu'il n'est pas
    // arrive, peindre toute l'allocation est sans danger -- le compositeur ne
    // recopie que la zone qu'il a annoncee.
    s.surface_alloc_width = environment_int("BO_SURFACE_WIDTH", 1100);
    s.surface_alloc_height = environment_int("BO_SURFACE_HEIGHT", 604);
    s.surface_width = s.surface_alloc_width;
    s.surface_height = s.surface_alloc_height;
    s.surface_stride = environment_int("BO_SURFACE_STRIDE", s.surface_alloc_width * 4);

    if (s.gui_fd >= 0) {
        auto flags = fcntl(s.gui_fd, F_GETFL, 0);
        if (flags >= 0)
            fcntl(s.gui_fd, F_SETFL, flags | O_NONBLOCK);
    }

    outln("[ladybird-bouchaud] M11_CHROME gui_fd={} surface_fd={} surface={}x{} toolbar={}",
        s.gui_fd, s.surface_fd, s.surface_width, s.surface_height, toolbar_height);
}

/// L'adresse du lien sous le pointeur, ou une chaine vide pour l'effacer.
///
/// Pas de `request_chrome_frame()` : la bulle n'est pas la barre d'outils, et
/// une recomposition de barre ne restaurerait pas la page sous l'ancienne
/// bulle. C'est `tick()` qui verra le calque bouger, au prochain tic de seize
/// millisecondes.
inline void set_survol_url(ByteString const& url)
{
    auto& s = state();
    if (s.survol_url == url)
        return;
    s.survol_url = url;
}

inline void clear_survol_url()
{
    set_survol_url(ByteString {});
}

/// Prend une `ByteString` et non une `StringView` : les appelants passent
/// `url.to_byte_string()`, un temporaire dont AK **supprime** `view()` pour
/// empecher exactement la vue pendante que cela produirait. La reference lie le
/// temporaire jusqu'a la fin de l'appel.
inline void set_committed_url(ByteString const& url)
{
    auto& s = state();
    s.committed_url = url;
    s.secure = url.starts_with("https://"sv);
    if (!s.address_focused)
        set_address_text(url.view());
    request_chrome_frame();
}

inline void set_loading(bool loading, StringView status)
{
    auto& s = state();
    // Un document qui commence a charger n'a plus de lien survole : celui
    // qu'on affichait appartenait au document precedent, et LibWeb n'enverra
    // pas de `unhover` pour un element qui n'existe plus. Sans cela la bulle
    // resterait, et montrerait une adresse sans rapport avec ce qui est a
    // l'ecran -- exactement ce contre quoi elle existe.
    if (loading && !s.loading)
        clear_survol_url();
    s.loading = loading;
    s.status = status;
    request_chrome_frame();
}

inline void set_title(ByteString const& title)
{
    state().title = title;
    send_title();
}

}

#endif
