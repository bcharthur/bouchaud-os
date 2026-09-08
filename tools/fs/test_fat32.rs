//! Preuve hote de l'ecriture FAT32.
//!
//! Deux niveaux de preuve, et le second est celui qui compte.
//!
//! Le premier est un LECTEUR ECRIT ICI, a partir de la specification, qui ne
//! partage aucune ligne avec le module de production. Un test qui relirait
//! avec le code qui a ecrit validerait la coherence d'une erreur avec
//! elle-meme.
//!
//! Le second est `tools/verifie-fat32-reel.py`, qui donne l'image produite a
//! `fsck.fat` et a `mtools` -- des implementations qui n'ont jamais entendu
//! parler de ce projet. Ce fichier-ci ecrit donc l'image sur le disque quand
//! `BO_FAT_IMAGE` le demande, pour que cette verification ait quelque chose a
//! examiner.
//!
//! # Ce que ces preuves cherchent
//!
//! Les fautes de FAT32 ne font pas planter : elles font disparaitre un
//! fichier. Le chargeur d'amorcage cherche `kernel-x86_64` et `boot.json`, et
//! ni l'un ni l'autre ne tient dans un nom 8.3. Ecrits sans nom long, ils
//! deviennent `KERNEL-X` et `BOOT.JSO` -- et la machine ne demarre pas, sans
//! un mot.

#![allow(dead_code)]

extern crate alloc;

#[path = "../../src/fs/gpt.rs"]
mod gpt;

#[path = "../../src/fs/fat32.rs"]
mod fat32;

use fat32::*;

/// Un disque en memoire.
struct Disque {
    taille_bloc: usize,
    octets: Vec<u8>,
}

impl Disque {
    fn neuf(blocs: u64, taille_bloc: usize) -> Self {
        Self { taille_bloc, octets: vec![0u8; (blocs as usize) * taille_bloc] }
    }
}

impl fat32::Support for Disque {
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
        let d = (lba as usize) * self.taille_bloc;
        if d + donnees.len() > self.octets.len() {
            return false;
        }
        self.octets[d..d + donnees.len()].copy_from_slice(donnees);
        true
    }
    fn vidange(&mut self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// Un lecteur INDEPENDANT, ecrit depuis la specification.
// ---------------------------------------------------------------------------

struct Lecteur<'a> {
    image: &'a [u8],
    bps: usize,
    spc: usize,
    reserves: usize,
    fats: usize,
    secteurs_par_fat: usize,
    racine: u32,
}

#[derive(Debug, PartialEq, Eq, Clone)]
struct Trouve {
    nom: String,
    repertoire: bool,
    amas: u32,
    taille: u32,
}

impl<'a> Lecteur<'a> {
    fn neuf(image: &'a [u8]) -> Self {
        let bps = u16::from_le_bytes([image[11], image[12]]) as usize;
        Self {
            bps,
            spc: image[13] as usize,
            reserves: u16::from_le_bytes([image[14], image[15]]) as usize,
            fats: image[16] as usize,
            secteurs_par_fat: u32::from_le_bytes([
                image[36], image[37], image[38], image[39],
            ]) as usize,
            racine: u32::from_le_bytes([image[44], image[45], image[46], image[47]]),
            image,
        }
    }

    fn premier_secteur_donnees(&self) -> usize {
        self.reserves + self.fats * self.secteurs_par_fat
    }

    fn fat(&self, amas: u32) -> u32 {
        let position = self.reserves * self.bps + amas as usize * 4;
        u32::from_le_bytes([
            self.image[position],
            self.image[position + 1],
            self.image[position + 2],
            self.image[position + 3],
        ]) & 0x0FFF_FFFF
    }

    fn octets_amas(&self, amas: u32) -> &[u8] {
        let secteur = self.premier_secteur_donnees() + (amas as usize - 2) * self.spc;
        let debut = secteur * self.bps;
        &self.image[debut..debut + self.spc * self.bps]
    }

    /// La chaine complete d'un fichier, concatenee.
    fn chaine(&self, premier: u32, taille: usize) -> Vec<u8> {
        let mut sortie = Vec::new();
        let mut amas = premier;
        while (2..0x0FFF_FFF7).contains(&amas) && sortie.len() < taille {
            sortie.extend_from_slice(self.octets_amas(amas));
            amas = self.fat(amas);
        }
        sortie.truncate(taille);
        sortie
    }

