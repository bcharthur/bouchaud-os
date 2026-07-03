//! Cascade/Computed style v1.
//!
//! Cette v1 applique les styles UA + attributs HTML + style inline. Elle cree
//! deja une table `NodeId -> ComputedStyle`; le branchement des feuilles CSS
//! externes dans ce resolver devient ensuite une extension naturelle.

use alloc::vec::Vec;
use alloc::string::{String, ToString};

use super::computed::{BoxSizing, ComputedStyle, CssLength, Display, Edges, Overflow, Position};
use super::color;
use super::units::{LenCtx, to_px};
use super::super::css_parser::{parse_decls, parse_stylesheet};
use super::super::style::{AttrOp, AttrSel, Comb, Pseudo, Rule, Sel};
use super::super::css_values::parse_transform;
use super::super::layout::tree::{DocumentSnapshot, NodeId, SnapshotNode};

#[derive(Clone, Default)]
pub struct StyleMap { pub styles: Vec<ComputedStyle> }

impl StyleMap {
    pub fn get(&self, id: NodeId) -> &ComputedStyle {
        self.styles.get(id).unwrap_or_else(|| self.styles.get(0).expect("StyleMap empty"))
    }
}

pub fn resolve_snapshot_styles(doc: &DocumentSnapshot, vw: i32, vh: i32) -> StyleMap {
    let mut styles: Vec<ComputedStyle> = Vec::new();
    styles.resize_with(doc.nodes.len().max(1), ComputedStyle::initial);
    let mut css_rules = Vec::new();
    for n in &doc.nodes {
        if n.tag == "style" && !n.text.trim().is_empty() { parse_stylesheet(&n.text, &mut css_rules); }
    }
    let mut visited = Vec::new();
    resolve_node(doc, 0, None, &css_rules, &mut styles, &mut visited, vw, vh);
    for i in 0..doc.nodes.len() { if !visited.contains(&i) { resolve_node(doc, i, None, &css_rules, &mut styles, &mut visited, vw, vh); } }
    StyleMap { styles }
}

fn resolve_node(doc: &DocumentSnapshot, id: NodeId, parent: Option<NodeId>, css_rules: &[Rule], styles: &mut [ComputedStyle], visited: &mut Vec<NodeId>, vw: i32, vh: i32) {
    if id >= doc.nodes.len() || visited.contains(&id) { return; }
    visited.push(id);
    let node = &doc.nodes[id];
    let parent_style = parent.and_then(|p| styles.get(p)).cloned();
    let mut st = ComputedStyle::inherit(parent_style.as_ref(), &node.tag);
    let path = path_to(doc, id);
    let mut matched: Vec<(u32, usize)> = Vec::new();
    for (ri, r) in css_rules.iter().enumerate() {
        if rule_matches(r, doc, &path) { matched.push((r.spec, ri)); }
    }
    matched.sort_by_key(|&(spec, ri)| (spec, ri));
    for (_, ri) in matched { apply_decls_to_computed(&css_rules[ri].decls, &mut st, vw, vh); }
    apply_html_presentational(node, &mut st, vw, vh);
    if let Some(style) = node.attr("style") { apply_decls_to_computed(&parse_decls(style), &mut st, vw, vh); }
    if node.attr("hidden").is_some() { st.display = Display::None; }
    styles[id] = st;
    for &c in &node.children { resolve_node(doc, c, Some(id), css_rules, styles, visited, vw, vh); }
}

fn path_to(doc: &DocumentSnapshot, id: NodeId) -> Vec<NodeId> {
    let mut rev = Vec::new();
    let mut cur = Some(id);
    while let Some(n) = cur {
        rev.push(n);
        cur = parent_of(doc, n);
        if rev.len() > doc.nodes.len().max(1) { break; }
    }
    rev.reverse();
    rev
}

fn parent_of(doc: &DocumentSnapshot, id: NodeId) -> Option<NodeId> {
    for n in &doc.nodes { if n.children.contains(&id) { return Some(n.id); } }
    None
}

