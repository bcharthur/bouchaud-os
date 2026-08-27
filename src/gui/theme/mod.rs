//! Semantic visual source of truth for Bouchaud Shell and window chrome.
mod tokens;
pub use tokens::*;

pub const BOUCHAUD_THEME: BouchaudTheme = BouchaudTheme {
    colors: Colors { background: COLOR_BACKGROUND, surface: COLOR_SURFACE, surface_elevated: COLOR_SURFACE_ELEVATED, text_primary: COLOR_TEXT_PRIMARY, text_secondary: COLOR_TEXT_SECONDARY, border: COLOR_BORDER, accent: COLOR_ACCENT, danger: COLOR_DANGER },
    spacing: Spacing { small: SPACE_4, medium: SPACE_8, large: SPACE_16 },
    radii: Radii { small: RADIUS_SM, medium: RADIUS_MD, window: RADIUS_WINDOW },
    window_metrics: WindowMetrics { titlebar_height: TITLEBAR_HEIGHT, button_width: WINDOW_BUTTON_WIDTH, shadow_extent: WINDOW_SHADOW_EXTENT },
};

pub struct BouchaudTheme { pub colors: Colors, pub spacing: Spacing, pub radii: Radii, pub window_metrics: WindowMetrics }
pub struct Colors { pub background:u32,pub surface:u32,pub surface_elevated:u32,pub text_primary:u32,pub text_secondary:u32,pub border:u32,pub accent:u32,pub danger:u32 }
pub struct Spacing { pub small:u32,pub medium:u32,pub large:u32 }
pub struct Radii { pub small:u32,pub medium:u32,pub window:u32 }
pub struct WindowMetrics { pub titlebar_height:u32,pub button_width:u32,pub shadow_extent:u32 }
