//! Moteur JavaScript embarque pour le navigateur graphique.
//!
//! Architecture volontairement interpretee et bornee (pas de JIT) : elle vise
//! le JavaScript utile au rendu de pages web dans Bouchaud Browser. Le moteur
//! gere un etat global, des fonctions, des conditions/boucles simples, les
//! expressions textuelles/numeriques les plus courantes et un DOM minimal
//! (`document.write`, `querySelector`, `getElementById`, `innerHTML`,
//! `textContent`). Les API dangereuses/asynchrones restent absentes tant que le
//! noyau n'a pas de boucle d'evenements navigateur complete.

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

#[derive(Clone)]
struct Function {
    name: String,
    args: Vec<String>,
    body: String,
}

#[derive(Default)]
struct JsEnv {
    vars: Vec<Var>,
    funcs: Vec<Function>,
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

    fn add_func(&mut self, f: Function) {
        if let Some(old) = self.funcs.iter_mut().find(|x| x.name == f.name) {
            *old = f;
            return;
        }
        if self.funcs.len() < 128 {
            self.funcs.push(f);
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
        let range = if id == "body" {
            element_inner_range_by_tag(out, "body")
        } else {
            element_inner_range_by_id(out, &id)
        };
        if let Some((a, b)) = range {
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
    let cleaned = strip_comments(src);
    let main = collect_functions(&cleaned, env);
    eval_block(&main, env);
}

fn eval_block(src: &str, env: &mut JsEnv) {
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
    if let Some(name) = stmt.strip_suffix("++") {
        let name = name.trim();
        if is_ident(name) {
            let v = to_num(&env.get(name)) + 1;
            env.set(name, v.to_string());
        }
        return;
    }
    if let Some(name) = stmt.strip_suffix("--") {
        let name = name.trim();
        if is_ident(name) {
            let v = to_num(&env.get(name)) - 1;
            env.set(name, v.to_string());
        }
        return;
    }
    if eval_if(stmt, env) || eval_for(stmt, env) {
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
    if let Some((name, rest)) = stmt.split_once("+=") {
        let name = name.trim();
        if is_ident(name) {
            let mut v = env.get(name);
            v.push_str(&eval_expr(rest, env));
            env.set(name, v);
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
    if let Some(value) = body_assignment(stmt, "innerHTML", env) {
        env.patches.push(("body".to_string(), value));
        return;
    }
    if let Some(value) = body_assignment(stmt, "textContent", env) {
        env.patches.push(("body".to_string(), escape_html(&value)));
        return;
    }
    if let Some((name, args)) = direct_call(stmt) {
        call_user_function_effect(&name, &args, env);
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
    let (args, close) = if let Some((a, c)) = call_args_pos(stmt, "document.getElementById") {
        (a, c)
    } else {
        let (selector, c) = call_args_pos(stmt, "document.querySelector")?;
        let selector = eval_expr(selector, env);
        let id = selector.strip_prefix('#')?;
        return dom_assignment_after(stmt, c, prop, id.to_string(), env);
    };
    dom_assignment_after(stmt, close, prop, eval_expr(args, env), env)
}

fn dom_assignment_after(
    stmt: &str,
    close: usize,
    prop: &str,
    id: String,
    env: &JsEnv,
) -> Option<(String, String)> {
    let after = stmt[close + 1..].trim_start();
    let want = [".", prop, "="].concat();
    if !after.starts_with(&want) {
        return None;
    }
    Some((id, eval_expr(&after[want.len()..], env)))
}

fn body_assignment(stmt: &str, prop: &str, env: &JsEnv) -> Option<String> {
    let want = ["document.body.", prop, "="].concat();
    let rest = stmt.trim().strip_prefix(&want)?;
    Some(eval_expr(rest, env))
}

fn call_args<'a>(stmt: &'a str, name: &str) -> Option<&'a str> {
    call_args_pos(stmt, name).map(|(args, _)| args)
}

fn call_args_pos<'a>(stmt: &'a str, name: &str) -> Option<(&'a str, usize)> {
    let p = stmt.find(name)? + name.len();
    let tail = stmt[p..].trim_start();
    if !tail.starts_with('(') {
        return None;
    }
    let start = p + stmt[p..].find('(')? + 1;
    let end = matching_paren(stmt, start - 1)?;
    Some((&stmt[start..end], end))
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
        } else if let Some((name, args)) = direct_call(p) {
            out.push_str(&call_user_function(&name, &args, env));
        } else if is_ident(p) {
            out.push_str(&env.get(p));
        } else if p == "true" || p == "false" || p == "null" || p == "undefined" {
            out.push_str(p);
        } else if p.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-') {
            out.push_str(p);
        }
    }
    out
}

fn strip_comments(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = String::new();
    let mut i = 0usize;
    let mut q = 0u8;
    let mut esc = false;
    while i < b.len() {
        let c = b[i];
        if q != 0 {
            out.push(c as char);
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == q {
                q = 0;
            }
            i += 1;
            continue;
        }
        if c == b'\'' || c == b'"' || c == b'`' {
            q = c;
            out.push(c as char);
            i += 1;
        } else if c == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(b.len());
        } else {
            out.push(c as char);
            i += 1;
        }
    }
    out
}