    /// Liste un repertoire, en reassemblant les noms longs.
    fn liste(&self, repertoire: u32) -> Vec<Trouve> {
        let mut sortie = Vec::new();
        let mut amas = repertoire;
        let mut morceaux: Vec<(u8, String)> = Vec::new();
        while (2..0x0FFF_FFF7).contains(&amas) {
            let donnees = self.octets_amas(amas);
            for i in 0..donnees.len() / 32 {
                let e = &donnees[i * 32..(i + 1) * 32];
                if e[0] == 0x00 {
                    return sortie;
                }
                if e[0] == 0xE5 {
                    morceaux.clear();
                    continue;
                }
                if e[11] == 0x0F {
                    // Entree de nom long : les treize caracteres a leurs
                    // positions non contigues.
                    const POSITIONS: [usize; 13] =
                        [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];
                    let mut texte = String::new();
                    for p in POSITIONS {
                        let u = u16::from_le_bytes([e[p], e[p + 1]]);
                        if u == 0 || u == 0xFFFF {
                            break;
                        }
                        texte.push(char::from_u32(u as u32).unwrap_or('?'));
                    }
                    morceaux.push((e[0] & 0x3F, texte));
                    continue;
                }
                if e[11] & 0x08 != 0 {
                    // Etiquette de volume.
                    morceaux.clear();
                    continue;
                }
                let nom = if morceaux.is_empty() {
                    let base = String::from_utf8_lossy(&e[0..8]).trim_end().to_string();
                    let ext = String::from_utf8_lossy(&e[8..11]).trim_end().to_string();
                    if ext.is_empty() { base } else { format!("{base}.{ext}") }
                } else {
                    // Les entrees ont ete ecrites dans l'ordre INVERSE : la
                    // derniere partie du nom en premier.
                    morceaux.sort_by_key(|(numero, _)| *numero);
                    morceaux.iter().map(|(_, t)| t.as_str()).collect::<String>()
                };
                morceaux.clear();
                let haut = u16::from_le_bytes([e[20], e[21]]) as u32;
                let bas = u16::from_le_bytes([e[26], e[27]]) as u32;
                sortie.push(Trouve {
                    nom,
                    repertoire: e[11] & 0x10 != 0,
                    amas: (haut << 16) | bas,
                    taille: u32::from_le_bytes([e[28], e[29], e[30], e[31]]),
                });
            }
            amas = self.fat(amas);
        }
        sortie
    }

    fn cherche(&self, repertoire: u32, nom: &str) -> Option<Trouve> {
        self.liste(repertoire).into_iter().find(|t| t.nom == nom)
    }
}

/// Fabrique une ESP comme l'installateur le fera.
fn esp(mio: u64, taille_bloc: usize) -> (Disque, Vec<u8>, Vec<u8>) {
    let secteurs = mio * 1024 * 1024 / taille_bloc as u64;
    let mut disque = Disque::neuf(secteurs, taille_bloc);
    let noyau: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let config = br#"{"version":1,"frame-buffer":{}}"#.to_vec();
    {
        let mut volume =
            formate(&mut disque, 0, secteurs, "BOUCHAUDESP", 0x1234_5678).expect("formatage");
        let racine = volume.racine();
        let efi = volume.cree_repertoire(racine, "EFI").expect("EFI");
        let boot = volume.cree_repertoire(efi, "BOOT").expect("BOOT");
        volume
            .ecrit_fichier(boot, "BOOTX64.EFI", &[0xAAu8; 40_000])
            .expect("bootx64");
        volume.ecrit_fichier(racine, "kernel-x86_64", &noyau).expect("noyau");
        volume.ecrit_fichier(racine, "boot.json", &config).expect("config");
        volume.ecrit_fichier(racine, "ramdisk", &[0x5Au8; 100_000]).expect("ramdisk");
        volume.termine().expect("volume referme");
    }
    (disque, noyau, config)
}

// ---------------------------------------------------------------------------
// La geometrie
// ---------------------------------------------------------------------------

#[test]
fn un_volume_trop_petit_pour_du_vrai_fat32_est_refuse() {
    // 16 Mio en amas de 512 octets, cela fait 32 768 amas : en dessous du
    // seuil. Un lecteur conforme y verrait du FAT16 et n'y trouverait rien.
    assert_eq!(choisit_amas(16 * 1024 * 1024 / 512, 512), None);
    // 64 Mio : 131 072 amas d'un secteur, au-dessus du seuil.
    assert_eq!(choisit_amas(64 * 1024 * 1024 / 512, 512), Some(1));
}

