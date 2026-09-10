//! Injection des achevements que le materiel ne produira qu'une fois.
//!
//! Le module de production `src/drivers/block/nvme/suivi.rs` est inclus tel
//! quel. Il ne touche aucun registre, ce qui permet de lui presenter ici les
//! sequences qu'un banc d'essai ne reproduit pas : un achevement qui arrive
//! apres l'echeance, un achevement portant un identifiant deja recolte, deux
//! achevements dans le desordre, un bouclage de file avec inversion de phase.
//!
//! Ce qui est cherche ici n'est PAS le cas nominal. Une lecture qui s'acheve
//! en cent microsecondes marche meme avec un pilote qui rendrait l'identifiant
//! sur echeance. Ce sont les fautes qui ne se voient jamais au banc et qui se
//! manifestent en corruption silencieuse.

#![allow(dead_code)]

#[path = "../../src/drivers/block/nvme/suivi.rs"]
mod suivi;

use suivi::*;

// ---------------------------------------------------------------------------
// La regle centrale : une echeance met en quarantaine, elle ne libere pas.
// ---------------------------------------------------------------------------

#[test]
fn une_echeance_ne_rend_pas_l_identifiant_au_pot() {
    let mut s = Suivi::neuf();
    let cid = s.alloue().expect("premier identifiant");
    assert!(s.expire(cid));
    assert_eq!(
        s.etat_de(cid),
        Some(EtatCid::Quarantaine),
        "une echeance depassee n'annule rien cote controleur : il peut encore \
         ecrire dans le tampon de cette commande"
    );
    assert_eq!(s.quarantaine(), 1);
    assert_eq!(s.en_vol(), 0);
}

#[test]
fn un_identifiant_en_quarantaine_n_est_jamais_reattribue() {
    let mut s = Suivi::neuf();
    let victime = s.alloue().unwrap();
    assert!(s.expire(victime));

    // On epuise tout le domaine. Si la quarantaine etait ignoree, le tour
    // complet redonnerait forcement l'identifiant abandonne.
    let mut vus = Vec::new();
    while let Some(cid) = s.alloue() {
        assert_ne!(
            cid, victime,
            "l'identifiant {} est en quarantaine : le reattribuer ferait \
             prendre l'achevement tardif de l'ancienne commande pour celui de \
             la nouvelle",
            victime
        );
        vus.push(cid);
    }
    assert_eq!(
        vus.len(),
        CID_MAX - 2,
        "tout le domaine moins le zero reserve et la victime en quarantaine"
    );
}

#[test]
fn l_achevement_tardif_leve_la_quarantaine_et_rien_d_autre() {
    let mut s = Suivi::neuf();
    let cid = s.alloue().unwrap();
    assert!(s.expire(cid));
    assert_eq!(s.quarantaine(), 1);

    // L'achevement finit par arriver. C'est la PREUVE que le controleur en a
    // fini avec le tampon : c'est exactement la que la quarantaine se leve.
    assert_eq!(s.range(cid, 0, 0), Verdict::Tardif);
    assert_eq!(s.quarantaine(), 0);
    assert_eq!(s.etat_de(cid), Some(EtatCid::Libre));
    assert_eq!(s.tardifs, 1);

    // Et il n'est surtout pas recoltable : personne ne l'attend plus.
    assert_eq!(
        s.recolte(cid),
        None,
        "un achevement tardif ne doit satisfaire aucun attendant"
    );
}

#[test]
fn echeance_puis_achevement_tardif_puis_reemission() {
    let mut s = Suivi::neuf();
    let premier = s.alloue().unwrap();
    assert!(s.expire(premier));
    assert_eq!(s.range(premier, 0, 0), Verdict::Tardif);

    // Une fois la quarantaine levee, l'identifiant redevient utilisable.
    let mut retrouve = false;
    for _ in 0..CID_MAX {
        let cid = s.alloue().unwrap();
        if cid == premier {
            retrouve = true;
            break;
        }
    }
    assert!(retrouve, "l'identifiant doit revenir au pot une fois l'achevement recu");
}

// ---------------------------------------------------------------------------
// Achevements que le controleur n'aurait pas du envoyer.
// ---------------------------------------------------------------------------

