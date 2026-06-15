//! Pilotes materiels de Bouchaud OS.
//!
//! - `vga`      : sortie texte VGA (0xb8000), cible principale de l'affichage ;
//! - `serial`   : UART 16550 sur COM1, sortie de debug pour QEMU (`-serial stdio`) ;
//! - `keyboard` : clavier PS/2 en polling, mapping AZERTY-FR.

pub mod vga;
pub mod serial;
pub mod keyboard;
pub mod gfx;
pub mod mouse;
