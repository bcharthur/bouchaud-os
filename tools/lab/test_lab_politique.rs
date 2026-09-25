//! La politique de transport borne-t-elle VRAIMENT son travail, et le curseur
//! avance-t-il toujours ?
//!
//! # Ce que ces epreuves defendent
//!
//! Le canal de telemetrie est le dernier lien vivant quand la reception est
//! morte. Il a deux facons de se saborder, et toutes deux sont silencieuses :
//!
//! 1. **s'auto-alimenter.** S'il emet un evenement dans l'anneau qu'il lit, un
//!    seul evenement suffit a lancer un trafic perpetuel : le canal se nourrit
//!    de sa propre trace, et il le fait d'autant plus que la machine est calme
//!    par ailleurs. `telemetrable()` refuse les evenements du transport, et
//!    c'est une REGLE, contredite ici ;
//!
//! 2. **ne pas avancer.** Un curseur qui reste sur un evenement qu'on ne peut
//!    pas transporter bloque tout ce qui le suit, pour toujours. Trois chemins
//!    y menaient : l'evenement trop grand, l'evenement refuse, et -- le plus
//!    retors -- le recalage dont le resultat etait calcule puis JETE.
//!
//! S'y ajoute le cout : borner sur ce qui est AJOUTE ne borne rien, puisqu'un
//! evenement refuse avance le curseur sans rien ajouter. Mille evenements
//! refuses tenaient donc dans un seul tour, pris a l'ordonnanceur que ce
//! canal est cense observer sans le perturber.
//!
//! Lance par `tools/ci/run_host_tests.sh`.

#[path = "../../src/kernel/debug/lab/anneau.rs"]
pub mod anneau;

#[path = "../../src/kernel/debug/lab/catalogue.rs"]
pub mod catalogue;

// `politique.rs` ecrit `crate::kernel::lab::{anneau, catalogue}` : on recree le
// chemin ici plutot que de modifier la source pour lui plaire. Un test qui
// oblige a tordre le code qu'il teste ne teste plus le meme code.
pub mod kernel {
    pub mod lab {
        pub use crate::anneau;
        pub use crate::catalogue;
    }
}

#[path = "../../src/net/diag_distant/tampon.rs"]
pub mod tampon;

#[path = "../../src/net/diag_distant/politique.rs"]
pub mod politique;

use anneau::Evenement;
use catalogue::{id, Categorie};
use politique::{
    ajoute, remplis, telemetrable, Ajout, Bornes, LectureTransport, MAX_AJOUTES,
    MAX_EXAMINES,
};
use tampon::Tampon;

/// Assez grand pour une poignee d'evenements, assez petit pour que la borne
/// d'octets se manifeste dans une epreuve.
const CHARGE: usize = 1024;
type Charge = Tampon<CHARGE>;

/// Un evenement court : categorie connue, arguments petits.
fn court(seq: u64) -> Evenement {
    Evenement {
        seq,
        t_ns: 1_000,
        cpu: 0,
        categorie: Categorie::Lab as u16,
        event_id: id::LAB_DEMARRE,
        args: [2048, 0, 0, 0],
    }
}

/// Un evenement que le catalogue ne connait pas, aux arguments maximaux.
///
/// Le rendu d'un identifiant inconnu sort les quatre arguments BRUTS, et
/// `u64::MAX` fait vingt chiffres chacun : c'est la ligne la plus longue que
/// le catalogue puisse produire.
fn enorme(seq: u64) -> Evenement {
    Evenement {
        seq,
        t_ns: u64::MAX,
        cpu: 7,
        categorie: Categorie::Lab as u16,
        event_id: 0xFFFF,
        args: [u64::MAX; 4],
    }
}

/// Un evenement du transport lui-meme.
fn du_transport(seq: u64, event_id: u32) -> Evenement {
    Evenement {
        seq,
        t_ns: 2_000,
        cpu: 0,
        categorie: Categorie::Remote as u16,
        event_id,
        args: [1, 2, 3, 0],
    }
}

