//! Preuve hote de la table de partitions GPT.
//!
//! Une table de partitions est exactement le genre de code qu'on ne peut pas
//! mettre au point sur la cible : elle s'ecrit une fois, et si elle est
//! fausse, la machine ne demarre plus et n'a plus rien a dire. Le module de
//! production ne parle qu'a un `Support` -- lire un bloc, ecrire un bloc --,
//! ce qui permet de lui donner ici un disque fait de `Vec<u8>` et de verifier
//! les octets produits, sommes de controle comprises.

#![allow(dead_code)]

extern crate alloc;

#[path = "../../src/fs/gpt.rs"]
mod gpt;

use gpt::*;

/// Un disque en memoire.
struct Disque {
    taille_bloc: usize,
    octets: Vec<u8>,
    vidanges: usize,
    /// Blocs dont l'ecriture doit echouer, pour eprouver les chemins d'erreur.
    refuse: Vec<u64>,
}

impl Disque {
    fn neuf(blocs: u64, taille_bloc: usize) -> Self {
        Self {
            taille_bloc,
            octets: vec![0u8; (blocs as usize) * taille_bloc],
            vidanges: 0,
            refuse: Vec::new(),
        }
    }
    fn bloc(&self, lba: u64) -> &[u8] {
        let d = (lba as usize) * self.taille_bloc;
        &self.octets[d..d + self.taille_bloc]
    }
}

impl Support for Disque {
    fn taille_bloc(&self) -> usize {
        self.taille_bloc
    }
    fn blocs(&self) -> u64 {
        (self.octets.len() / self.taille_bloc) as u64
    }
    fn lit(&mut self, lba: u64, sortie: &mut [u8]) -> bool {
        let d = (lba as usize) * self.taille_bloc;
        if d + sortie.len() > self.octets.len() {
            return false;
        }
        sortie.copy_from_slice(&self.octets[d..d + sortie.len()]);
        true
    }
    fn ecrit(&mut self, lba: u64, donnees: &[u8]) -> bool {
        if self.refuse.contains(&lba) {
            return false;
        }
        let d = (lba as usize) * self.taille_bloc;
        if d + donnees.len() > self.octets.len() {
            return false;
        }
        self.octets[d..d + donnees.len()].copy_from_slice(donnees);
        true
    }
    fn vidange(&mut self) -> bool {
        self.vidanges += 1;
        true
    }
}

fn disque_partitionne(blocs: u64, taille_bloc: usize) -> (Disque, Disposition, Vec<Partition>) {
    let mut disque = Disque::neuf(blocs, taille_bloc);
    let plan = disposition(blocs, taille_bloc).expect("disque assez grand");
    let esp_debut = aligne_mio(plan.premier_utilisable, taille_bloc);
    let esp_fin = esp_debut + (64 * 1024 * 1024 / taille_bloc) as u64 - 1;
    let sys_debut = aligne_mio(esp_fin + 1, taille_bloc);
    let partitions = vec![
        Partition::neuve(TYPE_ESP, Guid::depuis(1, 2, 3, [4; 8]), esp_debut, esp_fin, "BOUCHAUD ESP"),
        Partition::neuve(
            TYPE_SYSTEME_BOUCHAUD,
            Guid::depuis(5, 6, 7, [8; 8]),
            sys_debut,
            plan.dernier_utilisable,
            "BOUCHAUD SYSTEME",
        ),
    ];
    ecrit_table(&mut disque, Guid::depuis(9, 9, 9, [9; 8]), &partitions).expect("table ecrite");
    (disque, plan, partitions)
}

// ---------------------------------------------------------------------------
// CRC32
// ---------------------------------------------------------------------------

#[test]
fn crc32_est_bien_celui_de_la_specification() {
    // Vecteur canonique : "123456789" -> 0xCBF43926.
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(crc32(b""), 0);
}

// ---------------------------------------------------------------------------
// Le GUID mixte
// ---------------------------------------------------------------------------

#[test]
fn un_guid_gpt_n_est_pas_ecrit_dans_l_ordre_du_texte() {
    // C12A7328-F81F-11D2-BA4B-00A0C93EC93B, le type d'une ESP.
    let o = TYPE_ESP.octets();
    assert_eq!(
        &o[0..4],
        &[0x28, 0x73, 0x2A, 0xC1],
        "les trois premiers champs sont petit-boutistes ; ecrire les seize \
         octets dans l'ordre du texte donne un GUID que le micrologiciel ne \
         reconnait pas"
    );
    assert_eq!(&o[4..6], &[0x1F, 0xF8]);
    assert_eq!(&o[6..8], &[0xD2, 0x11]);
    assert_eq!(&o[8..16], &[0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9, 0x3B]);
}

