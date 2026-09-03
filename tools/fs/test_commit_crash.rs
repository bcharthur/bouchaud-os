//! Preuve hote du commit crash-safe de la persistance.
//!
//! Le module de production `src/fs/persistance/superbloc.rs` est inclus tel
//! quel : ce qui est mis a l'epreuve ici est le code qui decide, au montage,
//! quel etat du disque est le bon.
//!
//! # Ce que le format V1 ne pouvait pas promettre
//!
//! Il ecrivait le contenu, puis la table, puis l'en-tete. L'ordre etait le bon,
//! et il ne suffisait pas -- pour une raison qui n'a rien a voir avec l'ordre :
//!
//!   * l'en-tete vivait a un secteur FIXE. L'ecrire ecrasait le precedent ;
//!   * le contenu vivait aux memes secteurs d'une synchronisation a l'autre.
//!
//! Une coupure pendant l'ecriture du contenu laissait donc l'ANCIEN en-tete --
//! valide, magie correcte, nombre d'entrees correct -- pointant vers un contenu
//! a moitie neuf. Ce melange etait monte comme s'il etait coherent.
//!
//! # L'invariant que ces tests etablissent
//!
//! Une coupure a N'IMPORTE QUEL point d'une synchronisation laisse le disque
//! dans l'un de deux etats, et jamais dans un troisieme : soit l'ancien etat
//! complet, soit le nouveau etat complet.
//!
//! La verification est EXHAUSTIVE : la coupure est injectee apres chacune des
//! ecritures de la synchronisation, une par une, et l'etat monte est compare
//! aux deux seuls resultats acceptables.

#![allow(dead_code)]

#[path = "../../src/fs/persistance/superbloc.rs"]
mod superbloc;

use superbloc::{
    choisit, prochain, somme_controle, Superbloc, MAGIE_V2, TAILLE_SUPERBLOC, VERSION_V2,
};

const SECTEUR: usize = 512;

// ---------------------------------------------------------------------------
// Un disque simule, et une coupure de courant qui tombe ou l'on veut.
// ---------------------------------------------------------------------------

struct Disque {
    secteurs: Vec<[u8; SECTEUR]>,
    /// Ecritures restantes avant la coupure. `None` = pas de coupure.
    avant_coupure: Option<usize>,
    ecritures: usize,
}

impl Disque {
    fn neuf(nombre: usize) -> Self {
        Self { secteurs: vec![[0u8; SECTEUR]; nombre], avant_coupure: None, ecritures: 0 }
    }

    fn coupe_apres(&mut self, ecritures: usize) {
        self.avant_coupure = Some(ecritures);
        self.ecritures = 0;
    }

    /// Rend `false` quand le courant est coupe : l'appelant s'arrete, comme le
    /// ferait une machine.
    fn ecris(&mut self, secteur: usize, donnees: &[u8]) -> bool {
        if let Some(limite) = self.avant_coupure {
            if self.ecritures >= limite {
                return false;
            }
        }
        self.ecritures += 1;
        let n = donnees.len().min(SECTEUR);
        self.secteurs[secteur][..n].copy_from_slice(&donnees[..n]);
        true
    }

    /// Une ecriture DECHIREE : le secteur part a moitie.
    ///
    /// C'est le cas que la somme de controle existe pour attraper, et le seul
    /// que « ecrire un secteur est atomique » ne couvre pas sur tout materiel.
    fn ecris_dechire(&mut self, secteur: usize, donnees: &[u8], octets: usize) {
        let n = octets.min(donnees.len()).min(SECTEUR);
        self.secteurs[secteur][..n].copy_from_slice(&donnees[..n]);
    }

    fn lis(&self, secteur: usize) -> &[u8; SECTEUR] {
        &self.secteurs[secteur]
    }
}

// ---------------------------------------------------------------------------
// La synchronisation, reduite a sa sequence d'ecritures.
// ---------------------------------------------------------------------------

