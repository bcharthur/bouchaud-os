//! Pilote minimal Realtek RTL8111/RTL8168 PCIe Gigabit Ethernet.
//!
//! Cible materielle Stage 2 TRIGKEY : PCI 10EC:8168. Le pilote fonctionne en
//! polling, sans MSI/MSI-X, et utilise les anneaux DMA 64 bits du controleur.
//! Il est volontairement strict : aucun autre identifiant Realtek n'est accepte.
//!
//! Le contrat de registres/descripteurs suit le pilote r8169 du noyau Linux et
//! le pilote rtl8169 d'U-Boot. Les sequences PHY/EEE specifiques a une revision
//! ne sont pas fabriquees ici : si la liaison physique ne monte pas sur une
//! revision particuliere, l'initialisation echoue proprement et le bureau reste
//! utilisable hors ligne.

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{compiler_fence, AtomicBool, AtomicU32, AtomicU64, Ordering};

use crate::arch::x86_64::pci::{self, Bar, PciDevice};
use crate::drivers::anneau_rx as anneau;
use crate::kernel::{dmesg, lab, memory};

pub const VENDOR_REALTEK: u16 = 0x10EC;
pub const DEVICE_RTL8168: u16 = 0x8168;

const REG_MAC0: u32 = 0x00;
const REG_MAR0: u32 = 0x08;
const REG_TX_DESC_LOW: u32 = 0x20;
const REG_TX_DESC_HIGH: u32 = 0x24;
const REG_CHIP_CMD: u32 = 0x37;
const REG_TX_POLL: u32 = 0x38;
const REG_INTR_MASK: u32 = 0x3C;
const REG_INTR_STATUS: u32 = 0x3E;
const REG_TX_CONFIG: u32 = 0x40;
const REG_RX_CONFIG: u32 = 0x44;
const REG_RX_MISSED: u32 = 0x4C;
const REG_CFG9346: u32 = 0x50;
/// `Config2` : porte le bit d'autorisation `CLKREQ`.
const REG_CONFIG2: u32 = 0x53;
/// `Config5` : porte le bit d'autorisation ASPM.
const REG_CONFIG5: u32 = 0x56;
/// `MISC` : porte `RXDV_GATE` et les etats L2/L3 du lien PCIe.
const REG_MISC: u32 = 0x00F0;
const REG_PHY_STATUS: u32 = 0x6C;
const REG_RX_MAX_SIZE: u32 = 0xDA;
const REG_CPLUS_CMD: u32 = 0xE0;
const REG_RX_DESC_LOW: u32 = 0xE4;
const REG_RX_DESC_HIGH: u32 = 0xE8;

const CMD_RESET: u8 = anneau::cmd::RESET;
const CMD_RX_ENABLE: u8 = anneau::cmd::RX_ENB;
const CMD_TX_ENABLE: u8 = anneau::cmd::TX_ENB;
const TX_POLL_NPQ: u8 = 0x40;
const CFG9346_UNLOCK: u8 = 0xC0;
const CFG9346_LOCK: u8 = 0x00;
const PHY_LINK_STATUS: u8 = 0x02;
/// Bits de vitesse et de duplex du registre `PHYstatus` (0x6C).
const PHY_DUPLEX_COMPLET: u8 = 0x01;
const PHY_10M: u8 = 0x04;
const PHY_100M: u8 = 0x08;
const PHY_1000M: u8 = 0x10;

// ---------------------------------------------------------------------------
// BOUCHAUD_RTL8168_AUTONEGOCIATION_V1 : parler au PHY, et pas seulement le lire
// ---------------------------------------------------------------------------
//
// Le pilote LISAIT `PHYstatus` et n'ecrivait jamais dans le PHY. Sur la
// machine de reference, brancher le cable APRES le demarrage ne montait pas le
// lien : le PHY restait dans l'etat ou la reinitialisation du controleur
// l'avait laisse, sans autonegociation en cours, et le bit de lien ne montait
// donc jamais.
//
// `PHYAR` (0x60) est la fenetre MDIO du controleur : bit 31 arme l'operation
// (1 = ecriture), bits 16..20 le numero de registre, bits 0..15 la valeur.
// Le controleur efface le bit 31 quand une ecriture est finie, et le pose
// quand une lecture est prete.

/// Fenetre MDIO du controleur.
const REG_PHYAR: u32 = 0x60;
/// Bit d'armement d'une ECRITURE dans `PHYAR`.
const PHYAR_ECRITURE: u32 = 1 << 31;

/// Registre de controle du PHY (BMCR, IEEE 802.3 clause 22).
const MII_BMCR: u32 = 0x00;
/// Ce que nous annonçons en 10/100 (ANAR).
const MII_ANAR: u32 = 0x04;
/// Ce que nous annonçons en gigabit (GBCR).
const MII_GBCR: u32 = 0x09;

/// BMCR : relancer l'autonegociation.
const BMCR_RELANCE: u16 = 1 << 9;
/// BMCR : autonegociation activee.
const BMCR_AUTONEGOCIATION: u16 = 1 << 12;
/// BMCR : PHY en veille. Un PHY endormi ne voit jamais le cable.
const BMCR_VEILLE: u16 = 1 << 11;

/// ANAR : 10/100, half et full, plus le selecteur 802.3.
const ANAR_10_100: u16 = 0x01E1;
/// GBCR : 1000 half et full.
const GBCR_1000: u16 = 0x0300;

/// Tours d'attente d'une operation MDIO.
///
/// Le manuel donne vingt microsecondes par acces ; mille tours de lecture d'un
/// registre memoire mappe en couvrent largement plus, et la borne EXISTE pour
/// qu'un controleur muet ne fige pas le demarrage.
const MDIO_TOURS: usize = 20_000;

