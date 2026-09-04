//! Preuve hote du decodage PCI/PCIe.
//!
//! Le module de production `src/arch/x86_64/pci/decodage.rs` est inclus tel
//! quel. Il ne touche ni `0xCF8` ni `0xCFC` : c'est ce qui permet de lui donner
//! un espace de configuration FABRIQUE -- avec une liste de capacites qui
//! boucle, un BAR 64 bits, un pont mal configure -- au lieu d'attendre le
//! materiel qui produira le cas un jour.
//!
//! # Ce que l'enumeration ne voyait pas
//!
//! Elle balayait le BUS 0, lisait les BAR par mots de 32 bits, et ne regardait
//! jamais la liste de capacites. Les trois sont sans consequence sur i440fx --
//! tout y est sur le bus unique, et rien n'a de BAR 64 bits. Sur Q35, la
//! plateforme de reference moderne, les trois sont faux :
//!
//!   * les peripheriques sont derriere des PONTS RACINE PCIe ;
//!   * le BAR0 d'un NVMe est un BAR memoire 64 BITS ;
//!   * MSI-X vit dans la liste de capacites.

#![allow(dead_code)]

#[path = "../../src/arch/x86_64/pci/decodage.rs"]
mod decodage;

use decodage::*;

// ---------------------------------------------------------------------------
// LES BAR : le cas 64 bits, celui d'un NVMe.
// ---------------------------------------------------------------------------

#[test]
fn un_bar_64_bits_compose_ses_deux_moities() {
    // Type 0b10 = memoire 64 bits, prefetchable.
    let bas = 0xFEB0_0000u32 | (0b10 << 1) | 0x8;
    let haut = 0x0000_0001u32;
    assert_eq!(
        decode_bar(bas, haut),
        Bar::Memoire64 { adresse: 0x1_FEB0_0000, prefetch: true },
        "lu sur 32 bits, ce BAR donnerait 0xFEB00000 : une adresse plausible \
         qui pointe ailleurs, ce qui est pire qu'une erreur"
    );
    assert_eq!(decode_bar(bas, haut).adresse(), 0x1_FEB0_0000);
    assert!(decode_bar(bas, haut).double(), "il occupe DEUX emplacements de BAR");
}

#[test]
fn un_bar_32_bits_ignore_le_mot_haut() {
    let bas = 0xFEBC_0000u32;
    // Le mot haut appartient au BAR SUIVANT : le lire ici fabriquerait une
    // adresse absurde.
    assert_eq!(
        decode_bar(bas, 0xDEAD_BEEF),
        Bar::Memoire32 { adresse: 0xFEBC_0000, prefetch: false }
    );
    assert!(!decode_bar(bas, 0).double());
}

#[test]
fn un_bar_d_entree_sortie_se_reconnait() {
    assert_eq!(decode_bar(0xC001, 0), Bar::Port(0xC000));
}

/// Un BAR nul est ABSENT, pas une adresse zero « valide » : ecrire a l'adresse
/// physique zero se manifesterait bien plus tard, et ailleurs.
#[test]
fn un_bar_nul_est_absent() {
    assert_eq!(decode_bar(0, 0), Bar::Absent);
    assert_eq!(decode_bar(0, 0xFFFF_FFFF), Bar::Absent);
    assert_eq!(Bar::Absent.adresse(), 0);
}

// ---------------------------------------------------------------------------
// LA LISTE DE CAPACITES.
// ---------------------------------------------------------------------------

/// Un espace de configuration fabrique : un tableau de mots.
struct Config {
    mots: [u32; 64],
}

impl Config {
    fn neuf() -> Self {
        Self { mots: [0; 64] }
    }
    fn pose(&mut self, decalage: u8, valeur: u32) {
        self.mots[(decalage / 4) as usize] = valeur;
    }
    fn lit(&self, decalage: u8) -> u32 {
        self.mots[(decalage / 4) as usize]
    }
}

/// Un mot de capacite : identifiant en bas, pointeur suivant au-dessus.
fn capacite(identifiant: u8, suivant: u8) -> u32 {
    identifiant as u32 | ((suivant as u32) << 8)
}

const STATUT_AVEC_CAPACITES: u32 = (STATUT_CAPACITES as u32) << 16;

