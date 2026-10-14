//! Le delai AC'97 (`SNDCTL_DSP_GETODELAY`) remonte-t-il sans rien d'ecrit ?
//!
//! BOUCHAUD_AC97_POSITION_COHERENTE_V1. Banc os-primitives, H3a :
//! `pire_exces_odelay=7` -- le delai annonce a depasse, une fois, de 7 trames
//! ce qui restait a jouer. Ce test rejoue la sequence fautive : le moteur
//! finit un descripteur ENTRE la lecture de `CIV` et celle de `PICB`.

#[path = "../../src/drivers/audio/ac97_position.rs"]
mod position;

use position::{lit_position, occupes, trames_en_vol, Position, Registre, DESCRIPTEURS};

/// Un moteur scripte : chaque lecture rend la valeur suivante de sa file.
struct Moteur {
    lectures: Vec<(Registre, u16)>,
}

impl Moteur {
    fn lit(&mut self, r: Registre) -> u16 {
        assert!(!self.lectures.is_empty(), "lecture de trop : {r:?}");
        let (attendu, valeur) = self.lectures.remove(0);
        assert_eq!(attendu, r, "ordre des lectures");
        valeur
    }
}

/// 19 descripteurs de 256 trames en vol (cible de 100 ms), du 5 au 23.
fn etat() -> ([u16; DESCRIPTEURS], usize, usize) {
    let mut reelles = [0u16; DESCRIPTEURS];
    for k in 5..24 {
        reelles[k] = 256;
    }
    (reelles, 24, 19)
}

/// Ce que faisait le pilote : recolte sur une lecture de CIV, calcul sur une
/// autre, PICB sur une troisieme.
fn ancien_calcul(reelles: &[u16; DESCRIPTEURS], ecriture: usize, en_vol: usize, civ_recolte: usize, civ_calcul: usize, picb: u16) -> usize {
    let p = Position { civ: civ_recolte, picb: 1, sr: 0 };
    let reste = occupes(ecriture, p).min(en_vol);
    trames_en_vol(reelles, ecriture, reste, Position { civ: civ_calcul, picb, sr: 0 })
}

#[test]
fn le_releve_fautif_donne_les_7_trames_du_banc() {
    let (reelles, ecriture, en_vol) = etat();
    // Releve 1 : le descripteur 5 est joue a 7 trames (PICB = 249 trames).
    let avant = trames_en_vol(&reelles, ecriture, en_vol, Position { civ: 5, picb: 2 * 249, sr: 0 });
    assert_eq!(avant, 19 * 256 - 7);
    // Releve 2, ancien pilote : CIV lu 5, puis le moteur passe au 6 et charge
    // PICB a 512 echantillons ; le 5, joue, est compte entier.
    let apres = ancien_calcul(&reelles, ecriture, en_vol, 5, 5, 512);
    assert_eq!(apres - avant, 7, "le delai remonte de ce qui etait deja joue : la signature du banc");
    // Variante : CIV de la recolte ancien, CIV relu nouveau -- meme effet.
    let apres2 = ancien_calcul(&reelles, ecriture, en_vol, 5, 6, 512);
    assert_eq!(apres2 - avant, 7);
}

#[test]
fn le_releve_coherent_recommence_quand_civ_change() {
    // CIV=5, SR, PICB (deja celui du 6), CIV relu = 6 : recommence.
    let mut m = Moteur {
        lectures: vec![
            (Registre::Civ, 5),
            (Registre::Sr, 0),
            (Registre::Picb, 512),
            (Registre::Civ, 6),
            (Registre::Sr, 0),
            (Registre::Picb, 510),
            (Registre::Civ, 6),
        ],
    };
    let p = lit_position(|r| m.lit(r));
    assert_eq!(p, Position { civ: 6, picb: 510, sr: 0 });
    assert!(m.lectures.is_empty());

    // Le meme releve sert a la recolte et au calcul : le 5 sort, rien ne remonte.
    let (reelles, ecriture, en_vol) = etat();
    let avant = trames_en_vol(&reelles, ecriture, en_vol, Position { civ: 5, picb: 2 * 249, sr: 0 });
    let reste = occupes(ecriture, p);
    assert_eq!(reste, 18);
    let mut reelles_apres = reelles;
    reelles_apres[5] = 0;
    let apres = trames_en_vol(&reelles_apres, ecriture, reste, p);
    assert!(apres <= avant, "le delai remonte : {avant} -> {apres}");
    assert_eq!(apres, 18 * 256 - 1);
}

#[test]
fn un_releve_stable_ne_relit_qu_une_fois() {
    let mut m = Moteur {
        lectures: vec![(Registre::Civ, 37), (Registre::Sr, 3), (Registre::Picb, 0), (Registre::Civ, 37)],
    };
    let p = lit_position(|r| m.lit(r));
    // CIV est sur 5 bits cote materiel ; le modulo garde l'index dans la liste.
    assert_eq!(p, Position { civ: 37 % DESCRIPTEURS, picb: 0, sr: 3 });
}

#[test]
fn un_moteur_qui_ne_se_stabilise_pas_donne_un_delai_court_jamais_long() {
    let mut civ = 0u16;
    let p = lit_position(|r| match r {
        Registre::Civ => {
            civ += 1;
            civ
        }
        Registre::Sr => 0,
        Registre::Picb => 512,
    });
    assert_eq!(p.picb, 0, "courant tenu pour joue : trop court plutot que trop long");
}

/// Moteur arrete apres le dernier valide (DCH + CELV, PICB = 0, CIV pointe
/// dessus) : tout est joue (BOUCHAUD_AC97_HORLOGE_V1, conserve).
#[test]
fn moteur_arrete_sur_le_dernier_valide() {
    let p = Position { civ: 23, picb: 0, sr: position::SR_DCH | position::SR_CELV };
    assert_eq!(occupes(24, p), 0);
    let p = Position { civ: 23, picb: 2, sr: position::SR_DCH | position::SR_CELV };
    assert_eq!(occupes(24, p), 1);
}
