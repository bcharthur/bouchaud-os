//! Point d'entree de haut niveau du bureau.

/// Lance le bureau graphique (delegue au gestionnaire de fenetres).
pub fn run() {
    crate::gui::window_manager::run();
}
