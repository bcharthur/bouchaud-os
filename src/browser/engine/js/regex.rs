//! Moteur d'expressions rationnelles minimal mais reel (backtracking recursif),
//! inspire des implementations classiques (pike/thompson en variante recursive).
//! Couvre l'essentiel de la syntaxe JS utilisee par les bundles :
//!   litteraux, `.`, classes `[...]`/`[^...]` avec plages, `\d \w \s` (+ maj.),
//!   ancres `^ $`, frontieres `\b \B`, groupes `( )` et `(?: )`, alternation `|`,
//!   quantificateurs `* + ? {n} {n,} {n,m}` gourmands ou paresseux (`*?`),
//!   drapeaux `i` (casse), `g` (global), `m` (multiligne), `s` (dotall).
//! Volontairement sans lookahead/lookbehind ni back-references (rares et couteux).

use alloc::string::String;
use alloc::vec::Vec;
use alloc::vec;
use alloc::boxed::Box;

#[derive(Clone, Copy, Default)]
pub struct Flags { pub i: bool, pub g: bool, pub m: bool, pub s: bool }

#[derive(Clone)]
enum CItem { Ch(char), Range(char, char), Digit, NotDigit, Word, NotWord, Space, NotSpace }

#[derive(Clone)]
enum Node {
    Char(char),
    Any,
    Class(Vec<CItem>, bool),      // items, negated
    Start,
    End,
    WordB(bool),                  // \b (true) / \B (false)
    Open(usize),                  // debut de groupe capturant idx
    Close(usize),                 // fin de groupe capturant idx
    Alt(Vec<Vec<Node>>),          // branches
    Repeat(Box<Vec<Node>>, usize, Option<usize>, bool), // corps, min, max, gourmand
}

pub struct Regex {
    prog: Vec<Node>,
    pub ngroups: usize,
    pub flags: Flags,
    pub source: String,
}

pub struct Match {
    // Spans (indices de char) : [0] = match complet, [k] = groupe k (ou None).
    pub caps: Vec<Option<(usize, usize)>>,
}

struct Parser<'a> { c: &'a [char], i: usize, ngroups: usize }

impl<'a> Parser<'a> {
    fn parse_alt(&mut self) -> Vec<Node> {
        let mut branches = vec![self.parse_seq()];
        while self.peek() == Some('|') { self.i += 1; branches.push(self.parse_seq()); }
        if branches.len() == 1 { branches.pop().unwrap() } else { vec![Node::Alt(branches)] }
    }

    fn parse_seq(&mut self) -> Vec<Node> {
        let mut seq = Vec::new();
        while let Some(ch) = self.peek() {
            if ch == '|' || ch == ')' { break; }
            let atom = self.parse_atom();
            if atom.is_empty() { break; }
            // Quantificateur eventuel sur le dernier atome.
            if let Some(q) = self.peek() {
                if q == '*' || q == '+' || q == '?' || q == '{' {
                    if let Some((min, max)) = self.parse_quant(q) {
                        let greedy = if self.peek() == Some('?') { self.i += 1; false } else { true };
                        seq.push(Node::Repeat(Box::new(atom), min, max, greedy));
                        continue;
                    }
                }
            }
            seq.extend(atom);
        }
        seq
    }

    // Renvoie (min,max) et consomme le quantificateur, ou None si `{` non valide.
    fn parse_quant(&mut self, q: char) -> Option<(usize, Option<usize>)> {
        match q {
            '*' => { self.i += 1; Some((0, None)) }
            '+' => { self.i += 1; Some((1, None)) }
            '?' => { self.i += 1; Some((0, Some(1))) }
            '{' => {
                let save = self.i; self.i += 1;
                let min = self.parse_int();
                let mut max = min;
                let mut has_comma = false;
                if self.peek() == Some(',') { self.i += 1; has_comma = true; max = self.parse_int(); }
                if self.peek() == Some('}') {
                    self.i += 1;
                    let lo = min.unwrap_or(0);
                    let hi = if has_comma { max } else { Some(lo) };
                    Some((lo, hi))
                } else { self.i = save; None } // `{` litteral
            }
            _ => None,
        }
    }

