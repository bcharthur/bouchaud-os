//! URL et location : resolution d'URL relatives, decomposition en composants
//! (protocol/host/pathname/search/hash), objets location, URL et
//! URLSearchParams exposes au JavaScript.

use super::*;

/// Resout une URL contre une URL de base COMPLETE (pas seulement scheme+host) :
/// gere absolu, `//host/...`, `/racine`, `./relatif`, `../parent` — necessaire
/// pour les imports de modules ES (`import x from "./chunk.js"`).
pub(super) fn resolve_url(base: &str, rel: &str) -> String {
    if rel.starts_with("http://") || rel.starts_with("https://") { return rel.to_string(); }
    let scheme = scheme_of(base);
    if let Some(rest) = rel.strip_prefix("//") { return format!("{}://{}", scheme, rest); }
    let host = host_of(base);
    if rel.starts_with('/') { return format!("{}://{}{}", scheme, host, rel); }
    // Repertoire du document/module de base (sans query ni fragment).
    let after = base.strip_prefix("https://").or_else(|| base.strip_prefix("http://")).unwrap_or(base);
    let path = match after.find('/') { Some(i) => &after[i..], None => "/" };
    let path = path.split(['?', '#']).next().unwrap_or("/");
    let dir = match path.rfind('/') { Some(i) => &path[..i + 1], None => "/" };
    // Normalisation ./ et ../ segment par segment.
    let mut segs: Vec<&str> = Vec::new();
    for s in dir.split('/').chain(rel.split('/')) {
        match s { "" | "." => {} ".." => { segs.pop(); } x => segs.push(x) }
    }
    format!("{}://{}/{}", scheme, host, segs.join("/"))
}

/// Decompose une URL absolue en composants `location` :
/// (protocol, hostname, port, host, pathname, search, hash).
pub(super) fn split_url(u: &str) -> (String, String, String, String, String, String, String) {
    let (scheme, rest) = match u.find("://") { Some(p) => (&u[..p], &u[p + 3..]), None => ("https", u) };
    let cut = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (hostport, pathrest) = (&rest[..cut], &rest[cut..]);
    let (hostname, port) = match hostport.rfind(':') {
        Some(p) if !hostport[p + 1..].is_empty() && hostport[p + 1..].bytes().all(|c| c.is_ascii_digit()) =>
            (&hostport[..p], &hostport[p + 1..]),
        _ => (hostport, ""),
    };
    let (before_hash, hash) = match pathrest.find('#') { Some(p) => (&pathrest[..p], &pathrest[p..]), None => (pathrest, "") };
    let (path, search) = match before_hash.find('?') { Some(p) => (&before_hash[..p], &before_hash[p..]), None => (before_hash, "") };
    let pathname = if path.is_empty() { "/" } else { path };
    (format!("{}:", scheme), hostname.to_string(), port.to_string(), hostport.to_string(),
     pathname.to_string(), search.to_string(), hash.to_string())
}

/// Construit un objet `location`/`URL` complet (href, protocol, host, hostname,
/// port, pathname, search, hash, origin + reload/assign/replace/toString).
pub(super) fn make_location_obj(href: &str) -> Value {
    let loc = new_obj(Obj::plain());
    let (protocol, hostname, port, host, pathname, search, hash) = split_url(href);
    let origin = format!("{}//{}", protocol, host);
    set(&loc, "href", str_val(href.to_string()));
    set(&loc, "protocol", str_val(protocol));
    set(&loc, "host", str_val(host));
    set(&loc, "hostname", str_val(hostname));
    set(&loc, "port", str_val(port));
    set(&loc, "pathname", str_val(pathname));
    set(&loc, "search", str_val(search));
    set(&loc, "hash", str_val(hash));
    set(&loc, "origin", str_val(origin));
    set(&loc, "reload", native_val(|_it, _t, _a| Ok(Value::Undefined)));
    set(&loc, "assign", native_val(|_it, _t, _a| Ok(Value::Undefined)));
    set(&loc, "replace", native_val(|_it, _t, _a| Ok(Value::Undefined)));
    set(&loc, "toString", native_val(|it, t, _a| { let h = it.get_prop(&t, "href")?; Ok(str_val(it.to_string(&h))) }));
    loc
}

