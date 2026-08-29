//! Bouchaud OS performance observatory.
//!
//! API publique stable pour les jalons de démarrage existants, plus un
//! enregistreur de vol sans allocation/verrou et une corrélation GUI
//! input -> frame. Les fragments sont `include!` dans le même module afin
//! d'éviter une migration de visibilité pendant le diagnostic P0/P1.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

include!("perf/types.rs");
include!("perf/flight_recorder.rs");
include!("perf/browser.rs");
include!("perf/watchdog.rs");
include!("perf/report.rs");
