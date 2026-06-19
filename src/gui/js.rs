//! Mini moteur JavaScript pour le navigateur graphique.
//!
//! Objectif volontairement modeste et coherent avec le moteur HTML/CSS actuel :
//! executer les scripts inline courants qui injectent du HTML pendant le parsing
//! (`document.write`) ou hydratent un conteneur simple par id (`innerHTML`,
//! `textContent`). Ce n'est pas un moteur ECMAScript complet : pas de reseau
//! asynchrone, pas d'evenements, pas de JIT, mais assez pour de nombreuses pages
//! statiques qui dependent de petits scripts de bootstrap.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

const MAX_SCRIPT: usize = 128_000;
const MAX_OUTPUT: usize = 2_000_000;
const MAX_STEPS: usize = 20_000;

#[derive(Clone)]
struct Var {
    name: String,
    value: String,
}

#[derive(Default)]
struct JsEnv {
    vars: Vec<Var>,
    writes: Vec<String>,
    patches: Vec<(String, String)>,
    steps: usize,
}

impl JsEnv {
    fn get(&self, name: &str) -> String {
        self.vars
            .iter()
            .rev()
            .find(|v| v.name == name)
            .map(|v| v.value.clone())
            .unwrap_or_default()
    }

    fn set(&mut self, name: &str, value: String) {
        if let Some(v) = self.vars.iter_mut().rev().find(|v| v.name == name) {
            v.value = value;
            return;
        }
        if self.vars.len() < 256 {
            self.vars.push(Var {
                name: name.to_string(),
                value,
            });
        }
    }
}

/// Execute les `<script>` inline supportes et renvoie un HTML enrichi.
pub fn execute_inline(html: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(html.len().min(MAX_OUTPUT));
    let mut env = JsEnv::default();
    let mut i = 0usize;
    while i < html.len() && out.len() < MAX_OUTPUT {
        if starts_ci(&html[i..], b"<script") {
            let content_start = find_ci(html, b">", i).map(|p| p + 1).unwrap_or(html.len());
            let content_end = find_ci(html, b"</script", content_start).unwrap_or(html.len());
            if content_end > content_start && content_end - content_start <= MAX_SCRIPT {
                if let Ok(src) = core::str::from_utf8(&html[content_start..content_end]) {
                    eval_script(src, &mut env);
                    flush_writes(&mut out, &mut env);
                    apply_patches(&mut out, &mut env);
                }
            }
            i = find_ci(html, b">", content_end)
                .map(|p| p + 1)
                .unwrap_or(html.len());
            continue;
        }
        out.push(html[i]);
        i += 1;
    }
    out
}

fn flush_writes(out: &mut Vec<u8>, env: &mut JsEnv) {
    for s in env.writes.drain(..) {
        if out.len() + s.len() > MAX_OUTPUT {
            break;
        }
        out.extend_from_slice(s.as_bytes());
    }
}

fn apply_patches(out: &mut Vec<u8>, env: &mut JsEnv) {
    for (id, html) in env.patches.drain(..) {
        if let Some((a, b)) = element_inner_range_by_id(out, &id) {
            let mut next = Vec::with_capacity((out.len() + html.len()).min(MAX_OUTPUT));
            next.extend_from_slice(&out[..a]);
            next.extend_from_slice(html.as_bytes());
            next.extend_from_slice(&out[b..]);
            if next.len() <= MAX_OUTPUT {
                *out = next;
            }
        }
    }
}

fn eval_script(src: &str, env: &mut JsEnv) {
    for stmt in split_statements(src) {
        if env.steps >= MAX_STEPS {
            break;
        }
        env.steps += 1;
        eval_statement(stmt.trim(), env);
    }
}

fn eval_statement(stmt: &str, env: &mut JsEnv) {
    if stmt.is_empty() || stmt.starts_with("//") {
        return;
    }
    if let Some(rest) = stmt
        .strip_prefix("var ")
        .or_else(|| stmt.strip_prefix("let "))
        .or_else(|| stmt.strip_prefix("const "))
    {
        if let Some(eq) = rest.find('=') {
            let name = rest[..eq].trim();
            if is_ident(name) {
                env.set(name, eval_expr(&rest[eq + 1..], env));
            }
        }
        return;
    }
    if let Some(args) = call_args(stmt, "document.write") {
        env.writes.push(eval_expr(args, env));
        return;
    }
    if let Some(args) = call_args(stmt, "document.writeln") {
        let mut s = eval_expr(args, env);
        s.push('\n');
        env.writes.push(s);
        return;
    }
    if let Some((id, value)) = dom_assignment(stmt, "innerHTML", env) {
        env.patches.push((id, value));
        return;
    }
    if let Some((id, value)) = dom_assignment(stmt, "textContent", env) {
        env.patches.push((id, escape_html(&value)));
        return;
    }
    if let Some(eq) = stmt.find('=') {
        let name = stmt[..eq].trim();
        if is_ident(name) {
            env.set(name, eval_expr(&stmt[eq + 1..], env));
        }
    }
}

fn dom_assignment(stmt: &str, prop: &str, env: &JsEnv) -> Option<(String, String)> {
    let prefix = "document.getElementById";
    let args = call_args(stmt, prefix)?;
    let close = stmt.find(')')?;
    let after = stmt[close + 1..].trim_start();
    let want = [".", prop, "="].concat();
    if !after.starts_with(&want) {
        return None;
    }
    Some((eval_expr(args, env), eval_expr(&after[want.len()..], env)))
}

