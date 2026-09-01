use super::types::{ObjectKind, Rights, Signals};

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct RecvMeta {
    pub bytes: u64,
    pub handles: u64,
}

impl RecvMeta {
    pub const BYTE_LEN: usize = 16;
    pub fn bytes_le(self) -> [u8; Self::BYTE_LEN] {
        let mut out = [0u8; Self::BYTE_LEN];
        out[..8].copy_from_slice(&self.bytes.to_le_bytes());
        out[8..].copy_from_slice(&self.handles.to_le_bytes());
        out
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct HandleInfo {
    pub kind: u32,
    pub rights: u32,
    pub signals: u32,
    pub reserved: u32,
}

impl HandleInfo {
    pub const BYTE_LEN: usize = 16;
    pub fn new(kind: ObjectKind, rights: Rights, signals: Signals) -> Self {
        Self { kind: kind as u32, rights: rights.0, signals: signals.0, reserved: 0 }
    }
    pub fn bytes_le(self) -> [u8; Self::BYTE_LEN] {
        let mut out = [0u8; Self::BYTE_LEN];
        out[0..4].copy_from_slice(&self.kind.to_le_bytes());
        out[4..8].copy_from_slice(&self.rights.to_le_bytes());
        out[8..12].copy_from_slice(&self.signals.to_le_bytes());
        out[12..16].copy_from_slice(&self.reserved.to_le_bytes());
        out
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct WaitEvent {
    pub key: u64,
    pub signals: u32,
    pub reserved: u32,
}

impl WaitEvent {
    pub const BYTE_LEN: usize = 16;
    pub fn bytes_le(self) -> [u8; Self::BYTE_LEN] {
        let mut out = [0u8; Self::BYTE_LEN];
        out[0..8].copy_from_slice(&self.key.to_le_bytes());
        out[8..12].copy_from_slice(&self.signals.to_le_bytes());
        out[12..16].copy_from_slice(&self.reserved.to_le_bytes());
        out
    }
}
