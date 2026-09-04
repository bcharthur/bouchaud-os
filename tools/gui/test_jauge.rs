//! Preuve hote de la jauge de chargement.
//!
//! # Ce qui est teste
//!
//! `src/gui/jauge.rs`, inclus tel quel : le code REEL. Le module ne depend ni
//! d'une horloge, ni d'une tache, ni du framebuffer -- exactement pour cela.
//!
//! # Ce que ces tests protegent
//!
//! Une jauge de chargement est un objet qui MENT facilement, et dont le
//! mensonge ne se voit pas : il faudrait chronometrer l'ecran a la main pour
//! s'en apercevoir. Trois mensonges sont possibles, un test pour chacun :
//!
//!   1. couper le chronometre au milieu d'un rendu progressif et annoncer une
//!      duree trop courte -- le sens ou une mesure de performance se flatte ;
//!   2. rallumer la barre a chaque cran de molette, jusqu'a ce qu'elle ne
//!      veuille plus rien dire ;
//!   3. afficher 100 % avant que ce soit fini.
//!
//! Lance par `tools/gui/test-jauge.sh`.

#[path = "../../src/gui/jauge.rs"]
mod jauge;

use jauge::{
    formate_duree, Jauge, Phase, AFFICHAGE_MS, FENETRE_INTERACTION_MS, PLAFOND_CHARGE_MS,
    SEUIL_REPOS_MS,
};

// ─── Demarrage ─────────────────────────────────────────────────────────────

#[test]
fn une_jauge_neuve_chronometre_le_demarrage() {
    let jauge = Jauge::neuve(1_000);
    assert_eq!(jauge.phase(), Phase::Demarrage);
    assert!(jauge.visible(), "le demarrage se voit : c'est ce qui est lent");
    assert_eq!(jauge.demarrage_ms(), None, "rien n'est encore mesure");
    assert_eq!(jauge.duree_affichee_ms(3_500), 2_500);
}

#[test]
fn la_premiere_trame_fixe_la_duree_de_demarrage() {
    let mut jauge = Jauge::neuve(1_000);
    jauge.note_trame(9_400);
    assert_eq!(jauge.phase(), Phase::Termine);
    assert_eq!(jauge.demarrage_ms(), Some(8_400));
    assert_eq!(jauge.duree_affichee_ms(9_400), 8_400);
    assert_eq!(jauge.progression(9_400), 100);
}

#[test]
fn un_demarrage_abandonne_ne_chronometre_pas_indefiniment() {
    // Le compositeur declare le client muet apres six secondes et compose sa
    // surface a l'aveugle. Sans cette bascule, la jauge afficherait une
    // seconde de plus toutes les secondes, pour toujours.
    let mut jauge = Jauge::neuve(0);
    jauge.abandonne_demarrage();
    assert_eq!(jauge.phase(), Phase::Repos);
    assert!(!jauge.visible());
    assert_eq!(jauge.demarrage_ms(), None, "aucune duree n'a ete mesuree");
}

// ─── Chargement d'une page ─────────────────────────────────────────────────

/// Amene une jauge apres le demarrage, au repos, a la date rendue.
fn apres_demarrage() -> (Jauge, u64) {
    let mut jauge = Jauge::neuve(0);
    jauge.note_trame(1_000);
    let repos = 1_000 + AFFICHAGE_MS;
    jauge.tic(repos);
    assert_eq!(jauge.phase(), Phase::Repos);
    (jauge, repos)
}

#[test]
fn une_rafale_spontanee_est_un_chargement() {
    let (mut jauge, t) = apres_demarrage();
    jauge.note_trame(t + 10);
    assert_eq!(jauge.phase(), Phase::Charge);
    assert_eq!(jauge.duree_affichee_ms(t + 510), 500);
}

#[test]
fn un_rendu_progressif_n_est_pas_coupe_en_deux() {
    // Le mensonge n°1. Une page se peint par a-coups ; le chronometre ne doit
    // s'arreter qu'apres un VRAI silence, pas au premier trou.
    let (mut jauge, t) = apres_demarrage();
    jauge.note_trame(t);
    let mut date = t;
    for _ in 0..6 {
        date += SEUIL_REPOS_MS - 50;
        jauge.tic(date);
        assert_eq!(jauge.phase(), Phase::Charge, "un trou n'est pas une fin");
        jauge.note_trame(date);
    }
    jauge.tic(date + SEUIL_REPOS_MS);
    assert_eq!(jauge.phase(), Phase::Termine);
    assert_eq!(
        jauge.duree_affichee_ms(date + SEUIL_REPOS_MS),
        date - t,
        "la duree s'arrete a la DERNIERE trame, pas a la detection du repos"
    );
}

