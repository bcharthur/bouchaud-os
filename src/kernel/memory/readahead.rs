// Adaptive read-ahead for immutable file-backed clean pages.
//
// V14 couples cache prefetch with clustered PTE publication: the backing layer
// reads in large chunks, this layer keeps the next clean pages hot, and the
// fault handler can publish a small verified cluster into the process.

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use crate::kernel::clean_page_cache::Key;

include!("readahead/etat.rs");
include!("readahead/politique.rs");
include!("readahead/observe.rs");
include!("readahead/prefetch.rs");
include!("readahead/diagnostic.rs");
