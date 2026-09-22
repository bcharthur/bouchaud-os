//! CE QUE LE MONDE UTILISATEUR APPREND DU NOMBRE DE PROCESSEURS, A L'HOTE.
//!
//! # Le defaut que ce banc fige
//!
//! `/sys/devices/system/cpu/online` annoncait « 0 » sur une machine a seize
//! fils. Ce n'est pas un compte, c'est une PLAGE : celle-ci ne contient que le
//! processeur zero. Tout ce qui dimensionne un pool de threads -- la glibc par
//! `sysconf(_SC_NPROCESSORS_ONLN)`, AK, Skia, LibJS -- en concluait qu'il y
//! avait UN processeur, et creait un pool d'un fil.
//!
//! Le releve physique disait « la charge WebContent peut etre tres elevee sur
//! un seul coeur alors que le CPU global parait faible ». C'est la description
//! exacte de ce que ce fichier provoquait.
//!
//! # Pourquoi ces cas-la
//!
//! Chacun se traduit par une mauvaise taille de pool et par rien d'autre :
//! aucun ne fait echouer quoi que ce soit, aucun ne se voit dans un test
//! d'integration, et tous rendent le navigateur monofil.

#[path = "../../src/kernel/cpu_topologie.rs"]
mod cpu_topologie;

use cpu_topologie::{annonces, bloc, plage};

fn texte(count: usize) -> String {
    let mut tampon = [0u8; 32];
    let n = plage(count, &mut tampon);
    String::from_utf8(tampon[..n].to_vec()).expect("ascii")
}

#[test]
fn seize_processeurs_donnent_la_plage_complete() {
    // LE CAS REEL : Ryzen 7 5700U, seize fils, MAX_CPUS = 16.
    assert_eq!(texte(16), "0-15", "et surtout pas 0-16");
}

#[test]
fn un_seul_processeur_s_ecrit_comme_linux_l_ecrit() {
    // Linux ecrit « 0 » et jamais « 0-0 ». Une plage degeneree est lue
    // correctement par la plupart des analyseurs, et par la plupart seulement.
    assert_eq!(texte(1), "0");
}

#[test]
fn zero_processeur_n_ecrit_rien() {
    // Le piege : `count - 1` en arithmetique non signee donnerait « 0--1 » ou
    // « 0-18446744073709551615 ». Les deux se lisent comme une plage valide
    // par un analyseur permissif.
    assert_eq!(texte(0), "");
}

#[test]
fn les_plages_a_deux_chiffres_et_plus_sont_completes() {
    assert_eq!(texte(2), "0-1");
    assert_eq!(texte(4), "0-3");
    assert_eq!(texte(10), "0-9");
    assert_eq!(texte(11), "0-10");
    assert_eq!(texte(128), "0-127");
}

#[test]
fn un_tampon_trop_court_ne_deborde_pas() {
    // Le texte est produit pendant l'installation du sysroot, dans un tampon
    // de pile. Un depassement y serait une corruption silencieuse.
    let mut minuscule = [0u8; 2];
    let n = plage(1024, &mut minuscule);
    assert!(n <= minuscule.len());
    let mut vide: [u8; 0] = [];
    assert_eq!(plage(16, &mut vide), 0);
}

#[test]
fn les_blocs_de_cpuinfo_decrivent_coeurs_et_fils() {
    // Seize fils, deux par coeur : huit coeurs physiques. Skia et d'autres
    // choisissent leur parallelisme d'apres `cpu cores`, pas d'apres le
    // nombre de lignes.
    let premier = bloc(16, 2, 0).expect("le processeur zero existe");
    assert_eq!(premier.processeur, 0);
    assert_eq!(premier.siblings, 16);
    assert_eq!(premier.coeurs, 8);
    assert_eq!(premier.core_id, 0);

    let deuxieme = bloc(16, 2, 1).expect("le processeur un existe");
    assert_eq!(deuxieme.core_id, 0, "deux fils du meme coeur partagent leur core_id");

    let troisieme = bloc(16, 2, 2).expect("le processeur deux existe");
    assert_eq!(troisieme.core_id, 1);

    let dernier = bloc(16, 2, 15).expect("le processeur quinze existe");
    assert_eq!(dernier.core_id, 7);
}

#[test]
fn le_bloc_au_dela_du_dernier_n_existe_pas() {
    // La boucle d'ecriture s'arrete dessus : c'est le seul endroit qui
    // connaisse la borne.
    assert!(bloc(16, 2, 16).is_none());
    assert!(bloc(0, 2, 0).is_none());
}

#[test]
fn zero_fil_par_coeur_ne_divise_pas_par_zero() {
    // Un materiel qui ne renseigne pas ce champ est plus courant qu'on ne
    // croit. Un fil par coeur sous-estime le parallelisme au pire ; il ne
    // rend jamais un compte absurde, et il ne plante pas.
    let b = bloc(16, 0, 3).expect("le processeur trois existe");
    assert_eq!(b.coeurs, 16);
    assert_eq!(b.core_id, 3);
}

#[test]
fn le_nombre_annonce_est_borne_et_jamais_nul() {
    assert_eq!(annonces(16, 16), 16);
    assert_eq!(annonces(4, 16), 4);
    // Un noyau qui n'a pas fini d'enumerer rend zero. Publier une plage vide
    // ferait retomber les bibliotheques sur leurs heuristiques.
    assert_eq!(annonces(0, 16), 1);
    // Et jamais plus que ce que le noyau peut ordonnancer.
    assert_eq!(annonces(64, 16), 16);
    assert_eq!(annonces(8, 0), 1);
}
