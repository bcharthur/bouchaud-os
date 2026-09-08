//! Fuzzing des decodeurs qui lisent des octets que le noyau ne controle pas.
//!
//! # Ce qui distingue ces decodeurs du reste du noyau
//!
//! Un decodeur de table de partitions lit un disque. Un decodeur de
//! descripteur USB lit ce qu'un peripherique lui envoie. Un decodeur de
//! rapport HID lit ce qu'un clavier -- ou ce qui se PRESENTE comme un clavier
//! -- transmet. Dans les trois cas, les octets viennent de l'exterieur, et
//! rien ne garantit qu'ils ressemblent a ce que la specification decrit.
//!
//! Le reste du noyau se trompe sur ses propres donnees ; ceux-ci se trompent
//! sur celles d'un autre. C'est la difference entre un defaut et une surface
//! d'attaque : brancher une cle USB fabriquee ne demande aucun privilege.
//!
//! # Ce que ces tests cherchent, et ce qu'ils ne cherchent pas
//!
//! Ils ne verifient pas que le decodage est JUSTE -- c'est le travail des
//! suites dediees, qui donnent des entrees connues et comparent des sorties
//! connues. Ils verifient qu'aucune entree, si absurde soit-elle, ne fait
//! sortir le decodeur de ses bornes :
//!
//!   * aucune panique -- un `unwrap`, un depassement de tranche, une
//!     soustraction qui passe sous zero ;
//!   * aucune valeur rendue qui designerait de la memoire ou du disque hors
//!     de ce qu'on lui a donne ;
//!   * aucune boucle qui ne se termine pas.
//!
//! Le troisieme est le plus dangereux : une boucle infinie dans un decodeur
//! de descripteur fige le noyau au branchement d'un peripherique, et rien ne
//! le distingue d'une machine qui a plante.
//!
//! # Deterministe, et reproductible
//!
//! La graine vient de `BOUCHAUD_PROP_SEED`. La CI en passe soixante-quatre ;
//! un echec se rejoue avec la meme graine et donne exactement la meme entree.

#![allow(dead_code)]

extern crate alloc;

#[path = "../../src/fs/gpt.rs"]
mod gpt;

#[path = "../../src/fs/fat32.rs"]
mod fat32;

#[path = "../../src/drivers/block/nvme/decodage.rs"]
mod nvme;

#[path = "../../src/drivers/usb/hid/decodage.rs"]
mod hid;

#[path = "../../src/kernel/memory/compagnon.rs"]
mod compagnon;

/// Xorshift, comme le reste des tests de propriete du depot.
struct Alea(u64);

