//! Poser Bouchaud OS sur le disque interne, et ne plus dependre de la cle.
//!
//! # Ce qui manquait, et pourquoi ce n'etait pas un detail
//!
//! Le systeme demarrait en LIVE : une cle USB, un runtime en RAM, et rien qui
//! survive a une coupure. Ce n'etait pas un choix, c'etait une consequence --
//! le NVMe n'avait pas de pilote, donc il n'existait aucun peripherique
//! inscriptible sur lequel poser quoi que ce soit.
//!
//! # D'ou viennent les octets qu'on ecrit
//!
//! Pas de la cle. Le systeme vivant ne peut pas se recopier depuis elle : il
//! n'y a aucun pilote de stockage de masse USB, et le micrologiciel a rendu la
//! main depuis longtemps. Ce que la machine A, c'est son ARCHIVE, chargee en
//! memoire par le chargeur d'amorcage.
//!
//! La construction depose donc dans cette archive, sous `/install/`, les
//! fichiers dont l'ESP a besoin : le chargeur, le noyau, sa configuration.
//! L'installateur les y lit et les ecrit sur le disque. C'est exactement ce
//! que fait un installateur vivant qui porte l'image du systeme qu'il pose --
//! la difference etant qu'ici l'image et le systeme vivant sont la meme chose.
//!
//! Le ramdisk, lui, n'est pas relu depuis l'archive : il EST l'archive, et le
//! noyau en a l'adresse. Le systeme installe recoit donc, octet pour octet,
//! l'archive qui vient de tourner.
//!
//! # Ce que l'installation ne fait jamais
//!
//! Elle n'ecrit pas sans qu'on le demande. Un systeme live qui partitionne le
//! disque interne au demarrage detruirait la machine sur laquelle on voulait
//! juste l'essayer -- et c'est irreversible. L'installation est donc une
//! commande, jamais une etape d'amorcage.
//!
//! Elle refuse aussi un disque qui porte deja des partitions inconnues. Un
//! utilisateur qui a Windows sur ce disque doit se voir refuser l'installation
//! avec une phrase, pas decouvrir apres coup que ses partitions ont disparu.
//! `--ecrase` existe pour le dire explicitement.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::drivers::bloc::{self, Achevement, Descripteur, Genre, PiloteBloc, Requete, Volume};
use crate::drivers::nvme;
use crate::fs::fat32;
use crate::fs::gpt::{self, Partition, Support};

include!("installation/disposition.rs");

/// Ou l'archive porte les fichiers d'amorcage a recopier.
pub const DOSSIER_CHARGE: &str = "/install";

/// Le nom que le micrologiciel cherche, dans l'ordre ou il le cherche.
pub const CHEMIN_CHARGEUR: &str = "EFI/BOOT/BOOTX64.EFI";

/// Ce qui peut echouer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Erreur {
    DisqueAbsent,
    DisqueTropPetit,
    /// Le disque porte des partitions qu'on n'a pas ecrites.
    DisqueOccupe,
    ChargeIntrouvable,
    TableRefusee,
    FormatageRefuse,
    EcritureRefusee,
    ArchiveAbsente,
}

impl Erreur {
    pub fn phrase(self) -> &'static str {
        match self {
            Erreur::DisqueAbsent => "aucun disque interne : le pilote NVMe n'a pas demarre",
            Erreur::DisqueTropPetit => "disque trop petit pour porter une ESP et un systeme",
            Erreur::DisqueOccupe => {
                "le disque porte deja des partitions ; utilisez --ecrase pour les detruire"
            }
            Erreur::ChargeIntrouvable => {
                "/install est absent de l'archive : cette image ne sait pas s'installer"
            }
            Erreur::TableRefusee => "la table de partitions a ete refusee",
            Erreur::FormatageRefuse => "le formatage de la partition EFI a echoue",
            Erreur::EcritureRefusee => "le disque a refuse une ecriture",
            Erreur::ArchiveAbsente => "l'archive n'est pas en memoire : rien a recopier",
        }
    }
}