#[test]
fn un_guid_fait_l_aller_retour() {
    let o = TYPE_SYSTEME_BOUCHAUD.octets();
    assert_eq!(Guid::depuis_octets(&o), TYPE_SYSTEME_BOUCHAUD);
}

#[test]
fn un_guid_derive_porte_sa_version_et_sa_variante() {
    let mut graine = 0x1234_5678_9ABC_DEF0u64;
    let g = guid_aleatoire(&mut graine);
    assert_eq!(g.d3 >> 12, 4, "version 4");
    assert_eq!(g.d4[0] >> 6, 0b10, "variante RFC 4122");
    // Deterministe : deux appels sur la meme graine donnent la meme chose,
    // deux appels successifs donnent des choses differentes.
    let mut a = 7u64;
    let mut b = 7u64;
    assert_eq!(guid_aleatoire(&mut a), guid_aleatoire(&mut b));
    let mut c = 7u64;
    let premier = guid_aleatoire(&mut c);
    assert_ne!(premier, guid_aleatoire(&mut c));
}

// ---------------------------------------------------------------------------
// L'en-tete et sa somme
// ---------------------------------------------------------------------------

#[test]
fn la_somme_de_l_entete_se_calcule_son_propre_champ_a_zero() {
    let e = Entete {
        mon_lba: 1,
        autre_lba: 999,
        premier_utilisable: 34,
        dernier_utilisable: 966,
        disque: Guid::depuis(1, 1, 1, [1; 8]),
        tableau_lba: 2,
        entrees: 128,
        taille_entree: 128,
        crc_tableau: 0xDEAD_BEEF,
    };
    let bloc = encode_entete(&e, 512);
    let annoncee = u32::from_le_bytes(bloc[16..20].try_into().unwrap());
    let mut copie = bloc[..92].to_vec();
    copie[16..20].fill(0);
    assert_eq!(
        crc32(&copie),
        annoncee,
        "une somme calculee sur elle-meme est toujours fausse, et le \
         micrologiciel bascule alors silencieusement sur la copie de secours"
    );
    assert_eq!(decode_entete(&bloc), Some(e));
}

#[test]
fn la_somme_de_l_entete_ne_porte_pas_sur_le_remplissage() {
    let e = Entete {
        mon_lba: 1,
        autre_lba: 999,
        premier_utilisable: 34,
        dernier_utilisable: 966,
        disque: Guid::nul(),
        tableau_lba: 2,
        entrees: 128,
        taille_entree: 128,
        crc_tableau: 0,
    };
    let mut bloc = encode_entete(&e, 512);
    // Salir le remplissage APRES le 92e octet ne doit rien changer : la
    // specification calcule sur la taille annoncee, pas sur le bloc.
    bloc[200] = 0xFF;
    assert_eq!(
        decode_entete(&bloc),
        Some(e),
        "inclure le remplissage donnerait un en-tete que le micrologiciel \
         refuse des qu'un octet du bloc change"
    );
}

#[test]
fn un_entete_dont_la_somme_est_fausse_est_refuse() {
    let e = Entete {
        mon_lba: 1,
        autre_lba: 99,
        premier_utilisable: 34,
        dernier_utilisable: 60,
        disque: Guid::nul(),
        tableau_lba: 2,
        entrees: 128,
        taille_entree: 128,
        crc_tableau: 0,
    };
    let mut bloc = encode_entete(&e, 512);
    bloc[24] ^= 0x01;
    assert_eq!(
        decode_entete(&bloc),
        None,
        "rendre un en-tete « presque bon » ferait ecrire ensuite en se fiant \
         a des bornes qui ne veulent rien dire"
    );
}

#[test]
fn une_signature_etrangere_est_refusee() {
    let mut bloc = vec![0u8; 512];
    bloc[0..8].copy_from_slice(b"NOTAPART");
    assert_eq!(decode_entete(&bloc), None);
    assert_eq!(decode_entete(&[0u8; 8]), None);
}

// ---------------------------------------------------------------------------
// Le MBR de protection
// ---------------------------------------------------------------------------

#[test]
fn le_mbr_de_protection_couvre_tout_le_disque() {
    let mbr = mbr_protecteur(1000, 512);
    assert_eq!(mbr[446 + 4], 0xEE, "type GPT protective");
    assert_eq!(u32::from_le_bytes(mbr[454..458].try_into().unwrap()), 1);
    assert_eq!(u32::from_le_bytes(mbr[458..462].try_into().unwrap()), 999);
    assert_eq!(&mbr[510..512], &[0x55, 0xAA]);
}

#[test]
fn un_disque_immense_sature_le_champ_de_taille_du_mbr() {
    // Le champ fait trente-deux bits. Un debordement donnerait une taille
    // absurde -- et un outil tiers proposerait de « reparer » le disque.
    let mbr = mbr_protecteur(0x1_0000_0000 + 5000, 512);
    assert_eq!(
        u32::from_le_bytes(mbr[458..462].try_into().unwrap()),
        0xFFFF_FFFF
    );
}

