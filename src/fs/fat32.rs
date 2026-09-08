// Ecriture d'un systeme de fichiers FAT32 : la partition que le micrologiciel
// UEFI sait lire.
//
// # Pourquoi le noyau doit savoir ECRIRE du FAT
//
// Le micrologiciel ne charge un systeme que depuis une partition FAT. Pour
// installer Bouchaud sur le disque interne, il faut donc en fabriquer une --
// et la fabriquer depuis le systeme qui tourne, parce qu'il n'y a personne
// d'autre pour le faire.
//
// Le systeme vivant ne peut pas se recopier depuis sa cle : il n'y a pas de
// pilote de stockage de masse USB, et le micrologiciel a rendu la main. Ce
// qu'il a, c'est son ARCHIVE, chargee en memoire par le chargeur d'amorcage.
// L'installation depose donc dans la nouvelle ESP des fichiers que l'archive
// transporte -- le chargeur, le noyau, sa configuration -- exactement comme un
// installateur vivant porte l'image du systeme qu'il pose.
//
// # Ce que ce module fait, et ce qu'il ne fait pas
//
// Il FORMATE et il AJOUTE. Il ne sait ni effacer, ni tronquer, ni reutiliser
// un amas rendu, et c'est deliberé : une ESP se fabrique d'un coup, et un
// systeme de fichiers a moitie general est un systeme de fichiers dont les
// chemins rares ne sont jamais exerces. Ce qui n'est pas ecrit ne peut pas
// etre faux.
//
// # Les noms longs ne sont pas une commodite
//
// Le chargeur d'amorcage cherche `kernel-x86_64` et `boot.json`. Ni l'un ni
// l'autre ne tient dans un nom 8.3 : le premier fait treize caracteres, le
// second a une extension de quatre. Ecrits en 8.3, ils deviennent `KERNEL-X`
// et `BOOT.JSO`, et le chargeur ne trouve plus rien -- une machine qui ne
// demarre pas, sans un mot d'explication.
//
// Les entrees de nom long sont donc obligatoires ici, avec leur ordre
// INVERSE -- la derniere partie du nom vient en premier -- et leur somme de
// controle calculee sur le nom court associé. Une somme fausse fait ignorer
// tout le nom long, et on retombe silencieusement sur le nom court.

#![allow(dead_code)]

use alloc::vec;
use alloc::vec::Vec;

pub use super::gpt::Support;

/// Secteurs reserves avant la premiere FAT. Trente-deux est la valeur
/// canonique de FAT32 ; elle laisse la place au secteur d'information et a la
/// copie du secteur d'amorcage.
pub const RESERVES: u32 = 32;
/// Nombre de copies de la FAT.
pub const FATS: u32 = 2;
/// Secteur de la copie du secteur d'amorcage.
pub const SECTEUR_AMORCE_SECOURS: u32 = 6;
/// Secteur d'information sur l'espace libre.
pub const SECTEUR_INFO: u32 = 1;
/// Premier amas utilisable. Zero et un sont reserves.
pub const PREMIER_AMAS: u32 = 2;
/// Marque de fin de chaine.
pub const FIN_DE_CHAINE: u32 = 0x0FFF_FFFF;
/// En dessous de ce nombre d'amas, un volume n'est PAS du FAT32.
///
/// Ce n'est pas une convention : c'est la regle qui permet a un lecteur de
/// determiner le type. Un volume de 60 000 amas formate « en FAT32 » sera lu
/// comme du FAT16 par un micrologiciel conforme, et il n'y trouvera rien.
pub const AMAS_MINIMUM_FAT32: u32 = 65_525;

const ATTR_LECTURE_SEULE: u8 = 0x01;
const ATTR_CACHE: u8 = 0x02;
const ATTR_SYSTEME: u8 = 0x04;
const ATTR_ETIQUETTE: u8 = 0x08;
const ATTR_REPERTOIRE: u8 = 0x10;
const ATTR_ARCHIVE: u8 = 0x20;
const ATTR_NOM_LONG: u8 = 0x0F;

/// Ce qui peut echouer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Erreur {
    SecteurInvalide,
    VolumeTropPetit,
    VolumeTropGrand,
    PlusDAmas,
    LectureRefusee,
    EcritureRefusee,
    RepertoirePlein,
    NomInvalide,
    CheminIntrouvable,
}

/// La geometrie d'un volume FAT32.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parametres {
    pub octets_par_secteur: u32,
    pub secteurs_par_amas: u32,
    pub secteurs_reserves: u32,
    pub fats: u32,
    pub secteurs_par_fat: u32,
    pub secteurs_total: u32,
    pub amas: u32,
    pub premier_secteur_donnees: u32,
    pub amas_racine: u32,
}

impl Parametres {
    pub const fn octets_par_amas(&self) -> u32 {
        self.octets_par_secteur * self.secteurs_par_amas
    }

    /// Le premier secteur d'un amas.
    ///
    /// L'amas deux est le PREMIER de la zone de donnees : la soustraction est
    /// ce qui evite de laisser deux amas fantomes en tete du volume.
    pub const fn secteur_de_l_amas(&self, amas: u32) -> u32 {
        self.premier_secteur_donnees + (amas - PREMIER_AMAS) * self.secteurs_par_amas
    }
}