/// Geometrie simulee : deux superblocs, puis deux demi-zones de quatre
/// secteurs (un de table, trois de contenu).
const SUPERBLOCS: usize = 2;
const TABLE: usize = 1;
const CONTENU: usize = 3;
const DEMI: usize = TABLE + CONTENU;

fn debut_demi(demi: u32) -> usize { SUPERBLOCS + demi as usize * DEMI }
fn contenu_demi(demi: u32) -> usize { debut_demi(demi) + TABLE }

/// Ce qu'un etat committe contient : une table et son contenu.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Etat {
    generation: u64,
    table: [u8; SECTEUR],
    contenu: Vec<[u8; SECTEUR]>,
}

fn superbloc_lu(disque: &Disque, emplacement: usize) -> Option<Superbloc> {
    Superbloc::decode(disque.lis(emplacement))
}

/// Le montage : lire les deux superblocs, prendre la generation la plus haute
/// parmi les valides, verifier la somme de la table, rendre l'etat.
fn monte(disque: &Disque) -> Option<Etat> {
    let (_, superbloc) = choisit(superbloc_lu(disque, 0), superbloc_lu(disque, 1))?;
    let table = *disque.lis(debut_demi(superbloc.demi));
    if somme_controle(&table) != superbloc.somme_table {
        return None;
    }
    let mut contenu = Vec::new();
    for index in 0..superbloc.secteurs_contenu as usize {
        contenu.push(*disque.lis(contenu_demi(superbloc.demi) + index));
    }
    Some(Etat { generation: superbloc.generation, table, contenu })
}

/// Une synchronisation complete : contenu, table, puis LE COMMIT.
///
/// Rend `false` si le courant a ete coupe en route.
fn synchronise(disque: &mut Disque, marque: u8) -> bool {
    let courant = choisit(superbloc_lu(disque, 0), superbloc_lu(disque, 1));
    let (emplacement, demi, generation) = prochain(courant);

    // 1. Le contenu, dans la demi-zone INACTIVE.
    for index in 0..CONTENU {
        let mut secteur = [marque; SECTEUR];
        secteur[0] = marque;
        secteur[1] = index as u8;
        if !disque.ecris(contenu_demi(demi) + index, &secteur) {
            return false;
        }
    }

    // 2. La table.
    let mut table = [0u8; SECTEUR];
    table[0] = marque;
    table[1] = CONTENU as u8;
    if !disque.ecris(debut_demi(demi), &table) {
        return false;
    }

    // 3. LE COMMIT : un seul secteur.
    let superbloc = Superbloc {
        generation,
        demi,
        entrees: 1,
        secteurs_contenu: CONTENU as u64,
        somme_table: somme_controle(&table),
    };
    let mut secteur = [0u8; SECTEUR];
    superbloc.encode(&mut secteur);
    disque.ecris(emplacement, &secteur)
}

fn disque_neuf() -> Disque {
    Disque::neuf(SUPERBLOCS + 2 * DEMI)
}

// ---------------------------------------------------------------------------
// Le format lui-meme.
// ---------------------------------------------------------------------------

#[test]
fn un_superbloc_fait_un_aller_retour() {
    let superbloc = Superbloc {
        generation: 0x0123_4567_89AB_CDEF,
        demi: 1,
        entrees: 1234,
        secteurs_contenu: 4096,
        somme_table: 0xDEAD_BEEF,
    };
    let mut secteur = [0u8; SECTEUR];
    assert!(superbloc.encode(&mut secteur));
    assert_eq!(&secteur[0..8], MAGIE_V2);
    assert_eq!(Superbloc::decode(&secteur), Some(superbloc));
}

/// Un secteur vierge n'est pas un superbloc : un disque neuf doit se lire
/// comme vierge, pas comme une generation zero valide.
#[test]
fn un_secteur_vierge_n_est_pas_un_superbloc() {
    assert_eq!(Superbloc::decode(&[0u8; SECTEUR]), None);
    assert_eq!(Superbloc::decode(&[0xFFu8; SECTEUR]), None);
    assert_eq!(Superbloc::decode(&[]), None);
}

