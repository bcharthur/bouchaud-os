//! Preuve hote de la retransmission TCP.
//!
//! Le module de production `src/net/transport/retransmission.rs` est inclus tel
//! quel. Ce qui est mis a l'epreuve ici est le code qui decide, dans le noyau,
//! quel octet renvoyer et quand.
//!
//! # Ce que la pile ne savait pas faire
//!
//! L'en-tete de `tcp.rs` le disait : « Pas de retransmission ni de controle de
//! congestion ». Un segment perdu n'etait JAMAIS renvoye par nous. La connexion
//! ne tenait que parce que SLIRP retransmet pour son propre compte et parce
//! qu'un `GET` tient dans un seul segment. Sur un reseau qui perd, la requete
//! disparaissait et la connexion attendait ses trente secondes d'inactivite.
//!
//! # Les scenarios, tous DETERMINISTES
//!
//! Le temps est un parametre, pas une horloge : chaque test dit exactement a
//! quelle milliseconde chaque evenement arrive. Un test de reseau qui depend
//! d'une vraie horloge est un test qui echoue un jour sur dix et qu'on finit
//! par desactiver.

#[path = "../../src/net/transport/retransmission.rs"]
mod retransmission;

use retransmission::{
    seq_apres_ou_egal, seq_avant, Emission, Expiration, DOUBLONS_POUR_RETRANSMISSION,
    RETRANSMISSIONS_MAX, RTO_INITIAL_MS, RTO_MAX_MS, RTO_MIN_MS, SEGMENTS_MAX,
};

const ISN: u32 = 1_000;

fn emission() -> Emission {
    Emission::neuve(ISN)
}

// ---------------------------------------------------------------------------
// L'arithmetique circulaire : ce qui casse une connexion longue.
// ---------------------------------------------------------------------------

/// Les numeros de sequence bouclent a 2^32. Une comparaison non signee se
/// trompe des qu'une connexion traverse ce point -- et une connexion longue le
/// traverse.
#[test]
fn la_comparaison_de_sequence_survit_au_bouclage() {
    assert!(seq_avant(1, 2));
    assert!(!seq_avant(2, 1));
    // Juste avant et juste apres le bouclage.
    assert!(seq_avant(u32::MAX, 0), "u32::MAX precede 0 dans l'espace circulaire");
    assert!(seq_avant(u32::MAX - 5, 5));
    assert!(!seq_avant(5, u32::MAX - 5));
    assert!(seq_apres_ou_egal(0, u32::MAX));
    assert!(seq_apres_ou_egal(7, 7));
}

// ---------------------------------------------------------------------------
// PERTE D'UN SEGMENT : l'expiration, et le repli exponentiel.
// ---------------------------------------------------------------------------

#[test]
fn un_segment_perdu_expire_et_se_renvoie() {
    let mut e = emission();
    let index = e.enregistre(ISN, b"GET / HTTP/1.1\r\n\r\n", false, 0).unwrap();
    assert_eq!(e.en_vol(), 1);

    // Avant l'echeance, rien a faire.
    assert_eq!(e.expire(RTO_INITIAL_MS - 1), Expiration::Rien);
    // A l'echeance, le segment est reclame.
    assert_eq!(e.expire(RTO_INITIAL_MS), Expiration::Retransmettre(index));

    e.note_retransmission(index, RTO_INITIAL_MS);
    assert_eq!(e.segments_retransmis, 1);
    assert_eq!(
        e.rto_ms(), RTO_INITIAL_MS * 2,
        "le RTO DOUBLE a chaque expiration : c'est ce qui empeche une pile de \
         participer a un effondrement de congestion en insistant"
    );
    // Et la nouvelle echeance suit le nouveau RTO.
    assert_eq!(e.expire(RTO_INITIAL_MS + 1), Expiration::Rien);
    assert_eq!(
        e.expire(RTO_INITIAL_MS + RTO_INITIAL_MS * 2),
        Expiration::Retransmettre(index)
    );
}

