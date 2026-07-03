//! Event loop structurel.
//!
//! Le moteur JS garde encore ses Vec de callbacks historiques pour compatibilite,
//! mais cette structure documente et porte le modele navigateur : tasks,
//! microtasks, animation frame, timers et evenements DOM.

use alloc::collections::VecDeque;
use alloc::vec::Vec;

#[derive(Clone)]
pub enum TaskKind { Script, Timer, DomEvent, Networking, AnimationFrame }

#[derive(Clone)]
pub struct Task { pub kind: TaskKind, pub callback_id: usize }

#[derive(Clone, Default)]
pub struct EventLoop {
    pub task_queue: VecDeque<Task>,
    pub microtask_queue: VecDeque<Task>,
    pub animation_callbacks: Vec<Task>,
    pub timers: Vec<Task>,
}

impl EventLoop {
    pub fn new() -> EventLoop { EventLoop::default() }
    pub fn push_task(&mut self, t: Task) { self.task_queue.push_back(t); }
    pub fn push_microtask(&mut self, t: Task) { self.microtask_queue.push_back(t); }
    pub fn next_runnable(&mut self) -> Option<Task> {
        self.microtask_queue.pop_front().or_else(|| self.task_queue.pop_front())
    }
    pub fn is_idle(&self) -> bool { self.microtask_queue.is_empty() && self.task_queue.is_empty() && self.animation_callbacks.is_empty() && self.timers.is_empty() }
    pub fn drain_microtasks_first(&mut self) -> Option<Task> { self.microtask_queue.pop_front().or_else(|| self.task_queue.pop_front()) }
    pub fn queue_animation_frame(&mut self, callback_id: usize) { self.animation_callbacks.push(Task { kind: TaskKind::AnimationFrame, callback_id }); }
    pub fn promote_animation_frame(&mut self) { for t in self.animation_callbacks.drain(..) { self.task_queue.push_back(t); } }
}