/// Ce que l'installation a fait.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rapport {
    pub esp_premier: u64,
    pub esp_blocs: u64,
    pub systeme_premier: u64,
    pub systeme_blocs: u64,
    pub fichiers: usize,
    pub octets: u64,
    /// Le disque a-t-il confirme une vraie barriere d'ecriture ?
    pub barriere_reelle: bool,
}

// ---------------------------------------------------------------------------
// Le disque interne, vu comme un support
// ---------------------------------------------------------------------------

/// Le disque interne entier.
struct DisqueInterne {
    blocs: u64,
    taille_bloc: usize,
}

impl DisqueInterne {
    fn ouvre() -> Option<Self> {
        let (blocs, taille_bloc) = nvme::geometrie()?;
        Some(Self { blocs, taille_bloc })
    }
}

impl Support for DisqueInterne {
    fn taille_bloc(&self) -> usize {
        self.taille_bloc
    }
    fn blocs(&self) -> u64 {
        self.blocs
    }
    fn lit(&mut self, lba: u64, sortie: &mut [u8]) -> bool {
        let n = sortie.len() / self.taille_bloc;
        n != 0 && bloc::lit(nvme::VOLUME_INTERNE, lba, n, sortie).blocs() == n
    }
    fn ecrit(&mut self, lba: u64, donnees: &[u8]) -> bool {
        let n = donnees.len() / self.taille_bloc;
        n != 0 && bloc::ecrit(nvme::VOLUME_INTERNE, lba, n, donnees).blocs() == n
    }
    fn vidange(&mut self) -> bool {
        bloc::vidange(nvme::VOLUME_INTERNE)
    }
}

// ---------------------------------------------------------------------------
// Une partition, vue comme un volume
// ---------------------------------------------------------------------------

/// Une fenetre sur le disque interne, enregistree comme volume a part entiere.
///
/// # Pourquoi une fenetre et pas un second pilote
///
/// La persistance occupe la FIN de son volume. Lui donner le disque entier
/// ferait ecrire sa zone dans les derniers blocs du DISQUE -- c'est-a-dire
/// par-dessus la table GPT de secours. Elle doit donc voir une partition, et
/// rien d'autre : ce qui est hors de la fenetre n'existe pas pour elle.
///
/// Les bornes sont verifiees ici et pas seulement a l'ouverture : une requete
/// hors fenetre n'est pas une erreur theorique, c'est le mecanisme meme par
/// lequel un systeme de fichiers ecrit sur ses voisins.
pub struct Fenetre {
    premier: core::sync::atomic::AtomicU64,
    blocs: core::sync::atomic::AtomicU64,
}

impl Fenetre {
    pub const fn vide() -> Self {
        Self {
            premier: core::sync::atomic::AtomicU64::new(0),
            blocs: core::sync::atomic::AtomicU64::new(0),
        }
    }

    fn pose(&self, premier: u64, blocs: u64) {
        self.premier
            .store(premier, core::sync::atomic::Ordering::Release);
        self.blocs.store(blocs, core::sync::atomic::Ordering::Release);
    }

    fn bornes(&self) -> (u64, u64) {
        (
            self.premier.load(core::sync::atomic::Ordering::Acquire),
            self.blocs.load(core::sync::atomic::Ordering::Acquire),
        )
    }

    /// Traduit une adresse de la fenetre en adresse du disque.
    fn adresse(&self, lba: u64, n: usize) -> Option<u64> {
        let (premier, blocs) = self.bornes();
        if blocs == 0 {
            return None;
        }
        let fin = lba.checked_add(n as u64)?;
        if fin > blocs {
            return None;
        }
        premier.checked_add(lba)
    }
}

static SYSTEME: Fenetre = Fenetre::vide();

impl PiloteBloc for Fenetre {
    fn descripteur(&self) -> Descripteur {
        let (_, blocs) = self.bornes();
        if blocs == 0 {
            return Descripteur::absent();
        }
        let interne = bloc::descripteur(nvme::VOLUME_INTERNE);
        Descripteur {
            taille_bloc: interne.taille_bloc,
            blocs,
            profondeur_file: interne.profondeur_file,
            vidange_reelle: interne.vidange_reelle,
            nom: "nvme0p2",
        }
    }

