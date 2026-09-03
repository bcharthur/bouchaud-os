//! Preuve hote de l'arene DMA : ce que les pilotes rendent revient vraiment.
//!
//! Le module de production `src/kernel/memory/arene_dma.rs` est inclus tel
//! quel. Les adresses manipulees sont des adresses PHYSIQUES ; l'arene ne les
//! dereference jamais, ce qui la rend testable sans materiel.
//!
//! # Ce que l'arene precedente ne pouvait pas promettre
//!
//! Elle etait un pointeur qui monte, dans trois `static mut` sans verrou :
//!
//!   * rien ne se rendait -- un anneau reseau reinitialise perdait sa memoire
//!     jusqu'au redemarrage, et l'echec se manifestait plus tard sur un pilote
//!     qui n'avait rien fait de mal ;
//!   * deux pilotes s'initialisant en parallele pouvaient recevoir LA MEME
//!     adresse physique, et se marcher dessus dans un tampon que le materiel
//!     lit.

#[path = "../../src/kernel/memory/arene_dma.rs"]
mod arene_dma;

use arene_dma::{AreneDma, EtatDma, PAGE, REGIONS_MAX};
use std::collections::HashSet;
use std::sync::Arc;

const BASE: u64 = 0x1000_0000;

fn arene(pages: u64) -> AreneDma {
    let a = AreneDma::neuve();
    a.configure(BASE, BASE + pages * PAGE);
    a
}

// ---------------------------------------------------------------------------
// Ce qui existait deja : la frontiere.
// ---------------------------------------------------------------------------

#[test]
fn deux_allocations_ne_se_recouvrent_jamais() {
    let a = arene(16);
    let un = a.alloue(PAGE as usize).unwrap();
    let deux = a.alloue(PAGE as usize).unwrap();
    assert_ne!(un, deux);
    assert_eq!(deux, un + PAGE);
}

#[test]
fn une_allocation_est_arrondie_a_la_page() {
    let a = arene(16);
    let un = a.alloue(1).unwrap();
    let deux = a.alloue(1).unwrap();
    assert_eq!(deux - un, PAGE, "une demande d'un octet occupe une page entiere");
}

#[test]
fn une_arene_epuisee_echoue_au_lieu_de_deborder() {
    let a = arene(4);
    assert!(a.alloue(4 * PAGE as usize).is_some());
    assert!(a.alloue(PAGE as usize).is_none());
    assert_eq!(a.etat().echecs, 1);
}

#[test]
fn une_arene_non_configuree_ne_rend_rien() {
    let a = AreneDma::neuve();
    assert!(!a.configuree());
    assert!(a.alloue(PAGE as usize).is_none());
}

// ---------------------------------------------------------------------------
// CE QUI N'EXISTAIT PAS : rendre.
// ---------------------------------------------------------------------------

/// Le cas qui epuisait l'arene : un anneau rendu puis realloue.
#[test]
fn une_region_rendue_est_reutilisee() {
    let a = arene(64);
    let anneau = a.alloue(8 * PAGE as usize).unwrap();
    let apres = a.alloue(PAGE as usize).unwrap();

    a.libere(anneau, 8 * PAGE as usize);
    let reprise = a.alloue(8 * PAGE as usize).unwrap();

    assert_eq!(
        reprise, anneau,
        "la region rendue doit resservir : c'est tout l'objet du chantier"
    );
    assert_ne!(reprise, apres);
    assert_eq!(a.etat().reutilisations, 1);
}

