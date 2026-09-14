//! Bounded FIFO of physical press/release transitions. Never overwrites a release.
use alloc::collections::VecDeque;
pub struct KeyQueue { pending: VecDeque<[u8; 20]> }
impl KeyQueue {
    pub fn new() -> Self { Self { pending: VecDeque::new() } }
    pub fn push(&mut self, key: [u8; 20]) -> bool {
        if self.pending.len() == 256 { return false; }
        self.pending.push_back(key); true
    }
    pub fn front(&self) -> Option<[u8; 20]> { self.pending.front().copied() }
    pub fn delivered(&mut self) { self.pending.pop_front(); }
}
