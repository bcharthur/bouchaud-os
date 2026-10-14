//! Position du moteur PCM out de l'AC'97, lue d'un seul tenant, et ce qu'il
//! reste a jouer.
//!
//! BOUCHAUD_AC97_POSITION_COHERENTE_V1
//!
//! # Le defaut
//!
//! `SNDCTL_DSP_GETODELAY` lisait l'index du descripteur courant (`CIV`) dans
//! `recolte`, puis le RELISAIT, puis lisait `PICB` (echantillons restants du
//! descripteur courant) -- trois lectures separees d'un moteur qui avance
//! pendant ce temps. Si le moteur finit un descripteur entre deux lectures :
//!
//!   * `CIV` ancien, `PICB` du descripteur SUIVANT (pleine longueur) : le
//!     descripteur courant, deja joue, est compte entier ;
//!   * `CIV` de `recolte` ancien, `CIV` relu nouveau : rien n'est retranche du
//!     premier descripteur en vol, deja joue lui aussi.
//!
//! Le delai remonte alors, sans rien d'ecrit, de ce qui avait deja ete joue de
//! ce descripteur au releve precedent. Banc os-primitives, H3a
//! (`pire_exces_odelay=7`) : 7 trames, une fois.
//!
//! # Le correctif
//!
//! `lit_position` releve `CIV`, `SR`, `PICB`, puis RELIT `CIV` : s'il a change,
//! le releve est recommence. Le meme releve sert a `recolte` ET au calcul du
//! delai. Module pur : un test d'hote rejoue la sequence fautive.

/// Nombre de descripteurs de la liste. Le materiel en impose 32.
pub const DESCRIPTEURS: usize = 32;

/// Releves recommences au plus avant de renoncer (le moteur ne change pas de
/// descripteur deux fois en quelques lectures de port : un descripteur dure au
/// moins quelques millisecondes).
pub const ESSAIS: usize = 8;

// Bits de `PO_SR`.
pub const SR_DCH: u16 = 0x01; // le moteur DMA s'est arrete
pub const SR_CELV: u16 = 0x02; // l'index courant est le dernier valide

/// Les registres que lit `lit_position`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Registre {
    Civ,
    Sr,
    Picb,
}

/// Un releve coherent : `picb` et `sr` appartiennent au descripteur `civ`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    pub civ: usize,
    pub picb: u16,
    pub sr: u16,
}

/// Releve `CIV`, `SR`, `PICB`, puis relit `CIV` ; recommence s'il a change.
///
/// Si le moteur changeait de descripteur a chaque releve (jamais vu, et
/// impossible a 48 kHz), le dernier `CIV` est rendu avec `picb = 0` : le
/// courant est tenu pour joue. Le delai annonce peut alors etre trop COURT,
/// jamais trop long -- un lecteur ecrit un peu plus tot, il n'attend pas un
/// delai qui ne baisse pas.
pub fn lit_position<F: FnMut(Registre) -> u16>(mut lit: F) -> Position {
    let mut civ = lit(Registre::Civ);
    for _ in 0..ESSAIS {
        let sr = lit(Registre::Sr);
        let picb = lit(Registre::Picb);
        let relu = lit(Registre::Civ);
        if relu == civ {
            return Position { civ: civ as usize % DESCRIPTEURS, picb, sr };
        }
        civ = relu;
    }
    Position { civ: civ as usize % DESCRIPTEURS, picb: 0, sr: 0 }
}

/// Descripteurs encore en vol d'apres le releve `p`, `ecriture` etant le
/// prochain que le pilote remplira (modulo `DESCRIPTEURS`).
///
/// Le moteur s'arrete DE LUI-MEME apres le dernier descripteur valide
/// (`DCH` + `CELV`) et `CIV` reste POINTE sur lui : la distance `CIV ->
/// ecriture` vaut alors 1 alors que tout est joue (BOUCHAUD_AC97_HORLOGE_V1).
pub fn occupes(ecriture: usize, p: Position) -> usize {
    let ecriture = ecriture % DESCRIPTEURS;
    let tout_joue = p.sr & SR_DCH != 0
        && p.sr & SR_CELV != 0
        && p.picb == 0
        && (p.civ + 1) % DESCRIPTEURS == ecriture;
    if tout_joue {
        0
    } else if ecriture >= p.civ {
        ecriture - p.civ
    } else {
        DESCRIPTEURS - p.civ + ecriture
    }
}

/// Trames a 48 kHz que le materiel doit encore jouer : les `en_vol`
/// descripteurs qui precedent `ecriture`, moins ce qui est deja joue du
/// premier d'entre eux s'il est le courant (`PICB` compte les echantillons de
/// 16 bits qui lui restent, deux par trame).
pub fn trames_en_vol(reelles: &[u16; DESCRIPTEURS], ecriture: usize, en_vol: usize, p: Position) -> usize {
    if en_vol == 0 {
        return 0;
    }
    let premier = ecriture.wrapping_sub(en_vol) % DESCRIPTEURS;
    let mut total = 0usize;
    for k in 0..en_vol {
        total += reelles[(premier + k) % DESCRIPTEURS] as usize;
    }
    if p.civ == premier {
        let restants = p.picb as usize / 2;
        let deja = (reelles[premier] as usize).saturating_sub(restants);
        total = total.saturating_sub(deja);
    }
    total
}
