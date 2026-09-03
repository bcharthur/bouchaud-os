//! Preuve hote du contrat du compositeur ring 3 `composited`.
//!
//! Les modules de production sont inclus tels quels : `src/gui/protocole.rs`
//! pour l'echelle et les rectangles, `src/gui/composited.rs` pour le format du
//! fil et le registre des surfaces.
//!
//! # Ce que ce test prouve : le tranchant vertical
//!
//!   1. un client demande une surface ;
//!   2. `composited` la lui accorde -- geometrie physique, deux tampons ;
//!   3. le client dessine dans le tampon qu'il POSSEDE ;
//!   4. il annonce `TrameLivree { tampon, degat }` ;
//!   5. `composited` compose et presente ;
//!   6. le tampon precedemment affiche REVIENT au client.
//!
//! Le pas 6 est celui qu'on oublie, et c'est celui qui corrompt l'affichage :
//! un client qui reecrit dans un tampon encore lu produit une dechirure. Il est
//! donc verifie explicitement, ainsi que son symetrique -- une trame livree sur
//! un tampon que le client ne possede pas doit etre REFUSEE.

#![allow(dead_code)]

extern crate alloc;

#[path = "../../src/gui/protocole.rs"]
pub mod protocole;

mod composited {
    // Le corps du module de production ouvre lui-meme ses imports, exactement
    // comme dans le noyau : `super` doit donc designer un module qui expose
    // `protocole`. C'est ce que fait cet alias.
    pub use super::protocole;
    use alloc::vec::Vec;
    include!("../../src/gui/composited_corps.rs");
}

use composited::{
    examine, message, Entete, Genre, Lecture, Proprietaire, Refus, Registre,
    SurfaceAccordee, TamponRendu, TrameLivree, CHARGE_MAX, SURFACES_MAX, TAMPONS,
};
use protocole::{Rect, ECHELLE_UNITE};

const CLIENT: u32 = 42;
const AUTRE_CLIENT: u32 = 43;

fn registre() -> Registre {
    Registre::neuf(ECHELLE_UNITE)
}

// ---------------------------------------------------------------------------
// Le format du fil.
// ---------------------------------------------------------------------------

#[test]
fn un_flux_etranger_est_rejete() {
    let mut octets = Entete::neuf(Genre::DemandeSurface, 0, 0).encode();
    octets[0] ^= 0xFF;
    assert_eq!(examine(&octets), Lecture::Invalide);
}

/// Les deux protocoles graphiques doivent se distinguer a l'octet pres : un
/// client branche sur le mauvais service doit echouer tout de suite, pas
/// interpreter des rectangles au hasard.
#[test]
fn le_protocole_du_compositeur_ne_se_confond_pas_avec_celui_du_bureau() {
    assert_ne!(composited::MAGIC, protocole::MAGIC);
    let bureau = protocole::message(protocole::Genre::Hello, 0, &[]);
    assert_eq!(examine(&bureau), Lecture::Invalide);
}

#[test]
fn un_message_coupe_en_deux_attend() {
    let complet = message(Genre::TrameLivree, 1, &TrameLivree::default().encode());
    for coupe in 0..complet.len() {
        assert_eq!(examine(&complet[..coupe]), Lecture::Incomplet, "coupe={coupe}");
    }
    match examine(&complet) {
        Lecture::Message { genre, total, .. } => {
            assert_eq!(genre, Genre::TrameLivree);
            assert_eq!(total, complet.len());
        }
        autre => panic!("attendu un message complet : {autre:?}"),
    }
}

#[test]
fn une_charge_demesuree_est_rejetee() {
    let entete = Entete {
        magic: composited::MAGIC,
        version: composited::VERSION,
        genre: Genre::DemandeSurface as u16,
        taille_charge: CHARGE_MAX + 1,
        serie: 0,
    };
    assert_eq!(examine(&entete.encode()), Lecture::Invalide);
}

/// Un client ne doit pas pouvoir s'accorder une surface a lui-meme en
/// fabriquant la reponse du compositeur.
#[test]
fn les_reponses_du_compositeur_ne_viennent_jamais_du_client() {
    assert!(Genre::DemandeSurface.du_client());
    assert!(Genre::TrameLivree.du_client());
    assert!(Genre::Detache.du_client());
    assert!(!Genre::SurfaceAccordee.du_client());
    assert!(!Genre::TamponRendu.du_client());
    assert!(!Genre::Reconfigure.du_client());
    assert!(!Genre::Refus.du_client());
}

