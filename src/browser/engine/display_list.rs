//! Liste d'affichage et petites transformations de fragments.
//!
//! Dans une architecture type WebKit/Blink, cette couche correspond au produit
//! du layout avant peinture : rectangles, textes, images et couches d'empilement.

use alloc::string::String;
use alloc::vec::Vec;
use crate::gui::image::Image;

pub enum Item {
    Rect { x: i32, y: i32, w: i32, h: i32, color: u32 },
    RoundedRect { x: i32, y: i32, w: i32, h: i32, radius: i32, color: u32 },
    Text { x: i32, y: i32, s: String, color: u32, scale: usize, bold: bool },
    Image { x: i32, y: i32, w: i32, h: i32, idx: usize },
}

pub struct Link { pub x: i32, pub y: i32, pub w: i32, pub h: i32, pub href: String }

/// Champ de formulaire interactif (input texte / textarea) : zone cliquable
/// pour prendre le focus, saisir au clavier et soumettre (Entree). `action`,
/// `method` et `hidden` viennent du <form> englobant, aplatis ici pour que la
/// soumission construise l'URL complete (ex. /search?q=...&sca_esv=...).
pub struct FormField {
    pub x: i32, pub y: i32, pub w: i32, pub h: i32, // rect en coords document
    pub name: String,                    // attribut name (ex. "q")
    pub value: String,                   // valeur initiale
    pub action: String,                  // action du form (resolue plus tard)
    pub method: String,                  // "get" (seul supporte) / "post"
    pub hidden: Vec<(String, String)>,   // inputs hidden du meme form
}

/// Couche de peinture (stacking layer) : ensemble d'items partageant un ordre
/// d'empilement `z`, eventuellement ancres au viewport (`fixed`) et/ou clippes
/// a un rectangle (overflow). Les items sont en coordonnees document.
pub struct Layer {
    pub z: i32,
    pub fixed: bool,
    pub clip: Option<(i32, i32, i32, i32)>, // (x, y, w, h) en coords document
    pub items: Vec<Item>,
    pub links: Vec<Link>,
}

pub struct Page {
    pub title: String,
    pub items: Vec<Item>,
    pub links: Vec<Link>,
    pub fields: Vec<FormField>,
    pub images: Vec<Image>,
    pub height: i32,
    pub bg: u32,
    /// Couches positionnees (position: absolute/fixed/relative+z-index), triees
    /// par `z` croissant. Le flux normal est `items` (z=0 implicite).
    pub layers: Vec<Layer>,
}

pub fn translate_item(it: &mut Item, dx: i32, dy: i32) {
    match it {
        Item::Rect { x, y, .. } | Item::RoundedRect { x, y, .. } | Item::Text { x, y, .. } | Item::Image { x, y, .. } => { *x += dx; *y += dy; }
    }
}

fn scale_item_from(it: &mut Item, ox: i32, oy: i32, pct: i32) {
    if pct == 100 { return; }
    let sc = |v: i32| -> i32 { v * pct / 100 };
    match it {
        Item::Rect { x, y, w, h, .. } | Item::RoundedRect { x, y, w, h, .. } => {
            *x = ox + sc(*x - ox); *y = oy + sc(*y - oy);
            *w = sc(*w).max(1); *h = sc(*h).max(1);
        }
        Item::Text { x, y, scale, .. } => {
            *x = ox + sc(*x - ox); *y = oy + sc(*y - oy);
            let ns = ((*scale as i32 * pct + 50) / 100).clamp(1, 6) as usize;
            *scale = ns;
        }
        Item::Image { x, y, w, h, .. } => {
            *x = ox + sc(*x - ox); *y = oy + sc(*y - oy);
            *w = sc(*w).max(1); *h = sc(*h).max(1);
        }
    }
}

pub fn apply_box_transform(items: &mut [Item], links: &mut [Link], ox: i32, oy: i32, tx: i32, ty: i32, scale_pct: i32) {
    if tx == 0 && ty == 0 && scale_pct == 100 { return; }
    for it in items {
        scale_item_from(it, ox, oy, scale_pct);
        translate_item(it, tx, ty);
    }
    for lk in links {
        if scale_pct != 100 {
            lk.x = ox + (lk.x - ox) * scale_pct / 100;
            lk.y = oy + (lk.y - oy) * scale_pct / 100;
            lk.w = (lk.w * scale_pct / 100).max(1);
            lk.h = (lk.h * scale_pct / 100).max(1);
        }
        lk.x += tx; lk.y += ty;
    }
}