/// LE CAS QUE LA SOMME DE CONTROLE EXISTE POUR ATTRAPER : un secteur ecrit a
/// moitie. Sans elle, la moitie neuve pourrait se faire passer pour un
/// superbloc valide d'une generation plausible.
#[test]
fn un_superbloc_dechire_est_rejete() {
    let superbloc = Superbloc {
        generation: 42, demi: 0, entrees: 3,
        secteurs_contenu: 7, somme_table: 0x1234,
    };
    let mut complet = [0u8; SECTEUR];
    superbloc.encode(&mut complet);

    for octets in 1..TAILLE_SUPERBLOC {
        let mut disque = disque_neuf();
        disque.ecris_dechire(0, &complet, octets);
        assert_eq!(
            superbloc_lu(&disque, 0), None,
            "un superbloc coupe a {octets} octets doit etre REJETE"
        );
    }
    // Complet, il passe.
    let mut disque = disque_neuf();
    disque.ecris_dechire(0, &complet, SECTEUR);
    assert_eq!(superbloc_lu(&disque, 0), Some(superbloc));
}

/// N'importe quel bit retourne doit etre vu.
#[test]
fn un_bit_retourne_invalide_le_superbloc() {
    let superbloc = Superbloc {
        generation: 9, demi: 1, entrees: 1, secteurs_contenu: 2, somme_table: 5,
    };
    let mut secteur = [0u8; SECTEUR];
    superbloc.encode(&mut secteur);
    for octet in 0..TAILLE_SUPERBLOC {
        for bit in 0..8 {
            let mut abime = secteur;
            abime[octet] ^= 1 << bit;
            assert_eq!(
                Superbloc::decode(&abime), None,
                "octet {octet} bit {bit} : la corruption doit etre vue"
            );
        }
    }
}

#[test]
fn la_generation_la_plus_haute_gagne() {
    let ancien = Superbloc { generation: 7, demi: 0, entrees: 1, secteurs_contenu: 1, somme_table: 0 };
    let neuf = Superbloc { generation: 8, demi: 1, entrees: 2, secteurs_contenu: 2, somme_table: 0 };
    assert_eq!(choisit(Some(ancien), Some(neuf)), Some((1, neuf)));
    assert_eq!(choisit(Some(neuf), Some(ancien)), Some((0, neuf)));
    assert_eq!(choisit(Some(ancien), None), Some((0, ancien)));
    assert_eq!(choisit(None, Some(neuf)), Some((1, neuf)));
    assert_eq!(choisit(None, None), None);
}

/// Le commit suivant ecrit dans l'AUTRE emplacement et l'AUTRE demi-zone.
/// Sans cela, il ecraserait l'etat dont on depend encore.
#[test]
fn le_commit_suivant_alterne_toujours() {
    assert_eq!(prochain(None), (0, 0, 1), "zone vierge : emplacement 0, demi 0");
    let a = Superbloc { generation: 1, demi: 0, entrees: 1, secteurs_contenu: 1, somme_table: 0 };
    assert_eq!(prochain(Some((0, a))), (1, 1, 2));
    let b = Superbloc { generation: 2, demi: 1, entrees: 1, secteurs_contenu: 1, somme_table: 0 };
    assert_eq!(prochain(Some((1, b))), (0, 0, 3));
}

// ---------------------------------------------------------------------------
// LA COUPURE DE COURANT, INJECTEE PARTOUT.
// ---------------------------------------------------------------------------