#[test]
fn un_grand_volume_prend_de_gros_amas() {
    // 512 Mio : on peut prendre 8 secteurs par amas et rester au-dessus du
    // seuil (65 536 amas).
    let spc = choisit_amas(512 * 1024 * 1024 / 512, 512).unwrap();
    let params = calcule(512 * 1024 * 1024 / 512, 512, spc).unwrap();
    assert!(params.amas >= AMAS_MINIMUM_FAT32, "amas={}", params.amas);
    assert!(spc >= 8, "spc={spc}");
}

#[test]
fn la_taille_de_la_fat_est_le_point_fixe_et_non_une_estimation() {
    let p = calcule(400_000, 512, 8).unwrap();
    let entrees_par_secteur = 512 / 4;
    let voulu = (p.amas + 2).div_ceil(entrees_par_secteur);
    assert_eq!(
        p.secteurs_par_fat, voulu,
        "une FAT trop courte ne decrit pas ses derniers amas ; trop longue, \
         elle en mange"
    );
    assert_eq!(p.premier_secteur_donnees, RESERVES + p.secteurs_par_fat * FATS);
}

#[test]
fn l_amas_deux_est_le_premier_de_la_zone_de_donnees() {
    let p = calcule(400_000, 512, 8).unwrap();
    assert_eq!(
        p.secteur_de_l_amas(2),
        p.premier_secteur_donnees,
        "sans la soustraction, deux amas fantomes resteraient en tete du volume"
    );
    assert_eq!(p.secteur_de_l_amas(3), p.premier_secteur_donnees + 8);
}

#[test]
fn une_taille_de_secteur_non_conforme_est_refusee() {
    assert_eq!(calcule(400_000, 500, 8), None);
    assert_eq!(calcule(400_000, 512, 3), None, "les amas sont des puissances de deux");
}

// ---------------------------------------------------------------------------
// Les noms
// ---------------------------------------------------------------------------

#[test]
fn un_nom_court_reste_court() {
    assert!(tient_en_8_3("BOOTX64.EFI"));
    assert_eq!(&nom_court("BOOTX64.EFI", 1), b"BOOTX64 EFI");
    assert!(tient_en_8_3("RAMDISK"));
    assert_eq!(&nom_court("RAMDISK", 1), b"RAMDISK    ");
}

#[test]
fn les_deux_noms_du_chargeur_ne_tiennent_pas_en_8_3() {
    // C'est le coeur du probleme : sans nom long, le chargeur ne trouve rien.
    assert!(!tient_en_8_3("kernel-x86_64"), "treize caracteres");
    assert!(!tient_en_8_3("boot.json"), "extension de quatre caracteres");
    assert!(!tient_en_8_3("ramdisk"), "minuscules");
}

#[test]
fn un_nom_long_recoit_un_alias_numerote() {
    let court = nom_court("kernel-x86_64", 1);
    assert_eq!(&court, b"KERNEL~1   ");
    assert_eq!(&nom_court("kernel-x86_64", 2), b"KERNEL~2   ");
    assert_eq!(&nom_court("boot.json", 1), b"BOOT~1  JSO");
}

#[test]
fn la_somme_de_controle_est_celle_de_la_specification() {
    // Valeurs calculees par une implementation independante de la
    // specification (voir tools/verifie-fat32-reel.py, qui les recalcule).
    assert_eq!(somme_nom_court(b"THEQUI~1   "), 0xEE);
    assert_eq!(somme_nom_court(b"KERNEL~1   "), 0x17);
    assert_eq!(somme_nom_court(b"BOOT~1  JSO"), 0xFE);
}

#[test]
fn les_entrees_de_nom_long_sont_ecrites_a_l_envers() {
    // Vingt caracteres : deux entrees. La DERNIERE partie vient en premier.
    let nom = "un-nom-tres-tres-long-ici";
    let court = nom_court(nom, 1);
    let entrees = entrees_nom_long(nom, &court);
    assert_eq!(entrees.len(), 2);
    assert_eq!(
        entrees[0][0] & 0x3F,
        2,
        "l'entree de numero le plus haut vient en PREMIER ; l'ordre naturel \
         donne un nom lu a l'envers"
    );
    assert_eq!(entrees[0][0] & 0x40, 0x40, "marque de derniere entree");
    assert_eq!(entrees[1][0], 1);
    for e in &entrees {
        assert_eq!(e[11], 0x0F, "attribut de nom long");
        assert_eq!(e[13], somme_nom_court(&court));
        assert_eq!(&e[26..28], &[0, 0], "l'amas d'une entree longue est nul");
    }
}

