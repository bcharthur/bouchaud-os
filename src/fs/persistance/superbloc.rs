// Le superbloc A/B : ce qui fait qu'une coupure de courant ne laisse jamais un
// etat a moitie ecrit.
//
// CE QUE LE FORMAT V1 GARANTISSAIT, ET CE QU'IL NE GARANTISSAIT PAS
// =================================================================
//
// L'en-tete de `persistance.rs` etait honnete : « l'ecriture n'est pas
// atomique ». L'ordre -- contenu, puis table, puis en-tete -- etait le bon
// ordre, et il ne suffisait pas, pour une raison qui n'a rien a voir avec
// l'ordre :
//
//   * l'en-tete vit a un secteur FIXE. L'ecrire ecrase le precedent ;
//   * le contenu vit aux memes secteurs d'une synchronisation a l'autre. Le
//     reecrire ecrase le precedent.
//
// Une coupure pendant l'ecriture du contenu laissait donc l'ANCIEN en-tete --
// valide, magie correcte, nombre d'entrees correct -- pointant vers un contenu
// a moitie neuf. Au redemarrage, ce melange etait monte comme s'il etait
// coherent. C'est exactement l'etat que le chantier 5 interdit : « soit
// l'ancien, soit le nouveau, jamais un etat arbitrairement moitie ecrit
// utilise comme valide ».
//
// CE QUE LE FORMAT V2 ETABLIT
// ===========================
//
// Deux choses, et elles ne se remplacent pas :
//
//   1. DEUX DEMI-ZONES. Une synchronisation ecrit dans celle qui n'est PAS
//      active. Le contenu committe n'est jamais touche par l'ecriture en
//      cours ; une coupure au milieu n'abime que la moitie inactive, dont
//      personne ne depend ;
//   2. DEUX SUPERBLOCS, alternes, portant chacun une GENERATION et une somme
//      de controle. Le commit est l'ecriture d'un seul secteur : soit elle a
//      lieu, soit non. Si elle est dechiree, la somme de controle la rejette et
//      l'autre superbloc -- l'ancien, intact -- reste le bon.
//
// Le point de commit est donc reduit a UN secteur, et meme ce secteur-la peut
// echouer sans consequence. C'est ce qui rend l'invariant demontrable plutot
// que probable.
//
// CE QUE CE MODULE NE FAIT PAS
// ============================
//
// Il ne remplace pas la reecriture integrale par un journal. Une zone reecrite
// en entier a chaque `sync` convient a quelques mega-octets ecrits rarement ;
// un journal se justifie quand on ecrit souvent et beaucoup. Ce qui manquait
// n'etait pas le journal : c'etait le POINT DE COMMIT.

/// Reconnait une zone au format V2.
pub const MAGIE_V2: &[u8; 8] = b"BOPERSI2";
pub const VERSION_V2: u32 = 2;

/// Taille utile d'un superbloc, en octets : les quarante champs plus les
/// quatre de la somme de controle. Le reste du secteur est nul, et n'est pas
/// couvert -- un octet de bourrage qui change ne change rien.
pub const TAILLE_SUPERBLOC: usize = 44;

/// Le superbloc : ce qui designe la moitie valide, et comment le verifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Superbloc {
    /// Monte a chaque commit. La plus grande GAGNE : c'est la seule chose qui
    /// distingue l'etat neuf de l'ancien quand les deux sont valides.
    pub generation: u64,
    /// Demi-zone active : 0 ou 1.
    pub demi: u32,
    pub entrees: u32,
    /// Secteurs de contenu reellement occupes dans cette demi-zone.
    pub secteurs_contenu: u64,
    /// Somme de controle de la table. Une table dechiree est detectee au
    /// montage, pas au premier fichier illisible.
    pub somme_table: u32,
}

impl Superbloc {
    pub const fn neuf() -> Self {
        Self { generation: 0, demi: 0, entrees: 0, secteurs_contenu: 0, somme_table: 0 }
    }

    /// Ecrit le superbloc, somme de controle comprise.
    ///
    /// La somme couvre TOUT ce qui precede, y compris la magie et la version :
    /// un secteur dechire au milieu ne peut donc pas se faire passer pour un
    /// superbloc valide d'une generation plausible.
    pub fn encode(&self, secteur: &mut [u8]) -> bool {
        if secteur.len() < TAILLE_SUPERBLOC {
            return false;
        }
        for octet in secteur.iter_mut() {
            *octet = 0;
        }
        secteur[0..8].copy_from_slice(MAGIE_V2);
        secteur[8..12].copy_from_slice(&VERSION_V2.to_le_bytes());
        secteur[12..20].copy_from_slice(&self.generation.to_le_bytes());
        secteur[20..24].copy_from_slice(&self.demi.to_le_bytes());
        secteur[24..28].copy_from_slice(&self.entrees.to_le_bytes());
        secteur[28..36].copy_from_slice(&self.secteurs_contenu.to_le_bytes());
        secteur[36..40].copy_from_slice(&self.somme_table.to_le_bytes());
        let somme = somme_controle(&secteur[0..40]);
        secteur[40..44].copy_from_slice(&somme.to_le_bytes());
        true
    }