// ---------------------------------------------------------------------------
// La disposition
// ---------------------------------------------------------------------------

#[test]
fn la_disposition_reserve_les_deux_tables() {
    let plan = disposition(1000, 512).unwrap();
    assert_eq!(plan.blocs_tableau, 32, "128 entrees de 128 octets en blocs de 512");
    assert_eq!(plan.tableau_primaire, 2);
    assert_eq!(plan.premier_utilisable, 34);
    assert_eq!(plan.entete_secours, 999);
    assert_eq!(plan.tableau_secours, 967);
    assert_eq!(
        plan.dernier_utilisable, 966,
        "le dernier bloc utilisable PRECEDE le tableau de secours ; l'y \
         inclure ferait ecraser la table par la derniere partition"
    );
}

#[test]
fn un_bloc_de_4096_reduit_le_nombre_de_blocs_de_table() {
    let plan = disposition(1000, 4096).unwrap();
    assert_eq!(plan.blocs_tableau, 4);
    assert_eq!(plan.premier_utilisable, 6);
}

#[test]
fn un_disque_trop_petit_est_refuse_plutot_que_mal_partitionne() {
    assert_eq!(disposition(10, 512), None);
    assert_eq!(disposition(0, 512), None);
    // Une taille de bloc absurde aussi.
    assert_eq!(disposition(1000, 0), None);
}

#[test]
fn l_alignement_sur_le_mebioctet_est_calcule_en_blocs() {
    assert_eq!(aligne_mio(34, 512), 2048, "un mebioctet = 2048 blocs de 512");
    assert_eq!(aligne_mio(2048, 512), 2048, "deja aligne : inchange");
    assert_eq!(aligne_mio(2049, 512), 4096);
    assert_eq!(aligne_mio(6, 4096), 256, "un mebioctet = 256 blocs de 4096");
}

// ---------------------------------------------------------------------------
// Les entrees
// ---------------------------------------------------------------------------

#[test]
fn le_dernier_bloc_d_une_partition_est_inclus() {
    let p = Partition::neuve(TYPE_ESP, Guid::nul(), 2048, 2048 + 999, "X");
    assert_eq!(
        p.blocs(),
        1000,
        "GPT compte le dernier bloc INCLUS ; le traiter comme exclusif fait \
         deborder chaque partition d'un bloc sur la suivante"
    );
}

#[test]
fn le_nom_d_une_partition_est_en_utf16() {
    let p = Partition::neuve(TYPE_ESP, Guid::nul(), 100, 200, "ESP");
    let e = p.encode();
    assert_eq!(&e[56..62], &[b'E', 0, b'S', 0, b'P', 0]);
    assert_eq!(Partition::decode(&e).unwrap().nom, p.nom);
}

#[test]
fn une_entree_de_type_nul_n_est_pas_une_partition() {
    // Sans ce refus, les 128 emplacements du tableau seraient tous rendus
    // comme des partitions de taille zero.
    assert_eq!(Partition::decode(&[0u8; 128]), None);
}

#[test]
fn une_entree_fait_l_aller_retour() {
    let p = Partition::neuve(TYPE_SYSTEME_BOUCHAUD, Guid::depuis(1, 2, 3, [4; 8]), 4096, 99999, "SYS");
    assert_eq!(Partition::decode(&p.encode()), Some(p));
}

// ---------------------------------------------------------------------------
// La table complete, ecrite puis relue
// ---------------------------------------------------------------------------

#[test]
fn une_table_ecrite_se_relit() {
    let (mut disque, _plan, attendues) = disque_partitionne(400_000, 512);
    let (lues, degradee) = lit_table(&mut disque).expect("table lisible");
    assert!(!degradee, "l'en-tete primaire doit suffire");
    assert_eq!(lues.len(), 2);
    assert_eq!(lues[0], attendues[0]);
    assert_eq!(lues[1], attendues[1]);
}

#[test]
fn la_secours_n_est_pas_une_copie_de_la_primaire() {
    let (disque, plan, _) = disque_partitionne(400_000, 512);
    let primaire = decode_entete(disque.bloc(1)).expect("primaire valide");
    let secours = decode_entete(disque.bloc(plan.entete_secours)).expect("secours valide");
    assert_eq!(primaire.mon_lba, 1);
    assert_eq!(secours.mon_lba, plan.entete_secours);
    assert_eq!(
        secours.autre_lba, 1,
        "les deux LBA sont ECHANGES ; une copie octet pour octet ferait \
         pointer la secours sur elle-meme"
    );
    assert_eq!(
        secours.tableau_lba, plan.tableau_secours,
        "une copie ferait pointer la secours sur le tableau PRIMAIRE, donc \
         sur des blocs que l'on peut ecraser"
    );
    assert_eq!(secours.crc_tableau, primaire.crc_tableau);
}