unsafe fn mdio_ecrit(registre: u32, valeur: u16) -> bool {
    write32(
        REG_PHYAR,
        PHYAR_ECRITURE | ((registre & 0x1f) << 16) | valeur as u32,
    );
    for _ in 0..MDIO_TOURS {
        if read32(REG_PHYAR) & PHYAR_ECRITURE == 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

unsafe fn mdio_lit(registre: u32) -> Option<u16> {
    write32(REG_PHYAR, (registre & 0x1f) << 16);
    for _ in 0..MDIO_TOURS {
        let etat = read32(REG_PHYAR);
        if etat & PHYAR_ECRITURE != 0 {
            return Some((etat & 0xffff) as u16);
        }
        core::hint::spin_loop();
    }
    None
}

/// Reveille le PHY et relance son autonegociation.
///
/// Sans elle, un cable branche apres le demarrage n'etait jamais vu : le lien
/// ne monte que si les deux extremites negocient, et personne ne le demandait
/// de ce cote.
unsafe fn relance_autonegociation() -> bool {
    let Some(bmcr) = mdio_lit(MII_BMCR) else {
        crate::serial_println!("BOUCHAUD_TRIGKEY_RTL8168_MDIO_MUET");
        return false;
    };
    // Un PHY en veille ne voit pas le cable, quoi qu'on lui annonce ensuite.
    if bmcr & BMCR_VEILLE != 0 {
        let _ = mdio_ecrit(MII_BMCR, bmcr & !BMCR_VEILLE);
    }
    let _ = mdio_ecrit(MII_ANAR, ANAR_10_100);
    let _ = mdio_ecrit(MII_GBCR, GBCR_1000);
    let ok = mdio_ecrit(MII_BMCR, BMCR_AUTONEGOCIATION | BMCR_RELANCE);
    crate::serial_println!(
        "BOUCHAUD_TRIGKEY_RTL8168_AUTONEG bmcr_avant={:#06x} relance={}",
        bmcr,
        ok as u8,
    );
    ok
}

/// Relance l'autonegociation depuis l'exterieur du pilote.
///
/// Le veilleur de lien s'en sert quand le lien est bas depuis un moment : un
/// cable qu'on vient de brancher demande une negociation, et la demander deux
/// fois ne coute que quatre acces MDIO.
pub fn reveille_le_lien() -> bool {
    unsafe {
        if !READY {
            return false;
        }
        relance_autonegociation()
    }
}

/// Vitesse negociee, en megabits par seconde. Zero si le lien est bas.
pub fn vitesse_mbps() -> u32 {
    unsafe {
        if !READY {
            return 0;
        }
        let etat = read8(REG_PHY_STATUS);
        if etat & PHY_LINK_STATUS == 0 {
            return 0;
        }
        if etat & PHY_1000M != 0 {
            1000
        } else if etat & PHY_100M != 0 {
            100
        } else if etat & PHY_10M != 0 {
            10
        } else {
            0
        }
    }
}

/// Le lien est-il en duplex integral ?
///
/// Un lien a l'alternat (half duplex) sur du cuivre moderne signale presque
/// toujours une negociation ratee d'un cote : les collisions y divisent le
/// debit utile. C'est un signe de QUALITE, pas seulement de vitesse.
pub fn duplex_complet() -> bool {
    unsafe {
        READY
            && read8(REG_PHY_STATUS) & PHY_LINK_STATUS != 0
            && read8(REG_PHY_STATUS) & PHY_DUPLEX_COMPLET != 0
    }
}

/// Trames que la carte a laissees tomber faute de tampon libre.
///
/// Compteur materiel `RxMissed` (0x4C). Il ne bouge pas sur un reseau sain ;
/// s'il monte, le systeme ne relit pas assez vite et des paquets sont perdus
/// avant meme d'atteindre la pile.
pub fn trames_perdues() -> u32 {
    unsafe {
        if !READY {
            return 0;
        }
        read32(REG_RX_MISSED)
    }
}

/// Les bits d'acceptation, ecrits EN DERNIER comme `rtl_set_rx_mode`.
const ACCEPTATION: u32 =
    anneau::ACCEPTE_DIFFUSION | anneau::ACCEPTE_MULTIDIFFUSION | anneau::ACCEPTE_MA_MAC;
const TX_DMA_BURST: u32 = 7 << 8;
const TX_INTERFRAME_GAP: u32 = 3 << 24;

const DESC_OWN: u32 = anneau::OWN;
const DESC_EOR: u32 = anneau::EOR;
const DESC_FS: u32 = anneau::FS;
const DESC_LS: u32 = anneau::LS;

/// Longueur minimale d'une trame Ethernet, FCS exclu (IEEE 802.3).
///
/// # LA TRAME LA PLUS COURTE QU'ON EMET EST CELLE QUI ECHOUAIT
///
/// Une requete ARP fait 14 + 28 = 42 octets. La norme exige 60 octets avant
/// le FCS : en dessous, la trame est un « runt » que le commutateur d'en face
/// jette. Le bourrage n'etait fait ni ici ni par l'appelant, et il n'est pas
/// garanti par le controleur -- le pilote r8169 de Linux appelle
/// `skb_padto(skb, ETH_ZLEN)` avant d'armer le descripteur, precisement parce
/// qu'on ne peut pas compter dessus.
///
/// Le releve du 13 septembre montre exactement cette signature : DHCP, qui
/// est en diffusion mais fait plus de trois cents octets, fonctionne et pose
/// le bail ; ARP, qui fait quarante-deux octets, n'obtient jamais de reponse ;
/// et tout l'unicast -- donc toute requete DNS, donc toute page -- echoue
/// derriere lui avec `parti=false`.
///
/// C'est la seule trame de moins de soixante octets que la pile emette.
const TRAME_MIN: usize = 60;

const N_RX: usize = anneau::DESCRIPTEURS;
const N_TX: usize = 16;
const BUF_SIZE: usize = anneau::TAILLE_TAMPON as usize;
const DESC_SIZE: usize = 16;
const RESET_SPINS: usize = 1_000_000;
const LINK_WAIT_MS: u64 = 3_000;

static mut MMIO: u64 = 0;
static mut READY: bool = false;
static mut MAC: [u8; 6] = [0; 6];
static mut RX_RING: *mut u8 = core::ptr::null_mut();
/// Adresse PHYSIQUE de l'anneau RX. Sans elle, reconstruire l'anneau ne peut
/// pas reprogrammer le controleur, et le pointeur interne resterait desyncrone.
static mut RX_RING_P: u64 = 0;
static mut TX_RING_P: u64 = 0;
static mut TX_RING: *mut u8 = core::ptr::null_mut();
static mut RX_BUFFER_V: *mut u8 = core::ptr::null_mut();
static mut RX_BUFFER_P: u64 = 0;
static mut TX_BUFFER_V: *mut u8 = core::ptr::null_mut();
static mut TX_BUFFER_P: u64 = 0;
static mut RX_CUR: usize = 0;
static mut TX_CUR: usize = 0;
static mut TX_RING_FULL: u64 = 0;
/// Trames jetees parce que le controleur les a marquees en erreur.
static mut RX_ABIMEES: u64 = 0;

// ---------------------------------------------------------------------------
// BOUCHAUD_RTL8168_RX_VIVANT_V1 : le releve, et rien qu'un releve
// ---------------------------------------------------------------------------
//
// Le releve physique du 17 septembre dit « trames=104 » puis plus rien, et il
// ne permet PAS de choisir entre quatre pannes differentes : moteur arrete,
// anneau sature, descripteurs desynchronises, ou carte disparue du bus. Le
// prochain releve doit trancher sans qu'on ait a deviner.
//
// Ce sont des compteurs et des instantanes. AUCUN journal par paquet : un
// pilote qui imprime a chaque trame ne mesure plus que lui-meme.
static RX_PAQUETS: AtomicU64 = AtomicU64::new(0);
static RX_OCTETS: AtomicU64 = AtomicU64::new(0);
static TX_PAQUETS: AtomicU64 = AtomicU64::new(0);
static TX_OCTETS: AtomicU64 = AtomicU64::new(0);
static RX_DERNIER_NS: AtomicU64 = AtomicU64::new(0);
static TX_DERNIER_NS: AtomicU64 = AtomicU64::new(0);
static TX_PREMIER_NS: AtomicU64 = AtomicU64::new(0);

static ISR_LECTURES: AtomicU64 = AtomicU64::new(0);
static ISR_RX_OK: AtomicU64 = AtomicU64::new(0);
static ISR_RX_ERR: AtomicU64 = AtomicU64::new(0);
static ISR_RX_OVERFLOW: AtomicU64 = AtomicU64::new(0);
static ISR_RX_FIFO_OVER: AtomicU64 = AtomicU64::new(0);
static ISR_TX_ERR: AtomicU64 = AtomicU64::new(0);
static ISR_LINK_CHG: AtomicU64 = AtomicU64::new(0);
static ISR_SYSTEM_ERROR: AtomicU64 = AtomicU64::new(0);
static ISR_CARTE_ABSENTE: AtomicU64 = AtomicU64::new(0);

static RX_REARMEMENTS: AtomicU64 = AtomicU64::new(0);
static RX_REPRISES: AtomicU64 = AtomicU64::new(0);
static RX_REPRISES_ECHOUEES: AtomicU64 = AtomicU64::new(0);
static RX_ABANDONNEES: AtomicU64 = AtomicU64::new(0);
static REPRISE_DERNIERE_NS: AtomicU64 = AtomicU64::new(0);
static REPRISES_SANS_EFFET: AtomicU32 = AtomicU32::new(0);
static MAINTENANCE_DERNIERE_NS: AtomicU64 = AtomicU64::new(0);
static SANTE_DERNIERE_NS: AtomicU64 = AtomicU64::new(0);
// BOUCHAUD_RTL8168_RX_VIVANT_V1 : LA REPARATION SE FAIT SUR LE CHEMIN DE
// RECEPTION, ET NULLE PART AILLEURS.
//
// Reconstruire l'anneau, c'est le reecrire en entier et remettre `RX_CUR` a
// zero. Le faire depuis le veilleur de lien pendant que `receive` lit un
// descripteur serait exactement la course que `VERROU_RECEPTION` existe pour
// fermer : « un seul point sort les trames de la carte ».
//
// Le veilleur ne fait donc que DEMANDER ; la reparation a lieu au prochain
// passage du drainage, sous le verrou de celui-ci.
/// LA PROGRESSION DU MATERIEL DANS L'ANNEAU, ET LA REPRISE.
///
/// # Ce que le releve du 18 septembre ne permettait pas de dire
///
/// Il montrait `isr_rx_ok` qui monte de 33 a 67 pendant que `rx_packets`
/// reste fige a 64 et que les soixante-quatre descripteurs restent la
/// propriete du materiel. Impossible d'en conclure quoi que ce soit : la
/// carte annonce-t-elle des trames qu'elle n'ecrit pas, ou le pilote lit-il
/// un anneau que la carte a quitte ?
///
/// Ces compteurs repondent, sans une ligne par paquet.

/// Indice du descripteur que le PROCESSEUR regarde, au dernier examen.
static RX_TETE_CPU: AtomicU64 = AtomicU64::new(0);
/// Dernier indice ou le processeur a effectivement PRIS une trame.
static RX_DERNIER_DESC_CPU: AtomicU64 = AtomicU64::new(0);
/// Transitions `OWN` 1 -> 0 observees : le materiel a rendu un descripteur.
static RX_OWN_RENDUS: AtomicU64 = AtomicU64::new(0);
/// `RxOK` signale alors qu'AUCUN descripteur n'est revenu au processeur.
///
/// C'est la signature exacte du defaut : la carte dit avoir recu, et
/// l'anneau ne bouge pas.
static RX_OK_SANS_DESCRIPTEUR: AtomicU64 = AtomicU64::new(0);
/// Nombre de fois ou l'arret de reception a ete CONSTATE.
static RX_ARRET_DETECTE: AtomicU64 = AtomicU64::new(0);
/// Reparations demandees, et reparations reellement executees.
///
/// Les deux ensemble : le releve physique avait zero reprise SANS qu'on
/// puisse dire si personne ne l'avait demandee ou si l'execution echouait.
static REPARATIONS_DEMANDEES: AtomicU64 = AtomicU64::new(0);
static REPARATIONS_EXECUTEES: AtomicU64 = AtomicU64::new(0);
/// Degre de la derniere reparation executee.
static REPARATION_DEGRE: AtomicU64 = AtomicU64::new(0);
/// 0=inconnue, 1=moteur-rx, 2=rx-silencieux.
static REPARATION_RAISON: AtomicU64 = AtomicU64::new(0);
static REPARATION_DERNIERE_NS: AtomicU64 = AtomicU64::new(0);

/// L'EMISSION, PROUVEE PAR LE MATERIEL.
///
/// `tx_packets` compte les trames REMISES a la carte, pas celles qui sont
/// parties sur le cable. Le releve du 18 septembre montre onze DISCOVER DHCP
/// enfiles, 342 octets chacun -- et rien ne dit si la carte les a seulement
/// transmis. Sans cette distinction, un anneau TX bloque se lit exactement
/// comme un serveur DHCP muet.
static TX_ENFILES: AtomicU64 = AtomicU64::new(0);
/// Trames dont le materiel a RENDU le descripteur : elles sont parties.
static TX_TERMINES: AtomicU64 = AtomicU64::new(0);
/// Instant du dernier achevement constate.
static TX_DERNIER_TERMINE_NS: AtomicU64 = AtomicU64::new(0);
/// Interruptions `TxOK` vues.
static ISR_TX_OK: AtomicU64 = AtomicU64::new(0);

/// L'IDENTITE PCI DE LA CARTE, RETENUE POUR TOUJOURS.
///
/// Une reprise de degre quatre doit pouvoir reprogrammer la puce. Elle ne le
/// peut que si l'on a garde de quoi la retrouver : son BAR, et son adresse
/// sur le bus. L'ancienne version effacait `MMIO` -- apres quoi plus rien ne
/// savait ou etait la carte, et le pilote ne pouvait que se declarer absent.
static mut BAR_PHYSIQUE: u64 = 0;
static mut PCI_BUS: u8 = 0;
static mut PCI_SLOT: u8 = 0;
static mut PCI_FONCTION: u8 = 0;
/// La carte a-t-elle ete VUE sur le bus ? Ne redevient jamais faux.
static VUE_SUR_LE_BUS: AtomicBool = AtomicBool::new(false);
/// L'etat du pilote, au sens de `etat_pilote::Etat`.
static ETAT_PILOTE: AtomicU64 = AtomicU64::new(0);
/// Reinitialisations tentees, et reussies.
static REINITIALISATIONS: AtomicU64 = AtomicU64::new(0);
static REINITIALISATIONS_OK: AtomicU64 = AtomicU64::new(0);

/// LES TOURS D'ANNEAU, ET CE QUI SE PASSE AU SECOND.
///
/// # Le fait le plus important du releve du 18 septembre
///
/// ```text
/// rx_packets  0 -> 4 -> ... -> 62 -> 64      puis 64 pour toujours
/// rx_cur      0 -> 4 -> ... -> 62 -> 0       puis 0 pour toujours
/// ```
///
/// Soixante-quatre trames, soixante-quatre descripteurs : EXACTEMENT UN TOUR.
/// Le processeur repasse a l'indice zero, et le materiel ne remplit plus
/// jamais un descripteur -- alors qu'il les possede tous les soixante-quatre
/// et qu'il continue de signaler `RxOK`.
///
/// Un premier tour reussi ne prouve donc RIEN : c'est precisement ce que
/// faisait toute la campagne precedente. Ces compteurs distinguent le premier
/// tour du second, seul le second etant une preuve que l'anneau CIRCULE.
static RX_TOURS_CPU: AtomicU64 = AtomicU64::new(0);
/// Descripteurs rendus par le materiel pendant le premier tour, puis apres.
static RX_RENDUS_TOUR1: AtomicU64 = AtomicU64::new(0);
static RX_RENDUS_TOUR2: AtomicU64 = AtomicU64::new(0);
/// Descripteurs rearmes pendant le premier tour, et REUTILISES ensuite.
static RX_REARMES_TOUR1: AtomicU64 = AtomicU64::new(0);
static RX_REUTILISES_TOUR2: AtomicU64 = AtomicU64::new(0);

/// `RxOK` a progresse SANS que la reception progresse.
///
/// # Pourquoi l'ancien compteur mentait
///
/// `rx_ok_sans_desc` comptait un `RxOK` vu alors que le descripteur courant
/// portait encore `OWN`. Or c'est l'etat NORMAL d'un anneau au repos : le
/// releve montre `rx_packets=4` avec `rx_ok_sans_desc=3` pendant que la
/// reception fonctionne parfaitement. Un compteur d'anomalie qui monte en
/// marche nominale ne sert a rien.
///
/// La vraie anomalie demande trois choses ENSEMBLE, sur une fenetre bornee :
/// `RxOK` a progresse, `rx_packets` n'a pas bouge, et aucun descripteur n'est
/// revenu au processeur.
static RX_OK_SANS_PROGRES: AtomicU64 = AtomicU64::new(0);
/// Temoins de la derniere fenetre d'observation.
static TEMOIN_ISR_RX_OK: AtomicU64 = AtomicU64::new(0);
static TEMOIN_RX_PAQUETS: AtomicU64 = AtomicU64::new(0);
static TEMOIN_OWN_RENDUS: AtomicU64 = AtomicU64::new(0);
static TEMOIN_FENETRE_NS: AtomicU64 = AtomicU64::new(0);
/// Duree de la fenetre d'observation du progres, en nanosecondes.
const FENETRE_PROGRES_NS: u64 = 1_000_000_000;

/// LE VERDICT D'UNE REPRISE, RENDU APRES COUP.
///
/// On n'escalade que sur une PREUVE d'absence de progres : la reception n'a
/// pas avance, et aucun descripteur n'est revenu, alors qu'on a laisse a la
/// carte une fenetre bornee pour le montrer.
///
/// # Pourquoi `static mut` et pas des atomiques
///
/// La machine a etats est une DECISION composee -- juger, puis admettre ou
/// differer, puis armer -- et trois atomiques lues separement ne composent
/// pas : c'est exactement ainsi que l'echeance se faisait reecrire entre deux
/// lectures. Elle n'est touchee que depuis `repare_si_demande`, appelee par le
/// seul drainage verrouille, sous `VERROU_RECEPTION` ; `REPRISES_SANS_EFFET`
/// en reste le reflet atomique, pour les lecteurs qui ne tiennent rien.
static mut SUIVI_VERDICT: anneau::SuiviVerdict = anneau::SuiviVerdict::nouveau();
/// Demandes de reprise repoussees parce qu'un verdict courait encore.
static REPRISES_DIFFEREES: AtomicU64 = AtomicU64::new(0);
/// Fenetre laissee a une reprise pour faire ses preuves.
const FENETRE_VERDICT_NS: u64 = 3_000_000_000;

static REPARATION_DEMANDEE: AtomicBool = AtomicBool::new(false);

// BOUCHAUD_HOTFIX9_RX_PROOF_GATE_V1
static VERDICT_EN_ATTENTE: AtomicBool = AtomicBool::new(false);
static REPARATION_DIFFEREE_EN_ATTENTE: AtomicBool = AtomicBool::new(false);
static RX_OK_SANS_PROGRES_CONSOMMES: AtomicU64 = AtomicU64::new(0);
static RX_OK_SANS_PROGRES_DERNIER_NS: AtomicU64 = AtomicU64::new(0);
static REPARATIONS_REFUSEES_SANS_PREUVE: AtomicU64 = AtomicU64::new(0);
const PREUVE_RX_RECENTE_NS: u64 = 5_000_000_000;
/// Identifiant de revision lu dans `TxConfig`, et la generation qui en decoule.
static XID: AtomicU32 = AtomicU32::new(0);
static mut GENERATION: anneau::Generation = anneau::Generation::Inconnue;

/// Periode minimale entre deux maintenances de statut, en nanosecondes.
///
/// Une maintenance coute deux acces MMIO. La scrutation ARP appelle le
/// drainage en boucle serree pendant cinq cents millisecondes : sans borne,
/// nous passerions ce temps a lire un registre. Une milliseconde laisse mille
/// occasions par seconde de voir un moteur tomber, ce qui est tres au-dela du
/// besoin.
const MAINTENANCE_PERIODE_NS: u64 = 1_000_000;

/// Periode minimale entre deux examens de sante complets.
const SANTE_PERIODE_NS: u64 = 500_000_000;

/// L'instantane du pilote, pour `netetat` et la blackbox.
#[derive(Clone, Copy, Debug, Default)]
pub struct Releve {
    pub rx_cur: u32,
    pub tx_cur: u32,
    pub rx_paquets: u64,
    pub rx_octets: u64,
    pub tx_paquets: u64,
    pub tx_octets: u64,
    pub rx_dernier_ns: u64,
    pub tx_dernier_ns: u64,
    pub rx_desc_materiel: u32,
    pub rx_desc_processeur: u32,
    pub rx_desc_courant: u32,
    pub chip_cmd: u8,
    pub intr_status: u16,
    pub rx_missed: u32,
    pub isr_lectures: u64,
    pub isr_rx_ok: u64,
    pub isr_rx_err: u64,
    pub isr_rx_overflow: u64,
    pub isr_rx_fifo_over: u64,
    pub isr_tx_err: u64,
    pub isr_link_chg: u64,
    pub isr_system_error: u64,
    pub isr_carte_absente: u64,
    pub rx_rearmements: u64,
    pub rx_reprises: u64,
    pub rx_reprises_echouees: u64,
    pub rx_abandonnees: u64,
    pub rx_abimees: u64,
    pub tx_anneau_plein: u64,
    pub xid: u32,
    pub generation: &'static str,
    pub invariant: Option<&'static str>,
    /// La progression du materiel dans l'anneau. Voir les compteurs.
    pub rx_tete_cpu: u64,
    pub rx_dernier_desc_cpu: u64,
    pub rx_own_rendus: u64,
    pub rx_ok_sans_descripteur: u64,
    pub rx_arret_detecte: u64,
    pub reparations_demandees: u64,
    pub reparations_executees: u64,
    pub reparation_degre: u64,
    pub reparation_raison: u64,
    pub reparation_derniere_ns: u64,
    pub reprises_differees: u64,
    pub reprises_sans_effet: u32,
    pub reparations_refusees_sans_preuve: u64,
    pub preuves_rx_sans_progres_consommees: u64,
    pub reparation_differee_en_attente: bool,
    pub verdict_en_attente: bool,
    /// L'emission, prouvee par le materiel et non par la mise en file.
    pub tx_enfiles: u64,
    pub tx_termines: u64,
    pub tx_ok_isr: u64,
    pub tx_dernier_termine_ns: u64,
    pub tx_desc_possedes: u32,
    /// Les tours d'anneau : seul le SECOND prouve que l'anneau circule.
    pub rx_tours_cpu: u64,
    pub rx_rendus_tour1: u64,
    pub rx_rendus_tour2: u64,
    pub rx_rearmes_tour1: u64,
    pub rx_reutilises_tour2: u64,
    /// `RxOK` monte, reception figee, aucun descripteur rendu.
    pub rx_ok_sans_progres: u64,
    /// L'etat du pilote : presence, attachement, service.
    pub pilote: u8,
    pub reinitialisations: u64,
    pub reinitialisations_ok: u64,
}

/// L'etat du pilote, en une lecture. Sans effet de bord sur le materiel.
pub fn releve() -> Releve {
    unsafe {
        if !READY {
            return Releve::default();
        }
        let recensement = anneau::recense(N_RX, |i| desc_read32(RX_RING, i, 0));
        Releve {
            rx_cur: RX_CUR as u32,
            tx_cur: TX_CUR as u32,
            rx_paquets: RX_PAQUETS.load(Ordering::Relaxed),
            rx_octets: RX_OCTETS.load(Ordering::Relaxed),
            tx_paquets: TX_PAQUETS.load(Ordering::Relaxed),
            tx_octets: TX_OCTETS.load(Ordering::Relaxed),
            rx_dernier_ns: RX_DERNIER_NS.load(Ordering::Relaxed),
            tx_dernier_ns: TX_DERNIER_NS.load(Ordering::Relaxed),
            rx_desc_materiel: recensement.materiel as u32,
            rx_desc_processeur: recensement.processeur as u32,
            rx_desc_courant: desc_read32(RX_RING, RX_CUR, 0),
            chip_cmd: read8(REG_CHIP_CMD),
            // LECTURE SEULE : `releve` n'acquitte rien. Un diagnostic qui
            // modifie ce qu'il observe efface la panne qu'il doit nommer.
            intr_status: read16(REG_INTR_STATUS),
            rx_missed: read32(REG_RX_MISSED),
            isr_lectures: ISR_LECTURES.load(Ordering::Relaxed),
            isr_rx_ok: ISR_RX_OK.load(Ordering::Relaxed),
            isr_rx_err: ISR_RX_ERR.load(Ordering::Relaxed),
            isr_rx_overflow: ISR_RX_OVERFLOW.load(Ordering::Relaxed),
            isr_rx_fifo_over: ISR_RX_FIFO_OVER.load(Ordering::Relaxed),
            isr_tx_err: ISR_TX_ERR.load(Ordering::Relaxed),
            isr_link_chg: ISR_LINK_CHG.load(Ordering::Relaxed),
            isr_system_error: ISR_SYSTEM_ERROR.load(Ordering::Relaxed),
            isr_carte_absente: ISR_CARTE_ABSENTE.load(Ordering::Relaxed),
            rx_rearmements: RX_REARMEMENTS.load(Ordering::Relaxed),
            rx_reprises: RX_REPRISES.load(Ordering::Relaxed),
            rx_reprises_echouees: RX_REPRISES_ECHOUEES.load(Ordering::Relaxed),
            rx_abandonnees: RX_ABANDONNEES.load(Ordering::Relaxed),
            rx_abimees: RX_ABIMEES,
            tx_anneau_plein: TX_RING_FULL,
            xid: XID.load(Ordering::Relaxed),
            generation: anneau::nom(GENERATION),
            invariant: invariant_anneau(),
            rx_tete_cpu: RX_TETE_CPU.load(Ordering::Relaxed),
            rx_dernier_desc_cpu: RX_DERNIER_DESC_CPU.load(Ordering::Relaxed),
            rx_own_rendus: RX_OWN_RENDUS.load(Ordering::Relaxed),
            rx_ok_sans_descripteur: RX_OK_SANS_DESCRIPTEUR.load(Ordering::Relaxed),
            rx_arret_detecte: RX_ARRET_DETECTE.load(Ordering::Relaxed),
            reparations_demandees: REPARATIONS_DEMANDEES.load(Ordering::Relaxed),
            reparations_executees: REPARATIONS_EXECUTEES.load(Ordering::Relaxed),
            reparation_degre: REPARATION_DEGRE.load(Ordering::Relaxed),
            reparation_raison: REPARATION_RAISON.load(Ordering::Relaxed),
            reparation_derniere_ns: REPARATION_DERNIERE_NS.load(Ordering::Relaxed),
            reprises_differees: REPRISES_DIFFEREES.load(Ordering::Relaxed),
            reprises_sans_effet: REPRISES_SANS_EFFET.load(Ordering::Relaxed),
            reparations_refusees_sans_preuve:
                REPARATIONS_REFUSEES_SANS_PREUVE.load(Ordering::Relaxed),
            preuves_rx_sans_progres_consommees:
                RX_OK_SANS_PROGRES_CONSOMMES.load(Ordering::Relaxed),
            reparation_differee_en_attente:
                REPARATION_DIFFEREE_EN_ATTENTE.load(Ordering::Relaxed),
            verdict_en_attente: VERDICT_EN_ATTENTE.load(Ordering::Relaxed),
            tx_enfiles: TX_ENFILES.load(Ordering::Relaxed),
            tx_termines: TX_TERMINES.load(Ordering::Relaxed),
            tx_ok_isr: ISR_TX_OK.load(Ordering::Relaxed),
            tx_dernier_termine_ns: TX_DERNIER_TERMINE_NS.load(Ordering::Relaxed),
            tx_desc_possedes: tx_descripteurs_possedes(),
            rx_tours_cpu: RX_TOURS_CPU.load(Ordering::Relaxed),
            rx_rendus_tour1: RX_RENDUS_TOUR1.load(Ordering::Relaxed),
            rx_rendus_tour2: RX_RENDUS_TOUR2.load(Ordering::Relaxed),
            rx_rearmes_tour1: RX_REARMES_TOUR1.load(Ordering::Relaxed),
            rx_reutilises_tour2: RX_REUTILISES_TOUR2.load(Ordering::Relaxed),
            rx_ok_sans_progres: RX_OK_SANS_PROGRES.load(Ordering::Relaxed),
            pilote: etat_pilote() as u8,
            reinitialisations: REINITIALISATIONS.load(Ordering::Relaxed),
            reinitialisations_ok: REINITIALISATIONS_OK.load(Ordering::Relaxed),
        }
    }
}

/// L'invariant de l'anneau RX, lu sur le materiel.
unsafe fn invariant_anneau() -> Option<&'static str> {
    anneau::invariant_casse(
        N_RX,
        RX_BUFFER_P,
        anneau::TAILLE_TAMPON,
        |i| desc_read32(RX_RING, i, 0),
        |i| read_volatile(RX_RING.add(i * DESC_SIZE + 8) as *const u64),
    )
}

#[inline]
unsafe fn read8(offset: u32) -> u8 {
    read_volatile((MMIO + u64::from(offset)) as *const u8)
}

#[inline]
unsafe fn read16(offset: u32) -> u16 {
    read_volatile((MMIO + u64::from(offset)) as *const u16)
}

#[inline]
unsafe fn read32(offset: u32) -> u32 {
    read_volatile((MMIO + u64::from(offset)) as *const u32)
}

#[inline]
unsafe fn write8(offset: u32, value: u8) {
    write_volatile((MMIO + u64::from(offset)) as *mut u8, value);
}

#[inline]
unsafe fn write16(offset: u32, value: u16) {
    write_volatile((MMIO + u64::from(offset)) as *mut u16, value);
}

#[inline]
unsafe fn write32(offset: u32, value: u32) {
    write_volatile((MMIO + u64::from(offset)) as *mut u32, value);
}

#[inline]
unsafe fn desc_read64(ring: *mut u8, index: usize, offset: usize) -> u64 {
    core::ptr::read_volatile(ring.add(index * DESC_SIZE + offset) as *const u64)
}

unsafe fn desc_read32(ring: *mut u8, index: usize, offset: usize) -> u32 {
    read_volatile(ring.add(index * DESC_SIZE + offset) as *const u32)
}

#[inline]
unsafe fn desc_write32(ring: *mut u8, index: usize, offset: usize, value: u32) {
    write_volatile(ring.add(index * DESC_SIZE + offset) as *mut u32, value);
}

#[inline]
unsafe fn desc_write64(ring: *mut u8, index: usize, offset: usize, value: u64) {
    write_volatile(ring.add(index * DESC_SIZE + offset) as *mut u64, value);
}

fn memory_bar(device: &PciDevice) -> Option<u64> {
    let mut index = 0u8;
    while index < 6 {
        let bar = pci::bar_decode(device, index);
        match bar {
            Bar::Memoire32 { adresse, .. } => return Some(u64::from(adresse)),
            Bar::Memoire64 { adresse, .. } => return Some(adresse),
            Bar::Port(_) | Bar::Absent => {}
        }
        index += if bar.double() { 2 } else { 1 };
    }
    None
}

fn valid_mac(mac: &[u8; 6]) -> bool {
    mac.iter().any(|&b| b != 0) && mac.iter().any(|&b| b != 0xFF)
}

pub fn is_supported(device: &PciDevice) -> bool {
    device.vendor == VENDOR_REALTEK && device.device == DEVICE_RTL8168
}

pub fn is_ready() -> bool {
    unsafe { READY }
}

pub fn mac() -> [u8; 6] {
    unsafe { MAC }
}

pub fn tx_anneau_plein() -> u64 {
    unsafe { TX_RING_FULL }
}

/// Trames recues en erreur et jetees par le pilote.
pub fn rx_abimees() -> u64 {
    unsafe { RX_ABIMEES }
}

pub fn init_with_device(device: &PciDevice) -> bool {
    unsafe {
        if READY {
            return true;
        }
    }

    if !is_supported(device) {
        return false;
    }

    let bar = match memory_bar(device) {
        Some(address) if address != 0 => address,
        _ => {
            dmesg::log("rtl8168: aucun BAR MMIO exploitable");
            return false;
        }
    };

    crate::serial_println!(
        "BOUCHAUD_TRIGKEY_RTL8168_DETECTED pci={:04x}:{:04x} bus={:02x}:{:02x}.{} mmio={:#x}",
        device.vendor,
        device.device,
        device.bus,
        device.slot,
        device.func,
        bar,
    );

    pci::enable_bus_master(device);

    unsafe {
        // L'IDENTITE D'ABORD, ET POUR TOUJOURS.
        //
        // Sans elle, une reprise de degre quatre n'a aucun moyen de retrouver
        // la carte : c'est pour cela que l'ancienne version ne pouvait que se
        // declarer absente.
        BAR_PHYSIQUE = bar;
        PCI_BUS = device.bus;
        PCI_SLOT = device.slot;
        PCI_FONCTION = device.func;
        VUE_SUR_LE_BUS.store(true, Ordering::Relaxed);
        pose_etat(crate::drivers::etat_pilote::Etat::Detecte);

        MMIO = memory::phys_offset().wrapping_add(bar);

        // Arrete Rx/Tx puis reinitialise le MAC. Boucle bornee : un controleur
        // qui ne sort pas du reset ne doit jamais figer le boot du bureau.
        write8(REG_CHIP_CMD, 0);
        write16(REG_INTR_MASK, 0);
        write16(REG_INTR_STATUS, 0xFFFF);
        write8(REG_CHIP_CMD, CMD_RESET);

        let mut reset_ok = false;
        for _ in 0..RESET_SPINS {
            if read8(REG_CHIP_CMD) & CMD_RESET == 0 {
                reset_ok = true;
                break;
            }
            core::hint::spin_loop();
        }
        if !reset_ok {
            dmesg::log("rtl8168: reset timeout");
            MMIO = 0;
            return false;
        }

        let mut detected_mac = [0u8; 6];
        for (index, byte) in detected_mac.iter_mut().enumerate() {
            *byte = read8(REG_MAC0 + index as u32);
        }
        if !valid_mac(&detected_mac) {
            dmesg::log("rtl8168: adresse MAC invalide");
            MMIO = 0;
            return false;
        }
        MAC = detected_mac;

        // LES ANNEAUX NE S'ALLOUENT QU'UNE FOIS.
        //
        // Une reprise qui reallouerait ses anneaux DMA a chaque tentative
        // finirait par epuiser la memoire basse -- et une carte qui tombe
        // toutes les minutes tombe souvent.
        let (rx_ring_p, rx_ring_v) = match memory::alloc_dma(N_RX * DESC_SIZE) {
            Some(pair) => pair,
            None => {
                dmesg::log("rtl8168: allocation DMA anneau RX impossible");
                MMIO = 0;
                return false;
            }
        };
        let (tx_ring_p, tx_ring_v) = match memory::alloc_dma(N_TX * DESC_SIZE) {
            Some(pair) => pair,
            None => {
                dmesg::log("rtl8168: allocation DMA anneau TX impossible");
                MMIO = 0;
                return false;
            }
        };
        let (rx_buffer_p, rx_buffer_v) = match memory::alloc_dma(N_RX * BUF_SIZE) {
            Some(pair) => pair,
            None => {
                dmesg::log("rtl8168: allocation DMA buffers RX impossible");
                MMIO = 0;
                return false;
            }
        };
        let (tx_buffer_p, tx_buffer_v) = match memory::alloc_dma(N_TX * BUF_SIZE) {
            Some(pair) => pair,
            None => {
                dmesg::log("rtl8168: allocation DMA buffers TX impossible");
                MMIO = 0;
                return false;
            }
        };

        RX_RING = rx_ring_v;
        RX_RING_P = rx_ring_p;
        TX_RING = tx_ring_v;
        TX_RING_P = tx_ring_p;
        RX_BUFFER_P = rx_buffer_p;
        RX_BUFFER_V = rx_buffer_v;
        TX_BUFFER_P = tx_buffer_p;
        TX_BUFFER_V = tx_buffer_v;
        RX_CUR = 0;
        TX_CUR = 0;

        for index in 0..N_RX {
            desc_write32(RX_RING, index, 4, 0);
            desc_write64(
                RX_RING,
                index,
                8,
                anneau::adresse_tampon(rx_buffer_p, index, anneau::TAILLE_TAMPON),
            );
            desc_write32(
                RX_RING,
                index,
                0,
                anneau::opts1_rendu(index, N_RX, anneau::TAILLE_TAMPON),
            );
        }
        for index in 0..N_TX {
            let eor = if index + 1 == N_TX { DESC_EOR } else { 0 };
            desc_write32(TX_RING, index, 0, eor);
            desc_write32(TX_RING, index, 4, 0);
            desc_write64(
                TX_RING,
                index,
                8,
                tx_buffer_p + (index * BUF_SIZE) as u64,
            );
        }

        pose_etat(crate::drivers::etat_pilote::Etat::Lie);
    }

    if !unsafe { programme_le_materiel() } {
        pose_etat(crate::drivers::etat_pilote::Etat::Echec);
        return false;
    }

    let m = mac();
    let raw_version = unsafe { read32(REG_TX_CONFIG) };
    let identifiant = XID.load(Ordering::Relaxed);
    let (generation, rxcfg, chip) = unsafe {
        (anneau::nom(GENERATION), read32(REG_RX_CONFIG), read8(REG_CHIP_CMD))
    };
    dmesg::log_fmt(format_args!(
        "rtl8168: initialise MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} txcfg={:#010x} xid={:#05x} {}",
        m[0], m[1], m[2], m[3], m[4], m[5], raw_version, identifiant, generation,
    ));
    let _ = (rxcfg, chip);
    true
}

/// PROGRAMME LE MATERIEL, SANS RIEN REALLOUER.
///
/// # Pourquoi cette fonction existe
///
/// La reprise de degre quatre faisait ceci :
///
/// ```text
/// READY = false;
/// MMIO = 0;
/// return false;
/// ```
///
/// Ce n'est pas une reinitialisation, c'est une desactivation definitive du
/// pilote. Le releve du 18 septembre le montre a la seconde pres : a t=83,3 s
/// tous les compteurs retombent a zero, `chip_cmd` passe de `0x0c` a `0x00`,
/// et la fenetre Services annonce une carte « absente de ce materiel » --
/// une carte soudee, qui venait de recevoir soixante-quatre trames.
///
/// Une vraie reinitialisation garde l'identite PCI, reutilise les anneaux
/// deja alloues, et remet la puce en service. Elle peut echouer ; elle ne
/// doit jamais faire disparaitre le materiel.
unsafe fn programme_le_materiel() -> bool {
    if BAR_PHYSIQUE == 0 {
        return false;
    }
    MMIO = memory::phys_offset().wrapping_add(BAR_PHYSIQUE);

    // Arrete Rx/Tx, puis reinitialise le MAC.
    write8(REG_CHIP_CMD, 0);
    write16(REG_INTR_MASK, 0);
    write16(REG_INTR_STATUS, 0xFFFF);
    write8(REG_CHIP_CMD, CMD_RESET);
    let mut reset_ok = false;
    for _ in 0..RESET_SPINS {
        if read8(REG_CHIP_CMD) & CMD_RESET == 0 {
            reset_ok = true;
            break;
        }
        core::hint::spin_loop();
    }
    if !reset_ok {
        dmesg::log("rtl8168: reset timeout");
        return false;
    }

    // LES ANNEAUX SONT REMIS A NEUF, PAS REALLOUES.
    RX_CUR = 0;
    TX_CUR = 0;
    for index in 0..N_RX {
        desc_write32(RX_RING, index, 4, 0);
        desc_write64(
            RX_RING,
            index,
            8,
            anneau::adresse_tampon(RX_BUFFER_P, index, anneau::TAILLE_TAMPON),
        );
        desc_write32(
            RX_RING,
            index,
            0,
            anneau::opts1_rendu(index, N_RX, anneau::TAILLE_TAMPON),
        );
    }
    for index in 0..N_TX {
        let eor = if index + 1 == N_TX { DESC_EOR } else { 0 };
        desc_write32(TX_RING, index, 0, eor);
        desc_write32(TX_RING, index, 4, 0);
        desc_write64(TX_RING, index, 8, TX_BUFFER_P + (index * BUF_SIZE) as u64);
    }
    // La barriere DMA : les descripteurs doivent etre visibles AVANT que les
    // moteurs ne partent. Voir `memory::dma_wmb`.
    memory::dma_wmb();

        // LA REVISION DU SILICIUM D'ABORD : elle decide `RxConfig`.
        //
        // `TxConfig` porte l'identifiant de revision, et `rtl8169_init_one` le
        // lit exactement ainsi. Le pilote ecrivait jusqu'ici le meme `RxConfig`
        // pour toute la famille -- celui d'un RTL8169 de 2003.
        let identifiant = anneau::xid(read32(REG_TX_CONFIG));
        let generation = anneau::generation(identifiant);
        XID.store(identifiant, Ordering::Relaxed);
        GENERATION = generation;

        // `CPlusCmd` : on ne GARDE que ce que Linux garde.
        //
        // La version precedente posait `PCIDAC` pour adresser des anneaux
        // au-dessus de quatre gibioctets. Ce n'est pas ainsi qu'un RTL8168 y
        // accede -- les registres `*DescAddrHigh` ci-dessous portent les bits
        // hauts -- et `rtl_init_one` efface ce bit sur toute la famille.
        write8(REG_CFG9346, CFG9346_UNLOCK);

        // LES ECONOMIES D'ENERGIE DU 8168h, COUPEES AVANT DE DEMARRER.
        //
        // ASPM et CLKREQ laissent le lien PCIe descendre en L1 quand il est
        // calme. Le MAC continue alors de recevoir et de lever `RxOK` -- il a
        // ses propres tampons -- pendant que son moteur DMA ne peut plus
        // atteindre la memoire de l'hote.
        //
        // C'est exactement la signature du releve du 18 septembre : un tour
        // d'anneau pendant que le trafic est soutenu, puis plus un descripteur
        // rendu, `RxEnb` toujours arme, `RxMissed` a zero, aucune erreur.
        //
        // `rtl_hw_start_8168h_1` coupe les deux avant de demarrer le
        // materiel. On ne fait que desactiver des economies d'energie : ni le
        // format des descripteurs ni le protocole ne changent.
        if anneau::coupe_les_economies(GENERATION) {
            write8(REG_CONFIG2, anneau::config2_sans_clkreq(read8(REG_CONFIG2)));
            write8(REG_CONFIG5, anneau::config5_sans_aspm(read8(REG_CONFIG5)));
            // `RXDV_GATE` coupe l'alimentation du chemin de reception, et
            // L2/L3 laisse le lien PCIe s'endormir plus bas encore.
            write32(REG_MISC, anneau::misc_chemin_rx_ouvert(read32(REG_MISC)));
            crate::serial_println!(
                "BOUCHAUD_NET_RTL8168_ECONOMIES_COUPEES config2={:#04x} config5={:#04x} misc={:#010x}",
                read8(REG_CONFIG2),
                read8(REG_CONFIG5),
                read32(REG_MISC),
            );
        }

        write16(REG_CPLUS_CMD, anneau::cplus_cmd(read16(REG_CPLUS_CMD)));
        // La taille MAXIMALE acceptee, et non la taille moins un : une trame
        // de exactement `BUF_SIZE` octets tient dans le tampon.
        write16(REG_RX_MAX_SIZE, BUF_SIZE as u16);

        // LES BITS HAUTS COMPTENT. Un anneau au-dela de quatre gibioctets
        // n'est adressable que si `*DescAddrHigh` porte sa moitie haute ; les
        // ecrire systematiquement evite d'avoir a se demander ou l'allocateur
        // a bien voulu poser les anneaux.
        write32(REG_TX_DESC_LOW, TX_RING_P as u32);
        write32(REG_TX_DESC_HIGH, (TX_RING_P >> 32) as u32);
        write32(REG_RX_DESC_LOW, RX_RING_P as u32);
        write32(REG_RX_DESC_HIGH, (RX_RING_P >> 32) as u32);

        write32(REG_TX_CONFIG, TX_DMA_BURST | TX_INTERFRAME_GAP);
        write32(REG_MAR0, 0xFFFF_FFFF);
        write32(REG_MAR0 + 4, 0xFFFF_FFFF);
        write32(REG_RX_MISSED, 0);
        write16(REG_INTR_MASK, 0);
        write16(REG_INTR_STATUS, 0xFFFF);

        compiler_fence(Ordering::Release);
        // L'ORDRE DE `rtl_hw_start` : moteurs d'abord, `RxConfig` ensuite, et
        // les bits d'acceptation EN DERNIER (`rtl_set_rx_mode`). Programmer
        // `RxConfig` apres avoir arme les moteurs est ce que fait la
        // reference, et melanger l'acceptation dans la meme ecriture la ferait
        // perdre a chaque reprogrammation.
        write8(REG_CHIP_CMD, CMD_TX_ENABLE | CMD_RX_ENABLE);
        write32(REG_RX_CONFIG, anneau::rx_config(generation));
        write32(
            REG_RX_CONFIG,
            (read32(REG_RX_CONFIG) & !anneau::MASQUE_ACCEPTATION) | ACCEPTATION,
        );
        write8(REG_CFG9346, CFG9346_LOCK);

    pose_etat(crate::drivers::etat_pilote::Etat::Pret);

    let m = mac();
    let raw_version = unsafe { read32(REG_TX_CONFIG) };
    let identifiant = XID.load(Ordering::Relaxed);
    let (generation, rxcfg, chip) = unsafe {
        (anneau::nom(GENERATION), read32(REG_RX_CONFIG), read8(REG_CHIP_CMD))
    };
    dmesg::log_fmt(format_args!(
        "rtl8168: initialise MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} txcfg={:#010x} xid={:#05x} {}",
        m[0], m[1], m[2], m[3], m[4], m[5], raw_version, identifiant, generation,
    ));
    // LA REVISION DANS LE RELEVE, ET PAS SEULEMENT AU DEMARRAGE.
    //
    // Cette ligne-la partait sur la liaison serie huit minutes avant la
    // capture du releve physique du 17 septembre : l'anneau de trace l'avait
    // deja ecrasee, et la revision du silicium -- donc le bon `RxConfig` --
    // etait la seule chose que l'archive ne disait pas. `netetat` la porte
    // desormais aussi.
    crate::serial_println!(
        "BOUCHAUD_TRIGKEY_RTL8168_DRIVER_OK mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} \
xid={:#05x} generation={} rxcfg={:#010x} chip_cmd={:#04x}",
        m[0], m[1], m[2], m[3], m[4], m[5], identifiant, generation, rxcfg, chip,
    );
    // LE PHY D'ABORD, L'ATTENTE ENSUITE.
    //
    // Attendre un lien sans avoir demande de negociation, c'est attendre pour
    // rien : le lien ne monte que si les deux extremites negocient.
    unsafe { relance_autonegociation(); }

    // Une autonegociation cuivre gigabit prend couramment plus d'une seconde.
    // `net::demarre()` teste le lien juste apres `init()`, donc on lui laisse
    // une fenetre BORNEE avant de conclure que le cable est debranche.
    let start_ms = crate::kernel::timer::monotonic_ms();
    let deadline_ms = start_ms.saturating_add(LINK_WAIT_MS);
    let mut fallback_spins = 0usize;

    while !link_up() {
        let now_ms = crate::kernel::timer::monotonic_ms();

        if now_ms != start_ms && now_ms >= deadline_ms {
            break;
        }

        fallback_spins = fallback_spins.saturating_add(1);
        if fallback_spins >= 20_000_000 {
            crate::serial_println!(
                "BOUCHAUD_TRIGKEY_RTL8168_LINK_WAIT_FALLBACK_TIMEOUT"
            );
            break;
        }

        core::hint::spin_loop();
    }

    if link_up() {
        crate::serial_println!(
            "BOUCHAUD_TRIGKEY_RTL8168_LINK_UP vitesse_mbps={} duplex={}",
            vitesse_mbps(),
            if duplex_complet() { "complet" } else { "alternat" },
        );
    } else {
        crate::serial_println!("BOUCHAUD_TRIGKEY_RTL8168_LINK_DOWN");
    }
    true
}

/// L'etat du pilote : presence, attachement, service -- quatre faits.
pub fn etat_pilote() -> crate::drivers::etat_pilote::Etat {
    use crate::drivers::etat_pilote::Etat;
    match ETAT_PILOTE.load(Ordering::Relaxed) {
        1 => Etat::Detecte,
        2 => Etat::Lie,
        3 => Etat::Pret,
        4 => Etat::Reprise,
        5 => Etat::Echec,
        _ => Etat::Absent,
    }
}

/// Pose l'etat du pilote, en refusant les transitions interdites.
///
/// La seule qui compte vraiment : RIEN ne ramene a `Absent`. C'est celle que
/// l'ancienne reprise de degre quatre effectuait, et qui faisait disparaitre
/// une carte soudee du rapport.
fn pose_etat(vers: crate::drivers::etat_pilote::Etat) {
    use crate::drivers::etat_pilote::{transition_permise, Etat};
    let depuis = etat_pilote();
    if !transition_permise(depuis, vers) {
        crate::serial_println!(
            "BOUCHAUD_NET_RTL8168_ETAT_REFUSE depuis={} vers={}",
            depuis.nom(),
            vers.nom(),
        );
        return;
    }
    if depuis != vers {
        crate::serial_println!(
            "BOUCHAUD_NET_RTL8168_ETAT depuis={} vers={}",
            depuis.nom(),
            vers.nom(),
        );
    }
    ETAT_PILOTE.store(vers as u64, Ordering::Relaxed);
    unsafe {
        READY = matches!(vers, Etat::Pret);
    }
}

/// La puce est-elle sur le bus ? Independant de l'etat du pilote.
///
/// Une carte soudee ne s'en va pas parce que notre code a renonce.
pub fn presente() -> bool {
    VUE_SUR_LE_BUS.load(Ordering::Relaxed)
}

/// Un pilote est-il attache a cette puce ?
pub fn attache() -> bool {
    etat_pilote().attache()
}

pub fn link_up() -> bool {
    unsafe { READY && (read8(REG_PHY_STATUS) & PHY_LINK_STATUS != 0) }
}

pub fn send(frame: &[u8]) -> bool {
    if frame.is_empty() || frame.len() > BUF_SIZE {
        return false;
    }

    unsafe {
        if !READY {
            return false;
        }

        let index = TX_CUR;
        let previous = desc_read32(TX_RING, index, 0);
        if previous & DESC_OWN != 0 {
            TX_RING_FULL = TX_RING_FULL.saturating_add(1);
            return false;
        }

        let destination = TX_BUFFER_V.add(index * BUF_SIZE);
        core::ptr::copy_nonoverlapping(frame.as_ptr(), destination, frame.len());
        // Bourrage a soixante octets : voir `TRAME_MIN`. Des zeros, parce que
        // le contenu du remplissage n'a pas de sens et ne doit pas laisser
        // filtrer ce que le tampon DMA contenait au tour precedent.
        let longueur = frame.len().max(TRAME_MIN).min(BUF_SIZE);
        if longueur > frame.len() {
            core::ptr::write_bytes(destination.add(frame.len()), 0, longueur - frame.len());
        }

        let eor = if index + 1 == N_TX { DESC_EOR } else { 0 };
        desc_write64(
            TX_RING,
            index,
            8,
            TX_BUFFER_P + (index * BUF_SIZE) as u64,
        );
        desc_write32(TX_RING, index, 4, 0);
        compiler_fence(Ordering::Release);
        desc_write32(
            TX_RING,
            index,
            0,
            DESC_OWN | DESC_FS | DESC_LS | eor | longueur as u32,
        );
        compiler_fence(Ordering::Release);
        write8(REG_TX_POLL, TX_POLL_NPQ);
        TX_CUR = anneau::suivant(index, N_TX);
        let maintenant = crate::kernel::timer::monotonic_ns();
        TX_PAQUETS.fetch_add(1, Ordering::Relaxed);
        // ENFILE n'est pas PARTI. La preuve filaire, c'est le descripteur que
        // le materiel rend ; elle se compte dans `moissonne_les_emissions`.
        TX_ENFILES.fetch_add(1, Ordering::Relaxed);
        TX_OCTETS.fetch_add(longueur as u64, Ordering::Relaxed);
        TX_DERNIER_NS.store(maintenant, Ordering::Relaxed);
        // Le PREMIER instant d'emission, et lui seul, sert de reference quand
        // rien n'a jamais ete recu. Voir `anneau_rx::Sante::tx_premier_ns`.
        let _ = TX_PREMIER_NS.compare_exchange(
            0,
            maintenant,
            Ordering::AcqRel,
            Ordering::Relaxed,
        );
        true
    }
}

// ---------------------------------------------------------------------------
// BOUCHAUD_RTL8168_RX_VIVANT_V1 : la maintenance d'un pilote qui SCRUTE
// ---------------------------------------------------------------------------

/// Lit `IntrStatus`, compte ce qu'il porte, et l'acquitte.
///
/// # Le trou que cette fonction bouche
///
/// `IntrStatus` n'etait acquitte qu'a l'initialisation. Le pilote scrute, le
/// masque d'interruption est nul, et personne n'a jamais relu ce registre
/// ensuite : tout ce que le controleur y a signale pendant des heures --
/// debordement de file, plus de descripteur, erreur systeme -- etait invisible
/// ET restait verrouille.
///
/// U-Boot, qui scrute lui aussi, fait exactement cela dans `rtl_recv_common`,
/// et le fait DANS la branche ou le descripteur courant porte encore `OWN` --
/// c'est-a-dire au moment precis ou la reception semble vide. C'est la que
/// nous l'appelons.
///
/// Rend le statut LU (avant acquittement), ou zero.
unsafe fn maintenance_isr() -> u16 {
    let status = read16(REG_INTR_STATUS);
    ISR_LECTURES.fetch_add(1, Ordering::Relaxed);
    if anneau::carte_absente(status) {
        ISR_CARTE_ABSENTE.fetch_add(1, Ordering::Relaxed);
        return 0;
    }
    if status == 0 {
        return 0;
    }
    if status & anneau::isr::RX_OK != 0 { ISR_RX_OK.fetch_add(1, Ordering::Relaxed); }
    if status & anneau::isr::RX_ERR != 0 { ISR_RX_ERR.fetch_add(1, Ordering::Relaxed); }
    if status & anneau::isr::RX_OVERFLOW != 0 { ISR_RX_OVERFLOW.fetch_add(1, Ordering::Relaxed); }
    if status & anneau::isr::RX_FIFO_OVER != 0 { ISR_RX_FIFO_OVER.fetch_add(1, Ordering::Relaxed); }
    if status & anneau::isr::TX_OK != 0 { ISR_TX_OK.fetch_add(1, Ordering::Relaxed); }
    if status & anneau::isr::TX_ERR != 0 { ISR_TX_ERR.fetch_add(1, Ordering::Relaxed); }
    if status & anneau::isr::LINK_CHG != 0 { ISR_LINK_CHG.fetch_add(1, Ordering::Relaxed); }
    if status & anneau::isr::SYS_ERR != 0 { ISR_SYSTEM_ERROR.fetch_add(1, Ordering::Relaxed); }
    let a_ecrire = anneau::a_acquitter(status);
    if a_ecrire != 0 {
        write16(REG_INTR_STATUS, a_ecrire);
    }
    status
}

/// Combien de descripteurs d'emission le MATERIEL possede encore.
///
/// Zero : tout est parti. Non nul : autant de trames sont encore en vol, ou
/// bloquees.
unsafe fn tx_descripteurs_possedes() -> u32 {
    if !READY {
        return 0;
    }
    let mut possedes = 0u32;
    for index in 0..N_TX {
        if desc_read32(TX_RING, index, 0) & anneau::OWN != 0 {
            possedes += 1;
        }
    }
    possedes
}

/// Compte les emissions que le materiel a ACHEVEES.
///
/// Le descripteur rendu (bit `OWN` retombe) est la seule preuve que la trame
/// est partie sur le cable. Appelee depuis la maintenance : elle ne coute
/// qu'une lecture par descripteur, et seulement quand l'anneau est au repos.
unsafe fn moissonne_les_emissions() {
    if !READY {
        return;
    }
    let possedes = tx_descripteurs_possedes() as u64;
    let enfiles = TX_ENFILES.load(Ordering::Relaxed);
    // Ce qui a ete enfile et que le materiel ne detient plus est parti.
    let termines = enfiles.saturating_sub(possedes);
    let connus = TX_TERMINES.load(Ordering::Relaxed);
    if termines > connus {
        TX_TERMINES.store(termines, Ordering::Relaxed);
        TX_DERNIER_TERMINE_NS.store(
            crate::kernel::timer::monotonic_ns(),
            Ordering::Relaxed,
        );
    }
}

/// CAPTURE LE PASSAGE 63 -> 0, UNE SEULE FOIS PAR TOUR, ET AU PLUS DEUX TOURS.
///
/// # Ce qu'on cherche a savoir
///
/// Au premier bouclage, le processeur a rearme les soixante-quatre
/// descripteurs et les rend tous au materiel. Le releve du 18 septembre dit
/// qu'a partir de la, le materiel n'en remplit plus aucun -- tout en
/// continuant a signaler `RxOK`, sans `RxMissed`, sans erreur, et avec
/// `RxEnb` toujours arme.
///
/// Trois explications restent ouvertes : le materiel a quitte notre anneau,
/// il lit un anneau different de celui que nous ecrivons, ou il attend
/// quelque chose que nous ne lui donnons pas. Les registres ci-dessous les
/// separent -- et il faut les lire AU MOMENT du bouclage, pas trente secondes
/// apres.
///
/// Deux lignes en tout pour une session : pas un journal par paquet.
// ---------------------------------------------------------------------------
// LA CAPTURE LAB : CE QUE LE MATERIEL VOIT, AU MOMENT OU IL LE VOIT
// ---------------------------------------------------------------------------
//
// # Pourquoi ces captures ne passent plus par COM1
//
// La version precedente ecrivait tout dans `serial_println!`. Le releve
// TRIGKEY dit ce que cela vaut sur cette machine :
//
// ```text
// com1=bus-flottant  serial_bytes=0
// ```
//
// Pas un octet. Les captures de bouclage de toute la campagne precedente ont
// ete ecrites dans un port serie qui n'existe pas. Elles vont desormais dans
// l'anneau LAB -- de la RAM, lisible par le shell, la boite noire, le
// debugger distant et la telemetrie -- et la ligne serie reste en doublon,
// gratuite la ou elle marche.
//
// # Ce que la capture doit prouver, et ne prouve pas
//
// Elle ne repare rien. Le releve physique montre un premier tour parfait puis
// cent cinquante et une secondes de rien, avec `RxEnb` arme, `rx_missed=0` et
// `isr_rx_ok` qui monte. La question ouverte est : que voit le MATERIEL ?
// D'ou la relecture de `RxDescAddr` DANS LA PUCE plutot que la confiance en
// `RX_RING_P`, la carte des soixante-quatre bits `OWN`, et les descripteurs
// 63 et 0 avec leurs deux mots de statut et leur adresse DMA.
//
// `opts1_rendu()` repose deja `OWN | EOR | longueur` sur le dernier
// descripteur. On ne touche donc PAS a `EOR` : on va voir si la puce est
// d'accord.

/// Les raisons possibles d'une capture. Passees en `arg1` de `RX_REGISTRES`.
pub mod raison_capture {
    pub const BOUCLAGE_AVANT: u64 = 1;
    pub const BOUCLAGE_APRES: u64 = 2;
    pub const SECOND_TOUR_ABSENT: u64 = 3;
    pub const DMA_STALL: u64 = 4;
    pub const INVARIANT: u64 = 5;
    pub const DEMANDE: u64 = 6;
}

/// La carte des soixante-quatre bits `OWN`, un bit par descripteur.
///
/// Bit `i` a un : le MATERIEL possede le descripteur `i`. Le releve physique
/// donne `desc_nic=64 desc_cpu=0`, soit `0xffff_ffff_ffff_ffff` -- et c'est
/// precisement ce qui rend la panne muette : un anneau entierement rendu ne
/// peut produire aucune erreur, aucun `RxMissed`, aucun bit de statut.
unsafe fn carte_own() -> u64 {
    let mut carte = 0u64;
    for i in 0..N_RX.min(64) {
        if desc_read32(RX_RING, i, 0) & DESC_OWN != 0 {
            carte |= 1u64 << i;
        }
    }
    carte
}

/// Un descripteur, tel qu'il est en memoire : `opts1`, `opts2`, adresse DMA.
unsafe fn emet_descripteur(index: usize) {
    lab::emets(
        lab::Categorie::Rtl8168,
        lab::id::RX_DESC,
        [
            index as u64,
            desc_read32(RX_RING, index, 0) as u64,
            desc_read32(RX_RING, index, 4) as u64,
            desc_read64(RX_RING, index, 8),
        ],
    );
}

/// LA CAPTURE COMPLETE. Registres, descripteurs, adresses, energie.
///
/// Sans allocation et sans verrou : elle est appelable depuis le drainage
/// verrouille comme depuis l'auditeur.
pub fn capture_lab(raison: u64) {
    unsafe {
        if !READY {
            return;
        }
        // 1. Les registres que la panne met en cause.
        lab::emets(
            lab::Categorie::Rtl8168,
            lab::id::RX_REGISTRES,
            [
                read8(REG_CHIP_CMD) as u64,
                read16(REG_INTR_STATUS) as u64,
                read32(REG_RX_CONFIG) as u64,
                read16(REG_CPLUS_CMD) as u64,
            ],
        );

        // 2. Qui possede quoi, sur les soixante-quatre descripteurs.
        let recensement = anneau::recense(N_RX, |i| desc_read32(RX_RING, i, 0));
        lab::emets(
            lab::Categorie::Rtl8168,
            lab::id::RX_OWN_MAP,
            [
                carte_own(),
                recensement.materiel as u64,
                recensement.processeur as u64,
                RX_CUR as u64,
            ],
        );

        // 3. LES DEUX DESCRIPTEURS DE LA CHARNIERE. Le 63 porte `EOR`, le 0
        //    est celui que le materiel doit reprendre au second tour. La
        //    panne vit entre les deux.
        emet_descripteur(N_RX - 1);
        emet_descripteur(0);
        if RX_CUR != 0 && RX_CUR != N_RX - 1 {
            emet_descripteur(RX_CUR);
        }

        // 4. L'ADRESSE, RELUE DANS LA PUCE ET NON SUPPOSEE.
        //
        // `RX_RING_P` est ce que NOUS avons ecrit. Ce qui compte est ce que la
        // carte a garde : un registre reecrit par une reprise, une
        // reinitialisation partielle ou une economie d'energie s'y verrait, et
        // nulle part ailleurs.
        let relu = (read32(REG_RX_DESC_LOW) as u64)
            | ((read32(REG_RX_DESC_HIGH) as u64) << 32);
        lab::emets(
            lab::Categorie::Rtl8168,
            lab::id::RX_DESC_ADDR_RELU,
            [relu, RX_RING_P, (relu == RX_RING_P) as u64, raison],
        );

        // 5. LES ECONOMIES D'ENERGIE. ASPM et CLKREQ laissent le lien PCIe
        //    descendre en L1 : le MAC continue de lever `RxOK` -- il a ses
        //    propres tampons -- pendant que son moteur DMA ne peut plus
        //    atteindre la memoire de l'hote. C'est EXACTEMENT la signature du
        //    releve, et c'est pourquoi on les relit a chaque capture plutot
        //    que de croire qu'elles sont restees coupees.
        lab::emets(
            lab::Categorie::Rtl8168,
            lab::id::RX_ENERGIE,
            [
                read8(REG_CONFIG2) as u64,
                read8(REG_CONFIG5) as u64,
                read32(REG_MISC) as u64,
                1,
            ],
        );

        // 6. Ou en est la circulation de l'anneau.
        lab::emets(
            lab::Categorie::Rtl8168,
            lab::id::RX_PROGRES,
            [
                RX_PAQUETS.load(Ordering::Relaxed),
                RX_RENDUS_TOUR1.load(Ordering::Relaxed),
                RX_RENDUS_TOUR2.load(Ordering::Relaxed),
                RX_TOURS_CPU.load(Ordering::Relaxed),
            ],
        );
    }
}

/// Le bouclage `63 -> 0`, cote processeur : AVANT que `RX_CUR` ne reparte.
unsafe fn capture_le_bouclage(tour: u64, dernier_index: usize) {
    lab::emets(
        lab::Categorie::Rtl8168,
        lab::id::RING_WRAP_BEFORE,
        [
            RX_CUR as u64,
            RX_PAQUETS.load(Ordering::Relaxed),
            desc_read32(RX_RING, dernier_index, 0) as u64,
            desc_read32(RX_RING, 0, 0) as u64,
        ],
    );
    // LES DEUX PREMIERS TOURS SEULEMENT pour la capture complete. Un anneau
    // qui circule boucle des centaines de fois par seconde sous charge, et
    // soixante-dix evenements par tour noieraient tout le reste. Le releve
    // physique n'en a jamais eu qu'un seul a montrer.
    if tour <= 2 {
        capture_lab(raison_capture::BOUCLAGE_AVANT);
        crate::serial_println!(
            "BOUCHAUD_NET_RTL8168_BOUCLAGE tour={} index={} desc_dernier={:#010x} \
desc0={:#010x} desc0_dma={:#018x} anneau_dma={:#018x} \
rx_desc_low={:#010x} rx_desc_high={:#010x} chip_cmd={:#04x} rx_config={:#010x} \
cplus_cmd={:#06x} intr_status={:#06x} intr_mask={:#06x} rx_missed={} rx_max={} \
rx_paquets={} own_rendus={}",
            tour,
            dernier_index,
            desc_read32(RX_RING, dernier_index, 0),
            desc_read32(RX_RING, 0, 0),
            desc_read64(RX_RING, 0, 8),
            RX_RING_P,
            read32(REG_RX_DESC_LOW),
            read32(REG_RX_DESC_HIGH),
            read8(REG_CHIP_CMD),
            read32(REG_RX_CONFIG),
            read16(REG_CPLUS_CMD),
            read16(REG_INTR_STATUS),
            read16(REG_INTR_MASK),
            read32(REG_RX_MISSED),
            read16(REG_RX_MAX_SIZE),
            RX_PAQUETS.load(Ordering::Relaxed),
            RX_OWN_RENDUS.load(Ordering::Relaxed),
        );
    }
}

/// Le meme bouclage, APRES que `RX_CUR` soit revenu a zero.
///
/// # Pourquoi deux captures pour un seul instant
///
/// Entre les deux, une seule chose change de notre cote : le curseur
/// logiciel. Si un registre materiel, un bit `OWN` ou l'adresse relue bouge
/// entre « avant » et « apres », ce n'est PAS nous qui l'avons fait -- et
/// c'est exactement le genre de fait qui manque pour nommer la panne.
unsafe fn capture_apres_bouclage(tour: u64, dernier_index: usize) {
    lab::emets(
        lab::Categorie::Rtl8168,
        lab::id::RING_WRAP_AFTER,
        [
            RX_CUR as u64,
            RX_PAQUETS.load(Ordering::Relaxed),
            desc_read32(RX_RING, dernier_index, 0) as u64,
            desc_read32(RX_RING, 0, 0) as u64,
        ],
    );
    if tour <= 2 {
        capture_lab(raison_capture::BOUCLAGE_APRES);
    }
}

/// `RxOK` A-T-IL PROGRESSE SANS QUE LA RECEPTION PROGRESSE ?
///
/// Les trois conditions ensemble, sur une fenetre d'une seconde :
///
/// 1. `RxOK` a monte ;
/// 2. `rx_packets` n'a pas bouge ;
/// 3. aucun descripteur n'est revenu au processeur.
///
/// C'est cela, un `RxOK` sterile. L'ancien compteur ne verifiait que « le
/// descripteur courant porte `OWN` », qui est l'etat normal d'un anneau au
/// repos : il montait a trois pendant que la reception fonctionnait.
unsafe fn juge_le_progres(maintenant: u64) {
    let precedent = TEMOIN_FENETRE_NS.load(Ordering::Relaxed);
    if precedent == 0 {
        TEMOIN_FENETRE_NS.store(maintenant, Ordering::Relaxed);
        TEMOIN_ISR_RX_OK.store(ISR_RX_OK.load(Ordering::Relaxed), Ordering::Relaxed);
        TEMOIN_RX_PAQUETS.store(RX_PAQUETS.load(Ordering::Relaxed), Ordering::Relaxed);
        TEMOIN_OWN_RENDUS.store(RX_OWN_RENDUS.load(Ordering::Relaxed), Ordering::Relaxed);
        return;
    }
    if maintenant.saturating_sub(precedent) < FENETRE_PROGRES_NS {
        return;
    }

    let rx_ok = ISR_RX_OK.load(Ordering::Relaxed);
    let paquets = RX_PAQUETS.load(Ordering::Relaxed);
    let rendus = RX_OWN_RENDUS.load(Ordering::Relaxed);

    if rx_ok > TEMOIN_ISR_RX_OK.load(Ordering::Relaxed)
        && paquets == TEMOIN_RX_PAQUETS.load(Ordering::Relaxed)
        && rendus == TEMOIN_OWN_RENDUS.load(Ordering::Relaxed)
    {
        RX_OK_SANS_PROGRES.fetch_add(1, Ordering::Relaxed);
        RX_OK_SANS_PROGRES_DERNIER_NS.store(maintenant, Ordering::Release);
    }

    TEMOIN_FENETRE_NS.store(maintenant, Ordering::Relaxed);
    TEMOIN_ISR_RX_OK.store(rx_ok, Ordering::Relaxed);
    TEMOIN_RX_PAQUETS.store(paquets, Ordering::Relaxed);
    TEMOIN_OWN_RENDUS.store(rendus, Ordering::Relaxed);
}

/// BOUCHAUD_HOTFIX9_RX_PROOF_GATE_V1
/// Arme UNE reprise, ou UNE reprise differee.
fn demande_reparation(raison: u64, maintenant: u64) -> bool {
    REPARATION_RAISON.store(raison, Ordering::Relaxed);
    REPARATION_DERNIERE_NS.store(maintenant, Ordering::Relaxed);

    if VERDICT_EN_ATTENTE.load(Ordering::Acquire) {
        if REPARATION_DIFFEREE_EN_ATTENTE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            REPARATIONS_DEMANDEES.fetch_add(1, Ordering::Relaxed);
            RX_ARRET_DETECTE.fetch_add(1, Ordering::Relaxed);
            let n = REPRISES_DIFFEREES.fetch_add(1, Ordering::Relaxed) + 1;
            crate::serial_println!(
                "BOUCHAUD_NET_RTL8168_REPRISE_DIFFEREE_MISE_EN_FILE raison={} total={}",
                raison,
                n,
            );
            return true;
        }
        return false;
    }

    if REPARATION_DEMANDEE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        REPARATIONS_DEMANDEES.fetch_add(1, Ordering::Relaxed);
        RX_ARRET_DETECTE.fetch_add(1, Ordering::Relaxed);
        return true;
    }
    false
}