#[test]
fn un_achevement_pour_un_identifiant_jamais_emis_est_rejete() {
    let mut s = Suivi::neuf();
    assert_eq!(s.range(42, 0, 0), Verdict::Inconnu);
    assert_eq!(s.rejets_inconnu, 1);
}

#[test]
fn un_second_achevement_pour_le_meme_identifiant_est_rejete() {
    let mut s = Suivi::neuf();
    let cid = s.alloue().unwrap();
    assert_eq!(s.range(cid, 0, 0), Verdict::Attendu);
    assert_eq!(
        s.range(cid, 0, 0),
        Verdict::Double,
        "un doublon qui ecraserait le statut range ferait rendre a l'emetteur \
         un resultat qui n'est pas celui de sa commande"
    );
    assert_eq!(s.rejets_double, 1);
    // Le premier statut survit au doublon.
    assert_eq!(s.recolte(cid), Some((0, 0)));
}

#[test]
fn un_identifiant_hors_domaine_n_est_pas_replie_par_modulo() {
    let mut s = Suivi::neuf();
    // 300 % 256 vaut 44. Un pilote qui plie par modulo fabriquerait un
    // achevement pour l'identifiant 44, qui appartient peut-etre a une
    // commande bien vivante.
    let cid44 = {
        let mut trouve = None;
        for _ in 0..CID_MAX {
            let c = s.alloue().unwrap();
            if c == 44 {
                trouve = Some(c);
                break;
            }
        }
        trouve.expect("l'identifiant 44 est dans le domaine")
    };
    assert_eq!(s.range(300, 0, 0), Verdict::HorsDomaine);
    assert_eq!(
        s.etat_de(cid44),
        Some(EtatCid::EnVol),
        "l'identifiant 44 ne doit pas avoir ete achevé par un achevement qui \
         portait 300"
    );
    assert_eq!(s.rejets_hors_domaine, 1);
}

#[test]
fn un_achevement_zero_est_rejete() {
    let mut s = Suivi::neuf();
    assert_eq!(
        s.range(0, 0, 0),
        Verdict::HorsDomaine,
        "zero n'est jamais emis : un achevement qui le porte vient d'une \
         entree jamais ecrite, pas d'une commande"
    );
}

#[test]
fn un_achevement_d_avant_la_reinitialisation_est_dit_perime() {
    let mut s = Suivi::neuf();
    let cid = s.alloue().unwrap();
    assert!(s.expire(cid));
    s.reinitialise();
    assert_eq!(s.quarantaine(), 0, "un controleur remis a zero a perdu ses files");
    assert_eq!(
        s.range(cid, 0, 0),
        Verdict::Perime,
        "un achevement d'avant la reinitialisation doit etre distingue d'un \
         identifiant jamais emis : le diagnostic n'est pas le meme"
    );
    assert_eq!(s.rejets_perime, 1);
}

// ---------------------------------------------------------------------------
// Plusieurs commandes en vol.
// ---------------------------------------------------------------------------

#[test]
fn des_achevements_dans_le_desordre_vont_chacun_a_leur_emetteur() {
    let mut s = Suivi::neuf();
    let a = s.alloue().unwrap();
    let b = s.alloue().unwrap();
    let c = s.alloue().unwrap();
    assert_eq!(s.en_vol(), 3);

    // Le controleur acheve dans l'ordre c, a, b, avec trois statuts distincts.
    assert_eq!(s.range(c, 0, 0), Verdict::Attendu);
    assert_eq!(s.range(a, 2, 0x81), Verdict::Attendu);
    assert_eq!(s.range(b, 0, 0), Verdict::Attendu);
    assert_eq!(s.en_vol(), 0);

    assert_eq!(s.recolte(a), Some((2, 0x81)), "le statut d'erreur va a A, pas a B ni C");
    assert_eq!(s.recolte(b), Some((0, 0)));
    assert_eq!(s.recolte(c), Some((0, 0)));
}

