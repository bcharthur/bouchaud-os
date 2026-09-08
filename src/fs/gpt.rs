//! Table de partitions GPT : ce que le micrologiciel UEFI lit pour trouver un
//! systeme.
//!
//! # Pourquoi ce fichier ne connait pas le NVMe
//!
//! Il parle a un `Support` -- lire un bloc, ecrire un bloc, combien y en
//! a-t-il. Rien d'autre. Ce n'est pas de l'abstraction gratuite : c'est ce qui
//! permet a `tools/fs/test_gpt.rs` de partitionner un disque de mille blocs
//! fait de `Vec<u8>` et de VERIFIER les octets produits, y compris les deux
//! sommes de controle, au lieu d'attendre une machine physique pour decouvrir
//! qu'un champ est a la mauvaise place.
//!
//! Une table de partitions est exactement le genre de code qu'on ne peut pas
//! mettre au point sur la cible : elle s'ecrit une fois, et si elle est
//! fausse, la machine ne demarre plus et n'a plus rien a dire.
//!
//! # Les trois pieges de GPT
//!
//! **Le MBR de protection.** Un disque GPT porte quand meme un MBR au bloc
//! zero, dont l'unique entree couvre tout le disque avec le type `0xEE`. Il
//! n'est pas decoratif : sans lui, un outil qui ne connait que le MBR voit un
//! disque VIERGE et propose de le partitionner. Sa taille est plafonnee a
//! `0xFFFFFFFF` blocs parce que le champ du MBR fait trente-deux bits -- sur
//! un disque de 500 Go en 512 octets on est en dessous, mais coder le
//! debordement ici evite d'y penser plus tard.
//!
//! **Les deux sommes de controle.** L'en-tete en porte deux : celle du tableau
//! d'entrees, et la sienne propre -- calculee AVEC SON PROPRE CHAMP MIS A
//! ZERO. Un en-tete dont la somme se calcule sur elle-meme est toujours faux,
//! et le micrologiciel bascule alors silencieusement sur la copie de secours,
//! ce qui donne un disque qui demarre une fois sur deux selon l'ordre dans
//! lequel on a ecrit.
//!
//! **La copie de secours.** Elle n'est pas une copie : `MyLBA` et
//! `AlternateLBA` y sont ECHANGES, et `PartitionEntryLBA` designe l'autre
//! tableau. Une copie octet pour octet passe le controle de somme de l'en-tete
//! -- elle a ete calculee ailleurs -- et fait pointer la secours sur le
//! tableau primaire, donc sur des blocs que l'on peut ecraser.
//!
//! # Le GUID mixte
//!
//! Un GUID GPT n'est ni petit-boutiste ni grand-boutiste : ses trois premiers
//! champs sont petit-boutistes, les deux derniers grand-boutistes. Ecrire les
//! seize octets dans l'ordre du texte donne un GUID qui a l'air juste dans un
//! editeur hexadecimal et que le micrologiciel ne reconnait pas.

#![allow(dead_code)]

use alloc::vec;
use alloc::vec::Vec;

/// Ce dont la table a besoin d'un peripherique.
pub trait Support {
    fn taille_bloc(&self) -> usize;
    fn blocs(&self) -> u64;
    fn lit(&mut self, lba: u64, sortie: &mut [u8]) -> bool;
    fn ecrit(&mut self, lba: u64, donnees: &[u8]) -> bool;
    /// Exiger que ce qui precede soit durable. Rend `false` quand le support
    /// ne sait pas le garantir -- ce que l'appelant a le droit de savoir.
    fn vidange(&mut self) -> bool {
        false
    }
}

/// Entrees du tableau. La specification en exige au moins 128.
pub const ENTREES: usize = 128;
/// Taille d'une entree, en octets.
pub const TAILLE_ENTREE: usize = 128;
/// Octets du tableau d'entrees.
pub const OCTETS_TABLEAU: usize = ENTREES * TAILLE_ENTREE;
/// Taille utile de l'en-tete.
pub const TAILLE_ENTETE: usize = 92;

/// Signature d'un en-tete GPT.
pub const SIGNATURE: &[u8; 8] = b"EFI PART";

/// Le type d'une partition systeme EFI.
pub const TYPE_ESP: Guid = Guid::depuis(
    0xC12A_7328,
    0xF81F,
    0x11D2,
    [0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9, 0x3B],
);