/// La maintenance du passage a vide, bornee en frequence.
///
/// La scrutation ARP appelle le drainage en boucle serree pendant cinq cents
/// millisecondes. Sans borne, nous passerions ce temps a lire un registre
/// PCIe. Une milliseconde laisse mille occasions par seconde de voir un moteur
/// tomber.
unsafe fn maintenance_anneau_vide() {
    let maintenant = crate::kernel::timer::monotonic_ns();
    let precedent = MAINTENANCE_DERNIERE_NS.load(Ordering::Relaxed);
    if precedent != 0 && maintenant.saturating_sub(precedent) < MAINTENANCE_PERIODE_NS {
        return;
    }
    MAINTENANCE_DERNIERE_NS.store(maintenant, Ordering::Relaxed);

    let status = maintenance_isr();
    moissonne_les_emissions();

    if status & anneau::isr::RX_OK != 0 {
        RX_OK_SANS_DESCRIPTEUR.fetch_add(1, Ordering::Relaxed);
    }
    juge_le_progres(maintenant);

    if anneau::moteur_rx_a_relancer(read8(REG_CHIP_CMD)) {
        let _ = demande_reparation(1, maintenant);
    }
}

/// Execute une reparation armee. LE SEUL APPELANT DE `repare_reception`.
///
/// # Pourquoi un seul point, et pourquoi celui-la
///
/// Reconstruire l'anneau, c'est le reecrire en entier et remettre `RX_CUR` a
/// zero. Deux chemins sortent des trames de la carte : le drainage
/// verrouille, qui tient `VERROU_RECEPTION`, et le peripherique smoltcp, qui
/// ne le tient pas. Laisser la reparation partir du second -- ou du veilleur
/// de lien -- reouvrirait exactement la course que ce verrou ferme.
///
/// Cette fonction est donc appelee depuis le drainage verrouille, et de la
/// seulement. Tout le reste se contente de poser le drapeau.
pub fn repare_si_demande() -> bool {
    unsafe {
        if !READY {
            return false;
        }

        if !link_up() {
            SUIVI_VERDICT.reinitialise();
            VERDICT_EN_ATTENTE.store(false, Ordering::Release);
            REPARATION_DIFFEREE_EN_ATTENTE.store(false, Ordering::Release);
            REPARATION_DEMANDEE.store(false, Ordering::Release);
            REPRISES_SANS_EFFET.store(0, Ordering::Relaxed);
            return false;
        }

        let maintenant = crate::kernel::timer::monotonic_ns();

        let issue = SUIVI_VERDICT.conclut_si_du(
            maintenant,
            FENETRE_VERDICT_NS,
            RX_PAQUETS.load(Ordering::Relaxed),
            RX_OWN_RENDUS.load(Ordering::Relaxed),
        );
        let issue_effective = issue.map(|v| v.effective);
        if issue.is_some() {
            VERDICT_EN_ATTENTE.store(false, Ordering::Release);
        }
        publie_le_verdict(issue);

        if issue_effective == Some(true) {
            REPARATION_DIFFEREE_EN_ATTENTE.store(false, Ordering::Release);
            REPARATION_DEMANDEE.store(false, Ordering::Release);
        }

        if SUIVI_VERDICT.en_attente() {
            VERDICT_EN_ATTENTE.store(true, Ordering::Release);
            if REPARATION_DEMANDEE.swap(false, Ordering::AcqRel)
                && REPARATION_DIFFEREE_EN_ATTENTE
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                let n = REPRISES_DIFFEREES.fetch_add(1, Ordering::Relaxed) + 1;
                crate::serial_println!(
                    "BOUCHAUD_NET_RTL8168_REPRISE_DIFFEREE_MISE_EN_FILE raison={} total={}",
                    REPARATION_RAISON.load(Ordering::Relaxed),
                    n,
                );
            }
            return false;
        }

        let directe = REPARATION_DEMANDEE.swap(false, Ordering::AcqRel);
        let differee = REPARATION_DIFFEREE_EN_ATTENTE.swap(false, Ordering::AcqRel);
        if !directe && !differee {
            return false;
        }

        REPRISES_SANS_EFFET.store(SUIVI_VERDICT.sans_effet(), Ordering::Relaxed);

        match repare_reception() {
            None => {
                REPARATION_DIFFEREE_EN_ATTENTE.store(true, Ordering::Release);
                false
            }
            Some(ok) => {
                REPARATIONS_EXECUTEES.fetch_add(1, Ordering::Relaxed);
                SUIVI_VERDICT.arme(
                    crate::kernel::timer::monotonic_ns(),
                    RX_PAQUETS.load(Ordering::Relaxed),
                    RX_OWN_RENDUS.load(Ordering::Relaxed),
                );
                VERDICT_EN_ATTENTE.store(true, Ordering::Release);
                ok
            }
        }
    }
}

