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

extern crate alloc;

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

// ===========================================================================
// BOUCHAUD_C13_RTT_DE_POIGNEE_V1 -- la sonde qui ne mesurait rien
// ===========================================================================
//
// `[NET-TCP]` rendait `echantillons_rtt=0 srtt_ms=0 rto_ms=0` sur toutes les
// sessions. `rto_ms=0` trahissait le reste : `Emission::neuf` part de
// `RTO_INITIAL_MS`, jamais de zero -- donc `note_connexion` n'avait jamais ete
// appelee. L'estimateur vivait dans `tcp::fetch`, que le navigateur ring 3
// n'execute pas : ses sockets descendent par `TcpConn::connect`.
//
// Ces tests portent sur la mesure qui couvre CE chemin : le RTT de poignee de
// main, et les SYN perdus.
//
// Ils sont les seuls du fichier a toucher des compteurs GLOBAUX. Ils
// s'enchainent donc dans un unique test, et raisonnent sur des DIFFERENCES
// plutot que sur des valeurs absolues : le harnais de test execute les cas en
// parallele, et un test qui suppose un compteur global a zero est un test qui
// echoue le jour ou on en ajoute un autre a cote.

#[test]
fn les_poignees_de_main_alimentent_la_sonde_reseau() {
    use retransmission::{note_poignee, stats_poignee};

    let (p0, syn0, _, _, _) = stats_poignee();

    // Une poignee propre : un echantillon exploitable.
    note_poignee(Some(40), 0);
    let (p1, syn1, min1, max1, moyen1) = stats_poignee();
    assert_eq!(p1, p0 + 1, "la poignee est comptee");
    assert_eq!(syn1, syn0, "aucun SYN perdu");
    assert!(min1 > 0 && max1 > 0 && moyen1 > 0, "un echantillon existe desormais");
    assert!(min1 <= 40 && max1 >= 40, "40 ms est dans l'intervalle observe");

    // Une poignee dont le SYN a ete retransmis : Karn ECARTE l'echantillon,
    // mais la perte est comptee. C'est la regle qui compte le plus ici : un
    // RTT mesure apres retransmission ne dit pas de quel SYN il est la
    // reponse, et le retenir gonflerait la latence apparente d'un RTO entier.
    note_poignee(None, 2);
    let (p2, syn2, min2, max2, moyen2) = stats_poignee();
    assert_eq!(p2, p1 + 1, "elle est comptee comme poignee");
    assert_eq!(syn2, syn1 + 2, "et ses deux SYN perdus sont comptes");
    assert_eq!(
        (min2, max2, moyen2), (min1, max1, moyen1),
        "l'echantillon ambigu ne doit RIEN changer aux RTT"
    );

    // Le maximum suit une poignee lente, le minimum ne bouge pas.
    note_poignee(Some(900), 0);
    let (_, _, min3, max3, _) = stats_poignee();
    assert_eq!(min3, min2, "un RTT plus grand ne change pas le minimum");
    assert!(max3 >= 900, "le maximum retient la poignee lente");

    // Minimum et maximum ensemble disent la DISPERSION -- ce qui distingue un
    // lien lent d'un lien irregulier, et qu'une moyenne seule effacerait.
    assert!(max3 > min3, "l'ecart entre 40 ms et 900 ms doit rester visible");

    // Cinq SYN partis, aucune reponse : la connexion a echoue. Elle ne doit
    // produire AUCUN RTT -- surtout pas un zero, qui se lirait comme un reseau
    // instantane et ferait mentir le minimum.
    let (_, _, min4, max4, moyen4) = stats_poignee();
    note_poignee(None, 4);
    let (_, _, min5, max5, moyen5) = stats_poignee();
    assert_eq!((min4, max4, moyen4), (min5, max5, moyen5));
}