/// Combien d'octets cet evenement occupe, terminateur compris.
fn taille_rendue(ev: &Evenement) -> usize {
    let mut t = Charge::neuf();
    assert_eq!(ajoute(&mut t, ev), Ajout::Ajoute);
    t.len()
}

/// Le nombre de lignes d'un datagramme, quelle que soit sa taille.
fn lignes<const N: usize>(charge: &Tampon<N>) -> usize {
    charge.octets().iter().filter(|&&o| o == b'\n').count()
}

// ===========================================================================
// LA REGLE : CE QUI NE SE TRANSPORTE PAS
// ===========================================================================

#[test]
fn un_evenement_ordinaire_est_telemetrable() {
    assert!(telemetrable(Categorie::Lab as u16, id::LAB_DEMARRE));
    assert!(telemetrable(Categorie::Rtl8168 as u16, 0x100));
    assert!(telemetrable(Categorie::Blackbox as u16, 0x200));
}

#[test]
fn telemetrie_envoi_est_refuse() {
    // LE GARDE-FOU CONTRE L'AUTO-ALIMENTATION. Sans lui, chaque envoi reussi
    // fabrique l'evenement suivant a envoyer.
    assert!(!telemetrable(Categorie::Remote as u16, id::TELEMETRIE_ENVOI));
}

#[test]
fn telemetrie_abandon_est_refuse() {
    // ET L'ECHEC AUSSI. Un canal qui n'arrive pas a emettre et qui le note
    // dans l'anneau qu'il lit s'emballe d'autant plus qu'il va mal.
    assert!(!telemetrable(Categorie::Remote as u16, id::TELEMETRIE_ABANDON));
}

#[test]
fn les_autres_evenements_remote_passent() {
    // LE SERVEUR BRDP N'EST PAS LE TRANSPORT. Ses evenements sont produits par
    // les commandes d'un operateur, pas par le canal qui les transporte : ils
    // ne peuvent donc pas s'auto-entretenir, et les refuser ferait perdre
    // l'essentiel de ce qu'on veut voir.
    for e in [
        id::BRDP_ECOUTE,
        id::BRDP_CONNEXION,
        id::BRDP_AUTH,
        id::BRDP_COMMANDE,
        id::BRDP_DECONNEXION,
    ] {
        assert!(telemetrable(Categorie::Remote as u16, e), "event {e:#x} refuse a tort");
    }
}

// ===========================================================================
// `ajoute` : LES QUATRE VERDICTS
// ===========================================================================

#[test]
fn un_evenement_normal_est_ajoute() {
    let mut c = Charge::neuf();
    assert_eq!(ajoute(&mut c, &court(1)), Ajout::Ajoute);
    assert!(!c.tronque());
    assert_eq!(lignes(&c), 1);
    assert!(c.octets().ends_with(b"\n"));
}

#[test]
fn un_evenement_du_transport_est_refuse_sans_rien_ecrire() {
    let mut c = Charge::neuf();
    assert_eq!(ajoute(&mut c, &du_transport(1, id::TELEMETRIE_ENVOI)), Ajout::Refuse);
    assert!(c.is_empty(), "un refus n'ecrit pas un octet");
    assert!(!c.tronque());
}

#[test]
fn un_evenement_qui_ne_tient_dans_aucun_datagramme_est_trop_grand() {
    // La borne est choisie pour que l'enorme ne passe pas SEUL : c'est la
    // definition de `TropGrand`, et l'epreuve la verifie au lieu de la
    // supposer.
    assert!(
        taille_rendue(&enorme(1)) > 128,
        "l'epreuve suppose que l'enorme deborde 128 octets ; il n'en fait que {}",
        taille_rendue(&enorme(1)),
    );
    let mut c = Tampon::<128>::neuf();
    assert_eq!(ajoute(&mut c, &enorme(1)), Ajout::TropGrand);
    assert!(c.is_empty(), "le rollback n'a rien laisse");
    assert!(!c.tronque(), "et le drapeau est rendu intact");
}

