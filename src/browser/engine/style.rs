//! Structures de style CSS : selecteurs, regles et index de matching.
//!
//! Cette couche isole la partie "style resolver" du moteur, comme les moteurs
//! modernes separant DOM, style, layout et paint.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

// Combinateur reliant un compound a celui situe a sa GAUCHE dans la chaine.
// (Comme Servo/WebKit : descendant « a b », enfant « a>b », frere adjacent
// « a+b », frere general « a~b ».)
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Comb { Descendant, Child, Adjacent, General }

// Operateur d'un selecteur d'attribut : [a] [a=v] [a^=v] [a$=v] [a*=v] [a~=v] [a|=v].
#[derive(Clone)]
pub(super) enum AttrOp { Exists, Eq, Prefix, Suffix, Substr, Word, Dash }

#[derive(Clone)]
pub(super) struct AttrSel { pub name: String, pub op: AttrOp, pub val: String }

// Pseudo-classes structurelles/etat. Les etats non decidables statiquement
// (:hover/:focus...) ne sont pas stockes ici (traites comme neutres au parse).
#[derive(Clone)]
pub(super) enum Pseudo {
    FirstChild,
    LastChild,
    OnlyChild,
    Empty,
    Root,
    NthChild(i32, i32),      // an+b (1-based, depuis le debut)
    NthLastChild(i32, i32),  // an+b (depuis la fin)
    Not(Vec<Sel>),           // negation d'un ou plusieurs compounds simples
}

// Selecteur simple compose (« compound selector ») : un tag optionnel, un id
// optionnel, N classes, N selecteurs d'attribut, N pseudo-classes, et le
// combinateur qui le relie au compound de gauche.
#[derive(Clone)]
pub(super) struct Sel {
    pub tag: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub attrs: Vec<AttrSel>,
    pub pseudos: Vec<Pseudo>,
    pub comb: Comb,
}

impl Sel {
    pub fn any() -> Sel {
        Sel { tag: None, id: None, classes: Vec::new(), attrs: Vec::new(), pseudos: Vec::new(), comb: Comb::Descendant }
    }
    pub fn is_any(&self) -> bool {
        self.tag.is_none() && self.id.is_none() && self.classes.is_empty()
            && self.attrs.is_empty() && self.pseudos.is_empty()
    }
    // Cible-t-il la racine (body/html nu) pour la detection du fond global.
    pub fn is_root_tag(&self) -> bool {
        self.id.is_none() && self.classes.is_empty() && self.attrs.is_empty() && self.pseudos.is_empty()
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
