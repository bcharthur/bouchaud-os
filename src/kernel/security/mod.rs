pub mod access;
pub mod audit;
pub mod capability;
pub mod credentials;
pub mod execution;
pub mod filesystem;
pub mod memory;
pub mod network;
pub mod path;
pub mod policy;
pub mod profile;
pub mod sandbox;
pub mod syscall;

pub use capability::Capabilities;
pub use profile::SecurityProfile;

pub fn state() -> &'static str {
    "active (credentials per-processus, capabilities, W^X/NX, sandbox, audit)"
}