/// Trace un verdict rendu, et republie le compteur que la boite noire lit.
unsafe fn publie_le_verdict(issue: Option<anneau::IssueVerdict>) {
    let Some(issue) = issue else { return };
    REPRISES_SANS_EFFET.store(issue.sans_effet, Ordering::Relaxed);
    if issue.effective {
        return;
    }
    crate::serial_println!(
        "BOUCHAUD_NET_RTL8168_REPRISE_SANS_EFFET suite={} age_us={} rx_paquets={} \
own_rendus={}",
        issue.sans_effet,
        issue.age_ns / 1_000,
        RX_PAQUETS.load(Ordering::Relaxed),
        RX_OWN_RENDUS.load(Ordering::Relaxed),
    );
}

/// Rend au materiel tous les descripteurs que le processeur retient.
///
/// Les trames non drainees qui s'y trouvaient sont PERDUES, et comptees comme
/// telles : une trame que personne n'a lue en trois secondes n'interesse plus
/// personne, et la garder bloquerait l'anneau pour de bon.
unsafe fn rearme_descripteurs_retenus() -> u64 {
    let mut rendus = 0u64;
    for index in 0..N_RX {
        if desc_read32(RX_RING, index, 0) & DESC_OWN != 0 {
            continue;
        }
        desc_write64(RX_RING, index, 8, anneau::adresse_tampon(
            RX_BUFFER_P,
            index,
            anneau::TAILLE_TAMPON,
        ));
        desc_write32(RX_RING, index, 4, 0);
        compiler_fence(Ordering::Release);
        desc_write32(
            RX_RING,
            index,
            0,
            anneau::opts1_rendu(index, N_RX, anneau::TAILLE_TAMPON),
        );
        rendus += 1;
    }
    RX_REARMEMENTS.fetch_add(rendus, Ordering::Relaxed);
    rendus
}