fn elem_pos(doc: &DocumentSnapshot, id: NodeId) -> (usize, usize) {
    if let Some(p) = parent_of(doc, id).and_then(|p| doc.node(p)) {
        let elems: Vec<NodeId> = p.children.iter().copied().filter(|&c| doc.node(c).map(|n| n.tag != "#text").unwrap_or(false)).collect();
        let pos = elems.iter().position(|&x| x == id).map(|i| i + 1).unwrap_or(1);
        return (pos, elems.len().max(1));
    }
    (1, 1)
}

fn rule_matches(rule: &Rule, doc: &DocumentSnapshot, path: &[NodeId]) -> bool {
    if rule.chain.is_empty() || path.is_empty() { return false; }
    let mut pos = path.len() - 1;
    let last = rule.chain.last().unwrap();
    if !compound_matches(last, doc, path[pos]) { return false; }
    for sel in rule.chain.iter().rev().skip(1) {
        match sel.comb {
            Comb::Child => {
                if pos == 0 { return false; }
                pos -= 1;
                if !compound_matches(sel, doc, path[pos]) { return false; }
            }
            _ => {
                let mut found = None;
                let mut i = pos;
                while i > 0 {
                    i -= 1;
                    if compound_matches(sel, doc, path[i]) { found = Some(i); break; }
                }
                match found { Some(i) => pos = i, None => return false }
            }
        }
    }
    true
}

fn compound_matches(sel: &Sel, doc: &DocumentSnapshot, id: NodeId) -> bool {
    let node = match doc.node(id) { Some(n) => n, None => return false };
    if let Some(tag) = &sel.tag { if tag != "*" && node.tag != *tag { return false; } }
    if let Some(id_sel) = &sel.id { if node.attr("id") != Some(id_sel.as_str()) { return false; } }
    if !sel.classes.is_empty() {
        let cls = node.attr("class").unwrap_or("");
        for c in &sel.classes { if !cls.split_whitespace().any(|x| x == c) { return false; } }
    }
    for a in &sel.attrs { if !attr_matches(node, a) { return false; } }
    for p in &sel.pseudos { if !pseudo_matches(p, doc, id) { return false; } }
    true
}

fn attr_matches(node: &SnapshotNode, a: &AttrSel) -> bool {
    let v = match node.attr(&a.name) { Some(v) => v, None => return false };
    match a.op {
        AttrOp::Exists => true,
        AttrOp::Eq => v == a.val.as_str(),
        AttrOp::Prefix => !a.val.is_empty() && v.starts_with(a.val.as_str()),
        AttrOp::Suffix => !a.val.is_empty() && v.ends_with(a.val.as_str()),
        AttrOp::Substr => !a.val.is_empty() && v.contains(a.val.as_str()),
        AttrOp::Word => v.split_whitespace().any(|w| w == a.val.as_str()),
        AttrOp::Dash => v == a.val.as_str() || (v.len() > a.val.len() && v.starts_with(a.val.as_str()) && v.as_bytes()[a.val.len()] == b'-'),
    }
}

fn pseudo_matches(p: &Pseudo, doc: &DocumentSnapshot, id: NodeId) -> bool {
    match p {
        Pseudo::Root => doc.node(id).map(|n| n.tag == "html" || n.tag == "#root").unwrap_or(false),
        Pseudo::FirstChild => elem_pos(doc, id).0 == 1,
        Pseudo::LastChild => { let (pos, cnt) = elem_pos(doc, id); pos == cnt },
        Pseudo::OnlyChild => elem_pos(doc, id).1 == 1,
        Pseudo::Empty => doc.node(id).map(|n| n.children.is_empty() && n.text.trim().is_empty()).unwrap_or(false),
        Pseudo::NthChild(_, _) | Pseudo::NthLastChild(_, _) => true,
        Pseudo::Not(inner) => !inner.iter().any(|s| compound_matches(s, doc, id)),
    }
}

