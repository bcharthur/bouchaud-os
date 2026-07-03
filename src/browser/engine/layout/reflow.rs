//! Invalidation style/layout/paint.

use alloc::vec::Vec;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DirtyKind { Style, Layout, Paint, Subtree }

#[derive(Clone, Default)]
pub struct ReflowState {
    pub dirty_nodes: Vec<usize>,
    pub needs_style: bool,
    pub needs_layout: bool,
    pub needs_paint: bool,
}

impl ReflowState {
    pub fn clean() -> ReflowState { ReflowState { dirty_nodes: Vec::new(), needs_style: true, needs_layout: true, needs_paint: true } }
    pub fn mark(&mut self, node: usize, kind: DirtyKind) {
        if !self.dirty_nodes.contains(&node) { self.dirty_nodes.push(node); }
        match kind {
            DirtyKind::Style | DirtyKind::Subtree => { self.needs_style = true; self.needs_layout = true; self.needs_paint = true; }
            DirtyKind::Layout => { self.needs_layout = true; self.needs_paint = true; }
            DirtyKind::Paint => { self.needs_paint = true; }
        }
    }
    pub fn viewport_changed(&mut self) { self.needs_layout = true; self.needs_paint = true; }
    pub fn clear(&mut self) { self.dirty_nodes.clear(); self.needs_style = false; self.needs_layout = false; self.needs_paint = false; }
}
