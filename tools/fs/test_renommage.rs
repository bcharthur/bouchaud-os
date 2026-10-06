//! La decision de renommage (POSIX `rename`, Linux `renameat2`).
//!
//! BOUCHAUD_RENOMMAGE_POSIX_V1
//!
//! Compile `src/fs/renommage.rs` -- le code meme que le noyau execute -- et
//! verifie :
//!
//!   R1 chaque regle d'erreur, une a une, avec son errno ;
//!   R2 l'ordre des controles quand plusieurs s'appliquent (celui de Linux) ;
//!   R3 l'action rendue (rien / deplacer / remplacer / echanger) ;
//!   R4 sur un petit arbre modele qui applique l'action comme le noyau, les
//!      scenarios reels : le fichier temporaire de `FileDownloader` renomme a
//!      sa place, un remplacement qui ne laisse JAMAIS deux entrees du meme
//!      nom, l'echange, le refus de boucler un dossier dans lui-meme ;
//!   R5 exhaustivement sur toutes les combinaisons d'entree : jamais deux
//!      noms identiques dans un dossier apres un succes, jamais un dossier
//!      sous lui-meme.
//!
//! Lance par `tools/ci/run_host_tests.sh`.

#[path = "../../src/fs/renommage.rs"]
mod renommage;

use renommage::*;

fn fichier() -> Noeud {
    Noeud { genre: Genre::Fichier, racine: false, disque: false, vide: true }
}
fn dossier(vide: bool) -> Noeud {
    Noeud { genre: Genre::Dossier, racine: false, disque: false, vide }
}
fn demande(source: Option<Noeud>, cible: Option<Noeud>) -> Demande {
    Demande {
        drapeaux: 0,
        source,
        parent_cible: Parent::Dossier,
        nom_cible: Nom::Valide,
        cible,
        meme_noeud: false,
        cible_sous_la_source: false,
        source_sous_la_cible: false,
    }
}

#[test]
fn r1_chaque_erreur_a_son_errno() {
    let mut d = demande(Some(fichier()), None);
    d.drapeaux = 8;
    assert_eq!(decide(&d), Err(EINVAL), "drapeau inconnu");
    d.drapeaux = RENAME_NOREPLACE | RENAME_EXCHANGE;
    assert_eq!(decide(&d), Err(EINVAL), "NOREPLACE + EXCHANGE");
    d.drapeaux = RENAME_WHITEOUT;
    assert_eq!(decide(&d), Err(EINVAL), "WHITEOUT non pris en charge");

    assert_eq!(decide(&demande(None, None)), Err(ENOENT), "source absente");
    let mut racine = dossier(false);
    racine.racine = true;
    assert_eq!(decide(&demande(Some(racine), None)), Err(EBUSY), "source racine");

    let mut d = demande(Some(fichier()), None);
    d.parent_cible = Parent::Absent;
    assert_eq!(decide(&d), Err(ENOENT), "parent cible absent");
    d.parent_cible = Parent::PasUnDossier;
    assert_eq!(decide(&d), Err(ENOTDIR), "parent cible pas un dossier");

    let mut d = demande(Some(fichier()), None);
    d.nom_cible = Nom::Invalide;
    assert_eq!(decide(&d), Err(EINVAL));
    d.nom_cible = Nom::TropLong;
    assert_eq!(decide(&d), Err(ENAMETOOLONG));

    let mut d = demande(Some(fichier()), Some(fichier()));
    d.drapeaux = RENAME_NOREPLACE;
    assert_eq!(decide(&d), Err(EEXIST), "NOREPLACE sur cible existante");

    let mut d = demande(Some(fichier()), None);
    d.drapeaux = RENAME_EXCHANGE;
    assert_eq!(decide(&d), Err(ENOENT), "EXCHANGE sans cible");

    assert_eq!(decide(&demande(Some(fichier()), Some(dossier(true)))), Err(EISDIR));
    assert_eq!(decide(&demande(Some(dossier(true)), Some(fichier()))), Err(ENOTDIR));
    assert_eq!(decide(&demande(Some(dossier(true)), Some(dossier(false)))), Err(ENOTEMPTY));

    let mut d = demande(Some(dossier(false)), None);
    d.cible_sous_la_source = true;
    assert_eq!(decide(&d), Err(EINVAL), "dossier dans son propre sous-arbre");

    let mut cible_racine = dossier(false);
    cible_racine.racine = true;
    assert_eq!(decide(&demande(Some(dossier(true)), Some(cible_racine))), Err(EBUSY));

    let mut disque = fichier();
    disque.disque = true;
    assert_eq!(decide(&demande(Some(disque), None)), Err(EROFS));
    assert_eq!(decide(&demande(Some(fichier()), Some(disque))), Err(EROFS));
}