/// Paires (cle, valeur) d'un URLSearchParams (stockees dans `__pairs`).
pub(super) fn usp_pairs(v: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Value::Obj(o) = v {
        if let Some(Value::Obj(p)) = o.borrow().props.get("__pairs") {
            if let Some(arr) = &p.borrow().arr {
                for kv in arr {
                    if let Value::Obj(kvo) = kv {
                        if let Some(a) = &kvo.borrow().arr {
                            let k = if let Some(Value::Str(s)) = a.get(0) { (**s).clone() } else { String::new() };
                            let val = if let Some(Value::Str(s)) = a.get(1) { (**s).clone() } else { String::new() };
                            out.push((k, val));
                        }
                    }
                }
            }
        }
    }
    out
}
pub(super) fn usp_store(v: &Value, pairs: Vec<(String, String)>) {
    let arr: Vec<Value> = pairs.into_iter().map(|(k, val)| array_val(vec![str_val(k), str_val(val)])).collect();
    set(v, "__pairs", array_val(arr));
}

/// Objet URLSearchParams : parse `?a=b&c=d`, expose get/getAll/has/set/append/
/// delete/forEach/toString (suffisant pour location.search et les SERP).
pub(super) fn make_url_search_params(init: &str) -> Value {
    let s = init.strip_prefix('?').unwrap_or(init);
    let mut pairs: Vec<(String, String)> = Vec::new();
    for kv in s.split('&').filter(|x| !x.is_empty()) {
        let mut parts = kv.splitn(2, '=');
        let k = uri_decode(&parts.next().unwrap_or("").replace('+', " "));
        let v = uri_decode(&parts.next().unwrap_or("").replace('+', " "));
        pairs.push((k, v));
    }
    let o = new_obj(Obj::plain());
    usp_store(&o, pairs);
    set(&o, "get", native_val(|it, t, a| {
        let k = it.to_string(a.get(0).unwrap_or(&Value::Undefined));
        Ok(usp_pairs(&t).into_iter().find(|(pk, _)| pk == &k).map(|(_, v)| str_val(v)).unwrap_or(Value::Null))
    }));
    set(&o, "getAll", native_val(|it, t, a| {
        let k = it.to_string(a.get(0).unwrap_or(&Value::Undefined));
        Ok(array_val(usp_pairs(&t).into_iter().filter(|(pk, _)| pk == &k).map(|(_, v)| str_val(v)).collect()))
    }));
    set(&o, "has", native_val(|it, t, a| {
        let k = it.to_string(a.get(0).unwrap_or(&Value::Undefined));
        Ok(Value::Bool(usp_pairs(&t).iter().any(|(pk, _)| pk == &k)))
    }));
    set(&o, "set", native_val(|it, t, a| {
        let k = it.to_string(a.get(0).unwrap_or(&Value::Undefined));
        let v = it.to_string(a.get(1).unwrap_or(&Value::Undefined));
        let mut pairs: Vec<(String, String)> = usp_pairs(&t).into_iter().filter(|(pk, _)| pk != &k).collect();
        pairs.push((k, v));
        usp_store(&t, pairs);
        Ok(Value::Undefined)
    }));
    set(&o, "append", native_val(|it, t, a| {
        let k = it.to_string(a.get(0).unwrap_or(&Value::Undefined));
        let v = it.to_string(a.get(1).unwrap_or(&Value::Undefined));
        let mut pairs = usp_pairs(&t);
        pairs.push((k, v));
        usp_store(&t, pairs);
        Ok(Value::Undefined)
    }));
    set(&o, "delete", native_val(|it, t, a| {
        let k = it.to_string(a.get(0).unwrap_or(&Value::Undefined));
        let pairs: Vec<(String, String)> = usp_pairs(&t).into_iter().filter(|(pk, _)| pk != &k).collect();
        usp_store(&t, pairs);
        Ok(Value::Undefined)
    }));
    set(&o, "forEach", native_val(|it, t, a| {
        let cb = a.get(0).cloned().unwrap_or(Value::Undefined);
        for (k, v) in usp_pairs(&t) { it.call(cb.clone(), Value::Undefined, &[str_val(v), str_val(k)])?; }
        Ok(Value::Undefined)
    }));
    set(&o, "toString", native_val(|_it, t, _a| {
        let mut s = String::new();
        for (i, (k, v)) in usp_pairs(&t).into_iter().enumerate() {
            if i > 0 { s.push('&'); }
            s.push_str(&uri_encode_component(&k)); s.push('='); s.push_str(&uri_encode_component(&v));
        }
        Ok(str_val(s))
    }));
    o
}

/// Encodage x-www-form-urlencoded minimal (toString d'URLSearchParams).
pub(super) fn uri_encode_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => { out.push('%'); out.push_str(&format!("{:02X}", b)); }
        }
    }
    out
}