/// Le type d'une partition systeme Bouchaud.
///
/// Un GUID a nous plutot qu'un GUID Linux emprunte : un outil tiers qui lit ce
/// disque doit voir « je ne connais pas ce systeme », et non « ceci est un
/// systeme de fichiers Linux » -- ce qui l'autoriserait a proposer de le
/// monter, puis de le reparer.
pub const TYPE_SYSTEME_BOUCHAUD: Guid = Guid::depuis(
    0xB0DC_8A5D,
    0x0001,
    0x4B0D,
    [0x9E, 0x21, 0x42, 0x4F, 0x55, 0x43, 0x48, 0x44],
);

/// Un GUID, dans sa forme mixte GPT.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Guid {
    pub d1: u32,
    pub d2: u16,
    pub d3: u16,
    pub d4: [u8; 8],
}

impl Guid {
    pub const fn depuis(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> Self {
        Self { d1, d2, d3, d4 }
    }

    pub const fn nul() -> Self {
        Self { d1: 0, d2: 0, d3: 0, d4: [0; 8] }
    }

    pub const fn est_nul(&self) -> bool {
        self.d1 == 0 && self.d2 == 0 && self.d3 == 0
    }

    /// Les seize octets, dans l'ordre attendu par le micrologiciel.
    ///
    /// Trois champs petit-boutistes, puis huit octets tels quels. Ecrire les
    /// seize dans l'ordre du texte donne un GUID que rien ne reconnait.
    pub fn octets(&self) -> [u8; 16] {
        let mut o = [0u8; 16];
        o[0..4].copy_from_slice(&self.d1.to_le_bytes());
        o[4..6].copy_from_slice(&self.d2.to_le_bytes());
        o[6..8].copy_from_slice(&self.d3.to_le_bytes());
        o[8..16].copy_from_slice(&self.d4);
        o
    }

    pub fn depuis_octets(o: &[u8]) -> Self {
        if o.len() < 16 {
            return Self::nul();
        }
        let mut d4 = [0u8; 8];
        d4.copy_from_slice(&o[8..16]);
        Self {
            d1: u32::from_le_bytes([o[0], o[1], o[2], o[3]]),
            d2: u16::from_le_bytes([o[4], o[5]]),
            d3: u16::from_le_bytes([o[6], o[7]]),
            d4,
        }
    }
}

// ---------------------------------------------------------------------------
// CRC32
// ---------------------------------------------------------------------------

/// CRC32 IEEE, celui que GPT exige.
///
/// Table calculee a la volee plutot que stockee : elle fait un kibioctet, et
/// ce code tourne deux fois par installation.
pub fn crc32(donnees: &[u8]) -> u32 {
    let mut reste = 0xFFFF_FFFFu32;
    for octet in donnees {
        reste ^= *octet as u32;
        for _ in 0..8 {
            let bas = reste & 1;
            reste >>= 1;
            if bas != 0 {
                reste ^= 0xEDB8_8320;
            }
        }
    }
    !reste
}

// ---------------------------------------------------------------------------
// Description d'une partition
// ---------------------------------------------------------------------------

/// Ce qu'on veut poser sur le disque.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Partition {
    pub type_guid: Guid,
    pub unique: Guid,
    /// Premier bloc, inclus.
    pub premier: u64,
    /// Dernier bloc, INCLUS. GPT compte ainsi, et decrire une partition avec
    /// un dernier bloc exclusif la fait deborder d'un bloc sur la suivante.
    pub dernier: u64,
    pub attributs: u64,
    /// Nom, en caracteres ASCII ; il sera etendu en UTF-16.
    pub nom: [u8; 36],
}

impl Partition {
    pub fn neuve(type_guid: Guid, unique: Guid, premier: u64, dernier: u64, nom: &str) -> Self {
        let mut octets = [0u8; 36];
        for (i, c) in nom.bytes().take(36).enumerate() {
            octets[i] = c;
        }
        Self { type_guid, unique, premier, dernier, attributs: 0, nom: octets }
    }

    pub fn blocs(&self) -> u64 {
        self.dernier.saturating_sub(self.premier).saturating_add(1)
    }