/// Reconstruit l'anneau de reception et repositionne le materiel dessus.
unsafe fn reconstruit_anneau() {
    // Le materiel ne doit pas ecrire pendant qu'on deplace ce qu'il lit.
    write8(REG_CHIP_CMD, read8(REG_CHIP_CMD) & !CMD_RX_ENABLE);
    for index in 0..N_RX {
        desc_write32(RX_RING, index, 4, 0);
        desc_write64(RX_RING, index, 8, anneau::adresse_tampon(
            RX_BUFFER_P,
            index,
            anneau::TAILLE_TAMPON,
        ));
        desc_write32(
            RX_RING,
            index,
            0,
            anneau::opts1_rendu(index, N_RX, anneau::TAILLE_TAMPON),
        );
    }
    RX_CUR = 0;
    compiler_fence(Ordering::Release);
    // Reecrire la base remet le pointeur interne du controleur au debut : nos
    // deux curseurs repartent du meme descripteur.
    write32(REG_RX_DESC_LOW, RX_RING_P as u32);
    write32(REG_RX_DESC_HIGH, (RX_RING_P >> 32) as u32);
    compiler_fence(Ordering::Release);
    write8(REG_CHIP_CMD, CMD_TX_ENABLE | CMD_RX_ENABLE);
}

/// L'echelle de reprise, du moins invasif au plus invasif.
///
/// # Ne pas couper le lien pour une anomalie de reception
///
/// Une reinitialisation complete rend l'interface muette, fait retomber le
/// lien et oblige a refaire DHCP. Elle est au dernier barreau, et on n'y monte
/// qu'apres que les precedents ont echoue -- `anneau_rx::degre` encode la
/// regle, et un test hote la contredit sans demarrer la machine.
/// Rend `None` quand le repos entre reprises a parle : RIEN n'a ete fait,
/// donc il n'y a rien a juger. Confondre ce cas avec une reprise executee
/// ferait porter un verdict sur une reparation qui n'a pas eu lieu.
unsafe fn repare_reception() -> Option<bool> {
    let maintenant = crate::kernel::timer::monotonic_ns();
    // UNE REPRISE COUTE DES TRAMES, ET UNE LIGNE DE TRACE.
    //
    // Le chemin qui repond a `RxBufEmpty` est examine mille fois par seconde.
    // Sans cette borne, une puce qui poserait ce bit en permanence ferait
    // mille reprises et mille lignes par seconde : la trace deviendrait
    // illisible au moment precis ou elle doit servir, et l'anneau serait
    // reconstruit sans jamais laisser au moteur le temps de montrer qu'il est
    // reparti.
    let precedente = REPRISE_DERNIERE_NS.load(Ordering::Relaxed);
    if precedente != 0
        && maintenant.saturating_sub(precedente) < anneau::REPOS_ENTRE_REPRISES_NS
    {
        return None;
    }
    REPRISE_DERNIERE_NS.store(maintenant, Ordering::Relaxed);
    RX_REPRISES.fetch_add(1, Ordering::Relaxed);

    let paquets_avant = RX_PAQUETS.load(Ordering::Relaxed);
    let recensement = anneau::recense(N_RX, |i| desc_read32(RX_RING, i, 0));
    let invariant = invariant_anneau();
    let chip = read8(REG_CHIP_CMD);
    let degre = anneau::degre(
        invariant,
        recensement,
        chip,
        REPRISES_SANS_EFFET.load(Ordering::Relaxed),
    );
    REPARATION_DEGRE.store(degre as u64, Ordering::Relaxed);

    // 1 et 2 : drainer ce qui reste et acquitter les statuts. Toujours.
    let status = maintenance_isr();

    // 3 : rendre au materiel les descripteurs que nous retenons.
    if degre >= anneau::Degre::Rearme {
        RX_ABANDONNEES.fetch_add(rearme_descripteurs_retenus(), Ordering::Relaxed);
    }

    // 4 et 5 : verifier `RxEnb`, et relancer LE MOTEUR SEUL.
    if degre >= anneau::Degre::RelanceRx {
        write8(REG_CHIP_CMD, CMD_TX_ENABLE | CMD_RX_ENABLE);
    }

    // 6 : reconstruire l'anneau, seulement si son invariant est casse.
    if degre >= anneau::Degre::ReconstruitAnneau {
        reconstruit_anneau();
    }

    // 7 : reinitialiser la carte, en dernier recours seulement.
    if degre >= anneau::Degre::ReinitialiseCarte {
        // UNE REINITIALISATION, PAS UNE DESACTIVATION.
        //
        // L'ancienne version posait `READY = false` et `MMIO = 0`, puis
        // rendait faux. Le releve du 18 septembre en montre l'effet a la
        // seconde pres : a t=83,3 s tous les compteurs retombent a zero,
        // `chip_cmd` passe de `0x0c` a `0x00`, et la fenetre Services annonce
        // « rtl8168 absente de ce materiel » -- pour une puce soudee qui
        // venait de recevoir soixante-quatre trames.
        //
        // On reprogramme la puce avec les anneaux DEJA alloues, on relance la
        // negociation, et on garde l'identite PCI quoi qu'il arrive.
        REINITIALISATIONS.fetch_add(1, Ordering::Relaxed);
        pose_etat(crate::drivers::etat_pilote::Etat::Reprise);
        let remise_en_service = programme_le_materiel();
        if remise_en_service {
            relance_autonegociation();
            REINITIALISATIONS_OK.fetch_add(1, Ordering::Relaxed);
            // `programme_le_materiel` a deja pose `Pret`.
            crate::kernel::services::etat(
                "net.nic.rtl8168",
                crate::kernel::services::Etat::Reprise,
            );
        } else {
            // La puce est toujours la, le pilote toujours attache : il ne rend
            // simplement plus de service. JAMAIS « absente ».
            pose_etat(crate::drivers::etat_pilote::Etat::Echec);
            RX_REPRISES_ECHOUEES.fetch_add(1, Ordering::Relaxed);
        }
        crate::serial_println!(
            "BOUCHAUD_NET_RTL8168_REPRISE degre=reinitialise remise_en_service={} \
etat={} chip_cmd={:#04x} intr_status={:#06x} desc_materiel={} desc_processeur={} invariant={}",
            remise_en_service as u8,
            etat_pilote().nom(),
            chip,
            status,
            recensement.materiel,
            recensement.processeur,
            invariant.unwrap_or("intact"),
        );
        return Some(remise_en_service);
    }

    crate::kernel::services::etat("net.nic.rtl8168", crate::kernel::services::Etat::Reprise);
    crate::serial_println!(
        "BOUCHAUD_NET_RTL8168_REPRISE degre={} chip_cmd={:#04x} intr_status={:#06x} \
desc_materiel={} desc_processeur={} rx_cur={} invariant={} rx_paquets={}",
        match degre {
            anneau::Degre::Draine => "draine",
            anneau::Degre::Rearme => "rearme",
            anneau::Degre::RelanceRx => "relance-rx",
            anneau::Degre::ReconstruitAnneau => "reconstruit-anneau",
            anneau::Degre::ReinitialiseCarte => "reinitialise",
        },
        chip,
        status,
        recensement.materiel,
        recensement.processeur,
        RX_CUR,
        invariant.unwrap_or("intact"),
        paquets_avant,
    );
    Some(true)
}

