pub mod channel;
pub mod message;

pub use channel::ChannelEndpoint;
pub use message::{Message, TransferredHandle, MAX_MESSAGE_BYTES, MAX_MESSAGE_HANDLES};
