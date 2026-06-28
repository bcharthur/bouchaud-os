//! Structures de style CSS : selecteurs, regles et index de matching.
//!
//! Cette couche isole la partie "style resolver" du moteur, comme les moteurs
//! modernes separant DOM, style, layout et paint.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

#[derive(Clone)]
pub(super) enum Sel { Any, Tag(String), Class(String), Id(String), TagClass(String, String) }

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
            match r.chain.last().unwrap_or(&Sel::Any) {
                Sel::Any => idx.any.push(i),
                Sel::Tag(t) => push_bucket(&mut idx.tags, t, i),
                Sel::Class(c) => push_bucket(&mut idx.classes, c, i),
                Sel::Id(id) => push_bucket(&mut idx.ids, id, i),
                Sel::TagClass(_, c) => push_bucket(&mut idx.classes, c, i),
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