fn apply_html_presentational(node: &SnapshotNode, st: &mut ComputedStyle, vw: i32, vh: i32) {
    let ctx = LenCtx { font_px: st.font_size as f32, root_px: 16.0, percent_of: vw as f32, vw: vw as f32 / 100.0, vh: vh as f32 / 100.0 };
    if let Some(w) = node.attr("width").and_then(|v| to_px(v, &ctx)) { st.width = CssLength::Px(w as i32); }
    if let Some(h) = node.attr("height").and_then(|v| to_px(v, &ctx)) { st.height = CssLength::Px(h as i32); }
    if matches!(node.tag.as_str(), "input" | "button" | "select") {
        if matches!(st.width, CssLength::Auto) { st.width = CssLength::Px(if node.tag == "input" { 220 } else { 92 }); }
        if matches!(st.height, CssLength::Auto) { st.height = CssLength::Px(28); }
    }
    if node.tag == "textarea" {
        if matches!(st.width, CssLength::Auto) { st.width = CssLength::Px(260); }
        if matches!(st.height, CssLength::Auto) { st.height = CssLength::Px(90); }
    }
}

pub fn apply_decls_to_computed(decls: &[(String, String)], st: &mut ComputedStyle, vw: i32, vh: i32) {
    let ctx = LenCtx { font_px: st.font_size as f32, root_px: 16.0, percent_of: vw as f32, vw: vw as f32 / 100.0, vh: vh as f32 / 100.0 };
    for (prop, raw) in decls {
        let val = raw.trim();
        match prop.as_str() {
            "display" => st.display = match val {
                "none" => Display::None,
                "block" | "list-item" | "table" => Display::Block,
                "inline-block" => Display::InlineBlock,
                "flex" | "inline-flex" => Display::Flex,
                "grid" | "inline-grid" => Display::Grid,
                "inline" => Display::Inline,
                _ => st.display,
            },
            "position" => st.position = match val {
                "relative" => Position::Relative,
                "absolute" => Position::Absolute,
                "fixed" => Position::Fixed,
                "sticky" => Position::Sticky,
                _ => Position::Static,
            },
            "box-sizing" => st.box_sizing = if val == "border-box" { BoxSizing::BorderBox } else { BoxSizing::ContentBox },
            "width" => st.width = parse_length(val, &ctx, vw),
            "height" => st.height = parse_length(val, &ctx, vh),
            "min-width" => st.min_width = parse_length(val, &ctx, vw),
            "max-width" => st.max_width = parse_length(val, &ctx, vw),
            "min-height" => st.min_height = parse_length(val, &ctx, vh),
            "max-height" => st.max_height = parse_length(val, &ctx, vh),
            "margin" => st.margin = parse_edges(val, &ctx, vw),
            "margin-top" => st.margin.top = parse_length(val, &ctx, vw),
            "margin-right" => st.margin.right = parse_length(val, &ctx, vw),
            "margin-bottom" => st.margin.bottom = parse_length(val, &ctx, vw),
            "margin-left" => st.margin.left = parse_length(val, &ctx, vw),
            "padding" => st.padding = parse_edges(val, &ctx, vw),
            "padding-top" => st.padding.top = parse_length(val, &ctx, vw),
            "padding-right" => st.padding.right = parse_length(val, &ctx, vw),
            "padding-bottom" => st.padding.bottom = parse_length(val, &ctx, vw),
            "padding-left" => st.padding.left = parse_length(val, &ctx, vw),
            "border-width" => { let e = parse_edges_px(val, &ctx, vw); st.border_width = e; }
            "border" | "border-top" | "border-right" | "border-bottom" | "border-left" => {
                let mut w = 0;
                for tok in val.split_whitespace() { if let Some(px) = to_px(tok, &ctx) { w = px as i32; break; } }
                if w == 0 && val.contains("solid") { w = 1; }
                if prop == "border" { st.border_width = Edges::all(w.max(0)); }
                else if prop == "border-top" { st.border_width.top = w.max(0); }
                else if prop == "border-right" { st.border_width.right = w.max(0); }
                else if prop == "border-bottom" { st.border_width.bottom = w.max(0); }
                else { st.border_width.left = w.max(0); }
            }
            "color" => if let Some(c) = color::parse(val) { st.color = c; },
            "background" | "background-color" => if let Some(c) = color::parse(val.split_whitespace().next().unwrap_or(val)) { st.background_color = Some(c); },
            "font-size" => if let Some(px) = to_px(val, &ctx) { st.font_size = (px as i32).clamp(8, 96); st.line_height = (st.font_size * 6 / 5).max(st.font_size + 2); },
            "font-weight" => st.font_weight = if val == "bold" { 700 } else { val.parse::<u16>().unwrap_or(st.font_weight) },
            "line-height" => if val == "normal" { st.line_height = (st.font_size * 6 / 5).max(st.font_size + 2); } else if let Some(px) = to_px(val, &ctx) { st.line_height = (px as i32).max(1); },
            "overflow" => { let o = parse_overflow(val); st.overflow_x = o; st.overflow_y = o; }
            "overflow-x" => st.overflow_x = parse_overflow(val),
            "overflow-y" => st.overflow_y = parse_overflow(val),
            "z-index" => st.z_index = val.parse::<i32>().ok(),
            "opacity" => if let Ok(o) = val.parse::<f32>() { st.opacity_pct = ((o * 100.0) as i32).clamp(0, 100); if st.opacity_pct == 0 { st.display = Display::None; } },
            "transform" => { let (tx, ty, sc) = parse_transform(val); st.transform_tx += tx; st.transform_ty += ty; st.transform_scale_pct = st.transform_scale_pct * sc / 100; },
            "flex-direction" => st.flex_direction = if val.contains("column") { 1 } else { 0 },
            "justify-content" => st.justify_content = match val { "center" => 1, "flex-end" | "end" => 2, "space-between" => 3, "space-around" => 4, _ => 0 },
            "align-items" => st.align_items = match val { "center" => 1, "flex-end" | "end" => 2, "stretch" => 3, _ => 0 },
            "gap" | "column-gap" | "row-gap" => if let Some(px) = to_px(val.split_whitespace().next().unwrap_or(val), &ctx) { st.gap = (px as i32).max(0); },
            "visibility" => if matches!(val, "hidden" | "collapse") { st.display = Display::None; },
            _ => {}
        }
    }
}

