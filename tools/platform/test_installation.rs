//! Preuve hote de l'installation sur le disque interne.
//!
//! Ce test ne se contente pas d'affirmer l'arithmetique : il fabrique un
//! disque en memoire, y ecrit une VRAIE table GPT et une VRAIE ESP avec le
//! code de production, et depose l'image sur le disque quand
//! `BO_INSTALL_IMAGE` le demande -- pour que `tools/verifie-installation.py`
//! la donne a `fsck.fat` et `mtools`.
//!
//! C'est aussi pres qu'on peut aller de la machine physique sans l'avoir : ce
//! qui reste au-dela est le pilote NVMe et le micrologiciel, et rien d'autre.

#![allow(dead_code)]

extern crate alloc;

#[path = "../../src/fs/gpt.rs"]
mod gpt;

#[path = "../../src/fs/fat32.rs"]
mod fat32;

#[path = "../../src/platform/pc/installation/disposition.rs"]
mod disposition;

use disposition::*;
use gpt::{Partition, Support};

/// Un disque CREUX en memoire.
///
/// Un disque plein de seize gibioctets ferait echouer ces tests sur
/// l'allocation, pas sur le code -- et une installation n'ecrit qu'une
/// fraction du disque, ce qui est justement ce qu'on veut verifier. Les blocs
/// jamais ecrits se lisent nuls, exactement comme un disque neuf.
struct Disque {
    taille_bloc: usize,
    blocs: u64,
    ecrits: std::collections::HashMap<u64, Vec<u8>>,
    vidanges: usize,
}

impl Disque {
    fn neuf(blocs: u64, taille_bloc: usize) -> Self {
        Self { taille_bloc, blocs, ecrits: std::collections::HashMap::new(), vidanges: 0 }
    }

    fn bloc(&self, lba: u64) -> Vec<u8> {
        match self.ecrits.get(&lba) {
            Some(octets) => octets.clone(),
            None => vec![0u8; self.taille_bloc],
        }
    }

    /// Les octets d'une plage, materialises.
    fn plage(&self, premier: u64, blocs: u64) -> Vec<u8> {
        let mut sortie = Vec::with_capacity((blocs as usize) * self.taille_bloc);
        for i in 0..blocs {
            sortie.extend_from_slice(&self.bloc(premier + i));
        }
        sortie
    }

    /// Combien de blocs ont ete reellement ecrits.
    fn blocs_ecrits(&self) -> usize {
        self.ecrits.len()
    }
}

impl Support for Disque {
    fn taille_bloc(&self) -> usize {
        self.taille_bloc
    }
    fn blocs(&self) -> u64 {
        self.blocs
    }
    fn lit(&mut self, lba: u64, sortie: &mut [u8]) -> bool {
        let n = sortie.len() / self.taille_bloc;
        if n == 0 || lba + n as u64 > self.blocs {
            return false;
        }
        for i in 0..n {
            let d = i * self.taille_bloc;
            sortie[d..d + self.taille_bloc].copy_from_slice(&self.bloc(lba + i as u64));
        }
        true
    }
    fn ecrit(&mut self, lba: u64, donnees: &[u8]) -> bool {
        let n = donnees.len() / self.taille_bloc;
        if n == 0 || lba + n as u64 > self.blocs {
            return false;
        }
        for i in 0..n {
            let d = i * self.taille_bloc;
            self.ecrits.insert(
                lba + i as u64,
                donnees[d..d + self.taille_bloc].to_vec(),
            );
        }
        true
    }
    fn vidange(&mut self) -> bool {
        self.vidanges += 1;
        true
    }
}

const MIO: u64 = 1024 * 1024;
const GIO: u64 = 1024 * MIO;

// ---------------------------------------------------------------------------
// L'arithmetique
// ---------------------------------------------------------------------------

fn bornes(blocs: u64, taille_bloc: usize) -> (u64, u64) {
    let d = gpt::disposition(blocs, taille_bloc).expect("disposition");
    (d.premier_utilisable, d.dernier_utilisable)
}

#[test]
fn les_deux_partitions_sont_alignees_sur_le_mebioctet() {
    // 500 Go, la taille du disque de la machine de reference.
    let blocs = 500 * GIO / 512;
    let (premier, dernier) = bornes(blocs, 512);
    let p = plan(premier, dernier, 512, 800 * MIO).expect("plan");
    assert_eq!(p.esp_premier % 2048, 0, "un mebioctet fait 2048 blocs de 512");
    assert_eq!(
        p.systeme_premier % 2048,
        0,
        "un SSD qui recoit des ecritures desalignees fait des \
         lecture-modification-ecriture invisibles : la partition marche, et \
         elle est durablement plus lente"
    );
}