#[test]
fn un_nom_de_treize_caracteres_tient_dans_une_entree() {
    let nom = "kernel-x86_64";
    assert_eq!(nom.len(), 13);
    let entrees = entrees_nom_long(nom, &nom_court(nom, 1));
    assert_eq!(entrees.len(), 1);
    assert_eq!(entrees[0][0], 0x41, "premiere ET derniere");
}

// ---------------------------------------------------------------------------
// Le secteur d'amorcage
// ---------------------------------------------------------------------------

#[test]
fn le_secteur_d_amorcage_declare_bien_du_fat32() {
    let p = calcule(400_000, 512, 8).unwrap();
    let s = secteur_amorce(&p, 0x1234, b"BOUCHAUDESP");
    assert_eq!(u16::from_le_bytes([s[17], s[18]]), 0, "entrees racine nulles");
    assert_eq!(
        u16::from_le_bytes([s[19], s[20]]),
        0,
        "le petit compte de secteurs est NUL en FAT32 ; non nul, le volume \
         serait lu comme du FAT16"
    );
    assert_eq!(u16::from_le_bytes([s[22], s[23]]), 0, "FATSz16 nul");
    assert_eq!(u32::from_le_bytes([s[36], s[37], s[38], s[39]]), p.secteurs_par_fat);
    assert_eq!(u32::from_le_bytes([s[44], s[45], s[46], s[47]]), 2, "racine");
    assert_eq!(&s[82..90], b"FAT32   ");
    assert_eq!(&s[510..512], &[0x55, 0xAA]);
    assert_eq!(s[66], 0x29);
}

// ---------------------------------------------------------------------------
// Le volume complet
// ---------------------------------------------------------------------------

#[test]
fn une_esp_relue_par_un_lecteur_independant_contient_ses_fichiers() {
    let (disque, noyau, config) = esp(96, 512);
    let lecteur = Lecteur::neuf(&disque.octets);
    let racine = lecteur.racine;

    let efi = lecteur.cherche(racine, "EFI").expect("EFI present");
    assert!(efi.repertoire);
    let boot = lecteur.cherche(efi.amas, "BOOT").expect("BOOT present");
    assert!(boot.repertoire);
    let chargeur = lecteur.cherche(boot.amas, "BOOTX64.EFI").expect("chargeur present");
    assert_eq!(chargeur.taille, 40_000);
    assert_eq!(lecteur.chaine(chargeur.amas, 40_000), vec![0xAAu8; 40_000]);

    // Les deux noms que le chargeur cherche, et que le 8.3 detruirait.
    let k = lecteur
        .cherche(racine, "kernel-x86_64")
        .expect("le noyau doit etre trouvable par son nom LONG");
    assert_eq!(k.taille as usize, noyau.len());
    assert_eq!(lecteur.chaine(k.amas, noyau.len()), noyau);

    let c = lecteur.cherche(racine, "boot.json").expect("boot.json present");
    assert_eq!(lecteur.chaine(c.amas, config.len()), config);

    let r = lecteur.cherche(racine, "ramdisk").expect("ramdisk present");
    assert_eq!(r.taille, 100_000);
}

#[test]
fn un_sous_repertoire_porte_son_point_et_son_double_point() {
    let (disque, _, _) = esp(96, 512);
    let lecteur = Lecteur::neuf(&disque.octets);
    let efi = lecteur.cherche(lecteur.racine, "EFI").unwrap();
    let entrees = lecteur.liste(efi.amas);
    let point = entrees.iter().find(|t| t.nom == ".").expect(". present");
    let deux = entrees.iter().find(|t| t.nom == "..").expect(".. present");
    assert_eq!(point.amas, efi.amas);
    assert_eq!(
        deux.amas, 0,
        "le « .. » d'un repertoire dont le parent est la RACINE porte l'amas \
         ZERO ; un lecteur strict refuse l'autre forme"
    );
    // Le « .. » de BOOT, lui, designe vraiment EFI.
    let boot = lecteur.cherche(efi.amas, "BOOT").unwrap();
    let deux = lecteur.liste(boot.amas).into_iter().find(|t| t.nom == "..").unwrap();
    assert_eq!(deux.amas, efi.amas);
}

