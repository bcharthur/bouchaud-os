#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct Capabilities(pub u64);

impl Capabilities {
    pub const NONE: Self = Self(0);
    pub const EXEC: Self = Self(1 << 0);
    pub const EXEC_UNTRUSTED: Self = Self(1 << 1);
    pub const JIT: Self = Self(1 << 2);
    pub const DEVICE_IO: Self = Self(1 << 3);
    pub const PROCESS_CONTROL: Self = Self(1 << 4);
    pub const SET_IDENTITY: Self = Self(1 << 5);
    pub const FS_ADMIN: Self = Self(1 << 6);
    pub const NETWORK_ADMIN: Self = Self(1 << 7);
    pub const IPC_TRANSFER: Self = Self(1 << 8);
    pub const DEBUG: Self = Self(1 << 9);
    pub const SYSTEM_ADMIN: Self = Self(1 << 10);

    pub const ALL: Self = Self(
        Self::EXEC.0
            | Self::EXEC_UNTRUSTED.0
            | Self::JIT.0
            | Self::DEVICE_IO.0
            | Self::PROCESS_CONTROL.0
            | Self::SET_IDENTITY.0
            | Self::FS_ADMIN.0
            | Self::NETWORK_ADMIN.0
            | Self::IPC_TRANSFER.0
            | Self::DEBUG.0
            | Self::SYSTEM_ADMIN.0,
    );

    #[inline]
    pub const fn contains(self, wanted: Self) -> bool {
        (self.0 & wanted.0) == wanted.0
    }

    #[inline]
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    #[inline]
    pub const fn without(self, removed: Self) -> Self {
        Self(self.0 & !removed.0)
    }
}