#[test]
fn les_deux_partitions_ne_se_recouvrent_jamais() {
    for taille_bloc in [512usize, 4096] {
        for gio in [16u64, 64, 240, 500, 2000] {
            let blocs = gio * GIO / taille_bloc as u64;
            let (premier, dernier) = bornes(blocs, taille_bloc);
            let p = plan(premier, dernier, taille_bloc, 900 * MIO)
                .unwrap_or_else(|| panic!("plan absent pour {gio} Gio / {taille_bloc}"));
            assert!(
                !se_recouvrent(&p),
                "{gio} Gio / {taille_bloc} : un seul bloc commun corrompt les \
                 deux systemes de fichiers"
            );
            assert!(p.systeme_premier > p.esp_dernier);
        }
    }
}

#[test]
fn les_partitions_restent_dans_la_zone_utilisable() {
    let blocs = 500 * GIO / 512;
    let (premier, dernier) = bornes(blocs, 512);
    let p = plan(premier, dernier, 512, 800 * MIO).unwrap();
    assert!(p.esp_premier >= premier);
    assert!(
        p.systeme_dernier <= dernier,
        "deborder ecraserait la table GPT de secours, et le disque ne \
         demarrerait plus apres le premier remplissage"
    );
}

#[test]
fn l_esp_ne_descend_jamais_sous_le_seuil_du_fat32() {
    let blocs = 64 * GIO / 512;
    let (premier, dernier) = bornes(blocs, 512);
    // Une charge minuscule ne doit pas donner une ESP minuscule.
    let p = plan(premier, dernier, 512, 1024).unwrap();
    assert!(
        p.esp_blocs() * 512 >= ESP_MINIMUM_MIO * MIO,
        "sous 65 525 amas, un micrologiciel conforme lit le volume comme du \
         FAT16 et n'y trouve rien"
    );
    // Et la taille de l'ESP est bien choisie par le seuil, pas par la charge.
    assert_eq!(p.esp_blocs() * 512, ESP_MINIMUM_MIO * MIO);
}

#[test]
fn une_grosse_charge_agrandit_l_esp_avec_sa_marge() {
    let blocs = 500 * GIO / 512;
    let (premier, dernier) = bornes(blocs, 512);
    let charge = 1500 * MIO;
    let p = plan(premier, dernier, 512, charge).unwrap();
    let attendu = (charge / MIO + ESP_MARGE_MIO) * MIO;
    assert_eq!(p.esp_blocs() * 512, attendu);
    assert!(
        p.esp_blocs() * 512 > charge,
        "une ESP calee au plus juste rend la prochaine mise a jour impossible \
         sans repartitionner un disque qui porte deja des donnees"
    );
}

#[test]
fn un_disque_trop_petit_est_refuse_plutot_que_mal_partitionne() {
    // 900 Mio : de quoi porter l'ESP minimale et une partition systeme de
    // quelques dizaines de mebioctets -- c'est-a-dire trop petite pour la zone
    // de persistance, qui en fait 128 a elle seule.
    let blocs = 900 * MIO / 512;
    let (premier, dernier) = bornes(blocs, 512);
    assert_eq!(
        plan(premier, dernier, 512, 1024),
        None,
        "une partition systeme trop petite pour la zone de persistance serait \
         creee, formatee, declaree installee -- et chaque « sync » y \
         echouerait en silence"
    );
}

#[test]
fn un_disque_qui_ne_porte_meme_pas_l_esp_est_refuse() {
    let blocs = 100 * MIO / 512;
    let (premier, dernier) = bornes(blocs, 512);
    assert_eq!(plan(premier, dernier, 512, 1024), None);
}

#[test]
fn l_alignement_depend_de_la_taille_de_bloc() {
    assert_eq!(blocs_par_mio(512), 2048);
    assert_eq!(blocs_par_mio(4096), 256);
    assert_eq!(aligne(1, 512), 2048);
    assert_eq!(aligne(2048, 512), 2048);
    assert_eq!(aligne(2049, 512), 4096);
    assert_eq!(aligne(1, 4096), 256);
}