#[test]
fn les_deux_copies_de_la_fat_sont_identiques() {
    let (disque, _, _) = esp(96, 512);
    let lecteur = Lecteur::neuf(&disque.octets);
    let taille = lecteur.secteurs_par_fat * lecteur.bps;
    let premiere = lecteur.reserves * lecteur.bps;
    let seconde = premiere + taille;
    assert_eq!(
        &disque.octets[premiere..premiere + taille],
        &disque.octets[seconde..seconde + taille],
        "n'ecrire qu'une copie donne un volume que le systeme vivant lit sans \
         probleme et qu'un outil de reparation « repare » en recopiant la \
         copie perimee"
    );
}

#[test]
fn les_deux_premieres_entrees_de_la_fat_sont_reservees() {
    let (disque, _, _) = esp(96, 512);
    let lecteur = Lecteur::neuf(&disque.octets);
    assert_eq!(lecteur.fat(0) & 0xFF, 0xF8, "octet de support");
    assert_eq!(lecteur.fat(1), 0x0FFF_FFFF);
    assert_eq!(lecteur.fat(2), 0x0FFF_FFFF, "la racine tient en un amas");
}

#[test]
fn la_copie_du_secteur_d_amorcage_existe() {
    let (disque, _, _) = esp(96, 512);
    let bps = 512;
    assert_eq!(
        &disque.octets[..bps],
        &disque.octets[SECTEUR_AMORCE_SECOURS as usize * bps
            ..(SECTEUR_AMORCE_SECOURS as usize + 1) * bps],
        "son absence transforme une eraflure sur le premier secteur en \
         machine qui ne demarre plus"
    );
}

#[test]
fn le_secteur_d_information_porte_ses_trois_signatures() {
    let (disque, _, _) = esp(96, 512);
    let s = &disque.octets[512..1024];
    assert_eq!(u32::from_le_bytes(s[0..4].try_into().unwrap()), 0x4161_5252);
    assert_eq!(u32::from_le_bytes(s[484..488].try_into().unwrap()), 0x6141_7272);
    assert_eq!(u32::from_le_bytes(s[508..512].try_into().unwrap()), 0xAA55_0000);
}

#[test]
fn un_fichier_de_plusieurs_amas_a_une_chaine_continue() {
    let (disque, noyau, _) = esp(96, 512);
    let lecteur = Lecteur::neuf(&disque.octets);
    let k = lecteur.cherche(lecteur.racine, "kernel-x86_64").unwrap();
    let octets_amas = lecteur.spc * lecteur.bps;
    let attendus = noyau.len().div_ceil(octets_amas);
    let mut compte = 0;
    let mut amas = k.amas;
    while (2..0x0FFF_FFF7).contains(&amas) {
        compte += 1;
        amas = lecteur.fat(amas);
        assert!(compte <= attendus + 1, "chaine plus longue que le fichier");
    }
    assert_eq!(compte, attendus);
    assert_eq!(amas, 0x0FFF_FFFF, "la chaine se termine par une fin de chaine");
}

#[test]
fn deux_fichiers_ne_partagent_jamais_un_amas() {
    let (disque, _, _) = esp(96, 512);
    let lecteur = Lecteur::neuf(&disque.octets);
    let mut vus = std::collections::HashSet::new();
    let mut visite = |premier: u32| {
        let mut amas = premier;
        while (2..0x0FFF_FFF7).contains(&amas) {
            assert!(vus.insert(amas), "amas {amas} partage par deux chaines");
            amas = lecteur.fat(amas);
        }
    };
    visite(lecteur.racine);
    for t in lecteur.liste(lecteur.racine) {
        if t.amas >= 2 {
            visite(t.amas);
        }
    }
    let efi = lecteur.cherche(lecteur.racine, "EFI").unwrap();
    for t in lecteur.liste(efi.amas) {
        if t.amas >= 2 && t.nom != "." && t.nom != ".." {
            visite(t.amas);
        }
    }
}