fn collect_functions(src: &str, env: &mut JsEnv) -> String {
    let mut out = String::new();
    let mut pos = 0usize;
    while let Some(rel) = src[pos..].find("function") {
        let fpos = pos + rel;
        out.push_str(&src[pos..fpos]);
        let mut p = fpos + "function".len();
        while p < src.len() && src.as_bytes()[p].is_ascii_whitespace() {
            p += 1;
        }
        let ns = p;
        while p < src.len() {
            let c = src.as_bytes()[p] as char;
            if c == '_' || c.is_ascii_alphanumeric() {
                p += 1;
            } else {
                break;
            }
        }
        let name = src[ns..p].trim();
        while p < src.len() && src.as_bytes()[p].is_ascii_whitespace() {
            p += 1;
        }
        if name.is_empty() || p >= src.len() || src.as_bytes()[p] != b'(' {
            out.push_str("function");
            pos = fpos + "function".len();
            continue;
        }
        let args_end = match matching_paren(src, p) {
            Some(x) => x,
            None => break,
        };
        let args = src[p + 1..args_end]
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| is_ident(x))
            .collect();
        p = args_end + 1;
        while p < src.len() && src.as_bytes()[p].is_ascii_whitespace() {
            p += 1;
        }
        if p >= src.len() || src.as_bytes()[p] != b'{' {
            pos = p;
            continue;
        }
        let body_end = match matching_brace(src, p) {
            Some(x) => x,
            None => break,
        };
        env.add_func(Function {
            name: name.to_string(),
            args,
            body: src[p + 1..body_end].to_string(),
        });
        pos = body_end + 1;
    }
    out.push_str(&src[pos..]);
    out
}

fn eval_if(stmt: &str, env: &mut JsEnv) -> bool {
    let s = stmt.trim();
    if !s.starts_with("if") {
        return false;
    }
    let cond_open = match s.find('(') {
        Some(x) => x,
        None => return false,
    };
    let cond_close = match matching_paren(s, cond_open) {
        Some(x) => x,
        None => return false,
    };
    let cond = eval_condition(&s[cond_open + 1..cond_close], env);
    let rest = s[cond_close + 1..].trim_start();
    if !rest.starts_with('{') {
        return false;
    }
    let then_end = match matching_brace(rest, 0) {
        Some(x) => x,
        None => return false,
    };
    let then_body = &rest[1..then_end];
    let after = rest[then_end + 1..].trim_start();
    if cond {
        eval_block(then_body, env);
    } else if let Some(e) = after.strip_prefix("else") {
        let e = e.trim_start();
        if e.starts_with('{') {
            if let Some(end) = matching_brace(e, 0) {
                eval_block(&e[1..end], env);
            }
        }
    }
    true
}

fn eval_for(stmt: &str, env: &mut JsEnv) -> bool {
    let s = stmt.trim();
    if !s.starts_with("for") {
        return false;
    }
    let head_open = match s.find('(') {
        Some(x) => x,
        None => return false,
    };
    let head_close = match matching_paren(s, head_open) {
        Some(x) => x,
        None => return false,
    };
    let rest = s[head_close + 1..].trim_start();
    if !rest.starts_with('{') {
        return false;
    }
    let body_end = match matching_brace(rest, 0) {
        Some(x) => x,
        None => return false,
    };
    let parts = split_top_level(&s[head_open + 1..head_close], ';');
    if parts.len() != 3 {
        return false;
    }
    eval_statement(parts[0].trim(), env);
    let mut guard = 0usize;
    while eval_condition(parts[1], env) && guard < 10_000 && env.steps < MAX_STEPS {
        eval_block(&rest[1..body_end], env);
        eval_statement(parts[2].trim(), env);
        guard += 1;
    }
    true
}

fn eval_condition(expr: &str, env: &JsEnv) -> bool {
    let e = expr.trim();
    for op in ["===", "!==", "==", "!=", "<=", ">=", "<", ">"] {
        if let Some(p) = e.find(op) {
            let a = eval_expr(&e[..p], env);
            let b = eval_expr(&e[p + op.len()..], env);
            return match op {
                "===" | "==" => a == b,
                "!==" | "!=" => a != b,
                "<" => to_num(&a) < to_num(&b),
                ">" => to_num(&a) > to_num(&b),
                "<=" => to_num(&a) <= to_num(&b),
                ">=" => to_num(&a) >= to_num(&b),
                _ => false,
            };
        }
    }
    match eval_expr(e, env).as_str() {
        "" | "0" | "false" | "null" | "undefined" => false,
        _ => true,
    }
}