/// L'instantane commun aux deux pilotes, vu par le RTL8168.
///
/// Les memes champs que pour l'e1000, remplis avec les compteurs que ce
/// pilote tient deja. Rien n'est invente : `verdict_recuperation` ne juge que
/// des ECARTS entre deux instantanes.
pub fn instantane_rx() -> anneau::InstantaneRx {
    unsafe {
        if !READY {
            return anneau::InstantaneRx::default();
        }
        anneau::InstantaneRx {
            rx_paquets: RX_PAQUETS.load(Ordering::Relaxed),
            rx_cur: RX_CUR,
            // CETTE PUCE N'EXPOSE AUCUN REGISTRE DE TETE MATERIELLE.
            //
            // L'anneau y est pilote par les bits `OWN`, pas par un couple
            // tete/queue comme sur l'e1000. Mettre `RX_TETE_CPU` ici
            // fabriquerait une progression materielle a partir d'un curseur
            // logiciel : le champ reste donc a zero, et le verdict s'appuie
            // sur les criteres que ce pilote sait vraiment fournir.
            rx_tete_materiel: 0,
            dernier_desc_cpu: RX_DERNIER_DESC_CPU.load(Ordering::Relaxed) as usize,
            own_rendus: RX_OWN_RENDUS.load(Ordering::Relaxed),
            isr_rx_ok: ISR_RX_OK.load(Ordering::Relaxed),
            rx_ok_sans_progres: RX_OK_SANS_PROGRES.load(Ordering::Relaxed),
        }
    }
}

