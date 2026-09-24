//! L'anneau LAB tient-il sa promesse quand plusieurs coeurs ecrivent ?
//!
//! # Ce que ces epreuves defendent
//!
//! L'anneau est la SEULE source de verite du LAB : boite noire, shell, BRDP et
//! telemetrie y lisent tous. Une lecture dechiree non detectee y produirait le
//! pire resultat possible -- un evenement mi-ancien mi-neuf, avec un numero
//! qui le fait passer pour entier, et donc credible dans une enquete.
//!
//! On ne peut pas prouver l'absence de course par raisonnement seul, mais on
//! peut la rendre TRES probable et exiger qu'aucune incoherence ne sorte :
//! chaque evenement porte quatre arguments lies par une relation
//! arithmetique, et un melange de deux evenements la casse.
//!
//! Lance par `tools/ci/run_host_tests.sh`.

// Les deux modules sont pris A LA RACINE et non dans un `mod lab { ... }`
// englobant : `catalogue.rs` ecrit `use super::anneau`, et un module en ligne
// ajouterait un niveau de repertoire que `#[path]` ne saurait pas remonter.
#[path = "../../src/kernel/debug/lab/anneau.rs"]
mod anneau;
#[path = "../../src/kernel/debug/lab/catalogue.rs"]
mod catalogue;

use std::collections::HashSet;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anneau::{Anneau, Evenement, Lecture};
use catalogue::{id, Categorie, Forme, Style, DEFINITIONS};

// ===========================================================================
// L'anneau, seul
// ===========================================================================

#[test]
fn un_anneau_neuf_n_a_rien_a_dire() {
    let a: Anneau<8> = Anneau::nouveau();
    assert_eq!(a.emises(), 0);
    assert_eq!(a.prochaine(), 1);
    assert_eq!(a.plus_ancienne(), 1);
    assert_eq!(a.lis(1), Lecture::PasEncore);
}

#[test]
fn le_numero_zero_n_existe_pas() {
    // Zero est le « jamais servi » des emplacements. S'il designait aussi un
    // evenement, un emplacement vierge se lirait comme l'evenement zero.
    let a: Anneau<8> = Anneau::nouveau();
    a.emets(1, 0, 1, 2, [3, 4, 5, 6]);
    assert_eq!(a.lis(0), Lecture::Ecrasee);
}

#[test]
fn un_evenement_ressort_exactement_comme_il_est_entre() {
    let a: Anneau<8> = Anneau::nouveau();
    let seq = a.emets(123_456_789, 3, Categorie::Rtl8168 as u16, id::RX_DESC, [63, 0x8000_07ff, 0, 0xdead_beef]);
    assert_eq!(seq, 1);
    let Lecture::Evenement(ev) = a.lis(1) else { panic!("l'evenement doit etre lisible") };
    assert_eq!(
        ev,
        Evenement {
            seq: 1,
            t_ns: 123_456_789,
            cpu: 3,
            categorie: Categorie::Rtl8168 as u16,
            event_id: id::RX_DESC,
            args: [63, 0x8000_07ff, 0, 0xdead_beef],
        }
    );
}

#[test]
fn les_numeros_montent_sans_trou() {
    let a: Anneau<16> = Anneau::nouveau();
    for i in 1..=10u64 {
        assert_eq!(a.emets(i, 0, 0, 0, [i, 0, 0, 0]), i);
    }
    assert_eq!(a.emises(), 10);
    assert_eq!(a.prochaine(), 11);
    for i in 1..=10u64 {
        let Lecture::Evenement(ev) = a.lis(i) else { panic!("{i} illisible") };
        assert_eq!(ev.args[0], i);
    }
}