    /// Les 128 octets de l'entree.
    pub fn encode(&self) -> [u8; TAILLE_ENTREE] {
        let mut e = [0u8; TAILLE_ENTREE];
        e[0..16].copy_from_slice(&self.type_guid.octets());
        e[16..32].copy_from_slice(&self.unique.octets());
        e[32..40].copy_from_slice(&self.premier.to_le_bytes());
        e[40..48].copy_from_slice(&self.dernier.to_le_bytes());
        e[48..56].copy_from_slice(&self.attributs.to_le_bytes());
        // Le nom est en UTF-16LE. Nos noms sont ASCII : un octet, puis zero.
        for (i, c) in self.nom.iter().enumerate() {
            if *c == 0 {
                break;
            }
            e[56 + i * 2] = *c;
        }
        e
    }

    pub fn decode(e: &[u8]) -> Option<Self> {
        if e.len() < TAILLE_ENTREE {
            return None;
        }
        let type_guid = Guid::depuis_octets(&e[0..16]);
        if type_guid.est_nul() {
            return None;
        }
        let mut nom = [0u8; 36];
        for i in 0..36 {
            let c = e[56 + i * 2];
            if c == 0 {
                break;
            }
            nom[i] = c;
        }
        Some(Self {
            type_guid,
            unique: Guid::depuis_octets(&e[16..32]),
            premier: u64::from_le_bytes(e[32..40].try_into().ok()?),
            dernier: u64::from_le_bytes(e[40..48].try_into().ok()?),
            attributs: u64::from_le_bytes(e[48..56].try_into().ok()?),
            nom,
        })
    }
}

// ---------------------------------------------------------------------------
// L'en-tete
// ---------------------------------------------------------------------------

/// Un en-tete GPT decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entete {
    pub mon_lba: u64,
    pub autre_lba: u64,
    pub premier_utilisable: u64,
    pub dernier_utilisable: u64,
    pub disque: Guid,
    pub tableau_lba: u64,
    pub entrees: u32,
    pub taille_entree: u32,
    pub crc_tableau: u32,
}

/// Ecrit un en-tete dans un bloc, somme de controle comprise.
///
/// La somme de l'en-tete se calcule sur les 92 octets utiles AVEC SON PROPRE
/// CHAMP MIS A ZERO, et sur eux seuls -- pas sur le bloc entier. Inclure le
/// remplissage donne un en-tete que le micrologiciel refuse, et il bascule
/// alors sur la copie de secours sans rien dire.
pub fn encode_entete(entete: &Entete, taille_bloc: usize) -> Vec<u8> {
    let mut bloc = vec![0u8; taille_bloc];
    bloc[0..8].copy_from_slice(SIGNATURE);
    bloc[8..12].copy_from_slice(&0x0001_0000u32.to_le_bytes());
    bloc[12..16].copy_from_slice(&(TAILLE_ENTETE as u32).to_le_bytes());
    // bloc[16..20] : la somme, laissee a zero pour le calcul.
    bloc[24..32].copy_from_slice(&entete.mon_lba.to_le_bytes());
    bloc[32..40].copy_from_slice(&entete.autre_lba.to_le_bytes());
    bloc[40..48].copy_from_slice(&entete.premier_utilisable.to_le_bytes());
    bloc[48..56].copy_from_slice(&entete.dernier_utilisable.to_le_bytes());
    bloc[56..72].copy_from_slice(&entete.disque.octets());
    bloc[72..80].copy_from_slice(&entete.tableau_lba.to_le_bytes());
    bloc[80..84].copy_from_slice(&entete.entrees.to_le_bytes());
    bloc[84..88].copy_from_slice(&entete.taille_entree.to_le_bytes());
    bloc[88..92].copy_from_slice(&entete.crc_tableau.to_le_bytes());
    let somme = crc32(&bloc[0..TAILLE_ENTETE]);
    bloc[16..20].copy_from_slice(&somme.to_le_bytes());
    bloc
}