// ===========================================================================
// BOUCHAUD_TCP_SOCKETS_FIABLES_V1 -- le consentement YouTube de quatre minutes
// ===========================================================================
//
// Journal WHPX du 18/11 : trois POST partis sur une connexion HTTP/2 reutilisee
// vers www.youtube.com ne sont jamais arrives. Google a ferme la connexion a
// l'echeance d'inactivite (240 s depuis la derniere activite qu'IL avait vue),
// et la sauvegarde du consentement, qui attendait le premier POST, est partie
// seulement apres.
//
// Cause : `TcpConn::send`, le TCP des sockets d'anneau 3, n'avait AUCUNE file
// d'emission. Quand l'entree ARP de la passerelle avait expire, la trame ne
// partait pas -- `send_ip_immediat` rendait `false`, ignore -- et le numero de
// sequence avancait quand meme. Le pair attendait l'octet manquant pour
// toujours.
//
// Ces cas rejouent l'incident sur un modele de lien et de recepteur dont le
// temps est un parametre. Le recepteur est celui d'un TCP ordinaire (SLIRP) :
// il range dans l'ordre, met de cote ce qui arrive en avance, et acquitte
// cumulativement par des ACK PURS.

use retransmission::Renvoi;

/// Le recepteur d'en face : reassemble, et rend l'ACK cumulatif qu'il enverrait.
struct Recepteur {
    rcv_nxt: u32,
    recu: Vec<u8>,
    en_avance: Vec<(u32, Vec<u8>)>,
}

impl Recepteur {
    fn neuf(isn: u32) -> Self {
        Self { rcv_nxt: isn, recu: Vec::new(), en_avance: Vec::new() }
    }

    fn recoit(&mut self, seq: u32, charge: &[u8]) -> u32 {
        if seq_avant(seq, self.rcv_nxt) {
            // Doublon (copie retransmise d'un segment deja recu) : la partie
            // deja livree est ignoree, jamais livree deux fois.
            let deja = self.rcv_nxt.wrapping_sub(seq) as usize;
            if deja >= charge.len() {
                return self.rcv_nxt;
            }
            return self.recoit(self.rcv_nxt, &charge[deja..]);
        }
        if seq != self.rcv_nxt {
            if !self.en_avance.iter().any(|(s, _)| *s == seq) {
                self.en_avance.push((seq, charge.to_vec()));
            }
            return self.rcv_nxt;
        }
        self.recu.extend_from_slice(charge);
        self.rcv_nxt = self.rcv_nxt.wrapping_add(charge.len() as u32);
        while let Some(i) = self.en_avance.iter().position(|(s, _)| *s == self.rcv_nxt) {
            let (_, suite) = self.en_avance.remove(i);
            self.recu.extend_from_slice(&suite);
            self.rcv_nxt = self.rcv_nxt.wrapping_add(suite.len() as u32);
        }
        self.rcv_nxt
    }
}

/// Le corps d'une requete de `n` octets, reconnaissable octet par octet.
fn corps(n: usize, graine: u8) -> Vec<u8> {
    (0..n).map(|i| b'a' + ((i * 7 + graine as usize * 13) % 26) as u8).collect()
}

/// Emet `donnees` en segments de 1400 octets, comme `TcpConn::envoie`. Le
/// premier segment de la rafale ne part PAS si `arp_expire` : c'est exactement
/// ce que `send_ip_immediat` fait sur un cache ARP vide.
fn emet_requete(
    e: &mut Emission,
    pair: &mut Recepteur,
    seq: &mut u32,
    donnees: &[u8],
    maintenant: u64,
    arp_expire: bool,
) -> Vec<u32> {
    let mut acks = Vec::new();
    for (rang, morceau) in donnees.chunks(1400).enumerate() {
        e.enregistre(*seq, morceau, false, maintenant).expect("la file a de la place");
        let parti = !(arp_expire && rang == 0);
        if parti {
            acks.push(pair.recoit(*seq, morceau));
        }
        *seq = seq.wrapping_add(morceau.len() as u32);
    }
    acks
}

