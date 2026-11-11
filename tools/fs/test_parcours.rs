//! BOUCHAUD_FIND_PARCOURS_V1 : le parcours de `find` (`src/fs/parcours.rs`,
//! inclus tel quel).
//!
//! Ce qui est cherche : qu'aucune arborescence -- profonde, large, corrompue,
//! interdite -- ne fasse boucler, deborder la pile ou fuir ce qu'on ne peut
//! pas lire ; et que la sortie reste bornee.
//!
//! Lance par `tools/ci/run_host_tests.sh`.

#![allow(dead_code)]
extern crate alloc;

#[path = "../../src/fs/parcours.rs"]
mod parcours;

use parcours::*;

#[derive(Clone)]
struct N {
    utilise: bool,
    parent: usize,
    dossier: bool,
    nom: String,
    lisible: bool,
}

struct Faux(Vec<N>);

impl Faux {
    /// Racine seule (indice 0).
    fn neuf() -> Self {
        Faux(vec![N { utilise: true, parent: 0, dossier: true, nom: String::new(), lisible: true }])
    }
    fn ajoute(&mut self, parent: usize, nom: &str, dossier: bool) -> usize {
        self.0.push(N { utilise: true, parent, dossier, nom: nom.to_string(), lisible: true });
        self.0.len() - 1
    }
}

impl Arbre for Faux {
    fn taille(&self) -> usize { self.0.len() }
    fn utilise(&self, i: usize) -> bool { self.0[i].utilise }
    fn parent(&self, i: usize) -> usize { self.0[i].parent }
    fn dossier(&self, i: usize) -> bool { self.0[i].dossier }
    fn nom(&self, i: usize) -> &str { &self.0[i].nom }
    fn lisible(&self, i: usize) -> bool { self.0[i].lisible }
}

const LARGE: usize = 1 << 20;

fn etc() -> Faux {
    let mut f = Faux::neuf();
    let etc = f.ajoute(0, "etc", true);
    f.ajoute(etc, "hosts", false);
    f.ajoute(etc, "passwd", false);
    let ssl = f.ajoute(etc, "ssl", true);
    f.ajoute(ssl, "certs", true);
    let home = f.ajoute(0, "home", true);
    f.ajoute(home, "arthur", true);
    f
}

#[test]
fn prefixe_et_ordre_de_l_ancien_find() {
    let f = etc();
    let r = parcours(&f, 0, None, LARGE);
    assert_eq!(r.chemins, ["/etc", "/etc/hosts", "/etc/passwd", "/etc/ssl", "/etc/ssl/certs", "/home", "/home/arthur"]);
    assert!(r.refus.is_empty() && !r.tronque);
    let r = parcours(&f, 1, None, LARGE);
    assert_eq!(r.chemins, ["/etc/hosts", "/etc/passwd", "/etc/ssl", "/etc/ssl/certs"], "depart exclu");
}

#[test]
fn filtre_par_sous_chaine() {
    let r = parcours(&etc(), 0, Some("ss"), LARGE);
    assert_eq!(r.chemins, ["/etc/passwd", "/etc/ssl"]);
}

#[test]
fn fichier_ou_noeud_absent_comme_depart() {
    let mut f = etc();
    assert!(parcours(&f, 2, None, LARGE).chemins.is_empty(), "un fichier n'a pas de descendants");
    f.0[1].utilise = false;
    assert_eq!(parcours(&f, 1, None, LARGE), Resultat::default(), "noeud supprime");
    assert_eq!(parcours(&f, 999, None, LARGE), Resultat::default(), "indice hors bornes");
}

#[test]
fn arbre_tres_profond_sans_recursion() {
    // 100 000 niveaux : l'ancienne recursion aurait epuise la pile noyau
    // bien avant. Ici, une pile explicite sur le tas.
    let mut f = Faux::neuf();
    let mut p = 0;
    for _ in 0..100_000 {
        p = f.ajoute(p, "d", true);
    }
    let r = parcours(&f, 0, Some("zzz"), LARGE);
    assert_eq!(r.visites, 100_000);
    assert!(r.chemins.is_empty());
}

#[test]
fn arbre_large() {
    let mut f = Faux::neuf();
    let d = f.ajoute(0, "large", true);
    for k in 0..50_000 {
        f.ajoute(d, &format!("f{k}"), false);
    }
    let r = parcours(&f, d, None, LARGE * 8);
    assert_eq!(r.chemins.len(), 50_000);
    assert_eq!(r.chemins[0], "/large/f0");
    assert_eq!(r.chemins[49_999], "/large/f49999");
}

#[test]
fn cycle_de_parents_corrompus_ne_boucle_pas() {
    let mut f = etc();
    // /etc/ssl et /etc/ssl/certs se designent l'un l'autre : plus aucun ne
    // remonte a la racine, mais `ssl` reste enfant de... `certs`.
    let ssl = 4;
    let certs = 5;
    f.0[ssl].parent = certs;
    f.0[certs].parent = ssl;
    let r = parcours(&f, 0, None, LARGE);
    assert!(r.visites <= f.taille());
    assert!(r.chemins.iter().all(|c| !c.contains("ssl")), "le cycle est detache de l'arbre : {:?}", r.chemins);
    // Partir DANS le cycle : termine, chemins marques.
    let r = parcours(&f, ssl, None, LARGE);
    assert!(r.visites <= f.taille());
    assert!(r.chemins.iter().all(|c| c == "/?"));
    // Un noeud qui est son propre parent.
    let mut g = etc();
    g.0[2].parent = 2;
    let r = parcours(&g, 0, None, LARGE);
    assert!(!r.chemins.contains(&"/etc/hosts".to_string()));
}

#[test]
fn repertoire_illisible_signale_et_non_parcouru() {
    let mut f = etc();
    let arthur = 7;
    f.ajoute(arthur, "secret.txt", false);
    f.0[arthur].lisible = false;
    let r = parcours(&f, 0, None, LARGE);
    assert!(r.chemins.contains(&"/home/arthur".to_string()), "le repertoire lui-meme est liste par son parent");
    assert!(!r.chemins.iter().any(|c| c.contains("secret")), "son contenu ne fuit pas");
    assert_eq!(r.refus, ["/home/arthur"]);
    // Depart illisible.
    let r = parcours(&f, arthur, None, LARGE);
    assert!(r.chemins.is_empty());
    assert_eq!(r.refus, ["/home/arthur"]);
}

#[test]
fn sortie_bornee() {
    let mut f = Faux::neuf();
    let d = f.ajoute(0, "d", true);
    for k in 0..1000 {
        f.ajoute(d, &format!("{k:04}"), false);
    }
    let r = parcours(&f, 0, None, 100);
    assert!(r.tronque);
    let octets: usize = r.chemins.iter().map(|c| c.len() + 1).sum();
    assert!(octets <= 100, "{octets}");
}

#[test]
fn noms_inhabituels_et_longs() {
    let mut f = Faux::neuf();
    let long = "x".repeat(255);
    let a = f.ajoute(0, "espace et accents éà", true);
    let b = f.ajoute(a, &long, true);
    f.ajoute(b, "-n", false);
    let r = parcours(&f, 0, None, LARGE);
    assert_eq!(r.chemins[2], format!("/espace et accents éà/{long}/-n"));
}

#[test]
fn chemin_de_la_racine_et_d_un_noeud() {
    let f = etc();
    assert_eq!(chemin(&f, 0), "/");
    assert_eq!(chemin(&f, 5), "/etc/ssl/certs");
}