#[test]
fn le_disque_bascule_sur_la_secours_quand_la_primaire_est_detruite() {
    let (mut disque, _plan, attendues) = disque_partitionne(400_000, 512);
    // Une primaire aneantie : exactement ce pour quoi la secours existe.
    // L'en-tete primaire est au bloc UN ; le bloc zero est le MBR.
    for i in 512..1024 {
        disque.octets[i] = 0;
    }
    let (lues, degradee) = lit_table(&mut disque).expect("secours lisible");
    assert!(
        degradee,
        "un disque qui ne demarre que par sa secours est un disque abime, et \
         il faut savoir qu'il l'est"
    );
    assert_eq!(lues, attendues);
}

#[test]
fn un_tableau_altere_est_refuse_par_sa_somme() {
    let (mut disque, _plan, _) = disque_partitionne(400_000, 512);
    // Un octet du tableau primaire change, et la somme cesse de correspondre.
    disque.octets[2 * 512 + 40] ^= 0xFF;
    assert_eq!(lit_table(&mut disque), Err(Erreur::TableAbsente));
}

#[test]
fn le_mbr_est_ecrit_en_dernier() {
    // Refuser l'ecriture du MBR doit laisser un disque SANS table declaree
    // plutot qu'un disque qui se declare partitionne sans l'etre.
    let mut disque = Disque::neuf(400_000, 512);
    disque.refuse.push(0);
    let plan = disposition(400_000, 512).unwrap();
    let p = Partition::neuve(TYPE_ESP, Guid::nul(), 2048, 4095, "ESP");
    assert_eq!(
        ecrit_table(&mut disque, Guid::nul(), &[p]),
        Err(Erreur::EcritureRefusee)
    );
    assert_eq!(&disque.bloc(0)[510..512], &[0, 0], "aucun MBR annonce");
    // Les en-tetes, eux, sont deja la : c'est le sens de l'ordre choisi.
    assert!(decode_entete(disque.bloc(1)).is_some());
    assert!(decode_entete(disque.bloc(plan.entete_secours)).is_some());
}

#[test]
fn une_partition_hors_de_la_zone_utilisable_est_refusee() {
    let mut disque = Disque::neuf(1000, 512);
    // Le bloc 10 est dans le TABLEAU primaire.
    let p = Partition::neuve(TYPE_ESP, Guid::nul(), 10, 900, "X");
    assert_eq!(
        ecrit_table(&mut disque, Guid::nul(), &[p]),
        Err(Erreur::PartitionHorsZone)
    );
    // Le bloc 990 est dans le tableau de SECOURS.
    let q = Partition::neuve(TYPE_ESP, Guid::nul(), 100, 990, "X");
    assert_eq!(
        ecrit_table(&mut disque, Guid::nul(), &[q]),
        Err(Erreur::PartitionHorsZone)
    );
}

#[test]
fn deux_partitions_qui_se_chevauchent_sont_refusees() {
    let mut disque = Disque::neuf(400_000, 512);
    let a = Partition::neuve(TYPE_ESP, Guid::nul(), 2048, 4096, "A");
    let b = Partition::neuve(TYPE_SYSTEME_BOUCHAUD, Guid::nul(), 4096, 8192, "B");
    assert_eq!(
        ecrit_table(&mut disque, Guid::nul(), &[a, b]),
        Err(Erreur::PartitionsQuiSeChevauchent),
        "un seul bloc commun suffit a corrompre les deux systemes de fichiers"
    );
}

#[test]
fn une_partition_a_l_envers_est_refusee() {
    let mut disque = Disque::neuf(400_000, 512);
    let p = Partition::neuve(TYPE_ESP, Guid::nul(), 8192, 4096, "X");
    assert_eq!(
        ecrit_table(&mut disque, Guid::nul(), &[p]),
        Err(Erreur::PartitionHorsZone)
    );
}

#[test]
fn ecrire_une_table_demande_une_vidange() {
    let (disque, _, _) = disque_partitionne(400_000, 512);
    assert!(
        disque.vidanges >= 1,
        "une table qui n'est pas encore sur le plateau au moment de la coupure \
         laisse un disque partitionne a moitie"
    );
}

#[test]
fn une_table_en_blocs_de_4096_est_coherente() {
    let (mut disque, _plan, attendues) = disque_partitionne(200_000, 4096);
    let (lues, degradee) = lit_table(&mut disque).expect("table lisible");
    assert!(!degradee);
    assert_eq!(lues, attendues);
}

#[test]
fn un_disque_vierge_n_a_pas_de_table() {
    let mut disque = Disque::neuf(1000, 512);
    assert_eq!(lit_table(&mut disque), Err(Erreur::TableAbsente));
}