/// Decode un en-tete, en verifiant sa signature et sa somme.
///
/// Rend `None` des que l'un des deux est faux. Rendre un en-tete « presque
/// bon » serait la pire des sorties : l'appelant ecrirait ensuite en se fiant
/// a des bornes qui ne veulent rien dire.
pub fn decode_entete(bloc: &[u8]) -> Option<Entete> {
    if bloc.len() < TAILLE_ENTETE || &bloc[0..8] != SIGNATURE {
        return None;
    }
    let taille = u32::from_le_bytes(bloc[12..16].try_into().ok()?) as usize;
    if !(TAILLE_ENTETE..=bloc.len()).contains(&taille) {
        return None;
    }
    let annoncee = u32::from_le_bytes(bloc[16..20].try_into().ok()?);
    let mut copie = bloc[..taille].to_vec();
    copie[16..20].fill(0);
    if crc32(&copie) != annoncee {
        return None;
    }
    Some(Entete {
        mon_lba: u64::from_le_bytes(bloc[24..32].try_into().ok()?),
        autre_lba: u64::from_le_bytes(bloc[32..40].try_into().ok()?),
        premier_utilisable: u64::from_le_bytes(bloc[40..48].try_into().ok()?),
        dernier_utilisable: u64::from_le_bytes(bloc[48..56].try_into().ok()?),
        disque: Guid::depuis_octets(&bloc[56..72]),
        tableau_lba: u64::from_le_bytes(bloc[72..80].try_into().ok()?),
        entrees: u32::from_le_bytes(bloc[80..84].try_into().ok()?),
        taille_entree: u32::from_le_bytes(bloc[84..88].try_into().ok()?),
        crc_tableau: u32::from_le_bytes(bloc[88..92].try_into().ok()?),
    })
}

/// Le MBR de protection du bloc zero.
///
/// Sans lui, un outil qui ne connait que le MBR voit un disque VIERGE et
/// propose de le partitionner.
pub fn mbr_protecteur(blocs_disque: u64, taille_bloc: usize) -> Vec<u8> {
    let mut mbr = vec![0u8; taille_bloc];
    let entree = 446;
    mbr[entree] = 0x00; // non amorcable
    // CHS de depart, sature : personne ne le lit sur un disque GPT, mais la
    // valeur canonique est 0x000200.
    mbr[entree + 1] = 0x00;
    mbr[entree + 2] = 0x02;
    mbr[entree + 3] = 0x00;
    mbr[entree + 4] = 0xEE; // type « GPT protective »
    mbr[entree + 5] = 0xFF;
    mbr[entree + 6] = 0xFF;
    mbr[entree + 7] = 0xFF;
    mbr[entree + 8..entree + 12].copy_from_slice(&1u32.to_le_bytes());
    // Le champ fait trente-deux bits : un disque plus grand est sature, pas
    // tronque par debordement -- ce qui donnerait une taille absurde.
    let taille = (blocs_disque.saturating_sub(1)).min(0xFFFF_FFFF) as u32;
    mbr[entree + 12..entree + 16].copy_from_slice(&taille.to_le_bytes());
    mbr[510] = 0x55;
    mbr[511] = 0xAA;
    mbr
}

// ---------------------------------------------------------------------------
// La disposition
// ---------------------------------------------------------------------------

/// Ou vivent les elements de la table, pour une geometrie donnee.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Disposition {
    pub blocs_tableau: u64,
    pub tableau_primaire: u64,
    pub tableau_secours: u64,
    pub entete_secours: u64,
    pub premier_utilisable: u64,
    pub dernier_utilisable: u64,
}

/// Calcule la disposition d'un disque.
///
/// `None` quand le disque est trop petit pour porter les deux tables : mieux
/// vaut refuser que produire une table dont les zones se recouvrent.
pub fn disposition(blocs_disque: u64, taille_bloc: usize) -> Option<Disposition> {
    if taille_bloc < 512 || blocs_disque == 0 {
        return None;
    }
    let blocs_tableau = (OCTETS_TABLEAU as u64).div_ceil(taille_bloc as u64);
    // MBR + en-tete + tableau, des deux cotes, plus au moins un bloc utile.
    let minimum = 2 + blocs_tableau * 2 + 1 + 1;
    if blocs_disque < minimum {
        return None;
    }
    let entete_secours = blocs_disque - 1;
    let tableau_secours = entete_secours - blocs_tableau;
    let premier_utilisable = 2 + blocs_tableau;
    // Le dernier bloc utilisable est celui qui PRECEDE le tableau de secours.
    let dernier_utilisable = tableau_secours - 1;
    Some(Disposition {
        blocs_tableau,
        tableau_primaire: 2,
        tableau_secours,
        entete_secours,
        premier_utilisable,
        dernier_utilisable,
    })
}