/// La reception est-elle arretee ? Si oui, ARME une reparation.
///
/// # Cette fonction ne touche pas au materiel, et c'est voulu
///
/// Elle est appelee par le veilleur de lien, qui ne tient pas le verrou de
/// reception. Reconstruire l'anneau depuis la, pendant que `receive` lit un
/// descripteur, serait exactement la course que ce verrou existe pour fermer.
///
/// Elle ne fait donc que poser un drapeau. La reparation a lieu au prochain
/// passage du drainage, sous le verrou de celui-ci -- et l'appelant declenche
/// ce passage juste apres.
///
/// Rend `true` quand une reparation vient d'etre armee.
pub fn demande_reparation_si_arretee() -> bool {
    unsafe {
        if !READY {
            return false;
        }

        if VERDICT_EN_ATTENTE.load(Ordering::Acquire)
            || REPARATION_DIFFEREE_EN_ATTENTE.load(Ordering::Acquire)
            || REPARATION_DEMANDEE.load(Ordering::Acquire)
        {
            return true;
        }

        let maintenant = crate::kernel::timer::monotonic_ns();
        let precedent = SANTE_DERNIERE_NS.load(Ordering::Relaxed);
        if precedent != 0 && maintenant.saturating_sub(precedent) < SANTE_PERIODE_NS {
            return false;
        }
        SANTE_DERNIERE_NS.store(maintenant, Ordering::Relaxed);

        let sante = anneau::Sante {
            lien: link_up(),
            rx_dernier_ns: RX_DERNIER_NS.load(Ordering::Relaxed),
            tx_dernier_ns: TX_DERNIER_NS.load(Ordering::Relaxed),
            tx_premier_ns: TX_PREMIER_NS.load(Ordering::Relaxed),
            reprise_derniere_ns: REPRISE_DERNIERE_NS.load(Ordering::Relaxed),
        };

        if !anneau::reception_arretee(&sante, maintenant) {
            return false;
        }

        let preuves = RX_OK_SANS_PROGRES.load(Ordering::Acquire);
        let consommees = RX_OK_SANS_PROGRES_CONSOMMES.load(Ordering::Relaxed);
        let preuve_ns = RX_OK_SANS_PROGRES_DERNIER_NS.load(Ordering::Acquire);
        let reference = if sante.rx_dernier_ns == 0 {
            sante.tx_premier_ns
        } else {
            sante.rx_dernier_ns
        };

        let preuve_nouvelle = preuves > consommees
            && preuve_ns != 0
            && preuve_ns >= reference
            && maintenant.saturating_sub(preuve_ns) <= PREUVE_RX_RECENTE_NS;

        let preuve = anneau::PreuveArretRx {
            moteur_a_relancer: false,
            rx_ok_sans_progres_nouveau: preuve_nouvelle,
        };

        if !anneau::reception_arretee_confirmee(&sante, maintenant, preuve) {
            REPARATIONS_REFUSEES_SANS_PREUVE.fetch_add(1, Ordering::Relaxed);
            return false;
        }

        RX_OK_SANS_PROGRES_CONSOMMES.store(preuves, Ordering::Release);

        let armee = demande_reparation(2, maintenant);
        if armee {
            crate::kernel::services::erreur(
                "net.nic.rtl8168",
                "rx-silencieux-confirme",
            );
        }
        armee
    }
}

