//! Une connexion BRDP garde-t-elle l'ORDRE, et une commande refusee se
//! dit-elle ?
//!
//! # Ce que ces epreuves defendent
//!
//! Le client rattache les reponses aux commandes dans l'ordre d'emission :
//! c'est le seul ordre dont il dispose, puisque le protocole ne numerote pas
//! les reponses. Tout ce qui derange cet ordre lui fait attribuer chaque
//! reponse a la mauvaise commande, pour tout le reste de la connexion -- et il
//! n'a aucun moyen de s'en apercevoir.
//!
//! Trois defauts s'y attaquaient :
//!
//! 1. **une seule commande par segment.** TCP est un flux ; rien n'interdit au
//!    noyau distant de coller trois commandes dans un segment, et c'est meme
//!    ce qu'il fait des que le client enchaine. Les suivantes etaient perdues,
//!    silencieusement ;
//!
//! 2. **la ligne trop longue signalee APRES le segment.** Un drapeau leve
//!    pendant le decoupage et honore ensuite faisait sortir
//!    `ligne-trop-longue` derriere la commande qui la SUIVAIT dans le flux ;
//!
//! 3. **`events tail N` qui n'envoyait rien.** Le mode etait un booleen ;
//!    `tail` posait le curseur puis mettait « suit les evenements » a faux,
//!    donc aucune routine ne vidait jamais ce qu'on venait de demander.
//!
//! Lance par `tools/ci/run_host_tests.sh`.

extern crate alloc;

#[path = "../../src/net/security/tls/sha256.rs"]
pub mod sha256;

// `brdp.rs` ecrit `use crate::net::security::tls::sha256` : on recree le
// chemin ici plutot que de modifier la source pour lui plaire. Un test qui
// oblige a tordre le code qu'il teste ne teste plus le meme code.
pub mod net {
    pub mod security {
        pub mod tls {
            pub use crate::sha256;
        }
    }
}

#[path = "../../src/net/diag_distant/brdp.rs"]
pub mod brdp;

#[path = "../../src/net/diag_distant/session.rs"]
pub mod session;

use brdp::{Decoupeur, LIGNE_MAX};
use session::{
    avale, FileCommandes, ModeEvenements, COMMANDES_EN_ATTENTE, LIGNE_TROP_LONGUE,
};

/// La borne d'evenements servis par tour de boucle du serveur.
const PAR_TOUR: u32 = 16;

/// Vide la file et rend les lignes, dans l'ordre de sortie.
fn draine(file: &mut FileCommandes) -> Vec<Vec<u8>> {
    let mut sortie = Vec::new();
    let mut ligne = [0u8; LIGNE_MAX];
    while let Some(n) = file.retire(&mut ligne) {
        sortie.push(ligne[..n].to_vec());
    }
    sortie
}

/// Verse une suite de segments, et rend les lignes mises en file.
fn flux(segments: &[&[u8]]) -> (Vec<Vec<u8>>, bool) {
    let mut d = Decoupeur::neuf();
    let mut f = FileCommandes::neuve();
    let mut deborde = false;
    for s in segments {
        deborde |= avale(&mut d, &mut f, s);
    }
    (draine(&mut f), deborde)
}

fn texte(lignes: &[Vec<u8>]) -> Vec<String> {
    lignes.iter().map(|l| String::from_utf8_lossy(l).into_owned()).collect()
}

// ===========================================================================
// L'ORDRE : CE QUE TCP NE GARANTIT PAS
// ===========================================================================

#[test]
fn trois_commandes_dans_un_seul_segment_arrivent_toutes_et_dans_l_ordre() {
    // LE DEFAUT ORIGINEL. Le serveur n'en gardait qu'une et perdait les deux
    // autres, sans rien dire au client qui les attendait.
    let (lignes, deborde) = flux(&[
        br#"{"cmd":"status"}
{"cmd":"audit status"}
{"cmd":"blackbox status"}
"#,
    ]);
    assert!(!deborde);
    assert_eq!(
        texte(&lignes),
        vec![
            r#"{"cmd":"status"}"#,
            r#"{"cmd":"audit status"}"#,
            r#"{"cmd":"blackbox status"}"#,
        ],
    );
}