#[test]
fn le_seuil_de_repos_n_est_jamais_compte_dans_la_duree() {
    let (mut jauge, t) = apres_demarrage();
    jauge.note_trame(t);
    jauge.note_trame(t + 300);
    jauge.tic(t + 300 + SEUIL_REPOS_MS);
    assert_eq!(
        jauge.duree_affichee_ms(t + 5_000),
        300,
        "ajouter le seuil ferait payer a chaque page une demi-seconde qu'elle \
         n'a pas passee a charger"
    );
}

#[test]
fn defiler_ne_rallume_pas_la_jauge() {
    // Le mensonge n°2. Molette et clics produisent la meme rafale de trames
    // qu'un chargement ; seule l'entree les distingue.
    let (mut jauge, t) = apres_demarrage();
    let mut date = t;
    for _ in 0..40 {
        jauge.note_entree(date);
        jauge.note_trame(date + 5);
        jauge.tic(date + 5);
        assert_eq!(
            jauge.phase(),
            Phase::Repos,
            "une trame qui suit une entree est une interaction"
        );
        date += 30;
    }
}

#[test]
fn une_rafale_bien_apres_la_derniere_entree_reste_un_chargement() {
    // La reciproque du test precedent : le filtre ne doit pas rendre la jauge
    // aveugle a une page qui se recharge seule apres une interaction.
    let (mut jauge, t) = apres_demarrage();
    jauge.note_entree(t);
    jauge.note_trame(t + FENETRE_INTERACTION_MS);
    assert_eq!(jauge.phase(), Phase::Charge);
}

#[test]
fn un_changement_de_titre_demarre_un_chargement_meme_apres_une_frappe() {
    // Valider une URL, c'est taper puis appuyer sur Entree : l'entree est
    // fraiche, et pourtant c'est bien une navigation. Le titre tranche.
    let (mut jauge, t) = apres_demarrage();
    jauge.note_entree(t);
    jauge.note_titre(t + 1);
    assert_eq!(jauge.phase(), Phase::Charge);
    assert_eq!(jauge.duree_affichee_ms(t + 801), 800);
}

#[test]
fn un_titre_pendant_le_demarrage_ne_casse_pas_le_chronometre_de_demarrage() {
    let mut jauge = Jauge::neuve(0);
    jauge.note_titre(500);
    assert_eq!(jauge.phase(), Phase::Demarrage);
    jauge.note_trame(2_000);
    assert_eq!(jauge.demarrage_ms(), Some(2_000));
}

#[test]
fn deux_pages_separees_par_un_silence_sont_deux_chargements() {
    let (mut jauge, t) = apres_demarrage();
    jauge.note_trame(t);
    jauge.tic(t + SEUIL_REPOS_MS);
    assert_eq!(jauge.phase(), Phase::Termine);

    let seconde = t + SEUIL_REPOS_MS + SEUIL_REPOS_MS;
    jauge.note_trame(seconde);
    assert_eq!(jauge.phase(), Phase::Charge);
    assert_eq!(jauge.duree_affichee_ms(seconde + 120), 120);
}

#[test]
fn la_duree_mesuree_ne_depend_pas_de_la_cadence_des_tics() {
    // La propriete qui rend le nombre CREDIBLE. `tic` est appele par le
    // compositeur, dont la cadence varie avec la charge -- c'est-a-dire avec
    // exactement ce que la jauge pretend mesurer. Si la duree en dependait,
    // elle mesurerait le compositeur et non la page.
    let trames: [u64; 6] = [0, 120, 260, 300, 700, 980];

    let mesure = |pas_du_tic: u64| -> u64 {
        let (mut jauge, base) = apres_demarrage();
        let mut date = base;
        let fin = base + trames[trames.len() - 1] + 4 * SEUIL_REPOS_MS;
        let mut prochaine = 0usize;
        while date <= fin {
            while prochaine < trames.len() && base + trames[prochaine] <= date {
                jauge.note_trame(base + trames[prochaine]);
                prochaine += 1;
            }
            jauge.tic(date);
            date += pas_du_tic;
        }
        assert_eq!(jauge.phase(), Phase::Termine);
        jauge.duree_affichee_ms(fin)
    };

    let reference = mesure(1);
    assert_eq!(reference, 980, "du debut de la rafale a sa derniere trame");
    for pas in [2u64, 5, 16, 37, 100, 250] {
        assert_eq!(mesure(pas), reference, "tic tous les {} ms", pas);
    }
}