    fn soumet(&self, requete: Requete, tampon: &mut [u8]) -> Achevement {
        let Some(lba) = self.adresse(requete.lba, requete.blocs) else {
            return Achevement::Erreur;
        };
        bloc::lit(nvme::VOLUME_INTERNE, lba, requete.blocs, tampon)
    }

    fn soumet_ecriture(&self, requete: Requete, donnees: &[u8]) -> Achevement {
        if requete.genre == Genre::Vidange {
            // Une vidange porte sur le DISQUE, pas sur la fenetre : le cache
            // qu'elle vide est celui du controleur.
            return if bloc::vidange(nvme::VOLUME_INTERNE) {
                Achevement::Fait(0)
            } else {
                Achevement::Erreur
            };
        }
        let Some(lba) = self.adresse(requete.lba, requete.blocs) else {
            return Achevement::Erreur;
        };
        bloc::ecrit(nvme::VOLUME_INTERNE, lba, requete.blocs, donnees)
    }
}

// ---------------------------------------------------------------------------
// Lecture de la charge d'installation
// ---------------------------------------------------------------------------

/// Un fichier a deposer dans l'ESP.
struct Charge {
    /// Chemin dans l'ESP, separateurs `/`.
    destination: String,
    contenu: Vec<u8>,
}

fn lit_fichier(chemin: &str) -> Option<Vec<u8>> {
    let noeud = {
        let fs = crate::fs::ramfs::fs();
        fs.resolve(chemin, 0)?
    };
    let taille = crate::fs::backing::logical_len(noeud);
    if taille == 0 {
        return None;
    }
    let mut octets = vec![0u8; taille];
    let lus = crate::fs::backing::read_at(noeud, 0, &mut octets);
    if lus != taille {
        return None;
    }
    Some(octets)
}

/// Les fichiers d'amorcage, lus dans l'archive.
///
/// L'ordre compte pour la lisibilite du journal, pas pour le resultat. Ce qui
/// compte, c'est que le CHARGEUR soit obligatoire : sans lui, l'ESP est un
/// systeme de fichiers valide que le micrologiciel ignorera, ce qui est le
/// plus mauvais des resultats -- une installation qui se declare reussie et
/// une machine qui ne demarre pas.
fn charge_d_amorcage() -> Result<Vec<Charge>, Erreur> {
    let obligatoires = [
        ("bootx64.efi", CHEMIN_CHARGEUR),
        ("kernel-x86_64", "kernel-x86_64"),
        ("boot.json", "boot.json"),
    ];
    let mut sortie = Vec::new();
    for (source, destination) in obligatoires {
        let chemin = alloc::format!("{}/{}", DOSSIER_CHARGE, source);
        let Some(contenu) = lit_fichier(&chemin) else {
            crate::serial_println!("BOUCHAUD_INSTALL_CHARGE_ABSENTE {}", chemin);
            return Err(Erreur::ChargeIntrouvable);
        };
        sortie.push(Charge { destination: String::from(destination), contenu });
    }
    // Le second etage du chargeur n'existe que dans les images qui ont un
    // preboot : son absence n'est pas une faute.
    let optionnel = alloc::format!("{}/bouchaud-loader.efi", DOSSIER_CHARGE);
    if let Some(contenu) = lit_fichier(&optionnel) {
        sortie.push(Charge {
            destination: String::from("EFI/BOOT/BOUCHAUD-LOADER.EFI"),
            contenu,
        });
    }
    Ok(sortie)
}

static ARCHIVE_ADRESSE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static ARCHIVE_OCTETS: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

/// Note ou le chargeur a depose l'archive.
///
/// `BootInfo` n'est pas accessible depuis le shell : il est normalise une fois
/// au demarrage et personne ne le republie. Plutot que de rendre toute la
/// structure globale pour deux nombres, on note les deux nombres -- et
/// l'installateur ne peut donc pas lire autre chose que ce qu'on a decide de
/// lui donner.
pub fn note_archive(adresse: u64, octets: usize) {
    ARCHIVE_ADRESSE.store(adresse, core::sync::atomic::Ordering::Release);
    ARCHIVE_OCTETS.store(octets, core::sync::atomic::Ordering::Release);
}

