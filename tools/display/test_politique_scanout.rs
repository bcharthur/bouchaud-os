//! Le choix du scanout du bureau (BOUCHAUD_VIRTIO_GPU_2D_V1).
//!
//! Lance par `tools/ci/run_host_tests.sh`.

#[path = "../../src/drivers/display/politique_scanout.rs"]
mod politique_scanout;

use politique_scanout::{choisit, Scanout};

#[test]
fn bga_reste_prioritaire_quand_virtio_est_aussi_la() {
    // La baseline : tous les bancs tournent en `-vga std`. Ajouter un
    // virtio-gpu ne doit rien deplacer.
    assert_eq!(choisit(false, true, true), Scanout::Bga);
    assert_eq!(choisit(false, true, false), Scanout::Bga);
}

#[test]
fn virtio_ne_prend_l_ecran_que_sans_bga() {
    assert_eq!(choisit(false, false, true), Scanout::Virtio2d);
}

#[test]
fn virtio_en_echec_ne_presente_rien_plutot_que_mal() {
    // Autotest 2D en echec : pas de scanout virtio, meme sans BGA.
    assert_eq!(choisit(false, false, false), Scanout::Aucun);
}

#[test]
fn le_firmware_passe_avant_tout() {
    for bga in [false, true] {
        for virtio in [false, true] {
            assert_eq!(choisit(true, bga, virtio), Scanout::Firmware);
        }
    }
}

#[test]
fn les_libelles_sont_distincts() {
    let tous = [Scanout::Firmware, Scanout::Bga, Scanout::Virtio2d, Scanout::Aucun];
    for (i, a) in tous.iter().enumerate() {
        for b in &tous[i + 1..] {
            assert_ne!(a.label(), b.label());
        }
    }
}