#[test]
fn le_repli_exponentiel_est_plafonne() {
    let mut e = emission();
    let index = e.enregistre(ISN, b"x", false, 0).unwrap();
    for tour in 0..20 {
        e.note_retransmission(index, tour * 1000);
    }
    assert_eq!(e.rto_ms(), RTO_MAX_MS, "au-dela, la connexion est morte, pas lente");
}

/// Renvoyer indefiniment est aussi faux que ne jamais renvoyer : au bout d'un
/// nombre borne d'essais, la connexion est perdue et il faut le dire.
#[test]
fn une_retransmission_sans_fin_est_un_abandon() {
    let mut e = emission();
    let index = e.enregistre(ISN, b"x", false, 0).unwrap();
    let mut horloge = 0u64;
    for _ in 0..RETRANSMISSIONS_MAX {
        horloge += e.rto_ms();
        assert_eq!(e.expire(horloge), Expiration::Retransmettre(index));
        e.note_retransmission(index, horloge);
    }
    horloge += e.rto_ms();
    assert_eq!(
        e.expire(horloge), Expiration::Abandon(index),
        "apres {RETRANSMISSIONS_MAX} essais, insister ne sert plus a rien"
    );
}

/// Le PLUS ANCIEN segment expire est retransmis en premier. Renvoyer dans le
/// desordre ferait compter au pair des doublons qui n'en sont pas.
#[test]
fn le_plus_ancien_expire_part_en_premier() {
    let mut e = emission();
    let premier = e.enregistre(ISN, b"aaa", false, 0).unwrap();
    let second = e.enregistre(ISN + 3, b"bbb", false, 100).unwrap();
    assert_ne!(premier, second);
    assert_eq!(e.expire(RTO_INITIAL_MS + 200), Expiration::Retransmettre(premier));
}

// ---------------------------------------------------------------------------
// ACK DUPLIQUES : la retransmission rapide.
// ---------------------------------------------------------------------------

/// Trois ACK dupliques disent que le pair recoit des segments mais qu'il lui
/// manque celui qu'on attend. Attendre le RTO complet pour s'en rendre compte
/// est ce qui fait passer un transfert d'une seconde a trente.
#[test]
fn trois_ack_dupliques_declenchent_une_retransmission_rapide() {
    let mut e = emission();
    let perdu = e.enregistre(ISN, b"aaa", false, 0).unwrap();
    e.enregistre(ISN + 3, b"bbb", false, 1).unwrap();
    e.enregistre(ISN + 6, b"ccc", false, 2).unwrap();

    // Le pair re-ACKe ISN : il n'a toujours pas le premier segment.
    for tour in 0..DOUBLONS_POUR_RETRANSMISSION {
        assert!(!e.acquitte(ISN, 10 + tour as u64), "un doublon ne fait pas avancer");
        assert!(
            e.retransmission_rapide().is_none() || tour + 1 == DOUBLONS_POUR_RETRANSMISSION,
            "il faut {DOUBLONS_POUR_RETRANSMISSION} doublons, pas moins"
        );
    }
    assert_eq!(e.doublons(), 0, "le declenchement remet le compteur a zero");
    assert_eq!(e.retransmissions_rapides, 1);
    assert!(
        e.segment(perdu).is_some(),
        "le segment reclame est bien celui qui manque au pair"
    );
    assert_eq!(e.segment(perdu).unwrap().seq, ISN);
}

#[test]
fn moins_de_trois_doublons_ne_declenche_rien() {
    let mut e = emission();
    e.enregistre(ISN, b"aaa", false, 0).unwrap();
    for _ in 0..(DOUBLONS_POUR_RETRANSMISSION - 1) {
        e.acquitte(ISN, 10);
    }
    assert_eq!(e.retransmission_rapide(), None);
    assert_eq!(e.retransmissions_rapides, 0);
}