#[test]
fn r2_ordre_des_controles() {
    // Linux : NOREPLACE se juge avant « meme fichier ».
    let mut d = demande(Some(fichier()), Some(fichier()));
    d.meme_noeud = true;
    d.drapeaux = RENAME_NOREPLACE;
    assert_eq!(decide(&d), Err(EEXIST));
    // Meme fichier : succes avant tout controle de type ou de disque.
    let mut disque = fichier();
    disque.disque = true;
    let mut d = demande(Some(disque), Some(disque));
    d.meme_noeud = true;
    assert_eq!(decide(&d), Ok(Action::Rien));
    // La boucle se juge avant le type de la cible.
    let mut d = demande(Some(dossier(true)), Some(fichier()));
    d.cible_sous_la_source = true;
    assert_eq!(decide(&d), Err(EINVAL));
    // Un fichier n'a pas de sous-arbre : l'indicateur ne compte pas.
    let mut d = demande(Some(fichier()), None);
    d.cible_sous_la_source = true;
    assert_eq!(decide(&d), Ok(Action::Deplacer));
}

#[test]
fn r3_actions() {
    assert_eq!(decide(&demande(Some(fichier()), None)), Ok(Action::Deplacer));
    assert_eq!(decide(&demande(Some(fichier()), Some(fichier()))), Ok(Action::Remplacer));
    assert_eq!(decide(&demande(Some(dossier(false)), Some(dossier(true)))), Ok(Action::Remplacer));
    let mut d = demande(Some(fichier()), Some(dossier(false)));
    d.drapeaux = RENAME_EXCHANGE;
    assert_eq!(decide(&d), Ok(Action::Echanger), "l'echange ignore les types");
    let mut d = demande(Some(dossier(false)), Some(dossier(false)));
    d.drapeaux = RENAME_EXCHANGE;
    d.source_sous_la_cible = true;
    assert_eq!(decide(&d), Err(EINVAL), "echange qui boucle");
    assert_eq!(classe_nom("", 255), Nom::Invalide);
    assert_eq!(classe_nom(".", 255), Nom::Invalide);
    assert_eq!(classe_nom("..", 255), Nom::Invalide);
    assert_eq!(classe_nom("a", 255), Nom::Valide);
    assert_eq!(classe_nom(&"x".repeat(256), 255), Nom::TropLong);
}

// ---------------------------------------------------------------------------
// R4/R5 : un arbre modele qui applique l'action comme le noyau.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct N {
    utilise: bool,
    dossier: bool,
    parent: usize,
    nom: String,
}

#[derive(Clone, Debug)]
struct Arbre {
    n: Vec<N>,
}

impl Arbre {
    fn neuf() -> Self {
        Arbre { n: vec![N { utilise: true, dossier: true, parent: 0, nom: String::new() }] }
    }
    fn ajoute(&mut self, parent: usize, nom: &str, dossier: bool) -> usize {
        self.n.push(N { utilise: true, dossier, parent, nom: nom.into() });
        self.n.len() - 1
    }
    fn enfant(&self, parent: usize, nom: &str) -> Option<usize> {
        (1..self.n.len()).find(|&i| self.n[i].utilise && self.n[i].parent == parent && self.n[i].nom == nom)
    }
    fn vide(&self, d: usize) -> bool {
        !(1..self.n.len()).any(|i| self.n[i].utilise && self.n[i].parent == d)
    }
    fn sous(&self, mut x: usize, ancetre: usize) -> bool {
        loop {
            if x == ancetre {
                return true;
            }
            if x == 0 {
                return false;
            }
            x = self.n[x].parent;
        }
    }
    fn noeud(&self, i: usize) -> Noeud {
        Noeud { genre: if self.n[i].dossier { Genre::Dossier } else { Genre::Fichier },
                racine: i == 0, disque: false, vide: self.vide(i) }
    }
    /// `rename(source, (parent, nom))`, comme `sys_renameat2`.
    fn renomme(&mut self, source: Option<usize>, parent: usize, nom: &str, drapeaux: u32) -> Result<Action, i64> {
        let cible = self.enfant(parent, nom);
        let d = Demande {
            drapeaux,
            source: source.map(|s| self.noeud(s)),
            parent_cible: if self.n[parent].dossier { Parent::Dossier } else { Parent::PasUnDossier },
            nom_cible: classe_nom(nom, 255),
            cible: cible.map(|c| self.noeud(c)),
            meme_noeud: source.is_some() && cible == source,
            cible_sous_la_source: source.map_or(false, |s| self.sous(parent, s)),
            source_sous_la_cible: match (source, cible) {
                (Some(s), Some(c)) => self.sous(self.n[s].parent, c),
                _ => false,
            },
        };
        let action = decide(&d)?;
        let s = source.unwrap();
        match action {
            Action::Rien => {}
            Action::Deplacer => {
                self.n[s].parent = parent;
                self.n[s].nom = nom.into();
            }
            Action::Remplacer => {
                let c = cible.unwrap();
                self.n[c].utilise = false;
                self.n[s].parent = parent;
                self.n[s].nom = nom.into();
            }
            Action::Echanger => {
                let c = cible.unwrap();
                let (p, n) = (self.n[s].parent, self.n[s].nom.clone());
                self.n[s].parent = self.n[c].parent;
                self.n[s].nom = self.n[c].nom.clone();
                self.n[c].parent = p;
                self.n[c].nom = n;
            }
        }
        Ok(action)
    }
    fn coherent(&self) -> Result<(), String> {
        for i in 1..self.n.len() {
            if !self.n[i].utilise {
                continue;
            }
            for j in (i + 1)..self.n.len() {
                if self.n[j].utilise && self.n[j].parent == self.n[i].parent && self.n[j].nom == self.n[i].nom {
                    return Err(format!("deux entrees `{}` sous {}", self.n[i].nom, self.n[i].parent));
                }
            }
            // Chaque noeud remonte a la racine sans boucle.
            let mut x = i;
            for _ in 0..=self.n.len() {
                if x == 0 {
                    break;
                }
                x = self.n[x].parent;
            }
            if x != 0 {
                return Err(format!("noeud {} hors de l'arbre (boucle)", i));
            }
            if !self.n[self.n[i].parent].utilise || !self.n[self.n[i].parent].dossier {
                return Err(format!("noeud {} sous un parent invalide", i));
            }
        }
        Ok(())
    }
}