/// Retire une trame de l'anneau de reception.
///
/// # `None` veut dire « plus rien », et seulement cela
///
/// La premiere version rendait `None` dans DEUX cas differents : l'anneau est
/// vide, et la trame lue est abimee. Or tous les appelants lisent `None` comme
/// « plus rien a lire » et arretent leur drainage : une seule trame en erreur
/// -- une collision, un cable en cours de branchement -- suspendait donc la
/// lecture de tout ce qui la suivait dans l'anneau jusqu'au passage suivant.
/// Sur une resolution ARP, qui ecoute une fenetre bornee, cela suffit a perdre
/// la reponse et a conclure que le voisin est muet.
///
/// La trame abimee est desormais jetee ICI, sans rendre la main : la boucle
/// continue jusqu'a une trame bonne ou un anneau reellement vide. Elle est
/// comptee, parce qu'un compteur qui monte dit quelque chose du cable.
pub fn receive(out: &mut [u8]) -> Option<usize> {
    unsafe {
        if !READY {
            return None;
        }

        // Bornee par la taille de l'anneau : on ne peut pas jeter plus de
        // trames qu'il n'en contient, et la borne EXISTE pour qu'un anneau
        // entierement abime ne retienne pas l'appelant.
        for _ in 0..N_RX {
            let index = RX_CUR;
            let status = desc_read32(RX_RING, index, 0);
            // On ne lit pas un tampon dont on n'a pas d'abord constate qu'il
            // nous appartient. Voir `memory::dma_rmb`.
            memory::dma_rmb();
            // La tete que le PROCESSEUR regarde, a chaque examen. Sans elle,
            // un anneau fige et un anneau qui tourne se lisent pareil.
            RX_TETE_CPU.store(index as u64, Ordering::Relaxed);
            if status & anneau::OWN == 0 {
                // Le materiel a RENDU ce descripteur : transition 1 -> 0.
                RX_OWN_RENDUS.fetch_add(1, Ordering::Relaxed);
                RX_DERNIER_DESC_CPU.store(index as u64, Ordering::Relaxed);
                // PREMIER TOUR OU SECOND ? Seul le second prouve que l'anneau
                // circule.
                if RX_TOURS_CPU.load(Ordering::Relaxed) == 0 {
                    RX_RENDUS_TOUR1.fetch_add(1, Ordering::Relaxed);
                } else {
                    RX_RENDUS_TOUR2.fetch_add(1, Ordering::Relaxed);
                    RX_REUTILISES_TOUR2.fetch_add(1, Ordering::Relaxed);
                }
            }

            let copied = match anneau::examine(status, anneau::TAILLE_TAMPON) {
                anneau::Verdict::Materiel => {
                    // ICI, ET NULLE PART AILLEURS.
                    //
                    // C'est le point ou U-Boot fait sa maintenance de
                    // scrutation : le descripteur courant porte encore `OWN`,
                    // la reception semble vide, et c'est le seul instant ou
                    // relire `IntrStatus` ne coute rien. Un moteur tombe se
                    // voit la, et pas une trame plus tard.
                    maintenance_anneau_vide();
                    return None;
                }
                anneau::Verdict::Bonne(longueur) => {
                    let n = longueur.min(out.len());
                    let source = RX_BUFFER_V.add(index * BUF_SIZE);
                    core::ptr::copy_nonoverlapping(source, out.as_mut_ptr(), n);
                    Some(n)
                }
                anneau::Verdict::Abimee => {
                    RX_ABIMEES = RX_ABIMEES.saturating_add(1);
                    None
                }
            };

            desc_write64(
                RX_RING,
                index,
                8,
                anneau::adresse_tampon(RX_BUFFER_P, index, anneau::TAILLE_TAMPON),
            );
            desc_write32(RX_RING, index, 4, 0);
            // UNE VRAIE BARRIERE, ET NON UN `compiler_fence`.
            //
            // L'adresse et la longueur doivent etre visibles du peripherique
            // AVANT le bit de propriete, sans quoi la carte peut remplir un
            // descripteur qui pointe encore sur l'ancien tampon. Voir
            // `memory::dma_wmb` pour ce que le contrat x86_64 garantit
            // vraiment, et ce qu'il ne garantit pas.
            memory::dma_wmb();
            desc_write32(
                RX_RING,
                index,
                0,
                anneau::opts1_rendu(index, N_RX, anneau::TAILLE_TAMPON),
            );
            // La propriete doit etre partie avant qu'on ne touche a la suite.
            memory::dma_wmb();
            RX_REARMEMENTS.fetch_add(1, Ordering::Relaxed);
            if RX_TOURS_CPU.load(Ordering::Relaxed) == 0 {
                RX_REARMES_TOUR1.fetch_add(1, Ordering::Relaxed);
            }
            let suivant = anneau::suivant(index, N_RX);
            if suivant == 0 {
                // LE PASSAGE 63 -> 0 : le processeur boucle. C'est ici, et
                // seulement ici, qu'un tour s'acheve.
                let tour = RX_TOURS_CPU.fetch_add(1, Ordering::Relaxed) + 1;
                capture_le_bouclage(tour, index);
                RX_CUR = suivant;
                capture_apres_bouclage(tour, index);
            } else {
                RX_CUR = suivant;
            }

            if let Some(n) = copied {
                RX_PAQUETS.fetch_add(1, Ordering::Relaxed);
                RX_OCTETS.fetch_add(n as u64, Ordering::Relaxed);
                RX_DERNIER_NS.store(
                    crate::kernel::timer::monotonic_ns(),
                    Ordering::Relaxed,
                );
                return Some(n);
            }
        }
        None
    }
}

pub fn print_info() {
    if !is_ready() {
        crate::println!("rtl8168: non initialise");
        return;
    }
    let m = mac();
    crate::println!(
        "rtl8168: MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} lien={}",
        m[0],
        m[1],
        m[2],
        m[3],
        m[4],
        m[5],
        if link_up() { "UP" } else { "DOWN" },
    );
}

// ---------------------------------------------------------------------------
// LES LECTEURS PUBLICS : DE LA LECTURE, ET RIEN QUE DE LA LECTURE
// ---------------------------------------------------------------------------
//
// Le shell et le debugger distant ont besoin des descripteurs un par un. Ils
// n'ont AUCUNE raison d'ecrire, et ces fonctions leur en retirent le moyen :
// elles lisent la memoire de l'anneau et deux registres, point.
//
// Un diagnostic qui modifie ce qu'il observe efface la panne qu'il doit
// nommer. `tools/verifie-reseau-sans-triche.py` garde deja cette propriete
// pour `releve()` ; elle vaut ici pour la meme raison.

/// `opts1` du descripteur `index` : `OWN`, `EOR`, `FS`, `LS`, erreurs, longueur.
pub fn desc_opts1(index: usize) -> u32 {
    unsafe {
        if !READY || index >= N_RX {
            return 0;
        }
        desc_read32(RX_RING, index, 0)
    }
}

/// `opts2` du descripteur `index` : etiquette VLAN, et rien d'autre chez nous.
pub fn desc_opts2(index: usize) -> u32 {
    unsafe {
        if !READY || index >= N_RX {
            return 0;
        }
        desc_read32(RX_RING, index, 4)
    }
}

/// L'adresse DMA du tampon que porte le descripteur `index`.
pub fn desc_addr(index: usize) -> u64 {
    unsafe {
        if !READY || index >= N_RX {
            return 0;
        }
        desc_read64(RX_RING, index, 8)
    }
}

/// La carte des bits `OWN`, un bit par descripteur. Bit a un : au materiel.
pub fn carte_own_publique() -> u64 {
    unsafe {
        if !READY {
            return 0;
        }
        carte_own()
    }
}

/// L'adresse physique de l'anneau, telle que NOUS l'avons ecrite.
pub fn anneau_dma() -> u64 {
    unsafe {
        if !READY {
            return 0;
        }
        RX_RING_P
    }
}

/// L'adresse de l'anneau RELUE DANS LA PUCE.
///
/// Ce n'est pas `anneau_dma()`, et c'est tout l'interet : une reprise, une
/// reinitialisation partielle ou une economie d'energie qui aurait reecrit ce
/// registre ne se verrait nulle part ailleurs.
pub fn desc_addr_relu() -> u64 {
    unsafe {
        if !READY {
            return 0;
        }
        (read32(REG_RX_DESC_LOW) as u64) | ((read32(REG_RX_DESC_HIGH) as u64) << 32)
    }
}