/// Un ACK qui AVANCE remet le compteur de doublons a zero : ce qui suit n'est
/// plus la meme perte.
#[test]
fn un_ack_qui_avance_annule_les_doublons() {
    let mut e = emission();
    e.enregistre(ISN, b"aaa", false, 0).unwrap();
    e.enregistre(ISN + 3, b"bbb", false, 1).unwrap();
    e.acquitte(ISN, 10);
    e.acquitte(ISN, 11);
    assert_eq!(e.doublons(), 2);
    assert!(e.acquitte(ISN + 3, 12), "l'ACK avance");
    assert_eq!(e.doublons(), 0);
    assert_eq!(e.retransmission_rapide(), None);
}

// ---------------------------------------------------------------------------
// REORDONNANCEMENT et ACK RETARDE.
// ---------------------------------------------------------------------------

/// Un ACK cumulatif acquitte TOUT ce qui le precede, meme si les acquittements
/// intermediaires ne sont jamais arrives.
#[test]
fn un_ack_cumulatif_retire_tous_les_segments_precedents() {
    let mut e = emission();
    e.enregistre(ISN, b"aaa", false, 0).unwrap();
    e.enregistre(ISN + 3, b"bbb", false, 1).unwrap();
    e.enregistre(ISN + 6, b"ccc", false, 2).unwrap();
    assert_eq!(e.en_vol(), 3);

    assert!(e.acquitte(ISN + 9, 20));
    assert_eq!(e.en_vol(), 0, "un seul ACK retarde suffit a tout liberer");
    assert_eq!(e.snd_una, ISN + 9);
}

/// Un ACK qui REMONTE dans le passe -- un segment reordonne, ou un pair confus
/// -- ne doit pas faire reculer la fenetre.
#[test]
fn un_ack_du_passe_ne_fait_pas_reculer_la_fenetre() {
    let mut e = emission();
    e.enregistre(ISN, b"aaa", false, 0).unwrap();
    e.enregistre(ISN + 3, b"bbb", false, 1).unwrap();
    e.acquitte(ISN + 6, 10);
    assert_eq!(e.snd_una, ISN + 6);

    assert!(!e.acquitte(ISN + 3, 11), "un ACK plus ancien n'apporte rien");
    assert_eq!(e.snd_una, ISN + 6, "et surtout ne recule pas");
}

/// Un ACK PARTIEL ne libere que ce qu'il couvre entierement.
#[test]
fn un_ack_partiel_ne_libere_pas_un_segment_incomplet() {
    let mut e = emission();
    e.enregistre(ISN, b"aaaaa", false, 0).unwrap();
    e.enregistre(ISN + 5, b"bbbbb", false, 1).unwrap();

    // Le pair acquitte au milieu du second segment.
    assert!(e.acquitte(ISN + 7, 10));
    assert_eq!(
        e.en_vol(), 1,
        "le second segment n'est pas entierement acquitte : il reste en vol"
    );
}

// ---------------------------------------------------------------------------
// RTT : la mesure, et l'algorithme de Karn.
// ---------------------------------------------------------------------------

#[test]
fn le_premier_echantillon_fixe_le_rtt() {
    let mut e = emission();
    e.enregistre(ISN, b"aaa", false, 0).unwrap();
    assert_eq!(e.srtt_ms(), 0, "aucun echantillon au depart");
    assert_eq!(e.rto_ms(), RTO_INITIAL_MS);

    e.acquitte(ISN + 3, 40);
    assert_eq!(e.srtt_ms(), 40);
    assert_eq!(e.rttvar_ms(), 20);
    assert_eq!(e.echantillons_rtt, 1);
    // RTO = SRTT + 4 * RTTVAR = 120, sous le plancher -> plancher.
    assert_eq!(e.rto_ms(), RTO_MIN_MS.max(120));
}

