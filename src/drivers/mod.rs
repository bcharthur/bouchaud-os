//! Couche pilotes de Bouchaud OS.
//!
//! Les fichiers sont désormais classés physiquement par classe de périphérique.
//! Pendant la migration multiplateforme, les anciens noms publics
//! (`drivers::ata`, `drivers::e1000`, etc.) sont conservés afin de dissocier le
//! déplacement des fichiers des changements de comportement.

#[path = "audio/ac97.rs"]
pub mod ac97;
#[path = "block/ata.rs"]
pub mod ata;
#[path = "block/ata_bloc.rs"]
pub mod ata_bloc;
#[path = "api/block.rs"]
pub mod block;
#[path = "api/bloc.rs"]
pub mod bloc;
#[path = "block/disk.rs"]
pub mod disk;
#[path = "api/display.rs"]
pub mod display;
#[path = "network/e1000.rs"]
pub mod e1000;
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
    #[path = "uart16550.rs"]
    mod uart16550;
    pub use uart16550::*;
}
#[path = "display/vga_text.rs"]
pub mod vga;