/// LE CAS DE L'INCIDENT. Une requete passe, 65 s de silence, puis une requete
/// de trois segments dont le premier ne part pas.
#[test]
fn incident_youtube_un_segment_non_parti_est_renvoye() {
    let mut e = emission();
    let mut pair = Recepteur::neuf(ISN);
    let mut seq = ISN;

    // E1 : passe, et donne un echantillon de RTT de 12 ms.
    let e1 = corps(200, 1);
    for ack in emet_requete(&mut e, &mut pair, &mut seq, &e1, 0, false) {
        e.acquitte_segment(ack, false, 12);
    }
    assert_eq!(e.en_vol(), 0, "E1 est acquittee");

    // E2 apres 65 s : l'ARP a expire, le premier segment reste a quai.
    let t0 = 65_000u64;
    let e2 = corps(4000, 2);
    let acks = emet_requete(&mut e, &mut pair, &mut seq, &e2, t0, true);
    assert_eq!(pair.recu.len(), 200, "le pair n'a RIEN livre de E2 : il lui manque le debut");
    // Le pair re-ACKe le trou par des ACK purs : deux doublons, pas trois.
    for ack in acks {
        e.acquitte_segment(ack, false, t0 + 12);
    }
    assert!(e.retransmission_rapide().is_none(), "deux doublons ne suffisent pas");

    // La pompe tourne toutes les deux millisecondes (`poll`). Au plus tard a
    // l'echeance du RTO, le segment manquant repart.
    let mut t = t0;
    let mut livre_a = None;
    while t < t0 + 240_000 {
        t += 2;
        let mut envois = Vec::new();
        let bilan = e.renvoie(t, |s, c| { envois.push((s, c.to_vec())); true });
        assert!(!bilan.abandon);
        for (s, c) in envois {
            let ack = pair.recoit(s, &c);
            e.acquitte_segment(ack, false, t + 12);
        }
        if pair.recu.len() == 200 + 4000 {
            livre_a = Some(t);
            break;
        }
    }
    let livre_a = livre_a.expect("E2 doit etre livree bien avant l'echeance de 240 s du serveur");
    assert!(
        livre_a - t0 <= 1_000,
        "E2 livree {} ms apres l'envoi : un RTO, pas l'inactivite du serveur",
        livre_a - t0
    );
    assert_eq!(&pair.recu[..200], &e1[..]);
    assert_eq!(&pair.recu[200..], &e2[..], "flux intact : ni trou, ni doublon, ni decalage");
    assert_eq!(e.en_vol(), 0, "tout est acquitte");
}

/// Le temoin de l'ancien chemin : sans file d'emission, le meme scenario ne se
/// debloque JAMAIS -- c'est le serveur qui finit par fermer.
#[test]
fn incident_youtube_sans_renvoi_le_pair_attend_pour_toujours() {
    let mut e = emission();
    let mut pair = Recepteur::neuf(ISN);
    let mut seq = ISN;
    emet_requete(&mut e, &mut pair, &mut seq, &corps(4000, 2), 65_000, true);
    // Aucun appel a `renvoie` : c'est ce que faisait `TcpConn::send`.
    assert_eq!(pair.recu.len(), 0, "rien n'est livre, et rien ne le sera");
    assert_eq!(e.en_vol(), 3, "trois segments en vol que personne ne renverra");
}

/// Un segment qui PORTE DES DONNEES et repete l'ACK n'est pas un doublon
/// (RFC 5681, section 2). Le compter ferait retransmettre a tort, a chaque
/// reponse de plus de trois segments -- c'est-a-dire a chaque page.
#[test]
fn un_segment_de_donnees_qui_repete_l_ack_n_est_pas_un_doublon() {
    let mut e = emission();
    e.enregistre(ISN, b"GET / HTTP/1.1\r\n\r\n", false, 0).unwrap();
    for t in 0..5u64 {
        e.acquitte_segment(ISN, true, 10 + t);
    }
    assert!(e.retransmission_rapide().is_none(), "une reponse en cinq segments n'est pas une perte");

    for t in 0..3u64 {
        e.acquitte_segment(ISN, false, 20 + t);
    }
    assert!(e.retransmission_rapide().is_some(), "trois ACK PURS dupliques, si");
}

