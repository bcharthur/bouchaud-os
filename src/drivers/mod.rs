//! Couche pilotes de Bouchaud OS.
//!
//! Les fichiers sont désormais classés physiquement par classe de périphérique.
//! Pendant la migration multiplateforme, les anciens noms publics
//! (`drivers::ata`, `drivers::e1000`, etc.) sont conservés afin de dissocier le
//! déplacement des fichiers des changements de comportement.

#[path = "audio/ac97.rs"]
pub mod ac97;
#[path = "audio/ac97_position.rs"]
pub mod ac97_position;
#[path = "block/ata.rs"]
pub mod ata;
#[path = "block/ata_bloc.rs"]
pub mod ata_bloc;
/// BOUCHAUD_ATA_INTEGRITE_V1 : ecrire, vider, relire, comparer (commande `ata-integrite`).
#[path = "block/ata_integrite.rs"]
pub mod ata_integrite;
#[path = "api/block.rs"]
pub mod block;
#[path = "api/bloc.rs"]
pub mod bloc;
#[path = "block/disk.rs"]
pub mod disk;
#[path = "block/nvme.rs"]
pub mod nvme;
#[path = "api/display.rs"]
pub mod display;
/// BOUCHAUD_VIRTIO_PCI_V1 : transport virtio-PCI moderne et premier aller-retour virtio-gpu (P10).
#[path = "display/virtio_gpu.rs"]
pub mod virtio_gpu;
/// BOUCHAUD_VIRTIO_GPU_2D_V1 : qui presente le bureau (GOP > BGA > virtio-gpu 2D), pur et teste sur l'hote.
#[path = "display/virtio_gpu_dma.rs"]
pub mod virtio_gpu_dma;
#[path = "display/virtio_gpu_file.rs"]
pub mod virtio_gpu_file;
/// BOUCHAUD_VIRTIO_GPU_IRQ_V1 (P10+.1) : achevement par interruption MSI-X, pur et teste sur l'hote.
#[path = "display/virtio_gpu_irq.rs"]
pub mod virtio_gpu_irq;
/// BOUCHAUD_VIRTIO_GPU_TAMPONS_V1 (P10+.2) : possession de la memoire du scanout,
/// copies par trame (pur, teste sur l'hote).
#[path = "display/virtio_gpu_tampons.rs"]
pub mod virtio_gpu_tampons;
/// BOUCHAUD_VIRTIO_GPU_TRAME_V1 (P10+.3a) : une trame, une soumission (pur, teste sur l'hote).
#[path = "display/virtio_gpu_trame.rs"]
pub mod virtio_gpu_trame;
#[path = "display/politique_scanout.rs"]
pub mod politique_scanout;
/// Discipline de l'anneau de reception RTL8168, PURE : voir
/// `network/anneau_rx.rs`. Aucun acces materiel, donc verifiable par un test
/// hote sans demarrer la machine.
#[path = "network/anneau_rx.rs"]
pub mod anneau_rx;

/// La reconnaissance d'une navigation refusee par le navigateur, pure.
/// Voir `network/analyse_refus.rs`.
#[path = "network/analyse_refus.rs"]
pub mod analyse_refus;

/// Presence materielle, attachement, service et lien : quatre faits
/// distincts. Voir `network/etat_pilote.rs`.
#[path = "network/etat_pilote.rs"]
pub mod etat_pilote;
#[path = "network/e1000.rs"]
pub mod e1000;
#[path = "network/rtl8168.rs"]
pub mod rtl8168;
#[path = "usb/xhci_probe.rs"]
pub mod xhci_probe;
/// Equite d'acces au pilote USB, pure : voir `usb/equite_pilote.rs`.
#[path = "usb/equite_pilote.rs"]
pub mod equite_pilote;
/// Qui tient le pilote xHCI, pur : voir `usb/proprietaire_runtime.rs`.
#[path = "usb/proprietaire_runtime.rs"]
pub mod proprietaire_runtime;
/// Etat du transport Bulk-Only, pur : voir `usb/reprise_bot.rs`.
#[path = "usb/reprise_bot.rs"]
pub mod reprise_bot;
/// Ou passe le temps entre deux tours de scrutation HID, pur : voir
/// `usb/chrono_hid.rs`.
#[path = "usb/chrono_hid.rs"]
pub mod chrono_hid;
#[path = "usb/xhci_active.rs"]
pub mod xhci_active;
#[path = "display/bochs.rs"]
pub mod gfx;
#[path = "api/gpu.rs"]
pub mod gpu;
#[path = "input/ps2_keyboard.rs"]
pub mod keyboard;
#[path = "input/ps2_mouse.rs"]
pub mod mouse;
#[path = "api/network.rs"]
pub mod net;
pub mod serial {
    #[path = "lots.rs"]
    pub mod lots;
    #[path = "attente.rs"]
    pub mod attente;
    #[path = "differees.rs"]
    pub mod differees;
    #[path = "uart16550.rs"]
    mod uart16550;
    pub use uart16550::*;
}
#[path = "display/vga_text.rs"]
pub mod vga;