/// Une arene qui alloue puis rend TOUT doit revenir a son etat initial.
///
/// C'est l'invariant qui distingue « rendre » de « comptabiliser une
/// liberation » : sans repli de la frontiere, l'arene se remplirait quand meme,
/// juste plus lentement.
#[test]
fn tout_rendre_ramene_l_arene_a_son_etat_initial() {
    let a = arene(64);
    let initial = a.etat();

    let mut allouees = Vec::new();
    for _ in 0..16 {
        allouees.push(a.alloue(2 * PAGE as usize).unwrap());
    }
    assert_eq!(a.etat().utilise, 32 * PAGE);

    // Rendues dans le desordre : le repli ne doit pas dependre de l'ordre.
    allouees.reverse();
    for (index, base) in allouees.iter().enumerate() {
        let _ = index;
        a.libere(*base, 2 * PAGE as usize);
    }

    let final_ = a.etat();
    assert_eq!(final_.utilise, 0, "toute la memoire rendue doit etre reutilisable");
    assert_eq!(final_.total, initial.total);
    assert_eq!(
        final_.regions, 0,
        "les regions adjacentes doivent avoir fusionne, puis replie la frontiere"
    );

    // Et l'arene entiere doit etre reallouable d'un seul bloc.
    assert!(a.alloue(64 * PAGE as usize).is_some());
}

/// Rendre la region du MILIEU de trois doit produire UNE region, pas deux
/// adjacentes que la prochaine allocation ne saurait pas reunir.
#[test]
fn rendre_le_milieu_recolle_les_deux_voisines() {
    let a = arene(64);
    let un = a.alloue(4 * PAGE as usize).unwrap();
    let deux = a.alloue(4 * PAGE as usize).unwrap();
    let trois = a.alloue(4 * PAGE as usize).unwrap();
    let garde = a.alloue(PAGE as usize).unwrap();
    assert_ne!(garde, 0);

    a.libere(un, 4 * PAGE as usize);
    a.libere(trois, 4 * PAGE as usize);
    assert_eq!(a.etat().regions, 2, "deux trous separes par la region du milieu");

    a.libere(deux, 4 * PAGE as usize);
    assert_eq!(
        a.etat().regions, 1,
        "les trois doivent former UNE region contigue"
    );
    // Et cette region unique doit pouvoir servir une demande de sa taille.
    assert_eq!(a.alloue(12 * PAGE as usize), Some(un));
}

/// Le meilleur ajustement : ne pas hacher la seule region capable de reloger
/// un anneau pour servir une demande d'une page.
#[test]
fn le_meilleur_ajustement_preserve_les_grandes_regions() {
    let a = arene(128);
    let petit = a.alloue(PAGE as usize).unwrap();
    let _garde1 = a.alloue(PAGE as usize).unwrap();
    let grand = a.alloue(16 * PAGE as usize).unwrap();
    let _garde2 = a.alloue(PAGE as usize).unwrap();

    a.libere(petit, PAGE as usize);
    a.libere(grand, 16 * PAGE as usize);

    // Une demande d'une page doit prendre le PETIT trou.
    assert_eq!(a.alloue(PAGE as usize), Some(petit));
    // La grande region est donc restee entiere.
    assert_eq!(a.alloue(16 * PAGE as usize), Some(grand));
}

/// Une region rendue plus grande que la demande se SCINDE, le reste reste
/// disponible.
#[test]
fn une_region_trop_grande_se_scinde() {
    let a = arene(64);
    let bloc = a.alloue(8 * PAGE as usize).unwrap();
    let _garde = a.alloue(PAGE as usize).unwrap();
    a.libere(bloc, 8 * PAGE as usize);

    assert_eq!(a.alloue(2 * PAGE as usize), Some(bloc));
    assert_eq!(a.alloue(2 * PAGE as usize), Some(bloc + 2 * PAGE));
    assert_eq!(a.alloue(4 * PAGE as usize), Some(bloc + 4 * PAGE));
    assert_eq!(a.etat().regions, 0, "la region doit avoir ete entierement consommee");
}

/// Rendre une region HORS arene est compte et ignore : l'ajouter a la liste
/// corromprait les allocations suivantes.
#[test]
fn une_region_hors_arene_est_refusee_et_comptee() {
    let a = arene(16);
    a.libere(BASE - 0x10_0000, PAGE as usize);
    a.libere(BASE + 1024 * PAGE, PAGE as usize);
    assert_eq!(a.etat().debordements, 2);
    assert_eq!(a.etat().regions, 0, "rien de tout cela ne doit entrer dans la liste");
    // L'arene doit rester saine.
    assert_eq!(a.alloue(PAGE as usize), Some(BASE));
}

