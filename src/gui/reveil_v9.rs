// V9 wrapper around the existing `gui/reveil.rs`.
//
// The legacy file remains the source of truth for counters and wake semantics.
// This wrapper only injects BKL safe points at GUI boundaries.

#[path = "reveil.rs"]
mod legacy;

pub use legacy::{
    chaine,
    entrees,
    invalide,
    note_entree,
    note_recomposition_aveugle,
    note_sommeil_sans_fin,
    tours,
    trames_composees,
};

#[inline]
pub fn note_tour() {
    crate::kernel::task::stall_site_set(700, 0);
    // Boundary BEFORE reading the next iteration state.
    crate::gui::desktop_bkl::checkpoint(crate::gui::desktop_bkl::Site::Tour);
    legacy::note_tour();
}

#[inline]
pub fn note_trame(horloge_seule: bool) {
    crate::kernel::task::stall_site_set(740, horloge_seule as u64);
    legacy::note_trame(horloge_seule);
    // V15: un seul compteur FPS logique par trame UTILE composee. Les appels
    // `present_rect()` peuvent etre multiples pour une meme trame et ne sont
    // donc volontairement PAS utilises pour calculer les FPS.
    crate::gui::frame_clock::note_frame(horloge_seule);
    // A composed frame is a natural atomicity boundary for the desktop.
    crate::gui::desktop_bkl::checkpoint(crate::gui::desktop_bkl::Site::Trame);
}

#[inline]
pub fn note_trame_differee() {
    crate::kernel::task::stall_site_set(745, 0);
    legacy::note_trame_differee();
    // A busy loop that repeatedly misses its frame slot must also hand off.
    crate::gui::desktop_bkl::checkpoint(
        crate::gui::desktop_bkl::Site::TrameDifferee,
    );
}

#[inline]
pub fn note_culling(offerts: usize, occultes: usize, dessines: usize) {
    crate::kernel::task::stall_site_set(730, dessines as u64);
    // Culling itself only updates atomics. Do not release mid-scene here:
    // depth=1 is not sufficient proof that an outer GUI operation has ended.
    legacy::note_culling(offerts, occultes, dessines);
}

pub fn publie() {
    crate::kernel::task::stall_site_set(760, 0);
    // The report reads atomics / framebuffer counters and writes serial output.
    // It does not need the global kernel lock, so don't charge it to desktop.
    crate::gui::desktop_bkl::sans_bkl(
        crate::gui::desktop_bkl::Site::Rapport,
        || {
            legacy::publie();
            crate::gui::frame_clock::publie();
        },
    );
}