    /// Relit un superbloc. `None` des que quelque chose ne colle pas.
    pub fn decode(secteur: &[u8]) -> Option<Superbloc> {
        if secteur.len() < TAILLE_SUPERBLOC {
            return None;
        }
        if &secteur[0..8] != MAGIE_V2 {
            return None;
        }
        if u32::from_le_bytes([secteur[8], secteur[9], secteur[10], secteur[11]]) != VERSION_V2 {
            return None;
        }
        let attendue = u32::from_le_bytes([secteur[40], secteur[41], secteur[42], secteur[43]]);
        if somme_controle(&secteur[0..40]) != attendue {
            return None;
        }
        let demi = u32::from_le_bytes([secteur[20], secteur[21], secteur[22], secteur[23]]);
        if demi > 1 {
            return None;
        }
        Some(Superbloc {
            generation: u64::from_le_bytes([
                secteur[12], secteur[13], secteur[14], secteur[15],
                secteur[16], secteur[17], secteur[18], secteur[19],
            ]),
            demi,
            entrees: u32::from_le_bytes([secteur[24], secteur[25], secteur[26], secteur[27]]),
            secteurs_contenu: u64::from_le_bytes([
                secteur[28], secteur[29], secteur[30], secteur[31],
                secteur[32], secteur[33], secteur[34], secteur[35],
            ]),
            somme_table: u32::from_le_bytes([
                secteur[36], secteur[37], secteur[38], secteur[39],
            ]),
        })
    }
}

/// Lequel des deux superblocs monter.
///
/// La generation la plus HAUTE parmi les valides. Un superbloc invalide est
/// ignore, pas une erreur : c'est precisement le cas d'une coupure pendant le
/// commit, et il doit se resoudre par « l'autre », jamais par un echec de
/// montage.
///
/// Rend aussi l'emplacement (0 ou 1) : le commit suivant ecrira dans l'AUTRE,
/// pour que le superbloc courant reste intact jusqu'a ce que le nouveau soit
/// entierement ecrit.
pub fn choisit(a: Option<Superbloc>, b: Option<Superbloc>) -> Option<(usize, Superbloc)> {
    match (a, b) {
        (None, None) => None,
        (Some(a), None) => Some((0, a)),
        (None, Some(b)) => Some((1, b)),
        (Some(a), Some(b)) => {
            // Egalite impossible en pratique -- la generation monte a chaque
            // commit -- mais si elle survenait, prendre A est un choix stable,
            // et un choix stable vaut mieux qu'un choix arbitraire.
            if b.generation > a.generation { Some((1, b)) } else { Some((0, a)) }
        }
    }
}

/// Ou ecrire le prochain commit : l'autre emplacement, l'autre demi-zone.
pub const fn prochain(courant: Option<(usize, Superbloc)>) -> (usize, u32, u64) {
    match courant {
        // Zone vierge : emplacement 0, demi-zone 0, generation 1.
        None => (0, 0, 1),
        Some((emplacement, superbloc)) => (
            1 - emplacement,
            1 - superbloc.demi,
            superbloc.generation.wrapping_add(1),
        ),
    }
}

/// CRC-32 (polynome IEEE, reflechi), calcule sans table.
///
/// Sans table parce que la table couterait un kilo-octet de statique pour une
/// somme calculee quelques fois par synchronisation ; le calcul bit a bit tient
/// dans le budget de temps d'un `fsync` et ne coute aucune memoire.
pub fn somme_controle(donnees: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for octet in donnees {
        crc ^= *octet as u32;
        for _ in 0..8 {
            let bas = crc & 1;
            crc >>= 1;
            if bas != 0 {
                crc ^= 0xEDB8_8320;
            }
        }
    }
    !crc
}

// `superbloc_courant` -- qui lit reellement le disque -- vit dans `format.rs` :
// ce module reste PUR, sans ATA ni secteur, pour pouvoir etre mis a l'epreuve
// sur l'hote avec une coupure de courant injectee a chaque ecriture.