#[test]
fn r4_le_fichier_temporaire_du_telechargement_prend_sa_place() {
    // `FileDownloader` : ecrire `rapport.pdf.part-7`, puis le renommer.
    let mut a = Arbre::neuf();
    let depot = a.ajoute(0, "Downloads", true);
    let ancien = a.ajoute(depot, "rapport.pdf", false);
    let tmp = a.ajoute(depot, "rapport.pdf.part-7", false);
    assert_eq!(a.renomme(Some(tmp), depot, "rapport.pdf", 0), Ok(Action::Remplacer));
    assert!(!a.n[ancien].utilise, "l'ancien fichier est remplace");
    assert_eq!(a.enfant(depot, "rapport.pdf"), Some(tmp));
    assert_eq!(a.enfant(depot, "rapport.pdf.part-7"), None);
    a.coherent().unwrap();
}

#[test]
fn r4_l_ancien_renommage_laissait_deux_entrees() {
    // Le comportement d'AVANT, rejoue : rattacher sans regarder la cible.
    let mut a = Arbre::neuf();
    let d = a.ajoute(0, "d", true);
    a.ajoute(d, "x", false);
    let y = a.ajoute(d, "y", false);
    a.n[y].nom = "x".into();
    assert!(a.coherent().is_err(), "le modele detecte bien le defaut d'origine");
}

#[test]
fn r4_echange_et_boucles() {
    let mut a = Arbre::neuf();
    let p = a.ajoute(0, "p", true);
    let q = a.ajoute(0, "q", true);
    let f = a.ajoute(p, "f", false);
    let g = a.ajoute(q, "g", false);
    assert_eq!(a.renomme(Some(f), q, "g", RENAME_EXCHANGE), Ok(Action::Echanger));
    assert_eq!(a.enfant(q, "g"), Some(f));
    assert_eq!(a.enfant(p, "f"), Some(g));
    a.coherent().unwrap();
    // `mv p p/sous` : refuse.
    let sous = a.ajoute(p, "sous", true);
    assert_eq!(a.renomme(Some(p), sous, "p", 0), Err(EINVAL));
    // `mv p p` : rien.
    assert_eq!(a.renomme(Some(p), 0, "p", 0), Ok(Action::Rien));
    a.coherent().unwrap();
}

#[test]
fn r5_exhaustif_aucun_succes_ne_corrompt_l_arbre() {
    // Un petit arbre fixe ; TOUTES les paires (source, parent cible, nom
    // cible) et tous les jeux de drapeaux.
    let mut base = Arbre::neuf();
    let a = base.ajoute(0, "a", true);
    let b = base.ajoute(a, "b", true);
    base.ajoute(b, "c", false);
    base.ajoute(0, "d", false);
    base.ajoute(a, "e", false);
    base.ajoute(0, "vide", true);
    let noms = ["a", "b", "c", "d", "e", "vide", "neuf", ".", ".."];
    let drapeaux = [0, RENAME_NOREPLACE, RENAME_EXCHANGE, RENAME_WHITEOUT, 3, 9];
    let mut succes = 0;
    let mut essais = 0;
    for source in 0..base.n.len() {
        for parent in 0..base.n.len() {
            for nom in noms {
                for &fl in &drapeaux {
                    let mut t = base.clone();
                    essais += 1;
                    if t.renomme(Some(source), parent, nom, fl).is_ok() {
                        succes += 1;
                        if let Err(e) = t.coherent() {
                            panic!("source={source} parent={parent} nom={nom} drapeaux={fl} : {e}");
                        }
                    }
                }
            }
        }
    }
    assert!(succes > 20 && succes < essais, "succes={succes} essais={essais}");
}