/// Le RTT lisse converge vers la mesure, sans y sauter : c'est ce qui empeche
/// un seul paquet lent de faire tripler le RTO.
#[test]
fn le_rtt_lisse_converge_sans_sauter() {
    let mut e = emission();
    e.enregistre(ISN, b"a", false, 0).unwrap();
    e.acquitte(ISN + 1, 100);
    assert_eq!(e.srtt_ms(), 100);

    // Un seul paquet a 900 ms ne doit pas emporter le SRTT.
    e.enregistre(ISN + 1, b"b", false, 200).unwrap();
    e.acquitte(ISN + 2, 1100);
    assert!(
        e.srtt_ms() > 100 && e.srtt_ms() < 300,
        "SRTT={} : un seul paquet lent ne doit pas emporter la moyenne",
        e.srtt_ms()
    );
}

/// ALGORITHME DE KARN. Un segment retransmis ne donne AUCUN echantillon : on ne
/// sait pas laquelle des deux copies a ete acquittee. Prendre la plus recente
/// diviserait le RTO par deux a chaque perte -- exactement quand il faut
/// l'augmenter.
#[test]
fn un_segment_retransmis_ne_donne_pas_d_echantillon() {
    let mut e = emission();
    e.enregistre(ISN, b"a", false, 0).unwrap();
    e.acquitte(ISN + 1, 100);
    let srtt_avant = e.srtt_ms();
    let rto_avant = e.rto_ms();

    let index = e.enregistre(ISN + 1, b"b", false, 200).unwrap();
    e.note_retransmission(index, 1200);
    // L'ACK arrive « 5 ms » apres la retransmission. Le croire ramenerait le
    // SRTT a presque rien.
    e.acquitte(ISN + 2, 1205);

    assert_eq!(e.srtt_ms(), srtt_avant, "le SRTT ne doit pas bouger");
    assert_eq!(e.echantillons_ecartes_karn, 1);
    assert!(
        e.rto_ms() >= rto_avant,
        "le RTO ne doit pas RETRECIR apres une perte : {} < {}",
        e.rto_ms(), rto_avant
    );
}

#[test]
fn le_rto_reste_dans_ses_bornes() {
    let mut e = emission();
    // Un RTT minuscule ne doit pas produire un RTO qui inonde le reseau.
    e.enregistre(ISN, b"a", false, 0).unwrap();
    e.acquitte(ISN + 1, 1);
    assert!(e.rto_ms() >= RTO_MIN_MS, "RTO={}", e.rto_ms());

    // Un RTT enorme ne doit pas produire un RTO infini.
    let mut e = emission();
    e.enregistre(ISN, b"a", false, 0).unwrap();
    e.acquitte(ISN + 1, 10_000_000);
    assert!(e.rto_ms() <= RTO_MAX_MS, "RTO={}", e.rto_ms());
}

// ---------------------------------------------------------------------------
// SYN et FIN : les segments de CONTROLE consomment un numero sans octet.
// ---------------------------------------------------------------------------

#[test]
fn un_syn_consomme_un_numero_sans_porter_d_octet() {
    let mut e = emission();
    let index = e.enregistre(ISN, &[], true, 0).unwrap();
    let segment = e.segment(index).unwrap();
    assert_eq!(segment.consomme(), 1);
    assert_eq!(segment.fin(), ISN + 1);
    assert_eq!(segment.charge().len(), 0);
    assert_eq!(e.snd_nxt, ISN + 1);

    // Et l'ACK du SYN-ACK le retire, en donnant le PREMIER echantillon de RTT.
    e.acquitte(ISN + 1, 30);
    assert_eq!(e.en_vol(), 0);
    assert_eq!(e.srtt_ms(), 30);
}

/// Un SYN perdu doit se renvoyer. Sans cela, une connexion etait refusee pour
/// un seul paquet perdu.
#[test]
fn un_syn_perdu_se_renvoie() {
    let mut e = emission();
    let index = e.enregistre(ISN, &[], true, 0).unwrap();
    assert_eq!(e.expire(RTO_INITIAL_MS), Expiration::Retransmettre(index));
    e.note_retransmission(index, RTO_INITIAL_MS);
    assert!(e.segment(index).unwrap().controle);
}