#[test]
fn des_bornes_absurdes_ne_paniquent_pas() {
    assert_eq!(plan(100, 50, 512, 1024), None);
    assert_eq!(plan(0, u64::MAX, 0, 1024), None);
    // Une charge absurde mais qui ne deborde pas : l'ESP mangerait le disque
    // et la partition systeme serait un residu. Le plan est arithmetiquement
    // valide, et il faut quand meme le refuser.
    let (premier, dernier) = bornes(16 * GIO / 512, 512);
    assert_eq!(
        plan(premier, dernier, 512, 12 * GIO),
        None,
        "l'ESP ne prend jamais plus du quart de la zone utilisable"
    );
}

// ---------------------------------------------------------------------------
// L'installation complete, sur un disque en memoire
// ---------------------------------------------------------------------------

struct Resultat {
    disque: Disque,
    plan: Plan,
    noyau: Vec<u8>,
    archive: Vec<u8>,
}

/// Reproduit exactement ce que `installation::installe` fait, avec le code de
/// production pour la table et le systeme de fichiers.
fn installe(gio: u64, taille_bloc: usize) -> Resultat {
    let blocs = gio * GIO / taille_bloc as u64;
    let mut disque = Disque::neuf(blocs, taille_bloc);
    let table = gpt::disposition(blocs, taille_bloc).expect("disposition");

    let chargeur: Vec<u8> = (0..150_000u32).map(|i| (i % 251) as u8).collect();
    let noyau: Vec<u8> = (0..3_000_000u32).map(|i| (i % 241) as u8).collect();
    let config = br#"{"version":1,"frame-buffer":{}}"#.to_vec();
    let archive: Vec<u8> = (0..12_000_000u32).map(|i| (i % 233) as u8).collect();
    let utile = (chargeur.len() + noyau.len() + config.len() + archive.len()) as u64;

    let pose = plan(
        table.premier_utilisable,
        table.dernier_utilisable,
        taille_bloc,
        utile,
    )
    .expect("plan");

    let mut graine = 0xC0FF_EE00_1234_5678u64;
    let partitions = [
        Partition::neuve(
            gpt::TYPE_ESP,
            gpt::guid_aleatoire(&mut graine),
            pose.esp_premier,
            pose.esp_dernier,
            "BOUCHAUD ESP",
        ),
        Partition::neuve(
            gpt::TYPE_SYSTEME_BOUCHAUD,
            gpt::guid_aleatoire(&mut graine),
            pose.systeme_premier,
            pose.systeme_dernier,
            "BOUCHAUD SYSTEME",
        ),
    ];
    gpt::ecrit_table(&mut disque, gpt::guid_aleatoire(&mut graine), &partitions)
        .expect("table ecrite");

    {
        let mut volume = fat32::formate(
            &mut disque,
            pose.esp_premier,
            pose.esp_blocs(),
            "BOUCHAUDESP",
            0x1234_5678,
        )
        .expect("ESP formatee");
        let racine = volume.racine();
        let efi = volume.cree_repertoire(racine, "EFI").expect("EFI");
        let boot = volume.cree_repertoire(efi, "BOOT").expect("BOOT");
        volume.ecrit_fichier(boot, "BOOTX64.EFI", &chargeur).expect("chargeur");
        volume.ecrit_fichier(racine, "kernel-x86_64", &noyau).expect("noyau");
        volume.ecrit_fichier(racine, "boot.json", &config).expect("config");
        volume.ecrit_fichier(racine, "ramdisk", &archive).expect("archive");
        volume.termine().expect("volume referme");
    }
    Resultat { disque, plan: pose, noyau, archive }
}

#[test]
fn une_installation_produit_une_table_relisible() {
    let mut r = installe(16, 512);
    let (partitions, degradee) = gpt::lit_table(&mut r.disque).expect("table lisible");
    assert!(!degradee);
    assert_eq!(partitions.len(), 2);
    assert_eq!(partitions[0].type_guid, gpt::TYPE_ESP);
    assert_eq!(partitions[1].type_guid, gpt::TYPE_SYSTEME_BOUCHAUD);
    assert_eq!(partitions[0].premier, r.plan.esp_premier);
    assert_eq!(partitions[1].dernier, r.plan.systeme_dernier);
}

#[test]
fn les_deux_guid_de_partition_sont_differents() {
    let mut r = installe(16, 512);
    let (partitions, _) = gpt::lit_table(&mut r.disque).unwrap();
    assert_ne!(
        partitions[0].unique, partitions[1].unique,
        "un chargeur qui designe une partition par son GUID demarrerait la \
         mauvaise"
    );
}