/// L'archive qui tourne, telle que le chargeur l'a mise en memoire.
fn archive_vivante() -> Result<Vec<u8>, Erreur> {
    let adresse = ARCHIVE_ADRESSE.load(core::sync::atomic::Ordering::Acquire);
    let octets = ARCHIVE_OCTETS.load(core::sync::atomic::Ordering::Acquire);
    if adresse == 0 || octets == 0 {
        return Err(Erreur::ArchiveAbsente);
    }
    let source = crate::kernel::memory::phys_to_virt(adresse);
    let mut copie = vec![0u8; octets];
    unsafe { core::ptr::copy_nonoverlapping(source, copie.as_mut_ptr(), octets) };
    Ok(copie)
}

// ---------------------------------------------------------------------------
// L'etat du disque
// ---------------------------------------------------------------------------

/// Ce que le disque interne porte aujourd'hui.
#[derive(Clone, Debug, Default)]
pub struct Etat {
    pub present: bool,
    pub blocs: u64,
    pub taille_bloc: usize,
    pub partitions: Vec<(String, u64, u64)>,
    /// Le disque porte-t-il deja une installation Bouchaud ?
    pub bouchaud: bool,
    /// La table n'a pu etre lue que par sa copie de secours.
    pub degradee: bool,
}

/// Regarde le disque sans rien y ecrire.
pub fn examine() -> Etat {
    let Some(mut disque) = DisqueInterne::ouvre() else {
        return Etat::default();
    };
    let mut etat = Etat {
        present: true,
        blocs: disque.blocs,
        taille_bloc: disque.taille_bloc,
        ..Etat::default()
    };
    if let Ok((partitions, degradee)) = gpt::lit_table(&mut disque) {
        etat.degradee = degradee;
        for p in partitions {
            let nom = String::from_utf8_lossy(&p.nom)
                .trim_end_matches('\0')
                .trim()
                .into();
            if p.type_guid == gpt::TYPE_SYSTEME_BOUCHAUD {
                etat.bouchaud = true;
            }
            etat.partitions.push((nom, p.premier, p.blocs()));
        }
    }
    etat
}

// ---------------------------------------------------------------------------
// L'installation
// ---------------------------------------------------------------------------