#[test]
fn un_evenement_qui_tiendrait_seul_rend_plein_et_ne_perd_rien() {
    let taille = taille_rendue(&court(1));
    assert!(taille < CHARGE);

    let mut c = Charge::neuf();
    let mut mis: u64 = 0;
    // On remplit jusqu'a ce que le suivant ne tienne plus.
    loop {
        let avant = c.len();
        match ajoute(&mut c, &court(mis + 1)) {
            Ajout::Ajoute => mis += 1,
            Ajout::Plein => {
                // LE ROLLBACK EST EXACT : le tampon est rendu dans l'etat
                // confie, drapeau compris, donc ce qui suit peut y ecrire.
                assert_eq!(c.len(), avant);
                assert!(!c.tronque());
                break;
            }
            autre => panic!("verdict inattendu : {autre:?}"),
        }
        assert!(mis < 1_000, "le tampon aurait du se remplir");
    }
    assert!(mis > 0);
    assert_eq!(lignes(&c) as u64, mis);
}

// ===========================================================================
// LE CURSEUR AVANCE -- LES TROIS CHEMINS
// ===========================================================================

#[test]
fn un_tour_ordinaire_ajoute_et_avance() {
    let mut c = Charge::neuf();
    let (suivant, bilan) = remplis(&mut c, 1, Bornes::defaut(), |seq| {
        if seq <= 3 {
            LectureTransport::Evenement(court(seq))
        } else {
            LectureTransport::PasEncore
        }
    });
    assert_eq!(bilan.ajoutes, 3);
    assert_eq!(suivant, 4, "le curseur avance d'un par evenement lu");
    assert_eq!(lignes(&c), 3);
    assert_eq!(bilan.perdues, 0);
    assert!(!bilan.plein);
    assert!(!bilan.borne_examens);
}

#[test]
fn le_tour_suivant_ne_rejoue_rien_donc_n_ajoute_rien() {
    // AUCUNE AUTO-ALIMENTATION. La source est la seule entree ; une fois
    // epuisee, le canal se tait au lieu de se relire indefiniment.
    let mut c = Charge::neuf();
    let (suivant, b1) = remplis(&mut c, 1, Bornes::defaut(), |seq| {
        if seq == 1 { LectureTransport::Evenement(court(1)) } else { LectureTransport::PasEncore }
    });
    assert_eq!(b1.ajoutes, 1);
    assert_eq!(suivant, 2);

    let mut c2 = Charge::neuf();
    let (apres, b2) = remplis(&mut c2, suivant, Bornes::defaut(), |seq| {
        if seq == 1 { LectureTransport::Evenement(court(1)) } else { LectureTransport::PasEncore }
    });
    assert_eq!(b2.ajoutes, 0, "plus rien a dire");
    assert_eq!(apres, suivant, "et le curseur ne bouge pas pour rien");
    assert!(c2.is_empty());
}

#[test]
fn les_evenements_refuses_font_avancer_le_curseur() {
    let mut c = Charge::neuf();
    let (suivant, bilan) = remplis(&mut c, 1, Bornes::defaut(), |seq| match seq {
        1..=3 => LectureTransport::Evenement(du_transport(seq, id::TELEMETRIE_ENVOI)),
        4 => LectureTransport::Evenement(court(4)),
        _ => LectureTransport::PasEncore,
    });
    assert_eq!(bilan.refuses, 3);
    assert_eq!(bilan.ajoutes, 1);
    assert_eq!(suivant, 5, "refuser n'est pas rester sur place");
}

#[test]
fn un_evenement_trop_grand_fait_avancer_le_curseur() {
    // LE BLOCAGE DEFINITIF QU'ON INTERDIT. Si le curseur restait, le tour
    // suivant retomberait dessus, echouerait pareil, et tout ce qui le suit
    // serait bloque pour toujours -- dans l'outil meme qui sert a
    // diagnostiquer les pannes.
    let mut c = Tampon::<128>::neuf();
    let (suivant, bilan) = remplis(&mut c, 7, Bornes::defaut(), |seq| {
        if seq == 7 { LectureTransport::Evenement(enorme(7)) } else { LectureTransport::PasEncore }
    });
    assert_eq!(bilan.trop_grands, 1);
    assert_eq!(bilan.ajoutes, 0);
    assert_eq!(suivant, 8, "le curseur a enjambe l'evenement impossible");
}

