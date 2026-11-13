//! HTTP(S) de l'agent : la pile TLS du depot (`bouchaud-tls`, les fichiers du
//! noyau), sur un socket Linux, en mode STRICT (BOUCHAUD_TLS_CONFIANCE_V1) :
//! chaine de confiance, nom couvert, certificat valide, a CHAQUE saut, avant
//! l'envoi de toute requete. Jamais de repli de https vers http. Une source
//! declaree `http` (miroir local) reste permise : la decision ne repose que
//! sur la signature du manifeste.
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::os::fd::AsRawFd;
use std::time::Duration;

use bouchaud_tls::handshake::{self, EtatFlux, Plateforme, Transport};

/// Sans un octet recu pendant ce delai, la requete echoue (le lien est
/// coupe) : la reprise de la partie decide de la suite.
const SANS_PROGRES_MS: i32 = 20_000;
const REDIRECTIONS_MAX: usize = 5;

/// Un socket TCP vu comme le transport du handshake.
pub struct Flux {
    s: TcpStream,
    rx: Vec<u8>,
    fin: bool,
    ferme: bool,
}

impl Flux {
    fn lit_un_peu(&mut self) -> bool {
        if self.fin || self.ferme { return false; }
        if !crate::natif::attend_lisible(self.s.as_raw_fd(), SANS_PROGRES_MS) {
            self.ferme = true;
            return false;
        }
        let mut b = [0u8; 16384];
        match self.s.read(&mut b) {
            Ok(0) => { self.fin = true; false }
            Ok(n) => { self.rx.extend_from_slice(&b[..n]); true }
            Err(_) => { self.ferme = true; false }
        }
    }
}

impl Transport for Flux {
    fn fill(&mut self, want: usize) -> bool {
        while self.rx.len() < want {
            if !self.lit_un_peu() { return false; }
        }
        true
    }
    fn rx(&self) -> &[u8] { &self.rx }
    fn take(&mut self, n: usize) -> Vec<u8> {
        let n = n.min(self.rx.len());
        self.rx.drain(..n).collect()
    }
    fn send(&mut self, data: &[u8]) -> bool {
        if self.s.write_all(data).is_err() { self.ferme = true; return false; }
        true
    }
    fn etat(&self) -> EtatFlux {
        EtatFlux { peer_fin: self.fin, closed: self.ferme, rst_seen: false, fin_seen: self.fin }
    }
    fn close(&mut self) {
        let _ = self.s.shutdown(std::net::Shutdown::Both);
        self.ferme = true;
    }
}

/// Une reponse HTTP : code, en-tetes (noms en minuscules), corps.
pub struct Reponse {
    pub statut: u16,
    pub entetes: Vec<(String, String)>,
    pub corps: Vec<u8>,
}

impl Reponse {
    fn entete(&self, nom: &str) -> Option<&str> {
        self.entetes.iter().find(|(k, _)| k == nom).map(|(_, v)| v.as_str())
    }
}

/// Pourquoi une requete n'a pas abouti.
#[derive(Debug)]
pub enum Echec {
    /// Le certificat n'a pas ete accepte : la ligne `TLS_REFUS ...`.
    Confiance(String),
    /// DNS, connexion, handshake, lecture : le transport.
    Transport(String),
    /// Reponse illisible ou redirection refusee.
    Protocole(String),
}

struct Url { https: bool, hote: String, port: u16, chemin: String }

fn decoupe(url: &str) -> Option<Url> {
    let (https, reste) = if let Some(r) = url.strip_prefix("https://") { (true, r) }
        else if let Some(r) = url.strip_prefix("http://") { (false, r) } else { return None };
    let (autorite, chemin) = match reste.find('/') { Some(i) => (&reste[..i], &reste[i..]), None => (reste, "/") };
    let (hote, port) = match autorite.rsplit_once(':') {
        Some((h, p)) => (h, p.parse().ok()?),
        None => (autorite, if https { 443 } else { 80 }),
    };
    if hote.is_empty() { return None; }
    Some(Url { https, hote: hote.to_string(), port, chemin: chemin.to_string() })
}

/// L'horloge de la validation X.509 : `clock_gettime(CLOCK_REALTIME)` (le
/// noyau la tient de la RTC), en AAAAMMJJhhmmss UTC.
pub fn maintenant_stamp() -> u64 {
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (a, mo, j, h, mi, se) = date_utc(s);
    bouchaud_tls::validate::stamp(a, mo, j, h, mi, se)
}