/// Installe le systeme sur le disque interne.
///
/// `ecrase` autorise la destruction de partitions qui ne sont pas les notres.
/// Sans lui, un disque qui porte deja autre chose est refuse : un utilisateur
/// qui a un autre systeme sur ce disque doit se voir refuser l'installation
/// avec une phrase, pas la decouvrir detruite.
pub fn installe(ecrase: bool) -> Result<Rapport, Erreur> {
    let charge = charge_d_amorcage()?;
    let archive = archive_vivante()?;
    let Some(mut disque) = DisqueInterne::ouvre() else {
        return Err(Erreur::DisqueAbsent);
    };
    let taille_bloc = disque.taille_bloc;

    // Un disque deja occupe par autre chose que nous.
    if !ecrase {
        if let Ok((partitions, _)) = gpt::lit_table(&mut disque) {
            let etrangere = partitions
                .iter()
                .any(|p| p.type_guid != gpt::TYPE_ESP && p.type_guid != gpt::TYPE_SYSTEME_BOUCHAUD);
            if etrangere {
                return Err(Erreur::DisqueOccupe);
            }
        }
    }

    let table = gpt::disposition(disque.blocs, taille_bloc).ok_or(Erreur::DisqueTropPetit)?;

    // --- Ou poser les deux partitions --------------------------------------
    let utile: u64 = charge.iter().map(|c| c.contenu.len() as u64).sum::<u64>()
        + archive.len() as u64;
    let pose = plan(
        table.premier_utilisable,
        table.dernier_utilisable,
        taille_bloc,
        utile,
    )
    .ok_or(Erreur::DisqueTropPetit)?;
    let (esp_premier, esp_dernier) = (pose.esp_premier, pose.esp_dernier);
    let (systeme_premier, systeme_dernier) = (pose.systeme_premier, pose.systeme_dernier);
    let esp_blocs = pose.esp_blocs();
    let systeme_blocs = pose.systeme_blocs();

    // --- La table -----------------------------------------------------------
    // Les GUID viennent de l'entropie du noyau. Deux disques installes depuis
    // la meme image ne doivent pas porter le meme identifiant : un chargeur qui
    // designe une partition par son GUID demarrerait alors le mauvais disque.
    let mut graine = {
        let mut octets = [0u8; 8];
        crate::net::security::tls::rng::fill(&mut octets);
        u64::from_le_bytes(octets)
    };
    let disque_guid = gpt::guid_aleatoire(&mut graine);
    let partitions = [
        Partition::neuve(
            gpt::TYPE_ESP,
            gpt::guid_aleatoire(&mut graine),
            esp_premier,
            esp_dernier,
            "BOUCHAUD ESP",
        ),
        Partition::neuve(
            gpt::TYPE_SYSTEME_BOUCHAUD,
            gpt::guid_aleatoire(&mut graine),
            systeme_premier,
            systeme_dernier,
            "BOUCHAUD SYSTEME",
        ),
    ];
    gpt::ecrit_table(&mut disque, disque_guid, &partitions).map_err(|erreur| {
        crate::serial_println!("BOUCHAUD_INSTALL_TABLE_REFUSEE {:?}", erreur);
        Erreur::TableRefusee
    })?;
    crate::serial_println!(
        "BOUCHAUD_INSTALL_TABLE_OK esp={}+{} systeme={}+{}",
        esp_premier,
        esp_blocs,
        systeme_premier,
        systeme_blocs
    );

    // --- L'ESP --------------------------------------------------------------
    let mut fichiers = 0usize;
    let mut octets = 0u64;
    {
        let mut volume = fat32::formate(
            &mut disque,
            esp_premier,
            esp_blocs,
            "BOUCHAUDESP",
            graine as u32,
        )
        .map_err(|erreur| {
            crate::serial_println!("BOUCHAUD_INSTALL_FORMAT_REFUSE {:?}", erreur);
            Erreur::FormatageRefuse
        })?;

        for element in &charge {
            let (repertoire, nom) = pose_chemin(&mut volume, &element.destination)?;
            volume
                .ecrit_fichier(repertoire, nom, &element.contenu)
                .map_err(|_| Erreur::EcritureRefusee)?;
            fichiers += 1;
            octets += element.contenu.len() as u64;
        }

        let racine = volume.racine();
        volume
            .ecrit_fichier(racine, "ramdisk", &archive)
            .map_err(|_| Erreur::EcritureRefusee)?;
        fichiers += 1;
        octets += archive.len() as u64;

        volume.termine().map_err(|_| Erreur::EcritureRefusee)?;
    }

    // --- La partition systeme ----------------------------------------------
    // On efface le debut de la partition : la persistance y cherche un
    // superbloc, et d'anciens octets qui ressemblent a un superbloc valide la
    // feraient monter des donnees qui n'existent plus.
    effacer_debut(&mut disque, systeme_premier, taille_bloc)?;

    let barriere_reelle = disque.vidange();
    crate::serial_println!(
        "BOUCHAUD_INSTALL_GREEN fichiers={} octets={} esp={}+{} systeme={}+{} barriere={}",
        fichiers,
        octets,
        esp_premier,
        esp_blocs,
        systeme_premier,
        systeme_blocs,
        barriere_reelle as u8
    );

    Ok(Rapport {
        esp_premier,
        esp_blocs,
        systeme_premier,
        systeme_blocs,
        fichiers,
        octets,
        barriere_reelle,
    })
}