/// Choisit le nombre de secteurs par amas.
///
/// On prend le PLUS GRAND amas qui laisse encore assez d'amas pour que le
/// volume soit du vrai FAT32. Des amas plus gros veulent dire une FAT plus
/// courte, donc moins de secteurs a ecrire au formatage et moins a relire
/// ensuite ; mais passer sous le seuil ferait lire le volume comme du FAT16.
///
/// Rend `None` quand aucun choix ne convient : le volume est alors trop petit
/// pour porter du FAT32, et le dire vaut mieux que d'en fabriquer un faux.
pub fn choisit_amas(secteurs_total: u32, octets_par_secteur: u32) -> Option<u32> {
    for candidat in [64u32, 32, 16, 8, 4, 2, 1] {
        if candidat * octets_par_secteur > 65_536 {
            // La specification plafonne un amas a 64 Kio.
            continue;
        }
        let params = calcule(secteurs_total, octets_par_secteur, candidat)?;
        if params.amas >= AMAS_MINIMUM_FAT32 {
            return Some(candidat);
        }
    }
    None
}

/// Resout la geometrie pour un nombre de secteurs par amas donne.
///
/// La taille de la FAT et le nombre d'amas se definissent l'un l'autre : la
/// FAT decrit les amas, et les amas sont ce qui reste apres la FAT. On itere
/// jusqu'au point fixe plutot que d'appliquer une formule fermee, parce que la
/// formule fermee de la specification surestime d'un secteur dans certains cas
/// et qu'un secteur de FAT en trop se paie en amas perdus, pas en panne.
pub fn calcule(
    secteurs_total: u32,
    octets_par_secteur: u32,
    secteurs_par_amas: u32,
) -> Option<Parametres> {
    if !matches!(octets_par_secteur, 512 | 1024 | 2048 | 4096) {
        return None;
    }
    if secteurs_par_amas == 0 || !secteurs_par_amas.is_power_of_two() {
        return None;
    }
    if secteurs_total <= RESERVES + 2 {
        return None;
    }
    let entrees_par_secteur = octets_par_secteur / 4;
    let mut secteurs_par_fat = 1u32;
    let mut amas = 0u32;
    for _ in 0..8 {
        let utilises = RESERVES.checked_add(secteurs_par_fat.checked_mul(FATS)?)?;
        if utilises >= secteurs_total {
            return None;
        }
        amas = (secteurs_total - utilises) / secteurs_par_amas;
        if amas == 0 {
            return None;
        }
        let voulu = (amas + PREMIER_AMAS).div_ceil(entrees_par_secteur);
        if voulu == secteurs_par_fat {
            break;
        }
        secteurs_par_fat = voulu;
    }
    if amas > 268_435_445 {
        return None;
    }
    Some(Parametres {
        octets_par_secteur,
        secteurs_par_amas,
        secteurs_reserves: RESERVES,
        fats: FATS,
        secteurs_par_fat,
        secteurs_total,
        amas,
        premier_secteur_donnees: RESERVES + secteurs_par_fat * FATS,
        amas_racine: PREMIER_AMAS,
    })
}

// ---------------------------------------------------------------------------
// Les noms
// ---------------------------------------------------------------------------

/// La somme de controle qui lie un nom long a son nom court.
///
/// Une somme fausse fait IGNORER tout le nom long, sans erreur : le fichier
/// existe alors sous son seul nom court, et le chargeur ne le trouve plus.
pub fn somme_nom_court(court: &[u8; 11]) -> u8 {
    let mut somme = 0u8;
    for octet in court.iter() {
        somme = ((somme & 1) << 7)
            .wrapping_add(somme >> 1)
            .wrapping_add(*octet);
    }
    somme
}

/// Le nom 8.3 d'un nom quelconque.
///
/// `index` distingue les alias quand plusieurs noms longs se reduisent au meme
/// debut : `KERNEL~1`, `KERNEL~2`... Sans lui, deux fichiers auraient le meme
/// nom court, et un lecteur qui ignore les noms longs n'en verrait qu'un.
pub fn nom_court(nom: &str, index: u32) -> [u8; 11] {
    let mut court = [b' '; 11];
    let (base, extension) = match nom.rfind('.') {
        Some(point) if point > 0 => (&nom[..point], &nom[point + 1..]),
        _ => (nom, ""),
    };
    let permis = |c: u8| -> u8 {
        match c {
            b'A'..=b'Z' | b'0'..=b'9' => c,
            b'a'..=b'z' => c - 32,
            b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'(' | b')' | b'-' | b'@' | b'^'
            | b'_' | b'`' | b'{' | b'}' | b'~' => c,
            _ => b'_',
        }
    };
    // Le critere est EXACTEMENT celui de `tient_en_8_3`, et non une regle
    // ecrite deux fois. Deux criteres qui divergent donnent le pire des cas :
    // un nom juge court ici recoit un alias `~1`, un nom juge court la-bas ne
    // recoit aucune entree de nom long, et le fichier existe alors sous un nom
    // que personne ne cherche. C'est ce qui est arrive a `BOOTX64.EFI`, dont
    // le point etait compte comme un caractere interdit.
    let long = !tient_en_8_3(nom);

    let suffixe = if long || index > 1 {
        let mut s = [0u8; 8];
        s[0] = b'~';
        let mut n = index.max(1);
        let mut chiffres = [0u8; 7];
        let mut taille = 0;
        while n > 0 && taille < 7 {
            chiffres[taille] = b'0' + (n % 10) as u8;
            n /= 10;
            taille += 1;
        }
        for i in 0..taille {
            s[1 + i] = chiffres[taille - 1 - i];
        }
        Some((s, 1 + taille))
    } else {
        None
    };

    let place_base = match suffixe {
        Some((_, taille)) => 8usize.saturating_sub(taille),
        None => 8,
    };
    let mut i = 0;
    for c in base.bytes() {
        if i >= place_base {
            break;
        }
        if c == b' ' || c == b'.' {
            continue;
        }
        court[i] = permis(c);
        i += 1;
    }
    if let Some((s, taille)) = suffixe {
        for j in 0..taille {
            court[i + j] = s[j];
        }
    }
    for (j, c) in extension.bytes().take(3).enumerate() {
        court[8 + j] = permis(c);
    }
    court
}

