#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport { Stream, Datagram }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction { Receive, Send, Both }
