//! Preuve hote de la regle de vol de travail du chantier 2.
//!
//! Ce test inclut le module de PRODUCTION `src/kernel/scheduler/equilibrage.rs`
//! tel quel : ce qui est verifie ici est ce que le noyau execute, pas une
//! reecriture qui lui ressemble.
//!
//! # Le defaut, et pourquoi aucun test ne pouvait le voir
//!
//! `pression_volable()` rend le nombre de taches NORMALES EN ATTENTE. La tache
//! que le coeur execute n'y figure pas : elle a quitte la file en etant elue.
//!
//! La regle exigeait `pression > 1`. Avec des longueurs qui excluent la tache
//! courante, cela demandait DEUX taches en attente EN PLUS de celle qui tourne.
//! Un coeur executant une tache avec une seule autre en attente n'etait donc
//! jamais deleste -- meme avec trois coeurs au repos a cote.
//!
//! La campagne SMP4 le mesure sans ambiguite : `steal=0/0` sur les quatre
//! coeurs -- pas une seule TENTATIVE -- et `rej_bal` cumule a 2852, c'est-a-dire
//! 2852 occasions rejetees au filtre avant meme d'essayer.
//!
//! La regle vivait au milieu de `try_steal`, entre des acces per-CPU, des
//! identites generationnelles et des revendications atomiques. Elle n'etait
//! atteignable qu'en demarrant le systeme. Extraite en fonction pure, elle se
//! contredit ici en une milliseconde.

#[path = "../../src/kernel/scheduler/equilibrage.rs"]
mod equilibrage;

use equilibrage::{choisit_donneur, PRESSION_MINIMALE};

/// Pression fournie par un tableau, pour decrire une topologie en une ligne.
fn depuis(charges: &[usize]) -> impl Fn(usize) -> usize + '_ {
    move |cpu| charges.get(cpu).copied().unwrap_or(0)
}

/// LE test de regression. Un coeur execute une tache, une seule autre attend
/// derriere ; trois coeurs sont au repos. C'est le cas que `> 1` refusait, et
/// c'est exactement celui ou le vol cree du parallelisme.
#[test]
fn une_seule_tache_en_attente_suffit_a_designer_un_donneur() {
    let charges = [0, 0, 0, 1];
    assert_eq!(choisit_donneur(0, 4, depuis(&charges)), Some(3));
}

/// L'ancien seuil, ecrit tel qu'il etait, sur la meme topologie : rien. Ce test
/// fige la difference pour qu'un retour en arriere soit visible.
#[test]
fn l_ancien_seuil_aurait_refuse_le_meme_cas() {
    let charges = [0, 0, 0, 1];
    let ancien = (0..4)
        .filter(|&c| c != 0)
        .map(|c| (c, charges[c]))
        .filter(|&(_, p)| p > 1)
        .max_by_key(|&(_, p)| p)
        .map(|(c, _)| c);
    assert_eq!(ancien, None);
    assert!(choisit_donneur(0, 4, depuis(&charges)).is_some());
}

/// Une file vide partout n'offre rien : voler ne deplacerait aucun travail.
#[test]
fn sans_aucune_attente_il_n_y_a_pas_de_donneur() {
    let charges = [0, 0, 0, 0];
    assert_eq!(choisit_donneur(1, 4, depuis(&charges)), None);
}

/// Le plus charge, pour que le vol suivant ait encore un candidat plutot que
/// d'egaliser deux coeurs et de relancer un scan complet.
#[test]
fn le_donneur_retenu_est_le_plus_charge() {
    let charges = [0, 2, 7, 3];
    assert_eq!(choisit_donneur(0, 4, depuis(&charges)), Some(2));
}

/// Un coeur ne se vole pas lui-meme, meme s'il est de loin le plus charge.
#[test]
fn le_voleur_ne_se_choisit_jamais() {
    let charges = [9, 1, 0, 0];
    assert_eq!(choisit_donneur(0, 4, depuis(&charges)), Some(1));
}

/// Un coeur hors ligne ne doit pas etre designe : sa file ne sera pas servie.
#[test]
fn les_coeurs_hors_ligne_sont_ignores() {
    let charges = [0, 1, 0, 50];
    assert_eq!(choisit_donneur(0, 2, depuis(&charges)), Some(1));
}

/// Un seul coeur en ligne : il n'existe aucun autre a delester.
#[test]
fn un_seul_coeur_n_a_personne_a_delester() {
    let charges = [42];
    assert_eq!(choisit_donneur(0, 1, depuis(&charges)), None);
}

/// Le seuil est UNE tache en attente, et il est publie : le donneur garde
/// toujours ce qu'il execute, donc rien ne lui est retire.
#[test]
fn le_seuil_publie_vaut_une_tache_en_attente() {
    assert_eq!(PRESSION_MINIMALE, 1);
    let juste_au_seuil = [0, PRESSION_MINIMALE];
    assert_eq!(choisit_donneur(0, 2, depuis(&juste_au_seuil)), Some(1));
    let juste_en_dessous = [0, PRESSION_MINIMALE - 1];
    assert_eq!(choisit_donneur(0, 2, depuis(&juste_en_dessous)), None);
}

/// A charge egale un donneur est tout de meme retenu : l'absence de maximum
/// strict ne doit pas se traduire par un refus.
#[test]
fn des_charges_egales_designent_quand_meme_un_donneur() {
    let charges = [0, 2, 2, 2];
    assert!(choisit_donneur(0, 4, depuis(&charges)).is_some());
}
