//! Structures de style CSS : selecteurs, regles et index de matching.
//!
//! Cette couche isole la partie "style resolver" du moteur, comme les moteurs
//! modernes separant DOM, style, layout et paint.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

// Selecteur simple compose (comme les moteurs reels : Servo/WebKit gerent un
// « compound selector » = un tag optionnel + un id optionnel + N classes). On
// remplace l'ancien enum a une seule classe pour ne plus perdre les selecteurs
// composes tres frequents (`.gb_A.gb_B`, `div#main.active`).
#[derive(Clone)]
pub(super) struct Sel {
    pub tag: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
}

impl Sel {
    pub fn any() -> Sel { Sel { tag: None, id: None, classes: Vec::new() } }
    pub fn is_any(&self) -> bool { self.tag.is_none() && self.id.is_none() && self.classes.is_empty() }
    // Cible-t-il la racine (body/html nu) pour la detection du fond global.
    pub fn is_root_tag(&self) -> bool {
        self.id.is_none() && self.classes.is_empty()
            && matches!(self.tag.as_deref(), Some("body") | Some("html"))
    }
}

// Une regle CSS : chaine de selecteurs simples (combinateur descendant), les
// declarations, et la specificite cumulee. `chain` est ordonnee ancetre→cible :
// le DERNIER element doit matcher l'element courant, les precedents ses ancetres.
pub(super) struct Rule { pub chain: Vec<Sel>, pub decls: Vec<(String, String)>, pub spec: u32 }

pub(super) struct CssIndex {
    pub any: Vec<usize>,
    tags: Vec<(String, Vec<usize>)>,
    classes: Vec<(String, Vec<usize>)>,
    ids: Vec<(String, Vec<usize>)>,
}

impl CssIndex {
    pub fn new(css: &[Rule]) -> CssIndex {
        let mut idx = CssIndex { any: Vec::new(), tags: Vec::new(), classes: Vec::new(), ids: Vec::new() };
        for (i, r) in css.iter().enumerate() {
            // Clé d'indexation = partie la plus discriminante du selecteur cible
            // (id > 1re classe > tag > universel), comme le « key selector » des
            // vrais moteurs. Le matching complet est verifie ensuite.
            match r.chain.last() {
                Some(s) if s.id.is_some() => push_bucket(&mut idx.ids, s.id.as_ref().unwrap(), i),
                Some(s) if !s.classes.is_empty() => push_bucket(&mut idx.classes, &s.classes[0], i),
                Some(s) if s.tag.is_some() => push_bucket(&mut idx.tags, s.tag.as_ref().unwrap(), i),
                _ => idx.any.push(i),
            }
        }
        idx
    }

    fn bucket<'b>(buckets: &'b [(String, Vec<usize>)], key: &str) -> &'b [usize] {
        buckets.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_slice()).unwrap_or(&[])
    }

    pub fn tags(&self, tag: &str) -> &[usize] { Self::bucket(&self.tags, tag) }
    pub fn classes(&self, class: &str) -> &[usize] { Self::bucket(&self.classes, class) }
    pub fn ids(&self, id: &str) -> &[usize] { Self::bucket(&self.ids, id) }
}

fn push_bucket(buckets: &mut Vec<(String, Vec<usize>)>, key: &str, idx: usize) {
    if let Some((_, v)) = buckets.iter_mut().find(|(k, _)| k == key) {
        v.push(idx);
    } else {
        buckets.push((key.to_string(), alloc::vec![idx]));
    }
}