#[test]
fn un_evenement_trop_grand_puis_un_normal_laisse_partir_le_normal() {
    let mut c = Tampon::<128>::neuf();
    let (suivant, bilan) = remplis(&mut c, 1, Bornes::defaut(), |seq| match seq {
        1 => LectureTransport::Evenement(enorme(1)),
        2 => LectureTransport::Evenement(court(2)),
        _ => LectureTransport::PasEncore,
    });
    assert_eq!(bilan.trop_grands, 1);
    assert_eq!(bilan.ajoutes, 1, "le normal est parti DANS LE MEME TOUR");
    assert_eq!(suivant, 3);
    assert_eq!(lignes(&c), 1);
    assert!(!c.tronque(), "le rollback de l'enorme n'a pas laisse de trace");
}

// ===========================================================================
// LE RECALAGE : L'EPREUVE QUE LA PREMIERE REDACTION NE PASSAIT PAS
// ===========================================================================

#[test]
fn un_recalage_deplace_reellement_le_curseur() {
    // L'EPREUVE OBLIGATOIRE. Curseur a 10, anneau qui ne garde plus rien avant
    // 100, evenement valide a 100.
    //
    // La premiere redaction se recalait DANS la source et rendait `None` : le
    // nouveau curseur etait calcule puis jete, et `remplis` laissait le sien a
    // 10. Tour suivant : meme lecture, meme recalage, meme oubli. Le canal
    // brulait un tour de boucle pour ne rien faire, indefiniment, et recomptait
    // la meme perte a chaque fois.
    let mut lectures = 0u32;
    let mut c = Charge::neuf();
    let (suivant, bilan) = remplis(&mut c, 10, Bornes::defaut(), |seq| {
        lectures += 1;
        if seq < 100 {
            LectureTransport::Recale { nouveau: 100, perdues: 100 - seq }
        } else if seq == 100 {
            LectureTransport::Evenement(court(100))
        } else {
            LectureTransport::PasEncore
        }
    });

    assert_eq!(bilan.recalages, 1, "un seul recalage");
    assert_eq!(bilan.perdues, 90, "la perte est comptee UNE fois");
    assert_eq!(bilan.ajoutes, 1, "l'evenement 100 a pu etre transmis");
    assert!(suivant >= 100, "le curseur a bien franchi le trou");
    assert_eq!(suivant, 101);
    assert_eq!(lignes(&c), 1);

    // ET LE TOUR SUIVANT NE RETOMBE PAS SUR 10.
    let mut c2 = Charge::neuf();
    let (apres, b2) = remplis(&mut c2, suivant, Bornes::defaut(), |seq| {
        if seq < 100 {
            panic!("le tour suivant est reparti en arriere, a {seq}");
        }
        LectureTransport::PasEncore
    });
    assert_eq!(apres, suivant);
    assert_eq!(b2.recalages, 0);
    assert_eq!(b2.perdues, 0, "la perte n'est pas recomptee");
}

#[test]
fn une_serie_de_recalages_ne_boucle_pas() {
    let mut c = Charge::neuf();
    let (suivant, bilan) = remplis(&mut c, 1, Bornes::defaut(), |seq| match seq {
        1 => LectureTransport::Recale { nouveau: 50, perdues: 49 },
        50 => LectureTransport::Recale { nouveau: 80, perdues: 30 },
        80 => LectureTransport::Evenement(court(80)),
        _ => LectureTransport::PasEncore,
    });
    assert_eq!(bilan.recalages, 2);
    assert_eq!(bilan.perdues, 79);
    assert_eq!(bilan.ajoutes, 1);
    assert_eq!(suivant, 81);
}