#[test]
fn l_esp_ne_touche_pas_la_partition_systeme() {
    let r = installe(16, 512);
    // La partition systeme n'a rien recu : aucun de ses blocs ne doit avoir
    // ete ecrit. Sur un disque creux, la question se pose directement.
    for i in 0..64u64 {
        assert!(
            !r.disque.ecrits.contains_key(&(r.plan.systeme_premier + i)),
            "le formatage de l'ESP a deborde sur la partition systeme (bloc {i})"
        );
    }
}

#[test]
fn la_table_de_secours_survit_a_l_installation() {
    let r = installe(16, 512);
    let table = gpt::disposition(r.disque.blocs(), 512).unwrap();
    let secteur = r.disque.bloc(table.entete_secours);
    let entete = gpt::decode_entete(&secteur)
        .expect("la secours doit rester valide : c'est elle qui sauve un \
                 disque dont le premier secteur est abime");
    assert_eq!(entete.mon_lba, table.entete_secours);
}

#[test]
fn le_ramdisk_installe_est_celui_qui_tournait() {
    // Le systeme installe recoit, octet pour octet, l'archive qui vient de
    // tourner. Une archive tronquee donnerait un systeme qui demarre et dont
    // le navigateur manque.
    let r = installe(16, 512);
    // Les premiers mebioctets de l'ESP suffisent : l'en-tete, les FAT et la
    // racine y tiennent.
    let esp = r.disque.plage(r.plan.esp_premier, 8192);
    let taille = lit_taille(&esp, "ramdisk").expect("ramdisk present");
    assert_eq!(taille as usize, r.archive.len());
    let noyau = lit_taille(&esp, "kernel-x86_64").expect("noyau present");
    assert_eq!(noyau as usize, r.noyau.len());
}

/// Lit la taille d'un fichier de la racine, via un decodage minimal du
/// repertoire -- independant de l'ecrivain.
fn lit_taille(esp: &[u8], nom: &str) -> Option<u32> {
    let bps = u16::from_le_bytes([esp[11], esp[12]]) as usize;
    let spc = esp[13] as usize;
    let reserves = u16::from_le_bytes([esp[14], esp[15]]) as usize;
    let fats = esp[16] as usize;
    let par_fat = u32::from_le_bytes([esp[36], esp[37], esp[38], esp[39]]) as usize;
    let donnees = reserves + fats * par_fat;
    let racine = u32::from_le_bytes([esp[44], esp[45], esp[46], esp[47]]) as usize;
    let debut = (donnees + (racine - 2) * spc) * bps;
    let secteur = &esp[debut..debut + spc * bps];
    let mut morceaux: Vec<(u8, String)> = Vec::new();
    for i in 0..secteur.len() / 32 {
        let e = &secteur[i * 32..(i + 1) * 32];
        if e[0] == 0 {
            break;
        }
        if e[11] == 0x0F {
            const P: [usize; 13] = [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];
            let mut texte = String::new();
            for p in P {
                let u = u16::from_le_bytes([e[p], e[p + 1]]);
                if u == 0 || u == 0xFFFF {
                    break;
                }
                texte.push(char::from_u32(u as u32)?);
            }
            morceaux.push((e[0] & 0x3F, texte));
            continue;
        }
        if !morceaux.is_empty() {
            morceaux.sort_by_key(|(n, _)| *n);
            let complet: String = morceaux.iter().map(|(_, t)| t.as_str()).collect();
            morceaux.clear();
            if complet == nom {
                return Some(u32::from_le_bytes([e[28], e[29], e[30], e[31]]));
            }
        }
    }
    None
}

#[test]
fn une_installation_en_blocs_de_4096_est_coherente() {
    let mut r = installe(16, 4096);
    let (partitions, degradee) = gpt::lit_table(&mut r.disque).expect("table lisible");
    assert!(!degradee);
    assert_eq!(partitions.len(), 2);
    assert_eq!(partitions[0].premier % 256, 0, "aligne sur le mebioctet");
}

#[test]
fn ecrit_l_image_pour_fsck_et_mtools() {
    let Ok(chemin) = std::env::var("BO_INSTALL_IMAGE") else { return };
    let r = installe(16, 512);
    // On depose l'ESP seule : `fsck.fat` et `mtools` attendent un systeme de
    // fichiers, pas un disque partitionne.
    let esp = r.disque.plage(r.plan.esp_premier, r.plan.esp_blocs());
    std::fs::write(&chemin, &esp).expect("image ecrite");
}