fn call_args<'a>(stmt: &'a str, name: &str) -> Option<&'a str> {
    let p = stmt.find(name)? + name.len();
    let tail = stmt[p..].trim_start();
    if !tail.starts_with('(') {
        return None;
    }
    let start = p + stmt[p..].find('(')? + 1;
    let end = matching_paren(stmt, start - 1)?;
    Some(&stmt[start..end])
}

fn matching_paren(s: &str, open: usize) -> Option<usize> {
    let b = s.as_bytes();
    let mut q = 0u8;
    let mut esc = false;
    let mut depth = 0usize;
    let mut i = open;
    while i < b.len() {
        let c = b[i];
        if q != 0 {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == q {
                q = 0;
            }
        } else if c == b'\'' || c == b'"' {
            q = c;
        } else if c == b'(' {
            depth += 1;
        } else if c == b')' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn eval_expr(expr: &str, env: &JsEnv) -> String {
    let mut out = String::new();
    for part in split_plus(expr) {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        if let Some(s) = string_lit(p) {
            out.push_str(&s);
        } else if is_ident(p) {
            out.push_str(&env.get(p));
        } else if p.chars().all(|c| c.is_ascii_digit()) {
            out.push_str(p);
        }
    }
    out
}

fn split_statements(src: &str) -> Vec<&str> {
    split_top_level(src, ';')
}

fn split_plus(src: &str) -> Vec<&str> {
    split_top_level(src, '+')
}

fn split_top_level(src: &str, sep: char) -> Vec<&str> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut q = 0u8;
    let mut esc = false;
    let mut start = 0usize;
    for (i, &c) in b.iter().enumerate() {
        if q != 0 {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == q {
                q = 0;
            }
        } else if c == b'\'' || c == b'"' || c == b'`' {
            q = c;
        } else if c == sep as u8 {
            out.push(&src[start..i]);
            start = i + 1;
        }
    }
    out.push(&src[start..]);
    out
}

fn string_lit(s: &str) -> Option<String> {
    let b = s.as_bytes();
    if b.len() < 2 || !matches!(b[0], b'\'' | b'"' | b'`') || b.last() != Some(&b[0]) {
        return None;
    }
    let mut out = String::new();
    let mut i = 1usize;
    while i + 1 < b.len() {
        if b[i] == b'\\' && i + 2 < b.len() {
            i += 1;
            out.push(match b[i] {
                b'n' => '\n',
                b'r' => '\r',
                b't' => '\t',
                b'\'' => '\'',
                b'"' => '"',
                b'\\' => '\\',
                x => x as char,
            });
        } else {
            out.push(b[i] as char);
        }
        i += 1;
    }
    Some(out)
}

fn element_inner_range_by_id(html: &[u8], id: &str) -> Option<(usize, usize)> {
    let mut pos = 0usize;
    while let Some(lt) = find_ci(html, b"<", pos) {
        if lt + 1 >= html.len() || html[lt + 1] == b'/' || html[lt + 1] == b'!' {
            pos = lt + 1;
            continue;
        }
        let gt = find_ci(html, b">", lt)?;
        let tag = core::str::from_utf8(&html[lt + 1..gt]).ok()?;
        if tag_has_id(tag, id) {
            let name = tag
                .split(|c: char| c.is_ascii_whitespace() || c == '/' || c == '>')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            let close = ["</", &name].concat();
            let end = find_ci(html, close.as_bytes(), gt + 1).unwrap_or(gt + 1);
            return Some((gt + 1, end));
        }
        pos = gt + 1;
    }
    None
}

fn tag_has_id(tag: &str, id: &str) -> bool {
    for part in tag.split(|c: char| c.is_ascii_whitespace()) {
        if let Some(v) = part.strip_prefix("id=") {
            let v = v.trim_matches(|c| c == '"' || c == '\'' || c == '/');
            if v == id {
                return true;
            }
        }
    }
    false
}

fn escape_html(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn starts_ci(hay: &[u8], needle: &[u8]) -> bool {
    hay.len() >= needle.len()
        && hay[..needle.len()]
            .iter()
            .zip(needle)
            .all(|(a, b)| a.to_ascii_lowercase() == b.to_ascii_lowercase())
}

fn find_ci(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from >= hay.len() {
        return None;
    }
    let mut i = from;
    while i + needle.len() <= hay.len() {
        let mut k = 0;
        while k < needle.len() && hay[i + k].to_ascii_lowercase() == needle[k].to_ascii_lowercase() {
            k += 1;
        }
        if k == needle.len() {
            return Some(i);
        }
        i += 1;
    }
    None
}

pub fn selftest() -> Result<(), &'static str> {
    let html = br#"<div id="app">old</div><script>
        var who = 'Bouchaud';
        document.getElementById('app').innerHTML = '<b>' + who + '</b>';
        document.write('<p>OK</p>');
    </script>"#;
    let out = execute_inline(html);
    let s = core::str::from_utf8(&out).map_err(|_| "utf8")?;
    if !s.contains("<div id=\"app\"><b>Bouchaud</b></div>") {
        return Err("innerHTML");
    }
    if !s.contains("<p>OK</p>") || s.contains("<script>") {
        return Err("write");
    }
    Ok(())
}
