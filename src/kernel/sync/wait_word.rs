// Bouchaud OS native wait-on-word primitive.
//
// This is not a Linux futex implementation. It is the native kernel operation:
// wait while a 32-bit user word equals a value, keyed by physical identity.
// Compatibility ABIs translate their own futex contracts onto this primitive.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use super::{SpinLock, WaitSource, WaitSourceWake};

include!("wait_word/types.rs");
include!("wait_word/etat.rs");
include!("wait_word/cle.rs");
include!("wait_word/table.rs");
include!("wait_word/attente.rs");
include!("wait_word/reveil.rs");
include!("wait_word/nettoyage.rs");
include!("wait_word/diagnostic.rs");
