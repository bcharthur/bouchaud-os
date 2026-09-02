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
 * Pas d'onglets (M13), pas de bac a sable (M14), pas de rendu de texte vectoriel
 * dans le chrome : la barre d'outils utilise la police bitmap 8x8 du noyau, la
 * meme que `src/drivers/gfx/font.rs`, pour ne dependre d'aucune API de dessin
 * susceptible de bouger chez upstream.
 */

#pragma once

// Atlas de glyphes DejaVu, genere par tools/ladybird/chrome/fabrique-atlas.py.
// C'est de la donnee : ce fichier ne gagne aucune dependance de dessin.
#include "BouchaudAtlas.h"

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
inline constexpr u32 color_glyph = 0x00'e8'ea'ed;
inline constexpr u32 color_glyph_off = 0x00'6b'71'78;
inline constexpr u32 color_secure = 0x00'1e'8e'3e;
inline constexpr u32 color_insecure = 0x00'c5'39'29;
inline constexpr u32 color_page_backdrop = 0x00'ff'ff'ff;

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
    Surface = 0x100,
    Configure = 0x101,
    Focus = 0x102,
    Key = 0x103,
    Pointer = 0x104,
    Wheel = 0x105,
    CloseRequest = 0x106,
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
// Etat
// ----------------------------------------------------------------------------

struct State {
    // Descripteurs et geometrie fournis par le gestionnaire de fenetres.
    int gui_fd { -1 };
    int surface_fd { -1 };
    int surface_width { 0 };
    int surface_height { 0 };
    int surface_stride { 0 };
    // La surface GUI garde le même fd et la même géométrie pendant la vie du
    // client. La mapper une fois évite mmap/munmap et les shootdowns TLB à
    // chaque frame chrome/page.
    u8* surface_mapping { nullptr };
    size_t surface_mapping_bytes { 0 };

    // Barre d'adresse. Le texte est conserve en octets ASCII : le pilote clavier
    // du bureau n'expose pas encore de disposition non latine, et pretendre le
    // contraire ici ne rendrait pas la saisie plus juste.
    Vector<u8> address;
    size_t caret { 0 };
    bool address_focused { false };

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

    // Rendu M11: compteurs cumulatifs, journalises par paquets de 16 trames.
    u64 chrome_full_frames { 0 };
    u64 chrome_toolbar_frames { 0 };
    u64 page_frames { 0 };
    u64 chrome_pixels_written { 0 };
    u64 published_frames { 0 };

    // Rappels vers WebContent. Poses par `ConnectionFromClient::bouchaud_m11_start`.
    Function<void(Web::MouseEvent)> on_mouse_event;
    Function<void(Web::KeyEvent)> on_key_event;
    Function<void(ByteString)> on_navigate;
    Function<void(int)> on_history_delta;
    Function<void()> on_reload;
    Function<void()> on_stop;
    Function<void()> on_repaint;
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
    outln("[ladybird-bouchaud] M11_RENDER_STATS full={} toolbar={} page={} pixels={}",
        s.chrome_full_frames, s.chrome_toolbar_frames, s.page_frames, s.chrome_pixels_written);
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
    if (s.frame_after_wheel_pending && damage.y == 0 && damage.height == s.surface_height) {
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
    auto height_rows = static_cast<size_t>(s.surface_height);
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
    auto text_y = button_top + (button_height - glyph_height * 2) / 2;
    auto available = field_w - 20;

    StringBuilder builder;
    for (auto byte : s.address)
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
            auto caret = min(s.caret, address_text.length());
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
    draw_text(canvas, text_x, text_y, visible.view(), color_field_text, 2, available);

    if (s.address_focused) {
        auto caret = min(s.caret, address_text.length());
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
// Composition
// ----------------------------------------------------------------------------

/// Compose la barre d'outils et la derniere page connue dans la surface
/// partagee, puis annonce la trame au compositeur.
///
/// Ne demande rien au moteur : c'est une copie de pixels deja rasterises.
/// C'est ce qui permet a une lettre tapee dans la barre d'adresse de ne pas
/// declencher une mise en page complete du document.
inline bool compose_full()
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

    draw_toolbar(canvas);

    auto page_top = page_origin_y();
    auto page_height = s.surface_height - page_top;
    fill_rect(canvas, 0, page_top, s.surface_width, page_height, color_page_backdrop);

    size_t painted = 0;
    if (s.last_page.is_valid() && s.last_page.bitmap()) {
        auto const& bitmap = *s.last_page.bitmap();
        auto copy_width = min(bitmap.width(), s.surface_width);
        auto copy_height = min(bitmap.height(), page_height);
        for (int y = 0; y < copy_height; ++y) {
            auto const* source = bitmap.scanline(y);
            auto* destination = canvas.row(page_top + y);
            for (int x = 0; x < copy_width; ++x)
                destination[x] = source[x] & 0x00ffffffu;
        }
        painted = static_cast<size_t>(copy_width) * static_cast<size_t>(copy_height);
    }

    send_handshake();
    ++s.chrome_full_frames;
    ++s.page_frames;
    s.chrome_pixels_written += static_cast<u64>(s.surface_width) * static_cast<u64>(s.surface_height)
        + static_cast<u64>(painted);
    send_frame_ready({ 0, 0, s.surface_width, s.surface_height });

    if (!s.frame_seen) {
        s.frame_seen = true;
        outln("[ladybird-bouchaud] M11_FIRST_FRAME pixels={} viewport={}x{}",
            painted, s.surface_width, page_height);
    }
    return true;
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
inline bool present(Gfx::ShareableBitmap const& screenshot)
{
    auto& s = state();
    auto const valid = screenshot.is_valid() && screenshot.bitmap();
    if (s.frame_after_wheel_pending)
        outln("[ladybird-bouchaud] WEB_SCREENSHOT_READY after_wheel=1 valid={}", valid ? 1 : 0);
    if (valid)
        s.last_page = screenshot;
    // La page vient d'etre recomposee : une recomposition de chrome encore en
    // attente serait redondante.
    s.chrome_frames_pending = 0;
    return compose_full();
}

// ----------------------------------------------------------------------------
// Barre d'adresse
// ----------------------------------------------------------------------------

inline ByteString address_text()
{
    StringBuilder builder;
    for (auto byte : state().address)
        builder.append(static_cast<char>(byte));
    return builder.to_byte_string();
}

inline void set_address_text(StringView text)
{
    auto& s = state();
    s.address.clear_with_capacity();
    for (size_t index = 0; index < text.length(); ++index)
        s.address.append(static_cast<u8>(text[index]));
    s.caret = s.address.size();
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

    s.address_focused = false;
    s.loading = true;
    s.status = "chargement...";
    outln("[ladybird-bouchaud] M11_NAVIGATE url={}", target);
    if (s.on_navigate)
        s.on_navigate(target);
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
                s.address_focused = true;
                s.caret = s.address.size();
            } else {
                s.address_focused = false;
            }
            request_chrome_frame();
        }

        s.last_x = x;
        s.last_y = y;
        return;
    }