/// L'INVARIANT DU CHANTIER 5, verifie EXHAUSTIVEMENT.
///
/// Le disque porte un premier etat committe. Une seconde synchronisation est
/// interrompue apres chacune de ses ecritures, une par une. A chaque fois,
/// l'etat monte doit etre l'ANCIEN ou le NOUVEAU -- jamais un troisieme.
#[test]
fn une_coupure_a_n_importe_quel_point_ne_laisse_jamais_un_etat_hybride() {
    // L'etat de reference.
    let mut reference = disque_neuf();
    assert!(synchronise(&mut reference, 0xA1));
    let ancien = monte(&reference).expect("le premier etat doit se monter");
    assert_eq!(ancien.generation, 1);

    // L'etat complet visé par la seconde synchronisation.
    let mut complet = disque_neuf();
    synchronise(&mut complet, 0xA1);
    assert!(synchronise(&mut complet, 0xB2));
    let nouveau = monte(&complet).expect("le second etat doit se monter");
    assert_eq!(nouveau.generation, 2);
    assert_ne!(ancien, nouveau, "les deux etats doivent etre distinguables");

    // Toutes les coupures possibles.
    let ecritures_totales = CONTENU + TABLE + 1;
    for coupure in 0..=ecritures_totales {
        let mut disque = disque_neuf();
        synchronise(&mut disque, 0xA1);
        disque.coupe_apres(coupure);
        let termine = synchronise(&mut disque, 0xB2);

        let monte = monte(&disque).expect("le disque doit toujours se monter");
        assert!(
            monte == ancien || monte == nouveau,
            "coupure apres {coupure} ecriture(s) : etat HYBRIDE monte comme \
             valide -- generation={} (ancien={}, nouveau={})",
            monte.generation, ancien.generation, nouveau.generation
        );
        if termine {
            assert_eq!(monte, nouveau, "synchronisation terminee : le nouvel etat");
        } else {
            assert_eq!(
                monte, ancien,
                "coupure apres {coupure} : tant que le commit n'a pas eu lieu, \
                 c'est l'ANCIEN etat qui vaut, entierement"
            );
        }
    }
}

/// Le commit lui-meme peut etre DECHIRE. L'ancien superbloc reste alors le bon.
#[test]
fn un_commit_dechire_laisse_l_ancien_etat() {
    let mut reference = disque_neuf();
    synchronise(&mut reference, 0xA1);
    let ancien = monte(&reference).expect("premier etat");

    for octets in 1..TAILLE_SUPERBLOC {
        let mut disque = disque_neuf();
        synchronise(&mut disque, 0xA1);

        // Toute la synchronisation, sauf que le commit part a moitie.
        let courant = choisit(superbloc_lu(&disque, 0), superbloc_lu(&disque, 1));
        let (emplacement, demi, generation) = prochain(courant);
        for index in 0..CONTENU {
            let mut secteur = [0xB2u8; SECTEUR];
            secteur[1] = index as u8;
            disque.ecris(contenu_demi(demi) + index, &secteur);
        }
        let mut table = [0u8; SECTEUR];
        table[0] = 0xB2;
        disque.ecris(debut_demi(demi), &table);

        let superbloc = Superbloc {
            generation, demi, entrees: 1,
            secteurs_contenu: CONTENU as u64,
            somme_table: somme_controle(&table),
        };
        let mut secteur = [0u8; SECTEUR];
        superbloc.encode(&mut secteur);
        disque.ecris_dechire(emplacement, &secteur, octets);

        assert_eq!(
            monte(&disque), Some(ancien.clone()),
            "commit coupe a {octets} octets : l'ancien etat doit rester le bon"
        );
    }
}

/// Une TABLE incoherente avec son superbloc doit etre vue au montage, pas
/// plusieurs minutes plus tard sous la forme d'un fichier illisible au hasard.
#[test]
fn une_table_incoherente_est_vue_au_montage() {
    let mut disque = disque_neuf();
    synchronise(&mut disque, 0xA1);
    assert!(monte(&disque).is_some());

    // Un octet de la table change apres coup.
    let superbloc = choisit(superbloc_lu(&disque, 0), superbloc_lu(&disque, 1)).unwrap().1;
    disque.secteurs[debut_demi(superbloc.demi)][3] ^= 0xFF;
    assert_eq!(
        monte(&disque), None,
        "une table qui ne correspond plus a sa somme doit faire refuser la zone"
    );
}

