//! Layout tree v1 : arbre de boites consultable par JS.
//!
//! Le layout historique de `web.rs` continue de produire la display list. Cette
//! table parallele donne au moteur JS des mesures coherentes :
//! `getBoundingClientRect`, `offsetWidth`, `clientHeight`, etc. lisent ici et
//! forcent un flush style/layout si le DOM a mute.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use super::super::css::computed::{ComputedStyle, CssLength, Display, Position};
use super::super::css::cascade::StyleMap;

pub type NodeId = usize;
pub type LayoutBoxId = usize;

#[derive(Clone, Copy, Default)]
pub struct Rect { pub x: i32, pub y: i32, pub w: i32, pub h: i32 }

impl Rect {
    pub fn right(self) -> i32 { self.x.saturating_add(self.w) }
    pub fn bottom(self) -> i32 { self.y.saturating_add(self.h) }
}

#[derive(Clone)]
pub struct SnapshotNode {
    pub id: NodeId,
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    pub text: String,
    pub children: Vec<NodeId>,
}

impl SnapshotNode {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

#[derive(Clone, Default)]
pub struct DocumentSnapshot { pub nodes: Vec<SnapshotNode> }

impl DocumentSnapshot {
    pub fn node(&self, id: NodeId) -> Option<&SnapshotNode> { self.nodes.get(id) }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LayoutBoxKind { Root, Block, Inline, Replaced, Absolute, Fixed }

#[derive(Clone)]
pub struct LayoutBox {
    pub node_id: NodeId,
    pub kind: LayoutBoxKind,
    pub rect: Rect,
    pub children: Vec<LayoutBoxId>,
    pub stacking_context: bool,
    pub overflow_clip: Option<Rect>,
}

#[derive(Clone)]
pub struct LayoutResult {
    pub boxes: Vec<LayoutBox>,
    pub node_to_box: BTreeMap<NodeId, LayoutBoxId>,
    pub document_height: i32,
    pub viewport: Rect,
}

impl Default for LayoutResult {
    fn default() -> LayoutResult {
        LayoutResult { boxes: Vec::new(), node_to_box: BTreeMap::new(), document_height: 0, viewport: Rect { x: 0, y: 0, w: 1024, h: 768 } }
    }
}

impl LayoutResult {
    pub fn rect_for_node(&self, node: NodeId) -> Option<Rect> {
        self.node_to_box.get(&node).and_then(|&b| self.boxes.get(b)).map(|b| b.rect)
    }
}

pub fn build_layout_tree(doc: &DocumentSnapshot, styles: &StyleMap, vw: i32, vh: i32) -> LayoutResult {
    let viewport = Rect { x: 0, y: 0, w: vw.max(1), h: vh.max(1) };
    let mut out = LayoutResult { boxes: Vec::new(), node_to_box: BTreeMap::new(), document_height: vh.max(1), viewport };
    if doc.nodes.is_empty() { return out; }
    let root_id = 0usize;
    let root_box = push_box(&mut out, root_id, LayoutBoxKind::Root, viewport);
    let mut y = 0i32;
    for &child in &doc.nodes[root_id].children { y = layout_node(doc, styles, &mut out, child, root_box, 0, y, vw.max(1), 0); }
    if y <= 0 {
        // Documents sans wrapper html/body dans le snapshot : tente les noeuds directs.
        for n in 1..doc.nodes.len() { if !out.node_to_box.contains_key(&n) { y = layout_node(doc, styles, &mut out, n, root_box, 0, y, vw.max(1), 0); } }
    }
    out.document_height = y.max(vh.max(1));
    out
}

fn push_box(out: &mut LayoutResult, node_id: NodeId, kind: LayoutBoxKind, rect: Rect) -> LayoutBoxId {
    let id = out.boxes.len();
    out.boxes.push(LayoutBox { node_id, kind, rect, children: Vec::new(), stacking_context: false, overflow_clip: None });
    out.node_to_box.insert(node_id, id);
    id
}

fn layout_node(doc: &DocumentSnapshot, styles: &StyleMap, out: &mut LayoutResult, id: NodeId, parent_box: LayoutBoxId, x: i32, y: i32, avail_w: i32, depth: u32) -> i32 {
    if depth > 160 { return y; }
    let node = match doc.node(id) { Some(n) => n, None => return y };
    let st = styles.get(id);
    if st.display == Display::None { return y; }
    let margin_l = px(st.margin.left, avail_w, 0).max(0);
    let margin_r = px(st.margin.right, avail_w, 0).max(0);
    let margin_t = px(st.margin.top, avail_w, 0).max(0);
    let margin_b = px(st.margin.bottom, avail_w, 0).max(0);
    let pad_l = px(st.padding.left, avail_w, 0).max(0);
    let pad_r = px(st.padding.right, avail_w, 0).max(0);
    let pad_t = px(st.padding.top, avail_w, 0).max(0);
    let pad_b = px(st.padding.bottom, avail_w, 0).max(0);
    let border_x = st.border_width.left + st.border_width.right;
    let border_y = st.border_width.top + st.border_width.bottom;
    let outer_x = x + margin_l;
    let outer_y = y + margin_t;
    let content_avail = (avail_w - margin_l - margin_r - pad_l - pad_r - border_x).max(1);
    let mut w = st.width.resolve(avail_w, content_avail + pad_l + pad_r + border_x);
    if let CssLength::Auto = st.width {
        if matches!(st.display, Display::Inline | Display::InlineBlock) { w = intrinsic_width(node, st).min(content_avail).max(1) + pad_l + pad_r + border_x; }
    }
    w = constrain_width(w, st, avail_w).max(1);
    let kind = if st.position == Position::Fixed { LayoutBoxKind::Fixed }
        else if st.position == Position::Absolute { LayoutBoxKind::Absolute }
        else if matches!(node.tag.as_str(), "img" | "svg" | "canvas" | "video" | "audio" | "input" | "button" | "select" | "textarea") { LayoutBoxKind::Replaced }
        else if st.display == Display::Inline { LayoutBoxKind::Inline }
        else { LayoutBoxKind::Block };
    let box_id = push_box(out, id, kind, Rect { x: outer_x + st.transform_tx, y: outer_y + st.transform_ty, w, h: 0 });
    if let Some(p) = out.boxes.get_mut(parent_box) { p.children.push(box_id); }

    let child_x = outer_x + pad_l + st.border_width.left;
    let mut child_y = outer_y + pad_t + st.border_width.top;
    let child_w = (w - pad_l - pad_r - border_x).max(1);

    if st.display == Display::Flex && st.flex_direction == 0 {
        layout_flex_row(doc, styles, out, node, box_id, child_x, child_y, child_w, depth + 1);
        child_y += max_child_bottom(out, box_id).saturating_sub(child_y).max(line_h(st));
    } else if st.display == Display::Grid {
        layout_grid(doc, styles, out, node, box_id, child_x, child_y, child_w, depth + 1);
        child_y += max_child_bottom(out, box_id).saturating_sub(child_y).max(line_h(st));
    } else {
        for &c in &node.children { child_y = layout_node(doc, styles, out, c, box_id, child_x, child_y, child_w, depth + 1); }
    }

    let intrinsic_h = if node.children.is_empty() { intrinsic_height(node, st) } else { child_y - outer_y + pad_b + st.border_width.bottom };
    let mut h = st.height.resolve(out.viewport.h, intrinsic_h.max(line_h(st))).max(1);
    if matches!(node.tag.as_str(), "html" | "body") { h = h.max(out.viewport.h); }
    h = h.max(px(st.min_height, out.viewport.h, 0));
    if let CssLength::Px(mx) = st.max_height { if mx > 0 { h = h.min(mx); } }
    if let Some(b) = out.boxes.get_mut(box_id) {
        b.rect.h = h;
        if matches!(st.overflow_x, super::super::css::computed::Overflow::Hidden | super::super::css::computed::Overflow::Clip)
            || matches!(st.overflow_y, super::super::css::computed::Overflow::Hidden | super::super::css::computed::Overflow::Clip) {
            b.overflow_clip = Some(b.rect);
        }
        b.stacking_context = st.z_index.is_some() || st.opacity_pct < 100 || matches!(st.position, Position::Absolute | Position::Fixed | Position::Sticky);
    }
    if matches!(st.position, Position::Absolute | Position::Fixed) { y } else { outer_y + h + margin_b }
}

fn layout_flex_row(doc: &DocumentSnapshot, styles: &StyleMap, out: &mut LayoutResult, node: &SnapshotNode, parent: LayoutBoxId, x: i32, y: i32, w: i32, depth: u32) {
    let children: Vec<NodeId> = node.children.iter().copied().filter(|&c| styles.get(c).display != Display::None).collect();
    if children.is_empty() { return; }
    let gap = styles.get(node.id).gap.max(0);
    let each = ((w - gap * (children.len().saturating_sub(1) as i32)) / children.len() as i32).max(1);
    let mut cx = x;
    for c in children { let _ = layout_node(doc, styles, out, c, parent, cx, y, each, depth); cx += each + gap; }
}

fn layout_grid(doc: &DocumentSnapshot, styles: &StyleMap, out: &mut LayoutResult, node: &SnapshotNode, parent: LayoutBoxId, x: i32, y: i32, w: i32, depth: u32) {
    let gap = styles.get(node.id).gap.max(0);
    let cols = if w >= 900 { 3 } else if w >= 520 { 2 } else { 1 };
    let cell = ((w - gap * (cols - 1)) / cols).max(1);
    let mut i = 0i32;
    for &c in &node.children {
        if styles.get(c).display == Display::None { continue; }
        let cx = x + (i % cols) * (cell + gap);
        let cy = y + (i / cols) * (line_h(styles.get(c)) * 3 + gap);
        let _ = layout_node(doc, styles, out, c, parent, cx, cy, cell, depth);
        i += 1;
    }
}

fn max_child_bottom(out: &LayoutResult, b: LayoutBoxId) -> i32 {
    let mut m = out.boxes.get(b).map(|x| x.rect.y).unwrap_or(0);
    if let Some(parent) = out.boxes.get(b) {
        for &c in &parent.children { if let Some(cb) = out.boxes.get(c) { m = m.max(cb.rect.bottom()); } }
    }
    m
}

fn px(v: CssLength, parent: i32, fallback: i32) -> i32 { v.resolve(parent, fallback) }

fn constrain_width(w: i32, st: &ComputedStyle, parent: i32) -> i32 {
    let mut out = w;
    if let CssLength::Px(min) = st.min_width { out = out.max(min); }
    if let CssLength::Percent(min) = st.min_width { out = out.max(parent.saturating_mul(min) / 100); }
    if let CssLength::Px(max) = st.max_width { if max > 0 { out = out.min(max); } }
    if let CssLength::Percent(max) = st.max_width { if max > 0 { out = out.min(parent.saturating_mul(max) / 100); } }
    out.min(parent.max(1) * 4)
}

fn line_h(st: &ComputedStyle) -> i32 { st.line_height.max(st.font_size + 3).max(12) }

fn intrinsic_width(node: &SnapshotNode, st: &ComputedStyle) -> i32 {
    if let Some(w) = attr_px(node, "width") { return w; }
    match node.tag.as_str() {
        "input" => 220,
        "button" => 92,
        "select" => 160,
        "textarea" => 260,
        "img" | "svg" | "canvas" | "video" => 300,
        _ => (node.text.chars().count() as i32 * (st.font_size / 2).max(6)).clamp(1, 1200),
    }
}

fn intrinsic_height(node: &SnapshotNode, st: &ComputedStyle) -> i32 {
    if let Some(h) = attr_px(node, "height") { return h; }
    match node.tag.as_str() {
        "input" | "button" | "select" => 28,
        "textarea" => 90,
        "img" | "svg" | "canvas" | "video" => 160,
        "br" => line_h(st),
        "hr" => 12,
        _ => {
            let chars = node.text.chars().count() as i32;
            if chars == 0 { line_h(st) } else { ((chars / 80) + 1).max(1) * line_h(st) }
        }
    }
}

fn attr_px(node: &SnapshotNode, name: &str) -> Option<i32> {
    let v = node.attr(name)?.trim().trim_end_matches("px");
    v.parse::<f32>().ok().map(|x| x as i32).filter(|&x| x > 0)
}
