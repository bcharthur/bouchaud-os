use core::ops::{BitAnd, BitOr, BitOrAssign};

pub const ABI_MAJOR: u16 = 1;
pub const ABI_MINOR: u16 = 0;
pub const ABI_VERSION_PACKED: u64 = ((ABI_MAJOR as u64) << 16) | ABI_MINOR as u64;

/// Native syscall namespace. Linux x86-64 syscalls currently used by the
/// compatibility layer are below 512; the "BO" prefix makes accidental
/// collision visible in a register dump.
pub const NATIVE_BASE: u64 = 0x424f_0000;
pub const NATIVE_LAST: u64 = NATIVE_BASE + 0x00ff;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i64)]
pub enum Error {
    InvalidCall = 1,
    InvalidArgument = 2,
    BadHandle = 3,
    WrongType = 4,
    AccessDenied = 5,
    WouldBlock = 6,
    BufferTooSmall = 7,
    QueueFull = 8,
    PeerClosed = 9,
    NoSpace = 10,
    Fault = 11,
    TooLarge = 12,
    NotFound = 13,
}

impl Error {
    #[inline]
    pub const fn neg(self) -> i64 { -(self as i64) }
}

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum ObjectKind {
    Channel = 1,
    Event = 2,
    WaitSet = 3,
    SharedRegion = 4,
    LegacyFile = 0x100,
    LegacyWindow = 0x101,
    LegacySocket = 0x102,
    LegacyDevice = 0x103,
}

/// Opaque process-local identity: 31-bit generation + 32-bit slot.
///
/// Bit 63 stays zero, so a successful handle can always be returned as a
/// positive `i64`; native errors stay negative.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct HandleId(u64);

impl HandleId {
    pub const INVALID: Self = Self(0);

    #[inline]
    pub const fn new(slot: u32, generation: u32) -> Self {
        Self((((generation & 0x7fff_ffff) as u64) << 32) | slot as u64)
    }

    #[inline]
    pub const fn from_raw(raw: u64) -> Self { Self(raw & 0x7fff_ffff_ffff_ffff) }

    #[inline]
    pub const fn raw(self) -> u64 { self.0 }

    #[inline]
    pub const fn slot(self) -> u32 { self.0 as u32 }

    #[inline]
    pub const fn generation(self) -> u32 { ((self.0 >> 32) as u32) & 0x7fff_ffff }

    #[inline]
    pub const fn valid(self) -> bool { self.0 != 0 && self.generation() != 0 }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct Rights(pub u32);

impl Rights {
    pub const NONE: Self = Self(0);
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const SIGNAL: Self = Self(1 << 2);
    pub const MAP: Self = Self(1 << 3);
    pub const DUP: Self = Self(1 << 4);
    pub const TRANSFER: Self = Self(1 << 5);
    pub const INSPECT: Self = Self(1 << 6);
    pub const WAIT: Self = Self(1 << 7);

    /// Tous les droits definis.
    ///
    /// Sert de masque « ne rien attenuer » : c'est la valeur qui reproduit
    /// exactement le comportement d'avant l'attenuation de transfert, et donc
    /// celle qu'un appelant non migre doit pouvoir passer sans y penser.
    pub const TOUS: Self = Self(
        Self::READ.0 | Self::WRITE.0 | Self::SIGNAL.0 | Self::MAP.0 |
        Self::DUP.0 | Self::TRANSFER.0 | Self::INSPECT.0 | Self::WAIT.0
    );

    pub const CHANNEL_DEFAULT: Self = Self(
        Self::READ.0 | Self::WRITE.0 | Self::DUP.0 | Self::TRANSFER.0 |
        Self::INSPECT.0 | Self::WAIT.0
    );
    pub const EVENT_DEFAULT: Self = Self(
        Self::SIGNAL.0 | Self::DUP.0 | Self::TRANSFER.0 |
        Self::INSPECT.0 | Self::WAIT.0
    );
    pub const WAITSET_DEFAULT: Self = Self(
        Self::READ.0 | Self::WRITE.0 | Self::DUP.0 |
        Self::TRANSFER.0 | Self::INSPECT.0 | Self::WAIT.0
    );
    pub const SHM_DEFAULT: Self = Self(
        Self::READ.0 | Self::WRITE.0 | Self::MAP.0 | Self::DUP.0 |
        Self::TRANSFER.0 | Self::INSPECT.0
    );

    #[inline]
    pub const fn contains(self, wanted: Self) -> bool {
        (self.0 & wanted.0) == wanted.0
    }

    #[inline]
    pub const fn intersection(self, other: Self) -> Self { Self(self.0 & other.0) }

    #[inline]
    pub const fn without(self, removed: Self) -> Self { Self(self.0 & !removed.0) }

    #[inline]
    pub const fn subset_of(self, parent: Self) -> bool { parent.contains(self) }
}

impl BitOr for Rights {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self::Output { Self(self.0 | rhs.0) }
}
impl BitOrAssign for Rights {
    fn bitor_assign(&mut self, rhs: Self) { self.0 |= rhs.0; }
}
impl BitAnd for Rights {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self::Output { Self(self.0 & rhs.0) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct Signals(pub u32);

impl Signals {
    pub const NONE: Self = Self(0);
    pub const READABLE: Self = Self(1 << 0);
    pub const WRITABLE: Self = Self(1 << 1);
    pub const SIGNALED: Self = Self(1 << 2);
    pub const PEER_CLOSED: Self = Self(1 << 3);

    #[inline]
    pub const fn is_empty(self) -> bool { self.0 == 0 }
}

impl BitOr for Signals {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self::Output { Self(self.0 | rhs.0) }
}
impl BitOrAssign for Signals {
    fn bitor_assign(&mut self, rhs: Self) { self.0 |= rhs.0; }
}