/// Secondes Unix -> (annee, mois, jour, heure, minute, seconde), UTC.
pub fn date_utc(s: u64) -> (u64, u64, u64, u64, u64, u64) {
    let jours = (s / 86_400) as i64;
    let r = s % 86_400;
    // Algorithme « civil from days » (H. Hinnant).
    let z = jours + 719_468;
    let ere = z.div_euclid(146_097);
    let doe = z - ere * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let jour = doy - (153 * mp + 2) / 5 + 1;
    let mois = if mp < 10 { mp + 3 } else { mp - 9 };
    let annee = yoe + ere * 400 + if mois <= 2 { 1 } else { 0 };
    (annee as u64, mois as u64, jour as u64, r / 3600, (r / 60) % 60, r % 60)
}

fn connecte(u: &Url) -> Result<Flux, Echec> {
    let adresse = (u.hote.as_str(), u.port).to_socket_addrs()
        .map_err(|e| Echec::Transport(format!("dns hote={} erreur={}", u.hote, e)))?
        .find(|a| a.is_ipv4())
        .ok_or_else(|| Echec::Transport(format!("dns hote={} aucune_adresse_ipv4", u.hote)))?;
    let s = TcpStream::connect_timeout(&adresse, Duration::from_secs(20))
        .map_err(|e| Echec::Transport(format!("connexion {}:{} erreur={}", u.hote, u.port, e)))?;
    let _ = s.set_nodelay(true);
    Ok(Flux { s, rx: Vec::new(), fin: false, ferme: false })
}

fn requete(u: &Url) -> String {
    format!("GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: bouchaud-ota/1\r\nAccept: */*\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n",
        u.chemin, u.hote)
}

/// Une requete, une connexion (`Connection: close`) : la fin du corps est
/// soit Content-Length, soit chunked, soit la fermeture.
fn une_requete(u: &Url, max: usize) -> Result<Reponse, Echec> {
    let mut flux = connecte(u)?;
    let brut = if u.https {
        let mut sess = handshake::connect(flux, &u.hote,
            &Plateforme { aleatoire: crate::natif::aleatoire, maintenant: maintenant_stamp() })
            .map_err(|e| Echec::Transport(format!("handshake hote={} erreur={}", u.hote, e)))?;
        let r = &sess.report;
        if !(r.trusted && r.hostname_ok && !r.expired) {
            let ligne = format!("TLS_REFUS confiance={} nom={} expire={} ({}, CN={})",
                r.trusted, r.hostname_ok, r.expired, r.detail, r.subject_cn);
            sess.close();
            return Err(Echec::Confiance(ligne));
        }
        if !sess.send_app(requete(u).as_bytes()) {
            return Err(Echec::Transport("envoi_requete".into()));
        }
        // Borne : en-tetes + corps maximal + marge de cadrage chunked.
        let r = sess.recv_http(max + 64 * 1024, http_complet);
        sess.close();
        r
    } else {
        if !flux.send(requete(u).as_bytes()) { return Err(Echec::Transport("envoi_requete".into())); }
        while !http_complet(&flux.rx) && flux.rx.len() <= max + 64 * 1024 {
            if !flux.lit_un_peu() { break; }
        }
        std::mem::take(&mut flux.rx)
    };
    if brut.is_empty() { return Err(Echec::Transport("reponse_vide".into())); }
    analyse(&brut).ok_or_else(|| Echec::Protocole("reponse_illisible".into()))
}

/// Une reponse HTTP/1.1 est-elle complete ?
pub fn http_complet(r: &[u8]) -> bool {
    let Some(fin) = fin_entetes(r) else { return false };
    let tete = String::from_utf8_lossy(&r[..fin]).to_ascii_lowercase();
    if let Some(n) = tete.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()) {
        return r.len() >= fin + 4 + n;
    }
    if tete.lines().any(|l| l.starts_with("transfer-encoding:") && l.contains("chunked")) {
        return dechunk(&r[fin + 4..]).map(|(_, complet)| complet).unwrap_or(false);
    }
    false
}

fn fin_entetes(r: &[u8]) -> Option<usize> {
    r.windows(4).position(|w| w == b"\r\n\r\n")
}