/// Le nom tient-il tel quel dans un nom 8.3 ?
///
/// Quand c'est le cas, aucune entree de nom long n'est necessaire -- et ne pas
/// en poser evite une somme de controle de plus a se tromper.
pub fn tient_en_8_3(nom: &str) -> bool {
    let (base, extension) = match nom.rfind('.') {
        Some(point) if point > 0 => (&nom[..point], &nom[point + 1..]),
        _ => (nom, ""),
    };
    if base.is_empty() || base.len() > 8 || extension.len() > 3 {
        return false;
    }
    nom.bytes().all(|c| {
        c == b'.'
            || c.is_ascii_uppercase()
            || c.is_ascii_digit()
            || matches!(c, b'_' | b'-' | b'~' | b'!' | b'#' | b'$' | b'%' | b'&')
    })
}

/// Les entrees de nom long, dans l'ordre ou elles doivent etre ECRITES.
///
/// Elles precedent l'entree courte, et la derniere partie du nom vient en
/// PREMIER. Un ordre naturel produit un nom lu a l'envers -- ce qui, la
/// plupart du temps, n'empeche rien de fonctionner et rend juste le fichier
/// introuvable par son nom.
pub fn entrees_nom_long(nom: &str, court: &[u8; 11]) -> Vec<[u8; 32]> {
    let somme = somme_nom_court(court);
    let unites: Vec<u16> = nom.encode_utf16().collect();
    let morceaux = unites.len().div_ceil(13).max(1);
    let mut sortie = Vec::with_capacity(morceaux);
    for numero in (1..=morceaux).rev() {
        let mut e = [0u8; 32];
        e[0] = numero as u8 | if numero == morceaux { 0x40 } else { 0 };
        e[11] = ATTR_NOM_LONG;
        e[13] = somme;
        // Les positions des treize caracteres dans l'entree ne sont pas
        // contigues : cinq, puis six, puis deux, autour de champs herites du
        // format court.
        const POSITIONS: [usize; 13] = [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];
        let base = (numero - 1) * 13;
        for (i, position) in POSITIONS.iter().enumerate() {
            let valeur = match unites.get(base + i) {
                Some(u) => *u,
                // Un seul terminateur nul, puis du remplissage a 0xFFFF.
                None if base + i == unites.len() => 0,
                None => 0xFFFF,
            };
            e[*position..*position + 2].copy_from_slice(&valeur.to_le_bytes());
        }
        sortie.push(e);
    }
    sortie
}

/// L'entree courte d'un fichier ou d'un repertoire.
pub fn entree_courte(court: &[u8; 11], attributs: u8, amas: u32, taille: u32) -> [u8; 32] {
    let mut e = [0u8; 32];
    e[0..11].copy_from_slice(court);
    e[11] = attributs;
    // Date et heure : une valeur fixe et VALIDE plutot que zero. Zero est un
    // mois zero et un jour zero, que certains outils signalent comme une
    // corruption. 1er janvier 2026, minuit.
    let date = ((2026 - 1980) << 9) | (1 << 5) | 1;
    e[14..16].copy_from_slice(&0u16.to_le_bytes());
    e[16..18].copy_from_slice(&(date as u16).to_le_bytes());
    e[18..20].copy_from_slice(&(date as u16).to_le_bytes());
    e[20..22].copy_from_slice(&((amas >> 16) as u16).to_le_bytes());
    e[22..24].copy_from_slice(&0u16.to_le_bytes());
    e[24..26].copy_from_slice(&(date as u16).to_le_bytes());
    e[26..28].copy_from_slice(&(amas as u16).to_le_bytes());
    e[28..32].copy_from_slice(&taille.to_le_bytes());
    e
}

// ---------------------------------------------------------------------------
// Le volume
// ---------------------------------------------------------------------------