    fn parse_int(&mut self) -> Option<usize> {
        let start = self.i;
        while let Some(c) = self.peek() { if c.is_ascii_digit() { self.i += 1; } else { break; } }
        if self.i == start { None } else { self.c[start..self.i].iter().collect::<String>().parse().ok() }
    }

    fn parse_atom(&mut self) -> Vec<Node> {
        let ch = match self.peek() { Some(c) => c, None => return Vec::new() };
        match ch {
            '(' => {
                self.i += 1;
                let capturing = if self.starts_with("?:") { self.i += 2; false }
                    else if self.peek() == Some('?') {
                        // (?=...), (?!...), (?<...>) non geres : on saute le prefixe.
                        self.i += 1; if let Some(c) = self.peek() { if c == '=' || c == '!' { self.i += 1; } }
                        false
                    } else { true };
                let idx = if capturing { self.ngroups += 1; self.ngroups } else { 0 };
                let inner = self.parse_alt();
                if self.peek() == Some(')') { self.i += 1; }
                if capturing {
                    let mut v = vec![Node::Open(idx)];
                    v.extend(inner);
                    v.push(Node::Close(idx));
                    v
                } else { inner }
            }
            '[' => vec![self.parse_class()],
            '.' => { self.i += 1; vec![Node::Any] }
            '^' => { self.i += 1; vec![Node::Start] }
            '$' => { self.i += 1; vec![Node::End] }
            '\\' => { self.i += 1; self.parse_escape() }
            _ => { self.i += 1; vec![Node::Char(ch)] }
        }
    }

    fn parse_escape(&mut self) -> Vec<Node> {
        let ch = match self.peek() { Some(c) => c, None => return vec![Node::Char('\\')] };
        self.i += 1;
        match ch {
            'd' => vec![Node::Class(vec![CItem::Digit], false)],
            'D' => vec![Node::Class(vec![CItem::NotDigit], false)],
            'w' => vec![Node::Class(vec![CItem::Word], false)],
            'W' => vec![Node::Class(vec![CItem::NotWord], false)],
            's' => vec![Node::Class(vec![CItem::Space], false)],
            'S' => vec![Node::Class(vec![CItem::NotSpace], false)],
            'b' => vec![Node::WordB(true)],
            'B' => vec![Node::WordB(false)],
            'n' => vec![Node::Char('\n')],
            't' => vec![Node::Char('\t')],
            'r' => vec![Node::Char('\r')],
            'f' => vec![Node::Char('\u{c}')],
            'v' => vec![Node::Char('\u{b}')],
            '0' => vec![Node::Char('\0')],
            'u' => vec![Node::Char(self.parse_unicode())],
            'x' => vec![Node::Char(self.parse_hex(2))],
            other => vec![Node::Char(other)], // \. \\ \/ \( ... -> litteral
        }
    }

    fn parse_unicode(&mut self) -> char {
        if self.peek() == Some('{') {
            self.i += 1; let start = self.i;
            while let Some(c) = self.peek() { if c == '}' { break; } self.i += 1; }
            let v = u32::from_str_radix(&self.c[start..self.i].iter().collect::<String>(), 16).unwrap_or(0);
            if self.peek() == Some('}') { self.i += 1; }
            char::from_u32(v).unwrap_or('\u{FFFD}')
        } else { self.parse_hex(4) }
    }

    fn parse_hex(&mut self, n: usize) -> char {
        let start = self.i;
        for _ in 0..n { if self.peek().map_or(false, |c| c.is_ascii_hexdigit()) { self.i += 1; } }
        let v = u32::from_str_radix(&self.c[start..self.i].iter().collect::<String>(), 16).unwrap_or(0);
        char::from_u32(v).unwrap_or('\u{FFFD}')
    }

    fn parse_class(&mut self) -> Node {
        self.i += 1; // consomme '['
        let neg = if self.peek() == Some('^') { self.i += 1; true } else { false };
        let mut items = Vec::new();
        while let Some(ch) = self.peek() {
            if ch == ']' { self.i += 1; break; }
            let lo = if ch == '\\' {
                self.i += 1;
                match self.parse_class_escape() { Ok(c) => c, Err(items2) => { items.extend(items2); continue; } }
            } else { self.i += 1; ch };
            // Plage a-z ?
            if self.peek() == Some('-') && self.peek_at(1).map_or(false, |c| c != ']') {
                self.i += 1;
                let hi = if self.peek() == Some('\\') { self.i += 1; self.parse_class_escape().unwrap_or('-') }
                    else { let c = self.peek().unwrap(); self.i += 1; c };
                items.push(CItem::Range(lo, hi));
            } else {
                items.push(CItem::Ch(lo));
            }
        }
        Node::Class(items, neg)
    }