#[test]
fn ce_qui_a_fait_un_tour_est_declare_ecrase_et_non_devine() {
    // L'ANNEAU EST BORNE, ET LE DIT. Rendre un evenement plausible a la place
    // d'un evenement perdu serait le seul comportement vraiment dangereux.
    let a: Anneau<4> = Anneau::nouveau();
    for i in 1..=6u64 {
        a.emets(i, 0, 0, 0, [i, 0, 0, 0]);
    }
    assert_eq!(a.plus_ancienne(), 3, "quatre emplacements, six emissions");
    assert_eq!(a.lis(1), Lecture::Ecrasee);
    assert_eq!(a.lis(2), Lecture::Ecrasee);
    let Lecture::Evenement(ev) = a.lis(3) else { panic!("3 doit survivre") };
    assert_eq!(ev.args[0], 3);
    let Lecture::Evenement(ev) = a.lis(6) else { panic!("6 doit survivre") };
    assert_eq!(ev.args[0], 6);
    assert_eq!(a.lis(7), Lecture::PasEncore);
}

#[test]
fn un_lecteur_en_retard_apprend_combien_il_a_perdu() {
    let a: Anneau<4> = Anneau::nouveau();
    for i in 1..=10u64 {
        a.emets(i, 0, 0, 0, [i, 0, 0, 0]);
    }
    // Curseur reste a 1 : il en a rate six (les numeros 1 a 6).
    let (neuf, perdues) = a.recale(1);
    assert_eq!(neuf, 7);
    assert_eq!(perdues, 6);
    // Un lecteur a jour ne perd rien, et son curseur ne bouge pas.
    assert_eq!(a.recale(9), (9, 0));
    assert_eq!(a.recale(11), (11, 0));
}

#[test]
fn un_anneau_plein_mais_non_deborde_ne_perd_rien() {
    // La frontiere exacte : N emissions dans N emplacements.
    let a: Anneau<8> = Anneau::nouveau();
    for i in 1..=8u64 {
        a.emets(i, 0, 0, 0, [i, 0, 0, 0]);
    }
    assert_eq!(a.plus_ancienne(), 1);
    assert_eq!(a.recale(1), (1, 0));
    for i in 1..=8u64 {
        assert!(matches!(a.lis(i), Lecture::Evenement(_)), "{i} devait tenir");
    }
    // Une de plus, et la plus ancienne tombe.
    a.emets(9, 0, 0, 0, [9, 0, 0, 0]);
    assert_eq!(a.lis(1), Lecture::Ecrasee);
    assert_eq!(a.plus_ancienne(), 2);
}

// ===========================================================================
// PLUSIEURS COEURS
// ===========================================================================

/// La relation que porte tout evenement bien forme.
///
/// Un melange de deux evenements la casse dans tous les cas sauf collision
/// improbable : les quatre champs viennent du meme `v` par quatre chemins
/// differents.
fn charge(v: u64) -> [u64; 4] {
    [v, v ^ 0xFFFF_FFFF, v.wrapping_mul(2_654_435_761), !v]
}

fn charge_coherente(args: &[u64; 4]) -> bool {
    let v = args[0];
    args == &charge(v)
}