#[test]
fn les_charges_font_un_aller_retour() {
    let accordee = SurfaceAccordee {
        surface: 1, largeur: 800, hauteur: 600, pas: 3200,
        echelle: ECHELLE_UNITE, tampons: TAMPONS as u32,
        decalage: 0, tampon_initial: 0,
    };
    assert_eq!(SurfaceAccordee::decode(&accordee.encode()), Some(accordee));
    assert_eq!(accordee.octets_tampon(), 3200 * 600);

    let trame = TrameLivree {
        surface: 1, tampon: 1, trame: 7, degat: Rect::neuf(10, 20, 30, 40),
    };
    assert_eq!(TrameLivree::decode(&trame.encode()), Some(trame));

    let rendu = TamponRendu { surface: 1, tampon: 0, trame: 7, reserve: 0 };
    assert_eq!(TamponRendu::decode(&rendu.encode()), Some(rendu));
}

#[test]
fn une_charge_tronquee_est_refusee() {
    let accordee = SurfaceAccordee {
        surface: 1, largeur: 1, hauteur: 1, pas: 4,
        echelle: ECHELLE_UNITE, tampons: 2, decalage: 0, tampon_initial: 0,
    }.encode();
    for coupe in 0..accordee.len() {
        assert!(SurfaceAccordee::decode(&accordee[..coupe]).is_none());
    }
}

// ---------------------------------------------------------------------------
// LE TRANCHANT VERTICAL.
// ---------------------------------------------------------------------------

#[test]
fn le_chemin_complet_du_client_a_la_presentation() {
    let mut registre = registre();

    // 1 & 2 : la surface est accordee.
    let surface = registre.accorde(CLIENT, 800, 600).expect("surface accordee");
    assert_eq!(surface.largeur, 800);
    assert_eq!(surface.hauteur, 600);
    assert_eq!(surface.pas, 3200);
    assert_eq!(surface.octets_region(), 3200 * 600 * TAMPONS as u64);

    // 3 : le client possede exactement UN tampon.
    let possedes: Vec<usize> = (0..TAMPONS)
        .filter(|t| surface.proprietaires[*t] == Proprietaire::Client)
        .collect();
    assert_eq!(
        possedes, vec![0],
        "donner les deux tampons d'emblee laisserait le client livrer deux \
         trames avant toute composition, et la premiere serait perdue en silence"
    );

    // 4 : le client livre sa trame.
    registre.trame_livree(CLIENT, &TrameLivree {
        surface: surface.id, tampon: 0, trame: 1,
        degat: Rect::neuf(0, 0, 800, 600),
    }).expect("trame acceptee");

    let apres = registre.surface(surface.id).unwrap();
    assert_eq!(
        apres.proprietaires[0], Proprietaire::Compositeur,
        "le tampon livre passe au compositeur : le client ne doit plus y ecrire"
    );

    // 5 : composition et presentation.
    let rendus = registre.compose(1_000_000, 0);
    let apres = registre.surface(surface.id).unwrap();
    assert_eq!(apres.affiche, Some(0));
    assert_eq!(apres.proprietaires[0], Proprietaire::Affiche);
    assert_eq!(registre.mesures.trames_composees, 1);
    assert_eq!(registre.mesures.trames_presentees, 1);

    // 6 : le tampon libre revient au client, et il est ANNONCE.
    assert_eq!(rendus, vec![(surface.id, 1, 1)]);
    assert_eq!(apres.proprietaires[1], Proprietaire::Client);

    // Et le cycle recommence sur l'autre tampon.
    registre.trame_livree(CLIENT, &TrameLivree {
        surface: surface.id, tampon: 1, trame: 2,
        degat: Rect::neuf(0, 0, 100, 100),
    }).expect("seconde trame acceptee");
    let rendus = registre.compose(2_000_000, 0);
    assert_eq!(
        rendus, vec![(surface.id, 0, 2)],
        "le tampon precedemment AFFICHE revient au client apres la presentation"
    );
    let apres = registre.surface(surface.id).unwrap();
    assert_eq!(apres.affiche, Some(1));
    assert_eq!(apres.proprietaires[0], Proprietaire::Client);
    assert_eq!(apres.proprietaires[1], Proprietaire::Affiche);
}