    // Escape dans une classe : soit un char, soit une sous-classe (\d, \w...).
    fn parse_class_escape(&mut self) -> Result<char, Vec<CItem>> {
        let ch = match self.peek() { Some(c) => c, None => return Ok('\\') };
        self.i += 1;
        match ch {
            'd' => Err(vec![CItem::Digit]),
            'D' => Err(vec![CItem::NotDigit]),
            'w' => Err(vec![CItem::Word]),
            'W' => Err(vec![CItem::NotWord]),
            's' => Err(vec![CItem::Space]),
            'S' => Err(vec![CItem::NotSpace]),
            'n' => Ok('\n'), 't' => Ok('\t'), 'r' => Ok('\r'), 'f' => Ok('\u{c}'), 'v' => Ok('\u{b}'), '0' => Ok('\0'),
            'u' => Ok(self.parse_unicode()),
            'x' => Ok(self.parse_hex(2)),
            other => Ok(other),
        }
    }

    fn peek(&self) -> Option<char> { self.c.get(self.i).copied() }
    fn peek_at(&self, k: usize) -> Option<char> { self.c.get(self.i + k).copied() }
    fn starts_with(&self, s: &str) -> bool { s.chars().enumerate().all(|(k, c)| self.peek_at(k) == Some(c)) }
}

fn is_word(c: char) -> bool { c.is_ascii_alphanumeric() || c == '_' }

fn citem_match(item: &CItem, c: char, ci: bool) -> bool {
    let eqc = |a: char, b: char| if ci { a.eq_ignore_ascii_case(&b) } else { a == b };
    match item {
        CItem::Ch(x) => eqc(*x, c),
        CItem::Range(a, b) => {
            (c >= *a && c <= *b) || (ci && {
                let l = c.to_ascii_lowercase(); let u = c.to_ascii_uppercase();
                (l >= *a && l <= *b) || (u >= *a && u <= *b)
            })
        }
        CItem::Digit => c.is_ascii_digit(),
        CItem::NotDigit => !c.is_ascii_digit(),
        CItem::Word => is_word(c),
        CItem::NotWord => !is_word(c),
        CItem::Space => c.is_whitespace(),
        CItem::NotSpace => !c.is_whitespace(),
    }
}

impl Regex {
    pub fn new(pattern: &str, flags_str: &str) -> Regex {
        let mut f = Flags::default();
        for c in flags_str.chars() {
            match c { 'i' => f.i = true, 'g' => f.g = true, 'm' => f.m = true, 's' => f.s = true, _ => {} }
        }
        let chars: Vec<char> = pattern.chars().collect();
        let mut p = Parser { c: &chars, i: 0, ngroups: 0 };
        let prog = p.parse_alt();
        Regex { prog, ngroups: p.ngroups, flags: f, source: String::from(pattern) }
    }

    fn ceq(&self, a: char, b: char) -> bool { if self.flags.i { a.eq_ignore_ascii_case(&b) } else { a == b } }

    fn word_boundary(&self, chars: &[char], pos: usize) -> bool {
        let before = pos > 0 && is_word(chars[pos - 1]);
        let after = pos < chars.len() && is_word(chars[pos]);
        before != after
    }