/// Aligne un bloc sur une frontiere de mebioctet.
///
/// Un SSD qui recoit des ecritures desalignees sur ses pages internes fait des
/// lecture-modification-ecriture invisibles : la partition marche, et elle est
/// durablement plus lente. Un mebioctet est la convention, et elle couvre
/// toutes les tailles de page rencontrees.
pub fn aligne_mio(bloc: u64, taille_bloc: usize) -> u64 {
    let par_mio = (1024 * 1024 / taille_bloc).max(1) as u64;
    bloc.div_ceil(par_mio) * par_mio
}

/// Le tableau d'entrees complet, remplissage compris.
pub fn encode_tableau(partitions: &[Partition], taille_bloc: usize) -> Vec<u8> {
    let blocs = (OCTETS_TABLEAU as u64).div_ceil(taille_bloc as u64) as usize;
    let mut tableau = vec![0u8; blocs * taille_bloc];
    for (i, partition) in partitions.iter().take(ENTREES).enumerate() {
        let debut = i * TAILLE_ENTREE;
        tableau[debut..debut + TAILLE_ENTREE].copy_from_slice(&partition.encode());
    }
    tableau
}

/// Ce qui a echoue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Erreur {
    DisqueTropPetit,
    TropDePartitions,
    PartitionHorsZone,
    PartitionsQuiSeChevauchent,
    EcritureRefusee,
    LectureRefusee,
    TableAbsente,
    SecoursIncoherente,
}

/// Ecrit une table GPT complete sur le support.
///
/// # L'ordre des ecritures
///
/// Le MBR de protection part EN DERNIER. Tant qu'il n'est pas la, un outil
/// tiers voit un disque vierge -- ce qui est vrai tant que la table n'est pas
/// finie. L'ecrire en premier annoncerait une table GPT pendant les quelques
/// millisecondes ou elle n'existe pas encore, et une coupure a cet instant
/// laisserait un disque qui se declare partitionne sans l'etre.
pub fn ecrit_table<S: Support>(
    support: &mut S,
    disque: Guid,
    partitions: &[Partition],
) -> Result<Disposition, Erreur> {
    let taille_bloc = support.taille_bloc();
    let blocs_disque = support.blocs();
    let plan = disposition(blocs_disque, taille_bloc).ok_or(Erreur::DisqueTropPetit)?;
    if partitions.len() > ENTREES {
        return Err(Erreur::TropDePartitions);
    }
    for (i, p) in partitions.iter().enumerate() {
        if p.premier < plan.premier_utilisable
            || p.dernier > plan.dernier_utilisable
            || p.premier > p.dernier
        {
            return Err(Erreur::PartitionHorsZone);
        }
        for autre in &partitions[i + 1..] {
            if p.premier <= autre.dernier && autre.premier <= p.dernier {
                return Err(Erreur::PartitionsQuiSeChevauchent);
            }
        }
    }

    let tableau = encode_tableau(partitions, taille_bloc);
    let crc_tableau = crc32(&tableau[..OCTETS_TABLEAU]);

    let primaire = Entete {
        mon_lba: 1,
        autre_lba: plan.entete_secours,
        premier_utilisable: plan.premier_utilisable,
        dernier_utilisable: plan.dernier_utilisable,
        disque,
        tableau_lba: plan.tableau_primaire,
        entrees: ENTREES as u32,
        taille_entree: TAILLE_ENTREE as u32,
        crc_tableau,
    };
    // La secours n'est PAS une copie : les deux LBA sont echanges et le
    // tableau designe est l'autre. Une copie octet pour octet ferait pointer
    // la secours sur le tableau primaire, donc sur des blocs ecrasables.
    let secours = Entete {
        mon_lba: plan.entete_secours,
        autre_lba: 1,
        tableau_lba: plan.tableau_secours,
        ..primaire
    };

    ecrit_zone(support, plan.tableau_primaire, &tableau, taille_bloc)?;
    ecrit_zone(support, plan.tableau_secours, &tableau, taille_bloc)?;
    if !support.ecrit(plan.entete_secours, &encode_entete(&secours, taille_bloc)) {
        return Err(Erreur::EcritureRefusee);
    }
    if !support.ecrit(1, &encode_entete(&primaire, taille_bloc)) {
        return Err(Erreur::EcritureRefusee);
    }
    if !support.ecrit(0, &mbr_protecteur(blocs_disque, taille_bloc)) {
        return Err(Erreur::EcritureRefusee);
    }
    support.vidange();
    Ok(plan)
}