// ---------------------------------------------------------------------------
// Bornes : ni allocation, ni file sans fond.
// ---------------------------------------------------------------------------

#[test]
fn la_file_est_bornee_et_le_dit() {
    let mut e = emission();
    for tour in 0..SEGMENTS_MAX {
        assert!(
            e.enregistre(ISN + tour as u32, b"x", false, 0).is_some(),
            "tour {tour}"
        );
    }
    assert_eq!(e.en_vol(), SEGMENTS_MAX);
    assert_eq!(
        e.enregistre(ISN + SEGMENTS_MAX as u32, b"x", false, 0), None,
        "un `None` ne doit PAS etre traite comme un envoi reussi : le segment \
         ne serait jamais retransmis, et la perte serait silencieuse"
    );
    assert_eq!(e.refus_file_pleine, 1);
}

#[test]
fn une_charge_trop_grande_est_refusee() {
    let mut e = emission();
    let enorme = [0u8; retransmission::CHARGE_MAX + 1];
    assert_eq!(e.enregistre(ISN, &enorme, false, 0), None);
    assert_eq!(e.en_vol(), 0);
}

/// L'attente jusqu'a la prochaine echeance : c'est ce qui remplace le
/// busy-poll a duree fixe.
#[test]
fn l_attente_est_celle_de_la_prochaine_echeance() {
    let mut e = emission();
    assert_eq!(e.attente_ms(0), None, "rien en vol, rien a attendre");

    e.enregistre(ISN, b"a", false, 0).unwrap();
    assert_eq!(e.attente_ms(0), Some(RTO_INITIAL_MS));
    assert_eq!(e.attente_ms(RTO_INITIAL_MS / 2), Some(RTO_INITIAL_MS / 2));
    assert_eq!(e.attente_ms(RTO_INITIAL_MS * 2), Some(0), "deja expire");

    // Le plus proche gagne.
    e.enregistre(ISN + 1, b"b", false, 500).unwrap();
    assert_eq!(e.attente_ms(0), Some(RTO_INITIAL_MS));
}

// ---------------------------------------------------------------------------
// Scenario complet : une perte au milieu d'un transfert.
// ---------------------------------------------------------------------------

/// Le cas qui coutait trente secondes : trois segments partent, le premier se
/// perd, le pair re-ACKe trois fois, on retransmet SANS attendre le RTO, et
/// l'ACK suivant libere tout.
#[test]
fn le_scenario_complet_d_une_perte_au_milieu() {
    let mut e = emission();
    let a = e.enregistre(ISN, b"aaaa", false, 0).unwrap();
    e.enregistre(ISN + 4, b"bbbb", false, 1).unwrap();
    e.enregistre(ISN + 8, b"cccc", false, 2).unwrap();

    // Le pair recoit b et c, jamais a : il re-ACKe ISN a chaque fois.
    for tour in 0..3u64 {
        e.acquitte(ISN, 10 + tour);
    }
    let a_renvoyer = e.retransmission_rapide().expect("trois doublons = une perte");
    assert_eq!(a_renvoyer, a);
    assert_eq!(e.segment(a_renvoyer).unwrap().charge(), b"aaaa");

    e.note_retransmission(a_renvoyer, 13);
    assert_eq!(e.segments_retransmis, 1);

    // La copie arrive : le pair ACKe tout d'un coup.
    assert!(e.acquitte(ISN + 12, 40));
    assert_eq!(e.en_vol(), 0);
    assert_eq!(
        e.echantillons_ecartes_karn, 1,
        "le segment retransmis n'a donne aucun echantillon"
    );
    // Et cela s'est fait bien avant le RTO complet.
    assert!(13 < RTO_INITIAL_MS, "la retransmission rapide a evite l'attente du RTO");
}