#[test]
fn un_fichier_vide_n_occupe_aucun_amas() {
    let secteurs = 96 * 1024 * 1024 / 512;
    let mut disque = Disque::neuf(secteurs, 512);
    {
        let mut v = formate(&mut disque, 0, secteurs, "VIDE", 1).unwrap();
        let racine = v.racine();
        v.ecrit_fichier(racine, "VIDE.TXT", &[]).unwrap();
    }
    let lecteur = Lecteur::neuf(&disque.octets);
    let t = lecteur.cherche(lecteur.racine, "VIDE.TXT").unwrap();
    assert_eq!(t.taille, 0);
    assert_eq!(
        t.amas, 0,
        "un amas donne a un fichier de taille nulle ne serait jamais libere"
    );
}

#[test]
fn la_queue_du_dernier_secteur_est_remise_a_zero() {
    // Le cas qui compte est un fichier qui DEBORDE d'un secteur : le tampon
    // de travail est reutilise d'un secteur au suivant, et sans remise a zero
    // la queue du dernier secteur porte les octets du PRECEDENT.
    //
    // Un fichier plus court qu'un secteur ne montre rien -- le tampon est
    // neuf, donc nul -- et l'effacement de l'amas a l'allocation le couvre
    // aussi. C'est exactement pour cela que la premiere version de ce test
    // passait alors que la remise a zero avait ete retiree.
    let secteurs = 96 * 1024 * 1024 / 512;
    let mut disque = Disque::neuf(secteurs, 512);
    for (i, o) in disque.octets.iter_mut().enumerate() {
        *o = (i % 253) as u8;
    }
    let contenu: Vec<u8> = (0..515u32).map(|i| (i % 251) as u8 | 0x80).collect();
    {
        let mut v = formate(&mut disque, 0, secteurs, "SALE", 1).unwrap();
        let racine = v.racine();
        v.ecrit_fichier(racine, "DEBORDE.BIN", &contenu).unwrap();
        v.termine().unwrap();
    }
    let lecteur = Lecteur::neuf(&disque.octets);
    let t = lecteur.cherche(lecteur.racine, "DEBORDE.BIN").unwrap();
    assert_eq!(t.taille, 515);
    let amas = lecteur.octets_amas(t.amas);
    assert_eq!(&amas[..515], &contenu[..]);
    assert!(
        amas[515..].iter().all(|o| *o == 0),
        "la queue du dernier secteur porte les octets du secteur precedent : \
         le tampon de travail n'a pas ete remis a zero"
    );
}

#[test]
fn un_volume_formate_se_rouvre() {
    let (mut disque, _, _) = esp(96, 512);
    let volume = ouvre(&mut disque, 0).expect("relecture");
    let p = volume.parametres();
    assert_eq!(p.octets_par_secteur, 512);
    assert_eq!(p.amas_racine, 2);
    assert!(p.amas >= AMAS_MINIMUM_FAT32);
}

#[test]
fn le_volume_est_ecrit_a_son_decalage_et_pas_ailleurs() {
    // La faute qui ecrit un systeme de fichiers parfaitement forme par-dessus
    // la table de partitions.
    let decalage = 2048u64;
    let secteurs = 96 * 1024 * 1024 / 512;
    let mut disque = Disque::neuf(decalage + secteurs, 512);
    {
        let mut v = formate(&mut disque, decalage, secteurs, "DECALE", 1).unwrap();
        let racine = v.racine();
        v.ecrit_fichier(racine, "X.BIN", &[7; 10]).unwrap();
    }
    assert!(
        disque.octets[..(decalage as usize) * 512].iter().all(|o| *o == 0),
        "rien ne doit avoir ete ecrit AVANT le premier bloc de la partition"
    );
    let lecteur = Lecteur::neuf(&disque.octets[(decalage as usize) * 512..]);
    assert!(lecteur.cherche(lecteur.racine, "X.BIN").is_some());
}

// ---------------------------------------------------------------------------
// L'image, pour la verification externe
// ---------------------------------------------------------------------------

#[test]
fn ecrit_l_image_pour_fsck_et_mtools() {
    // `tools/verifie-fat32-reel.py` donne cette image a des implementations
    // qui n'ont jamais entendu parler de ce projet. Sans variable, ce test ne
    // fait rien -- il ne doit pas ecrire un fichier a l'aveugle.
    let Ok(chemin) = std::env::var("BO_FAT_IMAGE") else { return };
    let (disque, _, _) = esp(96, 512);
    std::fs::write(&chemin, &disque.octets).expect("image ecrite");
}
