//! Cœur générique de Bouchaud OS.
//!
//! Les sources sont désormais classées physiquement par sous-système. Les noms
//! historiques restent temporairement stables grâce à `#[path]` afin que cette
//! migration ne mélange pas refactor structurel et changement de comportement.

#[path = "../compat/linux/mod.rs"]
pub mod abi;
pub mod autorun;
#[path = "memory/page_cache.rs"]
pub mod clean_page_cache;
#[path = "memory/readahead.rs"]
pub mod readahead;
#[path = "memory/frame_cache.rs"]
pub mod frame_cache;
#[path = "memory/pressure.rs"]
pub mod memory_pressure;
#[path = "debug/dmesg.rs"]
pub mod dmesg;
#[path = "process/elf.rs"]
pub mod elf;
#[path = "process/loader/mod.rs"]
pub mod loader;
#[path = "process/exec.rs"]
pub mod exec;
#[path = "object/fd.rs"]
pub mod fd;
#[path = "object/handle.rs"]
pub mod handle;
#[path = "memory/heap.rs"]
pub mod heap;
#[path = "memory/magasin.rs"]
pub mod magasin;
#[path = "memory/arene_dma.rs"]
pub mod arene_dma;
pub mod input;
#[path = "debug/journal.rs"]
pub mod journal;
#[path = "memory/physical.rs"]
pub mod memory;
#[path = "debug/panic.rs"]
pub mod panic;
#[path = "memory/shared.rs"]
pub mod partage;
#[path = "debug/perf.rs"]
pub mod perf;
pub mod power;
#[path = "process/process.rs"]
pub mod process;
#[path = "process/resource.rs"]
pub mod resource;
#[path = "scheduler/core.rs"]
pub mod scheduler;
#[path = "scheduler/echeances.rs"]
pub mod echeances;
#[path = "process/signal.rs"]
pub mod signal;
#[path = "sync/bkl.rs"]
pub mod smp_lock;
pub mod sync;
#[path = "syscall/legacy.rs"]
pub mod syscall;
pub mod sysroot;
#[path = "process/thread.rs"]
pub mod task;
#[path = "time/timer.rs"]
pub mod timer;
#[path = "memory/vma.rs"]
pub mod vma;
#[path = "memory/frames_libres.rs"]
pub mod frames_libres;
#[path = "memory/virtual.rs"]
pub mod vmm;

pub mod security;
pub mod native;
#[path = "object/readiness.rs"]
pub mod readiness;