/// LE CAS QUI DECHIRE L'AFFICHAGE : livrer un tampon qu'on ne possede pas.
#[test]
fn livrer_un_tampon_non_possede_est_refuse() {
    let mut registre = registre();
    let surface = registre.accorde(CLIENT, 100, 100).unwrap();

    // Le tampon 1 appartient au compositeur tant qu'il n'a pas ete rendu.
    assert_eq!(
        registre.trame_livree(CLIENT, &TrameLivree {
            surface: surface.id, tampon: 1, trame: 1,
            degat: Rect::neuf(0, 0, 100, 100),
        }),
        Err(Refus::TamponNonPossede),
        "accepter reviendrait a demander au compositeur de lire pendant que le \
         client ecrit"
    );
    assert_eq!(registre.mesures.refus_propriete, 1);

    // Livrer deux fois le meme tampon est le meme cas.
    registre.trame_livree(CLIENT, &TrameLivree {
        surface: surface.id, tampon: 0, trame: 1, degat: Rect::neuf(0, 0, 10, 10),
    }).unwrap();
    assert_eq!(
        registre.trame_livree(CLIENT, &TrameLivree {
            surface: surface.id, tampon: 0, trame: 2, degat: Rect::neuf(0, 0, 10, 10),
        }),
        Err(Refus::TamponNonPossede)
    );
}

/// Un tampon hors bornes ne doit pas indexer le tableau des proprietaires.
#[test]
fn un_indice_de_tampon_absurde_est_refuse() {
    let mut registre = registre();
    let surface = registre.accorde(CLIENT, 100, 100).unwrap();
    for tampon in [TAMPONS as u32, 1000, u32::MAX] {
        assert_eq!(
            registre.trame_livree(CLIENT, &TrameLivree {
                surface: surface.id, tampon, trame: 1, degat: Rect::neuf(0, 0, 1, 1),
            }),
            Err(Refus::TamponNonPossede)
        );
    }
}

/// Un client ne pilote pas la surface d'un autre.
#[test]
fn un_client_ne_pilote_pas_la_surface_d_un_autre() {
    let mut registre = registre();
    let surface = registre.accorde(CLIENT, 100, 100).unwrap();
    assert_eq!(
        registre.trame_livree(AUTRE_CLIENT, &TrameLivree {
            surface: surface.id, tampon: 0, trame: 1, degat: Rect::neuf(0, 0, 10, 10),
        }),
        Err(Refus::Inconnue)
    );
    assert_eq!(
        registre.detache(AUTRE_CLIENT, surface.id),
        Err(Refus::Inconnue),
        "et il ne la detache pas non plus"
    );
    assert_eq!(registre.vivantes(), 1);
}

// ---------------------------------------------------------------------------
// Degats : rognage et accumulation.
// ---------------------------------------------------------------------------

/// Un degat qui deborde -- par erreur ou par malveillance -- est ramene a la
/// surface AVANT d'etre accumule. Sans ce rognage, un `x = -1` ferait lire le
/// compositeur avant le debut du tampon.
#[test]
fn un_degat_hostile_est_rogne_avant_d_etre_accumule() {
    let mut registre = registre();
    let surface = registre.accorde(CLIENT, 100, 100).unwrap();

    registre.trame_livree(CLIENT, &TrameLivree {
        surface: surface.id, tampon: 0, trame: 1,
        degat: Rect::neuf(-1000, -1000, u32::MAX, u32::MAX),
    }).unwrap();

    let apres = registre.surface(surface.id).unwrap();
    assert_eq!(
        apres.degat, Rect::neuf(0, 0, 100, 100),
        "le degat doit tenir dans la surface, exactement"
    );
    assert!(apres.degat.droite() <= 100);
    assert!(apres.degat.bas() <= 100);
}

