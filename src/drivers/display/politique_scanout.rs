//! Qui presente le bureau : la decision, et rien d'autre.
//!
//! BOUCHAUD_VIRTIO_GPU_2D_V1 (P10). Pure -- aucun acces materiel -- pour etre
//! testee sur l'hote (`tools/display/test_politique_scanout.rs`).
//!
//! L'ordre est une PRIORITE, pas une preference de performance :
//!
//!   1. le GOP du firmware (Trigkey, demarrage UEFI) : seul affichage prouve
//!      sur la machine physique ;
//!   2. BGA (QEMU `-vga std`) : l'affichage de TOUS les bancs existants. Tant
//!      que la baseline P1-P19 n'est pas verte, rien ne doit le deplacer ;
//!   3. virtio-gpu 2D : seulement si BGA est ABSENT (`-vga none`) et que
//!      l'autotest 2D du peripherique a reussi. C'est un remplacant de BGA,
//!      pas une acceleration ;
//!   4. aucun : le bureau compose dans son tampon, rien n'atteint l'ecran.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scanout {
    Firmware,
    Bga,
    Virtio2d,
    Aucun,
}

impl Scanout {
    pub fn label(self) -> &'static str {
        match self {
            Scanout::Firmware => "gop-firmware",
            Scanout::Bga => "bga-lfb",
            Scanout::Virtio2d => "virtio-gpu-2d",
            Scanout::Aucun => "aucun",
        }
    }
}

/// `virtio_pret` : controlq montee ET autotest 2D reussi.
pub fn choisit(firmware: bool, bga: bool, virtio_pret: bool) -> Scanout {
    if firmware {
        Scanout::Firmware
    } else if bga {
        Scanout::Bga
    } else if virtio_pret {
        Scanout::Virtio2d
    } else {
        Scanout::Aucun
    }
}