#[test]
fn la_liste_de_capacites_se_parcourt() {
    let mut config = Config::neuf();
    config.pose(OFFSET_CAPACITES, 0x40);
    config.pose(0x40, capacite(CAP_PCIE, 0x60));
    config.pose(0x60, capacite(CAP_MSI, 0x80));
    config.pose(0x80, capacite(CAP_MSIX, 0x00));

    let mut sortie = [Capacite { identifiant: 0, decalage: 0 }; 8];
    let trouvees = capacites(
        STATUT_AVEC_CAPACITES,
        |decalage| config.lit(decalage),
        &mut sortie,
    );
    assert_eq!(trouvees, 3);
    assert_eq!(sortie[0], Capacite { identifiant: CAP_PCIE, decalage: 0x40 });
    assert_eq!(sortie[1], Capacite { identifiant: CAP_MSI, decalage: 0x60 });
    assert_eq!(sortie[2], Capacite { identifiant: CAP_MSIX, decalage: 0x80 });

    assert_eq!(
        trouve_capacite(&sortie[..trouvees], CAP_MSIX).map(|c| c.decalage),
        Some(0x80)
    );
    assert_eq!(trouve_capacite(&sortie[..trouvees], 0x99), None);
}

/// Sans le bit d'etat, il n'y a PAS de liste : la lire quand meme
/// interpreterait des octets quelconques comme une chaine de capacites.
#[test]
fn sans_le_bit_d_etat_il_n_y_a_pas_de_liste() {
    let mut config = Config::neuf();
    config.pose(OFFSET_CAPACITES, 0x40);
    config.pose(0x40, capacite(CAP_MSIX, 0));
    let mut sortie = [Capacite { identifiant: 0, decalage: 0 }; 8];
    assert_eq!(capacites(0, |d| config.lit(d), &mut sortie), 0);
}

/// LE CAS QUI FIGE UN PARCOURS NAIF : une capacite chainee sur elle-meme.
///
/// Un materiel abime -- ou un peripherique hostile branche a chaud -- suffit a
/// le produire, et cela se manifesterait comme un boot qui ne finit jamais,
/// sans console.
#[test]
fn une_liste_qui_boucle_ne_fige_pas_le_parcours() {
    let mut config = Config::neuf();
    config.pose(OFFSET_CAPACITES, 0x40);
    config.pose(0x40, capacite(CAP_MSI, 0x40)); // chainee sur elle-meme

    let mut sortie = [Capacite { identifiant: 0, decalage: 0 }; 8];
    let trouvees = capacites(STATUT_AVEC_CAPACITES, |d| config.lit(d), &mut sortie);
    assert_eq!(trouvees, 1, "la capacite est lue une fois, et une seule");
}

#[test]
fn un_cycle_plus_long_est_aussi_ferme() {
    let mut config = Config::neuf();
    config.pose(OFFSET_CAPACITES, 0x40);
    config.pose(0x40, capacite(CAP_PCIE, 0x50));
    config.pose(0x50, capacite(CAP_MSI, 0x60));
    config.pose(0x60, capacite(CAP_MSIX, 0x40)); // retour au debut

    let mut sortie = [Capacite { identifiant: 0, decalage: 0 }; 16];
    let trouvees = capacites(STATUT_AVEC_CAPACITES, |d| config.lit(d), &mut sortie);
    assert_eq!(trouvees, 3, "chaque capacite du cycle est lue exactement une fois");
}

/// Un pointeur sous 0x40 pointe dans l'EN-TETE standard, pas dans une
/// capacite. Le suivre lirait le vendor/device comme un identifiant.
#[test]
fn un_pointeur_dans_l_entete_est_refuse() {
    let mut config = Config::neuf();
    config.pose(OFFSET_CAPACITES, 0x10);
    let mut sortie = [Capacite { identifiant: 0, decalage: 0 }; 8];
    assert_eq!(capacites(STATUT_AVEC_CAPACITES, |d| config.lit(d), &mut sortie), 0);
}

/// Le parcours ne deborde jamais du tampon qu'on lui donne.
#[test]
fn le_parcours_respecte_la_taille_de_sortie() {
    let mut config = Config::neuf();
    config.pose(OFFSET_CAPACITES, 0x40);
    config.pose(0x40, capacite(CAP_PCIE, 0x50));
    config.pose(0x50, capacite(CAP_MSI, 0x60));
    config.pose(0x60, capacite(CAP_MSIX, 0x00));

    let mut sortie = [Capacite { identifiant: 0, decalage: 0 }; 2];
    assert_eq!(capacites(STATUT_AVEC_CAPACITES, |d| config.lit(d), &mut sortie), 2);
}

