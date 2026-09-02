use super::abi::types::{ObjectKind, Signals};
use super::event::Event;
use super::ipc::ChannelEndpoint;
use super::shm::SharedRegion;
use super::waitset::WaitSet;

pub enum Object {
    Channel(ChannelEndpoint),
    Event(Event),
    WaitSet(WaitSet),
    SharedRegion(SharedRegion),
    Legacy(ObjectKind),
}

impl Object {
    pub fn kind(&self) -> ObjectKind {
        match self {
            Self::Channel(_) => ObjectKind::Channel,
            Self::Event(_) => ObjectKind::Event,
            Self::WaitSet(_) => ObjectKind::WaitSet,
            Self::SharedRegion(_) => ObjectKind::SharedRegion,
            Self::Legacy(kind) => *kind,
        }
    }

    pub fn signals(&self) -> Signals {
        match self {
            Self::Channel(channel) => channel.signals(),
            Self::Event(event) => event.signals(),
            Self::WaitSet(waitset) => waitset.signals(),
            Self::SharedRegion(region) => region.signals(),
            Self::Legacy(_) => Signals::NONE,
        }
    }
}