#[test]
fn quatre_emetteurs_concurrents_ne_produisent_aucun_evenement_hybride() {
    const EMETTEURS: u64 = 4;
    const PAR_EMETTEUR: u64 = 20_000;

    let a: Arc<Anneau<1024>> = Arc::new(Anneau::nouveau());
    let fini = Arc::new(AtomicBool::new(false));

    // LE LECTEUR TOURNE PENDANT L'ECRITURE, pas apres. Lire un anneau au repos
    // ne prouverait rien du protocole de sceau.
    let lecteur = {
        let a = Arc::clone(&a);
        let fini = Arc::clone(&fini);
        std::thread::spawn(move || {
            let mut curseur = 1u64;
            let (mut lus, mut perdus, mut dechirures) = (0u64, 0u64, 0u64);
            loop {
                match a.lis(curseur) {
                    Lecture::Evenement(ev) => {
                        assert!(
                            charge_coherente(&ev.args),
                            "EVENEMENT HYBRIDE seq={} args={:?} -- le sceau n'a pas \
tenu, et un lecteur aurait cru a un evenement entier",
                            ev.seq, ev.args,
                        );
                        assert_eq!(ev.seq, curseur);
                        lus += 1;
                        curseur += 1;
                    }
                    Lecture::Ecrasee => {
                        let (neuf, p) = a.recale(curseur);
                        perdus += p;
                        curseur = neuf.max(curseur + 1);
                    }
                    Lecture::Dechiree => dechirures += 1,
                    Lecture::PasEncore => {
                        if fini.load(Ordering::Acquire) && curseur >= a.prochaine() {
                            break;
                        }
                        std::hint::spin_loop();
                    }
                }
            }
            (lus, perdus, dechirures)
        })
    };

    let mut emetteurs = Vec::new();
    for e in 0..EMETTEURS {
        let a = Arc::clone(&a);
        emetteurs.push(std::thread::spawn(move || {
            for i in 0..PAR_EMETTEUR {
                let v = e * PAR_EMETTEUR + i + 1;
                a.emets(v, e as u16, Categorie::Lab as u16, id::LAB_DEMARRE, charge(v));
            }
        }));
    }
    for t in emetteurs {
        t.join().unwrap();
    }
    fini.store(true, Ordering::Release);
    let (lus, perdus, _dechirures) = lecteur.join().unwrap();

    assert_eq!(a.emises(), EMETTEURS * PAR_EMETTEUR);
    // Le lecteur voit tout ce qu'il n'a pas perdu, et RIEN de plus. L'egalite
    // interdit a la fois le doublon et le trou silencieux.
    assert_eq!(
        lus + perdus,
        EMETTEURS * PAR_EMETTEUR,
        "lus={lus} perdus={perdus} : la somme doit rendre compte de tout",
    );
}

#[test]
fn un_emetteur_seul_et_un_lecteur_ne_perdent_rien_si_l_anneau_est_assez_grand() {
    const COMBIEN: u64 = 5_000;
    let a: Arc<Anneau<8192>> = Arc::new(Anneau::nouveau());
    let ecrivain = {
        let a = Arc::clone(&a);
        std::thread::spawn(move || {
            for v in 1..=COMBIEN {
                a.emets(v, 0, Categorie::Lab as u16, id::LAB_DEMARRE, charge(v));
            }
        })
    };
    ecrivain.join().unwrap();

    let mut curseur = 1u64;
    let mut lus = 0u64;
    while curseur <= COMBIEN {
        match a.lis_stable(curseur, 8) {
            Lecture::Evenement(ev) => {
                assert!(charge_coherente(&ev.args));
                lus += 1;
                curseur += 1;
            }
            autre => panic!("{curseur} : {autre:?} alors que l'anneau contient tout"),
        }
    }
    assert_eq!(lus, COMBIEN);
}

// ===========================================================================
// LE CATALOGUE
// ===========================================================================

#[test]
fn aucun_identifiant_n_est_donne_deux_fois() {
    // Deux evenements sous un meme numero, et l'un des deux se lirait sous le
    // nom de l'autre. Dans un releve physique, c'est indetectable.
    let mut vus = HashSet::new();
    for def in DEFINITIONS {
        assert!(vus.insert(def.id), "identifiant {:#05x} en double ({})", def.id, def.nom);
    }
}

#[test]
fn aucun_nom_n_est_donne_deux_fois() {
    let mut vus = HashSet::new();
    for def in DEFINITIONS {
        assert!(vus.insert(def.nom), "nom {} en double", def.nom);
    }
}

#[test]
fn l_identifiant_porte_sa_categorie() {
    // On lit des vidages bruts quand le catalogue lui-meme est en cause.
    // `0x401` doit dire « audit » sans consulter la table.
    for def in DEFINITIONS {
        assert!(
            Categorie::de_l_identifiant(def.id).is_some(),
            "{} ({:#05x}) n'appartient a aucune categorie connue",
            def.nom, def.id,
        );
        assert!(def.id <= 0x6FF, "{} sort des categories", def.nom);
    }
}