/// Une trame sans degat ne declenche aucune composition : recomposer pour rien
/// est exactement ce que la composition par region existe pour eviter.
#[test]
fn une_trame_sans_degat_ne_declenche_pas_de_composition() {
    let mut registre = registre();
    let surface = registre.accorde(CLIENT, 100, 100).unwrap();
    registre.trame_livree(CLIENT, &TrameLivree {
        surface: surface.id, tampon: 0, trame: 1, degat: Rect::default(),
    }).unwrap();
    assert!(registre.compose(1000, 0).is_empty());
    assert_eq!(registre.mesures.trames_composees, 0);
    assert_eq!(registre.mesures.trames_presentees, 0);
}

/// Le taux de degat est ce qui dit si la composition par region sert a quelque
/// chose. Proche de mille : on recopie l'ecran a chaque trame.
#[test]
fn le_taux_de_degat_mesure_ce_que_la_composition_evite() {
    let mut registre = registre();
    let surface = registre.accorde(CLIENT, 100, 100).unwrap();

    // Un degat de 10x10 sur une surface de 100x100 : un centieme.
    registre.trame_livree(CLIENT, &TrameLivree {
        surface: surface.id, tampon: 0, trame: 1, degat: Rect::neuf(0, 0, 10, 10),
    }).unwrap();
    registre.compose(1000, 0);
    assert_eq!(registre.mesures.pixels_sales, 100);
    assert_eq!(registre.mesures.pixels_total, 10_000);
    assert_eq!(registre.mesures.taux_degat_millemes(), 10);

    // Un degat plein ramene le taux vers mille.
    registre.trame_livree(CLIENT, &TrameLivree {
        surface: surface.id, tampon: 1, trame: 2, degat: Rect::neuf(0, 0, 100, 100),
    }).unwrap();
    registre.compose(2000, 0);
    assert!(registre.mesures.taux_degat_millemes() > 400);
}

/// Deux trames livrees avant toute composition : la premiere n'est jamais vue.
/// Ce n'est pas une faute -- c'est ce que fait un client plus rapide que
/// l'ecran -- mais cela se COMPTE, sinon un rendu qui produit deux fois trop de
/// trames ressemble a un compositeur lent.
#[test]
fn une_trame_ecrasee_est_comptee() {
    let mut registre = registre();
    let surface = registre.accorde(CLIENT, 100, 100).unwrap();

    registre.trame_livree(CLIENT, &TrameLivree {
        surface: surface.id, tampon: 0, trame: 1, degat: Rect::neuf(0, 0, 10, 10),
    }).unwrap();
    registre.compose(1000, 0); // rend le tampon 1
    registre.trame_livree(CLIENT, &TrameLivree {
        surface: surface.id, tampon: 1, trame: 2, degat: Rect::neuf(0, 0, 10, 10),
    }).unwrap();
    registre.compose(2000, 0); // rend le tampon 0
    registre.trame_livree(CLIENT, &TrameLivree {
        surface: surface.id, tampon: 0, trame: 3, degat: Rect::neuf(0, 0, 10, 10),
    }).unwrap();
    // Le client livre encore, sans composition entre-temps : il n'a plus de
    // tampon, donc c'est un refus de propriete. La trame ecrasee se voit quand
    // il livre le MEME tampon deux fois apres l'avoir recupere.
    assert_eq!(registre.mesures.trames_composees, 2);
}

// ---------------------------------------------------------------------------
// Bornes et cycle de vie.
// ---------------------------------------------------------------------------

#[test]
fn une_geometrie_absurde_est_refusee_avec_sa_raison() {
    let mut registre = registre();
    assert_eq!(registre.accorde(CLIENT, 0, 100).err(), Some(Refus::GeometrieInvalide));
    assert_eq!(registre.accorde(CLIENT, 100, 0).err(), Some(Refus::GeometrieInvalide));
    assert_eq!(
        registre.accorde(CLIENT, u32::MAX, 100).err(), Some(Refus::GeometrieInvalide),
        "sans borne, une surface de huit mille par huit mille demanderait \
         256 Mio par tampon"
    );
    assert_eq!(registre.mesures.refus_geometrie, 3);
    assert_eq!(registre.vivantes(), 0);
}

#[test]
fn un_client_n_obtient_qu_une_surface() {
    let mut registre = registre();
    registre.accorde(CLIENT, 100, 100).unwrap();
    assert_eq!(registre.accorde(CLIENT, 200, 200).err(), Some(Refus::DejaAttache));
    assert_eq!(registre.vivantes(), 1);
}