#[test]
fn une_commande_fragmentee_sur_plusieurs_segments_arrive_entiere() {
    // LE CAS INVERSE, et il se produit sur un reseau charge. Un segment par
    // caractere est le pire cas, et rien ne l'interdit.
    let commande = br#"{"cmd":"rtl8168 ring"}"#;
    let mut segments: Vec<&[u8]> = commande.chunks(1).collect();
    segments.push(b"\n");
    let (lignes, deborde) = flux(&segments);
    assert!(!deborde);
    assert_eq!(texte(&lignes), vec![r#"{"cmd":"rtl8168 ring"}"#]);
}

#[test]
fn une_commande_a_cheval_sur_deux_segments_n_est_pas_coupee() {
    let (lignes, _) = flux(&[br#"{"cmd":"sta"#, b"tus\"}\n{\"cmd\":\"quit\"}\n"]);
    assert_eq!(texte(&lignes), vec![r#"{"cmd":"status"}"#, r#"{"cmd":"quit"}"#]);
}

#[test]
fn le_retour_chariot_de_windows_ne_fait_pas_echouer_une_commande() {
    let (lignes, _) = flux(&[b"{\"cmd\":\"status\"}\r\n"]);
    assert_eq!(texte(&lignes), vec![r#"{"cmd":"status"}"#]);
}

#[test]
fn une_ligne_vide_reste_une_ligne_et_garde_sa_place() {
    // Un client qui envoie `\n\n` n'est pas hostile ; sa ligne vide est
    // refusee par l'analyseur, pas escamotee par le decoupage.
    let (lignes, _) = flux(&[b"\n{\"cmd\":\"status\"}\n"]);
    assert_eq!(lignes.len(), 2);
    assert!(lignes[0].is_empty());
    assert_eq!(texte(&lignes)[1], r#"{"cmd":"status"}"#);
}

// ===========================================================================
// LA LIGNE TROP LONGUE : A SA PLACE DANS LE FLUX
// ===========================================================================

#[test]
fn une_ligne_trop_longue_precede_la_commande_qui_la_suit() {
    // L'EPREUVE DE L'ORDRE. Une premiere redaction levait un drapeau pendant
    // le decoupage et poussait la sentinelle APRES : `status` entrait en file
    // en premier, et le client recevait sa reponse AVANT `ligne-trop-longue`.
    // Il attribuait alors chaque reponse a la mauvaise commande, pour tout le
    // reste de la connexion.
    let mut segment = vec![b'x'; LIGNE_MAX + 100];
    segment.push(b'\n');
    segment.extend_from_slice(br#"{"cmd":"status"}"#);
    segment.push(b'\n');

    let (lignes, deborde) = flux(&[&segment]);
    assert!(!deborde);
    assert_eq!(lignes.len(), 2);
    assert_eq!(lignes[0], LIGNE_TROP_LONGUE, "la sentinelle sort EN PREMIER");
    assert_eq!(texte(&lignes)[1], r#"{"cmd":"status"}"#);
}

#[test]
fn une_ligne_trop_longue_entre_deux_commandes_garde_sa_position() {
    let mut segment = Vec::new();
    segment.extend_from_slice(b"{\"cmd\":\"status\"}\n");
    segment.extend_from_slice(&vec![b'y'; LIGNE_MAX + 1]);
    segment.push(b'\n');
    segment.extend_from_slice(b"{\"cmd\":\"quit\"}\n");

    let (lignes, _) = flux(&[&segment]);
    assert_eq!(lignes.len(), 3);
    assert_eq!(texte(&lignes)[0], r#"{"cmd":"status"}"#);
    assert_eq!(lignes[1], LIGNE_TROP_LONGUE);
    assert_eq!(texte(&lignes)[2], r#"{"cmd":"quit"}"#);
}

#[test]
fn deux_lignes_trop_longues_de_suite_donnent_deux_sentinelles() {
    let mut segment = Vec::new();
    for _ in 0..2 {
        segment.extend_from_slice(&vec![b'z'; LIGNE_MAX + 5]);
        segment.push(b'\n');
    }
    let (lignes, _) = flux(&[&segment]);
    assert_eq!(lignes, vec![LIGNE_TROP_LONGUE.to_vec(), LIGNE_TROP_LONGUE.to_vec()]);
}

#[test]
fn le_flux_n_est_pas_desynchronise_par_une_ligne_trop_longue() {
    // Tout est jete jusqu'au prochain retour a la ligne, et pas au-dela : un
    // client qui bafouille une fois doit pouvoir continuer.
    let mut segment = vec![b'w'; LIGNE_MAX * 3];
    segment.push(b'\n');
    segment.extend_from_slice(b"{\"cmd\":\"status\"}\n{\"cmd\":\"quit\"}\n");
    let (lignes, _) = flux(&[&segment]);
    assert_eq!(lignes.len(), 3);
    assert_eq!(lignes[0], LIGNE_TROP_LONGUE);
    assert_eq!(texte(&lignes)[1], r#"{"cmd":"status"}"#);
    assert_eq!(texte(&lignes)[2], r#"{"cmd":"quit"}"#);
}

#[test]
fn la_sentinelle_ne_peut_pas_etre_une_vraie_commande() {
    // Le protocole n'accepte que des objets JSON : une ligne utile commence
    // par `{`. La sentinelle commence par un octet nul.
    assert_eq!(LIGNE_TROP_LONGUE[0], 0);
    assert!(LIGNE_TROP_LONGUE.len() <= LIGNE_MAX);
    assert!(brdp::analyse(LIGNE_TROP_LONGUE).is_err());
}

// ===========================================================================
// LA FILE BORNEE : ON REFUSE LA PLUS RECENTE, ET ON LE DIT
// ===========================================================================

#[test]
fn la_file_accepte_exactement_sa_capacite() {
    let mut f = FileCommandes::neuve();
    for i in 0..COMMANDES_EN_ATTENTE {
        assert!(f.pousse(format!("{i}").as_bytes()), "refus au rang {i}");
    }
    assert!(f.pleine());
    assert_eq!(f.occupation(), COMMANDES_EN_ATTENTE);
    assert_eq!(f.refusees(), 0);
}

#[test]
fn une_file_pleine_refuse_la_plus_recente_et_garde_les_anciennes() {
    // UN ANNEAU D'EVENEMENTS PEUT ECRASER, UNE FILE DE COMMANDES NON. Ecraser
    // la plus ancienne ferait repondre au client dans un ordre qu'il n'a pas
    // demande, sans qu'il puisse le distinguer d'une perte reseau.
    let mut f = FileCommandes::neuve();
    for i in 0..COMMANDES_EN_ATTENTE {
        assert!(f.pousse(format!("ancienne-{i}").as_bytes()));
    }
    assert!(!f.pousse(b"la-plus-recente"), "la nouvelle est refusee");
    assert_eq!(f.refusees(), 1);

    let sorties = texte(&draine(&mut f));
    assert_eq!(sorties.len(), COMMANDES_EN_ATTENTE);
    assert_eq!(sorties[0], "ancienne-0", "la plus ancienne est intacte");
    assert_eq!(sorties[COMMANDES_EN_ATTENTE - 1], "ancienne-7");
    assert!(!sorties.iter().any(|s| s == "la-plus-recente"));
}

#[test]
fn le_debordement_remonte_a_l_appelant() {
    let mut segment = Vec::new();
    for i in 0..(COMMANDES_EN_ATTENTE + 3) {
        segment.extend_from_slice(format!("{{\"cmd\":\"c{i}\"}}\n").as_bytes());
    }
    let (lignes, deborde) = flux(&[&segment]);
    assert!(deborde, "le client DOIT l'apprendre");
    assert_eq!(lignes.len(), COMMANDES_EN_ATTENTE);
}

#[test]
fn une_ligne_plus_longue_que_la_borne_est_refusee_par_la_file() {
    let mut f = FileCommandes::neuve();
    assert!(!f.pousse(&vec![b'a'; LIGNE_MAX + 1]));
    assert!(f.est_vide());
    assert!(f.pousse(&vec![b'a'; LIGNE_MAX]), "la borne est inclusive");
}

#[test]
fn la_file_tourne_sans_melanger_l_ordre() {
    let mut f = FileCommandes::neuve();
    let mut ligne = [0u8; LIGNE_MAX];
    for tour in 0..5u32 {
        for i in 0..COMMANDES_EN_ATTENTE {
            assert!(f.pousse(format!("t{tour}-{i}").as_bytes()));
        }
        for i in 0..COMMANDES_EN_ATTENTE {
            let n = f.retire(&mut ligne).expect("la file devait contenir une ligne");
            assert_eq!(&ligne[..n], format!("t{tour}-{i}").as_bytes());
        }
        assert!(f.est_vide());
    }
}

#[test]
fn purge_vide_la_file_mais_garde_le_compteur_de_refus() {
    // `refusees` EST UN COMPTEUR DE SERVICE, pas un etat de connexion. Le
    // remettre a zero effacerait la trace d'un client qui maltraite le
    // serveur a chaque reconnexion.
    let mut f = FileCommandes::neuve();
    for i in 0..COMMANDES_EN_ATTENTE {
        assert!(f.pousse(format!("{i}").as_bytes()));
    }
    assert!(!f.pousse(b"de-trop"));
    assert_eq!(f.refusees(), 1);

    f.purge();
    assert!(f.est_vide());
    assert_eq!(f.occupation(), 0);
    assert_eq!(f.refusees(), 1, "le compteur survit a la reconnexion");
    assert!(f.pousse(b"apres-purge"));
}

// ===========================================================================
// LE MODE D'EVENEMENTS : UN ETAT, PAS UN BOOLEEN
// ===========================================================================

/// Rejoue l'arithmetique de `serveur::verse_des_evenements` sur un anneau qui
/// contient `disponibles` evenements. Rend le total servi et le mode final.
fn sert(mut mode: ModeEvenements, disponibles: u32) -> (u32, ModeEvenements, u32) {
    let mut reste = disponibles;
    let mut total = 0u32;
    let mut tours = 0u32;
    loop {
        if !mode.actif() {
            break;
        }
        tours += 1;
        assert!(tours < 1_000, "boucle : le mode ne s'epuise jamais");
        let budget = mode.budget(PAR_TOUR);
        let servis = budget.min(reste);
        reste -= servis;
        total += servis;
        let epuise = servis < budget;
        mode = mode.consomme(servis);
        if epuise && mode.finit_sur_vide() {
            mode = ModeEvenements::Aucun;
        }
        if epuise {
            break;
        }
    }
    (total, mode, tours)
}

#[test]
fn le_mode_par_defaut_n_envoie_rien() {
    let m = ModeEvenements::default();
    assert_eq!(m, ModeEvenements::Aucun);
    assert!(!m.actif());
    assert_eq!(m.budget(PAR_TOUR), 0);
}

#[test]
fn tail_cinq_rend_exactement_cinq() {
    // `events tail 5` en rend CINQ, pas un de plus. Le mode prend le plus
    // petit de son reste et de la borne du tour.
    let (total, fin, _) = sert(ModeEvenements::Queue { restant: 5 }, 1_000);
    assert_eq!(total, 5);
    assert_eq!(fin, ModeEvenements::Aucun, "puis il se tait");
}

#[test]
fn tail_cent_sur_vingt_rend_vingt_puis_aucun() {
    // UN `tail` QUI DEMANDE PLUS QUE L'ANNEAU N'A NE RESTE PAS PENDU. Sans
    // `finit_sur_vide`, il attendrait indefiniment les quatre-vingts
    // evenements qui n'existent pas.
    let (total, fin, _) = sert(ModeEvenements::Queue { restant: 100 }, 20);
    assert_eq!(total, 20);
    assert_eq!(fin, ModeEvenements::Aucun);
}

#[test]
fn tail_cent_sur_un_anneau_fourni_rend_cent_en_plusieurs_tours() {
    let (total, fin, tours) = sert(ModeEvenements::Queue { restant: 100 }, 1_000);
    assert_eq!(total, 100);
    assert_eq!(fin, ModeEvenements::Aucun);
    assert_eq!(tours, 100u32.div_ceil(PAR_TOUR), "borne par tour respectee");
}

#[test]
fn watch_reste_actif_meme_quand_l_anneau_se_tait() {
    // C'EST SON TRAVAIL D'ATTENDRE. Un `watch` qui s'arreterait au premier
    // creux ne suivrait rien du tout.
    let (total, fin, _) = sert(ModeEvenements::Suivi, 3);
    assert_eq!(total, 3);
    assert_eq!(fin, ModeEvenements::Suivi, "le suivi survit au vide");
    assert!(fin.actif());
    assert!(!fin.finit_sur_vide());
}

#[test]
fn watch_est_borne_par_tour() {
    let m = ModeEvenements::Suivi;
    assert_eq!(m.budget(PAR_TOUR), PAR_TOUR);
    assert_eq!(m.consomme(PAR_TOUR), ModeEvenements::Suivi);
    assert_eq!(m.consomme(0), ModeEvenements::Suivi);
}

#[test]
fn une_queue_partiellement_servie_garde_son_reste() {
    let m = ModeEvenements::Queue { restant: 10 };
    assert_eq!(m.budget(PAR_TOUR), 10);
    assert_eq!(m.consomme(4), ModeEvenements::Queue { restant: 6 });
    assert_eq!(m.consomme(10), ModeEvenements::Aucun);
    assert_eq!(m.consomme(99), ModeEvenements::Aucun, "on ne passe pas sous zero");
}

#[test]
fn une_queue_de_zero_est_deja_finie() {
    let m = ModeEvenements::Queue { restant: 0 };
    assert_eq!(m.budget(PAR_TOUR), 0);
    let (total, fin, _) = sert(m, 100);
    assert_eq!(total, 0);
    assert_eq!(fin, ModeEvenements::Aucun);
}

#[test]
fn aucun_ne_sert_rien_et_ne_boucle_pas() {
    let (total, fin, tours) = sert(ModeEvenements::Aucun, 1_000);
    assert_eq!(total, 0);
    assert_eq!(fin, ModeEvenements::Aucun);
    assert_eq!(tours, 0);
}

#[test]
fn watch_apres_un_tail_inacheve_remplace_le_reste_au_lieu_de_le_cumuler() {
    // UN SECOND `events` ECRASE LE PREMIER. Le serveur affecte le mode, il ne
    // l'additionne pas : un `watch` demande apres un `tail 5` a moitie servi
    // suit le flux, il ne sert pas d'abord les trois evenements restants. Le
    // client a change d'avis, et c'est le dernier avis qui vaut.
    let interrompu = ModeEvenements::Queue { restant: 5 }.consomme(2);
    assert_eq!(interrompu, ModeEvenements::Queue { restant: 3 });

    let apres = ModeEvenements::Suivi;
    assert!(apres.actif());
    assert!(!apres.finit_sur_vide(), "le suivi n'a pas de reste a epuiser");
    assert_eq!(apres.budget(PAR_TOUR), PAR_TOUR, "et pas le budget d'un reste de 3");

    let (total, fin, _) = sert(apres, 2);
    assert_eq!(total, 2);
    assert_eq!(fin, ModeEvenements::Suivi);
}