    // Clic dans la page : la barre d'adresse rend le foyer au document, sinon
    // les touches suivantes continueraient d'aller dans la barre.
    if (pressed != 0 && s.address_focused) {
        s.address_focused = false;
        request_chrome_frame();
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

inline void handle_key(u32 code, u32 code_point, u32 modifiers, u32 pressed)
{
    auto& s = state();
    auto const appui = pressed != 0;

    // La barre d'adresse est un widget du chrome, pas un document : elle
    // n'agit que sur l'appui. La page, elle, recoit les deux transitions.
    if (s.address_focused) {
        if (!appui)
            return;
        switch (code) {
        case ToucheCaractere:
            if (code_point >= 0x20 && code_point < 0x7f) {
                if (s.caret > s.address.size())
                    s.caret = s.address.size();
                s.address.insert(s.caret, static_cast<u8>(code_point));
                ++s.caret;
            }
            break;
        case ToucheRetour:
            if (s.caret > 0 && !s.address.is_empty()) {
                --s.caret;
                s.address.remove(s.caret);
            }
            break;
        case ToucheGauche:
            if (s.caret > 0)
                --s.caret;
            break;
        case ToucheDroite:
            if (s.caret < s.address.size())
                ++s.caret;
            break;
        case ToucheEntree:
            commit_address();
            break;
        case ToucheEchap:
            // Echap rend le foyer a la page et restaure l'URL affichee : une
            // saisie abandonnee ne doit pas laisser un texte qui ne correspond
            // plus a ce qui est a l'ecran.
            s.address_focused = false;
            set_address_text(s.committed_url.view());
            break;
        case ToucheTabulation:
        default:
            break;
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
            // La surface n'est pas reallouee par ce jalon (§11 du protocole) :
            // on note la geometrie sans y croire davantage que la surface.
            if (width > 0 && height > 0 && (width != s.surface_width || height != s.surface_height)) {
                outln("[ladybird-bouchaud] M11_CONFIGURE {}x{} (surface {}x{})",
                    width, height, s.surface_width, s.surface_height);
                // Le seul cas ou le chrome sait avant le moteur qu'il faut
                // repeindre la page : la geometrie a change, et rien dans le
                // document ne s'en est apercu.
                if (s.on_repaint)
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
    s.surface_width = environment_int("BO_SURFACE_WIDTH", 1100);
    s.surface_height = environment_int("BO_SURFACE_HEIGHT", 604);
    s.surface_stride = environment_int("BO_SURFACE_STRIDE", s.surface_width * 4);

    if (s.gui_fd >= 0) {
        auto flags = fcntl(s.gui_fd, F_GETFL, 0);
        if (flags >= 0)
            fcntl(s.gui_fd, F_SETFL, flags | O_NONBLOCK);
    }

    outln("[ladybird-bouchaud] M11_CHROME gui_fd={} surface_fd={} surface={}x{} toolbar={}",
        s.gui_fd, s.surface_fd, s.surface_width, s.surface_height, toolbar_height);
}

/// Hauteur utile pour la page, une fois la barre d'outils retiree.
inline int viewport_height()
{
    auto height = state().surface_height - toolbar_height;
    return height > 0 ? height : state().surface_height;
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