#[test]
fn une_erreur_de_statut_est_transmise_telle_quelle() {
    let mut s = Suivi::neuf();
    let cid = s.alloue().unwrap();
    // Type 2 (media), code 0x81 (unrecovered read error).
    assert_eq!(s.range(cid, 2, 0x81), Verdict::Attendu);
    assert_eq!(
        s.recolte(cid),
        Some((2, 0x81)),
        "un statut d'erreur qui se perdrait en route ferait rendre un tampon \
         vide comme une lecture reussie"
    );
}

#[test]
fn le_pot_s_epuise_proprement_au_lieu_de_reattribuer() {
    let mut s = Suivi::neuf();
    let mut n = 0;
    while s.alloue().is_some() {
        n += 1;
        assert!(n <= CID_MAX, "le pot ne doit pas boucler");
    }
    assert_eq!(n, CID_MAX - 1, "tout le domaine sauf le zero reserve");
    assert_eq!(
        s.alloue(),
        None,
        "refuser une entree-sortie vaut mieux que d'en corrompre une autre"
    );
}

// ---------------------------------------------------------------------------
// Bouclage des anneaux.
// ---------------------------------------------------------------------------

#[test]
fn la_phase_de_la_file_d_achevement_s_inverse_au_bouclage() {
    let mut cq = AnneauAchevement::neuf(4);
    assert!(cq.phase, "au premier tour, la phase attendue est vraie");
    for _ in 0..4 {
        assert!(cq.est_neuve(true));
        assert!(!cq.est_neuve(false));
        cq.avance();
    }
    assert_eq!(cq.tete, 0, "la tete revient a zero");
    assert!(
        !cq.phase,
        "au deuxieme tour, c'est la phase FAUSSE qui marque une entree neuve ; \
         un pilote qui garde la phase initiale relit indefiniment les entrees \
         du premier tour"
    );
    for _ in 0..4 {
        assert!(cq.est_neuve(false));
        assert!(!cq.est_neuve(true));
        cq.avance();
    }
    assert!(cq.phase, "et elle revient au troisieme tour");
}

#[test]
fn la_file_de_soumission_refuse_d_ecraser_une_commande_non_lue() {
    let mut sq = AnneauSoumission::neuf(4);
    assert_eq!(sq.places(), 3, "une entree reste vide pour distinguer plein de vide");
    assert_eq!(sq.pose(), Some(0));
    assert_eq!(sq.pose(), Some(1));
    assert_eq!(sq.pose(), Some(2));
    assert_eq!(
        sq.pose(),
        None,
        "le controleur n'a rien consomme : poser une quatrieme commande \
         ecraserait la premiere, qu'il n'a pas encore lue"
    );

    // Le controleur publie sa tete dans l'achevement : deux places se liberent.
    sq.tete_vue(2);
    assert_eq!(sq.places(), 2);
    assert_eq!(sq.pose(), Some(3));
    assert_eq!(sq.pose(), Some(0), "et la file boucle proprement");
}

#[test]
fn une_tete_de_soumission_absurde_est_ignoree() {
    let mut sq = AnneauSoumission::neuf(4);
    sq.pose();
    let avant = sq.tete;
    sq.tete_vue(9);
    assert_eq!(
        sq.tete, avant,
        "une tete hors de l'anneau viendrait d'un achevement corrompu ; la \
         croire ferait calculer des places libres qui n'existent pas"
    );
}

#[test]
fn le_bouclage_de_la_soumission_conserve_le_compte_de_places() {
    let mut sq = AnneauSoumission::neuf(8);
    // Deux tours complets, en tenant le controleur au courant.
    for tour in 0..16 {
        let index = sq.pose().expect("place libre");
        assert_eq!(index, tour % 8);
        sq.tete_vue(((tour + 1) % 8) as u16);
    }
    assert_eq!(sq.places(), 7, "apres deux tours, la file est de nouveau vide");
}

// ---------------------------------------------------------------------------
// Reinitialisation.
// ---------------------------------------------------------------------------

#[test]
fn la_reinitialisation_rend_tout_le_domaine() {
    let mut s = Suivi::neuf();
    for _ in 0..10 {
        let cid = s.alloue().unwrap();
        s.expire(cid);
    }
    assert_eq!(s.quarantaine(), 10);
    let epoque = s.epoque();
    s.reinitialise();
    assert_eq!(s.quarantaine(), 0);
    assert_eq!(s.en_vol(), 0);
    assert_ne!(s.epoque(), epoque, "l'epoque doit avancer, sinon les tardifs se confondent");
    let mut n = 0;
    while s.alloue().is_some() {
        n += 1;
    }
    assert_eq!(n, CID_MAX - 1, "tout le domaine est rendu");
}