/// Cree les repertoires d'un chemin et rend (repertoire, nom du fichier).
fn pose_chemin<'a, S: Support>(
    volume: &mut fat32::Volume<'a, S>,
    chemin: &str,
) -> Result<(u32, &'static str), Erreur> {
    // Les noms sont des constantes du module : on rend une reference statique
    // plutot qu'une tranche du chemin, pour ne pas emprunter `chemin` au-dela
    // de l'appel.
    let mut repertoire = volume.racine();
    let mut dernier = "";
    let total = chemin.split('/').count();
    for (i, element) in chemin.split('/').enumerate() {
        if element.is_empty() {
            continue;
        }
        if i + 1 == total {
            dernier = nom_statique(element).ok_or(Erreur::EcritureRefusee)?;
            break;
        }
        repertoire = volume
            .cree_repertoire(repertoire, element)
            .map_err(|_| Erreur::EcritureRefusee)?;
    }
    if dernier.is_empty() {
        return Err(Erreur::EcritureRefusee);
    }
    Ok((repertoire, dernier))
}

/// Les seuls noms de fichier que l'installateur depose.
///
/// Une liste FERMEE plutot qu'un nom quelconque : ce qui atterrit dans une ESP
/// decide de ce que le micrologiciel demarre, et une liste ouverte ferait de
/// ce chemin un moyen d'ecrire un fichier arbitraire dans la partition
/// d'amorcage.
fn nom_statique(nom: &str) -> Option<&'static str> {
    match nom {
        "BOOTX64.EFI" => Some("BOOTX64.EFI"),
        "BOUCHAUD-LOADER.EFI" => Some("BOUCHAUD-LOADER.EFI"),
        "kernel-x86_64" => Some("kernel-x86_64"),
        "boot.json" => Some("boot.json"),
        "ramdisk" => Some("ramdisk"),
        _ => None,
    }
}

