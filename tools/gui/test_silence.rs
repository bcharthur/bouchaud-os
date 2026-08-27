//! Preuve hote du verdict « ce client annonce-t-il ses trames ? ».
//!
//! # Ce qui est teste
//!
//! `src/gui/silence.rs`, inclus tel quel : le code REEL, pas un modele. Le type
//! ne depend de rien -- ni horloge, ni tache, ni framebuffer -- precisement
//! pour cela.
//!
//! # Le scenario qui a casse en production
//!
//! Ladybird met plus de six secondes a demarrer sous TCG. Il depassait donc le
//! delai de patience, etait declare muet, puis se mettait a parler le
//! protocole. Le drapeau « actif » passait a vrai et le drapeau « muet »
//! restait vrai, parce que rien ne le levait.
//!
//! Consequence mesuree sur un intervalle de releve : 94 trames utiles pour 94
//! recompositions aveugles -- exactement les memes. L'inactivite du bureau
//! etait entierement fabriquee par un verdict perime.
//!
//! Lance par `tools/gui/test-silence.sh`.

#[path = "../../src/gui/silence.rs"]
mod silence;

use silence::VerdictProtocole;

#[test]
fn un_client_neuf_ne_declenche_aucune_recomposition_aveugle() {
    let verdict = VerdictProtocole::neuf();
    assert!(!verdict.protocole_actif());
    assert!(
        !verdict.recompose_a_l_aveugle(),
        "avant le delai de patience, on ne devine rien : on attend"
    );
}

#[test]
fn un_client_muet_declenche_la_recomposition_aveugle() {
    let mut verdict = VerdictProtocole::neuf();
    assert!(verdict.declare_muet(), "le verdict change reellement");
    assert!(verdict.recompose_a_l_aveugle());
    assert!(!verdict.protocole_actif());
}

/// LE TEST DU BUG : demarrage lent, verdict de silence, PUIS protocole moderne.
///
/// C'est la sequence exacte de Ladybird sous TCG.
#[test]
fn un_protocole_tardif_revise_le_verdict_de_silence() {
    let mut verdict = VerdictProtocole::neuf();

    // 1. le delai de patience expire avant que Ladybird n'ait fini de demarrer
    verdict.declare_muet();
    assert!(verdict.recompose_a_l_aveugle(), "prealable : il est declare muet");

    // 2. le client finit par dire bonjour / annoncer une trame
    let revision = verdict.marque_protocole_actif();

    assert!(revision, "la revision doit etre signalee, pour pouvoir la journaliser");
    assert!(verdict.protocole_actif());
    assert!(
        !verdict.recompose_a_l_aveugle(),
        "un client qui annonce ses trames ne doit plus jamais etre recompose \
         a l'aveugle : c'est ce qui produisait 94 recompositions pour 94 trames"
    );
}

#[test]
fn la_revision_n_est_signalee_qu_une_fois() {
    let mut verdict = VerdictProtocole::neuf();
    verdict.declare_muet();
    assert!(verdict.marque_protocole_actif(), "premiere revision");
    assert!(
        !verdict.marque_protocole_actif(),
        "les messages suivants ne doivent pas rejournaliser la meme revision"
    );
    assert!(!verdict.recompose_a_l_aveugle());
}

#[test]
fn un_client_qui_parle_d_emblee_n_est_jamais_declare_muet() {
    let mut verdict = VerdictProtocole::neuf();
    assert!(!verdict.marque_protocole_actif(), "rien a reviser");
    assert!(
        !verdict.declare_muet(),
        "un silence passager ne doit pas relancer la recomposition aveugle \
         d'un client parfaitement bavard"
    );
    assert!(!verdict.recompose_a_l_aveugle());
}

/// Les deux etats ne peuvent jamais etre vrais ensemble, quel que soit l'ordre
/// des transitions. C'est l'invariant que deux booleens independants ne
/// tenaient pas.
#[test]
fn les_deux_etats_ne_sont_jamais_vrais_ensemble() {
    // Toutes les sequences de trois transitions parmi les trois possibles.
    for a in 0..3 {
        for b in 0..3 {
            for c in 0..3 {
                let mut verdict = VerdictProtocole::neuf();
                for etape in [a, b, c] {
                    match etape {
                        0 => { verdict.marque_protocole_actif(); }
                        1 => { verdict.declare_muet(); }
                        _ => verdict.retire_le_protocole(),
                    }
                }
                assert!(
                    !(verdict.protocole_actif() && verdict.recompose_a_l_aveugle()),
                    "sequence {a}{b}{c} : les deux etats sont vrais ensemble"
                );
            }
        }
    }
}

/// Un flux invalide retire la preuve, mais ne declare pas muet : c'est au delai
/// de patience de le faire, et lui seul sait s'il a expire.
#[test]
fn un_flux_invalide_retire_la_preuve_sans_declarer_muet() {
    let mut verdict = VerdictProtocole::neuf();
    verdict.marque_protocole_actif();
    verdict.retire_le_protocole();

    assert!(!verdict.protocole_actif());
    assert!(
        !verdict.recompose_a_l_aveugle(),
        "retirer la preuve n'est pas rendre un verdict de silence"
    );

    // ... et le delai de patience peut alors faire son travail.
    assert!(verdict.declare_muet());
    assert!(verdict.recompose_a_l_aveugle());
}

/// Apres un flux invalide puis un verdict de silence, un protocole valide doit
/// encore pouvoir tout reviser. Le cas le plus tordu, et celui qui reste
/// possible : un client qui balbutie avant de parler correctement.
#[test]
fn un_client_qui_balbutie_puis_parle_finit_par_etre_cru() {
    let mut verdict = VerdictProtocole::neuf();
    verdict.marque_protocole_actif();
    verdict.retire_le_protocole();
    verdict.declare_muet();
    assert!(verdict.recompose_a_l_aveugle());

    verdict.marque_protocole_actif();
    assert!(verdict.protocole_actif());
    assert!(!verdict.recompose_a_l_aveugle());
}