/// Un volume FAT32 en cours d'ecriture.
///
/// `decalage` est le premier bloc de la PARTITION sur le support. Tout le
/// reste du module compte en secteurs relatifs au volume : melanger les deux
/// origines est la faute qui ecrit un systeme de fichiers parfaitement forme
/// par-dessus la table de partitions.
pub struct Volume<'a, S: Support> {
    support: &'a mut S,
    decalage: u64,
    params: Parametres,
    /// Prochain amas a essayer d'allouer. La FAT reste la verite ; ce champ
    /// evite seulement de la reparcourir depuis le debut a chaque fichier.
    curseur: u32,
    /// Amas encore libres, ou `None` quand on ne le sait pas -- un volume
    /// ouvert sans avoir ete formate ici, par exemple.
    libres: Option<u32>,
}

impl<'a, S: Support> Volume<'a, S> {
    fn lit_secteur(&mut self, secteur: u32, sortie: &mut [u8]) -> Result<(), Erreur> {
        if self.support.lit(self.decalage + secteur as u64, sortie) {
            Ok(())
        } else {
            Err(Erreur::LectureRefusee)
        }
    }

    fn ecrit_secteur(&mut self, secteur: u32, donnees: &[u8]) -> Result<(), Erreur> {
        if self.support.ecrit(self.decalage + secteur as u64, donnees) {
            Ok(())
        } else {
            Err(Erreur::EcritureRefusee)
        }
    }

    pub fn parametres(&self) -> Parametres {
        self.params
    }

    /// Lit une entree de la FAT.
    fn fat(&mut self, amas: u32) -> Result<u32, Erreur> {
        let par_secteur = self.params.octets_par_secteur / 4;
        let secteur = self.params.secteurs_reserves + amas / par_secteur;
        let dans = (amas % par_secteur) as usize * 4;
        let mut bloc = vec![0u8; self.params.octets_par_secteur as usize];
        self.lit_secteur(secteur, &mut bloc)?;
        Ok(u32::from_le_bytes([bloc[dans], bloc[dans + 1], bloc[dans + 2], bloc[dans + 3]])
            & 0x0FFF_FFFF)
    }

    /// Ecrit une entree dans TOUTES les copies de la FAT.
    ///
    /// N'en ecrire qu'une donne un volume que le systeme vivant lit sans
    /// probleme -- il lit la premiere -- et qu'un outil de reparation declare
    /// incoherent, puis « repare » en recopiant la copie perimee.
    fn pose_fat(&mut self, amas: u32, valeur: u32) -> Result<(), Erreur> {
        let par_secteur = self.params.octets_par_secteur / 4;
        let dans = (amas % par_secteur) as usize * 4;
        let mut bloc = vec![0u8; self.params.octets_par_secteur as usize];
        for copie in 0..self.params.fats {
            let secteur = self.params.secteurs_reserves
                + copie * self.params.secteurs_par_fat
                + amas / par_secteur;
            self.lit_secteur(secteur, &mut bloc)?;
            bloc[dans..dans + 4].copy_from_slice(&(valeur & 0x0FFF_FFFF).to_le_bytes());
            self.ecrit_secteur(secteur, &bloc)?;
        }
        Ok(())
    }

    /// Prend un amas libre, le marque fin de chaine, et le remet a zero.
    ///
    /// La remise a zero n'est pas une precaution d'hygiene : un amas de
    /// repertoire qui garderait d'anciens octets ferait lire des entrees
    /// fantomes, et un amas de donnees ferait fuir vers la nouvelle ESP le
    /// contenu de ce qui l'occupait avant.
    fn alloue(&mut self) -> Result<u32, Erreur> {
        let fin = self.params.amas + PREMIER_AMAS;
        let depart = self.curseur.max(PREMIER_AMAS);
        for amas in depart..fin {
            if self.fat(amas)? == 0 {
                self.pose_fat(amas, FIN_DE_CHAINE)?;
                self.curseur = amas + 1;
                self.libres = self.libres.map(|n| n.saturating_sub(1));
                self.efface_amas(amas)?;
                return Ok(amas);
            }
        }
        Err(Erreur::PlusDAmas)
    }

    fn efface_amas(&mut self, amas: u32) -> Result<(), Erreur> {
        let vide = vec![0u8; self.params.octets_par_secteur as usize];
        let debut = self.params.secteur_de_l_amas(amas);
        for i in 0..self.params.secteurs_par_amas {
            self.ecrit_secteur(debut + i, &vide)?;
        }
        Ok(())
    }

    /// Le prochain amas d'une chaine, en en allouant un si besoin.
    fn suivant(&mut self, amas: u32) -> Result<u32, Erreur> {
        let valeur = self.fat(amas)?;
        if (PREMIER_AMAS..FIN_DE_CHAINE - 7).contains(&valeur) {
            return Ok(valeur);
        }
        let neuf = self.alloue()?;
        self.pose_fat(amas, neuf)?;
        Ok(neuf)
    }

