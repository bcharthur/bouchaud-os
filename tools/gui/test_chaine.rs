//! Le veilleur de la chaine `entree -> degat -> trame -> present -> LFB`.
//!
//! Deux proprietes, et elles se contredisent presque :
//!
//!   * il doit nommer le PREMIER maillon rompu, pas le dernier, ni tous ;
//!   * il ne doit pas inonder la trace — une ligne par episode, pas une par
//!     mouvement de souris.
//!
//! Un diagnostic qui parle a chaque evenement est aussi inutile qu'un
//! diagnostic muet : dans les deux cas, on ne trouve rien dans le journal.
//!
//! Lance par `tools/gui/test-chaine.sh`.

#[path = "../../src/gui/chaine.rs"]
mod chaine;

use chaine::{maillon_rompu, Chaine, Maillon, Veilleur, Verdict};

const DELAI: u64 = 500;

/// Une chaine complete : chaque maillon a avance.
fn complete(reference: Chaine) -> Chaine {
    Chaine {
        entrees: reference.entrees + 1,
        degats: reference.degats + 1,
        trames: reference.trames + 1,
        presents: reference.presents + 1,
        copies: reference.copies + 1,
    }
}

fn depart() -> Chaine {
    Chaine { entrees: 10, degats: 20, trames: 30, presents: 40, copies: 50 }
}

// ─── Quel maillon ──────────────────────────────────────────────────────────

#[test]
fn une_chaine_complete_n_a_pas_de_maillon_rompu() {
    let a = depart();
    assert_eq!(maillon_rompu(&a, &complete(a)), None);
}

#[test]
fn rien_n_a_bouge_du_tout() {
    let a = depart();
    assert_eq!(maillon_rompu(&a, &a), Some(Maillon::Entree));
}

#[test]
fn l_entree_arrive_mais_rien_ne_se_salit() {
    let a = depart();
    let mut b = complete(a);
    b.degats = a.degats;
    b.trames = a.trames;
    b.presents = a.presents;
    b.copies = a.copies;
    assert_eq!(maillon_rompu(&a, &b), Some(Maillon::Degat));
}

#[test]
fn le_degat_est_cree_mais_jamais_compose() {
    let a = depart();
    let mut b = complete(a);
    b.trames = a.trames;
    b.presents = a.presents;
    b.copies = a.copies;
    assert_eq!(maillon_rompu(&a, &b), Some(Maillon::Trame));
}

#[test]
fn la_trame_est_composee_mais_jamais_presentee() {
    let a = depart();
    let mut b = complete(a);
    b.presents = a.presents;
    b.copies = a.copies;
    assert_eq!(maillon_rompu(&a, &b), Some(Maillon::Present));
}

/// LE cas trompeur : tout monte, et rien n'atteint l'ecran.
#[test]
fn la_presentation_est_appelee_et_aucun_pixel_n_atteint_le_lfb() {
    let a = depart();
    let mut b = complete(a);
    b.copies = a.copies;
    assert_eq!(
        maillon_rompu(&a, &b),
        Some(Maillon::Copie),
        "frames_composed et presents montent, l'ecran ne change pas"
    );
}

/// Un maillon rompu immobilise tous les suivants. On ne nomme que le premier.
#[test]
fn seul_le_premier_maillon_rompu_est_nomme() {
    let a = depart();
    let mut b = a;
    b.entrees += 1; // seule l'entree a avance
    assert_eq!(maillon_rompu(&a, &b), Some(Maillon::Degat));
}

#[test]
fn chaque_maillon_a_un_nom_et_une_piste() {
    for maillon in [
        Maillon::Entree,
        Maillon::Degat,
        Maillon::Trame,
        Maillon::Present,
        Maillon::Copie,
    ] {
        assert!(!maillon.nom().is_empty());
        assert!(!maillon.piste().is_empty());
    }
}

// ─── Le veilleur ───────────────────────────────────────────────────────────

#[test]
fn un_veilleur_neuf_ne_dit_rien() {
    let mut veilleur = Veilleur::neuf();
    assert!(!veilleur.arme());
    assert_eq!(veilleur.examine(9_000, depart(), DELAI), Verdict::Rien);
}

#[test]
fn il_se_tait_avant_le_delai() {
    let mut veilleur = Veilleur::neuf();
    let a = depart();
    veilleur.note_entree(1_000, a);
    // Rien n'avance, mais il est trop tot pour s'en inquieter.
    for instant in [1_000, 1_100, 1_400, 1_499] {
        assert_eq!(veilleur.examine(instant, a, DELAI), Verdict::Rien, "t={instant}");
    }
    assert_eq!(veilleur.examine(1_500, a, DELAI), Verdict::Rupture(Maillon::Entree));
}

#[test]
fn il_se_tait_quand_la_chaine_va_jusqu_a_l_ecran() {
    let mut veilleur = Veilleur::neuf();
    let a = depart();
    veilleur.note_entree(1_000, a);
    assert_eq!(veilleur.examine(9_000, complete(a), DELAI), Verdict::Rien);
    assert!(!veilleur.arme(), "la surveillance est terminee");
}