/// (corps, complet) d'un corps chunked ; None : mal forme.
fn dechunk(mut d: &[u8]) -> Option<(Vec<u8>, bool)> {
    let mut out = Vec::new();
    loop {
        let Some(eol) = d.windows(2).position(|w| w == b"\r\n") else { return Some((out, false)) };
        let taille = std::str::from_utf8(&d[..eol]).ok()?.split(';').next()?.trim();
        let n = usize::from_str_radix(taille, 16).ok()?;
        d = &d[eol + 2..];
        if n == 0 { return Some((out, true)); }
        if d.len() < n + 2 { out.extend_from_slice(&d[..d.len().min(n)]); return Some((out, false)); }
        out.extend_from_slice(&d[..n]);
        d = &d[n + 2..];
    }
}

fn analyse(brut: &[u8]) -> Option<Reponse> {
    let fin = fin_entetes(brut)?;
    let tete = std::str::from_utf8(&brut[..fin]).ok()?;
    let mut lignes = tete.split("\r\n");
    let statut: u16 = lignes.next()?.split_whitespace().nth(1)?.parse().ok()?;
    let entetes: Vec<(String, String)> = lignes
        .filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string())))
        .collect();
    let reste = &brut[fin + 4..];
    let chunked = entetes.iter().any(|(k, v)| k == "transfer-encoding" && v.to_ascii_lowercase().contains("chunked"));
    let corps = if chunked {
        dechunk(reste)?.0
    } else if let Some(n) = entetes.iter().find(|(k, _)| k == "content-length").and_then(|(_, v)| v.parse::<usize>().ok()) {
        // Un corps COURT (fermeture avant Content-Length) est rendu tel quel :
        // l'appelant le voit tronque et reprend la partie.
        reste[..reste.len().min(n)].to_vec()
    } else {
        reste.to_vec()
    };
    Some(Reponse { statut, entetes, corps })
}

/// GET avec redirections (chacune repasse par la confiance stricte ; https
/// vers http est refuse).
pub fn get(url: &str, max: usize) -> Result<Reponse, Echec> {
    let mut courante = url.to_string();
    for _ in 0..=REDIRECTIONS_MAX {
        let u = decoupe(&courante).ok_or_else(|| Echec::Protocole(format!("url_invalide {}", courante)))?;
        let r = une_requete(&u, max)?;
        if matches!(r.statut, 301 | 302 | 303 | 307 | 308) {
            let cible = r.entete("location").ok_or_else(|| Echec::Protocole("redirection_sans_location".into()))?.to_string();
            let suivante = if cible.starts_with("http://") || cible.starts_with("https://") {
                cible
            } else if cible.starts_with('/') {
                format!("{}://{}:{}{}", if u.https { "https" } else { "http" }, u.hote, u.port, cible)
            } else {
                return Err(Echec::Protocole("redirection_relative_non_geree".into()));
            };
            if u.https && suivante.starts_with("http://") {
                return Err(Echec::Protocole("redirection_https_vers_http_refusee".into()));
            }
            courante = suivante;
            continue;
        }
        return Ok(r);
    }
    Err(Echec::Protocole("trop_de_redirections".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_utc_connue() {
        assert_eq!(date_utc(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(date_utc(1_763_000_000), (2025, 11, 13, 2, 13, 20));
        assert_eq!(date_utc(951_782_400), (2000, 2, 29, 0, 0, 0));
    }

    #[test]
    fn cadrage_http() {
        let r = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nabcde";
        assert!(http_complet(r));
        assert!(!http_complet(&r[..r.len() - 1]));
        let c = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n";
        assert!(http_complet(c));
        assert_eq!(analyse(c).unwrap().corps, b"abcde");
        // Corps tronque : rendu court, jamais complete d'octets inventes.
        let t = b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabc";
        assert_eq!(analyse(t).unwrap().corps, b"abc");
    }

    #[test]
    fn urls() {
        let u = decoupe("https://10.0.2.2:8443/rel/manifest").unwrap();
        assert!(u.https && u.hote == "10.0.2.2" && u.port == 8443 && u.chemin == "/rel/manifest");
        let u = decoupe("https://github.com/o/r/releases/latest/download").unwrap();
        assert_eq!((u.port, u.chemin.as_str()), (443, "/o/r/releases/latest/download"));
        assert!(decoupe("ftp://x/y").is_none());
    }
}
