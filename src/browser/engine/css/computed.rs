//! CSSOM calcule de Nautile.
//!
//! Cette couche est volontairement separee du layout heuristique historique de
//! `web.rs` : les Web APIs et le futur layout tree lisent toutes le meme
//! `ComputedStyle`, comme dans un vrai navigateur (DOM -> CSSOM -> computed
//! style -> layout tree -> paint).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Display { None, Inline, Block, InlineBlock, Flex, Grid }

impl Display {
    pub fn as_css(self) -> &'static str {
        match self {
            Display::None => "none",
            Display::Inline => "inline",
            Display::Block => "block",
            Display::InlineBlock => "inline-block",
            Display::Flex => "flex",
            Display::Grid => "grid",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Position { Static, Relative, Absolute, Fixed, Sticky }

impl Position {
    pub fn as_css(self) -> &'static str {
        match self {
            Position::Static => "static",
            Position::Relative => "relative",
            Position::Absolute => "absolute",
            Position::Fixed => "fixed",
            Position::Sticky => "sticky",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BoxSizing { ContentBox, BorderBox }

impl BoxSizing {
    pub fn as_css(self) -> &'static str {
        match self { BoxSizing::ContentBox => "content-box", BoxSizing::BorderBox => "border-box" }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Overflow { Visible, Hidden, Scroll, Auto, Clip }

impl Overflow {
    pub fn as_css(self) -> &'static str {
        match self {
            Overflow::Visible => "visible",
            Overflow::Hidden => "hidden",
            Overflow::Scroll => "scroll",
            Overflow::Auto => "auto",
            Overflow::Clip => "clip",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CssLength {
    Auto,
    Px(i32),
    Percent(i32), // entier en pourcentage, 100 = 100%
}

impl CssLength {
    pub fn auto_or_px(px: i32) -> CssLength { if px < 0 { CssLength::Auto } else { CssLength::Px(px) } }
    pub fn resolve(self, parent: i32, fallback: i32) -> i32 {
        match self {
            CssLength::Auto => fallback,
            CssLength::Px(v) => v,
            CssLength::Percent(p) => parent.saturating_mul(p) / 100,
        }
    }
    pub fn to_css(self) -> String {
        match self {
            CssLength::Auto => "auto".to_string(),
            CssLength::Px(v) => alloc::format!("{}px", v),
            CssLength::Percent(p) => alloc::format!("{}%", p),
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct Edges<T: Copy> { pub top: T, pub right: T, pub bottom: T, pub left: T }

impl<T: Copy> Edges<T> {
    pub fn all(v: T) -> Edges<T> { Edges { top: v, right: v, bottom: v, left: v } }
}

#[derive(Clone)]
pub struct ComputedStyle {
    pub display: Display,
    pub position: Position,
    pub box_sizing: BoxSizing,
    pub width: CssLength,
    pub height: CssLength,
    pub min_width: CssLength,
    pub max_width: CssLength,
    pub min_height: CssLength,
    pub max_height: CssLength,
    pub margin: Edges<CssLength>,
    pub padding: Edges<CssLength>,
    pub border_width: Edges<i32>,
    pub color: u32,
    pub background_color: Option<u32>,
    pub font_size: i32,
    pub font_weight: u16,
    pub line_height: i32,
    pub overflow_x: Overflow,
    pub overflow_y: Overflow,
    pub z_index: Option<i32>,
    pub opacity_pct: i32,
    pub transform_tx: i32,
    pub transform_ty: i32,
    pub transform_scale_pct: i32,
    pub flex_direction: u8, // 0 row, 1 column
    pub justify_content: u8,
    pub align_items: u8,
    pub gap: i32,
}

impl ComputedStyle {
    pub fn initial() -> ComputedStyle {
        ComputedStyle {
            display: Display::Inline,
            position: Position::Static,
            box_sizing: BoxSizing::ContentBox,
            width: CssLength::Auto,
            height: CssLength::Auto,
            min_width: CssLength::Auto,
            max_width: CssLength::Auto,
            min_height: CssLength::Auto,
            max_height: CssLength::Auto,
            margin: Edges::all(CssLength::Px(0)),
            padding: Edges::all(CssLength::Px(0)),
            border_width: Edges::all(0),
            color: 0x111111,
            background_color: None,
            font_size: 16,
            font_weight: 400,
            line_height: 19,
            overflow_x: Overflow::Visible,
            overflow_y: Overflow::Visible,
            z_index: None,
            opacity_pct: 100,
            transform_tx: 0,
            transform_ty: 0,
            transform_scale_pct: 100,
            flex_direction: 0,
            justify_content: 0,
            align_items: 0,
            gap: 0,
        }
    }

    pub fn inherit(parent: Option<&ComputedStyle>, tag: &str) -> ComputedStyle {
        let mut s = ComputedStyle::initial();
        if let Some(p) = parent {
            s.color = p.color;
            s.font_size = p.font_size;
            s.font_weight = p.font_weight;
            s.line_height = p.line_height;
        }
        s.display = default_display(tag);
        if matches!(tag, "b" | "strong" | "th") { s.font_weight = 700; }
        if matches!(tag, "h1" | "h2" | "h3") { s.font_weight = 700; s.font_size = if tag == "h1" { 32 } else if tag == "h2" { 24 } else { 20 }; s.line_height = s.font_size + 8; }
        if matches!(tag, "input" | "button" | "select" | "textarea" | "img" | "svg" | "canvas" | "video" | "audio") { s.display = Display::InlineBlock; }
        s
    }

    pub fn property(&self, name: &str) -> String {
        match css_prop_name(name).as_str() {
            "display" => self.display.as_css().to_string(),
            "position" => self.position.as_css().to_string(),
            "box-sizing" => self.box_sizing.as_css().to_string(),
            "width" => self.width.to_css(),
            "height" => self.height.to_css(),
            "min-width" => self.min_width.to_css(),
            "max-width" => self.max_width.to_css(),
            "min-height" => self.min_height.to_css(),
            "max-height" => self.max_height.to_css(),
            "margin-top" => self.margin.top.to_css(),
            "margin-right" => self.margin.right.to_css(),
            "margin-bottom" => self.margin.bottom.to_css(),
            "margin-left" => self.margin.left.to_css(),
            "padding-top" => self.padding.top.to_css(),
            "padding-right" => self.padding.right.to_css(),
            "padding-bottom" => self.padding.bottom.to_css(),
            "padding-left" => self.padding.left.to_css(),
            "border-top-width" => alloc::format!("{}px", self.border_width.top),
            "border-right-width" => alloc::format!("{}px", self.border_width.right),
            "border-bottom-width" => alloc::format!("{}px", self.border_width.bottom),
            "border-left-width" => alloc::format!("{}px", self.border_width.left),
            "color" => color_css(self.color),
            "background-color" => self.background_color.map(color_css).unwrap_or_else(|| "transparent".to_string()),
            "font-size" => alloc::format!("{}px", self.font_size),
            "font-weight" => self.font_weight.to_string(),
            "line-height" => alloc::format!("{}px", self.line_height),
            "overflow" => self.overflow_y.as_css().to_string(),
            "overflow-x" => self.overflow_x.as_css().to_string(),
            "overflow-y" => self.overflow_y.as_css().to_string(),
            "opacity" => alloc::format!("{}", self.opacity_pct as f32 / 100.0),
            "z-index" => self.z_index.map(|z| z.to_string()).unwrap_or_else(|| "auto".to_string()),
            "transform" => if self.transform_tx == 0 && self.transform_ty == 0 && self.transform_scale_pct == 100 { "none".to_string() } else { alloc::format!("translate({}px, {}px) scale({})", self.transform_tx, self.transform_ty, self.transform_scale_pct as f32 / 100.0) },
            "gap" => alloc::format!("{}px", self.gap),
            "flex-direction" => if self.flex_direction == 1 { "column" } else { "row" }.to_string(),
            _ => String::new(),
        }
    }

    pub fn property_names() -> Vec<&'static str> {
        alloc::vec![
            "display", "position", "box-sizing", "width", "height",
            "min-width", "max-width", "min-height", "max-height",
            "margin-top", "margin-right", "margin-bottom", "margin-left",
            "padding-top", "padding-right", "padding-bottom", "padding-left",
            "border-top-width", "border-right-width", "border-bottom-width", "border-left-width",
            "color", "background-color", "font-size", "font-weight", "line-height",
            "overflow", "overflow-x", "overflow-y", "opacity", "z-index", "transform", "gap", "flex-direction",
        ]
    }
}

pub fn default_display(tag: &str) -> Display {
    match tag {
        "#root" | "html" | "body" | "address" | "article" | "aside" | "blockquote" | "details" | "div" | "dl" |
        "fieldset" | "figcaption" | "figure" | "footer" | "form" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" |
        "header" | "hr" | "main" | "nav" | "ol" | "p" | "pre" | "section" | "table" | "ul" | "li" => Display::Block,
        "script" | "style" | "head" | "template" | "noscript" | "meta" | "link" | "title" => Display::None,
        _ => Display::Inline,
    }
}

pub fn css_prop_name(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_uppercase() { out.push('-'); out.push(ch.to_ascii_lowercase()); } else { out.push(ch); }
    }
    out
}

pub fn color_css(c: u32) -> String { alloc::format!("rgb({}, {}, {})", (c >> 16) & 255, (c >> 8) & 255, c & 255) }