#[test]
fn les_arguments_nommes_sont_les_premiers() {
    // UN TROU FERAIT RENDRE `arg3` SANS `arg2`, et personne ne saurait ce que
    // `arg3` compte. Le rendu s'arrete au premier `Absent` : un nom place
    // apres lui serait donc invisible, silencieusement.
    for def in DEFINITIONS {
        let mut fini = false;
        for i in 0..4 {
            if def.formes[i] == Forme::Absent {
                fini = true;
                assert!(
                    def.args[i].is_empty(),
                    "{} : l'argument {i} est nomme « {} » mais declare absent",
                    def.nom, def.args[i],
                );
            } else {
                assert!(!fini, "{} : l'argument {i} vient apres un trou", def.nom);
                assert!(!def.args[i].is_empty(), "{} : l'argument {i} n'a pas de nom", def.nom);
            }
        }
    }
}

#[test]
fn les_noms_d_arguments_sont_utilisables_en_json() {
    for def in DEFINITIONS {
        for nom in def.args.iter().filter(|n| !n.is_empty()) {
            assert!(
                nom.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{} : « {nom} » devra etre une cle JSON et un nom de champ Python",
                def.nom,
            );
        }
    }
}

#[test]
fn les_evenements_que_les_phases_annoncent_existent() {
    // L'ENONCE PHYSIQUE, EN TABLE. Ces noms sont ceux que la campagne doit
    // retrouver dans un vidage ; les perdre en cours de route, c'est perdre le
    // critere de reussite.
    for attendu in [
        "RING_WRAP_BEFORE",
        "RING_WRAP_AFTER",
        "RING_SECOND_LAP_TIMEOUT",
        "AUDIT_RX_DMA_STALL",
        "AUDIT_BLACKBOX_PERSISTENCE_STALL",
        "BB_PARTIEL_CHECKPOINT",
        "DHCP_IPV4_READY",
    ] {
        assert!(
            DEFINITIONS.iter().any(|d| d.nom == attendu),
            "{attendu} a disparu du catalogue",
        );
    }
}

fn rendu(ev: &Evenement, style: Style) -> String {
    let mut s = String::new();
    catalogue::rend(&mut s, ev, style).unwrap();
    s
}

fn exemple() -> Evenement {
    Evenement {
        seq: 41,
        t_ns: 12_345_678_901,
        cpu: 2,
        categorie: Categorie::Rtl8168 as u16,
        event_id: id::RX_REGISTRES,
        args: [0x0c, 0x0041, 0x0000_e70f, 0x2063],
    }
}

#[test]
fn les_trois_styles_nomment_le_meme_evenement_et_les_memes_arguments() {
    // LE POINT DE TOUT LE MODULE. Trois ponctuations, un seul vocabulaire :
    // une enquete ne doit pas traduire entre le shell, la boite noire et le
    // JSON pour reconnaitre un fait.
    let ev = exemple();
    let shell = rendu(&ev, Style::Shell);
    let bb = rendu(&ev, Style::Blackbox);
    let json = rendu(&ev, Style::Json);

    for texte in [&shell, &bb, &json] {
        assert!(texte.contains("RX_REGISTRES"), "{texte}");
        assert!(texte.contains("rtl8168"), "{texte}");
        for arg in ["chip_cmd", "intr_status", "rx_config", "cplus_cmd"] {
            assert!(texte.contains(arg), "{arg} manque dans : {texte}");
        }
    }
}

#[test]
fn le_style_shell_rend_les_registres_en_hexadecimal() {
    // Un registre en decimal est illisible, et c'est la forme -- donc la table
    // -- qui le sait, pas l'appelant.
    let shell = rendu(&exemple(), Style::Shell);
    assert!(shell.contains("chip_cmd=0x0c"), "{shell}");
    assert!(shell.contains("intr_status=0x0041"), "{shell}");
    assert!(shell.contains("rx_config=0x0000e70f"), "{shell}");
}