/// Deux synchronisations d'affilee doivent alterner : la seconde ne doit PAS
/// ecrire par-dessus le contenu dont la premiere depend.
#[test]
fn deux_synchronisations_n_ecrasent_pas_l_etat_committe() {
    let mut disque = disque_neuf();
    synchronise(&mut disque, 0xA1);
    let premier = choisit(superbloc_lu(&disque, 0), superbloc_lu(&disque, 1)).unwrap();
    let contenu_premier: Vec<[u8; SECTEUR]> = (0..CONTENU)
        .map(|i| *disque.lis(contenu_demi(premier.1.demi) + i))
        .collect();

    synchronise(&mut disque, 0xB2);
    let apres: Vec<[u8; SECTEUR]> = (0..CONTENU)
        .map(|i| *disque.lis(contenu_demi(premier.1.demi) + i))
        .collect();
    assert_eq!(
        contenu_premier, apres,
        "la demi-zone du premier etat ne doit pas avoir ete touchee"
    );
}

/// Un long cycle : dix synchronisations, chacune coupee, puis reprise. Le
/// disque doit rester montable a chaque etape.
#[test]
fn le_disque_reste_montable_a_travers_des_coupures_repetees() {
    let mut disque = disque_neuf();
    // Un etat committe d'abord : un disque vierge n'a legitimement rien a
    // monter, et l'invariant porte sur ce qui SUIT un commit.
    assert!(synchronise(&mut disque, 0xA0));
    let mut derniere_generation = monte(&disque).expect("premier etat").generation;
    for tour in 0..10u8 {
        // Une coupure au milieu, puis une reprise complete.
        disque.coupe_apres(tour as usize % (CONTENU + TABLE + 1));
        let _ = synchronise(&mut disque, 0xC0 + tour);
        disque.avant_coupure = None;

        let etat = monte(&disque).expect("le disque doit rester montable");
        assert!(
            etat.generation >= derniere_generation,
            "la generation ne doit jamais RECULER : {} < {}",
            etat.generation, derniere_generation
        );

        assert!(synchronise(&mut disque, 0xD0 + tour));
        let etat = monte(&disque).expect("montable apres la reprise");
        assert!(etat.generation > derniere_generation);
        derniere_generation = etat.generation;
    }
}

// ---------------------------------------------------------------------------
// LA MIGRATION V1 -> V2 : la premiere ecriture ne doit pas detruire la V1.
// ---------------------------------------------------------------------------
//
// Sur un disque V1, `superbloc_courant` rend `None`, donc `prochain(None)`
// choisit la demi-zone 0. Avec la geometrie reelle, elle commence au secteur 2
// et recouvre la table V1 (1..1024) DES sa premiere ecriture -- alors que
// l'en-tete V1, au secteur 0, dit toujours « valide ».
//
// Une coupure a ce moment laissait exactement l'etat que ce format existe pour
// interdire : une magie valide qui designe une table dechiquetee. Le repli V1
// montait des donnees corrompues.

/// La geometrie REELLE de `persistance.rs`, reproduite ici.
///
/// Les valeurs sont recopiees et non importees : le module de production touche
/// ATA. Le premier test verifie que le RECOUVREMENT est bien celui qu'on croit
/// -- si la geometrie bouge, c'est le raisonnement qu'il faut refaire, pas la
/// constante qu'il faut mettre a jour en silence.
mod migration {
    pub const SECTEURS_TABLE: u64 = 1024;
    pub const SECTEUR_CONTENU: u64 = 1 + SECTEURS_TABLE;
    pub const SECTEURS_ZONE: u64 = 262144;
    pub const SUPERBLOCS: u64 = 2;
    pub const DEMI: u64 = (SECTEURS_ZONE - SUPERBLOCS) / 2;