impl Alea {
    fn neuf(graine: u64) -> Self {
        Self(graine.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn suivant(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn borne(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.suivant() % n as u64) as usize }
    }
    fn octet(&mut self) -> u8 {
        self.suivant() as u8
    }
    /// Un tampon d'octets quelconques.
    fn tampon(&mut self, taille: usize) -> Vec<u8> {
        (0..taille).map(|_| self.octet()).collect()
    }
    /// Un tampon avec des valeurs de BORD : zero, un, maximum. Les octets
    /// vraiment aleatoires touchent rarement les cas limites d'un champ.
    fn tampon_de_bord(&mut self, taille: usize) -> Vec<u8> {
        (0..taille)
            .map(|_| match self.borne(6) {
                0 => 0x00,
                1 => 0xFF,
                2 => 0x01,
                3 => 0x7F,
                4 => 0x80,
                _ => self.octet(),
            })
            .collect()
    }
}

fn graine() -> u64 {
    std::env::var("BOUCHAUD_PROP_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
}

/// Combien de cas par propriete. Assez pour trouver, assez court pour la CI.
const TOURS: usize = 4000;

// ---------------------------------------------------------------------------
// GPT : un disque que n'importe qui peut brancher
// ---------------------------------------------------------------------------

#[test]
fn un_entete_gpt_quelconque_ne_panique_jamais() {
    let mut a = Alea::neuf(graine());
    for _ in 0..TOURS {
        let taille = 8 + a.borne(600);
        let mut bloc = a.tampon_de_bord(taille);
        // Une entree sur deux porte la bonne signature : sans elle, le
        // decodeur refuse au premier octet et on ne teste jamais la suite.
        if a.borne(2) == 0 && bloc.len() >= 8 {
            bloc[0..8].copy_from_slice(gpt::SIGNATURE);
        }
        let _ = gpt::decode_entete(&bloc);
    }
}

#[test]
fn un_entete_gpt_accepte_a_forcement_une_somme_juste() {
    // La propriete qui compte : si `decode_entete` rend quelque chose, la
    // somme de controle a ete verifiee. Un decodeur qui accepterait un en-tete
    // dont la somme est fausse ferait ecrire ensuite sur des bornes inventees.
    let mut a = Alea::neuf(graine() ^ 0xA1);
    for _ in 0..TOURS {
        let mut bloc = a.tampon_de_bord(512);
        bloc[0..8].copy_from_slice(gpt::SIGNATURE);
        // Taille d'en-tete plausible, sinon tout est refuse tres tot.
        let taille = 92u32;
        bloc[12..16].copy_from_slice(&taille.to_le_bytes());
        if gpt::decode_entete(&bloc).is_some() {
            let annoncee = u32::from_le_bytes(bloc[16..20].try_into().unwrap());
            let mut copie = bloc[..taille as usize].to_vec();
            copie[16..20].fill(0);
            assert_eq!(
                gpt::crc32(&copie),
                annoncee,
                "un en-tete a ete accepte sans que sa somme corresponde"
            );
        }
    }
}

#[test]
fn une_entree_de_partition_quelconque_ne_panique_jamais() {
    let mut a = Alea::neuf(graine() ^ 0xA2);
    for _ in 0..TOURS {
        let taille = a.borne(200);
        let octets = a.tampon_de_bord(taille);
        let _ = gpt::Partition::decode(&octets);
    }
}

#[test]
fn une_partition_decodee_fait_l_aller_retour_ou_est_refusee() {
    // Si une entree est acceptee, la reencoder doit redonner les memes champs.
    // Un decodage qui perdrait un bit ferait ecrire ailleurs qu'ou l'utilisateur
    // a demande.
    let mut a = Alea::neuf(graine() ^ 0xA3);
    for _ in 0..TOURS {
        let octets = a.tampon_de_bord(gpt::TAILLE_ENTREE);
        if let Some(p) = gpt::Partition::decode(&octets) {
            let reencode = p.encode();
            let relu = gpt::Partition::decode(&reencode).expect("reencode decodable");
            assert_eq!(relu.premier, p.premier);
            assert_eq!(relu.dernier, p.dernier);
            assert_eq!(relu.type_guid, p.type_guid);
            assert_eq!(relu.unique, p.unique);
        }
    }
}

#[test]
fn une_disposition_gpt_est_toujours_coherente() {
    let mut a = Alea::neuf(graine() ^ 0xA4);
    for _ in 0..TOURS {
        let blocs = a.suivant() % 100_000;
        let taille_bloc = [512usize, 1024, 2048, 4096][a.borne(4)];
        if let Some(d) = gpt::disposition(blocs, taille_bloc) {
            assert!(d.premier_utilisable <= d.dernier_utilisable);
            assert!(
                d.dernier_utilisable < d.tableau_secours,
                "la zone utilisable mord sur le tableau de secours"
            );
            assert!(d.entete_secours < blocs);
            assert!(d.tableau_primaire < d.premier_utilisable);
            assert!(d.tableau_secours + d.blocs_tableau <= d.entete_secours);
        }
    }
}

#[test]
fn le_mbr_de_protection_est_toujours_bien_forme() {
    let mut a = Alea::neuf(graine() ^ 0xA5);
    for _ in 0..500 {
        let blocs = a.suivant();
        let taille_bloc = [512usize, 4096][a.borne(2)];
        let mbr = gpt::mbr_protecteur(blocs, taille_bloc);
        assert_eq!(mbr.len(), taille_bloc);
        assert_eq!(&mbr[510..512], &[0x55, 0xAA]);
        assert_eq!(mbr[450], 0xEE);
    }
}

// ---------------------------------------------------------------------------
// FAT32 : le secteur d'amorcage d'une partition quelconque
// ---------------------------------------------------------------------------

/// Un disque en memoire qui ne rend que ce qu'on lui a donne.
struct Papier {
    taille_bloc: usize,
    octets: Vec<u8>,
}

impl gpt::Support for Papier {
    fn taille_bloc(&self) -> usize {
        self.taille_bloc
    }
    fn blocs(&self) -> u64 {
        (self.octets.len() / self.taille_bloc) as u64
    }
    fn lit(&mut self, lba: u64, sortie: &mut [u8]) -> bool {
        let d = (lba as usize).saturating_mul(self.taille_bloc);
        if d + sortie.len() > self.octets.len() {
            return false;
        }
        sortie.copy_from_slice(&self.octets[d..d + sortie.len()]);
        true
    }
    fn ecrit(&mut self, lba: u64, donnees: &[u8]) -> bool {
        let d = (lba as usize).saturating_mul(self.taille_bloc);
        if d + donnees.len() > self.octets.len() {
            return false;
        }
        self.octets[d..d + donnees.len()].copy_from_slice(donnees);
        true
    }
}

#[test]
fn ouvrir_un_volume_fat_quelconque_ne_panique_jamais() {
    let mut a = Alea::neuf(graine() ^ 0xB1);
    for _ in 0..TOURS {
        let mut disque = Papier { taille_bloc: 512, octets: a.tampon_de_bord(512 * 8) };
        let _ = fat32::ouvre(&mut disque, 0);
    }
}

#[test]
fn un_volume_fat_accepte_a_sa_zone_de_donnees_dans_le_volume() {
    // La propriete qui compte : si `ouvre` accepte, la zone de donnees
    // commence AVANT la fin du volume. Sinon le premier acces a un amas lirait
    // hors de la partition -- c'est-a-dire chez le voisin.
    let mut a = Alea::neuf(graine() ^ 0xB2);
    let mut acceptes = 0usize;
    for _ in 0..TOURS {
        let mut octets = a.tampon_de_bord(512 * 8);
        // On force des champs plausibles pour atteindre l'interieur.
        octets[11..13].copy_from_slice(&512u16.to_le_bytes());
        octets[13] = 1 << a.borne(7);
        let mut disque = Papier { taille_bloc: 512, octets };
        if let Ok(volume) = fat32::ouvre(&mut disque, 0) {
            let p = volume.parametres();
            acceptes += 1;
            assert!(
                p.premier_secteur_donnees < p.secteurs_total,
                "zone de donnees hors du volume : {p:?}"
            );
            assert!(p.secteurs_par_amas > 0);
            assert!(p.fats > 0);
            assert!(p.amas > 0, "un volume sans amas serait accepte");
            // La zone de donnees doit tenir dans ce qui reste.
            let fin = p.premier_secteur_donnees as u64
                + p.amas as u64 * p.secteurs_par_amas as u64;
            assert!(fin <= p.secteurs_total as u64, "les amas debordent : {p:?}");
        }
    }
    assert!(acceptes > 0, "aucun volume accepte : le fuzzing n'atteint pas l'interieur");
}

#[test]
fn le_calcul_de_geometrie_fat_ne_panique_jamais() {
    let mut a = Alea::neuf(graine() ^ 0xB3);
    for _ in 0..TOURS {
        let secteurs = (a.suivant() % 5_000_000) as u32;
        let bps = [512u32, 1024, 2048, 4096, 511, 0, 7][a.borne(7)];
        let spc = (a.suivant() % 300) as u32;
        if let Some(p) = fat32::calcule(secteurs, bps, spc) {
            assert!(p.secteurs_par_fat > 0);
            assert!(p.premier_secteur_donnees < p.secteurs_total);
            assert!(p.amas > 0);
            // Le point fixe doit tenir : la FAT decrit tous les amas.
            let par_secteur = p.octets_par_secteur / 4;
            assert!(
                p.secteurs_par_fat * par_secteur >= p.amas + 2,
                "la FAT ne decrit pas tous ses amas : {p:?}"
            );
        }
    }
}

#[test]
fn un_nom_de_fichier_quelconque_produit_un_nom_court_valide() {
    let mut a = Alea::neuf(graine() ^ 0xB4);
    for _ in 0..TOURS {
        let taille = a.borne(80);
        let brut: String = (0..taille)
            .map(|_| char::from(a.octet().max(1)))
            .collect();
        let court = fat32::nom_court(&brut, (a.suivant() % 1000) as u32);
        // Onze octets, jamais de zero, jamais de separateur : un nom court qui
        // porterait un `/` designerait un autre repertoire.
        assert_eq!(court.len(), 11);
        for octet in court.iter() {
            assert!(*octet != 0, "un nom court ne porte pas d'octet nul");
            assert!(*octet != b'/' && *octet != b'\\');
        }
        // Et les entrees de nom long qui vont avec restent bornees.
        let entrees = fat32::entrees_nom_long(&brut, &court);
        assert!(entrees.len() <= brut.chars().count().div_ceil(13).max(1) + 1);
        for e in &entrees {
            assert_eq!(e[11], 0x0F);
            assert_eq!(e[13], fat32::somme_nom_court(&court));
        }
    }
}

// ---------------------------------------------------------------------------
// NVMe : ce que le controleur repond
// ---------------------------------------------------------------------------

#[test]
fn un_identify_namespace_quelconque_ne_panique_jamais() {
    let mut a = Alea::neuf(graine() ^ 0xC1);
    for _ in 0..TOURS {
        let taille = a.borne(4200);
        let octets = a.tampon_de_bord(taille);
        if let Some(format) = nvme::format_bloc(&octets) {
            // La propriete qui compte : une taille de bloc acceptee est une
            // puissance de deux d'au moins 512. Un pilote qui recevrait autre
            // chose calculerait des adresses fausses en silence.
            assert!(
                format.taille_bloc.is_power_of_two(),
                "taille de bloc non puissance de deux : {}",
                format.taille_bloc
            );
            assert!(format.taille_bloc >= 512);
            assert!(format.taille_bloc <= 1 << 24);
        }
        let _ = nvme::blocs_du_namespace(&octets);
        let _ = nvme::transfert_max_octets(&octets, 4096);
    }
}

#[test]
fn la_liste_des_namespaces_reste_dans_sa_sortie() {
    let mut a = Alea::neuf(graine() ^ 0xC2);
    for _ in 0..TOURS {
        let taille = a.borne(300);
        let octets = a.tampon_de_bord(taille);
        let mut sortie = [0u32; 8];
        let n = nvme::namespaces_actifs(&octets, &mut sortie);
        assert!(n <= sortie.len(), "debordement de la sortie");
        for identifiant in sortie.iter().take(n) {
            assert!(*identifiant != 0, "un namespace nul a ete rendu");
        }
    }
}

#[test]
fn un_plan_prp_couvre_toujours_le_transfert() {
    // La propriete qui compte : la somme des pages du plan couvre au moins les
    // octets demandes. Un plan qui en couvrirait moins ferait ecrire au
    // controleur au-dela de ce qu'on lui a donne.
    let mut a = Alea::neuf(graine() ^ 0xC3);
    for _ in 0..TOURS {
        let physique = a.suivant() & 0x0000_FFFF_FFFF_FFFF;
        let octets = a.borne(1 << 20) + 1;
        let page = 4096usize;
        let plan = nvme::plan_prp(physique, octets, page);
        let decalage = (physique as usize) & (page - 1);
        let premiere = nvme::octets_dans_la_premiere_page(decalage, octets, page);
        let couvert = match plan {
            nvme::Prp::UnePage { .. } => premiere,
            nvme::Prp::DeuxPages { .. } => premiere + page,
            nvme::Prp::Liste { entrees, .. } => premiere + entrees * page,
        };
        assert!(
            couvert >= octets,
            "plan {plan:?} couvre {couvert} octets pour un transfert de {octets}"
        );
        // Et il ne surcouvre jamais d'une page entiere : sinon on immobilise
        // une page de liste pour rien.
        assert!(couvert < octets + page);
    }
}

#[test]
fn un_achevement_quelconque_se_decode() {
    let mut a = Alea::neuf(graine() ^ 0xC4);
    for _ in 0..TOURS {
        let cqe = [
            a.suivant() as u32,
            a.suivant() as u32,
            a.suivant() as u32,
            a.suivant() as u32,
        ];
        let d = nvme::decode_achevement(cqe);
        // Une reussite exige un statut ENTIEREMENT nul : c'est ce qui evite de
        // prendre une erreur generique pour un succes.
        if d.reussi() {
            assert_eq!(d.type_statut, 0);
            assert_eq!(d.code_statut, 0);
        }
    }
}

// ---------------------------------------------------------------------------
// HID : ce qu'un peripherique se disant clavier envoie
// ---------------------------------------------------------------------------

#[test]
fn un_rapport_de_clavier_quelconque_reste_dans_ses_bornes() {
    let mut a = Alea::neuf(graine() ^ 0xD1);
    let mut etat = hid::EtatClavier::default();
    for _ in 0..TOURS {
        let taille = a.borne(40);
        let donnees = a.tampon_de_bord(taille);
        let identifiant = if a.borne(3) == 0 { a.octet() } else { 0 };
        let mut sortie = [hid::Evenement { code: 0, etendu: false, appui: false };
            hid::EVENEMENTS_MAX];
        if let Some(n) = hid::evenements_clavier(&mut etat, identifiant, &donnees, &mut sortie) {
            assert!(
                n <= hid::EVENEMENTS_MAX,
                "un rapport a produit {n} evenements, la sortie en tient {}",
                hid::EVENEMENTS_MAX
            );
            for e in sortie.iter().take(n) {
                assert!(e.code != 0, "un code de touche nul a ete emis");
            }
        }
    }
}

#[test]
fn une_souris_quelconque_ne_rend_que_trois_boutons() {
    let mut a = Alea::neuf(graine() ^ 0xD2);
    for _ in 0..TOURS {
        let taille = a.borne(20);
        let donnees = a.tampon_de_bord(taille);
        let identifiant = if a.borne(3) == 0 { a.octet() } else { 0 };
        if let Some(s) = hid::decode_souris(identifiant, &donnees) {
            assert!(
                s.boutons <= 0x07,
                "les boutons lateraux ne sont plus masques : {:#04x}",
                s.boutons
            );
        }
    }
}

#[test]
fn la_table_des_touches_ne_rend_jamais_un_code_nul() {
    // Un code PS/2 nul n'est pas une touche : l'emettre ferait taper un
    // caractere que personne n'a demande.
    for usage in 0..=255u8 {
        if let Some((code, _)) = hid::usage_ps2(usage) {
            assert!(code != 0, "l'usage {usage:#04x} rend le code nul");
            assert!(code < 0x80, "un code PS/2 a le bit 7 libre pour le relachement");
        }
    }
    for bit in 0..=255u8 {
        if let Some((code, _)) = hid::modificateur_ps2(bit) {
            assert!(code != 0 && code < 0x80);
        }
    }
}

// ---------------------------------------------------------------------------
// Compagnon : des suites d'operations quelconques
// ---------------------------------------------------------------------------

struct Sable {
    liens: Vec<usize>,
}

impl compagnon::Terrain for Sable {
    fn lien(&self, bloc: usize) -> usize {
        self.liens[bloc]
    }
    fn pose_lien(&mut self, bloc: usize, suivant: usize) {
        self.liens[bloc] = suivant;
    }
}

#[test]
fn l_allocateur_compagnon_ne_perd_ni_ne_double_jamais_de_memoire() {
    let mut a = Alea::neuf(graine() ^ 0xE1);
    for _ in 0..40 {
        let blocs = 8 + a.borne(500);
        let mut bitmap = vec![0u64; compagnon::mots_bitmap(blocs)];
        let mut terrain = Sable { liens: vec![compagnon::AUCUN; blocs] };
        let mut allocateur = compagnon::Compagnon::neuf(blocs, &mut bitmap).unwrap();

        // Alimentation par blocs alignes.
        let mut position = 0usize;
        while position < blocs {
            let mut ordre = compagnon::ORDRES - 1;
            loop {
                let taille = 1usize << ordre;
                if compagnon::aligne(position, ordre) && position + taille <= blocs {
                    break;
                }
                if ordre == 0 {
                    break;
                }
                ordre -= 1;
            }
            assert!(allocateur.rend(&mut terrain, position, ordre));
            position += 1usize << ordre;
        }
        assert_eq!(allocateur.disponibles(), blocs);

        let mut tenus: Vec<(usize, usize)> = Vec::new();
        let mut occupe = vec![false; blocs];
        for _ in 0..600 {
            if a.borne(3) != 0 || tenus.is_empty() {
                let ordre = a.borne(compagnon::ORDRES);
                if let Some(bloc) = allocateur.prend(&mut terrain, ordre) {
                    assert!(compagnon::aligne(bloc, ordre));
                    assert!(bloc + (1usize << ordre) <= blocs, "allocation hors zone");
                    for i in 0..(1usize << ordre) {
                        assert!(!occupe[bloc + i], "bloc {} rendu deux fois", bloc + i);
                        occupe[bloc + i] = true;
                    }
                    tenus.push((bloc, ordre));
                }
            } else {
                let index = a.borne(tenus.len());
                let (bloc, ordre) = tenus.swap_remove(index);
                for i in 0..(1usize << ordre) {
                    occupe[bloc + i] = false;
                }
                assert!(allocateur.rend(&mut terrain, bloc, ordre));
            }
            let tenu: usize = tenus.iter().map(|(_, o)| 1usize << o).sum();
            assert_eq!(
                allocateur.disponibles() + tenu,
                blocs,
                "la somme du libre et du tenu a quitte la zone"
            );
        }
        for (bloc, ordre) in tenus {
            assert!(allocateur.rend(&mut terrain, bloc, ordre));
        }
        assert_eq!(allocateur.disponibles(), blocs, "de la memoire a ete perdue");
    }
}