    /// Ajoute des entrees de repertoire, en etendant la chaine si besoin.
    fn ajoute_entrees(&mut self, repertoire: u32, entrees: &[[u8; 32]]) -> Result<(), Erreur> {
        let par_secteur = (self.params.octets_par_secteur / 32) as usize;
        let mut amas = repertoire;
        let mut bloc = vec![0u8; self.params.octets_par_secteur as usize];
        let mut restantes = entrees;
        // Une garde de parcours : une FAT abimee pourrait boucler, et un
        // repertoire circulaire ferait tourner l'installateur pour toujours.
        for _ in 0..self.params.amas.max(1) {
            let debut = self.params.secteur_de_l_amas(amas);
            for s in 0..self.params.secteurs_par_amas {
                self.lit_secteur(debut + s, &mut bloc)?;
                // Les entrees d'un meme nom doivent rester CONTIGUES : le nom
                // long et son entree courte forment un tout, et les separer
                // par une frontiere de secteur casse le lien.
                let mut libres = 0;
                for i in 0..par_secteur {
                    if bloc[i * 32] == 0x00 || bloc[i * 32] == 0xE5 {
                        libres += 1;
                    } else {
                        libres = 0;
                    }
                    if libres >= restantes.len() {
                        let premier = i + 1 - restantes.len();
                        for (j, entree) in restantes.iter().enumerate() {
                            bloc[(premier + j) * 32..(premier + j + 1) * 32]
                                .copy_from_slice(entree);
                        }
                        self.ecrit_secteur(debut + s, &bloc)?;
                        return Ok(());
                    }
                }
            }
            amas = self.suivant(amas)?;
            restantes = entrees;
        }
        Err(Erreur::RepertoirePlein)
    }

    /// Cherche un sous-repertoire par son nom court, et rend son amas.
    fn cherche(&mut self, repertoire: u32, court: &[u8; 11]) -> Result<Option<u32>, Erreur> {
        let par_secteur = (self.params.octets_par_secteur / 32) as usize;
        let mut amas = repertoire;
        let mut bloc = vec![0u8; self.params.octets_par_secteur as usize];
        for _ in 0..self.params.amas.max(1) {
            let debut = self.params.secteur_de_l_amas(amas);
            for s in 0..self.params.secteurs_par_amas {
                self.lit_secteur(debut + s, &mut bloc)?;
                for i in 0..par_secteur {
                    let e = &bloc[i * 32..(i + 1) * 32];
                    if e[0] == 0x00 {
                        return Ok(None);
                    }
                    if e[0] == 0xE5 || e[11] == ATTR_NOM_LONG {
                        continue;
                    }
                    if &e[0..11] == &court[..] {
                        let haut = u16::from_le_bytes([e[20], e[21]]) as u32;
                        let bas = u16::from_le_bytes([e[26], e[27]]) as u32;
                        return Ok(Some((haut << 16) | bas));
                    }
                }
            }
            let valeur = self.fat(amas)?;
            if !(PREMIER_AMAS..FIN_DE_CHAINE - 7).contains(&valeur) {
                return Ok(None);
            }
            amas = valeur;
        }
        Ok(None)
    }

    /// Cree un sous-repertoire, ou rend celui qui existe deja.
    pub fn cree_repertoire(&mut self, parent: u32, nom: &str) -> Result<u32, Erreur> {
        if nom.is_empty() || nom.len() > 255 {
            return Err(Erreur::NomInvalide);
        }
        let court = nom_court(nom, 1);
        if let Some(existant) = self.cherche(parent, &court)? {
            return Ok(existant);
        }
        let amas = self.alloue()?;
        // « . » et « .. » sont obligatoires dans un sous-repertoire. Le « .. »
        // d'un repertoire dont le parent est la RACINE porte l'amas ZERO, pas
        // l'amas de la racine : la specification le veut ainsi, et un lecteur
        // strict refuse l'autre forme.
        let mut point = [b' '; 11];
        point[0] = b'.';
        let mut deux_points = [b' '; 11];
        deux_points[0] = b'.';
        deux_points[1] = b'.';
        let parent_declare = if parent == self.params.amas_racine { 0 } else { parent };
        self.ajoute_entrees(
            amas,
            &[
                entree_courte(&point, ATTR_REPERTOIRE, amas, 0),
                entree_courte(&deux_points, ATTR_REPERTOIRE, parent_declare, 0),
            ],
        )?;

        let mut entrees = if tient_en_8_3(nom) {
            Vec::new()
        } else {
            entrees_nom_long(nom, &court)
        };
        entrees.push(entree_courte(&court, ATTR_REPERTOIRE, amas, 0));
        self.ajoute_entrees(parent, &entrees)?;
        Ok(amas)
    }

    /// Ecrit un fichier dans un repertoire.
    pub fn ecrit_fichier(
        &mut self,
        parent: u32,
        nom: &str,
        donnees: &[u8],
    ) -> Result<(), Erreur> {
        if nom.is_empty() || nom.len() > 255 {
            return Err(Erreur::NomInvalide);
        }
        if donnees.len() > u32::MAX as usize {
            return Err(Erreur::VolumeTropGrand);
        }
        let octets_amas = self.params.octets_par_amas() as usize;
        let (premier, _) = if donnees.is_empty() {
            // Un fichier vide n'a PAS d'amas : lui en donner un ferait
            // pointer une taille nulle sur un amas alloue, que rien ne
            // libererait jamais.
            (0u32, 0u32)
        } else {
            let premier = self.alloue()?;
            let mut courant = premier;
            let mut ecrits = 0usize;
            while ecrits < donnees.len() {
                let morceau = &donnees[ecrits..(ecrits + octets_amas).min(donnees.len())];
                self.ecrit_amas(courant, morceau)?;
                ecrits += morceau.len();
                if ecrits < donnees.len() {
                    let suivant = self.alloue()?;
                    self.pose_fat(courant, suivant)?;
                    courant = suivant;
                }
            }
            (premier, courant)
        };

        let court = nom_court(nom, 1);
        let mut entrees = if tient_en_8_3(nom) {
            Vec::new()
        } else {
            entrees_nom_long(nom, &court)
        };
        entrees.push(entree_courte(
            &court,
            ATTR_ARCHIVE,
            premier,
            donnees.len() as u32,
        ));
        self.ajoute_entrees(parent, &entrees)
    }

