//! Qui passe devant sur le pilote USB.
//!
//! Le 16 septembre, deux symptomes rapportes le meme jour : « le clavier
//! fonctionne mais quand je commence a taper dans Ladybird, il est
//! deconnecte », et une archive blackbox qui s'arrete a 7,30 s, a l'instant
//! ou les services demarrent. Ni l'un ni l'autre n'etait en panne : tous deux
//! renoncaient au verrou du pilote USB, que le systeme de fichiers tenait en
//! continu pour lire les quatre cents mebioctets du navigateur.

#[path = "../../src/drivers/usb/equite_pilote.rs"]
mod equite;

use equite::{
    cede_encore, Equite, CESSION_MAXIMALE_NS, FAMINE_AVANT_PRIORITE_NS, SAUTS_AVANT_PRIORITE,
};

const MS: u64 = 1_000_000;

fn saut(e: &Equite, ms: u64) -> bool {
    e.saut(ms * MS, SAUTS_AVANT_PRIORITE, FAMINE_AVANT_PRIORITE_NS)
}

#[test]
fn un_saut_isole_ne_reclame_rien() {
    let e = Equite::neuve();
    assert!(!saut(&e, 100));
    assert!(!e.reclame(), "une commande BOT qui passe n'est pas une famine");
}

#[test]
fn quelques_sauts_d_affilee_reclament_le_passage() {
    let e = Equite::neuve();
    for i in 0..SAUTS_AVANT_PRIORITE - 1 {
        assert!(!saut(&e, 100 + i), "saut {} ne devrait pas reclamer", i);
    }
    assert!(saut(&e, 100 + SAUTS_AVANT_PRIORITE));
    assert!(e.reclame());
}

#[test]
fn une_famine_longue_reclame_meme_sans_beaucoup_de_sauts() {
    // Un consommateur lent peut etre affame sans scruter souvent : le seuil
    // de DUREE existe pour lui.
    let e = Equite::neuve();
    assert!(!saut(&e, 0));
    assert!(saut(&e, FAMINE_AVANT_PRIORITE_NS / MS + 1));
    assert!(e.reclame());
}

#[test]
fn le_premier_saut_date_la_famine() {
    let e = Equite::neuve();
    saut(&e, 1_000);
    // Si chaque saut reposait l'horodatage, la famine paraitrait
    // perpetuellement naissante et le seuil de duree ne tomberait jamais.
    for t in 1_001..1_010 {
        saut(&e, t);
    }
    let e2 = Equite::neuve();
    saut(&e2, 1_000);
    assert!(
        saut(&e2, 1_000 + FAMINE_AVANT_PRIORITE_NS / MS + 1),
        "le seuil de duree doit se mesurer depuis le PREMIER saut"
    );
}

#[test]
fn un_succes_eteint_la_reclamation() {
    let e = Equite::neuve();
    for i in 0..SAUTS_AVANT_PRIORITE + 4 {
        saut(&e, 100 + i);
    }
    assert!(e.reclame());
    e.succes();
    assert!(!e.reclame(), "le passage obtenu met fin a la famine");
    assert_eq!(e.sauts(), 0);
}

#[test]
fn apres_un_succes_la_famine_repart_de_zero() {
    let e = Equite::neuve();
    for i in 0..SAUTS_AVANT_PRIORITE + 1 {
        saut(&e, 100 + i);
    }
    e.succes();
    // Un seul saut apres coup ne doit pas rereclamer immediatement.
    assert!(!saut(&e, 500));
    assert!(!e.reclame());
}

#[test]
fn le_systeme_de_fichiers_cede_tant_qu_on_reclame() {
    assert!(cede_encore(true, 0, CESSION_MAXIMALE_NS));
    assert!(cede_encore(true, CESSION_MAXIMALE_NS - 1, CESSION_MAXIMALE_NS));
}

#[test]
fn il_ne_cede_pas_quand_personne_ne_reclame() {
    assert!(!cede_encore(false, 0, CESSION_MAXIMALE_NS));
    assert!(!cede_encore(false, CESSION_MAXIMALE_NS * 10, CESSION_MAXIMALE_NS));
}

#[test]
fn la_cession_est_bornee() {
    // Un consommateur qui reclame sans jamais aboutir -- pilote en panne,
    // peripherique parti -- ne doit pas bloquer le systeme de fichiers.
    // Remplacer une famine par un blocage serait un plus mauvais marche.
    assert!(!cede_encore(true, CESSION_MAXIMALE_NS, CESSION_MAXIMALE_NS));
    assert!(!cede_encore(true, u64::MAX, CESSION_MAXIMALE_NS));
}

#[test]
fn les_cessions_se_comptent() {
    let e = Equite::neuve();
    assert_eq!(e.cessions(), 0);
    e.note_cession();
    e.note_cession();
    assert_eq!(e.cessions(), 2, "le releve doit pouvoir dire que ca a servi");
}

#[test]
fn les_seuils_restent_raisonnables() {
    // Reclamer des le premier saut ferait ceder le systeme de fichiers a
    // chaque commande, et le debit s'effondrerait.
    assert!(SAUTS_AVANT_PRIORITE >= 2);
    // Une cession plus longue que la famine qui la declenche laisserait le
    // systeme de fichiers ceder plus longtemps qu'il n'a fait attendre.
    assert!(CESSION_MAXIMALE_NS <= FAMINE_AVANT_PRIORITE_NS);
}

