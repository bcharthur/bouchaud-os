//! Evaluateur d'expressions arithmetiques.
//!
//! Sert la calculatrice du bureau. C'est un analyseur par descente recursive
//! sur la grammaire habituelle des priorites :
//!
//! ```text
//! expression := terme     (('+' | '-') terme)*
//! terme      := unaire    (('*' | '/' | '%') unaire)*
//! unaire     := ('+' | '-')* facteur
//! facteur    := nombre | '(' expression ')'
//! ```
//!
//! Auparavant, ce calcul passait par l'interpreteur JavaScript du navigateur
//! embarque. Celui-ci portait tout un DOM et une cascade CSS pour additionner
//! deux nombres — et il est parti avec le navigateur, qui vit maintenant en
//! ring 3. Cinquante lignes suffisent, et elles ne dependent de rien.

use alloc::string::{String, ToString};

/// Evalue une expression et rend le resultat formate.
pub fn eval(source: &str) -> Result<String, &'static str> {
    let octets: alloc::vec::Vec<u8> = source.bytes().filter(|b| *b != b' ').collect();
    let mut position = 0usize;
    let valeur = expression(&octets, &mut position)?;
    if position != octets.len() {
        return Err("caractere inattendu");
    }
    Ok(formate(valeur))
}

/// Rend un nombre sans decimales inutiles : `4` et non `4.0000000`.
fn formate(valeur: f64) -> String {
    if !valeur.is_finite() {
        return "Erreur".to_string();
    }
    if valeur == libm_trunc(valeur) && valeur.abs() < 1e15 {
        return alloc::format!("{}", valeur as i64);
    }
    // Six decimales, puis on enleve les zeros de queue.
    let mut texte = alloc::format!("{:.6}", valeur);
    while texte.ends_with('0') {
        texte.pop();
    }
    if texte.ends_with('.') {
        texte.pop();
    }
    texte
}

/// `trunc` sans bibliotheque mathematique : le noyau est `no_std`.
fn libm_trunc(valeur: f64) -> f64 {
    if valeur.abs() >= 9.007_199_254_740_992e15 {
        return valeur;
    }
    valeur as i64 as f64
}

fn expression(octets: &[u8], position: &mut usize) -> Result<f64, &'static str> {
    let mut valeur = terme(octets, position)?;
    while *position < octets.len() {
        match octets[*position] {
            b'+' => { *position += 1; valeur += terme(octets, position)?; }
            b'-' => { *position += 1; valeur -= terme(octets, position)?; }
            _ => break,
        }
    }
    Ok(valeur)
}

fn terme(octets: &[u8], position: &mut usize) -> Result<f64, &'static str> {
    let mut valeur = unaire(octets, position)?;
    while *position < octets.len() {
        match octets[*position] {
            b'*' => { *position += 1; valeur *= unaire(octets, position)?; }
            b'/' => {
                *position += 1;
                let diviseur = unaire(octets, position)?;
                if diviseur == 0.0 {
                    return Err("division par zero");
                }
                valeur /= diviseur;
            }
            b'%' => {
                *position += 1;
                let diviseur = unaire(octets, position)?;
                if diviseur == 0.0 {
                    return Err("division par zero");
                }
                valeur -= libm_trunc(valeur / diviseur) * diviseur;
            }
            _ => break,
        }
    }
    Ok(valeur)
}

fn unaire(octets: &[u8], position: &mut usize) -> Result<f64, &'static str> {
    if *position < octets.len() {
        if octets[*position] == b'-' {
            *position += 1;
            return Ok(-unaire(octets, position)?);
        }
        if octets[*position] == b'+' {
            *position += 1;
            return unaire(octets, position);
        }
    }
    facteur(octets, position)
}

fn facteur(octets: &[u8], position: &mut usize) -> Result<f64, &'static str> {
    if *position >= octets.len() {
        return Err("expression incomplete");
    }
    if octets[*position] == b'(' {
        *position += 1;
        let valeur = expression(octets, position)?;
        if *position >= octets.len() || octets[*position] != b')' {
            return Err("parenthese non fermee");
        }
        *position += 1;
        return Ok(valeur);
    }
    nombre(octets, position)
}

fn nombre(octets: &[u8], position: &mut usize) -> Result<f64, &'static str> {
    let debut = *position;
    while *position < octets.len() && (octets[*position].is_ascii_digit() || octets[*position] == b'.') {
        *position += 1;
    }
    if debut == *position {
        return Err("nombre attendu");
    }
    let texte = core::str::from_utf8(&octets[debut..*position]).map_err(|_| "nombre illisible")?;
    texte.parse::<f64>().map_err(|_| "nombre illisible")
}

/// Autotest : quelques expressions dont le resultat est connu.
pub fn selftest() -> Result<(), &'static str> {
    let cas: &[(&str, &str)] = &[
        ("2+3*4", "14"),
        ("(2+3)*4", "20"),
        ("-5+2", "-3"),
        ("7/2", "3.5"),
        ("10%3", "1"),
        ("2*(3+4)-1", "13"),
    ];
    for (source, attendu) in cas {
        match eval(source) {
            Ok(obtenu) if obtenu == *attendu => {}
            _ => return Err("resultat inattendu"),
        }
    }
    if eval("1/0").is_ok() {
        return Err("la division par zero aurait du echouer");
    }
    if eval("2+").is_ok() {
        return Err("une expression incomplete aurait du echouer");
    }
    Ok(())
}