    fn ecrit_amas(&mut self, amas: u32, donnees: &[u8]) -> Result<(), Erreur> {
        let taille = self.params.octets_par_secteur as usize;
        let debut = self.params.secteur_de_l_amas(amas);
        let mut bloc = vec![0u8; taille];
        for i in 0..self.params.secteurs_par_amas as usize {
            let dans = i * taille;
            if dans >= donnees.len() {
                break;
            }
            let fin = (dans + taille).min(donnees.len());
            bloc[..fin - dans].copy_from_slice(&donnees[dans..fin]);
            // Le reste du dernier secteur est remis a zero : sans cela, la
            // queue d'un amas neuf porterait ce que l'effacement y avait mis,
            // ce qui est correct, ou ce qu'un amas reutilise y avait laisse,
            // ce qui ne l'est pas.
            bloc[fin - dans..].fill(0);
            self.ecrit_secteur(debut + i as u32, &bloc)?;
        }
        Ok(())
    }

    /// L'amas de la racine.
    pub fn racine(&self) -> u32 {
        self.params.amas_racine
    }

    /// Exige que tout soit sur le plateau.
    pub fn vidange(&mut self) -> bool {
        self.support.vidange()
    }

    /// Referme le volume : ecrit le nombre d'amas libres, puis vide le cache.
    ///
    /// Tant qu'on ne l'appelle pas, le secteur d'information annonce
    /// [`INCONNU`], ce qui est legal et ce qu'un lecteur accepte sans rien
    /// dire. C'est voulu : oublier cet appel doit donner un volume dont le
    /// compte est INCONNU, jamais un volume dont le compte est FAUX -- lequel
    /// ferait proposer a `fsck.fat` de reparer une ESP qui n'a rien.
    pub fn termine(&mut self) -> Result<(), Erreur> {
        let libres = match self.libres {
            Some(n) => n,
            None => self.compte_libres()?,
        };
        let info = secteur_info(&self.params, libres, self.curseur);
        self.ecrit_secteur(SECTEUR_INFO, &info)?;
        self.ecrit_secteur(SECTEUR_AMORCE_SECOURS + SECTEUR_INFO, &info)?;
        self.vidange();
        Ok(())
    }

    /// Parcourt la FAT et compte les amas libres.
    fn compte_libres(&mut self) -> Result<u32, Erreur> {
        let par_secteur = self.params.octets_par_secteur / 4;
        let mut bloc = vec![0u8; self.params.octets_par_secteur as usize];
        let mut libres = 0u32;
        let total = self.params.amas + PREMIER_AMAS;
        let mut amas = PREMIER_AMAS;
        while amas < total {
            let secteur = self.params.secteurs_reserves + amas / par_secteur;
            self.lit_secteur(secteur, &mut bloc)?;
            let debut = amas % par_secteur;
            for i in debut..par_secteur {
                if amas >= total {
                    break;
                }
                let d = (i as usize) * 4;
                let valeur = u32::from_le_bytes([bloc[d], bloc[d + 1], bloc[d + 2], bloc[d + 3]])
                    & 0x0FFF_FFFF;
                if valeur == 0 {
                    libres += 1;
                }
                amas += 1;
            }
        }
        Ok(libres)
    }
}

// ---------------------------------------------------------------------------
// Le formatage
// ---------------------------------------------------------------------------