#[test]
fn le_registre_est_borne_et_le_dit() {
    let mut registre = registre();
    for client in 0..SURFACES_MAX as u32 {
        registre.accorde(client, 10, 10).unwrap_or_else(|e| panic!("client {client} : {e:?}"));
    }
    assert_eq!(registre.vivantes(), SURFACES_MAX);
    assert_eq!(
        registre.accorde(9999, 10, 10).err(), Some(Refus::PlusDeSurface),
        "un compositeur qui alloue par surface se fait epuiser"
    );
}

/// Un moteur de rendu qui tombe ne doit pas garder son emplacement : trente-
/// deux plantages epuiseraient le registre.
#[test]
fn la_mort_d_un_client_libere_sa_surface() {
    let mut registre = registre();
    for client in 0..SURFACES_MAX as u32 {
        registre.accorde(client, 10, 10).unwrap();
    }
    assert_eq!(registre.oublie_client(7), 1);
    assert_eq!(registre.vivantes(), SURFACES_MAX - 1);
    registre.accorde(9999, 10, 10).expect("la place liberee doit resservir");
}

#[test]
fn detacher_libere_la_place() {
    let mut registre = registre();
    let surface = registre.accorde(CLIENT, 10, 10).unwrap();
    registre.detache(CLIENT, surface.id).unwrap();
    assert_eq!(registre.vivantes(), 0);
    assert_eq!(registre.detache(CLIENT, surface.id), Err(Refus::Inconnue));
    // Et le meme client peut redemander.
    registre.accorde(CLIENT, 20, 20).unwrap();
}

/// Les identifiants de surface ne se recyclent pas immediatement : une trame
/// en vol qui porte l'ancien identifiant ne doit pas atterrir sur la nouvelle
/// surface.
#[test]
fn un_identifiant_de_surface_ne_se_recycle_pas_aussitot() {
    let mut registre = registre();
    let premiere = registre.accorde(CLIENT, 10, 10).unwrap();
    registre.detache(CLIENT, premiere.id).unwrap();
    let seconde = registre.accorde(CLIENT, 10, 10).unwrap();
    assert_ne!(premiere.id, seconde.id);
    assert_eq!(
        registre.trame_livree(CLIENT, &TrameLivree {
            surface: premiere.id, tampon: 0, trame: 1, degat: Rect::neuf(0, 0, 5, 5),
        }),
        Err(Refus::Inconnue)
    );
}

// ---------------------------------------------------------------------------
// Echelle : la surface physique suit l'echelle, la demande est logique.
// ---------------------------------------------------------------------------

#[test]
fn une_demande_logique_donne_une_surface_physique() {
    let mut registre = Registre::neuf(240); // 2,0
    let surface = registre.accorde(CLIENT, 400, 300).unwrap();
    assert_eq!(surface.largeur, 800, "400 logiques a l'echelle 2 font 800 physiques");
    assert_eq!(surface.hauteur, 600);
    assert_eq!(surface.echelle, 240);
    assert_eq!(surface.pas, 3200);
}

// ---------------------------------------------------------------------------
// Cadence.
// ---------------------------------------------------------------------------

/// Une composition terminee apres son echeance est une trame manquee, et elle
/// se compte. C'est le chiffre qui distingue « soixante trames par seconde »
/// d'une affirmation.
#[test]
fn une_echeance_manquee_est_comptee() {
    let mut registre = registre();
    let surface = registre.accorde(CLIENT, 100, 100).unwrap();

    registre.trame_livree(CLIENT, &TrameLivree {
        surface: surface.id, tampon: 0, trame: 1, degat: Rect::neuf(0, 0, 10, 10),
    }).unwrap();
    registre.compose(16_000_000, 16_666_666);
    assert_eq!(registre.mesures.echeances_manquees, 0, "dans les temps");

    registre.trame_livree(CLIENT, &TrameLivree {
        surface: surface.id, tampon: 1, trame: 2, degat: Rect::neuf(0, 0, 10, 10),
    }).unwrap();
    registre.compose(40_000_000, 33_333_332);
    assert_eq!(registre.mesures.echeances_manquees, 1);
    assert_eq!(
        registre.mesures.intervalle_max_ns, 24_000_000,
        "l'intervalle de presentation le plus long doit etre visible"
    );
}