fn direct_call(stmt: &str) -> Option<(String, Vec<String>)> {
    let s = stmt.trim();
    let open = s.find('(')?;
    if !s.ends_with(')') {
        return None;
    }
    let name = s[..open].trim();
    if !is_ident(name) {
        return None;
    }
    let close = matching_paren(s, open)?;
    if close != s.len() - 1 {
        return None;
    }
    Some((
        name.to_string(),
        split_top_level(&s[open + 1..close], ',')
            .iter()
            .map(|x| x.trim().to_string())
            .collect(),
    ))
}

fn call_user_function(name: &str, args: &[String], env: &JsEnv) -> String {
    let f = match env.funcs.iter().find(|f| f.name == name) {
        Some(f) => f.clone(),
        None => return String::new(),
    };
    let mut child = JsEnv {
        vars: env.vars.clone(),
        funcs: env.funcs.clone(),
        writes: Vec::new(),
        patches: Vec::new(),
        steps: env.steps,
    };
    for (i, name) in f.args.iter().enumerate() {
        let v = args.get(i).map(|a| eval_expr(a, env)).unwrap_or_default();
        child.set(name, v);
    }
    for stmt in split_statements(&f.body) {
        let s = stmt.trim();
        if let Some(ret) = s.strip_prefix("return ") {
            return eval_expr(ret, &child);
        }
        eval_statement(s, &mut child);
    }
    String::new()
}

fn call_user_function_effect(name: &str, args: &[String], env: &mut JsEnv) -> String {
    let f = match env.funcs.iter().find(|f| f.name == name) {
        Some(f) => f.clone(),
        None => return String::new(),
    };
    let mut child = JsEnv {
        vars: env.vars.clone(),
        funcs: env.funcs.clone(),
        writes: Vec::new(),
        patches: Vec::new(),
        steps: env.steps,
    };
    for (i, name) in f.args.iter().enumerate() {
        let v = args.get(i).map(|a| eval_expr(a, env)).unwrap_or_default();
        child.set(name, v);
    }
    let mut ret = String::new();
    for stmt in split_statements(&f.body) {
        let s = stmt.trim();
        if let Some(r) = s.strip_prefix("return ") {
            ret = eval_expr(r, &child);
            break;
        }
        eval_statement(s, &mut child);
    }
    env.steps = child.steps;
    env.writes.extend(child.writes);
    env.patches.extend(child.patches);
    ret
}

fn matching_brace(s: &str, open: usize) -> Option<usize> {
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
        } else if c == b'\'' || c == b'"' || c == b'`' {
            q = c;
        } else if c == b'{' {
            depth += 1;
        } else if c == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn to_num(s: &str) -> i64 {
    let mut n = 0i64;
    let mut neg = false;
    for (i, c) in s.trim().chars().enumerate() {
        if i == 0 && c == '-' {
            neg = true;
        } else if c.is_ascii_digit() {
            n = n * 10 + (c as i64 - '0' as i64);
        } else {
            break;
        }
    }
    if neg { -n } else { n }
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
    let mut paren = 0usize;
    let mut brace = 0usize;
    let mut bracket = 0usize;
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
        } else if c == b'(' {
            paren += 1;
        } else if c == b')' {
            paren = paren.saturating_sub(1);
        } else if c == b'{' {
            brace += 1;
        } else if c == b'}' {
            brace = brace.saturating_sub(1);
        } else if c == b'[' {
            bracket += 1;
        } else if c == b']' {
            bracket = bracket.saturating_sub(1);
        } else if c == sep as u8 && paren == 0 && brace == 0 && bracket == 0 {
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

fn element_inner_range_by_tag(html: &[u8], wanted: &str) -> Option<(usize, usize)> {
    let mut pos = 0usize;
    while let Some(lt) = find_ci(html, b"<", pos) {
        if lt + 1 >= html.len() || html[lt + 1] == b'/' || html[lt + 1] == b'!' {
            pos = lt + 1;
            continue;
        }
        let gt = find_ci(html, b">", lt)?;
        let tag = core::str::from_utf8(&html[lt + 1..gt]).ok()?;
        let name = tag
            .split(|c: char| c.is_ascii_whitespace() || c == '/' || c == '>')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if name == wanted {
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
        function strong(x) { return '<b>' + x + '</b>'; }
        document.querySelector('#app').innerHTML = strong(who);
        var list = '';
        for (var i = 0; i < 3; i++) { list += '<li>' + i + '</li>'; }
        document.write('<ul>' + list + '</ul>');
    </script>"#;
    let out = execute_inline(html);
    let s = core::str::from_utf8(&out).map_err(|_| "utf8")?;
    if !s.contains("<div id=\"app\"><b>Bouchaud</b></div>") {
        return Err("innerHTML");
    }
    if !s.contains("<li>0</li><li>1</li><li>2</li>") || s.contains("<script>") {
        return Err("write/for");
    }
    Ok(())
}