/// Le secteur d'amorcage d'un volume FAT32.
pub fn secteur_amorce(params: &Parametres, numero_volume: u32, etiquette: &[u8; 11]) -> Vec<u8> {
    let mut s = vec![0u8; params.octets_par_secteur as usize];
    // Un saut valide, meme si personne ne l'execute : certains lecteurs
    // refusent un volume dont les trois premiers octets ne ressemblent pas a
    // un saut.
    s[0] = 0xEB;
    s[1] = 0x58;
    s[2] = 0x90;
    s[3..11].copy_from_slice(b"BOUCHAUD");
    s[11..13].copy_from_slice(&(params.octets_par_secteur as u16).to_le_bytes());
    s[13] = params.secteurs_par_amas as u8;
    s[14..16].copy_from_slice(&(params.secteurs_reserves as u16).to_le_bytes());
    s[16] = params.fats as u8;
    // Entrees racine et petit compte de secteurs : ZERO en FAT32. Une valeur
    // non nulle ferait lire le volume comme du FAT16.
    s[17..19].copy_from_slice(&0u16.to_le_bytes());
    s[19..21].copy_from_slice(&0u16.to_le_bytes());
    s[21] = 0xF8; // support fixe
    s[22..24].copy_from_slice(&0u16.to_le_bytes());
    s[24..26].copy_from_slice(&63u16.to_le_bytes());
    s[26..28].copy_from_slice(&255u16.to_le_bytes());
    s[28..32].copy_from_slice(&0u32.to_le_bytes());
    s[32..36].copy_from_slice(&params.secteurs_total.to_le_bytes());
    s[36..40].copy_from_slice(&params.secteurs_par_fat.to_le_bytes());
    s[40..42].copy_from_slice(&0u16.to_le_bytes());
    s[42..44].copy_from_slice(&0u16.to_le_bytes());
    s[44..48].copy_from_slice(&params.amas_racine.to_le_bytes());
    s[48..50].copy_from_slice(&(SECTEUR_INFO as u16).to_le_bytes());
    s[50..52].copy_from_slice(&(SECTEUR_AMORCE_SECOURS as u16).to_le_bytes());
    s[64] = 0x80;
    s[66] = 0x29; // signature de bloc d'amorcage etendu
    s[67..71].copy_from_slice(&numero_volume.to_le_bytes());
    s[71..82].copy_from_slice(etiquette);
    s[82..90].copy_from_slice(b"FAT32   ");
    let taille = params.octets_par_secteur as usize;
    s[taille - 2] = 0x55;
    s[taille - 1] = 0xAA;
    s
}

/// Valeur qui signifie « on n'a pas compte ».
///
/// Elle est prevue par la specification, et c'est la seule chose honnete a
/// ecrire tant qu'on n'a pas compte. Un nombre PERIME, lui, est une
/// incoherence : `fsck.fat` la signale et propose de « corriger » le volume,
/// ce qui est exactement le genre de reparation qu'on ne veut pas qu'un outil
/// tiers entreprenne sur une ESP.
pub const INCONNU: u32 = 0xFFFF_FFFF;

/// Le secteur d'information sur l'espace libre.
pub fn secteur_info(params: &Parametres, libres: u32, prochain: u32) -> Vec<u8> {
    let mut s = vec![0u8; params.octets_par_secteur as usize];
    s[0..4].copy_from_slice(&0x4161_5252u32.to_le_bytes());
    s[484..488].copy_from_slice(&0x6141_7272u32.to_le_bytes());
    s[488..492].copy_from_slice(&libres.to_le_bytes());
    s[492..496].copy_from_slice(&prochain.to_le_bytes());
    let taille = params.octets_par_secteur as usize;
    s[taille - 4..taille].copy_from_slice(&0xAA55_0000u32.to_le_bytes());
    s
}

/// Formate un volume FAT32 sur une partition, et rend un volume ouvert.
///
/// `decalage` est le premier bloc de la partition. `secteurs` sa taille.
pub fn formate<'a, S: Support>(
    support: &'a mut S,
    decalage: u64,
    secteurs: u64,
    etiquette: &str,
    numero_volume: u32,
) -> Result<Volume<'a, S>, Erreur> {
    let octets_par_secteur = support.taille_bloc() as u32;
    if secteurs > u32::MAX as u64 {
        return Err(Erreur::VolumeTropGrand);
    }
    let secteurs = secteurs as u32;
    let spc = choisit_amas(secteurs, octets_par_secteur).ok_or(Erreur::VolumeTropPetit)?;
    let params = calcule(secteurs, octets_par_secteur, spc).ok_or(Erreur::VolumeTropPetit)?;

    let mut label = [b' '; 11];
    for (i, c) in etiquette.bytes().take(11).enumerate() {
        label[i] = c.to_ascii_uppercase();
    }

    let mut volume =
        Volume { support, decalage, params, curseur: PREMIER_AMAS, libres: None };

    // La FAT d'abord, entierement remise a zero. Ne pas l'effacer laisserait
    // les chaines de l'ancien systeme de fichiers, et le premier fichier
    // ecrit atterrirait dans un amas que la FAT declare deja pris -- un
    // volume qui parait vide et dont l'espace libre est faux.
    let vide = vec![0u8; octets_par_secteur as usize];
    for copie in 0..params.fats {
        for i in 0..params.secteurs_par_fat {
            volume.ecrit_secteur(params.secteurs_reserves + copie * params.secteurs_par_fat + i, &vide)?;
        }
    }
    // Les deux entrees reservees, puis la racine en fin de chaine.
    volume.pose_fat(0, 0x0FFF_FFF8)?;
    volume.pose_fat(1, FIN_DE_CHAINE)?;
    volume.pose_fat(params.amas_racine, FIN_DE_CHAINE)?;
    volume.curseur = params.amas_racine + 1;
    volume.libres = Some(params.amas - 1);
    volume.efface_amas(params.amas_racine)?;

    let amorce = secteur_amorce(&params, numero_volume, &label);
    volume.ecrit_secteur(0, &amorce)?;
    // La copie du secteur d'amorcage n'est pas facultative : un
    // micrologiciel qui trouve le premier secteur abime la cherche la, et son
    // absence transforme une eraflure en machine qui ne demarre plus.
    volume.ecrit_secteur(SECTEUR_AMORCE_SECOURS, &amorce)?;
    // INCONNU, et non un nombre : le compte juste ne sera connu qu'a
    // `termine()`, et annoncer maintenant un nombre que les ecritures
    // suivantes rendront faux est ce que `fsck.fat` signale.
    let info = secteur_info(&params, INCONNU, params.amas_racine + 1);
    volume.ecrit_secteur(SECTEUR_INFO, &info)?;
    volume.ecrit_secteur(SECTEUR_AMORCE_SECOURS + SECTEUR_INFO, &info)?;

    // L'etiquette de volume est une entree de la racine, pas seulement un
    // champ du secteur d'amorcage. Les outils lisent l'une ou l'autre.
    let entree = entree_courte(&label, ATTR_ETIQUETTE, 0, 0);
    volume.ajoute_entrees(params.amas_racine, &[entree])?;
    volume.vidange();
    Ok(volume)
}

