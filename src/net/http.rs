//! Construction de requete HTTP/1.0 et reperage du corps de reponse.

use alloc::format;
use alloc::string::String;

/// Construit une requete `GET` HTTP/1.0 (connexion fermee apres reponse).
pub fn build_get(host: &str, path: &str) -> String {
    format!(
        "GET {} HTTP/1.0\r\nHost: {}\r\nUser-Agent: BouchaudOS\r\nConnection: close\r\n\r\n",
        path, host
    )
}

/// Renvoie l'index du debut du corps (apres l'en-tete `\r\n\r\n`).
pub fn body_offset(resp: &[u8]) -> Option<usize> {
    if resp.len() < 4 { return None; }
    let mut i = 0;
    while i + 3 < resp.len() {
        if resp[i] == b'\r' && resp[i + 1] == b'\n' && resp[i + 2] == b'\r' && resp[i + 3] == b'\n' {
            return Some(i + 4);
        }
        i += 1;
    }
    None
}
