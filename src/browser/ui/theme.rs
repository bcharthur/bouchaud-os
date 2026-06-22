//! Palette et constantes visuelles de Nautile — theme inspiré Chrome/Firefox.
//!
//! Toutes les mesures sont en pixels ; les couleurs sont en 0x00RRGGBB.

// ── Barre d'onglets ─────────────────────────────────────────────────────────
pub const TABS_H: usize = 20;
pub const TAB_ACTIVE_BG:   u32 = 0xffffff;
pub const TAB_INACTIVE_BG: u32 = 0xdde1e7;
pub const TAB_TEXT_ACTIVE: u32 = 0x202124;
pub const TAB_TEXT_INACT:  u32 = 0x5f6368;
pub const TAB_ACCENT:      u32 = 0x1a73e8; // trait bas onglet actif
pub const TAB_BORDER:      u32 = 0xc8ccd0;
pub const TAB_MAX_W:       usize = 160;
pub const TAB_MIN_W:       usize = 50;
pub const TAB_CLOSE_W:     usize = 12;
pub const NEW_TAB_W:       usize = 20;

// ── Barre d'outils ───────────────────────────────────────────────────────────
pub const TOOLBAR_H: usize = 22;
pub const TOOLBAR_BG: u32  = 0xf1f3f4;
pub const SEP_COLOR: u32   = 0xdadce0;

// Chrome total (onglets + barre) : c'est ce qui est retranche de la hauteur utile
pub const CHROME_H: usize = TABS_H + TOOLBAR_H;

// ── Boutons de navigation ────────────────────────────────────────────────────
pub const BTN_W: usize = 16;
pub const BTN_H: usize = 14;
// Positions X relatives au debut de la barre d'outils (bx inclus)
pub const BTN_MARGIN_L: usize = 4;
pub const BACK_BX:      usize = BTN_MARGIN_L;
pub const FWD_BX:       usize = BACK_BX    + BTN_W + 2;
pub const REFRESH_BX:   usize = FWD_BX     + BTN_W + 2;
pub const HOME_BX:      usize = REFRESH_BX + BTN_W + 2;
pub const ADDR_BX:      usize = HOME_BX    + BTN_W + 6; // debut de la barre URL
pub const ADDR_R_PAD:   usize = 4;                      // marge droite URL

pub const BTN_FG_ON:  u32 = 0x3c4043;
pub const BTN_FG_OFF: u32 = 0xbbbfc6;
pub const BTN_BG:     u32 = 0xf1f3f4;

// ── Barre d'adresse ──────────────────────────────────────────────────────────
pub const ADDR_H:        usize = 14;
pub const ADDR_BG:       u32   = 0xffffff;
pub const ADDR_BDR_DEF:  u32   = 0xdadce0;
pub const ADDR_BDR_FOC:  u32   = 0x1a73e8;
pub const ADDR_FG:       u32   = 0x202124;
pub const ADDR_HINT_FG:  u32   = 0x9aa0a6;
pub const ADDR_HTTPS_FG: u32   = 0x1e8e3e;
pub const ADDR_HTTP_FG:  u32   = 0xe53935;

// ── Ascenseur ────────────────────────────────────────────────────────────────
pub const SCROLL_W:     usize = 8;
pub const SCROLL_TRACK: u32   = 0xf1f3f4;
pub const SCROLL_THUMB: u32   = 0xbbbfc6;

// ── Chargement ───────────────────────────────────────────────────────────────
pub const LOAD_FG: u32   = 0x1a73e8;
pub const LOAD_H:  usize = 3;
