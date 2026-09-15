//! Le classement des appels systeme les plus emis.
//!
//! Un top N borne est facile a ecrire de travers : l'entree la plus chaude
//! perdue parce que le tableau etait plein, une place ecrasee au lieu d'etre
//! decalee, un classement qui cesse d'etre trie apres la premiere eviction.
//! Aucun de ces defauts ne se voit dans un journal -- il produit une ligne
//! plausible avec les mauvais chiffres, ce qui est pire que pas de ligne.

#[path = "../../src/compat/linux/profil.rs"]
mod profil;

use profil::{insere, Chaud};

fn c(numero: u64, appels: u32) -> Chaud {
    Chaud { numero, appels, eagain: 0 }
}

fn classe(entrees: &[(u64, u32)], places: usize) -> Vec<(u64, u32)> {
    let mut sortie = vec![c(0, 0); places];
    let mut poses = 0usize;
    for &(numero, appels) in entrees {
        poses = insere(&mut sortie, poses, c(numero, appels));
    }
    sortie[..poses].iter().map(|e| (e.numero, e.appels)).collect()
}

#[test]
fn le_classement_reste_trie_du_plus_chaud_au_moins() {
    assert_eq!(
        classe(&[(1, 10), (2, 50), (3, 30)], 8),
        vec![(2, 50), (3, 30), (1, 10)]
    );
}

#[test]
fn une_entree_plus_froide_que_le_dernier_d_un_classement_plein_est_ignoree() {
    // Trois places, quatre candidats : le plus froid ne doit pas entrer, et
    // surtout il ne doit pas expulser un plus chaud.
    assert_eq!(
        classe(&[(1, 100), (2, 90), (3, 80), (4, 1)], 3),
        vec![(1, 100), (2, 90), (3, 80)]
    );
}

#[test]
fn une_entree_plus_chaude_expulse_la_derniere_et_pas_une_autre() {
    assert_eq!(
        classe(&[(1, 100), (2, 90), (3, 80), (4, 95)], 3),
        vec![(1, 100), (4, 95), (2, 90)]
    );
}

#[test]
fn la_plus_chaude_arrivee_en_dernier_prend_bien_la_premiere_place() {
    // Le cas que rate un classement qui n'insere qu'a la fin : l'appel qui
    // designe le tour en rond est justement celui qui explose tard.
    assert_eq!(
        classe(&[(1, 5), (2, 6), (3, 7), (4, 4_000_000)], 4),
        vec![(4, 4_000_000), (3, 7), (2, 6), (1, 5)]
    );
}

#[test]
fn un_classement_plein_garde_exactement_les_n_plus_chauds() {
    let entrees: Vec<(u64, u32)> = (0..50).map(|i| (i as u64, i as u32)).collect();
    let obtenu = classe(&entrees, 5);
    assert_eq!(obtenu, vec![(49, 49), (48, 48), (47, 47), (46, 46), (45, 45)]);
}

#[test]
fn des_egalites_ne_font_perdre_aucune_place() {
    let obtenu = classe(&[(1, 7), (2, 7), (3, 7), (4, 7)], 3);
    assert_eq!(obtenu.len(), 3);
    assert!(obtenu.iter().all(|&(_, appels)| appels == 7));
}

#[test]
fn un_classement_sans_place_ne_panique_pas() {
    let mut vide: [Chaud; 0] = [];
    assert_eq!(insere(&mut vide, 0, c(1, 999)), 0);
}

#[test]
fn le_nombre_de_places_occupees_ne_depasse_jamais_le_tableau() {
    let mut sortie = vec![c(0, 0); 2];
    let mut poses = 0usize;
    for i in 0..100u32 {
        poses = insere(&mut sortie, poses, c(i as u64, i));
        assert!(poses <= 2, "poses={} au tour {}", poses, i);
    }
    assert_eq!(poses, 2);
}

#[test]
fn eagain_suit_son_entree_et_ne_glisse_pas_de_place() {
    // Le nombre d'EAGAIN doit rester attache a SON appel. S'il glissait d'une
    // place au decalage, la ligne accuserait le mauvais appel de tourner en
    // rond -- et c'est precisement le chiffre sur lequel on decide.
    let mut sortie = vec![c(0, 0); 3];
    let mut poses = 0;
    for (numero, appels, eagain) in [(10u64, 5u32, 1u32), (20, 50, 49), (30, 20, 0)] {
        poses = insere(&mut sortie, poses, Chaud { numero, appels, eagain });
    }
    let vu: Vec<(u64, u32, u32)> = sortie[..poses]
        .iter()
        .map(|e| (e.numero, e.appels, e.eagain))
        .collect();
    assert_eq!(vu, vec![(20, 50, 49), (30, 20, 0), (10, 5, 1)]);
}