#[test]
fn un_recalage_qui_n_avance_pas_rend_la_main_au_lieu_de_boucler() {
    // LA SOURCE PEUT MENTIR. Un `Recale` qui propose de reprendre la ou l'on
    // est deja n'est pas une invitation a recommencer : c'est une fin de tour.
    // Sans cette regle, la garantie de terminaison dependrait de la source.
    let mut appels = 0u32;
    let mut c = Charge::neuf();
    let (suivant, bilan) = remplis(&mut c, 42, Bornes::defaut(), |seq| {
        appels += 1;
        assert!(appels < 1_000, "boucle : {appels} appels");
        LectureTransport::Recale { nouveau: seq, perdues: 0 }
    });
    assert_eq!(appels, 1, "une seule lecture, puis on rend la main");
    assert_eq!(suivant, 42);
    assert_eq!(bilan.recalages, 0);
}

#[test]
fn un_recalage_en_arriere_est_refuse() {
    let mut c = Charge::neuf();
    let (suivant, _) = remplis(&mut c, 500, Bornes::defaut(), |_| {
        LectureTransport::Recale { nouveau: 1, perdues: 0 }
    });
    assert_eq!(suivant, 500, "le curseur ne recule jamais");
}

// ===========================================================================
// LE COUT D'UN TOUR : LES DEUX BORNES
// ===========================================================================

#[test]
fn mille_evenements_refuses_ne_coutent_qu_un_tour_borne() {
    // L'EPREUVE DE LA SECONDE BORNE. Compter seulement ce qui est AJOUTE ne
    // borne rien : mille refus tenaient dans un seul tour, pris a
    // l'ordonnanceur que ce canal est cense observer sans le perturber.
    let mut lectures = 0u32;
    let mut c = Charge::neuf();
    let bornes = Bornes::defaut();
    let (suivant, bilan) = remplis(&mut c, 1, bornes, |seq| {
        lectures += 1;
        assert!(lectures <= 1_000, "tour non borne : {lectures} lectures");
        if seq <= 1_000 {
            LectureTransport::Evenement(du_transport(seq, id::TELEMETRIE_ENVOI))
        } else {
            LectureTransport::PasEncore
        }
    });

    assert_eq!(lectures as usize, bornes.max_examines, "exactement la borne");
    assert_eq!(bilan.examines as usize, bornes.max_examines);
    assert!(bilan.borne_examens, "le tour dit qu'il s'est arrete sur la borne");
    assert_eq!(bilan.ajoutes, 0);
    assert_eq!(bilan.refuses as usize, bornes.max_examines);
    assert_eq!(suivant, 1 + bornes.max_examines as u64, "LE CURSEUR A PROGRESSE");
    assert!(c.is_empty());
}

#[test]
fn les_tours_successifs_finissent_par_tout_epuiser_sans_rien_rejouer() {
    // LE TRAVAIL EST ETALE, PAS REPETE. C'est la contrepartie de la borne :
    // elle ne serait qu'un abandon si le tour suivant recommencait a zero.
    let bornes = Bornes::defaut();
    let mut curseur = 1u64;
    let mut tours = 0u32;
    let mut vus = Vec::new();
    while curseur <= 1_000 {
        tours += 1;
        assert!(tours < 100, "trop de tours : la borne n'etale rien");
        let mut c = Charge::neuf();
        let (suivant, bilan) = remplis(&mut c, curseur, bornes, |seq| {
            if seq <= 1_000 {
                vus.push(seq);
                LectureTransport::Evenement(du_transport(seq, id::TELEMETRIE_ENVOI))
            } else {
                LectureTransport::PasEncore
            }
        });
        assert!(suivant > curseur, "un tour qui n'avance pas");
        assert!(bilan.examines as usize <= bornes.max_examines);
        curseur = suivant;
    }
    // CHAQUE NUMERO A ETE LU UNE FOIS, ET UNE SEULE.
    let mut tries = vus.clone();
    tries.sort_unstable();
    tries.dedup();
    assert_eq!(tries.len(), vus.len(), "un numero a ete relu");
    assert_eq!(vus.len(), 1_000);
    assert_eq!(tours as usize, 1_000usize.div_ceil(bornes.max_examines));
}