/// Un ACK qui porte des donnees ET avance la fenetre acquitte bel et bien.
#[test]
fn un_segment_de_donnees_qui_avance_l_ack_acquitte() {
    let mut e = emission();
    e.enregistre(ISN, b"abcd", false, 0).unwrap();
    assert!(e.acquitte_segment(ISN + 4, true, 9));
    assert_eq!(e.en_vol(), 0);
}

/// Plusieurs segments expires ensemble : tous repartent, DANS L'ORDRE, et le
/// RTO ne double qu'une fois. Le doubler par segment ferait passer trois pertes
/// d'une seconde a huit.
#[test]
fn une_expiration_groupee_double_le_rto_une_seule_fois() {
    let mut e = emission();
    // Enregistres dans le desordre de la file : l'ordre d'emission doit
    // suivre les numeros de sequence, pas les indices.
    e.enregistre(ISN + 6, b"ccc", false, 0).unwrap();
    e.enregistre(ISN, b"aaa", false, 0).unwrap();
    e.enregistre(ISN + 3, b"bbb", false, 0).unwrap();

    let mut ordre = Vec::new();
    let bilan: Renvoi = e.renvoie(RTO_INITIAL_MS, |s, _| { ordre.push(s); true });
    assert_eq!(ordre, vec![ISN, ISN + 3, ISN + 6]);
    assert_eq!(bilan.renvoyes, 3);
    assert_eq!(e.rto_ms(), RTO_INITIAL_MS * 2, "un seul doublement pour une seule expiration");
    assert_eq!(e.segments_retransmis, 3);

    // Rien avant la nouvelle echeance.
    let mut rien = 0;
    e.renvoie(RTO_INITIAL_MS + 1, |_, _| { rien += 1; true });
    assert_eq!(rien, 0);
}

/// La retransmission rapide passe par `renvoie`, sans doubler le RTO : le pair
/// recoit, il ne s'agit pas d'une congestion.
#[test]
fn renvoie_sert_la_retransmission_rapide_sans_repli() {
    let mut e = emission();
    e.enregistre(ISN, b"aaaa", false, 0).unwrap();
    e.enregistre(ISN + 4, b"bbbb", false, 0).unwrap();
    for t in 0..3u64 {
        e.acquitte_segment(ISN, false, 5 + t);
    }
    let mut renvoyes = Vec::new();
    let bilan = e.renvoie(10, |s, _| { renvoyes.push(s); true });
    assert!(bilan.rapide);
    assert_eq!(renvoyes, vec![ISN], "seul le trou repart");
    assert_eq!(e.rto_ms(), RTO_INITIAL_MS);
}

/// Un pair disparu : `renvoie` finit par le dire, au lieu d'insister sans fin.
#[test]
fn renvoie_signale_l_abandon() {
    let mut e = emission();
    e.enregistre(ISN, b"x", false, 0).unwrap();
    let mut t = 0u64;
    let mut abandon = false;
    for _ in 0..(RETRANSMISSIONS_MAX + 2) {
        t += RTO_MAX_MS;
        if e.renvoie(t, |_, _| true).abandon {
            abandon = true;
            break;
        }
    }
    assert!(abandon, "apres {RETRANSMISSIONS_MAX} essais la connexion est perdue");
}

// ---------------------------------------------------------------------------
// QUALIFICATION : pertes dans les deux sens, non-duplication, temporisation,
// fin de connexion. Toujours sur le modele : temps et pertes sont des
// parametres, donc chaque echec se rejoue a l'identique.
// ---------------------------------------------------------------------------

