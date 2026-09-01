use alloc::collections::VecDeque;
use alloc::sync::Arc;

use crate::kernel::sync::SpinLock;

use super::message::Message;
use crate::kernel::native::abi::types::{Error, Result, Signals};

const MAX_QUEUED_MESSAGES: usize = 64;
const MAX_QUEUED_BYTES: usize = 2 * 1024 * 1024;

struct ChannelInner {
    queues: [VecDeque<Message>; 2],
    queued_bytes: [usize; 2],
    closed: [bool; 2],
}

pub struct ChannelState {
    inner: SpinLock<ChannelInner>,
}

pub struct ChannelEndpoint {
    state: Arc<ChannelState>,
    side: usize,
}

impl ChannelEndpoint {
    pub fn pair() -> (Self, Self) {
        let state = Arc::new(ChannelState {
            inner: SpinLock::new(ChannelInner {
                queues: [VecDeque::new(), VecDeque::new()],
                queued_bytes: [0, 0],
                closed: [false, false],
            }),
        });
        (
            Self { state: Arc::clone(&state), side: 0 },
            Self { state, side: 1 },
        )
    }

    #[inline]
    fn peer(&self) -> usize { 1 - self.side }

    pub fn send(&self, message: Message) -> Result<usize> {
        let bytes = message.bytes.len();
        let target = self.peer();
        let mut state = self.state.inner.lock();

        if state.closed[target] { return Err(Error::PeerClosed); }
        if state.queues[target].len() >= MAX_QUEUED_MESSAGES
            || state.queued_bytes[target].saturating_add(bytes) > MAX_QUEUED_BYTES
        {
            return Err(Error::QueueFull);
        }

        state.queued_bytes[target] += bytes;
        state.queues[target].push_back(message);
        Ok(bytes)
    }

    pub fn front_sizes(&self) -> Result<(usize, usize)> {
        let state = self.state.inner.lock();
        match state.queues[self.side].front() {
            Some(message) => Ok((message.bytes.len(), message.handles.len())),
            None if state.closed[self.peer()] => Err(Error::PeerClosed),
            None => Err(Error::WouldBlock),
        }
    }

    pub fn recv(&self) -> Result<Message> {
        let mut state = self.state.inner.lock();
        match state.queues[self.side].pop_front() {
            Some(message) => {
                state.queued_bytes[self.side] =
                    state.queued_bytes[self.side].saturating_sub(message.bytes.len());
                Ok(message)
            }
            None if state.closed[self.peer()] => Err(Error::PeerClosed),
            None => Err(Error::WouldBlock),
        }
    }

    pub fn requeue_front(&self, message: Message) {
        let mut state = self.state.inner.lock();
        state.queued_bytes[self.side] =
            state.queued_bytes[self.side].saturating_add(message.bytes.len());
        state.queues[self.side].push_front(message);
    }

    pub fn signals(&self) -> Signals {
        let state = self.state.inner.lock();
        let mut signals = Signals::NONE;
        if !state.queues[self.side].is_empty() { signals |= Signals::READABLE; }
        if !state.closed[self.peer()]
            && state.queues[self.peer()].len() < MAX_QUEUED_MESSAGES
            && state.queued_bytes[self.peer()] < MAX_QUEUED_BYTES
        {
            signals |= Signals::WRITABLE;
        }
        if state.closed[self.peer()] { signals |= Signals::PEER_CLOSED; }
        signals
    }
}

impl Drop for ChannelEndpoint {
    fn drop(&mut self) {
        let mut state = self.state.inner.lock();
        state.closed[self.side] = true;
    }
}