#[test]
fn une_reinitialisation_a_vide_ne_casse_rien() {
    let mut s = Suivi::neuf();
    s.reinitialise();
    let cid = s.alloue().expect("le pot est intact");
    assert_eq!(s.range(cid, 0, 0), Verdict::Attendu);
    assert_eq!(s.recolte(cid), Some((0, 0)));
}

// ---------------------------------------------------------------------------
// La course entre l'echeance et l'achevement.
// ---------------------------------------------------------------------------

#[test]
fn un_achevement_arrive_juste_avant_l_echeance_n_est_pas_perdu() {
    let mut s = Suivi::neuf();
    let cid = s.alloue().unwrap();
    // L'achevement se range pendant que l'emetteur decide d'abandonner.
    assert_eq!(s.range(cid, 0, 0), Verdict::Attendu);
    assert!(
        !s.expire(cid),
        "la course existe : traiter en echeance une commande achevee perdrait \
         un resultat valide et mettrait en quarantaine un tampon libre"
    );
    assert_eq!(s.recolte(cid), Some((0, 0)));
    assert_eq!(s.quarantaine(), 0);
}

#[test]
fn recolter_une_commande_encore_en_vol_ne_rend_rien() {
    let mut s = Suivi::neuf();
    let cid = s.alloue().unwrap();
    assert_eq!(
        s.recolte(cid),
        None,
        "rendre un tampon avant l'achevement, c'est lire pendant que le \
         controleur ecrit"
    );
    assert_eq!(s.etat_de(cid), Some(EtatCid::EnVol));
}

// ---------------------------------------------------------------------------
// La propriete du tampon de rebond.
// ---------------------------------------------------------------------------

#[test]
fn le_tampon_n_est_pas_disponible_tant_qu_une_quarantaine_tient() {
    let mut s = Suivi::neuf();
    assert!(s.tampon_disponible(), "rien en vol : le tampon est libre");

    let cid = s.alloue().unwrap();
    assert!(
        s.tampon_disponible(),
        "une commande en vol tient le tampon par le jeton d'entree-sortie, \
         pas par la quarantaine : les deux mecanismes sont distincts"
    );

    assert!(s.expire(cid));
    assert!(
        !s.tampon_disponible(),
        "la commande est abandonnee, mais le controleur, lui, n'a rien annule : \
         rendre le tampon maintenant, c'est autoriser une ecriture DMA dans le \
         tampon de la commande SUIVANTE"
    );

    assert_eq!(s.range(cid, 0, 0), Verdict::Tardif);
    assert!(
        s.tampon_disponible(),
        "l'achevement tardif est la preuve que le controleur en a fini"
    );
}

#[test]
fn plusieurs_quarantaines_retiennent_le_tampon_jusqu_a_la_derniere() {
    let mut s = Suivi::neuf();
    let a = s.alloue().unwrap();
    let b = s.alloue().unwrap();
    assert!(s.expire(a));
    assert!(s.expire(b));
    assert!(!s.tampon_disponible());
    assert_eq!(s.range(a, 0, 0), Verdict::Tardif);
    assert!(
        !s.tampon_disponible(),
        "une seule des deux commandes a rendu son tampon ; l'autre est toujours \
         susceptible d'ecrire"
    );
    assert_eq!(s.range(b, 0, 0), Verdict::Tardif);
    assert!(s.tampon_disponible());
}

#[test]
fn la_reinitialisation_rend_le_tampon_sans_achevement() {
    let mut s = Suivi::neuf();
    let cid = s.alloue().unwrap();
    assert!(s.expire(cid));
    assert!(!s.tampon_disponible());
    s.reinitialise();
    assert!(
        s.tampon_disponible(),
        "un controleur remis a zero a perdu ses files : c'est le SECOND \
         evenement, et le dernier, qui leve une quarantaine"
    );
}