#[test]
fn une_famine_qui_commence_a_l_instant_zero_est_datee() {
    // Le defaut d'origine : `premier_saut_ns` valait zero pour dire « pas
    // encore date », et zero est aussi l'horodatage du tout premier instant.
    // Un `compare_exchange(0, 0)` reussit sans rien ecrire ; la famine
    // restait indatable et le passage n'etait jamais reclame.
    let e = Equite::neuve();
    assert!(!saut(&e, 0));
    assert!(
        saut(&e, FAMINE_AVANT_PRIORITE_NS / MS + 1),
        "une famine commencee a l'instant zero doit se dater comme les autres"
    );
    assert!(e.reclame());
}

// ===========================================================================
// Le tourniquet : ceder n'est pas laisser passer
// ===========================================================================

use equite::{Tourniquet, RESERVATION_MAXIMALE_NS};

#[test]
fn un_tourniquet_neuf_ne_bloque_personne() {
    let t = Tourniquet::neuf();
    assert!(!t.reserve());
    assert!(!t.doit_ceder(0, RESERVATION_MAXIMALE_NS));
    assert!(!t.doit_ceder(10_000 * MS, RESERVATION_MAXIMALE_NS));
}

#[test]
fn une_reclamation_ferme_le_passage_aux_autres() {
    // LE DEFAUT QUE CECI CORRIGE
    //
    // `cede_encore` faisait ATTENDRE le systeme de fichiers avant de prendre
    // le verrou -- et rien ne l'empechait de le reprendre juste apres. Le banc
    // a mesure trente-six refus consecutifs pour la scrutation HID et cent
    // quarante-quatre millisecondes de famine, cession active.
    let t = Tourniquet::neuf();
    t.reclame(100 * MS);
    assert!(t.reserve());
    assert!(t.doit_ceder(101 * MS, RESERVATION_MAXIMALE_NS));
    assert!(t.doit_ceder(140 * MS, RESERVATION_MAXIMALE_NS));
}

#[test]
fn le_passage_se_rouvre_des_que_la_scrutation_a_eu_son_tour() {
    let t = Tourniquet::neuf();
    t.reclame(100 * MS);
    t.libere();
    assert!(!t.reserve());
    assert!(!t.doit_ceder(101 * MS, RESERVATION_MAXIMALE_NS));
}

#[test]
fn une_reservation_qui_traine_expire_et_rend_le_disque() {
    // Une reservation qui ne s'eteint pas transforme une famine du clavier en
    // blocage du stockage. Un fil HID mort ne doit pas emporter le disque.
    let t = Tourniquet::neuf();
    t.reclame(100 * MS);
    assert!(t.doit_ceder(149 * MS, RESERVATION_MAXIMALE_NS));
    assert!(
        !t.doit_ceder(151 * MS, RESERVATION_MAXIMALE_NS),
        "au-dela de la borne, le passage se rouvre tout seul"
    );
    assert!(!t.reserve(), "et la reservation est bien levee");
    assert_eq!(t.compteurs().2, 1, "l'expiration se compte");
}

#[test]
fn une_famine_continue_ne_repousse_pas_sa_propre_expiration() {
    // Si chaque reclamation rajeunissait la reservation, une scrutation
    // affamee en continu la garderait ouverte pour toujours -- et la borne ne
    // serait jamais atteinte.
    let t = Tourniquet::neuf();
    t.reclame(100 * MS);
    for i in 1..40 {
        t.reclame((100 + i * 4) as u64 * MS);
    }
    assert!(
        !t.doit_ceder(151 * MS, RESERVATION_MAXIMALE_NS),
        "la reservation date du PREMIER appel, pas du dernier"
    );
}

#[test]
fn les_reservations_se_comptent_une_seule_fois_par_serie() {
    let t = Tourniquet::neuf();
    t.reclame(10 * MS);
    t.reclame(11 * MS);
    t.reclame(12 * MS);
    assert_eq!(t.compteurs().0, 1);
    t.libere();
    t.reclame(20 * MS);
    assert_eq!(t.compteurs().0, 2);
}

#[test]
fn les_refus_opposes_aux_autres_se_comptent() {
    // Sans ce chiffre, on ne sait pas si le tourniquet a servi ou s'il n'a
    // jamais eu l'occasion de s'ouvrir.
    let t = Tourniquet::neuf();
    t.reclame(0);
    for _ in 0..5 {
        assert!(t.doit_ceder(1 * MS, RESERVATION_MAXIMALE_NS));
    }
    assert_eq!(t.compteurs().1, 5);
}

#[test]
fn la_borne_est_celle_du_seuil_de_defaut_produit() {
    // Cinquante millisecondes : au-dela, un ecart de scrutation est un defaut
    // produit. Une reservation plus longue ne protegerait plus ce qu'elle est
    // censee protéger.
    assert_eq!(RESERVATION_MAXIMALE_NS, 50 * MS);
}