    // Matche `seq` a partir de `pos`, avec une pile de continuations (sequences
    // restantes a matcher apres celle-ci). Renvoie la position de fin.
    fn m(&self, seq: &[Node], chars: &[char], pos: usize,
         caps: &mut Vec<Option<(usize, usize)>>, conts: &[&[Node]]) -> Option<usize> {
        let (first, rest) = match seq.split_first() {
            Some(x) => x,
            None => {
                return match conts.split_first() {
                    Some((n, r)) => self.m(n, chars, pos, caps, r),
                    None => Some(pos),
                };
            }
        };
        match first {
            Node::Char(c) => if pos < chars.len() && self.ceq(chars[pos], *c) { self.m(rest, chars, pos + 1, caps, conts) } else { None },
            Node::Any => if pos < chars.len() && (self.flags.s || chars[pos] != '\n') { self.m(rest, chars, pos + 1, caps, conts) } else { None },
            Node::Class(items, neg) => {
                if pos < chars.len() {
                    let hit = items.iter().any(|it| citem_match(it, chars[pos], self.flags.i));
                    if hit != *neg { return self.m(rest, chars, pos + 1, caps, conts); }
                }
                None
            }
            Node::Start => if pos == 0 || (self.flags.m && chars[pos - 1] == '\n') { self.m(rest, chars, pos, caps, conts) } else { None },
            Node::End => if pos == chars.len() || (self.flags.m && chars[pos] == '\n') { self.m(rest, chars, pos, caps, conts) } else { None },
            Node::WordB(want) => if self.word_boundary(chars, pos) == *want { self.m(rest, chars, pos, caps, conts) } else { None },
            Node::Open(i) => {
                let save = caps[*i];
                caps[*i] = Some((pos, pos));
                let r = self.m(rest, chars, pos, caps, conts);
                if r.is_none() { caps[*i] = save; }
                r
            }
            Node::Close(i) => {
                let save = caps[*i];
                if let Some((s, _)) = caps[*i] { caps[*i] = Some((s, pos)); }
                let r = self.m(rest, chars, pos, caps, conts);
                if r.is_none() { caps[*i] = save; }
                r
            }
            Node::Alt(branches) => {
                // Empile `rest` comme continuation puis essaie chaque branche.
                let mut nc: Vec<&[Node]> = Vec::with_capacity(conts.len() + 1);
                nc.push(rest);
                nc.extend_from_slice(conts);
                for b in branches {
                    let mut c2 = caps.clone();
                    if let Some(e) = self.m(b, chars, pos, &mut c2, &nc) { *caps = c2; return Some(e); }
                }
                None
            }
            Node::Repeat(body, min, max, greedy) => {
                self.match_repeat(body, *min, *max, *greedy, 0, chars, pos, caps, rest, conts)
            }
        }
    }

    fn match_repeat(&self, body: &[Node], min: usize, max: Option<usize>, greedy: bool, count: usize,
                    chars: &[char], pos: usize, caps: &mut Vec<Option<(usize, usize)>>,
                    rest: &[Node], conts: &[&[Node]]) -> Option<usize> {
        let more_allowed = max.map_or(true, |m| count < m);
        let can_stop = count >= min;

        let try_more = |s: &Regex, caps: &mut Vec<Option<(usize, usize)>>| -> Option<usize> {
            if !more_allowed { return None; }
            let mut c2 = caps.clone();
            let p1 = s.m(body, chars, pos, &mut c2, &[])?;
            if p1 == pos && count >= min { return None; } // garde-fou match vide
            let e = s.match_repeat(body, min, max, greedy, count + 1, chars, p1, &mut c2, rest, conts)?;
            *caps = c2;
            Some(e)
        };
        let try_stop = |s: &Regex, caps: &mut Vec<Option<(usize, usize)>>| -> Option<usize> {
            if !can_stop { return None; }
            s.m(rest, chars, pos, caps, conts)
        };

        if greedy {
            if let Some(e) = try_more(self, caps) { return Some(e); }
            try_stop(self, caps)
        } else {
            if let Some(e) = try_stop(self, caps) { return Some(e); }
            try_more(self, caps)
        }
    }

    /// Cherche le premier match a partir de l'index de char `start`.
    pub fn exec(&self, chars: &[char], start: usize) -> Option<Match> {
        let anchored = matches!(self.prog.first(), Some(Node::Start)) && !self.flags.m;
        let mut i = start;
        loop {
            let mut caps = vec![None; self.ngroups + 1];
            if let Some(e) = self.m(&self.prog, chars, i, &mut caps, &[]) {
                caps[0] = Some((i, e));
                return Some(Match { caps });
            }
            if anchored || i >= chars.len() { return None; }
            i += 1;
        }
    }
}