#[test]
fn une_page_qui_s_anime_sans_fin_ne_rallume_pas_la_jauge() {
    // Une video ou une animation CSS produit des trames sans jamais se taire.
    // Le critere de repos ne peut pas conclure ; la jauge doit se taire plutot
    // que d'afficher un chronometre qui ne s'arrete plus.
    let (mut jauge, t) = apres_demarrage();
    jauge.note_trame(t);
    let mut date = t;
    let mut vue_en_charge_apres_le_plafond = false;
    while date < t + PLAFOND_CHARGE_MS + 60_000 {
        date += 33;
        jauge.note_trame(date);
        jauge.tic(date);
        if date > t + PLAFOND_CHARGE_MS + SEUIL_REPOS_MS && jauge.visible() {
            vue_en_charge_apres_le_plafond = true;
        }
    }
    assert!(
        !vue_en_charge_apres_le_plafond,
        "passe le plafond, la jauge doit se taire et le rester"
    );

    // Et elle redevient bavarde des que la page se repose vraiment.
    let reprise = date + 5 * SEUIL_REPOS_MS;
    jauge.note_trame(reprise);
    assert_eq!(jauge.phase(), Phase::Charge);
}

#[test]
fn le_resultat_s_efface_apres_son_temps_d_affichage() {
    let (mut jauge, t) = apres_demarrage();
    jauge.note_trame(t);
    jauge.tic(t + SEUIL_REPOS_MS);
    jauge.tic(t + AFFICHAGE_MS - 1);
    assert!(jauge.visible(), "avant l'echeance, la duree reste lisible");
    jauge.tic(t + AFFICHAGE_MS + SEUIL_REPOS_MS);
    assert_eq!(jauge.phase(), Phase::Repos);
    assert!(!jauge.visible());
}

// ─── Progression ───────────────────────────────────────────────────────────

#[test]
fn la_progression_n_atteint_jamais_cent_avant_la_fin() {
    // Le mensonge n°3.
    let (mut jauge, t) = apres_demarrage();
    jauge.note_trame(t);
    for seconde in 0..600u64 {
        let date = t + seconde * 1_000;
        assert!(
            jauge.progression(date) < 100,
            "a {} s la barre annonce deja la fin",
            seconde
        );
    }
}

#[test]
fn la_progression_croit_et_reste_bornee() {
    let (mut jauge, t) = apres_demarrage();
    jauge.note_trame(t);
    let mut precedente = 0u8;
    for pas in 0..2_000u64 {
        let courante = jauge.progression(t + pas * 10);
        assert!(courante >= precedente, "la barre ne doit jamais reculer");
        precedente = courante;
    }
    assert!(precedente > 90, "elle doit bien finir par remplir la barre");
}

#[test]
fn la_progression_est_nulle_au_repos_et_pleine_a_la_fin() {
    let (mut jauge, t) = apres_demarrage();
    assert_eq!(jauge.progression(t), 0);
    jauge.note_trame(t);
    jauge.tic(t + SEUIL_REPOS_MS);
    assert_eq!(jauge.progression(t + SEUIL_REPOS_MS), 100);
}

#[test]
fn le_demarrage_progresse_plus_lentement_qu_une_page() {
    // Deux demi-vies distinctes : un demarrage de quatre secondes est normal,
    // une page de quatre secondes ne l'est pas. Une barre unique aurait fait
    // paraitre l'un fige et l'autre instantane.
    let demarrage = Jauge::neuve(0);
    let (mut page, t) = apres_demarrage();
    page.note_trame(t);
    assert!(page.progression(t + 1_000) > demarrage.progression(1_000));
}

// ─── Mise en forme ─────────────────────────────────────────────────────────

#[test]
fn les_durees_se_lisent_dans_l_unite_qui_convient() {
    assert_eq!(formate_duree(0).as_str(), "0 ms");
    assert_eq!(formate_duree(612).as_str(), "612 ms");
    assert_eq!(formate_duree(999).as_str(), "999 ms");
    assert_eq!(formate_duree(1_000).as_str(), "1,00 s");
    assert_eq!(formate_duree(1_845).as_str(), "1,84 s");
    assert_eq!(formate_duree(9_999).as_str(), "9,99 s");
    assert_eq!(formate_duree(12_340).as_str(), "12,3 s");
    assert_eq!(formate_duree(59_999).as_str(), "59,9 s");
    assert_eq!(formate_duree(60_000).as_str(), "1 min 00 s");
    assert_eq!(formate_duree(184_000).as_str(), "3 min 04 s");
}

#[test]
fn aucune_duree_ne_deborde_le_tampon() {
    // Le tampon est fixe : le rendu d'une trame ne doit pas dependre de
    // l'allocateur, precisement parce que la jauge se dessine quand le
    // systeme est charge.
    for ms in [u64::MAX, u64::MAX / 2, 3_600_000, 86_400_000] {
        let rendu = formate_duree(ms);
        assert!(!rendu.as_str().is_empty(), "{} ne rend rien", ms);
        assert!(rendu.as_str().len() <= 16);
    }
}
