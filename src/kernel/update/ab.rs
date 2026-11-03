// Mise a jour atomique A/B : la logique PURE (BOUCHAUD_UPDATE_AB_V1).
//
// Ce fichier ne connait ni le NVMe, ni l'ESP, ni le chargeur : seulement des
// octets et des etats. Il est inclus tel quel par tools/update/test_ab.rs,
// qui le met en echec sur machine hote (coupure d'alimentation a chaque octet
// d'ecriture du bloc d'amorcage, a chaque etape d'une mise a jour, boucles de
// crash). C'est le meme motif que installation/disposition.rs : ce qui decide
// si la machine redemarre ne se met pas au point sur la machine.
//
// # Le contrat
//
//   download -> verify -> install inactive -> flush -> mark candidate
//            -> boot (essais decomptes AVANT le boot) -> health check -> commit
//   echec ou essais epuises -> rollback vers le slot actif, qui n'a pas bouge
//
// Invariants (verifies par le test hote) :
//   I1  a tout instant, au moins un slot Good existe et `choisit_boot` le rend
//       si aucun candidat n'a d'essai ;
//   I2  un slot Bad n'est jamais choisi ;
//   I3  un candidat n'est boote qu'un nombre BORNE de fois sans health check ;
//   I4  une ecriture coupee du bloc d'amorcage rend l'ancien ou le nouvel
//       enregistrement, jamais un melange (deux copies, sequence + CRC) ;
//   I5  le slot actif n'est jamais la cible d'une installation.

/// Les deux emplacements systeme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    A = 0,
    B = 1,
}

impl Slot {
    pub const fn autre(self) -> Slot {
        match self { Slot::A => Slot::B, Slot::B => Slot::A }
    }
    const fn index(self) -> usize { self as usize }
}

/// Etat d'un emplacement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Etat {
    /// Rien d'amorcable (jamais installe, ou en cours d'ecriture).
    Vide,
    /// Ecrit, verifie, vide sur le disque, et en attente de boot.
    Candidat { essais: u8 },
    /// A demarre ET passe son controle de sante au moins une fois.
    Bon,
    /// A echoue : ne sera plus jamais choisi tant qu'on ne le reinstalle pas.
    Mauvais,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InfoSlot {
    pub etat: Etat,
    /// Empreinte SHA-256 de l'image systeme posee dans ce slot.
    pub image: [u8; 32],
}

/// Le bloc d'amorcage : ce que le chargeur lit pour choisir un slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Enregistrement {
    /// Augmente a chaque ecriture. Departage les deux copies.
    pub sequence: u64,
    pub actif: Slot,
    pub slots: [InfoSlot; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refus {
    /// I5 : on n'ecrit jamais par-dessus le systeme qui tourne.
    CibleActive,
    /// L'image ecrite ne correspond pas a l'empreinte annoncee.
    ImageNonVerifiee,
    /// Un candidat est deja en attente : finir (commit/rollback) d'abord.
    CandidatEnCours,
    /// Le slot n'est pas dans l'etat que l'operation suppose.
    EtatInattendu,
    /// Nombre d'essais nul : le candidat ne serait jamais boote.
    EssaisNuls,
}

impl Enregistrement {
    /// Premiere installation : un seul systeme, deja juge bon.
    pub const fn initial(actif: Slot, image: [u8; 32]) -> Self {
        let vide = InfoSlot { etat: Etat::Vide, image: [0; 32] };
        let mut slots = [vide, vide];
        slots[actif.index()] = InfoSlot { etat: Etat::Bon, image };
        Enregistrement { sequence: 1, actif, slots }
    }

    pub fn slot(&self, s: Slot) -> InfoSlot { self.slots[s.index()] }

    fn suivant(&self) -> Self {
        let mut n = *self;
        n.sequence = self.sequence.wrapping_add(1);
        n
    }

    /// Etape 1 : declarer que le slot inactif va etre ecrit. Il devient Vide
    /// AVANT la premiere ecriture d'image : une coupure pendant l'ecriture
    /// laisse un slot Vide, jamais un slot « Bon » a moitie ecrit.
    pub fn commence_installation(&self, cible: Slot) -> Result<Self, Refus> {
        if cible == self.actif { return Err(Refus::CibleActive); }
        if matches!(self.slot(cible).etat, Etat::Candidat { .. }) {
            return Err(Refus::CandidatEnCours);
        }
        let mut n = self.suivant();
        n.slots[cible.index()] = InfoSlot { etat: Etat::Vide, image: [0; 32] };
        Ok(n)
    }

    /// Etape 2 : l'image est ecrite, relue, son empreinte verifiee et le
    /// disque vide (flush). Le slot devient candidat avec `essais` boots.
    pub fn marque_candidat(
        &self,
        cible: Slot,
        image: [u8; 32],
        empreinte_relue: [u8; 32],
        essais: u8,
    ) -> Result<Self, Refus> {
        if cible == self.actif { return Err(Refus::CibleActive); }
        if self.slot(cible).etat != Etat::Vide { return Err(Refus::EtatInattendu); }
        if image != empreinte_relue { return Err(Refus::ImageNonVerifiee); }
        if essais == 0 { return Err(Refus::EssaisNuls); }
        let mut n = self.suivant();
        n.slots[cible.index()] = InfoSlot { etat: Etat::Candidat { essais }, image };
        Ok(n)
    }

    /// Au demarrage : quel slot booter, et l'enregistrement a ECRIRE AVANT de
    /// le booter. L'essai est consomme avant le boot : un candidat qui fige la
    /// machine ne peut pas etre reessaye indefiniment (I3).
    pub fn choisit_boot(&self) -> (Slot, Self) {
        let autre = self.actif.autre();
        match self.slot(autre).etat {
            Etat::Candidat { essais } if essais > 0 => {
                let mut n = self.suivant();
                n.slots[autre.index()].etat = Etat::Candidat { essais: essais - 1 };
                (autre, n)
            }
            Etat::Candidat { .. } => {
                // Essais epuises sans controle de sante : retour arriere.
                let mut n = self.suivant();
                n.slots[autre.index()].etat = Etat::Mauvais;
                (self.actif, n)
            }
            _ => (self.actif, *self),
        }
    }

    /// Le systeme boote dans `slot` a passe son controle de sante.
    /// Un candidat devient Bon ET actif ; l'ancien actif reste Bon : c'est la
    /// version vers laquelle un retour arriere manuel reste possible.
    pub fn sante_ok(&self, slot: Slot) -> Self {
        match self.slot(slot).etat {
            Etat::Candidat { .. } => {
                let mut n = self.suivant();
                n.slots[slot.index()].etat = Etat::Bon;
                n.actif = slot;
                n
            }
            _ => *self,
        }
    }

    /// Le controle de sante du candidat a echoue : il ne sera plus choisi.
    pub fn sante_ko(&self, slot: Slot) -> Self {
        match self.slot(slot).etat {
            Etat::Candidat { .. } => {
                let mut n = self.suivant();
                n.slots[slot.index()].etat = Etat::Mauvais;
                n
            }
            _ => *self,
        }
    }

    /// Retour arriere manuel vers la version precedente, si elle est Bonne.
    pub fn revient(&self) -> Result<Self, Refus> {
        let autre = self.actif.autre();
        if self.slot(autre).etat != Etat::Bon { return Err(Refus::EtatInattendu); }
        let mut n = self.suivant();
        n.actif = autre;
        Ok(n)
    }
}

// --- Encodage sur disque : deux copies, sequence + CRC (I4) ---------------

pub const OCTETS: usize = 96;
const MAGIC: [u8; 8] = *b"BOAB0001";

fn crc32(octets: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &o in octets {
        crc ^= o as u32;
        for _ in 0..8 {
            let masque = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & masque);
        }
    }
    !crc
}