fn ecrit_zone<S: Support>(
    support: &mut S,
    lba: u64,
    donnees: &[u8],
    taille_bloc: usize,
) -> Result<(), Erreur> {
    for (i, morceau) in donnees.chunks(taille_bloc).enumerate() {
        if !support.ecrit(lba + i as u64, morceau) {
            return Err(Erreur::EcritureRefusee);
        }
    }
    Ok(())
}

/// Relit la table et rend les partitions qu'elle decrit.
///
/// Bascule sur la copie de secours quand l'en-tete primaire est illisible --
/// c'est exactement ce pour quoi elle existe -- et le DIT dans le second
/// membre du couple, parce qu'un disque qui ne demarre que par sa secours est
/// un disque abime dont il faut savoir qu'il l'est.
pub fn lit_table<S: Support>(support: &mut S) -> Result<(Vec<Partition>, bool), Erreur> {
    let taille_bloc = support.taille_bloc();
    let blocs_disque = support.blocs();
    let mut bloc = vec![0u8; taille_bloc];

    let mut degradee = false;
    let entete = {
        if support.lit(1, &mut bloc) {
            decode_entete(&bloc)
        } else {
            None
        }
    };
    let entete = match entete {
        Some(e) => e,
        None => {
            degradee = true;
            if blocs_disque == 0 || !support.lit(blocs_disque - 1, &mut bloc) {
                return Err(Erreur::LectureRefusee);
            }
            decode_entete(&bloc).ok_or(Erreur::TableAbsente)?
        }
    };

    if entete.taille_entree as usize != TAILLE_ENTREE || entete.entrees == 0 {
        return Err(Erreur::SecoursIncoherente);
    }
    let octets = entete.entrees as usize * TAILLE_ENTREE;
    let blocs = (octets as u64).div_ceil(taille_bloc as u64);
    let mut tableau = vec![0u8; (blocs as usize) * taille_bloc];
    for i in 0..blocs {
        let debut = (i as usize) * taille_bloc;
        if !support.lit(entete.tableau_lba + i, &mut tableau[debut..debut + taille_bloc]) {
            return Err(Erreur::LectureRefusee);
        }
    }
    if crc32(&tableau[..octets]) != entete.crc_tableau {
        return Err(Erreur::TableAbsente);
    }

    let mut sortie = Vec::new();
    for i in 0..entete.entrees as usize {
        let debut = i * TAILLE_ENTREE;
        if let Some(p) = Partition::decode(&tableau[debut..debut + TAILLE_ENTREE]) {
            sortie.push(p);
        }
    }
    Ok((sortie, degradee))
}

// ---------------------------------------------------------------------------
// GUID derives d'une graine
// ---------------------------------------------------------------------------

/// Melange deterministe, pour fabriquer des GUID a partir d'une graine.
///
/// Deterministe VOLONTAIREMENT : l'installateur fournit une graine tiree de
/// l'entropie du noyau, et le test hote fournit une graine fixe et compare des
/// octets. Un generateur qui appellerait le noyau directement rendrait tout ce
/// fichier intestable.
pub fn melange(etat: &mut u64) -> u64 {
    *etat = etat.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *etat;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Un GUID version 4 derive d'une graine.
///
/// Les bits de version et de variante sont poses : un GUID qui ne les porte
/// pas reste unique, et certains outils le refusent comme malforme.
pub fn guid_aleatoire(etat: &mut u64) -> Guid {
    let a = melange(etat);
    let b = melange(etat);
    let mut d4 = [0u8; 8];
    d4.copy_from_slice(&b.to_be_bytes());
    let d3 = ((a >> 32) as u16 & 0x0FFF) | 0x4000;
    d4[0] = (d4[0] & 0x3F) | 0x80;
    Guid { d1: a as u32, d2: (a >> 48) as u16, d3, d4 }
}