/// Generateur pseudo-aleatoire a graine fixe (xorshift) : des pertes
/// « aleatoires » qui se rejouent a l'identique.
struct Alea(u64);
impl Alea {
    fn tire(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn perd(&mut self, pour_mille: u64) -> bool {
        self.tire() % 1000 < pour_mille
    }
}

/// Le pair a RECU, mais son ACK s'est perdu : on renvoie une copie. Il doit
/// l'ignorer -- le flux livre ne contient l'octet qu'une fois.
#[test]
fn un_ack_perdu_fait_renvoyer_une_copie_que_le_pair_ignore() {
    let mut e = emission();
    let mut pair = Recepteur::neuf(ISN);
    let donnees = corps(1000, 4);
    e.enregistre(ISN, &donnees, false, 0).unwrap();
    let _ack_perdu = pair.recoit(ISN, &donnees);
    assert_eq!(pair.recu.len(), 1000);

    let mut copies = 0;
    let mut t = 0u64;
    while !e.vide() && t < 10_000 {
        t += 2;
        let mut envois = Vec::new();
        e.renvoie(t, |s, c| { envois.push((s, c.to_vec())); true });
        for (s, c) in envois {
            copies += 1;
            let ack = pair.recoit(s, &c);
            e.acquitte_segment(ack, false, t + 5);
        }
    }
    assert_eq!(copies, 1, "une seule copie, a l'echeance du RTO");
    assert!(e.vide(), "l'ACK de la copie libere la file");
    assert_eq!(pair.recu, donnees, "le pair n'a rien livre deux fois");
}

/// Gros transfert sur un lien qui perd dans LES DEUX SENS (donnees ET ACK),
/// avec une file bornee a SEGMENTS_MAX comme dans `TcpConn::ecrit`. Plusieurs
/// graines : le flux livre doit etre exactement le flux emis, et tout doit
/// finir acquitte, sans abandon.
#[test]
fn transfert_avec_pertes_dans_les_deux_sens_livre_le_flux_exact() {
    let mut pertes_totales = 0u64;
    for graine in [0x9e3779b97f4a7c15u64, 7, 1234567, 0xdeadbeef, 42] {
        for pour_mille in [20u64, 100] {
            let mut alea = Alea(graine);
            let mut e = emission();
            let mut pair = Recepteur::neuf(ISN);
            let donnees = corps(200_000, (graine % 26) as u8);
            let mut seq = ISN;
            let mut off = 0usize;
            let mut t = 0u64;
            let mut acks_en_vol: Vec<(u64, u32)> = Vec::new(); // (arrivee, ack)
            let rtt = 10u64;
            let mut pertes_donnees = 0u64;
            while (off < donnees.len() || !e.vide()) && t < 3_600_000 {
                t += 2;
                // Les ACK arrives.
                acks_en_vol.retain(|&(arrivee, ack)| {
                    if arrivee <= t { e.acquitte_segment(ack, false, t); false } else { true }
                });
                // Emettre ce que la file accepte.
                while off < donnees.len() && e.a_de_la_place() {
                    let fin = (off + 1400).min(donnees.len());
                    let morceau = &donnees[off..fin];
                    e.enregistre(seq, morceau, false, t).unwrap();
                    if !alea.perd(pour_mille) {
                        let ack = pair.recoit(seq, morceau);
                        if !alea.perd(pour_mille) { acks_en_vol.push((t + rtt, ack)); }
                    } else {
                        pertes_donnees += 1;
                    }
                    seq = seq.wrapping_add(morceau.len() as u32);
                    off = fin;
                }
                // Renvois.
                let mut envois = Vec::new();
                let bilan = e.renvoie(t, |s, c| { envois.push((s, c.to_vec())); true });
                assert!(!bilan.abandon, "graine {graine:#x} perte {pour_mille}/1000 : abandon a tort");
                for (s, c) in envois {
                    if !alea.perd(pour_mille) {
                        let ack = pair.recoit(s, &c);
                        if !alea.perd(pour_mille) { acks_en_vol.push((t + rtt, ack)); }
                    }
                }
            }
            assert_eq!(pair.recu.len(), donnees.len(), "graine {graine:#x} perte {pour_mille}/1000 : flux incomplet");
            assert!(pair.recu == donnees, "graine {graine:#x} perte {pour_mille}/1000 : flux corrompu ou duplique");
            assert!(e.vide(), "graine {graine:#x} : tout est acquitte");
            eprintln!("graine={graine:#x} perte={pour_mille}/1000 segments_perdus={pertes_donnees} renvois={} fin_ms={t}", e.segments_retransmis);
            assert!(e.segments_retransmis >= pertes_donnees, "chaque segment perdu a ete renvoye au moins une fois");
            assert!(t < 60_000, "graine {graine:#x} perte {pour_mille}/1000 : 200 Ko en {t} ms");
            pertes_totales += pertes_donnees;
        }
    }
    assert!(pertes_totales >= 20, "les graines exercent bien des pertes ({pertes_totales})");
}

/// Le RTO double sur une perte, puis REVIENT a la mesure des qu'un segment
/// non retransmis est acquitte : une perte isolee ne laisse pas la connexion
/// lente pour toujours.
#[test]
fn le_rto_revient_a_la_mesure_apres_une_perte() {
    let mut e = emission();
    e.enregistre(ISN, b"aaaa", false, 0).unwrap();
    e.acquitte_segment(ISN + 4, false, 12);
    let rto_mesure = e.rto_ms();
    assert_eq!(rto_mesure, RTO_MIN_MS, "RTT de 12 ms : RTO au plancher");

    e.enregistre(ISN + 4, b"bbbb", false, 100).unwrap();
    e.renvoie(100 + rto_mesure, |_, _| true);
    assert_eq!(e.rto_ms(), rto_mesure * 2, "la perte double le RTO");
    e.acquitte_segment(ISN + 8, false, 100 + rto_mesure + 12);

    // Le segment suivant, lui, n'est pas retransmis : son echantillon compte.
    e.enregistre(ISN + 8, b"cccc", false, 1_000).unwrap();
    e.acquitte_segment(ISN + 12, false, 1_012);
    assert_eq!(e.rto_ms(), RTO_MIN_MS, "le RTO est revenu a la mesure");
}

/// Pair disparu : l'abandon arrive en temps BORNE. Une application qui
/// attend sur `recv` doit apprendre `ETIMEDOUT`, pas rester figee.
#[test]
fn un_pair_disparu_est_abandonne_en_temps_borne() {
    let mut e = emission();
    e.enregistre(ISN, b"POST", false, 0).unwrap();
    let mut t = 0u64;
    let mut abandon_a = None;
    while t < 3_600_000 {
        t += 2;
        if e.renvoie(t, |_, _| true).abandon {
            abandon_a = Some(t);
            break;
        }
    }
    let abandon_a = abandon_a.expect("le pair muet finit par etre abandonne");
    // RTO initial 1 s, double a chaque essai, plafonne a RTO_MAX_MS :
    // 1+2+4+8+16+32+60+60+60 s = 243 s au plus.
    assert!(abandon_a <= 250_000, "abandon apres {} ms", abandon_a);
    assert!(abandon_a >= 60_000, "pas d'abandon premature : {} ms", abandon_a);
}

/// Un FIN du pair n'acquitte pas des donnees qu'il n'a pas recues : ce qui
/// est en vol reste a renvoyer jusqu'a l'ACK.
#[test]
fn un_ack_qui_n_avance_pas_ne_libere_rien() {
    let mut e = emission();
    e.enregistre(ISN, b"aaaa", false, 0).unwrap();
    e.enregistre(ISN + 4, b"bbbb", false, 0).unwrap();
    // Le pair acquitte le premier seulement.
    e.acquitte_segment(ISN + 4, true, 10);
    assert_eq!(e.en_vol(), 1, "le second reste en vol");
    let mut renvoye = Vec::new();
    e.renvoie(RTO_INITIAL_MS + 100, |s, _| { renvoye.push(s); true });
    assert_eq!(renvoye, vec![ISN + 4]);
}