fn effacer_debut<S: Support>(
    support: &mut S,
    premier: u64,
    taille_bloc: usize,
) -> Result<(), Erreur> {
    let vide = vec![0u8; taille_bloc];
    for i in 0..8u64 {
        if !support.ecrit(premier + i, &vide) {
            return Err(Erreur::EcritureRefusee);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Le montage au demarrage
// ---------------------------------------------------------------------------

/// Cherche une partition systeme Bouchaud et l'expose comme volume de donnees.
///
/// Rend `true` quand elle existe. C'est ce qui distingue un demarrage INSTALLE
/// d'un demarrage live : la persistance ecrit alors sur un disque, et ce
/// qu'elle ecrit survit a l'extinction.
///
/// La fenetre est POSEE avant l'enregistrement. Un volume enregistre sur une
/// fenetre encore vide rendrait un descripteur absent au premier appelant qui
/// regarde, et la persistance conclurait qu'il n'y a pas de disque.
pub fn monte_le_systeme_installe() -> bool {
    let Some(mut disque) = DisqueInterne::ouvre() else {
        return false;
    };
    let Ok((partitions, degradee)) = gpt::lit_table(&mut disque) else {
        return false;
    };
    if degradee {
        crate::serial_println!("BOUCHAUD_INSTALL_TABLE_DEGRADEE");
    }
    let Some(systeme) = partitions
        .iter()
        .find(|p| p.type_guid == gpt::TYPE_SYSTEME_BOUCHAUD)
    else {
        return false;
    };
    SYSTEME.pose(systeme.premier, systeme.blocs());
    bloc::enregistre(Volume::DONNEES, &SYSTEME);
    SYSTEME_MONTE.store(true, core::sync::atomic::Ordering::Release);
    crate::serial_println!(
        "BOUCHAUD_INSTALL_SYSTEME_MONTE premier={} blocs={}",
        systeme.premier,
        systeme.blocs()
    );
    true
}

// ---------------------------------------------------------------------------
// Le montage DIFFERE
// ---------------------------------------------------------------------------

/// La partition Bouchaud est-elle montee sur `Volume::DONNEES` ?
///
/// # Pourquoi la question ne peut pas se poser autrement
///
/// `bloc::present(Volume::DONNEES)` ne repond PAS a cette question. Le pilote
/// ATA enregistre lui aussi son disque esclave sur ce meme volume, a
/// l'amorcage et sans condition. Un volume present ne dit donc rien de ce
/// qu'il porte.
///
/// La distinction n'est pas academique : une commande d'ecriture qui se fierait
/// a `present()` ecrirait sur le disque de la machine en croyant ecrire dans
/// une partition dediee. Le scenario H10 l'a montre sur un disque vierge --
/// l'ecriture avait lieu, a un LBA calcule sur une partition qui n'existait
/// pas.
///
/// Ce drapeau n'est pose que par le montage reussi d'une partition portant le
/// GUID de type Bouchaud.
static SYSTEME_MONTE: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// La partition Bouchaud est-elle montee ?
pub fn systeme_monte() -> bool {
    SYSTEME_MONTE.load(core::sync::atomic::Ordering::Acquire)
}

/// Un montage a-t-il ete demande, et pas encore tente ?
static MONTAGE_DEMANDE: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);
/// Le montage differe a-t-il deja ete tente ? Il ne l'est qu'une fois.
static MONTAGE_TENTE: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// Demande le montage du systeme installe SANS le faire maintenant.
///
/// # Pourquoi la persistance ne peut plus etre une etape d'amorcage
///
/// Sonder le disque interne au demarrage, c'est faire dependre l'arrivee au
/// bureau d'un peripherique dont on ne sait rien. Un NVMe muet -- un modele
/// que le pilote ne mene pas jusqu'a l'achevement, un controleur laisse dans
/// un etat batard par le micrologiciel -- n'echouait pas : il faisait ATTENDRE.
/// Et une attente placee avant le premier affichage est, vue de l'utilisateur,
/// un ecran fige, sans clavier ni souris, sans rien qui dise pourquoi.
///
/// Or rien de ce que le bureau affiche ne vient de ce disque. L'archive
/// Ladybird voyage dans l'image UEFI et vit en RAM ; la persistance n'ajoute
/// que la survie des fichiers a une coupure. C'est un CONFORT, et un confort
/// ne prend pas le systeme en otage.
///
/// Le montage est donc demande ici et execute apres le premier rendu du
/// bureau, par [`execute_le_montage_differe`]. Le pire cas devient : le bureau
/// apparait, la souris bouge, et la persistance reste absente -- ce qui se lit
/// dans le journal au lieu de se deviner devant un ecran arrete.
pub fn differe_le_montage() {
    MONTAGE_DEMANDE.store(true, core::sync::atomic::Ordering::Release);
    crate::serial_println!(
        "BOUCHAUD_NVME_PERSISTENCE_DEFERRED raison=arrivee-au-bureau-prioritaire"
    );
}

/// Le montage differe reste-t-il a faire ?
pub fn montage_differe_en_attente() -> bool {
    MONTAGE_DEMANDE.load(core::sync::atomic::Ordering::Acquire)
        && !MONTAGE_TENTE.load(core::sync::atomic::Ordering::Acquire)
}

/// Lance le montage differe DANS SON PROPRE FIL, et rend la main aussitot.
///
/// # Ce que l'appel en ligne coutait au bureau
///
/// Le montage etait appele depuis la boucle de trames du compositeur, sur SA
/// pile et dans SON quantum. Tout ce que fait le disque, le bureau le subissait
/// donc : une commande lente est une trame perdue, et la profondeur de pile du
/// montage s'ajoutait a celle du compositeur -- qui est deja la plus longue
/// chaine du systeme.
///
/// Rien de tout cela n'est necessaire. Le montage n'a aucun resultat que la
/// trame en cours attende : il enregistre un volume, et le systeme de fichiers
/// le trouvera quand il regardera.
///
/// # IL N'Y A PAS DE REPLI EN LIGNE, ET C'EST DELIBERE
///
/// La premiere version retombait sur un montage SYNCHRONE quand le fil ne
/// pouvait pas etre cree. C'etait exactement le mauvais echange.
///
/// Ne pas pouvoir creer une tache veut dire : memoire sous pression, table des
/// processus pleine, systeme deja degrade. C'est le PIRE moment pour remettre
/// une operation disque potentiellement longue sur le fil graphique -- celui
/// qui doit continuer a repondre justement parce que le reste va mal.
///
/// Le comportement est donc : le compositeur continue, la persistance reste
/// non montee, et l'etat degrade est DIT. Un montage qu'on n'a pas fait se
/// rattrape ; un bureau qu'on a fige pendant que la machine manquait de
/// memoire, non.
///
/// # Ce que cette separation protege, et ce qu'elle ne protege PAS
///
/// Elle protege la LATENCE et la PILE du compositeur. Elle ne protege pas le
/// noyau d'une faute fatale : un `#DF` en anneau zero dans le fil de montage
/// tue le systeme entier, exactement comme dans le bureau. Deplacer une faute
/// noyau d'un fil vers un autre ne l'isole pas.
pub fn lance_le_montage_differe() -> bool {
    if !montage_differe_en_attente() {
        return false;
    }
    if crate::kernel::task::spawn_noyau(fil_de_montage, "montage") {
        crate::serial_println!("BOUCHAUD_NVME_PERSISTENCE_FIL_LANCE");
        return true;
    }
    // Le drapeau n'est PAS consomme : `execute_le_montage_differe` ne l'a pas
    // vu. Une reprise ulterieure depuis un service reste donc possible, et
    // c'est ce que dit `montage_differe_en_attente()`.
    MONTAGES_REFUSES.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    crate::serial_println!(
        "BOUCHAUD_NVME_PERSISTENCE_FIL_REFUSE raison=tache-non-creee \
         consequence=persistance-non-montee etat=degrade"
    );
    false
}

/// Tentatives de lancement du fil de montage refusees.
///
/// Non nul veut dire que la persistance n'est pas montee ALORS QU'ELLE ETAIT
/// DEMANDEE. Sans ce compteur, un systeme sous pression perdrait sa
/// persistance en silence.
static MONTAGES_REFUSES: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

/// Lancements du fil de montage refuses, pour le diagnostic.
pub fn montages_refuses() -> u64 {
    MONTAGES_REFUSES.load(core::sync::atomic::Ordering::Relaxed)
}

/// Le corps du fil de montage.
fn fil_de_montage() -> ! {
    execute_le_montage_differe();
    crate::kernel::task::exit_current(0)
}

/// Execute le montage differe, une seule fois.
///
/// Rend `true` quand une partition systeme a ete montee. Rend `false` dans
/// tous les autres cas -- y compris quand il n'y avait rien a faire --, et
/// n'attend jamais plus que ce que le pilote bloc s'autorise.
///
/// L'appelant est le bureau, pas l'amorcage : voir [`differe_le_montage`].
pub fn execute_le_montage_differe() -> bool {
    if !MONTAGE_DEMANDE.load(core::sync::atomic::Ordering::Acquire) {
        return false;
    }
    if MONTAGE_TENTE.swap(true, core::sync::atomic::Ordering::AcqRel) {
        return false;
    }

    if !nvme::present() {
        crate::serial_println!(
            "BOUCHAUD_NVME_PERSISTENCE_ABSENTE raison={}",
            if nvme::hors_service() { "disque-hors-service" } else { "aucun-disque" }
        );
        return false;
    }

    crate::serial_println!("BOUCHAUD_NVME_PERSISTENCE_PROBE_BEGIN");
    if !monte_le_systeme_installe() {
        crate::serial_println!(
            "BOUCHAUD_NVME_PERSISTENCE_ABSENTE raison=aucune-partition-bouchaud hors_service={}",
            nvme::hors_service() as u8
        );
        return false;
    }
    let restaures = crate::fs::persistance::monte();
    crate::serial_println!(
        "BOUCHAUD_STAGE2_PERSIST_NVME fichiers={} (differe)",
        restaures
    );
    crate::kernel::dmesg::log("nvme: persistance montee apres l'arrivee au bureau");
    true
}

/// Le disque interne porte-t-il un GUID de partition ESP a nous ?
pub fn esp_installee() -> bool {
    examine()
        .partitions
        .iter()
        .any(|(nom, _, _)| nom.starts_with("BOUCHAUD"))
}