fn encode_etat(e: Etat) -> [u8; 2] {
    match e {
        Etat::Vide => [0, 0],
        Etat::Candidat { essais } => [1, essais],
        Etat::Bon => [2, 0],
        Etat::Mauvais => [3, 0],
    }
}

fn decode_etat(b: [u8; 2]) -> Option<Etat> {
    match b {
        [0, 0] => Some(Etat::Vide),
        [1, n] => Some(Etat::Candidat { essais: n }),
        [2, 0] => Some(Etat::Bon),
        [3, 0] => Some(Etat::Mauvais),
        _ => None,
    }
}

/// magic(8) seq(8) actif(1) pad(1) etatA(2) etatB(2) pad(2) imageA(32) imageB(32) pad(4) crc(4)
pub fn encode(e: &Enregistrement) -> [u8; OCTETS] {
    let mut o = [0u8; OCTETS];
    o[0..8].copy_from_slice(&MAGIC);
    o[8..16].copy_from_slice(&e.sequence.to_le_bytes());
    o[16] = e.actif as u8;
    o[18..20].copy_from_slice(&encode_etat(e.slots[0].etat));
    o[20..22].copy_from_slice(&encode_etat(e.slots[1].etat));
    o[24..56].copy_from_slice(&e.slots[0].image);
    o[56..88].copy_from_slice(&e.slots[1].image);
    let crc = crc32(&o[..OCTETS - 4]);
    o[OCTETS - 4..].copy_from_slice(&crc.to_le_bytes());
    o
}

pub fn decode(o: &[u8]) -> Option<Enregistrement> {
    if o.len() != OCTETS || o[0..8] != MAGIC { return None; }
    let crc = u32::from_le_bytes([o[92], o[93], o[94], o[95]]);
    if crc32(&o[..OCTETS - 4]) != crc { return None; }
    let actif = match o[16] { 0 => Slot::A, 1 => Slot::B, _ => return None };
    let mut a = [0u8; 32];
    let mut b = [0u8; 32];
    a.copy_from_slice(&o[24..56]);
    b.copy_from_slice(&o[56..88]);
    let e = Enregistrement {
        sequence: u64::from_le_bytes([o[8], o[9], o[10], o[11], o[12], o[13], o[14], o[15]]),
        actif,
        slots: [
            InfoSlot { etat: decode_etat([o[18], o[19]])?, image: a },
            InfoSlot { etat: decode_etat([o[20], o[21]])?, image: b },
        ],
    };
    // Un enregistrement dont l'actif n'est pas Bon ne peut pas exister : le
    // refuser plutot que de booter un slot jamais valide.
    if e.slot(e.actif).etat != Etat::Bon { return None; }
    Some(e)
}

/// Lit les deux copies et rend la plus recente VALIDE.
pub fn selectionne(copie0: &[u8], copie1: &[u8]) -> Option<Enregistrement> {
    match (decode(copie0), decode(copie1)) {
        (Some(x), Some(y)) => Some(if y.sequence > x.sequence { y } else { x }),
        (Some(x), None) => Some(x),
        (None, Some(y)) => Some(y),
        (None, None) => None,
    }
}

/// Quelle copie ecraser : jamais la plus recente valide.
pub fn copie_a_ecrire(copie0: &[u8], copie1: &[u8]) -> usize {
    match (decode(copie0), decode(copie1)) {
        (Some(x), Some(y)) => if x.sequence <= y.sequence { 0 } else { 1 },
        (Some(_), None) => 1,
        _ => 0,
    }
}