/// La liste est bornee. Un debordement n'est pas une fuite silencieuse : il est
/// COMPTE, pour qu'on agrandisse la liste au lieu de chercher une fuite de
/// pilote qui n'existe pas.
#[test]
fn le_debordement_de_la_liste_est_compte_et_non_silencieux() {
    let a = arene(4 * (REGIONS_MAX as u64 + 8));
    // Des regions non adjacentes : une page allouee, une page gardee, etc.
    let mut rendues = Vec::new();
    for _ in 0..(REGIONS_MAX + 4) {
        let bloc = a.alloue(PAGE as usize).unwrap();
        let _garde = a.alloue(PAGE as usize).unwrap();
        rendues.push(bloc);
    }
    for base in &rendues {
        a.libere(*base, PAGE as usize);
    }
    let etat = a.etat();
    assert_eq!(etat.regions, REGIONS_MAX as u64, "la liste est pleine, pas au-dela");
    assert!(
        etat.debordements >= 4,
        "les regions non enregistrees doivent etre comptees : {etat:?}"
    );
}

// ---------------------------------------------------------------------------
// La course qui pouvait rendre DEUX FOIS la meme adresse physique.
// ---------------------------------------------------------------------------

/// Plusieurs pilotes s'initialisent en parallele. Aucune adresse ne doit
/// sortir deux fois -- c'est la faute que `static mut` sans verrou permettait,
/// et elle se manifestait comme deux pilotes ecrivant dans le meme tampon lu
/// par le materiel.
#[test]
fn aucune_adresse_n_est_servie_deux_fois_sous_contention() {
    const FILS: usize = 4;
    const PAR_FIL: usize = 200;

    let a = Arc::new(arene((FILS * PAR_FIL) as u64 + 16));
    let mut fils = Vec::new();
    for _ in 0..FILS {
        let a = Arc::clone(&a);
        fils.push(std::thread::spawn(move || {
            let mut obtenues = Vec::new();
            for _ in 0..PAR_FIL {
                obtenues.push(a.alloue(PAGE as usize).expect("arene dimensionnee"));
            }
            obtenues
        }));
    }

    let mut toutes = Vec::new();
    for f in fils {
        toutes.extend(f.join().unwrap());
    }
    let uniques: HashSet<u64> = toutes.iter().copied().collect();
    assert_eq!(
        uniques.len(), toutes.len(),
        "une adresse physique servie deux fois : deux pilotes ecriraient dans \
         le meme tampon"
    );
    assert_eq!(a.etat().allocations, (FILS * PAR_FIL) as u64);
}

/// Allocations ET liberations simultanees : l'arene doit rester coherente et
/// ne jamais servir une adresse encore detenue.
#[test]
fn allouer_et_liberer_en_meme_temps_reste_coherent() {
    const FILS: usize = 4;
    const TOURS: usize = 300;

    let a = Arc::new(arene(2048));
    let mut fils = Vec::new();
    for _ in 0..FILS {
        let a = Arc::clone(&a);
        fils.push(std::thread::spawn(move || {
            let mut detenues: Vec<u64> = Vec::new();
            for tour in 0..TOURS {
                if tour % 3 == 2 {
                    if let Some(base) = detenues.pop() {
                        a.libere(base, 2 * PAGE as usize);
                        continue;
                    }
                }
                if let Some(base) = a.alloue(2 * PAGE as usize) {
                    assert!(
                        !detenues.contains(&base),
                        "l'arene a servi une adresse que ce fil detient deja"
                    );
                    detenues.push(base);
                }
            }
            for base in &detenues {
                a.libere(*base, 2 * PAGE as usize);
            }
            detenues.len()
        }));
    }
    for f in fils {
        f.join().unwrap();
    }

    let etat: EtatDma = a.etat();
    assert_eq!(
        etat.utilise, 0,
        "tout a ete rendu : l'arene doit etre entierement disponible ({etat:?})"
    );
    assert_eq!(etat.debordements, 0, "aucune region n'a du etre abandonnee");
}