    pub const fn debut_demi(demi: u32) -> u64 {
        SUPERBLOCS + demi as u64 * DEMI
    }

    /// La regle de `demi_sure`, sans le disque.
    pub fn demi_sure(demi_prevue: u32, fin_v1: u64) -> Option<u32> {
        let recouvre = |demi: u32| fin_v1 != 0 && debut_demi(demi) <= fin_v1;
        if !recouvre(demi_prevue) {
            return Some(demi_prevue);
        }
        let autre = 1 - demi_prevue;
        if !recouvre(autre) {
            return Some(autre);
        }
        None
    }
}

/// LE DEFAUT, reproduit : la demi-zone 0 recouvre la table V1.
#[test]
fn la_demi_zone_zero_recouvre_la_table_v1() {
    use migration::*;
    assert!(
        debut_demi(0) <= SECTEURS_TABLE,
        "la demi-zone 0 commence a {} et la table V1 finit a {} : elles se \
         recouvrent, et c'est le defaut que ce test verrouille",
        debut_demi(0), SECTEURS_TABLE
    );
    assert!(
        debut_demi(1) > SECTEUR_CONTENU,
        "la demi-zone 1, elle, commence au-dela de la table V1"
    );
}

/// Un disque V1 dont le contenu tient dans une demi-zone migre vers celle qui
/// ne le recouvre PAS.
#[test]
fn la_migration_choisit_la_demi_zone_qui_ne_recouvre_pas_la_v1() {
    use migration::*;
    let fin_v1 = SECTEUR_CONTENU + 8192;
    assert_eq!(
        demi_sure(0, fin_v1), Some(1),
        "la demi-zone 0 prevue recouvre la V1 : la migration doit basculer sur \
         la 1, qui commence a {}",
        debut_demi(1)
    );
}

/// Un disque deja en V2 n'a aucune V1 a preserver : l'alternance A/B du format
/// reste intacte.
#[test]
fn sans_v1_vivante_l_alternance_reste_intacte() {
    use migration::*;
    assert_eq!(demi_sure(0, 0), Some(0));
    assert_eq!(demi_sure(1, 0), Some(1));
}

/// Une V1 qui deborde des DEUX demi-zones : la migration est refusee AVANT
/// d'avoir rien detruit.
///
/// Ce n'est pas un nouveau mode d'echec : un tel contenu ne tiendrait pas dans
/// une demi-zone de toute facon, et la synchronisation echouerait plus loin. La
/// difference est qu'elle echoue avant d'avoir ecrase la V1.
#[test]
fn une_v1_trop_grande_refuse_la_migration_avant_de_detruire() {
    use migration::*;
    let fin_v1 = debut_demi(1) + 1;
    assert_eq!(demi_sure(0, fin_v1), None);
    assert_eq!(demi_sure(1, fin_v1), None);
}

// ---------------------------------------------------------------------------
// La somme de controle elle-meme.
// ---------------------------------------------------------------------------

/// Le CRC-32 doit etre le vrai : une somme maison qui rate un cas est pire
/// qu'aucune somme, parce qu'on lui fait confiance.
#[test]
fn le_crc32_est_conforme() {
    assert_eq!(somme_controle(b""), 0x0000_0000);
    assert_eq!(somme_controle(b"a"), 0xE8B7_BE43);
    assert_eq!(somme_controle(b"123456789"), 0xCBF4_3926);
    assert_eq!(somme_controle(b"The quick brown fox jumps over the lazy dog"), 0x414F_A339);
}

#[test]
fn la_version_est_epinglee() {
    assert_eq!(VERSION_V2, 2);
    assert_eq!(MAGIE_V2, b"BOPERSI2");
    assert_ne!(MAGIE_V2, b"BOPERSI1", "les deux formats doivent se distinguer");
}