#[test]
fn la_borne_d_ajouts_limite_le_datagramme() {
    let bornes = Bornes::neuves(4, 64);
    let mut c = Charge::neuf();
    let (suivant, bilan) = remplis(&mut c, 1, bornes, |seq| {
        LectureTransport::Evenement(court(seq))
    });
    assert_eq!(bilan.ajoutes, 4);
    assert_eq!(suivant, 5);
    assert_eq!(lignes(&c), 4);
    assert!(!bilan.borne_examens, "c'est la borne d'ajouts qui a servi");
}

#[test]
fn un_melange_de_refus_et_d_ajouts_respecte_les_deux_bornes() {
    let bornes = Bornes::neuves(3, 10);
    let mut c = Charge::neuf();
    let (suivant, bilan) = remplis(&mut c, 1, bornes, |seq| {
        if seq % 2 == 0 {
            LectureTransport::Evenement(court(seq))
        } else {
            LectureTransport::Evenement(du_transport(seq, id::TELEMETRIE_ABANDON))
        }
    });
    assert_eq!(bilan.ajoutes, 3);
    assert!(bilan.examines <= 10);
    assert_eq!(bilan.ajoutes + bilan.refuses, bilan.examines);
    assert_eq!(suivant, 1 + bilan.examines as u64);
}

#[test]
fn aucune_source_ne_peut_rendre_un_tour_infini() {
    // LA GARANTIE DE TERMINAISON NE DEPEND PAS DE LA SOURCE. On lui fait
    // rendre le pire de chaque genre, en alternance, et le tour s'arrete.
    for genre in 0..4u32 {
        let mut appels = 0u32;
        let mut c = Tampon::<128>::neuf();
        let (suivant, bilan) = remplis(&mut c, 1, Bornes::defaut(), |seq| {
            appels += 1;
            assert!(appels <= MAX_EXAMINES as u32, "tour non borne, genre {genre}");
            match genre {
                0 => LectureTransport::Evenement(enorme(seq)),
                1 => LectureTransport::Evenement(du_transport(seq, id::TELEMETRIE_ENVOI)),
                2 => LectureTransport::Recale { nouveau: seq + 1, perdues: 1 },
                _ => LectureTransport::Evenement(court(seq)),
            }
        });
        assert!(suivant >= 1);
        assert!(bilan.examines as usize <= MAX_EXAMINES);
        assert!(bilan.ajoutes as usize <= MAX_AJOUTES);
    }
}

#[test]
fn les_bornes_par_defaut_sont_coherentes() {
    // La borne de cout doit etre PLUS LARGE que celle du datagramme, sinon une
    // poignee de refus amputerait un datagramme par ailleurs remplissable.
    assert!(MAX_EXAMINES > MAX_AJOUTES);
    let d = Bornes::defaut();
    assert_eq!(d.max_ajoutes, MAX_AJOUTES);
    assert_eq!(d.max_examines, MAX_EXAMINES);
}

#[test]
fn une_source_vide_rend_un_tour_nul_et_immediat() {
    let mut appels = 0u32;
    let mut c = Charge::neuf();
    let (suivant, bilan) = remplis(&mut c, 9, Bornes::defaut(), |_| {
        appels += 1;
        LectureTransport::PasEncore
    });
    // UNE lecture : il faut bien demander pour apprendre qu'il n'y a rien.
    assert_eq!(appels, 1);
    assert_eq!(bilan.examines, 1);
    assert_eq!(suivant, 9, "et le curseur ne bouge pas");
    assert_eq!(bilan.ajoutes, 0);
    assert_eq!(bilan.refuses, 0);
    assert_eq!(bilan.trop_grands, 0);
    assert_eq!(bilan.recalages, 0);
    assert_eq!(bilan.perdues, 0);
    assert!(!bilan.plein);
    assert!(!bilan.borne_examens, "on s'est arrete sur la fin, pas sur la borne");
}