// ---------------------------------------------------------------------------
// LES VECTEURS : deux encodages, deux pieges.
// ---------------------------------------------------------------------------

/// « Table Size » est un nombre de vecteurs MOINS UN. Le lire tel quel donne
/// toujours un vecteur de trop peu, et la derniere file d'un NVMe n'aurait
/// jamais d'interruption.
#[test]
fn msix_annonce_ses_vecteurs_moins_un() {
    assert_eq!(vecteurs_msix(0x0000), 1, "zero signifie UN vecteur");
    assert_eq!(vecteurs_msix(0x0003), 4);
    assert_eq!(vecteurs_msix(0x07FF), 2048, "le maximum du champ");
    // Les bits hauts -- activation, masquage global -- ne comptent pas.
    assert_eq!(vecteurs_msix(0x8003), 4);
}

/// Le champ MSI est un LOGARITHME. Le lire comme un compte donnerait cinq
/// vecteurs pour un peripherique qui en demande trente-deux.
#[test]
fn msi_annonce_un_logarithme() {
    assert_eq!(vecteurs_msi(0b0000), 1);
    assert_eq!(vecteurs_msi(0b0010), 2);
    assert_eq!(vecteurs_msi(0b0100), 4);
    assert_eq!(vecteurs_msi(0b1010), 32);
    // Une valeur reservee ne doit pas produire un decalage absurde.
    assert_eq!(vecteurs_msi(0b1110), 32);
}

// ---------------------------------------------------------------------------
// LES PONTS : ce qui rend Q35 visible.
// ---------------------------------------------------------------------------

#[test]
fn un_pont_se_reconnait_a_sa_classe_et_a_son_entete() {
    assert!(est_pont(CLASSE_PONT, SOUS_CLASSE_PONT_PCI, 0x01));
    assert!(est_pont(CLASSE_PONT, SOUS_CLASSE_PONT_PCI, 0x81), "multifonction");
    // Un peripherique ordinaire de la meme classe n'est pas un pont PCI-PCI.
    assert!(!est_pont(CLASSE_PONT, 0x00, 0x00), "pont hote, en-tete de type 0");
    assert!(!est_pont(0x01, SOUS_CLASSE_PONT_PCI, 0x01));
    assert!(!est_pont(CLASSE_PONT, SOUS_CLASSE_PONT_PCI, 0x00));
}

#[test]
fn le_bus_derriere_un_pont_se_lit_dans_son_mot_0x18() {
    // Mot 0x18 d'un en-tete de type 1 : bus primaire en 7:0, SECONDAIRE en
    // 15:8, subordonne en 23:16, temporisation en 31:24.
    let mot = |primaire: u8, secondaire: u8, subordonne: u8| -> u32 {
        primaire as u32 | ((secondaire as u32) << 8) | ((subordonne as u32) << 16)
    };
    assert_eq!(bus_secondaire(mot(0, 1, 4)), 1);
    assert_eq!(bus_subordonne(mot(0, 1, 4)), 4);
    // Une topologie Q35 typique : plusieurs ports racine, chacun son bus.
    assert_eq!(bus_secondaire(mot(0, 3, 3)), 3);
    assert_eq!(bus_subordonne(mot(0, 3, 3)), 3);
    // La temporisation en 31:24 ne doit pas deborder sur le subordonne.
    assert_eq!(bus_subordonne(mot(0, 1, 4) | (0xFF << 24)), 4);
}

#[test]
fn un_nvme_se_reconnait_a_ses_trois_octets() {
    assert!(est_nvme(CLASSE_STOCKAGE, SOUS_CLASSE_NVME, PROGIF_NVME));
    // Un autre controleur de stockage de la meme sous-classe n'est pas un NVMe.
    assert!(!est_nvme(CLASSE_STOCKAGE, SOUS_CLASSE_NVME, 0x00));
    // ATA : meme classe, autre sous-classe.
    assert!(!est_nvme(CLASSE_STOCKAGE, 0x01, PROGIF_NVME));
    assert!(!est_nvme(0x02, SOUS_CLASSE_NVME, PROGIF_NVME));
}

/// Sans le bit multifonction, sept acces de configuration sur huit sont
/// inutiles a chaque emplacement.
#[test]
fn le_bit_multifonction_evite_sept_acces_sur_huit() {
    assert!(multifonction(0x80));
    assert!(multifonction(0x81));
    assert!(!multifonction(0x00));
    assert!(!multifonction(0x01));
}