/// La propriete anti-inondation : un episode, une ligne.
#[test]
fn il_ne_repete_pas_le_meme_diagnostic() {
    let mut veilleur = Veilleur::neuf();
    let a = depart();
    let mut bloque = a;
    bloque.entrees += 1;
    bloque.degats += 1;
    bloque.trames += 1;
    bloque.presents += 1; // les copies ne montent pas

    veilleur.note_entree(1_000, a);
    assert_eq!(veilleur.examine(2_000, bloque, DELAI), Verdict::Rupture(Maillon::Copie));
    for tour in 0..500 {
        bloque.entrees += 1;
        bloque.presents += 1;
        assert_eq!(
            veilleur.examine(2_001 + tour, bloque, DELAI),
            Verdict::Rien,
            "tour {tour} : le diagnostic ne doit pas se repeter"
        );
    }
}

/// Une entree qui arrive pendant une surveillance ne repousse pas l'echeance.
///
/// Sans cela, un mouvement de souris continu — exactement la situation ou l'on
/// veut savoir — reporterait la mesure indefiniment.
#[test]
fn un_flot_d_entrees_ne_repousse_pas_l_echeance() {
    let mut veilleur = Veilleur::neuf();
    let a = depart();
    let mut courant = a;

    veilleur.note_entree(1_000, a);
    for tour in 1..=60u64 {
        courant.entrees += 1; // la souris bouge, rien d'autre n'avance
        veilleur.note_entree(1_000 + tour * 10, courant);
        let verdict = veilleur.examine(1_000 + tour * 10, courant, DELAI);
        if tour * 10 >= DELAI {
            assert_eq!(
                verdict,
                Verdict::Rupture(Maillon::Degat),
                "au tour {tour} l'echeance est depassee"
            );
            return;
        }
        assert_eq!(verdict, Verdict::Rien, "tour {tour}");
    }
    panic!("l'echeance n'a jamais ete atteinte");
}

#[test]
fn le_retablissement_est_signale_une_fois() {
    let mut veilleur = Veilleur::neuf();
    let a = depart();
    let mut bloque = a;
    bloque.entrees += 1;

    veilleur.note_entree(1_000, a);
    assert_eq!(veilleur.examine(2_000, bloque, DELAI), Verdict::Rupture(Maillon::Degat));
    assert_eq!(
        veilleur.examine(3_000, complete(a), DELAI),
        Verdict::Retabli(Maillon::Degat)
    );
    // Le veilleur est desarme : plus rien tant qu'une entree ne le rearme pas.
    assert_eq!(veilleur.examine(4_000, a, DELAI), Verdict::Rien);
    assert!(!veilleur.arme());
}

#[test]
fn un_retablissement_sans_rupture_signalee_reste_silencieux() {
    let mut veilleur = Veilleur::neuf();
    let a = depart();
    veilleur.note_entree(1_000, a);
    assert_eq!(veilleur.examine(1_050, complete(a), DELAI), Verdict::Rien);
}

/// Le maillon rompu peut se deplacer : chaque nouveau est dit une fois.
#[test]
fn un_maillon_different_merite_une_nouvelle_ligne() {
    let mut veilleur = Veilleur::neuf();
    let a = depart();
    let mut courant = a;
    courant.entrees += 1;

    veilleur.note_entree(1_000, a);
    assert_eq!(veilleur.examine(2_000, courant, DELAI), Verdict::Rupture(Maillon::Degat));
    assert_eq!(veilleur.examine(2_100, courant, DELAI), Verdict::Rien);

    courant.degats += 1;
    courant.trames += 1;
    assert_eq!(
        veilleur.examine(2_200, courant, DELAI),
        Verdict::Rupture(Maillon::Present),
        "le blocage a avance : il merite d'etre redit"
    );
    assert_eq!(veilleur.examine(2_300, courant, DELAI), Verdict::Rien);
}

/// Apres un episode complet, un nouvel episode doit pouvoir parler.
#[test]
fn un_nouvel_episode_peut_parler_a_nouveau() {
    let mut veilleur = Veilleur::neuf();
    let a = depart();
    let mut courant = a;
    courant.entrees += 1;

    veilleur.note_entree(1_000, a);
    assert_eq!(veilleur.examine(2_000, courant, DELAI), Verdict::Rupture(Maillon::Degat));

    let retabli = complete(courant);
    assert_eq!(veilleur.examine(2_500, retabli, DELAI), Verdict::Retabli(Maillon::Degat));

    // Deuxieme episode, meme symptome : il doit se dire.
    let mut bis = retabli;
    veilleur.note_entree(3_000, bis);
    bis.entrees += 1;
    assert_eq!(veilleur.examine(4_000, bis, DELAI), Verdict::Rupture(Maillon::Degat));
}

/// Le temps monotone du noyau peut deborder ; le veilleur ne doit pas s'affoler.
#[test]
fn un_debordement_du_temps_ne_declenche_rien_de_faux() {
    let mut veilleur = Veilleur::neuf();
    let a = depart();
    veilleur.note_entree(u64::MAX - 100, a);
    // 50 ms plus tard, en debordant : le delai n'est pas atteint.
    assert_eq!(veilleur.examine(u64::MAX.wrapping_add(51), a, DELAI), Verdict::Rien);
    assert_eq!(
        veilleur.examine(u64::MAX.wrapping_add(600), a, DELAI),
        Verdict::Rupture(Maillon::Entree)
    );
}