/// Ouvre un volume deja formate.
pub fn ouvre<'a, S: Support>(
    support: &'a mut S,
    decalage: u64,
) -> Result<Volume<'a, S>, Erreur> {
    let taille = support.taille_bloc();
    let mut s = vec![0u8; taille];
    if !support.lit(decalage, &mut s) {
        return Err(Erreur::LectureRefusee);
    }
    // CE SECTEUR VIENT DU DISQUE, ET LE DISQUE VIENT DE QUELQU'UN D'AUTRE.
    //
    // Une ESP fabriquee sur une cle qu'on branche est la voie la plus courte
    // vers ce decodeur : brancher une cle ne demande aucun privilege. Chaque
    // champ ci-dessous est donc verifie AVANT d'etre utilise pour calculer une
    // adresse, et non apres.
    //
    // Le fuzzing a montre que ce n'etait pas une precaution theorique : sans
    // ces controles, un volume a ZERO amas etait accepte, et
    // `secteur_de_l_amas(racine)` soustrayait alors deux d'un compte nul.
    let octets_par_secteur = u16::from_le_bytes([s[11], s[12]]) as u32;
    if !matches!(octets_par_secteur, 512 | 1024 | 2048 | 4096) {
        return Err(Erreur::SecteurInvalide);
    }
    let secteurs_par_amas = s[13] as u32;
    // Un amas est une puissance de deux de secteurs, plafonnee a 64 Kio. Une
    // valeur de trois ou de sept n'est pas du FAT : les adresses calculees
    // dessus ne designeraient rien.
    if secteurs_par_amas == 0
        || !secteurs_par_amas.is_power_of_two()
        || secteurs_par_amas * octets_par_secteur > 65_536
    {
        return Err(Erreur::SecteurInvalide);
    }
    let params = Parametres {
        octets_par_secteur,
        secteurs_par_amas,
        secteurs_reserves: u16::from_le_bytes([s[14], s[15]]) as u32,
        fats: s[16] as u32,
        secteurs_par_fat: u32::from_le_bytes([s[36], s[37], s[38], s[39]]),
        secteurs_total: u32::from_le_bytes([s[32], s[33], s[34], s[35]]),
        amas: 0,
        premier_secteur_donnees: 0,
        amas_racine: u32::from_le_bytes([s[44], s[45], s[46], s[47]]),
    };
    if params.secteurs_par_fat == 0 || params.fats == 0 || params.secteurs_reserves == 0 {
        return Err(Erreur::SecteurInvalide);
    }
    // La zone reservee plus les FAT peut deborder d'un `u32` sur des valeurs
    // fabriquees. Un debordement donnerait un premier secteur de donnees
    // PLUS PETIT que la zone reservee, et tout ce qui suit lirait la FAT en
    // croyant lire des donnees.
    let Some(occupe) = params
        .secteurs_par_fat
        .checked_mul(params.fats)
        .and_then(|total| total.checked_add(params.secteurs_reserves))
    else {
        return Err(Erreur::SecteurInvalide);
    };
    if occupe >= params.secteurs_total {
        return Err(Erreur::SecteurInvalide);
    }
    let premier_secteur_donnees = occupe;
    let amas = (params.secteurs_total - premier_secteur_donnees) / secteurs_par_amas;
    // UN VOLUME A ZERO AMAS N'EST PAS UN VOLUME.
    //
    // Il etait accepte, et `secteur_de_l_amas(2)` soustrayait alors deux d'un
    // compte nul. Le premier acces lisait -- ou ecrivait -- a un secteur que
    // personne n'avait choisi.
    if amas == 0 {
        return Err(Erreur::SecteurInvalide);
    }
    // La racine doit designer un amas QUI EXISTE. Les amas zero et un sont
    // reserves ; au-dela du dernier, `secteur_de_l_amas` calcule un secteur
    // hors de la partition.
    if params.amas_racine < PREMIER_AMAS
        || params.amas_racine >= amas.saturating_add(PREMIER_AMAS)
    {
        return Err(Erreur::SecteurInvalide);
    }
    let params = Parametres { amas, premier_secteur_donnees, ..params };
    Ok(Volume { support, decalage, params, curseur: PREMIER_AMAS, libres: None })
}
