//! Protocole minimal entre le gestionnaire de fenetres et un client ring 3.
//!
//! Ce module ne transporte encore rien : il fixe le format binaire et les
//! messages avant d'ajouter le socketpair/memfd dans un prochain jalon. Garder
//! cette frontiere petite evite d'exposer l'API interne du WM a Qt.

/// "BOGU" en ASCII, pour rejeter immediatement un flux qui n'est pas le notre.
pub const MAGIC: u32 = 0x5547_4F42;
pub const PROTOCOL_VERSION: u16 = 1;

#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageKind {
    Hello = 1,
    CreateWindow = 2,
    SetTitle = 3,
    Damage = 4,
    Close = 5,
    Configure = 0x100,
    Focus = 0x101,
    Key = 0x102,
    Pointer = 0x103,
    Wheel = 0x104,
    CloseRequest = 0x105,
}

/// En-tete fixe de chaque message. Le payload suit immediatement.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub magic: u32,
    pub version: u16,
    pub kind: u16,
    pub payload_len: u32,
    pub serial: u32,
}

impl Header {
    pub const fn new(kind: MessageKind, payload_len: u32, serial: u32) -> Self {
        Self {
            magic: MAGIC,
            version: PROTOCOL_VERSION,
            kind: kind as u16,
            payload_len,
            serial,
        }
    }

    pub const fn valid(&self) -> bool {
        self.magic == MAGIC && self.version == PROTOCOL_VERSION
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Configure {
    pub window_id: u32,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
}