#[test]
fn le_style_json_garde_des_entiers_analysables() {
    // `0x0000e70f` dans un champ numerique casserait tout analyseur. Le client
    // PC formate lui-meme ; la machine envoie des nombres.
    let json = rendu(&exemple(), Style::Json);
    assert!(json.starts_with('{') && json.ends_with('}'), "{json}");
    assert!(json.contains("\"chip_cmd\":12"), "{json}");
    assert!(json.contains("\"rx_config\":59151"), "{json}");
    assert!(!json.contains("0x"), "aucun hexadecimal ne doit sortir en JSON : {json}");
    assert!(json.contains("\"seq\":41"));
    assert!(json.contains("\"t_ns\":12345678901"));
    assert!(json.contains("\"cpu\":2"));
}

#[test]
fn un_booleen_se_rend_selon_le_style() {
    let ev = Evenement {
        seq: 1,
        t_ns: 0,
        cpu: 0,
        categorie: Categorie::Rtl8168 as u16,
        event_id: id::RX_RECOVERY_END,
        args: [0, 3, 4_200_000_000, 64],
    };
    assert!(rendu(&ev, Style::Json).contains("\"effective\":false"));
    assert!(rendu(&ev, Style::Shell).contains("effective=0"));
    // Les nanosecondes se lisent en microsecondes au shell, brutes en JSON.
    assert!(rendu(&ev, Style::Shell).contains("age_ns=4200000us"));
    assert!(rendu(&ev, Style::Json).contains("\"age_ns\":4200000000"));
}

#[test]
fn un_evenement_inconnu_du_catalogue_ne_disparait_pas() {
    // C'EST QUAND LE CATALOGUE EST EN RETARD SUR LES SONDES qu'on a le plus
    // besoin de voir passer ce qu'on ne sait pas nommer. L'escamoter ferait
    // mentir le releve par omission.
    let ev = Evenement {
        seq: 7,
        t_ns: 1_000,
        cpu: 0,
        categorie: Categorie::Lab as u16,
        event_id: 0x0FE,
        args: [1, 2, 3, 4],
    };
    let shell = rendu(&ev, Style::Shell);
    assert!(shell.contains("EVENEMENT_INCONNU"), "{shell}");
    assert!(shell.contains("event_id=0x0fe"), "{shell}");
    assert!(shell.contains("arg0=0x1") && shell.contains("arg3=0x4"), "{shell}");

    let json = rendu(&ev, Style::Json);
    assert!(json.contains("\"event_id\":254"), "{json}");
    assert!(json.contains("\"arg0\":1"), "{json}");
    assert!(json.ends_with('}'));
}

#[test]
fn une_categorie_inconnue_ne_fait_pas_mentir_le_rendu() {
    let ev = Evenement {
        seq: 1,
        t_ns: 0,
        cpu: 0,
        categorie: 99,
        event_id: id::LAB_DEMARRE,
        args: [0; 4],
    };
    assert!(rendu(&ev, Style::Shell).contains(" ? "), "{}", rendu(&ev, Style::Shell));
    assert!(rendu(&ev, Style::Json).contains("\"cat\":\"?\""));
}

#[test]
fn chaque_definition_se_rend_sans_paniquer_dans_les_trois_styles() {
    // Une table qui grandit sans etre exercee finit par contenir une ligne qui
    // ne sort jamais correctement.
    for def in DEFINITIONS {
        let ev = Evenement {
            seq: 1,
            t_ns: u64::MAX,
            cpu: u16::MAX,
            categorie: Categorie::de_l_identifiant(def.id).unwrap() as u16,
            event_id: def.id,
            args: [u64::MAX, 0, 1, u64::MAX / 3],
        };
        for style in [Style::Shell, Style::Blackbox, Style::Json] {
            let texte = rendu(&ev, style);
            assert!(texte.contains(def.nom), "{} en {style:?}", def.nom);
            if style == Style::Json {
                assert!(texte.starts_with('{') && texte.ends_with('}'), "{texte}");
            }
        }
    }
}

#[test]
fn le_nom_d_un_identifiant_absent_est_dit_et_non_devine() {
    assert_eq!(catalogue::nom(id::AUDIT_RX_DMA_STALL), "AUDIT_RX_DMA_STALL");
    assert_eq!(catalogue::nom(0xFFFF), "EVENEMENT_INCONNU");
    assert!(catalogue::definition(0xFFFF).is_none());
}
