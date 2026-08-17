//! Pilotes materiels de Bouchaud OS.
//!
//! - `vga`      : sortie texte VGA (0xb8000), cible principale de l'affichage ;
//! - `serial`   : UART 16550 sur COM1, sortie de debug pour QEMU (`-serial stdio`) ;
//! - `keyboard` : clavier PS/2 en polling, mapping AZERTY-FR.

pub mod ac97;
pub mod ata;
pub mod block;
pub mod disk;
pub mod display;
pub mod e1000;
pub mod gfx;
pub mod keyboard;
pub mod mouse;
pub mod net;
pub mod serial;
pub mod vga;
