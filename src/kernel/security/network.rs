use super::capability::Capabilities;
use super::policy::Snapshot;

const AF_PACKET: u32 = 17;
const SOCK_TYPE_MASK: u32 = 0xf;
const SOCK_RAW: u32 = 3;

pub fn socket_allowed(security: Snapshot, domain: u32, socket_type: u32) -> bool {
    let raw = socket_type & SOCK_TYPE_MASK == SOCK_RAW || domain == AF_PACKET;
    !raw || security.capabilities.contains(Capabilities::NETWORK_ADMIN)
}
