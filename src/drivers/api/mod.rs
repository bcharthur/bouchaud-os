//! Contrats stables par classe de périphérique.
//!
//! Les implémentations existantes restent accessibles par les anciens modules
//! pendant la migration. Les consommateurs seront ensuite déplacés vers des
//! interfaces `BlockDevice`, `NetworkDevice`, `DisplayDevice`, `InputDevice`,
//! `AudioDevice` et `SerialDevice` indépendantes du matériel.