fn parse_length(val: &str, ctx: &LenCtx, parent: i32) -> CssLength {
    let v = val.trim();
    if v == "auto" || v.is_empty() { return CssLength::Auto; }
    if let Some(p) = v.strip_suffix('%') { return p.trim().parse::<f32>().ok().map(|x| CssLength::Percent(x as i32)).unwrap_or(CssLength::Auto); }
    to_px(v, ctx).map(|px| CssLength::Px((px as i32).max(0))).unwrap_or_else(|| if parent > 0 { CssLength::Auto } else { CssLength::Px(0) })
}

fn parse_edges(val: &str, ctx: &LenCtx, parent: i32) -> super::computed::Edges<CssLength> {
    let parts: Vec<CssLength> = val.split_whitespace().map(|p| parse_length(p, ctx, parent)).collect();
    match parts.len() {
        1 => Edges::all(parts[0]),
        2 => Edges { top: parts[0], right: parts[1], bottom: parts[0], left: parts[1] },
        3 => Edges { top: parts[0], right: parts[1], bottom: parts[2], left: parts[1] },
        n if n >= 4 => Edges { top: parts[0], right: parts[1], bottom: parts[2], left: parts[3] },
        _ => Edges::all(CssLength::Px(0)),
    }
}

fn parse_edges_px(val: &str, ctx: &LenCtx, parent: i32) -> Edges<i32> {
    let e = parse_edges(val, ctx, parent);
    Edges { top: e.top.resolve(parent, 0), right: e.right.resolve(parent, 0), bottom: e.bottom.resolve(parent, 0), left: e.left.resolve(parent, 0) }
}

fn parse_overflow(v: &str) -> Overflow {
    match v { "hidden" => Overflow::Hidden, "scroll" => Overflow::Scroll, "auto" => Overflow::Auto, "clip" => Overflow::Clip, _ => Overflow::Visible }
}
