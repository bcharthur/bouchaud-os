use super::types::{NATIVE_BASE, NATIVE_LAST};

pub const VERSION: u64 = NATIVE_BASE + 0x00;
pub const HANDLE_CLOSE: u64 = NATIVE_BASE + 0x01;
pub const HANDLE_DUP: u64 = NATIVE_BASE + 0x02;
pub const HANDLE_INFO: u64 = NATIVE_BASE + 0x03;
pub const HANDLE_COUNT: u64 = NATIVE_BASE + 0x04;

pub const CHANNEL_CREATE: u64 = NATIVE_BASE + 0x10;
pub const CHANNEL_SEND: u64 = NATIVE_BASE + 0x11;
pub const CHANNEL_RECV: u64 = NATIVE_BASE + 0x12;
/// Envoi avec ATTENUATION des droits transferes.
///
/// `CHANNEL_SEND` reste inchange -- il transfere les droits tels quels --
/// parce qu'un appelant deja compile ne peut pas deviner qu'un sixieme
/// argument est apparu. Le nouveau numero porte un tableau de masques, un par
/// handle : chaque capacite franchit la frontiere avec l'intersection de ses
/// droits et de son masque. C'est ce qui permet a un courtier de donner une
/// vue en LECTURE SEULE d'une region qu'il detient en lecture-ecriture.
pub const CHANNEL_SEND_ATTENUE: u64 = NATIVE_BASE + 0x13;

pub const EVENT_CREATE: u64 = NATIVE_BASE + 0x20;
pub const EVENT_SIGNAL: u64 = NATIVE_BASE + 0x21;
pub const EVENT_RESET: u64 = NATIVE_BASE + 0x22;
pub const EVENT_QUERY: u64 = NATIVE_BASE + 0x23;

pub const WAITSET_CREATE: u64 = NATIVE_BASE + 0x30;
pub const WAITSET_ADD: u64 = NATIVE_BASE + 0x31;
pub const WAITSET_REMOVE: u64 = NATIVE_BASE + 0x32;
pub const WAITSET_POLL: u64 = NATIVE_BASE + 0x33;

pub const SHM_CREATE: u64 = NATIVE_BASE + 0x40;
pub const SHM_SIZE: u64 = NATIVE_BASE + 0x41;
pub const SHM_READ: u64 = NATIVE_BASE + 0x42;
pub const SHM_WRITE: u64 = NATIVE_BASE + 0x43;

#[inline]
pub const fn is_native(number: u64) -> bool {
    number >= NATIVE_BASE && number <= NATIVE_LAST
}

pub const fn name(number: u64) -> &'static str {
    match number {
        VERSION => "bo_version",
        HANDLE_CLOSE => "bo_handle_close",
        HANDLE_DUP => "bo_handle_dup",
        HANDLE_INFO => "bo_handle_info",
        HANDLE_COUNT => "bo_handle_count",
        CHANNEL_CREATE => "bo_channel_create",
        CHANNEL_SEND => "bo_channel_send",
        CHANNEL_RECV => "bo_channel_recv",
        CHANNEL_SEND_ATTENUE => "bo_channel_send_attenue",
        EVENT_CREATE => "bo_event_create",
        EVENT_SIGNAL => "bo_event_signal",
        EVENT_RESET => "bo_event_reset",
        EVENT_QUERY => "bo_event_query",
        WAITSET_CREATE => "bo_waitset_create",
        WAITSET_ADD => "bo_waitset_add",
        WAITSET_REMOVE => "bo_waitset_remove",
        WAITSET_POLL => "bo_waitset_poll",
        SHM_CREATE => "bo_shm_create",
        SHM_SIZE => "bo_shm_size",
        SHM_READ => "bo_shm_read",
        SHM_WRITE => "bo_shm_write",
        _ => "bo_unknown",
    }
}
