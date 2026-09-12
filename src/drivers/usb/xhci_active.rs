//! xHCI + USB HID bring-up for the physical TRIGKEY reference machine.
//!
//! V3.3 takes ownership of every reachable xHCI controller, powers/resets root
//! ports, builds command/event rings, enumerates directly-attached USB devices,
//! configures HID boot keyboard/mouse interrupt endpoints, and polls them from
//! the GUI event loop. Interrupt/MSI-x support can replace polling later without
//! changing the input-facing contract.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;
use core::ptr::{copy_nonoverlapping, read_volatile, write_bytes, write_volatile};
use core::sync::atomic::{fence, AtomicBool, AtomicU64, AtomicUsize, Ordering};

use crate::arch::x86_64::pci::{self, PciDevice};
use crate::drivers::bloc::{Achevement, Descripteur, Genre, PiloteBloc, Requete, Volume};
use crate::kernel::memory;

const USBCMD_RUN: u32 = 1 << 0;
const USBCMD_HCRST: u32 = 1 << 1;
const USBSTS_HCH: u32 = 1 << 0;
const USBSTS_CNR: u32 = 1 << 11;

const HCC_CSZ: u32 = 1 << 2;
const HCC_PPC: u32 = 1 << 3;

const PORTSC_CCS: u32 = 1 << 0;
const PORTSC_PED: u32 = 1 << 1;
const PORTSC_PR: u32 = 1 << 4;
const PORTSC_PP: u32 = 1 << 9;
const PORTSC_SPEED_SHIFT: u32 = 10;
const PORTSC_WPR: u32 = 1 << 31;
const PORTSC_CHANGE_BITS: u32 =
    (1 << 17) | (1 << 18) | (1 << 19) | (1 << 20) | (1 << 21) | (1 << 22) | (1 << 23);

const TRB_SIZE: usize = 16;
const RING_BYTES: usize = 4096;
const TRBS_PER_RING: usize = RING_BYTES / TRB_SIZE;
const TRB_CYCLE: u32 = 1;
const TRB_CHAIN: u32 = 1 << 4;
const TRB_IOC: u32 = 1 << 5;
const TRB_ISP: u32 = 1 << 2;
const TRB_IDT: u32 = 1 << 6;
const TRB_DIR_IN: u32 = 1 << 16;
const LINK_TRB_TYPE: u32 = 6;
const LINK_TC: u32 = 1 << 1;

const TRB_NORMAL: u32 = 1;
const TRB_SETUP_STAGE: u32 = 2;
const TRB_DATA_STAGE: u32 = 3;
const TRB_STATUS_STAGE: u32 = 4;
const CMD_ENABLE_SLOT: u32 = 9;
const CMD_DISABLE_SLOT: u32 = 10;
const CMD_ADDRESS_DEVICE: u32 = 11;
const CMD_CONFIGURE_ENDPOINT: u32 = 12;
const CMD_EVALUATE_CONTEXT: u32 = 13;
const CMD_RESET_ENDPOINT: u32 = 14;
const CMD_SET_TR_DEQUEUE: u32 = 16;
const EVT_TRANSFER: u32 = 32;
const EVT_COMMAND_COMPLETION: u32 = 33;
const EVT_PORT_STATUS_CHANGE: u32 = 34;

const CC_SUCCESS: u8 = 1;
const CC_SHORT_PACKET: u8 = 13;
/// Le peripherique a refuse la commande et a bloque le point de terminaison.
/// Ce n'est PAS une panne : le transport BOT s'en sert pour dire « commande
/// impossible », et un pilote qui traite un STALL comme une panne perd une cle
/// parfaitement saine des la premiere commande facultative.
const CC_STALL: u8 = 6;

const MAX_PORTS_PER_CONTROLLER: usize = 32;
const MAX_HID_ENDPOINTS_PER_CONTROLLER: usize = 16;
/// Supports de masse suivis par controleur.
const MAX_STOCKAGE_PAR_CONTROLEUR: usize = 2;
/// Tampon de donnees d'un support, en octets.
///
/// Soixante-quatre kibioctets, soit cent-vingt-huit blocs de 512 : assez pour
/// que le cout d'un aller-retour BOT -- trois transferts et deux attentes
/// d'evenement -- soit amorti, et assez petit pour que l'arene DMA en porte
/// deux sans se fragmenter.
const TAMPON_STOCKAGE: usize = 64 * 1024;
/// Essais de `TEST UNIT READY` avant d'abandonner un support.
///
/// Une cle qui vient d'etre branchee repond « pas prete » plusieurs fois de
/// suite pendant qu'elle monte en puissance. Abandonner au premier refus rend
/// le montage dependant du hasard.
const ESSAIS_UNITE_PRETE: usize = 20;
/// Temps maximal accorde a la reinitialisation d'un port de concentrateur.
///
/// Bornee, et c'est le point : un port qui ne sort jamais de reinitialisation
/// -- un peripherique defectueux, un cable a moitie enfonce -- ne doit pas
/// suspendre le demarrage de la machine.
const REINITIALISATION_MAX_MS: u32 = 800;
const MAX_CONFIG_DESCRIPTOR: usize = 4096;
const MAX_EVENTS_PER_POLL: usize = 64;

/// Combien de rapports HID peuvent etre mis de cote pendant un transfert de
/// controle.
///
/// Seize suffisent largement : un transfert de controle dure quelques
/// millisecondes, et un clavier n'emet qu'a chaque frappe. La borne existe
/// pour que le tampon ne puisse pas grandir, pas parce qu'on s'attend a le
/// remplir -- et un debordement se COMPTE, il ne se tait pas.
const EVENEMENTS_DIFFERES: usize = 16;

/// Combien de peripheriques branches a chaud on enumere par tour de scrutation.
///
/// Un seul. Enumerer prend une trentaine de millisecondes -- adressage, lecture
/// des descripteurs, configuration --, et le bureau appelle `poll()` depuis sa
/// boucle de dessin. En faire plusieurs d'affilee ferait sauter l'image.
const BRANCHEMENTS_PAR_TOUR: usize = 1;
const MAX_RUNTIME_DEVICES: usize = 32;
// UNE ATTENTE SE BORNE EN TEMPS, PAS EN NOMBRE DE TOURS.
//
// # Ce que trente millions de tours coutaient sur la machine de reference
//
// `WAIT_SPINS` valait 30 000 000. Un tour est une instruction `pause`, qui
// coute une trentaine de cycles sur un Zen 3 : l'attente complete valait donc
// environ un milliard de cycles, soit **330 ms** sur le Ryzen 7 5800H du
// TRIGKEY a 3,194 GHz.
//
// Ce n'etait pas theorique. `poll_control_fallback` emet un `GET_REPORT` sur
// EP0, un tour sur deux, pour CHAQUE point de terminaison muet en
// Interrupt-IN. Le recepteur Logitech du TRIGKEY expose une interface
// vendeur (`if=2 subclass=0 protocol=0`) qui n'a aucune raison d'y repondre :
// une attente non aboutie par tour de scrutation.
//
// L'enregistreur de vol mesure le resultat sans interpretation :
//
//     hid polls=3.19/s     une scrutation toutes les 313 ms
//     330 ms attendus, 313 ms mesures
//
// Tout ce qui depend de l'entree -- le pointeur, les frappes, et le
// compositeur qui n'est reveille que par elles -- avancait donc a 3 Hz. C'est
// ce que « inutilisable » voulait dire.
//
// # Pourquoi le temps et non les tours
//
// Un compte de tours ne dit rien : la meme constante vaut 30 ms sur une
// machine et 330 ms sur une autre, selon le cout d'un `pause`. Une borne en
// nanosecondes vaut ce qu'elle annonce, partout.
//
// Deux budgets, parce que deux usages :
//
//   * l'enumeration et les commandes du controleur sont rares et doivent
//     tolerer un peripherique lent ;
//   * la scrutation tourne mille fois par seconde et ne doit RIEN tolerer :
//     un peripherique HID repond a `GET_REPORT` en quelques microsecondes, et
//     celui qui ne repond pas en quatre millisecondes ne repondra pas.
const BUDGET_ATTENTE_NS: u64 = 500_000_000;
/// Budget d'une attente sur le chemin de scrutation. Voir ci-dessus.
const BUDGET_SCRUTATION_NS: u64 = 4_000_000;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static CONNECTED: AtomicUsize = AtomicUsize::new(0);
static ENABLED: AtomicUsize = AtomicUsize::new(0);
static ADDRESS_OK: AtomicUsize = AtomicUsize::new(0);
static CONTROL_OK: AtomicUsize = AtomicUsize::new(0);
static USB_DEVICES: AtomicUsize = AtomicUsize::new(0);
static HID_KEYBOARDS: AtomicUsize = AtomicUsize::new(0);
static HID_MICE: AtomicUsize = AtomicUsize::new(0);
// Le decodage pur -- tables de touches, differences de rapports,
// disposition de la souris -- vit a part et est mis a l'epreuve sur l'hote.
#[path = "hid/decodage.rs"]
mod hid;

// La traversee d'un concentrateur est de l'arithmetique de champs de bits :
// chaine de route, contexte de slot, descripteur, etat d'un port. Une faute
// n'y produit aucun message -- le controleur adresse un peripherique qui n'est
// pas la. Elle vit donc a part, et une machine la relit.
#[path = "concentrateur/decodage.rs"]
mod concentrateur;

// Le protocole du stockage de masse -- enveloppes CBW/CSW, blocs de commande
// SCSI, recherche de l'interface BOT dans le descripteur de configuration --
// est de l'octet pur. Il vit a part et une machine le relit : une cle qui
// repond de travers ne se fabrique pas sur commande.
//
// Il est partage par les DEUX consommateurs du stockage de masse : le
// enregistreur de vol (`blackbox_storage.rs`) et le pilote de volume
// (`PiloteUsbStockage`). Le declarer deux fois compilerait le meme fichier
// deux fois dans le meme module, avec deux jeux de constantes qui pourraient
// diverger sans que rien ne le dise.
#[path = "stockage/decodage.rs"]
mod stockage;

include!("blackbox_storage.rs"); // BOUCHAUD_TRIGKEY_BLACKBOX_V2

use concentrateur::CLASSE_CONCENTRATEUR;

// BOUCHAUD_XHCI_HID_TRANSPORT_V33
static HID_POLLS: AtomicUsize = AtomicUsize::new(0);
static HID_TRANSFER_EVENTS: AtomicUsize = AtomicUsize::new(0);
static HID_REPORTS: AtomicUsize = AtomicUsize::new(0);
static HID_KEYBOARD_REPORTS: AtomicUsize = AtomicUsize::new(0);
static HID_MOUSE_REPORTS: AtomicUsize = AtomicUsize::new(0);
static HID_TRANSFER_ERRORS: AtomicUsize = AtomicUsize::new(0);
static HID_REARMS: AtomicUsize = AtomicUsize::new(0);
static HID_KICKS: AtomicUsize = AtomicUsize::new(0);
/// Concentrateurs trouves, traverses ou non.
static CONCENTRATEURS: AtomicUsize = AtomicUsize::new(0);
/// Concentrateurs qu'on n'a PAS pu traverser -- et c'est le compteur qui
/// compte : un clavier branche derriere l'un d'eux ne repondra pas, et sans ce
/// chiffre on chercherait le defaut dans le code du clavier, qui marche.
static CONCENTRATEURS_ECHOUES: AtomicUsize = AtomicUsize::new(0);
/// Peripheriques atteints DERRIERE un concentrateur.
static PERIPHERIQUES_DERRIERE: AtomicUsize = AtomicUsize::new(0);
/// Evenements qu'on n'a pas pu mettre de cote -- donc des frappes perdues.
static EVENEMENTS_PERDUS: AtomicUsize = AtomicUsize::new(0);
/// Peripheriques branches apres le demarrage.
static BRANCHEMENTS: AtomicUsize = AtomicUsize::new(0);
/// Peripheriques debranches apres le demarrage.
static DEBRANCHEMENTS: AtomicUsize = AtomicUsize::new(0);
static HID_CONTROL_POLLS: AtomicUsize = AtomicUsize::new(0);
static HID_CONTROL_REPORTS: AtomicUsize = AtomicUsize::new(0);
static HID_CONTROL_FAILS: AtomicUsize = AtomicUsize::new(0);
/// Code d'achevement du DERNIER transfert de controle echoue.
///
/// Sans lui, le repli ne peut pas distinguer « ce transfert n'est pas passe »
/// de « ce peripherique ne repondra jamais » -- et il appliquait la meme peine
/// aux deux.
static DERNIER_CODE_CONTROLE: AtomicUsize = AtomicUsize::new(0);
static CONTROLLERS: AtomicUsize = AtomicUsize::new(0);
static RUNTIME_BUSY: AtomicBool = AtomicBool::new(false);
static mut RUNTIME: Option<Runtime> = None;

#[derive(Clone, Debug)]
pub struct ActiveSummary {
    pub present: bool,
    pub active: bool,
    pub controllers_seen: usize,
    pub controllers_active: usize,
    pub bar0: u64,
    pub hci_version: u16,
    pub max_slots: u8,
    pub max_ports: u8,
    pub scratchpads: usize,
    pub connected_ports: usize,
    pub enabled_ports: usize,
    pub usb_devices: usize,
    pub hid_keyboards: usize,
    pub hid_mice: usize,
    pub error: Option<&'static str>,
}

impl ActiveSummary {
    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "xHCI USB HID PHYSICAL V3");
        let _ = writeln!(out, "present={}", self.present);
        let _ = writeln!(out, "active={}", self.active);
        let _ = writeln!(out, "controllers_seen={}", self.controllers_seen);
        let _ = writeln!(out, "controllers_active={}", self.controllers_active);
        let _ = writeln!(out, "bar0_first={:#x}", self.bar0);
        let _ = writeln!(out, "hci_version_first={:#06x}", self.hci_version);
        let _ = writeln!(out, "max_slots_first={}", self.max_slots);
        let _ = writeln!(out, "max_ports_first={}", self.max_ports);
        let _ = writeln!(out, "scratchpads_total={}", self.scratchpads);
        let _ = writeln!(out, "connected_ports={}", self.connected_ports);
        let _ = writeln!(out, "enabled_ports={}", self.enabled_ports);
        let _ = writeln!(out, "usb_devices={}", self.usb_devices);
        let _ = writeln!(out, "hid_keyboards={}", self.hid_keyboards);
        let _ = writeln!(out, "hid_mice={}", self.hid_mice);
        let _ = writeln!(out, "polling={}", self.hid_keyboards + self.hid_mice != 0);
        let _ = writeln!(out, "error={}", self.error.unwrap_or("none"));
        let _ = writeln!(out, "next=MSI-X/interrupt driven xHCI + hub/hotplug expansion");
        out
    }
}

#[derive(Clone, Copy, Default)]
struct Trb {
    parameter: u64,
    status: u32,
    control: u32,
}

#[derive(Clone, Copy)]
struct ProducerRing {
    phys: u64,
    virt: usize,
    index: usize,
    cycle: u32,
}

#[derive(Clone, Copy)]
struct EventRing {
    phys: u64,
    virt: usize,
    index: usize,
    cycle: u32,
}

#[derive(Clone, Copy)]
struct HidDescriptor {
    interface: u8,
    subclass: u8,
    protocol: u8,
    kind: u8, // 1 keyboard, 2 mouse, 0 unknown HID
    /// Le descripteur de RAPPORT de cette interface a-t-il ete lu et classe ?
    ///
    /// Il fait autorite : il enumere les usages que l'interface publie. Quand
    /// il a parle, le repli aveugle de `install_hid_endpoints` n'a plus rien
    /// a deviner -- et surtout, il ne doit PAS contredire ce qu'il dit.
    classe_par_rapport: bool,
    report_len: u16,
    report_id: u8,
    endpoint_address: u8,
    max_packet: u16,
    interval: u8,
}

const EMPTY_HID_DESCRIPTOR: HidDescriptor = HidDescriptor {
    interface: 0,
    subclass: 0,
    protocol: 0,
    kind: 0,
    classe_par_rapport: false,
    report_len: 0,
    report_id: 0,
    endpoint_address: 0,
    max_packet: 0,
    interval: 0,
};

#[derive(Clone, Copy)]
struct HidEndpoint {
    active: bool,
    slot_id: u8,
    dci: u8,
    interface: u8,
    protocol: u8,
    kind: u8,
    /// Source de boutons reservee a CE point de terminaison, ou `usize::MAX`.
    ///
    /// L'etat global des boutons est l'union des sources : sans identite
    /// propre, un rapport « rien d'enfonce » d'une interface au repos effacait
    /// le bouton maintenu sur une autre. Voir `mouse/etat.rs`.
    source_souris: usize,
    report_id: u8,
    max_packet: u16,
    ring: ProducerRing,
    buffer_phys: u64,
    buffer_virt: usize,
    buffer_len: usize,
    /// L'etat du clavier entre deux rapports, tenu par le decodeur pur.
    clavier: hid::EtatClavier,
    /// Evenements de transfert recus par CE point de terminaison.
    ///
    /// # Pourquoi il ne peut pas etre global
    ///
    /// Le compteur l'etait, et le repli EP0 s'eteignait des qu'UN periphérique
    /// produisait un evenement. Sur la machine de reference, la souris en a
    /// produit 556 et le clavier 6 : la souris coupait le repli, et le
    /// clavier -- muet en Interrupt-IN -- se retrouvait sans aucun transport.
    ///
    /// Un compteur par point de terminaison fait exactement l'inverse de
    /// masquer le probleme : il NOMME le peripherique qui ne repond pas, au
    /// lieu de laisser croire que tout va bien parce qu'un autre repond.
    evenements: u32,
    /// Instant ou ce point a ete arme pour la premiere fois.
    ///
    /// Le repli EP0 lui laisse un DELAI DE GRACE : un point qui vient d'etre
    /// arme n'a pas encore eu l'occasion de produire un evenement, et le
    /// doubler d'un `GET_REPORT` synchrone le condamnerait au pont avant
    /// meme de l'avoir essaye. L'ancien code exprimait la meme idee par un
    /// compteur global de tours (`poll_no < 32`), qui ne disait rien d'un
    /// peripherique branche a chaud une minute apres le demarrage.
    arme_depuis_ns: u64,
    /// Echecs consecutifs du repli EP0 sur CE point de terminaison.
    echecs_repli: u8,
    /// L'entree en quarantaine de ce point a-t-elle deja ete annoncee ?
    ///
    /// La quarantaine se REPOSE a chaque reprise ratee ; l'annoncer a chaque
    /// fois reviendrait a inonder le journal pour dire ce qui n'a pas change.
    quarantaine_annoncee: bool,
    /// Instant avant lequel le repli ne sera pas retente sur ce point.
    ///
    /// # Pourquoi une quarantaine, et pas un simple compteur
    ///
    /// Un point de terminaison qui ne repond pas a `GET_REPORT` ne va pas se
    /// mettre a repondre au tour suivant. Le redemander mille fois par seconde
    /// ne le reveille pas : cela paie son echeance mille fois par seconde, et
    /// c'est tout le systeme qui ralentit -- le recepteur Logitech du TRIGKEY
    /// expose une interface vendeur qui n'a jamais eu de raison de repondre.
    ///
    /// Le repli reste un pont de compatibilite : il est RETENTE, mais une fois
    /// par seconde, pas cinq cents. Un peripherique qui se met a repondre est
    /// donc repris, et celui qui reste muet ne coute plus rien.
    repli_muet_jusqu_a_ns: u64,
}

const EMPTY_RING: ProducerRing = ProducerRing {
    phys: 0,
    virt: 0,
    index: 0,
    cycle: 1,
};

const EMPTY_HID_ENDPOINT: HidEndpoint = HidEndpoint {
    arme_depuis_ns: 0,
    echecs_repli: 0,
    quarantaine_annoncee: false,
    repli_muet_jusqu_a_ns: 0,
    evenements: 0,
    active: false,
    slot_id: 0,
    dci: 0,
    interface: 0,
    protocol: 0,
    kind: 0,
    source_souris: usize::MAX,
    report_id: 0,
    max_packet: 0,
    ring: EMPTY_RING,
    buffer_phys: 0,
    buffer_virt: 0,
    buffer_len: 0,
    clavier: hid::EtatClavier { modificateurs: 0, touches: [0; 6] },
};

/// Un point de terminaison BULK et son anneau de transfert.
#[derive(Clone, Copy)]
struct PointBulk {
    dci: u8,
    /// Adresse USB du point, bit 7 compris. Le `CLEAR_FEATURE(ENDPOINT_HALT)`
    /// la demande, et elle ne se retrouve pas depuis le DCI : la division par
    /// deux perd le sens.
    adresse: u8,
    ring: ProducerRing,
}

const POINT_BULK_VIDE: PointBulk = PointBulk { dci: 0, adresse: 0, ring: EMPTY_RING };

/// Un support de masse USB, tel que le transport Bulk-Only le voit.
#[derive(Clone, Copy)]
struct Stockage {
    actif: bool,
    slot_id: u8,
    interface: u8,
    entree: PointBulk,
    sortie: PointBulk,
    /// Tampon de donnees, partage par les lectures et les ecritures.
    donnees_phys: u64,
    donnees_virt: usize,
    /// Tampon des enveloppes CBW et CSW. Separe du tampon de donnees : les
    /// trois transferts d'une commande BOT sont distincts, et faire porter le
    /// CSW par la fin du tampon de donnees ecraserait le dernier bloc lu.
    enveloppe_phys: u64,
    enveloppe_virt: usize,
    /// Etiquette du prochain CBW. Elle IDENTIFIE la reponse : un CSW dont
    /// l'etiquette ne correspond pas repond a une commande precedente.
    etiquette: u32,
    taille_bloc: u32,
    blocs: u64,
    amovible: bool,
}

const STOCKAGE_VIDE: Stockage = Stockage {
    actif: false,
    slot_id: 0,
    interface: 0,
    entree: POINT_BULK_VIDE,
    sortie: POINT_BULK_VIDE,
    donnees_phys: 0,
    donnees_virt: 0,
    enveloppe_phys: 0,
    enveloppe_virt: 0,
    etiquette: 1,
    taille_bloc: 0,
    blocs: 0,
    amovible: false,
};

struct Controller {
    dev: PciDevice,
    base: usize,
    op: usize,
    doorbells: usize,
    intr0: usize,
    hci_version: u16,
    max_slots: u8,
    max_ports: u8,
    context_size: usize,
    ppc: bool,
    slot_types: [u8; MAX_PORTS_PER_CONTROLLER],
    port_major: [u8; MAX_PORTS_PER_CONTROLLER],
    command: ProducerRing,
    events: EventRing,
    dcbaa_phys: u64,
    dcbaa_virt: usize,
    blackbox_storage: Option<BlackboxStorage>,
    hids: [HidEndpoint; MAX_HID_ENDPOINTS_PER_CONTROLLER],
    stockages: [Stockage; MAX_STOCKAGE_PAR_CONTROLEUR],
    stockage_count: usize,
    // UN RAPPORT DE CLAVIER N'EST PAS UN EVENEMENT QU'ON JETTE.
    //
    // L'anneau d'evenements est unique : les achevements de commande, les
    // achevements de transfert de controle et les rapports HID y arrivent
    // melanges. `wait_event` attend UN evenement precis -- et jetait tous les
    // autres. Un rapport de clavier qui arrive pendant un transfert de
    // controle disparaissait donc, et la frappe avec lui.
    //
    // Personne ne le voyait : au demarrage les points de terminaison HID ne
    // sont armes qu'apres l'enumeration, precisement pour eviter ce
    // croisement. Mais le branchement a chaud rend le croisement NORMAL --
    // enumerer un peripherique pendant qu'un autre envoie des rapports --, et
    // la solution « armer plus tard » n'y marche plus.
    //
    // On les met donc de cote au lieu de les perdre.
    differes: [Trb; EVENEMENTS_DIFFERES],
    differes_len: usize,
    /// Ports dont l'etat a change et qu'il reste a traiter, un bit par port.
    ports_a_traiter: u32,
    arbre: [Option<NoeudUsb>; NOEUDS_MAX],
    compte_noeuds: usize,
    hid_count: usize,
    devices: [Option<Device>; MAX_RUNTIME_DEVICES],
    connected_ports: usize,
    enabled_ports: usize,
    usb_devices: usize,
}

struct Runtime {
    controllers: Vec<Controller>,
}

/// OU un peripherique est branche dans l'arbre USB.
///
/// # Pourquoi une structure, et non deux entiers
///
/// Adresser un peripherique derriere un concentrateur demande quatre choses
/// qui vont ensemble et qu'on ne peut pas retrouver l'une sans l'autre : le
/// port du concentrateur RACINE (jamais celui du concentrateur intermediaire),
/// la chaine de route qui dit le chemin, la profondeur qui dit ou ecrire le
/// prochain etage, et le transactionneur du plus proche concentrateur haute
/// vitesse. Les passer separement, c'est se tromper d'un tot ou tard.
#[derive(Clone, Copy)]
struct Chemin {
    /// Port du concentrateur racine, numerote a partir de un.
    port_racine: u8,
    /// Chaine de route xHCI. Zero pour un peripherique branche a la racine.
    route: u32,
    /// Zero a la racine, un derriere un concentrateur, et ainsi de suite.
    profondeur: usize,
    /// Identifiant de vitesse xHCI du peripherique lui-meme.
    vitesse: u8,
    /// Slot du concentrateur haute vitesse qui traduit pour ce peripherique,
    /// ou zero s'il n'en a pas besoin.
    tt_slot: u8,
    /// Port de ce concentrateur ou le peripherique est branche.
    tt_port: u8,
}

impl Chemin {
    const fn racine(port: u8, vitesse: u8) -> Self {
        Self { port_racine: port, route: 0, profondeur: 0, vitesse, tt_slot: 0, tt_port: 0 }
    }
}

/// Combien de peripheriques peuvent attendre leur tour d'etre enumeres.
const CHEMINS_EN_ATTENTE: usize = 32;

/// La traversee est en LARGEUR, et non en profondeur.
///
/// Une descente recursive empilerait, a chaque etage, le tampon de
/// descripteurs et la table des points de terminaison de cet etage. Cinq
/// etages de cela sur une pile de noyau, c'est un debordement qu'on ne
/// diagnostique pas -- la machine redemarre, sans rien dire.
///
/// Une file bornee retire cette facon d'echouer : la pile reste plate, et un
/// arbre plus grand que la file se DIT au lieu de deborder.
struct FileChemins {
    entrees: [Chemin; CHEMINS_EN_ATTENTE],
    tete: usize,
    queue: usize,
    debordements: usize,
}

impl FileChemins {
    fn neuve() -> Self {
        Self {
            entrees: [Chemin::racine(0, 0); CHEMINS_EN_ATTENTE],
            tete: 0,
            queue: 0,
            debordements: 0,
        }
    }

    fn pousse(&mut self, chemin: Chemin) {
        if self.queue >= CHEMINS_EN_ATTENTE {
            self.debordements = self.debordements.saturating_add(1);
            return;
        }
        self.entrees[self.queue] = chemin;
        self.queue += 1;
    }

    fn tire(&mut self) -> Option<Chemin> {
        if self.tete >= self.queue {
            return None;
        }
        let chemin = self.entrees[self.tete];
        self.tete += 1;
        Some(chemin)
    }
}

/// Ce qu'on garde d'un peripherique une fois enumere, pour pouvoir le DIRE.
///
/// Sans cette table, la seule trace de l'arbre USB est une suite de lignes de
/// journal emises au demarrage. Sur une machine sans console serie, elles sont
/// perdues. `lsusb` les rejoue depuis l'etat, a n'importe quel moment.
#[derive(Clone, Copy)]
struct NoeudUsb {
    port_racine: u8,
    route: u32,
    profondeur: u8,
    vitesse: u8,
    slot: u8,
    vendeur: u16,
    produit: u16,
    classe: u8,
    /// 0 aucun, 1 clavier, 2 souris.
    genre_hid: u8,
}

const NOEUDS_MAX: usize = 32;

#[derive(Clone, Copy)]
struct Device {
    slot_id: u8,
    root_port: u8,
    speed: u8,
    ep0_mps: u16,
    out_ctx_phys: u64,
    out_ctx_virt: usize,
    in_ctx_phys: u64,
    in_ctx_virt: usize,
    ep0: ProducerRing,
    control_phys: u64,
    control_virt: usize,
}

#[inline]
unsafe fn r8(base: usize, off: usize) -> u8 {
    read_volatile((base + off) as *const u8)
}
#[inline]
unsafe fn r16(base: usize, off: usize) -> u16 {
    read_volatile((base + off) as *const u16)
}
#[inline]
unsafe fn r32(base: usize, off: usize) -> u32 {
    read_volatile((base + off) as *const u32)
}
#[inline]
unsafe fn w32(base: usize, off: usize, value: u32) {
    write_volatile((base + off) as *mut u32, value);
}
#[inline]
unsafe fn w64(base: usize, off: usize, value: u64) {
    write_volatile((base + off) as *mut u64, value);
}

fn wait_until(mut predicate: impl FnMut() -> bool) -> bool {
    let debut = crate::kernel::timer::monotonic_ns();
    let mut tours = 0u32;
    loop {
        if predicate() {
            return true;
        }
        tours = tours.wrapping_add(1);
        // L'horloge se relit tous les 256 tours. La lire a chaque tour
        // couterait plus cher que l'attente qu'elle borne.
        if tours & 0xff == 0
            && crate::kernel::timer::monotonic_ns().saturating_sub(debut)
                >= BUDGET_ATTENTE_NS
        {
            return false;
        }
        core::hint::spin_loop();
    }
}

fn wait_ms(ms: u64) {
    let start = crate::kernel::timer::monotonic_ns();
    let deadline = start.saturating_add(ms.saturating_mul(1_000_000));
    if start != 0 {
        while crate::kernel::timer::monotonic_ns() < deadline {
            core::hint::spin_loop();
        }
    } else {
        for _ in 0..ms.saturating_mul(100_000) {
            core::hint::spin_loop();
        }
    }
}

fn max_scratchpads(hcs2: u32) -> usize {
    // xHCI HCSPARAMS2: Max Scratchpad Buffers Hi = 31:27, Lo = 25:21.
    let hi = ((hcs2 >> 27) & 0x1f) as usize;
    let lo = ((hcs2 >> 21) & 0x1f) as usize;
    (hi << 5) | lo
}

/// Decalage de `USBLEGCTLSTS` depuis la capacite de support hérite.
const LEGACY_CONTROLE: usize = 0x04;

/// Bits RESERVES de `USBLEGCTLSTS`, ceux qu'il faut PRESERVER.
///
/// Tout le reste est soit une autorisation de SMI -- qu'on veut a zero --,
/// soit un evenement RW1C -- qu'on efface en y ecrivant un.
const LEGACY_BITS_RESERVES: u32 = (0x7 << 1) | (0xff << 5) | (0x7 << 17);

/// Evenements SMI, effaces en y ecrivant un.
const LEGACY_EVENEMENTS_SMI: u32 = 0x7 << 29;

/// Prend le controleur au micrologiciel, et desarme ses interruptions systeme.
///
/// # Les deux moities de cette fonction
///
/// **Prendre la main.** Le micrologiciel possede le controleur pendant tout
/// l'amorcage : c'est ainsi qu'un clavier USB marche dans le menu du BIOS. La
/// remise se fait par un semaphore -- on pose « OS possede », on attend que
/// « BIOS possede » tombe.
///
/// **Desarmer les SMI, et c'est la moitie qui manquait.** Prendre le
/// controleur ne suffit pas : tant que les autorisations de SMI de
/// `USBLEGCTLSTS` restent posees, le micrologiciel continue d'etre APPELE sur
/// chaque evenement USB, par une interruption de gestion systeme que le noyau
/// ne voit pas et ne peut pas masquer.
///
/// Le symptome est celui-la meme qu'on cherche : un clavier qui marche dans le
/// BIOS et pas dans le systeme. Le micrologiciel intercepte l'evenement, le
/// traite pour son emulation PS/2, et le noyau ne recoit rien -- ou recoit un
/// anneau d'evenements dans lequel quelqu'un d'autre a avance la tete.
///
/// Sur une machine AMD dont le BIOS propose « USB legacy emulation », c'est le
/// cas NORMAL, pas le cas rare.
///
/// # Un micrologiciel qui ne rend jamais la main
///
/// Certains ne baissent jamais leur bit. Attendre indefiniment donnerait une
/// machine sans clavier et sans explication ; on force alors la reprise en
/// effacant le bit nous-memes. Ce n'est pas elegant, et c'est ce que fait
/// n'importe quel systeme qui demarre sur du materiel reel.
unsafe fn legacy_handoff(base: usize, hccparams1: u32) -> bool {
    let mut off = (((hccparams1 >> 16) & 0xffff) as usize) * 4;
    for _ in 0..64 {
        if off == 0 {
            return true;
        }
        let header = r32(base, off);
        let cap_id = (header & 0xff) as u8;
        let next = ((header >> 8) & 0xff) as usize;
        if cap_id == 1 {
            // OS Owned Semaphore. Wait until BIOS Owned is released.
            w32(base, off, header | (1 << 24));
            let rendu = header & (1 << 16) == 0
                || wait_until(|| r32(base, off) & (1 << 16) == 0);
            if !rendu {
                // Reprise forcee. Un micrologiciel qui ne rend pas la main
                // laisserait sinon la machine sans clavier, et sans rien pour
                // le dire.
                let courant = r32(base, off);
                w32(base, off, courant & !(1 << 16));
                crate::serial_println!(
                    "BOUCHAUD_XHCI_HANDOFF_FORCE etat={:#010x} raison=micrologiciel-ne-rend-pas-la-main",
                    courant
                );
            }

            // LES SMI, MAINTENANT.
            //
            // On garde les bits reserves, on met a zero toutes les
            // autorisations, et on ecrit un dans les evenements pour les
            // effacer. Les faire dans cet ordre compte : effacer un evenement
            // dont l'autorisation est encore posee le fait revenir.
            let controle = r32(base, off + LEGACY_CONTROLE);
            let desarme = (controle & LEGACY_BITS_RESERVES) | LEGACY_EVENEMENTS_SMI;
            w32(base, off + LEGACY_CONTROLE, desarme);
            crate::serial_println!(
                "BOUCHAUD_XHCI_SMI_DESARMES avant={:#010x} apres={:#010x} rendu={}",
                controle,
                r32(base, off + LEGACY_CONTROLE),
                rendu as u8
            );
            return true;
        }
        if next == 0 {
            return true;
        }
        off = off.saturating_add(next * 4);
    }
    false
}

fn alloc_zeroed(bytes: usize) -> Option<(u64, usize)> {
    let (phys, virt) = memory::alloc_dma(bytes)?;
    unsafe { write_bytes(virt, 0, bytes) };
    Some((phys, virt as usize))
}

/// Rend a l'arene tout ce qu'un peripherique occupait.
///
/// # Ce que le branchement a chaud a change
///
/// Tant que l'enumeration n'avait lieu qu'au demarrage, ne rien rendre ne
/// coutait rien : ce qui etait pris l'etait une fois pour toutes. Brancher et
/// debrancher fait de la meme omission une FUITE -- quatre pages plus deux
/// anneaux par branchement -- et une arene DMA epuisee ne se manifeste pas par
/// un message clair : les enumerations suivantes echouent avec « dma-* » et le
/// peripherique qu'on vient de brancher ne repond pas.
fn abandonne_device(controller: &mut Controller, device: &Device) {
    disable_slot(controller, device.slot_id);
    libere_device(device);
}

fn libere_device(device: &Device) {
    memory::free_dma(device.out_ctx_phys, 4096);
    memory::free_dma(device.in_ctx_phys, 4096);
    memory::free_dma(device.control_phys, 4096);
    memory::free_dma(device.ep0.phys, RING_BYTES);
}

fn trb_type(control: u32) -> u32 {
    (control >> 10) & 0x3f
}

fn completion_code(status: u32) -> u8 {
    (status >> 24) as u8
}

fn event_slot(event: Trb) -> u8 {
    (event.control >> 24) as u8
}

fn event_dci(event: Trb) -> u8 {
    ((event.control >> 16) & 0x1f) as u8
}

unsafe fn write_trb(virt: usize, index: usize, trb: Trb, cycle: u32) {
    let ptr = virt + index * TRB_SIZE;
    write_volatile(ptr as *mut u64, trb.parameter);
    write_volatile((ptr + 8) as *mut u32, trb.status);
    fence(Ordering::Release);
    write_volatile((ptr + 12) as *mut u32, (trb.control & !TRB_CYCLE) | cycle);
}

unsafe fn prepare_link(ring: &ProducerRing, cycle: u32) {
    let index = TRBS_PER_RING - 1;
    let ptr = ring.virt + index * TRB_SIZE;
    write_volatile(ptr as *mut u64, ring.phys);
    write_volatile((ptr + 8) as *mut u32, 0);
    fence(Ordering::Release);
    write_volatile(
        (ptr + 12) as *mut u32,
        (LINK_TRB_TYPE << 10) | LINK_TC | cycle,
    );
}

fn ring_push(ring: &mut ProducerRing, trb: Trb) -> u64 {
    unsafe {
        let pointer = ring.phys + (ring.index * TRB_SIZE) as u64;
        write_trb(ring.virt, ring.index, trb, ring.cycle);
        ring.index += 1;
        if ring.index == TRBS_PER_RING - 1 {
            prepare_link(ring, ring.cycle);
            ring.index = 0;
            ring.cycle ^= 1;
        }
        pointer
    }
}

fn alloc_producer_ring() -> Option<ProducerRing> {
    let (phys, virt) = alloc_zeroed(RING_BYTES)?;
    let ring = ProducerRing {
        phys,
        virt,
        index: 0,
        cycle: 1,
    };
    unsafe { prepare_link(&ring, 1) };
    Some(ring)
}

fn next_event(controller: &mut Controller) -> Option<Trb> {
    unsafe {
        let ptr = controller.events.virt + controller.events.index * TRB_SIZE;
        let control = read_volatile((ptr + 12) as *const u32);
        if control & TRB_CYCLE != controller.events.cycle {
            return None;
        }
        fence(Ordering::Acquire);
        let event = Trb {
            parameter: read_volatile(ptr as *const u64),
            status: read_volatile((ptr + 8) as *const u32),
            control,
        };

        controller.events.index += 1;
        if controller.events.index == TRBS_PER_RING {
            controller.events.index = 0;
            controller.events.cycle ^= 1;
        }
        let erdp = controller.events.phys
            + (controller.events.index * TRB_SIZE) as u64
            | (1 << 3); // clear EHB while advancing ERDP
        w64(controller.intr0, 0x18, erdp);
        Some(event)
    }
}

/// Met un evenement de cote, ou compte sa perte.
fn differe(controller: &mut Controller, event: Trb) {
    if controller.differes_len >= EVENEMENTS_DIFFERES {
        EVENEMENTS_PERDUS.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let index = controller.differes_len;
    controller.differes[index] = event;
    controller.differes_len = index + 1;
}

/// Traite et vide la file des evenements mis de cote, dans l'ordre d'arrivee.
///
/// L'ORDRE EST LA RAISON D'ETRE DE LA FILE : deux frappes traitees a l'envers,
/// c'est une touche relachee avant d'etre appuyee, donc une touche qui reste
/// enfoncee. Elle est donc videe du plus ancien au plus recent, et jamais
/// partiellement.
fn traite_differes(controller: &mut Controller) {
    let differes = controller.differes_len;
    controller.differes_len = 0;
    for index in 0..differes {
        let event = controller.differes[index];
        process_hid_event(controller, event);
    }
}

fn wait_event(controller: &mut Controller, wanted_type: u32, slot: Option<u8>, dci: Option<u8>) -> Option<Trb> {
    wait_event_budget(controller, wanted_type, slot, dci, BUDGET_ATTENTE_NS)
}

/// Attend un evenement, au plus `budget_ns`.
///
/// Le budget est un ARGUMENT et non une constante : le chemin de scrutation
/// n'a pas la meme patience que l'enumeration, et confondre les deux est
/// exactement ce qui rendait la machine de reference inutilisable.
fn wait_event_budget(
    controller: &mut Controller,
    wanted_type: u32,
    slot: Option<u8>,
    dci: Option<u8>,
    budget_ns: u64,
) -> Option<Trb> {
    let debut = crate::kernel::timer::monotonic_ns();
    let mut tours = 0u32;
    loop {
        if let Some(event) = next_event(controller) {
            let ty = trb_type(event.control);
            if ty == EVT_PORT_STATUS_CHANGE {
                // Un changement de port pendant un transfert de controle : on
                // note le port et on continue. Enumerer ICI reentrerait dans
                // le transfert en cours.
                note_port_a_traiter(controller, port_de_l_evenement(event));
                continue;
            }
            if ty == wanted_type
                && slot.map_or(true, |s| event_slot(event) == s)
                && dci.map_or(true, |e| event_dci(event) == e)
            {
                return Some(event);
            }
            // CE QUI NE NOUS ETAIT PAS DESTINE N'EST PAS A JETER.
            //
            // Un rapport de clavier qui arrive pendant ce transfert est une
            // FRAPPE. Le jeter la perd, et rien ne le dit.
            if ty == EVT_TRANSFER {
                differe(controller, event);
                // BOUCHAUD_USB_ENTREE_PENDANT_STOCKAGE_V1
                //
                // ET IL N'EST PAS NON PLUS A FAIRE ATTENDRE.
                //
                // Mis de cote, ce rapport n'etait traite qu'au tour de
                // scrutation SUIVANT -- c'est-a-dire une fois le verrou du
                // pilote rendu. Or une commande de stockage tient ce verrou
                // pendant trois transferts et deux attentes, chacune bornee a
                // un demi-seconde : une cle qui repond mal gelait l'entree
                // pendant plus d'une seconde.
                //
                // C'est la forme exacte de ce que l'utilisateur a decrit en
                // ouvrant le navigateur : « FPS 2 » avec le processeur a 22 %,
                // et « la souris met trop de temps a se deplacer (multiple
                // freeze) ». La machine n'etait pas saturee, elle ATTENDAIT.
                //
                // Le traitement a lieu ici, dans l'ordre d'arrivee, parce
                // qu'il est sur : `process_hid_event` decode un rapport et
                // rearme son extremite. Il n'emet aucun transfert et n'attend
                // aucun evenement, donc il ne peut pas reentrer dans l'attente
                // qui l'appelle.
                traite_differes(controller);
            }
        }
        tours = tours.wrapping_add(1);
        if tours & 0xff == 0
            && crate::kernel::timer::monotonic_ns().saturating_sub(debut) >= budget_ns
        {
            return None;
        }
        core::hint::spin_loop();
    }
}

/// Numero de port porte par un evenement de changement d'etat de port.
///
/// xHCI 1.2 §6.4.2.3 : le champ est dans les bits 31:24 du PARAMETRE, et non
/// dans le mot de controle ou vivent le slot et le type.
fn port_de_l_evenement(event: Trb) -> u8 {
    ((event.parameter >> 24) & 0xff) as u8
}

fn note_port_a_traiter(controller: &mut Controller, port: u8) {
    if port == 0 || port as usize > MAX_PORTS_PER_CONTROLLER {
        return;
    }
    controller.ports_a_traiter |= 1u32 << (port - 1);
}

fn ring_doorbell(controller: &Controller, slot: u8, target: u8) {
    fence(Ordering::Release);
    unsafe { w32(controller.doorbells, slot as usize * 4, target as u32) };
}

fn command_raw(controller: &mut Controller, parameter: u64, control: u32) -> Result<Trb, &'static str> {
    let pointer = ring_push(
        &mut controller.command,
        Trb { parameter, status: 0, control },
    );
    ring_doorbell(controller, 0, 0);
    let event = wait_event(controller, EVT_COMMAND_COMPLETION, None, None).ok_or("command-timeout")?;
    if event.parameter & !0xf != pointer & !0xf {
        // Command completions are ordered. A mismatched pointer means the event
        // stream is no longer the one we submitted; fail closed.
        return Err("command-pointer-mismatch");
    }
    let cc = completion_code(event.status);
    if cc != CC_SUCCESS {
        crate::serial_println!(
            "BOUCHAUD_XHCI_COMMAND_FAIL type={} slot={} cc={} event_status={:#010x}",
            trb_type(control), (control >> 24) as u8, cc, event.status,
        );
        return Err("command-completion-error");
    }
    Ok(event)
}

fn command(controller: &mut Controller, parameter: u64, command_type: u32, slot: u8) -> Result<Trb, &'static str> {
    command_raw(
        controller,
        parameter,
        (command_type << 10) | ((slot as u32) << 24),
    )
}

fn enable_slot(controller: &mut Controller, root_port: u8) -> Result<u8, &'static str> {
    let slot_type = controller
        .slot_types
        .get(root_port.saturating_sub(1) as usize)
        .copied()
        .unwrap_or(0);
    let event = command_raw(
        controller,
        0,
        (CMD_ENABLE_SLOT << 10) | ((slot_type as u32) << 16),
    )?;
    let slot = event_slot(event);
    if slot == 0 {
        Err("enable-slot-zero")
    } else {
        Ok(slot)
    }
}

fn disable_slot(controller: &mut Controller, slot: u8) {
    let _ = command(controller, 0, CMD_DISABLE_SLOT, slot);
}

fn context_ptr(base: usize, context_size: usize, index: usize) -> usize {
    base + context_size * index
}

unsafe fn ctx_r32(ctx: usize, dword: usize) -> u32 {
    read_volatile((ctx + dword * 4) as *const u32)
}
unsafe fn ctx_w32(ctx: usize, dword: usize, value: u32) {
    write_volatile((ctx + dword * 4) as *mut u32, value);
}
unsafe fn ctx_w64(ctx: usize, qword: usize, value: u64) {
    write_volatile((ctx + qword * 8) as *mut u64, value);
}

fn initial_ep0_mps(speed: u8) -> u16 {
    // Robust defaults used by mature xHCI stacks: LS=8, FS/HS=64, SS=512.
    match speed {
        2 => 8,
        1 | 3 => 64,
        4 | 5 => 512,
        _ => 64,
    }
}

fn fill_slot_context(ctx: usize, chemin: &Chemin, context_entries: u8) {
    unsafe {
        ctx_w32(
            ctx,
            0,
            concentrateur::slot_dw0(chemin.route, chemin.vitesse, context_entries, false, false),
        );
        // Le PORT RACINE, toujours -- pas le port du concentrateur
        // intermediaire, qui est deja dans la chaine de route. Y mettre le
        // port intermediaire designe un autre sous-arbre.
        ctx_w32(ctx, 1, concentrateur::slot_dw1(0, chemin.port_racine, 0));
        // Le transactionneur, s'il en faut un. Sans lui, un clavier basse
        // vitesse derriere un concentrateur haute vitesse est adresse et ne
        // repond a rien -- et c'est le cas COURANT, pas l'exception.
        ctx_w32(ctx, 2, concentrateur::slot_dw2(chemin.tt_slot, chemin.tt_port, 0));
        ctx_w32(ctx, 3, 0);
    }
}

fn fill_endpoint_context(ctx: usize, ep_type: u8, max_packet: u16, interval: u8, ring: &ProducerRing) {
    unsafe {
        ctx_w32(ctx, 0, (interval as u32) << 16);
        let force_event = if ep_type == 7 { 1 } else { 0 };
        ctx_w32(
            ctx,
            1,
            force_event | (3 << 1) | ((ep_type as u32) << 3) | ((max_packet as u32) << 16),
        );
        ctx_w64(ctx, 1, ring.phys | 1); // TR Dequeue Pointer + DCS
        // xHCI 1.2 §6.2.3: EP0 Average TRB Length must be 8 bytes.
        let average = if ep_type == 4 { 8 } else { max_packet.max(1) as u32 };
        let max_esit = if ep_type == 7 { (max_packet as u32) << 16 } else { 0 };
        ctx_w32(ctx, 4, average | max_esit);
    }
}

/// Le plus grand DCI deja declare dans le contexte de slot.
///
/// # Pourquoi on ne repart pas de un
///
/// « Context Entries » borne les points de terminaison que le controleur
/// considere comme existants. Un peripherique composite -- stockage ET clavier
/// dans la meme configuration -- est configure en DEUX commandes, et la
/// seconde recalculait ce champ a partir de ses seuls points. Elle le faisait
/// donc DESCENDRE, et les points de la premiere disparaissaient : la cle etait
/// montee, puis cessait de repondre des que le clavier etait configure.
fn dci_deja_declare(slot_ctx: usize) -> u8 {
    unsafe { ((ctx_r32(slot_ctx, 0) >> 27) & 0x1f) as u8 }
}

fn clear_input(device: &Device) {
    unsafe { write_bytes(device.in_ctx_virt as *mut u8, 0, 4096) };
}

fn address_device(controller: &mut Controller, chemin: &Chemin) -> Result<Device, &'static str> {
    let root_port = chemin.port_racine;
    let speed = chemin.vitesse;
    let slot_id = enable_slot(controller, root_port)?;
    let Some((out_ctx_phys, out_ctx_virt)) = alloc_zeroed(4096) else {
        disable_slot(controller, slot_id);
        return Err("dma-output-context");
    };
    let Some((in_ctx_phys, in_ctx_virt)) = alloc_zeroed(4096) else {
        disable_slot(controller, slot_id);
        return Err("dma-input-context");
    };
    let Some(ep0) = alloc_producer_ring() else {
        disable_slot(controller, slot_id);
        return Err("dma-ep0-ring");
    };
    let Some((control_phys, control_virt)) = alloc_zeroed(4096) else {
        disable_slot(controller, slot_id);
        return Err("dma-control-buffer");
    };

    unsafe {
        write_volatile(
            (controller.dcbaa_virt + slot_id as usize * 8) as *mut u64,
            out_ctx_phys,
        );
        // Input Control Context: add Slot Context + EP0 Context.
        write_volatile((in_ctx_virt + 4) as *mut u32, (1 << 0) | (1 << 1));
    }
    let slot_ctx = context_ptr(in_ctx_virt, controller.context_size, 1);
    let ep0_ctx = context_ptr(in_ctx_virt, controller.context_size, 2);
    let ep0_mps = initial_ep0_mps(speed);
    fill_slot_context(slot_ctx, chemin, 1);
    fill_endpoint_context(ep0_ctx, 4, ep0_mps, 0, &ep0);

    if let Err(error) = command(
        controller,
        in_ctx_phys,
        CMD_ADDRESS_DEVICE,
        slot_id,
    ) {
        disable_slot(controller, slot_id);
        return Err(error);
    }
    // Respect the SET_ADDRESS recovery interval before the first EP0 request.
    wait_ms(2);
    ADDRESS_OK.fetch_add(1, Ordering::Relaxed);
    crate::serial_println!(
        "BOUCHAUD_USB_ADDRESS_OK port={} route={:#07x} profondeur={} slot={} speed={} ep0_mps={} tt_slot={} tt_port={}",
        root_port, chemin.route, chemin.profondeur, slot_id, speed, ep0_mps,
        chemin.tt_slot, chemin.tt_port,
    );

    Ok(Device {
        slot_id,
        root_port,
        speed,
        ep0_mps,
        out_ctx_phys,
        out_ctx_virt,
        in_ctx_phys,
        in_ctx_virt,
        ep0,
        control_phys,
        control_virt,
    })
}

fn setup_packet(bm_request: u8, request: u8, value: u16, index: u16, length: u16) -> u64 {
    (bm_request as u64)
        | ((request as u64) << 8)
        | ((value as u64) << 16)
        | ((index as u64) << 32)
        | ((length as u64) << 48)
}

fn control_transfer(
    controller: &mut Controller,
    device: &mut Device,
    setup: u64,
    data_len: usize,
    data_in: bool,
    budget_ns: u64,
) -> Result<usize, &'static str> {
    if data_len > 4096 {
        return Err("control-buffer-too-small");
    }
    unsafe { write_bytes(device.control_virt as *mut u8, 0, data_len) };

    let trt = if data_len == 0 {
        0
    } else if data_in {
        3
    } else {
        2
    };
    ring_push(
        &mut device.ep0,
        Trb {
            parameter: setup,
            status: 8,
            // Setup Stage TD is exactly one Setup TRB: CH must remain zero.
            control: (TRB_SETUP_STAGE << 10) | TRB_IDT | (trt << 16),
        },
    );

    if data_len != 0 {
        ring_push(
            &mut device.ep0,
            Trb {
                parameter: device.control_phys,
                status: data_len as u32,
                // One contiguous data buffer => one Data Stage TRB: CH=0.
                control: (TRB_DATA_STAGE << 10)
                    | if data_in { TRB_DIR_IN } else { 0 },
            },
        );
    }

    let status_dir = if data_len == 0 || !data_in { TRB_DIR_IN } else { 0 };
    ring_push(
        &mut device.ep0,
        Trb {
            parameter: 0,
            status: 0,
            control: (TRB_STATUS_STAGE << 10) | TRB_IOC | status_dir,
        },
    );
    ring_doorbell(controller, device.slot_id, 1);

    let event = wait_event_budget(
        controller,
        EVT_TRANSFER,
        Some(device.slot_id),
        Some(1),
        budget_ns,
    )
    .ok_or_else(|| {
        // Une echeance n'a pas de code d'achevement : le peripherique n'a rien
        // rendu du tout. C'est le cas qui merite la quarantaine longue.
        DERNIER_CODE_CONTROLE.store(0, Ordering::Relaxed);
        "control-transfer-timeout"
    })?;
    let cc = completion_code(event.status);
    if cc != CC_SUCCESS && cc != CC_SHORT_PACKET {
        crate::serial_println!(
            "BOUCHAUD_USB_CONTROL_FAIL slot={} ep=1 cc={} setup={:#018x} len={} status={:#010x}",
            device.slot_id, cc, setup, data_len, event.status
        );
        DERNIER_CODE_CONTROLE.store(cc as usize, Ordering::Relaxed);
        return Err("control-transfer-error");
    }
    CONTROL_OK.fetch_add(1, Ordering::Relaxed);
    let residual = (event.status & 0x00ff_ffff) as usize;
    Ok(data_len.saturating_sub(residual.min(data_len)))
}

fn get_descriptor(
    controller: &mut Controller,
    device: &mut Device,
    descriptor_type: u8,
    index: u8,
    length: usize,
) -> Result<usize, &'static str> {
    let setup = setup_packet(
        0x80,
        6,
        ((descriptor_type as u16) << 8) | index as u16,
        0,
        length as u16,
    );
    control_transfer(controller, device, setup, length, true, BUDGET_ATTENTE_NS)
}

fn evaluate_ep0_mps(controller: &mut Controller, device: &mut Device, mps: u16) -> Result<(), &'static str> {
    if mps == device.ep0_mps {
        return Ok(());
    }
    clear_input(device);
    let ep0_out = context_ptr(device.out_ctx_virt, controller.context_size, 1);
    let ep0_ctx = context_ptr(device.in_ctx_virt, controller.context_size, 2);
    unsafe {
        // Evaluate Context: only EP0 is evaluated. Start from the controller's
        // current output context so dequeue/state fields remain identical; only
        // Max Packet Size is changed after the first 8-byte descriptor read.
        write_volatile((device.in_ctx_virt + 4) as *mut u32, 1 << 1);
        copy_nonoverlapping(
            ep0_out as *const u8,
            ep0_ctx as *mut u8,
            controller.context_size,
        );
        let dw1 = ctx_r32(ep0_ctx, 1);
        ctx_w32(ep0_ctx, 1, (dw1 & 0x0000_ffff) | ((mps as u32) << 16));
    }
    command(
        controller,
        device.in_ctx_phys,
        CMD_EVALUATE_CONTEXT,
        device.slot_id,
    )?;
    device.ep0_mps = mps;
    Ok(())
}

fn set_configuration(controller: &mut Controller, device: &mut Device, value: u8) -> Result<(), &'static str> {
    let setup = setup_packet(0x00, 9, value as u16, 0, 0);
    let _ = control_transfer(controller, device, setup, 0, false, BUDGET_ATTENTE_NS)?;
    Ok(())
}

fn set_boot_protocol(controller: &mut Controller, device: &mut Device, interface: u8) -> bool {
    let setup = setup_packet(0x21, 0x0b, 0, interface as u16, 0);
    control_transfer(controller, device, setup, 0, false, BUDGET_ATTENTE_NS).is_ok()
}

fn set_idle(controller: &mut Controller, device: &mut Device, interface: u8) {
    // HID SET_IDLE duration=0/report-id=0. Boot keyboards do not require this
    // for state changes, but several real devices only start cleanly after the
    // host has completed the class initialization sequence used by PC stacks.
    let setup = setup_packet(0x21, 0x0a, 0, interface as u16, 0);
    let _ = control_transfer(controller, device, setup, 0, false, BUDGET_ATTENTE_NS);
}

fn hid_item_value(bytes: &[u8]) -> u32 {
    let mut value = 0u32;
    for (shift, byte) in bytes.iter().take(4).enumerate() {
        value |= (*byte as u32) << (shift * 8);
    }
    value
}

fn classify_hid_report_descriptor(bytes: &[u8]) -> (u8, u8) {
    // Report-aware classifier. In addition to the canonical top-level Mouse /
    // Keyboard application usages, keep semantic hints seen in nested
    // collections. Several wireless receivers put Pointer/X/Y/Button usages in
    // a report collection whose top-level application is vendor-specific.
    let mut offset = 0usize;
    let mut usage_page = 0u32;
    let mut local_usage = 0u32;
    let mut kind = 0u8;
    let mut report_id = 0u8;
    let mut saw_pointer = false;
    let mut saw_x = false;
    let mut saw_y = false;
    let mut saw_buttons = false;
    let mut saw_keyboard_page = false;
    while offset < bytes.len() {
        let prefix = bytes[offset];
        offset += 1;
        if prefix == 0xfe {
            if offset + 2 > bytes.len() { break; }
            let size = bytes[offset] as usize;
            offset = offset.saturating_add(2).saturating_add(size);
            continue;
        }
        let size_code = (prefix & 0x03) as usize;
        let size = if size_code == 3 { 4 } else { size_code };
        if offset + size > bytes.len() { break; }
        let ty = (prefix >> 2) & 0x03;
        let tag = (prefix >> 4) & 0x0f;
        let value = hid_item_value(&bytes[offset..offset + size]);
        offset += size;
        match (ty, tag) {
            (1, 0) => {
                usage_page = value;
                if usage_page == 0x07 { saw_keyboard_page = true; }
                if usage_page == 0x09 { saw_buttons = true; }
            }
            (1, 8) => {
                if report_id == 0 { report_id = value as u8; }
            }
            (2, 0) => {
                local_usage = value;
                if usage_page == 0x01 {
                    if value == 0x01 || value == 0x02 { saw_pointer = true; }
                    if value == 0x30 { saw_x = true; }
                    if value == 0x31 { saw_y = true; }
                    if value == 0x06 { kind = 1; }
                }
            }
            (0, 10) => { // Collection
                if usage_page == 0x01 && local_usage == 0x02 {
                    kind = 2; // Generic Desktop / Mouse
                } else if usage_page == 0x01 && local_usage == 0x06 {
                    kind = 1; // Generic Desktop / Keyboard
                }
                local_usage = 0;
            }
            (0, _) => local_usage = 0,
            _ => {}
        }
    }
    if kind == 0 && saw_keyboard_page {
        kind = 1;
    }
    if kind == 0 && (saw_pointer || (saw_x && saw_y) || (saw_buttons && (saw_x || saw_y))) {
        kind = 2;
    }
    (kind, report_id)
}

fn get_hid_report_descriptor(
    controller: &mut Controller,
    device: &mut Device,
    interface: u8,
    length: usize,
) -> Result<usize, &'static str> {
    let length = length.min(4096);
    if length == 0 { return Err("hid-report-length-zero"); }
    let setup = setup_packet(
        0x81, // device-to-host, standard, interface
        6,    // GET_DESCRIPTOR
        (0x22u16) << 8,
        interface as u16,
        length as u16,
    );
    control_transfer(controller, device, setup, length, true, BUDGET_ATTENTE_NS)
}

fn enrich_hid_descriptor(
    controller: &mut Controller,
    device: &mut Device,
    descriptor: &mut HidDescriptor,
) {
    // Boot interfaces have an explicit protocol and are the most reliable path.
    descriptor.kind = match (descriptor.subclass, descriptor.protocol) {
        (1, 1) => 1,
        (1, 2) => 2,
        _ => 0,
    };

    // Report-only interfaces (subclass/protocol 0) are extremely common on
    // wireless receivers and gaming mice. Read their HID Report Descriptor and
    // classify the application collection instead of dropping them.
    if descriptor.kind == 0 && descriptor.report_len != 0 {
        match get_hid_report_descriptor(
            controller,
            device,
            descriptor.interface,
            descriptor.report_len as usize,
        ) {
            Ok(received) if received != 0 => {
                let bytes = unsafe {
                    core::slice::from_raw_parts(device.control_virt as *const u8, received)
                };
                let (kind, report_id) = classify_hid_report_descriptor(bytes);
                if descriptor.kind == 0 { descriptor.kind = kind; }
                descriptor.report_id = report_id;
                // Le descripteur a parle, y compris quand il dit « ni clavier
                // ni souris » : c'est un verdict, pas une absence de verdict.
                descriptor.classe_par_rapport = true;
                crate::serial_println!(
                    "BOUCHAUD_HID_REPORT_DESC slot={} if={} subclass={} protocol={} kind={} report_id={} bytes={}",
                    device.slot_id, descriptor.interface, descriptor.subclass,
                    descriptor.protocol, descriptor.kind, descriptor.report_id, received,
                );
            }
            _ => {
                crate::serial_println!(
                    "BOUCHAUD_HID_REPORT_DESC_FAIL slot={} if={} len={}",
                    device.slot_id, descriptor.interface, descriptor.report_len,
                );
            }
        }
    }
}

fn parse_hid_descriptors(bytes: &[u8], output: &mut [HidDescriptor]) -> (u8, usize) {
    if bytes.len() < 9 || bytes[1] != 2 {
        return (0, 0);
    }
    let configuration_value = bytes[5];
    let mut current_interface = 0u8;
    let mut current_subclass = 0u8;
    let mut current_protocol = 0u8;
    let mut current_hid = false;
    let mut current_report_len = 0u16;
    let mut count = 0usize;
    let mut offset = 0usize;

    while offset + 2 <= bytes.len() {
        let len = bytes[offset] as usize;
        let ty = bytes[offset + 1];
        if len < 2 || offset + len > bytes.len() {
            break;
        }
        match ty {
            4 if len >= 9 => {
                current_interface = bytes[offset + 2];
                let alternate = bytes[offset + 3];
                let class = bytes[offset + 5];
                current_subclass = bytes[offset + 6];
                current_protocol = bytes[offset + 7];
                current_hid = alternate == 0 && class == 3;
                current_report_len = 0;
                if current_hid {
                    crate::serial_println!(
                        "BOUCHAUD_HID_INTERFACE if={} subclass={} protocol={}",
                        current_interface, current_subclass, current_protocol,
                    );
                }
            }
            0x21 if current_hid && len >= 9 => {
                // HID descriptor: first subordinate descriptor is normally the
                // Report Descriptor (0x22), with its length in bytes 7..8.
                let count_desc = bytes[offset + 5] as usize;
                let mut pos = offset + 6;
                for _ in 0..count_desc {
                    if pos + 3 > offset + len { break; }
                    if bytes[pos] == 0x22 {
                        current_report_len = u16::from_le_bytes([bytes[pos + 1], bytes[pos + 2]]);
                        break;
                    }
                    pos += 3;
                }
            }
            5 if len >= 7 && current_hid && count < output.len() => {
                let address = bytes[offset + 2];
                let attributes = bytes[offset + 3] & 0x03;
                if address & 0x80 != 0 && attributes == 3 {
                    let kind = hid::genre_interface(current_subclass, current_protocol);
                    output[count] = HidDescriptor {
                        interface: current_interface,
                        subclass: current_subclass,
                        protocol: current_protocol,
                        kind,
                        classe_par_rapport: false,
                        report_len: current_report_len,
                        report_id: 0,
                        endpoint_address: address,
                        max_packet: u16::from_le_bytes([
                            bytes[offset + 4],
                            bytes[offset + 5],
                        ]) & 0x07ff,
                        interval: bytes[offset + 6],
                    };
                    count += 1;
                }
            }
            _ => {}
        }
        offset += len;
    }
    (configuration_value, count)
}

fn xhci_interval(speed: u8, usb_interval: u8) -> u8 {
    if speed >= 3 {
        // High/Super: xHCI stores bInterval-1.
        return usb_interval.max(1).saturating_sub(1).min(15);
    }
    // Low/Full speed bInterval is expressed in 1ms frames. xHCI wants the
    // largest power-of-two microframe interval not exceeding that period:
    // floor(log2(8*bInterval)). This matches mature xHCI host stacks and fixes
    // the V3.2 ceil() off-by-one on non-power-of-two intervals (e.g. 10ms).
    let microframes = (usb_interval.max(1) as u32).saturating_mul(8);
    let log2 = (31u32.saturating_sub(microframes.leading_zeros())) as u8;
    log2.clamp(3, 15)
}

fn configure_hids(
    controller: &mut Controller,
    device: &mut Device,
    descriptors: &mut [HidDescriptor],
) -> Result<usize, &'static str> {
    if descriptors.is_empty() {
        return Ok(0);
    }
    for descriptor in descriptors.iter_mut() {
        enrich_hid_descriptor(controller, device, descriptor);
    }
    let available = MAX_HID_ENDPOINTS_PER_CONTROLLER.saturating_sub(controller.hid_count);
    let wanted = descriptors.len().min(available);
    if wanted == 0 {
        return Err("hid-endpoint-table-full");
    }

    clear_input(device);
    let slot_out = context_ptr(device.out_ctx_virt, controller.context_size, 0);
    let slot_in = context_ptr(device.in_ctx_virt, controller.context_size, 1);
    unsafe {
        copy_nonoverlapping(
            slot_out as *const u8,
            slot_in as *mut u8,
            controller.context_size,
        );
    }

    let mut add_flags = 1u32; // Slot Context
    let mut highest_dci = dci_deja_declare(slot_out).max(1);
    let base_index = controller.hid_count;
    let mut installed = 0usize;

    for descriptor in descriptors.iter().take(wanted) {
        let mut kind = descriptor.kind;
        // BOUCHAUD_HID_REPLI_SOURIS_V2
        //
        // CE QUE CE REPLI FAISAIT DE TROP
        // -------------------------------
        // Il promouvait en souris TOUTE interface HID d'entree non classee,
        // meme celle dont le descripteur de rapport venait d'etre lu et
        // repondait « ni clavier ni souris ». Sur la machine de reference,
        // l'interface VENDEUR d'un recepteur sans fil passait ainsi pour une
        // souris : ses notifications (etat de pile, association) etaient
        // relues comme des boutons et un deplacement.
        //
        // Deux degats, tous deux rapportes : le curseur se TELEPORTAIT (des
        // octets de protocole lus comme dx/dy) et le bouton VACILLAIT, ce qui
        // rendait tout glissement de fenetre impossible.
        //
        // Le repli ne parle donc plus que lorsque le descripteur de rapport
        // s'est TU -- illisible ou absent. C'est bien un repli : la ou on ne
        // sait rien, une souris presumee vaut mieux qu'un peripherique perdu.
        if kind == 0
            && !descriptor.classe_par_rapport
            && descriptor.max_packet >= 3
            && descriptor.protocol != 1
        {
            kind = 2;
            crate::serial_println!(
                "BOUCHAUD_HID_HEURISTIC_MOUSE if={} subclass={} protocol={} ep={:#04x}",
                descriptor.interface, descriptor.subclass, descriptor.protocol,
                descriptor.endpoint_address,
            );
        }
        if kind == 0 {
            crate::serial_println!(
                "BOUCHAUD_HID_UNSUPPORTED if={} subclass={} protocol={} ep={:#04x}",
                descriptor.interface, descriptor.subclass, descriptor.protocol,
                descriptor.endpoint_address,
            );
            continue;
        }
        let ep_number = descriptor.endpoint_address & 0x0f;
        if ep_number == 0 {
            continue;
        }
        let dci = ep_number.saturating_mul(2).saturating_add(1);
        if dci >= 32 || descriptor.max_packet == 0 {
            continue;
        }
        let Some(ring) = alloc_producer_ring() else {
            continue;
        };
        let Some((buffer_phys, buffer_virt)) = alloc_zeroed(4096) else {
            continue;
        };
        let buffer_len = (descriptor.max_packet as usize).clamp(3, 64);
        let ep_ctx = context_ptr(
            device.in_ctx_virt,
            controller.context_size,
            dci as usize + 1,
        );
        fill_endpoint_context(
            ep_ctx,
            7, // Interrupt IN
            descriptor.max_packet,
            xhci_interval(device.speed, descriptor.interval),
            &ring,
        );
        add_flags |= 1u32 << dci;
        highest_dci = highest_dci.max(dci);
        controller.hids[base_index + installed] = HidEndpoint {
            arme_depuis_ns: 0,
            echecs_repli: 0,
            quarantaine_annoncee: false,
            repli_muet_jusqu_a_ns: 0,
            evenements: 0,
            active: false,
            slot_id: device.slot_id,
            dci,
            interface: descriptor.interface,
            protocol: descriptor.protocol,
            kind,
            source_souris: if kind == 2 {
                crate::drivers::mouse::reserve_source_souris()
            } else {
                usize::MAX
            },
            report_id: descriptor.report_id,
            max_packet: descriptor.max_packet,
            ring,
            buffer_phys,
            buffer_virt,
            buffer_len,
            clavier: hid::EtatClavier { modificateurs: 0, touches: [0; 6] },
        };
        if kind == 2 {
            // La source est la PREUVE que deux « souris » ne s'ecrasent plus.
            // Sans cette ligne, le releve de vol ne dirait que `souris=3`, ce
            // qui etait deja vrai quand elles partageaient le meme etat.
            crate::serial_println!(
                "BOUCHAUD_HID_SOURCE_SOURIS slot={} if={} ep={:#04x} source={} classe_par_rapport={}",
                device.slot_id,
                descriptor.interface,
                descriptor.endpoint_address,
                controller.hids[base_index + installed].source_souris,
                descriptor.classe_par_rapport as u8,
            );
        }
        installed += 1;
    }

    if installed == 0 {
        return Ok(0);
    }

    unsafe {
        write_volatile((device.in_ctx_virt + 4) as *mut u32, add_flags);
        let dw0 = ctx_r32(slot_in, 0);
        ctx_w32(
            slot_in,
            0,
            (dw0 & !(0x1f << 27)) | ((highest_dci as u32) << 27),
        );
    }

    command(
        controller,
        device.in_ctx_phys,
        CMD_CONFIGURE_ENDPOINT,
        device.slot_id,
    )?;

    for index in base_index..base_index + installed {
        let dci = controller.hids[index].dci;
        let out_ctx = context_ptr(device.out_ctx_virt, controller.context_size, dci as usize);
        let (state, dequeue) = unsafe {
            (ctx_r32(out_ctx, 0) & 0x7, read_volatile((out_ctx + 8) as *const u64))
        };
        crate::serial_println!(
            "BOUCHAUD_HID_ENDPOINT_STATE slot={} dci={} state={} deq={:#x} ring={:#x}",
            device.slot_id, dci, state, dequeue, controller.hids[index].ring.phys,
        );
    }

    for index in base_index..base_index + installed {
        let interface = controller.hids[index].interface;
        let protocol = controller.hids[index].protocol;
        if protocol == 1 || protocol == 2 {
            let boot = set_boot_protocol(controller, device, interface);
            crate::serial_println!(
                "BOUCHAUD_HID_SET_PROTOCOL slot={} if={} boot={}",
                device.slot_id, interface, boot as u8,
            );
        }
        set_idle(controller, device, interface);
    }
    wait_ms(10);

    let arme_a = crate::kernel::timer::monotonic_ns();
    for index in base_index..base_index + installed {
        // Arm only after every port on this controller has finished control
        // enumeration, so interrupt events cannot steal command/control events.
        controller.hids[index].active = true;
        controller.hids[index].arme_depuis_ns = arme_a;
    }
    controller.hid_count += installed;
    Ok(installed)
}

fn arm_hid_endpoint(controller: &mut Controller, index: usize) {
    if index >= controller.hids.len() {
        return;
    }
    let (slot, dci) = {
        let endpoint = &mut controller.hids[index];
        unsafe { write_bytes(endpoint.buffer_virt as *mut u8, 0, endpoint.buffer_len) };
        ring_push(
            &mut endpoint.ring,
            Trb {
                parameter: endpoint.buffer_phys,
                status: endpoint.buffer_len as u32,
                // ISP matters for HID: many endpoints advertise 64-byte MPS
                // while keyboard/mouse reports are only 8/4 bytes. Ask for a
                // Transfer Event on that short packet as well as IOC.
                control: (TRB_NORMAL << 10) | TRB_IOC | TRB_ISP,
            },
        );
        (endpoint.slot_id, endpoint.dci)
    };
    fence(Ordering::SeqCst);
    ring_doorbell(controller, slot, dci);
    HID_REARMS.fetch_add(1, Ordering::Relaxed);
}

fn arm_all_hids(controller: &mut Controller) {
    for index in 0..controller.hid_count {
        if controller.hids[index].active {
            arm_hid_endpoint(controller, index);
        }
    }
}

/// Note ce peripherique dans l'arbre du controleur.
fn note_noeud(
    controller: &mut Controller,
    chemin: &Chemin,
    slot: u8,
    vendeur: u16,
    produit: u16,
    classe: u8,
    genre_hid: u8,
) {
    if controller.compte_noeuds >= NOEUDS_MAX {
        return;
    }
    let index = controller.compte_noeuds;
    controller.arbre[index] = Some(NoeudUsb {
        port_racine: chemin.port_racine,
        route: chemin.route,
        profondeur: chemin.profondeur.min(u8::MAX as usize) as u8,
        vitesse: chemin.vitesse,
        slot,
        vendeur,
        produit,
        classe,
        genre_hid,
    });
    controller.compte_noeuds = index + 1;
}

/// `bConfigurationValue` de la premiere configuration.
///
/// Un concentrateur non configure refuse les requetes de ses ports et se
/// comporte comme s'il n'avait aucun port occupe -- c'est-a-dire exactement
/// comme un concentrateur vide. Rien ne distingue les deux cas dans le
/// journal, d'ou l'importance de ne pas sauter cette etape.
fn valeur_de_configuration(controller: &mut Controller, device: &mut Device) -> Option<u8> {
    let recu = get_descriptor(controller, device, 2, 0, 9).ok()?;
    if recu < 9 {
        return None;
    }
    let valeur = unsafe { read_volatile((device.control_virt + 5) as *const u8) };
    if valeur == 0 {
        None
    } else {
        Some(valeur)
    }
}

/// Declare au controleur que ce peripherique EST un concentrateur.
///
/// # Sans cette commande, la traversee ne sert a rien
///
/// Le controleur refuse d'adresser quoi que ce soit derriere un slot dont le
/// bit `Hub` n'est pas pose : il ne sait pas qu'il y a un « derriere ». La
/// commande echoue avec un code de parametre invalide, et le journal dit
/// « adresse impossible » sans jamais nommer la cause.
///
/// Le nombre de ports et le temps de reflexion vont dans la meme commande :
/// le controleur s'en sert pour dimensionner ses fenetres de transaction. Un
/// temps de reflexion faux ne casse rien tout de suite -- il fait perdre des
/// paquets sous charge, ce qui se manifeste par une souris qui saute.
fn declare_concentrateur(
    controller: &mut Controller,
    device: &mut Device,
    ports: u8,
    temps_reflexion: u8,
) -> Result<(), &'static str> {
    clear_input(device);
    let slot_out = context_ptr(device.out_ctx_virt, controller.context_size, 0);
    let slot_in = context_ptr(device.in_ctx_virt, controller.context_size, 1);
    unsafe {
        // On part du contexte de SORTIE, celui que le controleur tient a jour :
        // les champs qu'on ne change pas doivent rester identiques, sans quoi
        // on ecraserait l'etat du slot avec des zeros.
        copy_nonoverlapping(slot_out as *const u8, slot_in as *mut u8, controller.context_size);
        // A0 seul : on ne touche a aucun point de terminaison.
        write_volatile((device.in_ctx_virt + 4) as *mut u32, 1 << 0);
        let dw0 = ctx_r32(slot_in, 0);
        ctx_w32(slot_in, 0, dw0 | (1 << 26));
        let dw1 = ctx_r32(slot_in, 1);
        ctx_w32(slot_in, 1, (dw1 & 0x00ff_ffff) | ((ports as u32) << 24));
        let dw2 = ctx_r32(slot_in, 2);
        ctx_w32(slot_in, 2, (dw2 & !(0x3 << 16)) | (((temps_reflexion & 0x3) as u32) << 16));
    }
    command(
        controller,
        device.in_ctx_phys,
        CMD_CONFIGURE_ENDPOINT,
        device.slot_id,
    )?;
    Ok(())
}

/// Lit les quatre octets d'etat d'un port de concentrateur.
fn etat_port_concentrateur(
    controller: &mut Controller,
    device: &mut Device,
    port: u8,
    superspeed: bool,
) -> Option<concentrateur::EtatPort> {
    let recu = control_transfer(
        controller,
        device,
        concentrateur::requete_etat_port(port),
        4,
        true,
        BUDGET_ATTENTE_NS,
    )
    .ok()?;
    if recu < 4 {
        return None;
    }
    let octets = unsafe { core::slice::from_raw_parts(device.control_virt as *const u8, 4) };
    concentrateur::etat_port(octets, superspeed)
}

/// Efface les bits de changement d'un port, un par un.
///
/// Un changement qu'on laisse pose est re-signale sans fin : le concentrateur
/// le repete, et une traversee qui le relit rebranche indefiniment le meme
/// peripherique.
fn efface_changements(
    controller: &mut Controller,
    device: &mut Device,
    port: u8,
    changements: u16,
) {
    for (bit, fonctionnalite) in concentrateur::CHANGEMENTS {
        if changements & bit != 0 {
            let _ = control_transfer(
                controller,
                device,
                concentrateur::requete_efface_port(fonctionnalite, port),
                0,
                false,
                BUDGET_ATTENTE_NS,
            );
        }
    }
}

/// Reinitialise un port de concentrateur et rend la vitesse negociee.
///
/// La vitesse n'est lisible qu'APRES la reinitialisation : les bits qui la
/// portent ne veulent rien dire tant que le port n'est pas actif. La lire
/// avant programme une taille de paquet fausse, donc un peripherique qui ne
/// repond jamais.
fn reinitialise_port_concentrateur(
    controller: &mut Controller,
    device: &mut Device,
    port: u8,
    superspeed: bool,
) -> Option<u8> {
    if control_transfer(
        controller,
        device,
        concentrateur::requete_pose_port(concentrateur::PORT_REINITIALISATION, port),
        0,
        false,
        BUDGET_ATTENTE_NS,
    )
    .is_err()
    {
        return None;
    }
    // Bornee : un port qui ne sort jamais de reinitialisation ne doit pas
    // suspendre le demarrage de la machine.
    let mut restant = REINITIALISATION_MAX_MS / 10;
    loop {
        wait_ms(10);
        let etat = etat_port_concentrateur(controller, device, port, superspeed)?;
        if !etat.en_reinitialisation && etat.active {
            efface_changements(controller, device, port, etat.changements);
            // Intervalle de reprise apres reinitialisation, USB 2.0 §7.1.7.5.
            wait_ms(10);
            return Some(etat.vitesse);
        }
        if !etat.connecte {
            efface_changements(controller, device, port, etat.changements);
            return None;
        }
        restant = restant.saturating_sub(1);
        if restant == 0 {
            crate::serial_println!(
                "BOUCHAUD_USB_CONCENTRATEUR_PORT_TIMEOUT slot={} port={}",
                device.slot_id, port,
            );
            return None;
        }
    }
}

/// Descend d'un etage : alimente, reinitialise, et met en file ce qui repond.
///
/// # Ce que cette fonction ne fait PAS
///
/// Elle n'enumere rien elle-meme. Chaque peripherique trouve est POUSSE dans
/// la file, et enumere plus tard, a plat. C'est ce qui garde la pile du noyau
/// constante quelle que soit la profondeur de l'arbre.
fn traverse_concentrateur(
    controller: &mut Controller,
    device: &mut Device,
    chemin: &Chemin,
    file: &mut FileChemins,
) -> Result<usize, &'static str> {
    let superspeed = chemin.vitesse >= concentrateur::VITESSE_SUPER;
    let longueur = if superspeed { 12 } else { 9 };
    let recu = control_transfer(
        controller,
        device,
        concentrateur::requete_descripteur(superspeed, longueur as u16),
        longueur,
        true,
        BUDGET_ATTENTE_NS,
    )?;
    let octets = unsafe { core::slice::from_raw_parts(device.control_virt as *const u8, recu) };
    let Some(descripteur) = concentrateur::descripteur(octets) else {
        // LES OCTETS, PAS SEULEMENT LE VERDICT.
        //
        // « descripteur invalide » ne dit pas lequel des champs a ete refuse,
        // et un concentrateur refuse a tort est indiscernable d'un
        // concentrateur reellement casse. Les six premiers octets suffisent a
        // trancher, et ils tiennent sur une ligne.
        let mut apercu = [0u8; 6];
        for (index, octet) in apercu.iter_mut().enumerate() {
            *octet = if index < octets.len() { octets[index] } else { 0 };
        }
        crate::serial_println!(
            "BOUCHAUD_USB_CONCENTRATEUR_DESCRIPTEUR slot={} lu={} octets={:02x?}",
            device.slot_id, recu, apercu,
        );
        return Err("descripteur-concentrateur-invalide");
    };

    declare_concentrateur(
        controller,
        device,
        descripteur.ports,
        descripteur.temps_reflexion,
    )?;

    // Alimenter TOUS les ports d'abord, puis attendre UNE fois : le delai
    // d'etablissement court en parallele sur tous les ports.
    for port in 1..=descripteur.ports {
        let _ = control_transfer(
            controller,
            device,
            concentrateur::requete_pose_port(concentrateur::PORT_ALIMENTATION, port),
            0,
            false,
            BUDGET_ATTENTE_NS,
        );
    }
    wait_ms(descripteur.delai_alimentation_ms.max(20).min(600) as u64);

    let mut trouves = 0usize;
    for port in 1..=descripteur.ports {
        let Some(etat) = etat_port_concentrateur(controller, device, port, superspeed) else {
            continue;
        };
        if etat.changements != 0 {
            efface_changements(controller, device, port, etat.changements);
        }
        if !etat.connecte {
            continue;
        }
        let Some(vitesse) = reinitialise_port_concentrateur(controller, device, port, superspeed)
        else {
            continue;
        };
        let Some(route) = concentrateur::route_enfant(chemin.route, chemin.profondeur, port) else {
            CONCENTRATEURS_ECHOUES.fetch_add(1, Ordering::Relaxed);
            crate::serial_println!(
                "BOUCHAUD_USB_CONCENTRATEUR_TROP_PROFOND slot={} port={} profondeur={} \
                 cause=la-chaine-de-route-xhci-ne-compte-que-cinq-etages",
                device.slot_id, port, chemin.profondeur,
            );
            continue;
        };
        // Le transactionneur s'HERITE : un concentrateur pleine vitesse
        // derriere un concentrateur haute vitesse garde celui de son ancetre.
        // Ne le recalculer qu'au premier saut donnerait, plus bas, un
        // transactionneur nul et un peripherique muet.
        let (tt_slot, tt_port) = if chemin.tt_slot != 0 {
            (chemin.tt_slot, chemin.tt_port)
        } else if concentrateur::requiert_transactionneur(chemin.vitesse, vitesse) {
            (device.slot_id, port)
        } else {
            (0, 0)
        };
        file.pousse(Chemin {
            port_racine: chemin.port_racine,
            route,
            profondeur: chemin.profondeur + 1,
            vitesse,
            tt_slot,
            tt_port,
        });
        trouves += 1;
    }
    crate::serial_println!(
        "BOUCHAUD_USB_CONCENTRATEUR_TRAVERSE slot={} ports={} occupes={} \
         temps_reflexion={} delai_alimentation_ms={}",
        device.slot_id, descripteur.ports, trouves, descripteur.temps_reflexion,
        descripteur.delai_alimentation_ms,
    );
    Ok(trouves)
}

fn enumerate_port(controller: &mut Controller, port_index: usize, file: &mut FileChemins) {
    let port = (port_index + 1) as u8;
    let portsc = unsafe { r32(controller.op, 0x400 + port_index * 0x10) };
    if portsc & PORTSC_CCS == 0 { return; }
    if portsc & PORTSC_PED == 0 {
        crate::serial_println!(
            "BOUCHAUD_USB_ENUM_FAIL port={} phase=port-not-enabled portsc={:#010x}",
            port, portsc
        );
        return;
    }
    let speed = ((portsc >> PORTSC_SPEED_SHIFT) & 0x0f) as u8;
    enumerate_device(controller, Chemin::racine(port, speed), file);
}

/// Enumere UN peripherique, ou qu'il soit dans l'arbre.
///
/// Un peripherique branche a la racine et un peripherique branche derriere
/// trois concentrateurs suivent exactement la meme suite d'etapes ; seul le
/// `Chemin` change. Avoir deux chemins de code pour les deux cas, c'est
/// garantir qu'un seul des deux sera corrige quand un defaut apparaitra.
fn enumerate_device(controller: &mut Controller, chemin: Chemin, file: &mut FileChemins) {
    let port = chemin.port_racine;
    let speed = chemin.vitesse;
    let mut device = match address_device(controller, &chemin) {
        Ok(device) => device,
        Err(error) => {
            crate::serial_println!(
                "BOUCHAUD_USB_ENUM_FAIL port={} route={:#07x} phase=address error={}",
                port, chemin.route, error
            );
            if chemin.profondeur != 0 {
                CONCENTRATEURS_ECHOUES.fetch_add(1, Ordering::Relaxed);
            }
            return;
        }
    };

    // Full-speed devices disclose their real EP0 MPS in the first 8 bytes.
    if device.speed <= 2 {
        match get_descriptor(controller, &mut device, 1, 0, 8) {
            Ok(received) if received >= 8 => {
                let encoded = unsafe { read_volatile((device.control_virt + 7) as *const u8) };
                let mps = if device.speed >= 4 {
                    1u16.checked_shl(encoded as u32).unwrap_or(512)
                } else {
                    encoded as u16
                };
                if matches!(mps, 8 | 16 | 32 | 64 | 512) {
                    if let Err(error) = evaluate_ep0_mps(controller, &mut device, mps) {
                        crate::serial_println!(
                            "BOUCHAUD_USB_ENUM_FAIL port={} phase=eval-mps error={}",
                            port,
                            error
                        );
                        abandonne_device(controller, &device);
                        return;
                    }
                }
            }
            _ => {
                abandonne_device(controller, &device);
                return;
            }
        }
    }

    let device_len = match get_descriptor(controller, &mut device, 1, 0, 18) {
        Ok(len) if len >= 18 => len,
        _ => {
            crate::serial_println!("BOUCHAUD_USB_ENUM_FAIL port={} phase=device-descriptor", port);
            abandonne_device(controller, &device);
            return;
        }
    };
    let _ = device_len;
    let vendor = unsafe {
        u16::from_le_bytes([
            read_volatile((device.control_virt + 8) as *const u8),
            read_volatile((device.control_virt + 9) as *const u8),
        ])
    };
    let product = unsafe {
        u16::from_le_bytes([
            read_volatile((device.control_virt + 10) as *const u8),
            read_volatile((device.control_virt + 11) as *const u8),
        ])
    };

    // UN CONCENTRATEUR SE TRAVERSE, ET S'IL NE SE TRAVERSE PAS, IL SE DIT
    //
    // Un clavier branche derriere un concentrateur ne repond pas tant que
    // personne n'est descendu chercher ses ports. Et un echec de descente ne
    // ressemble a rien : le clavier est simplement absent. Les deux moities
    // comptent donc -- descendre, et nommer l'echec quand on ne peut pas.
    let classe_peripherique =
        unsafe { read_volatile((device.control_virt + 4) as *const u8) };
    if classe_peripherique == CLASSE_CONCENTRATEUR {
        CONCENTRATEURS.fetch_add(1, Ordering::Relaxed);
        crate::serial_println!(
            "BOUCHAUD_USB_CONCENTRATEUR port={} route={:#07x} profondeur={} slot={} \
             vid={:04x} pid={:04x}",
            port, chemin.route, chemin.profondeur, device.slot_id, vendor, product,
        );
        // Un concentrateur doit etre CONFIGURE avant de repondre aux requetes
        // de ses ports : non configure, il refuse `GET_STATUS` et se comporte
        // comme s'il n'avait aucun port occupe.
        let configuration = valeur_de_configuration(controller, &mut device);
        match configuration
            .ok_or("configuration-introuvable")
            .and_then(|valeur| {
                set_configuration(controller, &mut device, valeur)?;
                wait_ms(10);
                traverse_concentrateur(controller, &mut device, &chemin, file)
            }) {
            Ok(_) => {}
            Err(error) => {
                CONCENTRATEURS_ECHOUES.fetch_add(1, Ordering::Relaxed);
                crate::serial_println!(
                    "BOUCHAUD_USB_CONCENTRATEUR_ECHEC port={} slot={} error={} \
                     consequence=les-peripheriques-derriere-ce-concentrateur-\
                     ne-sont-pas-enumeres",
                    port, device.slot_id, error,
                );
            }
        }
        if (device.slot_id as usize) < controller.devices.len() {
            controller.devices[device.slot_id as usize] = Some(device);
        }
        note_noeud(controller, &chemin, device.slot_id, vendor, product, classe_peripherique, 0);
        controller.usb_devices += 1;
        return;
    }
    if chemin.profondeur != 0 {
        PERIPHERIQUES_DERRIERE.fetch_add(1, Ordering::Relaxed);
    }

    let config_head = match get_descriptor(controller, &mut device, 2, 0, 9) {
        Ok(len) if len >= 9 => len,
        _ => {
            crate::serial_println!("BOUCHAUD_USB_ENUM_FAIL port={} phase=config-header", port);
            abandonne_device(controller, &device);
            return;
        }
    };
    let _ = config_head;
    let total = unsafe {
        u16::from_le_bytes([
            read_volatile((device.control_virt + 2) as *const u8),
            read_volatile((device.control_virt + 3) as *const u8),
        ]) as usize
    }
    .clamp(9, MAX_CONFIG_DESCRIPTOR);

    let config_len = match get_descriptor(controller, &mut device, 2, 0, total) {
        Ok(len) if len >= 9 => len,
        _ => {
            crate::serial_println!("BOUCHAUD_USB_ENUM_FAIL port={} phase=config-descriptor", port);
            abandonne_device(controller, &device);
            return;
        }
    };

    let mut descriptors = [EMPTY_HID_DESCRIPTOR; MAX_HID_ENDPOINTS_PER_CONTROLLER];
    let config_slice = unsafe {
        core::slice::from_raw_parts(device.control_virt as *const u8, config_len)
    };
    let blackbox_descriptor = parse_blackbox_storage_descriptor(config_slice);
    let (configuration_value, hid_count) = parse_hid_descriptors(config_slice, &mut descriptors);
    // LE DESCRIPTEUR EST LU MAINTENANT, PAS PLUS TARD.
    //
    // `config_slice` designe le tampon de controle du peripherique, que le
    // PROCHAIN transfert de controle reecrira. Le lire apres un
    // `SET_CONFIGURATION` chercherait l'interface de stockage dans la reponse
    // d'une autre requete.
    let interface_stockage = stockage::trouve_interface_stockage(config_slice);
    let genre_hid = descriptors[..hid_count]
        .iter()
        .map(|d| d.kind)
        .find(|k| *k != 0)
        .unwrap_or(0);
    note_noeud(
        controller, &chemin, device.slot_id, vendor, product, classe_peripherique, genre_hid,
    );
    controller.usb_devices += 1;
    crate::serial_println!(
        "BOUCHAUD_USB_ENUM_OK port={} slot={} speed={} vid={:04x} pid={:04x} hid_in_endpoints={}",
        port,
        device.slot_id,
        speed,
        vendor,
        product,
        hid_count
    );

    // LE PERIPHERIQUE EST ENREGISTRE MEME QUAND IL N'A RIEN QUI NOUS SERVE.
    //
    // Une cle USB, une carte son, un lecteur d'empreintes : rien a piloter,
    // mais un slot pris et quatre pages de contextes. Sans enregistrement, le
    // debranchement ne rend ni l'un ni les autres -- et le journal ne dit rien
    // non plus, puisque l'arbre ne le connait pas.
    //
    // L'enregistrement est REFAIT plus bas apres la configuration : l'anneau
    // de controle avance a chaque transfert, et garder une copie perimee
    // ferait ecrire au mauvais endroit.
    if (device.slot_id as usize) < controller.devices.len() {
        controller.devices[device.slot_id as usize] = Some(device);
    }

    // DEUX CONSOMMATEURS POUR UN MEME PERIPHERIQUE, ET UN SEUL PEUT L'AVOIR.
    //
    // L'enregistreur de vol reclame une cle dont le disque porte la partition
    // `BOUCHAUD-BLACKBOX` ; le pilote de volume reclame n'importe quelle
    // interface BOT/SCSI. Sur la meme cle, ce sont LES MEMES points de
    // terminaison : les configurer deux fois creerait deux anneaux pour un
    // seul DCI, et le second ecraserait le premier sans que rien ne le dise.
    //
    // L'enregistreur passe donc en premier -- c'est la ligne de vie du
    // diagnostic physique, et il ne prend que les cles qui portent SA
    // partition. Ce qu'il n'a pas pris revient au pilote de volume.
    //
    // La configuration est posee UNE fois, avant les deux : un peripherique
    // composite doit garder toutes ses moities, et un second
    // `SET_CONFIGURATION` reinitialiserait ce que le premier vient d'etablir.
    let veut_stockage = interface_stockage.is_some();
    let veut_blackbox = blackbox_descriptor.is_some();
    if (hid_count == 0 && !veut_stockage && !veut_blackbox) || configuration_value == 0 {
        // L'enumeration est verte, mais il n'y a rien a piloter ici.
        if (device.slot_id as usize) < controller.devices.len() {
            controller.devices[device.slot_id as usize] = Some(device);
        }
        return;
    }

    if let Err(error) = set_configuration(controller, &mut device, configuration_value) {
        crate::serial_println!(
            "BOUCHAUD_HID_CONFIG_FAIL port={} phase=set-configuration error={}",
            port,
            error
        );
        if (device.slot_id as usize) < controller.devices.len() {
            controller.devices[device.slot_id as usize] = Some(device);
        }
        return;
    }
    wait_ms(10);

    // BOUCHAUD_TRIGKEY_BLACKBOX_V2
    // Une interface Mass Storage n'est conservee comme cible d'ecriture que si
    // SON PROPRE disque contient la partition GPT BOUCHAUD-BLACKBOX.
    let mut pris_par_la_blackbox = false;
    if let Some(storage_descriptor) = blackbox_descriptor {
        match configure_blackbox_storage(controller, &mut device, storage_descriptor) {
            Ok(true) => pris_par_la_blackbox = true,
            Ok(false) => {
                crate::serial_println!(
                    "BOUCHAUD_BLACKBOX_USB_SKIP slot={} cause=partition-absente-ou-support-incompatible",
                    device.slot_id
                );
            }
            Err(error) => {
                crate::serial_println!(
                    "BOUCHAUD_BLACKBOX_USB_SKIP slot={} cause={}",
                    device.slot_id,
                    error
                );
            }
        }
    }

    // UN SUPPORT DE MASSE N'EST PAS UN HID, ET C'EST TOUT CE QUI LES SEPARE.
    //
    // Le peripherique etait enumere, il apparaissait dans `lsusb`, et il ne
    // servait a rien : le pilote ne savait armer qu'un point INTERRUPT IN. Une
    // cle USB demande deux points BULK, dans les deux sens.
    if let Some(iface) = interface_stockage {
        if pris_par_la_blackbox {
            crate::serial_println!(
                "BOUCHAUD_USB_STOCKAGE_CEDE slot={} raison=enregistreur-de-vol",
                device.slot_id
            );
        } else {
            match configure_stockage(controller, &mut device, &iface) {
                Ok(index) => {
                    if let Err(erreur) = demarre_stockage(controller, &mut device, index) {
                        crate::serial_println!(
                            "BOUCHAUD_USB_STOCKAGE_ECHEC port={} phase=demarrage error={}",
                            port, erreur,
                        );
                    }
                }
                Err(erreur) => {
                    crate::serial_println!(
                        "BOUCHAUD_USB_STOCKAGE_ECHEC port={} phase=configure-endpoint error={}",
                        port, erreur,
                    );
                }
            }
        }
    }

    if hid_count == 0 {
        if (device.slot_id as usize) < controller.devices.len() {
            controller.devices[device.slot_id as usize] = Some(device);
        }
        return;
    }

    match configure_hids(controller, &mut device, &mut descriptors[..hid_count]) {
        Ok(installed) => {
            crate::serial_println!(
                "BOUCHAUD_HID_CONFIG_OK port={} endpoints={}",
                port,
                installed
            );
        }
        Err(error) => {
            crate::serial_println!(
                "BOUCHAUD_HID_CONFIG_FAIL port={} phase=configure-endpoint error={}",
                port,
                error
            );
        }
    }
    if (device.slot_id as usize) < controller.devices.len() {
        controller.devices[device.slot_id as usize] = Some(device);
    }
}

/// Retire du systeme tout ce qui appartenait a un slot.
///
/// # Ce qu'un debranchement laisse derriere lui, si on ne fait rien
///
/// Le point de terminaison reste ARME : chaque tour de scrutation sonne sa
/// cloche pour un peripherique qui n'est plus la, le controleur repond par une
/// erreur, et le journal se remplit. Pire, le slot n'est jamais rendu -- une
/// dizaine de branchements-debranchements et il n'y a plus de slot libre,
/// donc plus rien de branchable.
///
/// Le tableau est COMPACTE plutot que troue : `hid_count` borne les boucles,
/// et une entree morte au milieu ferait sauter celles d'apres.
fn retire_slot(controller: &mut Controller, slot: u8) -> usize {
    let mut retires = 0usize;
    let mut index = 0usize;
    while index < controller.hid_count {
        if controller.hids[index].slot_id == slot {
            // L'anneau et le tampon de cette extremite, rendus eux aussi : ils
            // sont alloues PAR EXTREMITE, pas par peripherique, et un clavier
            // composite en a deux.
            memory::free_dma(controller.hids[index].ring.phys, RING_BYTES);
            memory::free_dma(controller.hids[index].buffer_phys, 4096);
            // La source de boutons AUSSI se rend. Un debranchement bouton
            // enfonce laisserait sinon le bureau croire le bouton maintenu
            // pour toujours, et la source ne reviendrait jamais au tableau.
            crate::drivers::mouse::libere_source_souris(
                controller.hids[index].source_souris,
            );
            let dernier = controller.hid_count - 1;
            controller.hids[index] = controller.hids[dernier];
            controller.hids[dernier] = EMPTY_HID_ENDPOINT;
            controller.hid_count = dernier;
            retires += 1;
            continue;
        }
        index += 1;
    }
    // LE STOCKAGE AUSSI SE DEBRANCHE.
    //
    // Un support laisse derriere lui deux anneaux, un tampon de donnees de
    // soixante-quatre kibioctets et un tampon d'enveloppe. Ne pas les rendre
    // epuise l'arene DMA en quelques branchements -- et l'entree survivante
    // dans la table ferait ecrire la prochaine lecture dans les anneaux d'un
    // peripherique qui n'est plus la.
    let mut index = 0usize;
    while index < controller.stockage_count {
        if controller.stockages[index].slot_id == slot {
            let st = controller.stockages[index];
            memory::free_dma(st.entree.ring.phys, RING_BYTES);
            memory::free_dma(st.sortie.ring.phys, RING_BYTES);
            memory::free_dma(st.donnees_phys, TAMPON_STOCKAGE);
            memory::free_dma(st.enveloppe_phys, 4096);
            let dernier = controller.stockage_count - 1;
            controller.stockages[index] = controller.stockages[dernier];
            controller.stockages[dernier] = STOCKAGE_VIDE;
            controller.stockage_count = dernier;
            retires += 1;
            crate::serial_println!("BOUCHAUD_USB_STOCKAGE_RETIRE slot={}", slot);
            continue;
        }
        index += 1;
    }

    let mut noeud = 0usize;
    while noeud < controller.compte_noeuds {
        let appartient = controller.arbre[noeud]
            .map(|n| n.slot == slot)
            .unwrap_or(false);
        if appartient {
            let dernier = controller.compte_noeuds - 1;
            controller.arbre[noeud] = controller.arbre[dernier];
            controller.arbre[dernier] = None;
            controller.compte_noeuds = dernier;
            continue;
        }
        noeud += 1;
    }
    // Le recorder USB possede ses propres rings Bulk.
    retire_blackbox_storage(controller, slot);
    // Le slot est rendu au controleur AVANT la memoire : il ne doit plus
    // pouvoir ecrire dans des contextes qu'on vient de rendre a l'arene.
    disable_slot(controller, slot);
    if (slot as usize) < controller.devices.len() {
        if let Some(device) = controller.devices[slot as usize].take() {
            libere_device(&device);
        }
    }
    retires
}

/// Tout ce qui pend au port racine `port`, y compris derriere un concentrateur.
///
/// Debrancher un concentrateur emporte ce qui etait branche dessus, et le
/// controleur ne l'annonce PAS : il ne signale que le port racine. Ne retirer
/// que le concentrateur laisserait ses enfants armes sur un chemin qui n'existe
/// plus.
fn retire_sous_arbre(controller: &mut Controller, port: u8) -> usize {
    let mut slots = [0u8; NOEUDS_MAX];
    let mut combien = 0usize;
    for noeud in controller.arbre[..controller.compte_noeuds].iter().flatten() {
        if noeud.port_racine == port && combien < slots.len() {
            slots[combien] = noeud.slot;
            combien += 1;
        }
    }
    let mut retires = 0usize;
    for slot in slots[..combien].iter().copied() {
        retires += retire_slot(controller, slot);
        DEBRANCHEMENTS.fetch_add(1, Ordering::Relaxed);
    }
    if combien != 0 {
        crate::serial_println!(
            "BOUCHAUD_USB_DEBRANCHEMENT port={} peripheriques={} extremites_hid={}",
            port, combien, retires,
        );
    }
    retires
}

/// Traite UN port dont l'etat a change depuis le demarrage.
///
/// # Pourquoi cela ne peut pas se faire dans la boucle des evenements
///
/// Enumerer emet des transferts de controle, qui attendent leurs propres
/// evenements. Le faire depuis la boucle qui draine l'anneau reentrerait
/// dedans. Les ports sont donc NOTES pendant le drainage et traites apres.
fn traite_port_change(controller: &mut Controller, port_index: usize) {
    let off = 0x400 + port_index * 0x10;
    let portsc = unsafe { r32(controller.op, off) };
    let port = (port_index + 1) as u8;

    // Les bits de changement s'effacent en y ecrivant un, et les autres champs
    // ne doivent pas etre rejoues -- `neutral_port_state` s'en charge.
    if portsc & PORTSC_CHANGE_BITS != 0 {
        unsafe {
            w32(
                controller.op,
                off,
                neutral_port_state(portsc) | (portsc & PORTSC_PP) | (portsc & PORTSC_CHANGE_BITS),
            )
        };
    }

    if portsc & PORTSC_CCS == 0 {
        retire_sous_arbre(controller, port);
        return;
    }

    // Deja enumere : un changement sur un port occupe qu'on connait
    // (surintensite, reprise) ne demande pas de reenumeration.
    let deja = controller
        .arbre[..controller.compte_noeuds]
        .iter()
        .flatten()
        .any(|n| n.port_racine == port);
    if deja {
        return;
    }

    if portsc & PORTSC_PED == 0 && !reset_root_port(controller, port_index) {
        crate::serial_println!(
            "BOUCHAUD_USB_BRANCHEMENT_ECHEC port={} phase=reset portsc={:#010x}",
            port, portsc,
        );
        return;
    }

    BRANCHEMENTS.fetch_add(1, Ordering::Relaxed);
    crate::serial_println!("BOUCHAUD_USB_BRANCHEMENT port={}", port);
    let mut file = FileChemins::neuve();
    enumerate_port(controller, port_index, &mut file);
    while let Some(chemin) = file.tire() {
        enumerate_device(controller, chemin, &mut file);
    }
    // Les extremites nouvellement installees ne sont armees qu'ICI, une fois
    // toute l'enumeration finie : armer plus tot ferait arriver des rapports
    // au milieu des transferts de controle qui restent a faire.
    arm_all_hids(controller);
    crate::serial_println!(
        "BOUCHAUD_USB_BRANCHEMENT_OK port={} claviers={} souris={}",
        port,
        controller.hids[..controller.hid_count].iter().filter(|e| e.kind == 1).count(),
        controller.hids[..controller.hid_count].iter().filter(|e| e.kind == 2).count(),
    );
}

/// Scrute les ports a la recherche d'un changement que l'anneau n'a pas dit.
///
/// # Pourquoi ne pas se fier aux seuls evenements
///
/// Les interruptions sont desactivees : l'anneau n'est lu que lorsqu'on le
/// draine, et un evenement de changement de port peut avoir ete consomme par
/// un `wait_event` d'une version anterieure, ou ne jamais avoir ete poste par
/// un controleur avare. Relire les bits de changement coute une lecture par
/// port et ferme ce trou-la.
fn releve_ports_changes(controller: &mut Controller) {
    let ports = controller.max_ports.min(MAX_PORTS_PER_CONTROLLER as u8) as usize;
    for port_index in 0..ports {
        let portsc = unsafe { r32(controller.op, 0x400 + port_index * 0x10) };
        if portsc & PORTSC_CHANGE_BITS != 0 {
            controller.ports_a_traiter |= 1u32 << port_index;
        }
    }
}

fn neutral_port_state(value: u32) -> u32 {
    // Do not replay RW1C/RW1S fields while changing power/reset bits.
    value & !(PORTSC_PED | PORTSC_PR | PORTSC_WPR | PORTSC_CHANGE_BITS)
}

fn wait_port_enabled(controller: &Controller, port_index: usize, timeout_ms: u64) -> bool {
    let off = 0x400 + port_index * 0x10;
    let start = crate::kernel::timer::monotonic_ns();
    let deadline = start.saturating_add(timeout_ms.saturating_mul(1_000_000));
    loop {
        let value = unsafe { r32(controller.op, off) };
        if value & PORTSC_CCS != 0 && value & PORTSC_PED != 0 { return true; }
        if value & PORTSC_CCS == 0 { return false; }
        if start != 0 && crate::kernel::timer::monotonic_ns() >= deadline { return false; }
        core::hint::spin_loop();
    }
}

fn reset_root_port(controller: &mut Controller, port_index: usize) -> bool {
    let off = 0x400 + port_index * 0x10;
    let before = unsafe { r32(controller.op, off) };
    if before & PORTSC_CCS == 0 { return false; }
    if before & PORTSC_PED != 0 { return true; }

    // USB2 ports use PR; USB3 ports use WPR. Choose from the Supported Protocol
    // capability instead of assuming a particular numeric Speed ID mapping.
    let major = controller.port_major[port_index];
    let reset_bit = if major >= 3 { PORTSC_WPR } else { PORTSC_PR };
    let power = before & PORTSC_PP;
    unsafe { w32(controller.op, off, neutral_port_state(before) | power | reset_bit) };
    if !wait_until(|| unsafe { r32(controller.op, off) } & reset_bit == 0) {
        crate::serial_println!(
            "BOUCHAUD_XHCI_PORT_RESET_FAIL port={} protocol={} reason=reset-bit-timeout before={:#010x}",
            port_index + 1, major, before
        );
        return false;
    }
    if wait_port_enabled(controller, port_index, 250) { return true; }
    let after = unsafe { r32(controller.op, off) };
    crate::serial_println!(
        "BOUCHAUD_XHCI_PORT_RESET_FAIL port={} protocol={} reason=ped-timeout after={:#010x}",
        port_index + 1, major, after
    );
    false
}

fn prepare_root_ports(controller: &mut Controller) {
    if controller.ppc {
        for port in 0..controller.max_ports.min(MAX_PORTS_PER_CONTROLLER as u8) as usize {
            let off = 0x400 + port * 0x10;
            let value = unsafe { r32(controller.op, off) };
            if value & PORTSC_PP == 0 {
                unsafe { w32(controller.op, off, neutral_port_state(value) | PORTSC_PP) };
            }
        }
        wait_ms(50);
    }
    // HCRST destroyed the firmware's host state; let attached devices settle.
    wait_ms(100);

    let mut connected = 0usize;
    let mut enabled = 0usize;
    for port in 0..controller.max_ports.min(MAX_PORTS_PER_CONTROLLER as u8) as usize {
        let off = 0x400 + port * 0x10;
        let before = unsafe { r32(controller.op, off) };
        if before & PORTSC_CCS == 0 {
            crate::serial_println!(
                "BOUCHAUD_XHCI_ROOT_PORT port={} protocol={} before={:#010x} connected=0",
                port + 1, controller.port_major[port], before
            );
            continue;
        }
        connected += 1;
        wait_ms(20);
        let reset_ok = reset_root_port(controller, port);
        let mut after = unsafe { r32(controller.op, off) };
        if reset_ok && after & PORTSC_PED != 0 { enabled += 1; }
        if after & PORTSC_CHANGE_BITS != 0 {
            unsafe {
                w32(controller.op, off,
                    neutral_port_state(after) | (after & PORTSC_PP) | (after & PORTSC_CHANGE_BITS));
            }
            after = unsafe { r32(controller.op, off) };
        }
        let speed = ((after >> PORTSC_SPEED_SHIFT) & 0x0f) as u8;
        crate::serial_println!(
            "BOUCHAUD_XHCI_ROOT_PORT port={} protocol={} connected=1 enabled={} reset_ok={} speed_id={} before={:#010x} after={:#010x}",
            port + 1, controller.port_major[port], (after & PORTSC_PED != 0) as u8,
            reset_ok as u8, speed, before, after
        );
    }
    controller.connected_ports = connected;
    controller.enabled_ports = enabled;
}

fn protocol_info(
    base: usize,
    hccparams1: u32,
    max_ports: u8,
) -> ([u8; MAX_PORTS_PER_CONTROLLER], [u8; MAX_PORTS_PER_CONTROLLER]) {
    let mut slot_types = [0u8; MAX_PORTS_PER_CONTROLLER];
    let mut major = [0u8; MAX_PORTS_PER_CONTROLLER];
    let mut off = (((hccparams1 >> 16) & 0xffff) as usize) * 4;
    for _ in 0..64 {
        if off == 0 { break; }
        let header = unsafe { r32(base, off) };
        let cap_id = (header & 0xff) as u8;
        let next = ((header >> 8) & 0xff) as usize;
        if cap_id == 2 {
            let protocol_major = ((header >> 24) & 0xff) as u8;
            let ports = unsafe { r32(base, off + 8) };
            let start = (ports & 0xff) as usize;
            let count = ((ports >> 8) & 0xff) as usize;
            let slot_type = (unsafe { r32(base, off + 12) } & 0x1f) as u8;
            for port in start..start.saturating_add(count) {
                if port != 0 && port <= max_ports as usize && port <= slot_types.len() {
                    slot_types[port - 1] = slot_type;
                    major[port - 1] = protocol_major;
                }
            }
        }
        if next == 0 { break; }
        off = off.saturating_add(next * 4);
    }
    (slot_types, major)
}

fn init_controller(dev: PciDevice) -> Result<(Controller, usize), &'static str> {
    let bar0 = pci::bar_decode(&dev, 0).adresse();
    if bar0 == 0 {
        return Err("bar0-absent");
    }
    pci::enable_bus_master(&dev);
    let base = memory::phys_to_virt(bar0) as usize;

    unsafe {
        let cap_len = r8(base, 0x00) as usize;
        let hci_version = r16(base, 0x02);
        let hcs1 = r32(base, 0x04);
        let hcs2 = r32(base, 0x08);
        let hccparams1 = r32(base, 0x10);
        let dboff = (r32(base, 0x14) & !0x3) as usize;
        let rtsoff = (r32(base, 0x18) & !0x1f) as usize;
        if !(0x20..=0x80).contains(&cap_len) {
            return Err("caplength-invalide");
        }
        let max_slots = (hcs1 & 0xff) as u8;
        let max_ports = ((hcs1 >> 24) & 0xff) as u8;
        if max_slots == 0 || max_ports == 0 {
            return Err("controller-capabilities-empty");
        }
        let scratchpads = max_scratchpads(hcs2);
        if scratchpads > 1024 {
            return Err("scratchpad-count-invalid");
        }
        let context_size = if hccparams1 & HCC_CSZ != 0 { 64 } else { 32 };
        let ppc = hccparams1 & HCC_PPC != 0;
        let (slot_types, port_major) = protocol_info(base, hccparams1, max_ports);
        let op = base + cap_len;

        let mut connected_before = 0usize;
        for port in 0..max_ports.min(MAX_PORTS_PER_CONTROLLER as u8) as usize {
            let value = r32(op, 0x400 + port * 0x10);
            if value & PORTSC_CCS != 0 {
                connected_before += 1;
            }
        }
        crate::serial_println!(
            "BOUCHAUD_XHCI_CONTROLLER_DISCOVERED bdf={:02x}:{:02x}.{} vid={:04x} did={:04x} bar0={:#x} version={:#06x} slots={} ports={} connected_before={} csz={} ppc={} scratchpads={}",
            dev.bus,
            dev.slot,
            dev.func,
            dev.vendor,
            dev.device,
            bar0,
            hci_version,
            max_slots,
            max_ports,
            connected_before,
            context_size,
            ppc as u8,
            scratchpads
        );

        if !legacy_handoff(base, hccparams1) {
            return Err("bios-ownership-timeout");
        }

        let mut cmd = r32(op, 0x00);
        cmd &= !USBCMD_RUN;
        w32(op, 0x00, cmd);
        if !wait_until(|| r32(op, 0x04) & USBSTS_HCH != 0) {
            return Err("halt-timeout");
        }

        w32(op, 0x00, r32(op, 0x00) | USBCMD_HCRST);
        if !wait_until(|| {
            let c = r32(op, 0x00);
            let s = r32(op, 0x04);
            c & USBCMD_HCRST == 0 && s & USBSTS_CNR == 0
        }) {
            return Err("reset-timeout");
        }
        if r32(op, 0x08) & 1 == 0 {
            return Err("page-4k-non-supportee");
        }

        let Some((dcbaa_phys, dcbaa_virt)) = alloc_zeroed(256 * 8) else {
            return Err("dma-dcbaa");
        };
        if scratchpads != 0 {
            let Some((array_phys, array_virt)) = alloc_zeroed(scratchpads * 8) else {
                return Err("dma-scratch-array");
            };
            for index in 0..scratchpads {
                let Some((page_phys, _)) = alloc_zeroed(4096) else {
                    return Err("dma-scratch-page");
                };
                write_volatile((array_virt + index * 8) as *mut u64, page_phys);
            }
            write_volatile(dcbaa_virt as *mut u64, array_phys);
        }

        let command = alloc_producer_ring().ok_or("dma-command-ring")?;
        let (event_phys, event_virt) = alloc_zeroed(RING_BYTES).ok_or("dma-event-ring")?;
        let events = EventRing {
            phys: event_phys,
            virt: event_virt,
            index: 0,
            cycle: 1,
        };
        let (erst_phys, erst_virt) = alloc_zeroed(64).ok_or("dma-erst")?;
        write_volatile(erst_virt as *mut u64, event_phys);
        write_volatile((erst_virt + 8) as *mut u32, TRBS_PER_RING as u32);
        write_volatile((erst_virt + 12) as *mut u32, 0);

        w64(op, 0x30, dcbaa_phys);
        w64(op, 0x18, command.phys | 1);
        w32(op, 0x38, max_slots as u32);

        let intr0 = base + rtsoff + 0x20;
        w32(intr0, 0x00, 1); // clear pending, keep IE off: V3 polls
        w32(intr0, 0x08, 1);
        w64(intr0, 0x10, erst_phys);
        w64(intr0, 0x18, event_phys | (1 << 3));

        let mut controller = Controller {
            dev,
            base,
            op,
            doorbells: base + dboff,
            intr0,
            hci_version,
            max_slots,
            max_ports,
            context_size,
            ppc,
            slot_types,
            port_major,
            command,
            events,
            dcbaa_phys,
            dcbaa_virt,
            blackbox_storage: None,
            hids: [EMPTY_HID_ENDPOINT; MAX_HID_ENDPOINTS_PER_CONTROLLER],
            differes: [Trb { parameter: 0, status: 0, control: 0 }; EVENEMENTS_DIFFERES],
            differes_len: 0,
            ports_a_traiter: 0,
            arbre: [None; NOEUDS_MAX],
            compte_noeuds: 0,
            hid_count: 0,
            devices: [None; MAX_RUNTIME_DEVICES],
            stockages: [STOCKAGE_VIDE; MAX_STOCKAGE_PAR_CONTROLEUR],
            stockage_count: 0,
            connected_ports: 0,
            enabled_ports: 0,
            usb_devices: 0,
        };
        let _ = controller.base;
        let _ = controller.dcbaa_phys;

        w32(op, 0x00, r32(op, 0x00) | USBCMD_RUN);
        if !wait_until(|| r32(op, 0x04) & USBSTS_HCH == 0) {
            return Err("run-timeout");
        }
        prepare_root_ports(&mut controller);

        crate::serial_println!(
            "BOUCHAUD_XHCI_CONTROLLER_ACTIVE_OK bdf={:02x}:{:02x}.{} connected={} enabled={}",
            controller.dev.bus,
            controller.dev.slot,
            controller.dev.func,
            controller.connected_ports,
            controller.enabled_ports
        );
        Ok((controller, scratchpads))
    }
}



/// Traduit un rapport de clavier et pousse les codes PS/2.
///
/// La difference entre deux rapports, la table des touches et l'ordre des
/// evenements vivent dans `hid::decodage`, qui ne touche rien et que
/// `tools/platform/test_hid.rs` met a l'epreuve touche par touche. Ici il ne
/// reste que la remise a la pile d'entree.
fn process_keyboard_report(endpoint: &mut HidEndpoint, data: &[u8]) -> bool {
    let mut sortie = [hid::Evenement { code: 0, etendu: false, appui: false };
        hid::EVENEMENTS_MAX];
    let Some(n) = hid::evenements_clavier(
        &mut endpoint.clavier,
        endpoint.report_id,
        data,
        &mut sortie,
    ) else {
        return false;
    };
    for evenement in sortie.iter().take(n) {
        push_ps2(evenement.code, evenement.etendu, evenement.appui);
    }
    true
}

fn process_mouse_report(endpoint: &HidEndpoint, data: &[u8]) -> bool {
    let Some(souris) = hid::decode_souris(endpoint.report_id, data) else {
        return false;
    };
    crate::drivers::mouse::inject_usb_report(
        endpoint.source_souris, souris.boutons, souris.dx, souris.dy, souris.roue,
    );
    true
}

fn process_hid_event(controller: &mut Controller, event: Trb) {
    HID_TRANSFER_EVENTS.fetch_add(1, Ordering::Relaxed);
    let slot = event_slot(event);
    let dci = event_dci(event);
    // Le compteur PAR point de terminaison : c'est lui qui decide du repli,
    // et le compteur global ne sert plus qu'au diagnostic.
    for index in 0..controller.hid_count {
        let ep = &mut controller.hids[index];
        if ep.active && ep.slot_id == slot && ep.dci == dci {
            ep.evenements = ep.evenements.saturating_add(1);
            break;
        }
    }
    let cc = completion_code(event.status);
    let Some(index) = (0..controller.hid_count).find(|&index| {
        let ep = &controller.hids[index];
        ep.active && ep.slot_id == slot && ep.dci == dci
    }) else {
        crate::serial_println!(
            "BOUCHAUD_HID_EVENT_UNMATCHED slot={} dci={} cc={} status={:#010x}",
            slot, dci, cc, event.status,
        );
        return;
    };

    let buffer_len = controller.hids[index].buffer_len;
    let residual = (event.status & 0x00ff_ffff) as usize;
    let actual = buffer_len.saturating_sub(residual.min(buffer_len));
    if (cc == CC_SUCCESS || cc == CC_SHORT_PACKET) && actual != 0 {
        let data = unsafe {
            core::slice::from_raw_parts(
                controller.hids[index].buffer_virt as *const u8,
                actual,
            )
        };
        HID_REPORTS.fetch_add(1, Ordering::Relaxed);
        let accepted = match controller.hids[index].kind {
            1 => {
                let ok = process_keyboard_report(&mut controller.hids[index], data);
                if ok {
                    let previous = HID_KEYBOARD_REPORTS.fetch_add(1, Ordering::Relaxed);
                    if previous == 0 {
                        crate::serial_println!("BOUCHAUD_HID_KEYBOARD_INPUT_GREEN slot={} dci={}", slot, dci);
                    }
                }
                ok
            }
            2 => {
                let ok = process_mouse_report(&controller.hids[index], data);
                if ok {
                    let previous = HID_MOUSE_REPORTS.fetch_add(1, Ordering::Relaxed);
                    if previous == 0 {
                        crate::serial_println!("BOUCHAUD_HID_MOUSE_INPUT_GREEN slot={} dci={}", slot, dci);
                    }
                }
                ok
            }
            _ => false,
        };
        if HID_REPORTS.load(Ordering::Relaxed) <= 16 {
            crate::serial_println!(
                "BOUCHAUD_HID_REPORT slot={} dci={} kind={} cc={} actual={} accepted={} b0={:#04x} b1={:#04x} b2={:#04x} b3={:#04x}",
                slot, dci, controller.hids[index].kind, cc, actual, accepted as u8,
                data.get(0).copied().unwrap_or(0), data.get(1).copied().unwrap_or(0),
                data.get(2).copied().unwrap_or(0), data.get(3).copied().unwrap_or(0),
            );
        }
    } else if cc != CC_SUCCESS && cc != CC_SHORT_PACKET {
        let n = HID_TRANSFER_ERRORS.fetch_add(1, Ordering::Relaxed) + 1;
        if n <= 16 {
            crate::serial_println!(
                "BOUCHAUD_HID_TRANSFER_FAIL slot={} dci={} cc={} residual={} status={:#010x}",
                slot, dci, cc, residual, event.status,
            );
        }
    }

    // A completed TD is consumed whatever its status. Keep one receive TD
    // outstanding so the input path survives shorts and transient transaction
    // errors without requiring an interrupt-driven producer yet.
    arm_hid_endpoint(controller, index);
}

fn control_get_report(controller: &mut Controller, endpoint_index: usize) -> bool {
    if endpoint_index >= controller.hid_count { return false; }
    let endpoint = controller.hids[endpoint_index];
    if !endpoint.active || endpoint.slot_id as usize >= controller.devices.len() { return false; }
    let Some(mut device) = controller.devices[endpoint.slot_id as usize] else { return false; };

    let report_id = endpoint.report_id;
    let length = endpoint.buffer_len.clamp(3, 64);
    let setup = setup_packet(
        0xa1, // device-to-host, class, interface
        0x01, // GET_REPORT
        ((1u16) << 8) | report_id as u16, // Input report + Report ID
        endpoint.interface as u16,
        length as u16,
    );
    HID_CONTROL_POLLS.fetch_add(1, Ordering::Relaxed);
    // LE BUDGET COURT, ET C'EST TOUT LE CORRECTIF.
    //
    // Ce transfert est sur le chemin de scrutation : il s'execute un tour
    // sur deux, pour chaque point de terminaison muet. Lui laisser la
    // patience de l'enumeration revenait a payer une echeance de 330 ms
    // par tour, et c'est ce qui rendait le TRIGKEY inutilisable.
    let result = control_transfer(
        controller, &mut device, setup, length, true, BUDGET_SCRUTATION_NS,
    );
    controller.devices[endpoint.slot_id as usize] = Some(device);
    let received = match result {
        Ok(received) if received != 0 => received.min(length),
        _ => {
            HID_CONTROL_FAILS.fetch_add(1, Ordering::Relaxed);
            return false;
        }
    };

    let data = unsafe {
        core::slice::from_raw_parts(device.control_virt as *const u8, received)
    };
    let accepted = match controller.hids[endpoint_index].kind {
        1 => process_keyboard_report(&mut controller.hids[endpoint_index], data),
        2 => process_mouse_report(&controller.hids[endpoint_index], data),
        _ => false,
    };
    if accepted {
        let previous = HID_CONTROL_REPORTS.fetch_add(1, Ordering::Relaxed);
        match controller.hids[endpoint_index].kind {
            1 => {
                HID_KEYBOARD_REPORTS.fetch_add(1, Ordering::Relaxed);
                if previous == 0 {
                    crate::serial_println!("BOUCHAUD_HID_CONTROL_FALLBACK_GREEN kind=keyboard");
                }
            }
            2 => {
                HID_MOUSE_REPORTS.fetch_add(1, Ordering::Relaxed);
                if previous == 0 {
                    crate::serial_println!("BOUCHAUD_HID_CONTROL_FALLBACK_GREEN kind=mouse");
                }
            }
            _ => {}
        }
    }
    accepted
}

/// Sert AU PLUS UN point de terminaison muet, a partir de `depart`.
///
/// Rend l'indice servi, pour que le tour suivant reparte du point d'apres :
/// sans ce curseur, le premier point muet du tableau serait le seul jamais
/// interroge.
///
/// # Pourquoi un seul, et pourquoi hors de la scrutation
///
/// `control_get_report` est un transfert de controle SYNCHRONE. Le releve
/// physique du 12 septembre le chiffre : la boucle de scrutation annoncait
/// `polls=11065` en soixante-douze secondes, soit CENT SOIXANTE-SIX tours par
/// seconde la ou son `sleep_ticks(1)` en vise mille. Les cinq sixiemes du
/// temps partaient dans les `GET_REPORT` de cinq points muets, enchaines dans
/// le meme tour et le verrou du pilote tenu du debut a la fin.
///
/// La souris etait donc echantillonnee a quatre-vingts hertz -- douze
/// millisecondes de grain -- et c'est cela que l'utilisateur decrit comme
/// « elle met trop de temps a se deplacer ».
///
/// Le repli vit desormais sur son propre fil (`fil_repli_ep0`), un point par
/// tour, le verrou pris et rendu a chaque fois. Le drainage garde ses mille
/// tours par seconde, et les rapports qui arrivent PENDANT un `GET_REPORT`
/// sont traites sur place (`traite_differes`).
fn repli_ep0_un_point(controller: &mut Controller, depart: usize) -> Option<usize> {
    if controller.hid_count == 0 {
        return None;
    }
    // Interrupt-IN remains the preferred path. If the physical controller has
    // produced no endpoint Transfer Event after initial arming, use the HID
    // class GET_REPORT request over EP0 as a compatibility bridge. HID 1.11
    // requires GET_REPORT support on HID devices; this path is deliberately a
    // fallback, not the long-term periodic transport.
    // LA DECISION EST PAR PERIPHERIQUE, ET C'EST TOUT LE CORRECTIF.
    //
    // La condition portait sur un compteur GLOBAL : le repli s'eteignait des
    // qu'UN periphérique produisait un evenement de transfert. Sur la machine
    // de reference, la souris en a produit 556 et le clavier 6 -- la souris
    // coupait le repli, et le clavier, muet en Interrupt-IN, se retrouvait
    // sans aucun transport. Le journal physique le dit sans ambiguite :
    // `BOUCHAUD_HID_CONTROL_FALLBACK_GREEN kind=keyboard` une seule fois, puis
    // 556 rapports de souris et plus un seul du clavier.
    //
    // Le repli reste ce qu'il doit etre : un pont de compatibilite pour les
    // peripheriques qui ne repondent pas en Interrupt-IN, et rien d'autre. Un
    // peripherique qui repond n'y passe jamais. La difference est qu'un
    // peripherique muet n'est plus prive de secours parce que son voisin,
    // lui, va bien.
    let maintenant = crate::kernel::timer::monotonic_ns();
    let total = controller.hid_count;
    for decalage in 0..total {
        let index = (depart + decalage) % total;
        if controller.hids[index].evenements != 0 {
            continue;
        }
        // UN POINT QU'ON VIENT D'ARMER N'EST PAS UN POINT MUET.
        //
        // Il lui faut le temps de produire son premier evenement. Sans ce
        // delai, le pont EP0 se declencherait sur tout peripherique a
        // l'instant meme de son branchement, et un peripherique parfaitement
        // sain finirait sur le transport lent.
        if maintenant
            < controller.hids[index]
                .arme_depuis_ns
                .saturating_add(GRACE_INTERRUPT_NS)
        {
            continue;
        }
        // UN POINT MUET EST EN QUARANTAINE, PAS INTERROGE MILLE FOIS.
        if maintenant < controller.hids[index].repli_muet_jusqu_a_ns {
            continue;
        }
        // UNE ERREUR TRANSITOIRE N'EST PAS UN PERIPHERIQUE MUET.
        //
        // On distingue les deux avant de decider de la peine : un transfert qui
        // n'est pas passe se retente dans vingt millisecondes, un peripherique
        // qui ne repond pas attend une seconde.
        let avant = HID_CONTROL_FAILS.load(Ordering::Relaxed);
        let repond = control_get_report(controller, index);
        let transitoire = !repond
            && HID_CONTROL_FAILS.load(Ordering::Relaxed) != avant
            && DERNIER_CODE_CONTROLE.load(Ordering::Relaxed) == CC_ERREUR_TRANSACTION as usize;
        if repond {
            // Le point s'est remis a repondre : il sort de quarantaine, et sa
            // prochaine rechute sera annoncee de nouveau.
            controller.hids[index].echecs_repli = 0;
            controller.hids[index].quarantaine_annoncee = false;
        } else if transitoire {
            let ep = &mut controller.hids[index];
            ep.repli_muet_jusqu_a_ns = maintenant.saturating_add(QUARANTAINE_TRANSITOIRE_NS);
        } else {
            {
                let ep = &mut controller.hids[index];
                ep.echecs_repli = ep.echecs_repli.saturating_add(1);
                if ep.echecs_repli >= ECHECS_AVANT_QUARANTAINE {
                    ep.repli_muet_jusqu_a_ns =
                        maintenant.saturating_add(QUARANTAINE_REPLI_NS);
                    // UNE FOIS, A L'ENTREE EN QUARANTAINE, ET PAS A CHAQUE
                    // REPRISE.
                    //
                    // La quarantaine est RETENTEE chaque seconde -- c'est ce
                    // qui fait d'elle un pont et non une condamnation. Mais
                    // une interface vendeur ne se met jamais a repondre : la
                    // reprise echoue, la quarantaine se repose, et la ligne
                    // se reimprimait. Deux lignes par seconde et pour
                    // toujours, sur un journal serie dont le budget est
                    // borne : le bruit finissait par chasser tout le reste.
                    //
                    // L'ETAT COURANT se lit ailleurs, et sans bruit :
                    // `replis_en_quarantaine` sort dans `[USB-HID-V3]`, a
                    // cadence de diagnostic.
                    if !ep.quarantaine_annoncee {
                        ep.quarantaine_annoncee = true;
                        let (slot, dci, echecs) = (ep.slot_id, ep.dci, ep.echecs_repli);
                        REPLIS_EN_QUARANTAINE.fetch_add(1, Ordering::Relaxed);
                        crate::serial_println!(
                            "BOUCHAUD_HID_REPLI_QUARANTAINE slot={} dci={} echecs={} \
reprise_dans_ms={}",
                            slot,
                            dci,
                            echecs,
                            QUARANTAINE_REPLI_NS / 1_000_000,
                        );
                    }
                }
            }
        }
        // UN SEUL POINT PAR TOUR, ET C'EST TOUT L'INTERET DE CETTE BOUCLE.
        //
        // `control_get_report` est un transfert de controle SYNCHRONE : trois
        // etapes et une attente, le verrou du pilote tenu du debut a la fin.
        // En servir cinq d'affilee, c'est tenir ce verrou plusieurs
        // millisecondes -- et c'est ce qui ramenait la scrutation de mille
        // tours par seconde a cent soixante-six sur la machine de reference.
        return Some(index);
    }
    None
}

/// Delai laisse a un point fraichement arme avant de lui proposer le pont EP0.
///
/// Deux cents millisecondes : le temps qu'un peripherique produise son premier
/// rapport en Interrupt-IN. En dessous, un peripherique sain branche a chaud
/// serait bascule sur le transport lent avant d'avoir eu sa chance.
const GRACE_INTERRUPT_NS: u64 = 200_000_000;

/// Echecs consecutifs du repli EP0 avant sa mise en quarantaine.
const ECHECS_AVANT_QUARANTAINE: u8 = 2;
/// Duree de la quarantaine du repli EP0 d'un point de terminaison muet.
///
/// Une seconde : assez pour que le cout devienne negligeable devant les mille
/// scrutations de cette seconde, assez court pour qu'un peripherique qui se
/// reveille soit repris sans que l'utilisateur le remarque.
const QUARANTAINE_REPLI_NS: u64 = 1_000_000_000;
/// Quarantaine apres une erreur TRANSITOIRE.
///
/// # Pourquoi deux durees et pas une
///
/// La quarantaine existe pour un peripherique qui ne repondra JAMAIS -- une
/// interface vendeur qui n'implemente pas `GET_REPORT`. Une seconde y est le
/// bon prix : le cout devient negligeable, et la reprise reste imperceptible.
///
/// Une erreur de transaction n'a rien a voir. Elle dit que CE transfert-la
/// n'est pas passe : un cable, un concentrateur, un peripherique qu'on vient
/// de rebrancher, ou un anneau qu'on n'a pas servi assez vite. Le releve
/// physique le montre a l'instant ou Ladybird demarre -- `cc=4` sur deux
/// points de terminaison qui fonctionnaient jusque-la.
///
/// Leur appliquer la quarantaine longue punit un peripherique sain pour un
/// incident passager, et c'est ce que l'utilisateur a senti apres avoir
/// rebranche sa souris : « lent, elle se teleporte ». Vingt millisecondes
/// suffisent a ne pas boucler dessus, et se reprennent sans qu'on le voie.
const QUARANTAINE_TRANSITOIRE_NS: u64 = 20_000_000;
/// Erreur de transaction USB : le transfert n'est pas passe, le peripherique
/// n'a rien refuse.
const CC_ERREUR_TRANSACTION: u8 = 4;
/// Points de terminaison mis en quarantaine, pour le diagnostic.
static REPLIS_EN_QUARANTAINE: AtomicUsize = AtomicUsize::new(0);

fn push_ps2(code: u8, extended: bool, pressed: bool) {
    if extended {
        crate::drivers::keyboard::push_scancode(0xe0);
    }
    crate::drivers::keyboard::push_scancode(if pressed { code } else { code | 0x80 });
}





// ---------------------------------------------------------------------------
// Le transport BULK, et le stockage de masse qui s'en sert
// ---------------------------------------------------------------------------
//
// # Ce qui manquait
//
// Le pilote ne savait armer qu'un seul genre de point de terminaison : INTERRUPT
// IN, celui des claviers et des souris. Un point BULK ne s'arme pas de la meme
// facon -- il n'a pas d'intervalle, il n'est pas ré-arme en boucle, et il va
// dans les DEUX sens --, et sans lui aucune cle USB n'est lisible : le
// peripherique est enumere, il apparait dans `lsusb`, et il ne sert a rien.
//
// # Pourquoi le meme anneau, et pourquoi deux
//
// Un point de terminaison a SON anneau de transfert. Le transport Bulk-Only
// envoie la commande par le point OUT et lit la reponse par le point IN : deux
// anneaux, deux cloches, deux evenements d'achevement. Les confondre revient a
// sonner pour un sens et attendre l'autre, ce qui n'echoue pas -- cela attend.
//
// # Le STALL n'est pas une panne
//
// Le transport BOT s'en sert pour dire « commande impossible ». Un
// `REQUEST SENSE` apres un STALL est NORMAL. Ce qui n'est pas normal, c'est de
// continuer sans avoir debloque le point : le peripherique refuse alors tout,
// et le pilote conclut que la cle est morte. La recuperation a donc deux
// moities, et les deux comptent -- `Reset Endpoint` du cote de l'hote,
// `CLEAR_FEATURE(ENDPOINT_HALT)` du cote du peripherique.

/// Supports de masse trouves.
static STOCKAGES_TROUVES: AtomicUsize = AtomicUsize::new(0);
/// Supports montes, c'est-a-dire dont la capacite est connue.
static STOCKAGES_PRETS: AtomicUsize = AtomicUsize::new(0);
static BULK_TRANSFERTS: AtomicUsize = AtomicUsize::new(0);
static BULK_OCTETS: AtomicU64 = AtomicU64::new(0);
static BULK_STALLS: AtomicUsize = AtomicUsize::new(0);
static BULK_RECUPERATIONS: AtomicUsize = AtomicUsize::new(0);
static BULK_ECHECS: AtomicUsize = AtomicUsize::new(0);
static BOT_REINITIALISATIONS: AtomicUsize = AtomicUsize::new(0);
/// Transferts ou le controleur et le peripherique ne comptent pas pareil.
///
/// Non nul veut dire qu'un des deux ment, et le pilote prend le plus petit des
/// deux. Sans ce compteur, ce desaccord serait invisible.
static BULK_DESACCORDS: AtomicUsize = AtomicUsize::new(0);

/// Prepare le contexte des deux points BULK et les fait configurer.
///
/// Le contexte d'entree est reconstruit depuis le contexte de SORTIE, comme
/// pour les HID : le contexte de slot courant porte deja l'adresse, la vitesse
/// et la chaine de route, et le reecrire a la main serait une occasion de plus
/// de se tromper.
fn configure_stockage(
    controller: &mut Controller,
    device: &mut Device,
    iface: &stockage::InterfaceStockage,
) -> Result<usize, &'static str> {
    if controller.stockage_count >= MAX_STOCKAGE_PAR_CONTROLEUR {
        return Err("table-stockage-pleine");
    }
    let dci_in = stockage::dci_pour(iface.entree);
    let dci_out = stockage::dci_pour(iface.sortie);
    if dci_in < 2 || dci_in >= 32 || dci_out < 2 || dci_out >= 32 {
        return Err("dci-hors-bornes");
    }

    let Some(ring_in) = alloc_producer_ring() else {
        return Err("dma-anneau-in");
    };
    let Some(ring_out) = alloc_producer_ring() else {
        memory::free_dma(ring_in.phys, RING_BYTES);
        return Err("dma-anneau-out");
    };
    let Some((donnees_phys, donnees_virt)) = alloc_zeroed(TAMPON_STOCKAGE) else {
        memory::free_dma(ring_in.phys, RING_BYTES);
        memory::free_dma(ring_out.phys, RING_BYTES);
        return Err("dma-tampon-donnees");
    };
    let Some((enveloppe_phys, enveloppe_virt)) = alloc_zeroed(4096) else {
        memory::free_dma(ring_in.phys, RING_BYTES);
        memory::free_dma(ring_out.phys, RING_BYTES);
        memory::free_dma(donnees_phys, TAMPON_STOCKAGE);
        return Err("dma-tampon-enveloppe");
    };

    clear_input(device);
    let slot_out = context_ptr(device.out_ctx_virt, controller.context_size, 0);
    let slot_in = context_ptr(device.in_ctx_virt, controller.context_size, 1);
    unsafe {
        copy_nonoverlapping(slot_out as *const u8, slot_in as *mut u8, controller.context_size);
    }

    // Type 6 : Bulk IN. Type 2 : Bulk OUT. L'intervalle ne veut rien dire pour
    // du bulk -- il est asynchrone --, et le mettre a autre chose que zero
    // ferait reserver de la bande passante periodique qui n'existe pas.
    fill_endpoint_context(
        context_ptr(device.in_ctx_virt, controller.context_size, dci_in as usize + 1),
        6,
        iface.entree_mps,
        0,
        &ring_in,
    );
    fill_endpoint_context(
        context_ptr(device.in_ctx_virt, controller.context_size, dci_out as usize + 1),
        2,
        iface.sortie_mps,
        0,
        &ring_out,
    );

    let add_flags = 1u32 | (1u32 << dci_in) | (1u32 << dci_out);
    let highest_dci = dci_in.max(dci_out).max(dci_deja_declare(slot_out));
    unsafe {
        write_volatile((device.in_ctx_virt + 4) as *mut u32, add_flags);
        let dw0 = ctx_r32(slot_in, 0);
        ctx_w32(slot_in, 0, (dw0 & !(0x1f << 27)) | ((highest_dci as u32) << 27));
    }

    if let Err(erreur) = command(
        controller,
        device.in_ctx_phys,
        CMD_CONFIGURE_ENDPOINT,
        device.slot_id,
    ) {
        memory::free_dma(ring_in.phys, RING_BYTES);
        memory::free_dma(ring_out.phys, RING_BYTES);
        memory::free_dma(donnees_phys, TAMPON_STOCKAGE);
        memory::free_dma(enveloppe_phys, 4096);
        return Err(erreur);
    }

    let index = controller.stockage_count;
    controller.stockages[index] = Stockage {
        actif: true,
        slot_id: device.slot_id,
        interface: iface.interface,
        entree: PointBulk { dci: dci_in, adresse: iface.entree, ring: ring_in },
        sortie: PointBulk { dci: dci_out, adresse: iface.sortie, ring: ring_out },
        donnees_phys,
        donnees_virt,
        enveloppe_phys,
        enveloppe_virt,
        etiquette: 1,
        taille_bloc: 0,
        blocs: 0,
        amovible: false,
    };
    controller.stockage_count = index + 1;
    STOCKAGES_TROUVES.fetch_add(1, Ordering::Relaxed);
    crate::serial_println!(
        "BOUCHAUD_USB_STOCKAGE_TROUVE slot={} if={} in={:#04x}/dci{} out={:#04x}/dci{} mps={}",
        device.slot_id, iface.interface, iface.entree, dci_in, iface.sortie, dci_out,
        iface.entree_mps,
    );
    Ok(index)
}

/// Un transfert BULK : un seul TRB Normal, une cloche, un evenement.
///
/// Rend le nombre d'octets REELLEMENT transferes. Un peripherique qui en donne
/// moins que demande le dit dans le residu de l'evenement, et le pilote qui
/// rend la taille demandee croit avoir un secteur entier alors qu'il en a la
/// moitie -- l'autre moitie etant ce que le tampon contenait avant.
fn bulk_transfert(
    controller: &mut Controller,
    index: usize,
    entree: bool,
    phys: u64,
    octets: usize,
) -> Result<usize, &'static str> {
    if octets == 0 || octets > TAMPON_STOCKAGE {
        return Err("bulk-longueur");
    }
    let (slot, dci) = {
        let st = &mut controller.stockages[index];
        let point = if entree { &mut st.entree } else { &mut st.sortie };
        ring_push(
            &mut point.ring,
            Trb {
                parameter: phys,
                status: octets as u32,
                // ISP autant que IOC : un paquet court est la reponse NORMALE
                // d'un peripherique qui a moins a donner, et sans ISP il
                // n'arriverait aucun evenement -- le transfert attendrait son
                // echeance pour rien.
                control: (TRB_NORMAL << 10) | TRB_IOC | TRB_ISP,
            },
        );
        (st.slot_id, point.dci)
    };
    fence(Ordering::SeqCst);
    ring_doorbell(controller, slot, dci);

    let event = wait_event(controller, EVT_TRANSFER, Some(slot), Some(dci))
        .ok_or("bulk-echeance")?;
    let cc = completion_code(event.status);
    let residu = (event.status & 0x00ff_ffff) as usize;
    if cc == CC_SUCCESS || cc == CC_SHORT_PACKET {
        BULK_TRANSFERTS.fetch_add(1, Ordering::Relaxed);
        let transferes = octets.saturating_sub(residu.min(octets));
        BULK_OCTETS.fetch_add(transferes as u64, Ordering::Relaxed);
        return Ok(transferes);
    }
    if cc == CC_STALL {
        BULK_STALLS.fetch_add(1, Ordering::Relaxed);
        return Err("bulk-stall");
    }
    BULK_ECHECS.fetch_add(1, Ordering::Relaxed);
    crate::serial_println!(
        "BOUCHAUD_USB_BULK_ECHEC slot={} dci={} cc={} status={:#010x} octets={}",
        slot, dci, cc, event.status, octets,
    );
    Err("bulk-erreur")
}

/// Debloque un point de terminaison arrete par un STALL.
///
/// Les DEUX moities comptent. `Reset Endpoint` remet l'etat cote HOTE ; sans
/// `CLEAR_FEATURE(ENDPOINT_HALT)`, le PERIPHERIQUE refuse encore tout, et le
/// pilote conclut que la cle est morte alors qu'elle attend qu'on la debloque.
/// L'inverse est aussi vrai : effacer le halt sans reinitialiser le point
/// laisse le controleur refuser les TRB suivants.
fn recupere_point_bulk(
    controller: &mut Controller,
    device: &mut Device,
    index: usize,
    entree: bool,
) -> bool {
    let (slot, dci, adresse) = {
        let st = &controller.stockages[index];
        let point = if entree { &st.entree } else { &st.sortie };
        (st.slot_id, point.dci, point.adresse)
    };

    let controle = (CMD_RESET_ENDPOINT << 10) | ((dci as u32) << 16) | ((slot as u32) << 24);
    if command_raw(controller, 0, controle).is_err() {
        return false;
    }

    // L'anneau repart de son debut. Le controleur reprendra a l'adresse qu'on
    // lui donne avec DCS a un ; les TRB deja consommes portent l'ancien cycle
    // et ne seront pas repris.
    let phys = {
        let st = &mut controller.stockages[index];
        let point = if entree { &mut st.entree } else { &mut st.sortie };
        point.ring.index = 0;
        point.ring.cycle = 1;
        unsafe { prepare_link(&point.ring, 1) };
        point.ring.phys
    };
    let controle = (CMD_SET_TR_DEQUEUE << 10) | ((dci as u32) << 16) | ((slot as u32) << 24);
    if command_raw(controller, phys | 1, controle).is_err() {
        return false;
    }

    // CLEAR_FEATURE(ENDPOINT_HALT) : destinataire « point de terminaison »
    // (0x02), fonctionnalite zero, index = adresse du point.
    let setup = setup_packet(0x02, 1, 0, adresse as u16, 0);
    if control_transfer(controller, device, setup, 0, false, BUDGET_ATTENTE_NS).is_err() {
        return false;
    }
    BULK_RECUPERATIONS.fetch_add(1, Ordering::Relaxed);
    crate::serial_println!(
        "BOUCHAUD_USB_STOCKAGE_RECUPERE slot={} dci={} ep={:#04x}",
        slot, dci, adresse,
    );
    true
}

/// La procedure de reprise du transport BOT, dans l'ordre que la specification
/// impose.
///
/// Reinitialisation de classe, PUIS deblocage des deux points. L'inverse ne
/// marche pas : la reinitialisation remet le peripherique en attente d'un
/// nouveau CBW, et c'est seulement apres qu'il accepte de voir ses points
/// debloques.
fn reinitialisation_bot(controller: &mut Controller, device: &mut Device, index: usize) -> bool {
    let interface = controller.stockages[index].interface;
    BOT_REINITIALISATIONS.fetch_add(1, Ordering::Relaxed);
    // 0x21 : hote vers peripherique, type classe, destinataire interface.
    // 0xFF : Bulk-Only Mass Storage Reset.
    let setup = setup_packet(0x21, 0xFF, 0, interface as u16, 0);
    if control_transfer(controller, device, setup, 0, false, BUDGET_ATTENTE_NS).is_err() {
        crate::serial_println!(
            "BOUCHAUD_USB_STOCKAGE_REINIT_ECHEC slot={} if={}",
            controller.stockages[index].slot_id, interface,
        );
        return false;
    }
    let entree = recupere_point_bulk(controller, device, index, true);
    let sortie = recupere_point_bulk(controller, device, index, false);
    entree && sortie
}

/// Une commande du transport Bulk-Only : CBW, donnees, CSW.
///
/// Rend le nombre d'octets de la phase de donnees reellement transferes.
fn bot_commande(
    controller: &mut Controller,
    device: &mut Device,
    index: usize,
    cdb: &[u8],
    vers_hote: bool,
    octets: u32,
) -> Result<usize, &'static str> {
    if octets as usize > TAMPON_STOCKAGE {
        return Err("bot-trop-long");
    }
    let (etiquette, enveloppe_virt, enveloppe_phys, donnees_phys) = {
        let st = &mut controller.stockages[index];
        let etiquette = st.etiquette;
        // L'etiquette CHANGE a chaque commande : c'est elle qui distingue la
        // reponse a celle-ci de la reponse a la precedente. Une etiquette fixe
        // rendrait la verification du CSW sans objet.
        st.etiquette = st.etiquette.wrapping_add(1);
        (etiquette, st.enveloppe_virt, st.enveloppe_phys, st.donnees_phys)
    };

    let mut cbw = [0u8; stockage::CBW_OCTETS];
    if !stockage::encode_cbw(&mut cbw, etiquette, octets, vers_hote, 0, cdb) {
        return Err("bot-cbw-impossible");
    }
    unsafe {
        copy_nonoverlapping(cbw.as_ptr(), enveloppe_virt as *mut u8, stockage::CBW_OCTETS);
    }

    // Phase de commande.
    bulk_transfert(controller, index, false, enveloppe_phys, stockage::CBW_OCTETS)?;

    // Phase de donnees. Un STALL ici est prevu par le protocole : le
    // peripherique refuse les donnees et attend qu'on vienne lire son CSW. On
    // debloque le point et on continue, au lieu de declarer la cle perdue.
    let mut transferes = octets as usize;
    if octets != 0 {
        match bulk_transfert(controller, index, vers_hote, donnees_phys, octets as usize) {
            Ok(n) => transferes = n,
            Err("bulk-stall") => {
                // Le peripherique refuse les donnees et attend qu'on vienne
                // lire son CSW : rien n'a ete transfere.
                transferes = 0;
                if !recupere_point_bulk(controller, device, index, vers_hote) {
                    return Err("bot-stall-non-recupere");
                }
            }
            Err(erreur) => return Err(erreur),
        }
    }

    // Phase de statut. Un STALL ici aussi est prevu : la specification demande
    // de debloquer et de RELIRE une fois. Deux STALL de suite veulent dire que
    // le peripherique ne suit plus le protocole, et c'est la reprise complete.
    let csw_phys = enveloppe_phys + stockage::CBW_OCTETS as u64;
    let csw_virt = enveloppe_virt + stockage::CBW_OCTETS;
    let lu = match bulk_transfert(controller, index, true, csw_phys, stockage::CSW_OCTETS) {
        Ok(n) => n,
        Err("bulk-stall") => {
            if !recupere_point_bulk(controller, device, index, true) {
                return Err("bot-csw-non-recupere");
            }
            bulk_transfert(controller, index, true, csw_phys, stockage::CSW_OCTETS)
                .map_err(|_| "bot-csw-illisible")?
        }
        Err(erreur) => return Err(erreur),
    };
    // Un CSW tronque n'est pas un CSW abime : c'est autre chose. Le decoder
    // sur un tampon a moitie rempli lirait un residu et un statut inventes,
    // dont l'un des deux dirait « tout va bien ».
    if lu < stockage::CSW_OCTETS {
        reinitialisation_bot(controller, device, index);
        return Err("bot-csw-tronque");
    }

    let brut = unsafe { core::slice::from_raw_parts(csw_virt as *const u8, stockage::CSW_OCTETS) };
    let Some(csw) = stockage::decode_csw(brut) else {
        reinitialisation_bot(controller, device, index);
        return Err("bot-csw-invalide");
    };
    if !stockage::transfert_complet(&csw, etiquette) {
        if csw.statut == stockage::StatutCsw::ErreurDePhase {
            // Erreur de phase : le peripherique et l'hote ne sont plus d'accord
            // sur l'etat du transport. Rien d'autre que la reprise complete ne
            // les remet d'accord.
            reinitialisation_bot(controller, device, index);
            return Err("bot-erreur-de-phase");
        }
        return Err("bot-commande-echouee");
    }

    // DEUX COMPTES, ET LE PLUS PETIT GAGNE.
    //
    // Le controleur dit combien d'octets il a REELLEMENT deplaces ; le CSW dit
    // combien le PERIPHERIQUE croit en avoir donnes. Ils devraient s'accorder.
    // Quand ils ne s'accordent pas, prendre celui du peripherique ferait rendre
    // a l'appelant des octets que le tampon contenait AVANT le transfert -- la
    // moitie d'un secteur, et l'autre moitie d'un secteur d'avant.
    let dit_par_le_peripherique = stockage::octets_transferes(&csw, octets) as usize;
    if dit_par_le_peripherique != transferes {
        BULK_DESACCORDS.fetch_add(1, Ordering::Relaxed);
    }
    Ok(dit_par_le_peripherique.min(transferes))
}

/// Interroge un support fraichement configure : prete, quel genre, quelle
/// capacite.
fn demarre_stockage(
    controller: &mut Controller,
    device: &mut Device,
    index: usize,
) -> Result<(), &'static str> {
    // `TEST UNIT READY` echoue tant que le support monte en puissance ; c'est
    // la reponse NORMALE d'une cle qu'on vient de brancher. Le refus se
    // constate sur la duree, pas sur le premier essai.
    let mut prete = false;
    for _ in 0..ESSAIS_UNITE_PRETE {
        let cdb = stockage::cdb_test_unite_prete();
        if bot_commande(controller, device, index, &cdb, true, 0).is_ok() {
            prete = true;
            break;
        }
        // Le sens : une unite qui n'est pas prete l'explique, et la demander
        // est ce qui efface la condition d'erreur en attente. Sans cela,
        // certaines cles repondent en erreur a TOUT ce qui suit.
        let sens = stockage::cdb_demande_sens(18);
        let _ = bot_commande(controller, device, index, &sens, true, 18);
        wait_ms(50);
    }
    if !prete {
        return Err("unite-jamais-prete");
    }

    let cdb = stockage::cdb_interroge(36);
    let lu = bot_commande(controller, device, index, &cdb, true, 36)?;
    let brut = unsafe {
        core::slice::from_raw_parts(controller.stockages[index].donnees_virt as *const u8, lu)
    };
    let interrogation = stockage::decode_interrogation(brut).ok_or("interrogation-invalide")?;
    if !stockage::support_utilisable(&interrogation) {
        return Err("support-non-bloc");
    }
    controller.stockages[index].amovible = interrogation.amovible;

    let cdb = stockage::cdb_lit_capacite();
    let lu = bot_commande(controller, device, index, &cdb, true, 8)?;
    let brut = unsafe {
        core::slice::from_raw_parts(controller.stockages[index].donnees_virt as *const u8, lu)
    };
    let capacite = stockage::decode_capacite(brut).ok_or("capacite-invalide")?;
    let st = &mut controller.stockages[index];
    st.taille_bloc = capacite.taille_bloc;
    st.blocs = capacite.blocs();
    let (taille_bloc, blocs, slot_id, amovible) = (st.taille_bloc, st.blocs, st.slot_id, st.amovible);
    STOCKAGES_PRETS.fetch_add(1, Ordering::Relaxed);
    crate::serial_println!(
        "BOUCHAUD_USB_STOCKAGE_PRET slot={} blocs={} taille_bloc={} octets={} amovible={}",
        slot_id, blocs, taille_bloc, capacite.octets(), amovible as u8,
    );
    // LE VOLUME EST PUBLIE ICI, ET C'EST CE QUI FAIT DE CE PILOTE UN PILOTE.
    //
    // Sans enregistrement, la pile BOT n'aurait pour appelant que le test qui
    // la prouve. Enregistree, la cle devient lisible par FAT32, par le
    // decoupage GPT et par la persistance sans qu'aucun des trois ne change.
    publie_le_volume_usb(taille_bloc, blocs);
    Ok(())
}

/// Lit ou ecrit des blocs, en decoupant selon le tampon.
///
/// `READ (10)` compte les blocs sur seize bits et le tampon en porte
/// cent-vingt-huit : la boucle est donc bornee par le tampon, jamais par le
/// champ. Elle rend le nombre de blocs REELLEMENT transferes -- un transfert
/// partiel se dit, il ne s'arrondit pas.
fn stockage_transfere(
    controller: &mut Controller,
    device: &mut Device,
    index: usize,
    ecriture: bool,
    lba: u64,
    blocs: usize,
    tampon: &mut [u8],
    source: Option<&[u8]>,
) -> usize {
    let (taille_bloc, total_blocs, donnees_virt) = {
        let st = &controller.stockages[index];
        (st.taille_bloc, st.blocs, st.donnees_virt)
    };
    if taille_bloc == 0 || blocs == 0 {
        return 0;
    }
    // Un LBA hors bornes n'est pas une panne du support : c'est une erreur
    // d'appelant, et la laisser partir couterait un aller-retour materiel pour
    // recevoir une erreur moins claire.
    match lba.checked_add(blocs as u64) {
        Some(fin) if fin <= total_blocs => {}
        _ => return 0,
    }
    if lba > u32::MAX as u64 {
        // `READ (10)` adresse sur trente-deux bits. Au-dela il faut `READ (16)`,
        // que ce pilote n'emet pas -- et servir l'adresse tronquee lirait le
        // mauvais secteur en silence.
        return 0;
    }

    let par_transfert = stockage::blocs_par_transfert(TAMPON_STOCKAGE, taille_bloc) as usize;
    if par_transfert == 0 {
        return 0;
    }
    let octets_bloc = taille_bloc as usize;
    let mut faits = 0usize;
    while faits < blocs {
        let lot = (blocs - faits).min(par_transfert);
        let octets = lot * octets_bloc;
        let decalage = faits * octets_bloc;
        if ecriture {
            let Some(donnees) = source else { return faits };
            if decalage + octets > donnees.len() {
                return faits;
            }
            unsafe {
                copy_nonoverlapping(
                    donnees.as_ptr().add(decalage),
                    donnees_virt as *mut u8,
                    octets,
                );
            }
        }
        let cdb = stockage::cdb_transfert_10(ecriture, (lba + faits as u64) as u32, lot as u16);
        let resultat = bot_commande(controller, device, index, &cdb, !ecriture, octets as u32);
        let transferes = match resultat {
            Ok(n) => n,
            Err(erreur) => {
                crate::serial_println!(
                    "BOUCHAUD_USB_STOCKAGE_IO_ECHEC ecriture={} lba={} blocs={} erreur={}",
                    ecriture as u8, lba + faits as u64, lot, erreur,
                );
                return faits;
            }
        };
        if !ecriture {
            if decalage + transferes > tampon.len() {
                return faits;
            }
            unsafe {
                copy_nonoverlapping(
                    donnees_virt as *const u8,
                    tampon.as_mut_ptr().add(decalage),
                    transferes,
                );
            }
        }
        // Un transfert partiel s'arrete la ou il s'est arrete. Compter le lot
        // entier ferait croire a des blocs qui n'ont pas ete lus.
        let blocs_faits = transferes / octets_bloc;
        faits += blocs_faits;
        if blocs_faits != lot {
            break;
        }
    }
    faits
}


// ---------------------------------------------------------------------------
// Le support USB derriere la couche bloc generique
// ---------------------------------------------------------------------------
//
// # Pourquoi un pilote de la couche bloc, et pas une fonction de lecture
//
// « Une fonctionnalite n'est pas faite si elle existe sans appelant reel. »
// Une pile BOT qui exposerait `lit_secteur()` n'aurait pour appelant que le
// test qui la prouve : le systeme de fichiers, lui, parle a un VOLUME. En
// s'enregistrant comme `PiloteBloc`, la cle devient lisible par FAT32, par le
// decoupage GPT et par la persistance sans qu'aucun de ces trois ne change.
//
// # Le verrou, et pourquoi il est celui du pilote USB
//
// Le transport BOT est synchrone : trois transferts et deux attentes
// d'evenement par commande. Il se sert du MEME anneau d'evenements que la
// scrutation HID, et deux fils qui le liraient en meme temps se voleraient
// leurs achevements. `RUNTIME_BUSY` est deja ce verrou-la ; en prendre un
// second n'ajouterait qu'un ordre de plus a respecter.
//
// # Ce que le descripteur ne demande PAS au verrou
//
// La capacite est publiee dans deux atomiques au moment du montage. Le
// systeme de fichiers demande le descripteur bien plus souvent qu'il ne lit,
// et le lui faire payer une attente sur le verrou du pilote USB ferait
// dependre la reactivite de l'interface du trafic disque.

/// Volume attribue au premier support USB monte. Le disque interne prend
/// deja `Volume(2)`.
pub const VOLUME_USB: Volume = Volume(3);

/// Capacite publiee, pour que `descripteur()` ne prenne aucun verrou.
static USB_BLOCS: AtomicU64 = AtomicU64::new(0);
static USB_TAILLE_BLOC: AtomicUsize = AtomicUsize::new(0);
/// Requetes refusees parce que le pilote USB etait occupe.
static USB_OCCUPE: AtomicUsize = AtomicUsize::new(0);

/// Tours de garde avant d'abandonner l'attente du verrou du pilote USB.
///
/// Une commande BOT dure quelques millisecondes. Attendre sans limite ferait
/// d'un pilote USB bloque un systeme de fichiers bloque ; rendre `Erreur`
/// apres une attente bornee laisse l'appelant decider.
const ATTENTE_RUNTIME: usize = 50_000_000;

/// Prend le pilote USB pour la duree d'une operation.
fn avec_le_pilote_usb<R>(action: impl FnOnce(&mut Runtime) -> R) -> Option<R> {
    let mut tours = 0usize;
    while RUNTIME_BUSY
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        tours += 1;
        if tours >= ATTENTE_RUNTIME {
            USB_OCCUPE.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        core::hint::spin_loop();
    }
    let resultat = unsafe {
        #[allow(static_mut_refs)]
        RUNTIME.as_mut().map(action)
    };
    RUNTIME_BUSY.store(false, Ordering::Release);
    resultat
}

/// Le premier support pret, s'il y en a un.
fn premier_support(runtime: &mut Runtime) -> Option<(usize, usize)> {
    for (ic, controleur) in runtime.controllers.iter().enumerate() {
        for index in 0..controleur.stockage_count {
            let st = &controleur.stockages[index];
            if st.actif && st.taille_bloc != 0 && st.blocs != 0 {
                return Some((ic, index));
            }
        }
    }
    None
}

/// Publie la capacite du premier support pret, et enregistre le volume.
fn publie_le_volume_usb(taille_bloc: u32, blocs: u64) {
    USB_TAILLE_BLOC.store(taille_bloc as usize, Ordering::Release);
    USB_BLOCS.store(blocs, Ordering::Release);
    crate::drivers::bloc::enregistre(VOLUME_USB, &PILOTE_USB);
    crate::serial_println!(
        "BOUCHAUD_USB_STOCKAGE_VOLUME volume={} blocs={} taille_bloc={} mio={}",
        VOLUME_USB.indice(),
        blocs,
        taille_bloc,
        blocs.saturating_mul(taille_bloc as u64) / (1024 * 1024),
    );
}

pub struct PiloteUsbStockage;
static PILOTE_USB: PiloteUsbStockage = PiloteUsbStockage;

impl PiloteUsbStockage {
    /// Le corps commun aux deux sens : trouver le support, sortir son
    /// peripherique de la table, transferer, le remettre.
    ///
    /// Le peripherique est SORTI parce que `bot_commande` a besoin du
    /// controleur et du peripherique en meme temps, et que le second vit dans
    /// le premier. Le remettre est obligatoire : l'anneau de controle a avance,
    /// et la copie que l'on tient est la seule a jour.
    fn transfere(
        &self,
        ecriture: bool,
        lba: u64,
        blocs: usize,
        tampon: &mut [u8],
        source: Option<&[u8]>,
    ) -> Achevement {
        let resultat = avec_le_pilote_usb(|runtime| {
            let Some((ic, index)) = premier_support(runtime) else {
                return Achevement::Absent;
            };
            let controleur = &mut runtime.controllers[ic];
            let slot = controleur.stockages[index].slot_id as usize;
            if slot >= controleur.devices.len() {
                return Achevement::Erreur;
            }
            let Some(mut device) = controleur.devices[slot].take() else {
                return Achevement::Erreur;
            };
            let faits = stockage_transfere(
                controleur, &mut device, index, ecriture, lba, blocs, tampon, source,
            );
            controleur.devices[slot] = Some(device);
            if faits == blocs {
                Achevement::Fait(faits)
            } else {
                Achevement::Erreur
            }
        });
        resultat.unwrap_or(Achevement::Erreur)
    }
}

impl PiloteBloc for PiloteUsbStockage {
    fn descripteur(&self) -> Descripteur {
        Descripteur {
            taille_bloc: USB_TAILLE_BLOC.load(Ordering::Acquire).max(1),
            blocs: USB_BLOCS.load(Ordering::Acquire),
            // Le transport BOT est strictement sequentiel : une commande, sa
            // reponse, puis la suivante. Declarer plus laisserait croire a un
            // parallelisme que le protocole interdit.
            profondeur_file: 1,
            // Le pilote n'emet pas `SYNCHRONIZE CACHE`. Le declarer faux est le
            // seul choix honnete : une cle avec un cache d'ecriture peut avoir
            // accepte une ecriture sans l'avoir posee.
            vidange_reelle: false,
            nom: "usb-bot",
        }
    }

    fn soumet(&self, requete: Requete, tampon: &mut [u8]) -> Achevement {
        match requete.genre {
            Genre::Lecture => self.transfere(false, requete.lba, requete.blocs, tampon, None),
            // Une ecriture par ce chemin n'a pas de donnees a poser : c'est une
            // erreur d'appelant, pas une panne du support.
            Genre::Ecriture => Achevement::Erreur,
            Genre::Vidange => Achevement::Fait(0),
        }
    }

    fn soumet_ecriture(&self, requete: Requete, donnees: &[u8]) -> Achevement {
        match requete.genre {
            Genre::Ecriture => {
                let mut vide: [u8; 0] = [];
                self.transfere(true, requete.lba, requete.blocs, &mut vide, Some(donnees))
            }
            Genre::Lecture => Achevement::Erreur,
            Genre::Vidange => Achevement::Fait(0),
        }
    }
}

/// Supports trouves, supports prets, transferts bulk, octets, stalls,
/// recuperations, reprises BOT, echecs, desaccords, refus pour cause d'occupe.
pub fn stockage_stats() -> (usize, usize, usize, u64, usize, usize, usize, usize, usize, usize) {
    (
        STOCKAGES_TROUVES.load(Ordering::Relaxed),
        STOCKAGES_PRETS.load(Ordering::Relaxed),
        BULK_TRANSFERTS.load(Ordering::Relaxed),
        BULK_OCTETS.load(Ordering::Relaxed),
        BULK_STALLS.load(Ordering::Relaxed),
        BULK_RECUPERATIONS.load(Ordering::Relaxed),
        BOT_REINITIALISATIONS.load(Ordering::Relaxed),
        BULK_ECHECS.load(Ordering::Relaxed),
        BULK_DESACCORDS.load(Ordering::Relaxed),
        USB_OCCUPE.load(Ordering::Relaxed),
    )
}

/// Un support USB est-il monte et lisible ?
pub fn stockage_pret() -> bool {
    USB_BLOCS.load(Ordering::Acquire) != 0
}

pub fn log_stockage() {
    let (trouves, prets, transferts, octets, stalls, recuperations, reprises, echecs, desaccords, occupes) =
        stockage_stats();
    if trouves == 0 {
        return;
    }
    crate::serial_println!(
        "[USB-STOCKAGE] trouves={} prets={} blocs={} taille_bloc={} transferts={} octets={} \
         stalls={} recuperations={} reprises_bot={} echecs={} desaccords={} occupes={}",
        trouves, prets,
        USB_BLOCS.load(Ordering::Relaxed),
        USB_TAILLE_BLOC.load(Ordering::Relaxed),
        transferts, octets, stalls, recuperations, reprises, echecs, desaccords, occupes,
    );
}

pub fn is_active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}

pub fn connected_ports() -> usize {
    CONNECTED.load(Ordering::Acquire)
}

pub fn enabled_ports() -> usize {
    ENABLED.load(Ordering::Acquire)
}

pub fn addressed_devices() -> usize {
    ADDRESS_OK.load(Ordering::Acquire)
}

pub fn control_transfers_ok() -> usize {
    CONTROL_OK.load(Ordering::Acquire)
}

pub fn usb_devices() -> usize {
    USB_DEVICES.load(Ordering::Acquire)
}

pub fn hid_keyboards() -> usize {
    HID_KEYBOARDS.load(Ordering::Acquire)
}

/// Recompte les extremites HID de TOUS les controleurs et publie le total.
///
/// # Pourquoi ce recompte existe
///
/// `HID_KEYBOARDS` et `HID_MICE` n'etaient ecrits qu'a la fin de `bring_up`,
/// avec ce que l'enumeration du DEMARRAGE avait trouve. Un clavier branche
/// APRES le demarrage etait donc enumere, adresse, configure, arme -- la table
/// des ports le montrait nommement -- et les compteurs restaient a zero.
///
/// Ce n'est pas cosmetique. `stage2` decide du repli PS/2 sur ces compteurs, la
/// barre du bureau les affiche, et le scenario d'integration « branchement a
/// chaud » les interroge pour savoir si le clavier repond : il echouait sur une
/// machine ou le clavier fonctionnait, ce qui est la pire forme de test -- il
/// accuse le sous-systeme qui marche.
///
/// Le recompte porte sur l'ensemble des controleurs, et pas sur celui qui vient
/// de changer : le total est un total. Il tient dans une passe sur les
/// extremites deja en memoire.
///
/// # Securite
/// A n'appeler que depuis un contexte qui tient deja `RUNTIME_BUSY`, comme
/// `poll`, ou pendant `bring_up` avant publication.
unsafe fn recompte_les_hid(runtime: &Runtime) {
    let mut claviers = 0usize;
    let mut souris = 0usize;
    for controller in runtime.controllers.iter() {
        for index in 0..controller.hid_count {
            match controller.hids[index].kind {
                1 => claviers += 1,
                2 => souris += 1,
                _ => {}
            }
        }
    }
    HID_KEYBOARDS.store(claviers, Ordering::Release);
    HID_MICE.store(souris, Ordering::Release);
}

pub fn hid_mice() -> usize {
    HID_MICE.load(Ordering::Acquire)
}

/// Concentrateurs vus sur les ports racine et non traverses.
///
/// Un nombre non nul explique a lui seul un clavier qui ne repond pas : il est
/// derriere, et l'enumeration ne descend pas.
pub fn concentrateurs_non_traverses() -> usize {
    CONCENTRATEURS_ECHOUES.load(Ordering::Acquire)
}

/// Concentrateurs trouves, traverses ou non.
pub fn concentrateurs() -> usize {
    CONCENTRATEURS.load(Ordering::Acquire)
}

/// Peripheriques atteints derriere un concentrateur.
pub fn peripheriques_derriere_concentrateur() -> usize {
    PERIPHERIQUES_DERRIERE.load(Ordering::Acquire)
}

pub fn hid_polling() -> bool {
    is_active() && hid_keyboards().saturating_add(hid_mice()) != 0
}

pub fn hid_ready() -> bool {
    hid_keyboards() != 0 && hid_mice() != 0
}

/// Publie, PAR peripherique, s'il repond en Interrupt-IN.
///
/// Le repli EP0 existe pour qu'un peripherique muet reste utilisable. Il ne
/// doit pas rendre ce mutisme invisible : sans cette ligne, un clavier servi
/// uniquement par le repli ressemble a un clavier qui marche.
pub fn log_hid_transports() {
    unsafe {
        let Some(runtime) = RUNTIME.as_ref() else { return };
        for controller in runtime.controllers.iter() {
            for index in 0..controller.hid_count {
                let ep = &controller.hids[index];
                if !ep.active {
                    continue;
                }
                crate::serial_println!(
                    "BOUCHAUD_HID_TRANSPORT slot={} dci={} kind={} evenements={} transport={}",
                    ep.slot_id, ep.dci, ep.kind, ep.evenements,
                    if ep.evenements != 0 { "interrupt-in" } else { "repli-ep0" },
                );
            }
        }
    }
}

pub fn hid_control_fallback_stats() -> (usize, usize, usize) {
    (
        HID_CONTROL_POLLS.load(Ordering::Acquire),
        HID_CONTROL_REPORTS.load(Ordering::Acquire),
        HID_CONTROL_FAILS.load(Ordering::Acquire),
    )
}

pub fn hid_transport_stats() -> (usize, usize, usize, usize, usize, usize, usize, usize) {
    (
        HID_POLLS.load(Ordering::Acquire),
        HID_TRANSFER_EVENTS.load(Ordering::Acquire),
        HID_REPORTS.load(Ordering::Acquire),
        HID_KEYBOARD_REPORTS.load(Ordering::Acquire),
        HID_MOUSE_REPORTS.load(Ordering::Acquire),
        HID_TRANSFER_ERRORS.load(Ordering::Acquire),
        HID_REARMS.load(Ordering::Acquire),
        HID_KICKS.load(Ordering::Acquire),
    )
}

/// Y a-t-il un controleur xHCI dont il faille surveiller les ports ?
///
/// # Pourquoi ce n'est pas `hid_polling()`
///
/// `hid_polling()` dit « un clavier ou une souris USB a repondu ». Le cas qui
/// compte pour le branchement a chaud est exactement l'INVERSE : demarrer sans
/// clavier, puis en brancher un. Gater la surveillance sur la presence d'un
/// HID rendrait le branchement a chaud inoperant precisement quand on en a
/// besoin.
/// Combien de controleurs xHCI ont ete mis en service.
///
/// Zero veut dire qu'il n'y a pas de bus USB a interroger : ce n'est PAS la
/// meme chose qu'un bus present sur lequel rien ne repond, et confondre les
/// deux ferait echouer un essai pour une absence de materiel.
pub fn controleurs() -> usize {
    CONTROLLERS.load(Ordering::Acquire)
}

pub fn surveille_branchements() -> bool {
    ACTIVE.load(Ordering::Acquire) && CONTROLLERS.load(Ordering::Acquire) != 0
}

/// Periode de relecture des bits de changement des ports, en millisecondes.
///
/// Deux cents millisecondes : assez court pour qu'un branchement paraisse
/// immediat -- personne ne mesure un cinquieme de seconde entre la prise et le
/// curseur --, assez long pour que huit lectures de registre par controleur ne
/// pesent rien. La periode est en TEMPS et non en nombre de tours : le bureau
/// scrute toutes les deux millisecondes quand un HID repond et toutes les
/// deux cent cinquante quand aucun ne repond, et une borne en tours donnerait
/// deux comportements tres differents.
const PERIODE_SCRUTATION_PORTS_MS: u64 = 200;

static DERNIERE_SCRUTATION_PORTS_NS: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

// ---------------------------------------------------------------------------
// Le fil d'entree
// ---------------------------------------------------------------------------

/// Le fil de scrutation tourne-t-il ?
static FIL_HID_ACTIF: AtomicBool = AtomicBool::new(false);
/// Tours du fil. Non nul veut dire qu'il vit vraiment.
static FIL_HID_TOURS: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

/// Le fil d'entree a-t-il pris la scrutation en charge ?
pub fn fil_hid_actif() -> bool {
    FIL_HID_ACTIF.load(Ordering::Acquire)
}

/// Tours effectues par le fil d'entree.
pub fn fil_hid_tours() -> u64 {
    FIL_HID_TOURS.load(Ordering::Relaxed)
}

fn fil_hid() -> ! {
    loop {
        FIL_HID_TOURS.fetch_add(1, Ordering::Relaxed);
        poll();
        // UN TICK, SOIT UNE MILLISECONDE.
        //
        // C'est la cadence d'une souris USB rapide. Scruter plus vite ne
        // gagnerait rien -- le peripherique ne produit pas plus -- et
        // couterait un coeur ; scruter plus lentement se sentirait au
        // pointeur.
        crate::kernel::task::sleep_ticks(1);
    }
}

static FIL_REPLI_ACTIF: AtomicBool = AtomicBool::new(false);
static REPLI_CURSEUR: AtomicUsize = AtomicUsize::new(0);
static REPLI_TOURS: AtomicU64 = AtomicU64::new(0);
static REPLI_SERVIS: AtomicU64 = AtomicU64::new(0);

/// Tours et points servis par le fil du repli EP0.
pub fn repli_ep0_compteurs() -> (u64, u64) {
    (
        REPLI_TOURS.load(Ordering::Relaxed),
        REPLI_SERVIS.load(Ordering::Relaxed),
    )
}

/// Sert un point muet, verrou pris puis RENDU.
///
/// Le verrou est le meme que celui de la scrutation, et c'est voulu : deux
/// fils qui liraient l'anneau d'evenements en meme temps se voleraient leurs
/// achevements. Ce qui change, c'est la DUREE de sa tenue -- un transfert de
/// controle, pas cinq.
fn repli_ep0_un_tour() -> bool {
    if RUNTIME_BUSY
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return false;
    }
    // LE CURSEUR TOURNE SUR LES DEUX AXES.
    //
    // Il choisit le controleur de depart ET le point de depart dans ce
    // controleur. La machine de reference en a DEUX ; sans rotation, le
    // premier aurait toujours la main, et un clavier branche sur le second ne
    // recevrait jamais son pont.
    let curseur = REPLI_CURSEUR.load(Ordering::Relaxed);
    let mut servi = false;
    unsafe {
        #[allow(static_mut_refs)]
        if let Some(runtime) = RUNTIME.as_mut() {
            let nombre = runtime.controllers.len();
            for decalage in 0..nombre {
                let ic = (curseur + decalage) % nombre;
                if repli_ep0_un_point(&mut runtime.controllers[ic], curseur).is_some() {
                    REPLI_CURSEUR.store(curseur.wrapping_add(1), Ordering::Relaxed);
                    REPLI_SERVIS.fetch_add(1, Ordering::Relaxed);
                    servi = true;
                    break;
                }
            }
        }
    }
    RUNTIME_BUSY.store(false, Ordering::Release);
    servi
}

/// Le pont EP0, sur son propre fil.
///
/// # CE QUE CE FIL RETIRE DE LA BOUCLE D'ENTREE
///
/// Le repli vivait DANS `poll()`, un tour sur deux, et servait tous les points
/// muets d'affilee. Chacun coute un transfert de controle synchrone, le verrou
/// du pilote tenu du debut a la fin. Le releve du 12 septembre le chiffre :
/// `polls=11065` en soixante-douze secondes -- cent soixante-six tours par
/// seconde la ou `sleep_ticks(1)` en vise mille.
///
/// La souris etait donc lue a quatre-vingts hertz, douze millisecondes de
/// grain, et c'est cela qu'on sent comme « elle met trop de temps a se
/// deplacer ».
///
/// # Pourquoi il ne rend pas le repli moins bon
///
/// Il sert UN point par tour et dort un tick. Avec deux points muets -- un
/// clavier et une interface qui ne repond jamais -- chacun est interroge cinq
/// cents fois par seconde, soit quatre fois plus souvent qu'avant. Ce n'est
/// pas un compromis : c'est le meme travail, sorti du chemin ou il coutait
/// cher.
fn fil_repli_ep0() -> ! {
    loop {
        REPLI_TOURS.fetch_add(1, Ordering::Relaxed);
        repli_ep0_un_tour();
        crate::kernel::task::sleep_ticks(1);
    }
}

/// Lance le fil du repli EP0.
pub fn demarre_le_fil_repli_ep0() -> bool {
    if FIL_REPLI_ACTIF.load(Ordering::Acquire) {
        return true;
    }
    // Interactive : ce fil porte des frappes et des mouvements de pointeur
    // pour tout peripherique muet en Interrupt-IN. Sur la machine de
    // reference, le clavier N'A PAS d'autre transport.
    if crate::kernel::task::spawn_noyau_priorite(
        fil_repli_ep0,
        "usb-repli",
        crate::kernel::task::Priorite::Interactive,
    ) {
        FIL_REPLI_ACTIF.store(true, Ordering::Release);
        crate::serial_println!(
            "BOUCHAUD_USB_REPLI_FIL_LANCE periode_ms=1 priorite=interactive"
        );
        return true;
    }
    crate::serial_println!(
        "BOUCHAUD_USB_REPLI_FIL_REFUSE raison=tache-non-creee \
consequence=clavier-muet-sans-transport"
    );
    false
}

/// Sort la scrutation des entrees de la boucle de trames du compositeur.
///
/// # LE DEFAUT, MESURE SUR LA MACHINE DE REFERENCE
///
/// `poll()` etait appele depuis la boucle de trames, et de nulle part
/// ailleurs. L'entree etait donc lue A LA CADENCE DU RENDU : une trame longue
/// ne ralentissait pas le pointeur, elle l'ARRETAIT.
///
/// L'enregistreur de vol du TRIGKEY le montre sans interpretation. Pendant les
/// cinq premieres secondes du bureau -- chargement des polices --, le journal
/// porte `FPS: 0` puis `FPS: 1`, et la souris n'etait lue qu'une fois par
/// seconde. Sur toute la session, 3 581 scrutations en 14,7 secondes : deux
/// cent quarante par seconde en moyenne, mais reparties selon le rendu et non
/// selon le temps.
///
/// Rien dans la scrutation n'appartient au rendu. Elle lit des registres et
/// des anneaux d'evenements ; la trame en cours n'attend aucun de ses
/// resultats.
///
/// # POURQUOI CE FIL N'EST PAS MIGRABLE
///
/// Le contraire de ce qu'on ferait d'instinct, et la raison est physique.
///
/// Sur le TRIGKEY, seize coeurs sont en ligne -- `SMP4_AP_STARTED count=15`,
/// `online=16` -- et l'enregistreur de vol ne contient QUE des evenements de
/// `cpu=0`. Les compteurs de tick le confirment a chaque echantillon :
/// `timer0` avance de deux mille a treize mille, `timer1`, `timer2` et
/// `timer3` restent a zero.
///
/// Le PIT est une source unique, routee vers un seul coeur, et aucun timer
/// LAPIC par coeur n'est arme. Une tache placee sur un coeur secondaire n'y
/// serait donc jamais preemptee, et son `sleep_ticks` n'y serait jamais
/// echu : elle s'endormirait pour de bon.
///
/// Le fil reste donc sur le coeur zero, ou le tick existe. Il y partage le
/// processeur avec le compositeur, mais il en est desormais INDEPENDANT :
/// c'est le tick qui l'elit, pas la fin d'une trame.
/// L'ENREGISTREUR DE VOL N'EST PAS SUR LE CHEMIN DE L'ENTREE.
///
/// # Ce qu'il coutait a l'entree
///
/// `poll()` -- la fonction que le fil d'entree appelle mille fois par seconde
/// -- se terminait par `blackbox::poll()`. Toutes les 250 ms, celui-ci ecrit
/// plusieurs enregistrements sur la cle USB, par transferts Bulk SYNCHRONES.
/// Sur le TRIGKEY, la cle de demarrage EST la cible de l'enregistreur : chaque
/// scrutation d'entree payait donc l'enregistreur.
///
/// Rien de tout cela ne se voyait sous QEMU : sans cle cible,
/// `blackbox_storage_ready()` est faux et la fonction sort immediatement. Le
/// defaut n'existait que sur la machine, ce qui est la pire facon d'exister.
///
/// # Pourquoi un fil separe est SUR
///
/// `blackbox_append_record` prend deja `RUNTIME_BUSY` par echange compare, et
/// SAUTE l'enregistrement s'il est occupe -- le compteur `bb_busy_skips`
/// existait avant ce lot. L'appeler depuis un autre fil ne cree donc aucune
/// concurrence nouvelle : au pire un enregistrement est saute, et il est
/// compte.
///
/// La cadence est de vingt millisecondes ; `blackbox::poll()` se limite
/// lui-meme a 250 ms, et n'ecrit donc pas plus qu'avant.
fn fil_blackbox() -> ! {
    loop {
        // UNE FENETRE RENDUE SE REPREND TOUT DE SUITE.
        //
        // `poll()` rend vrai quand il a du renoncer faute d'avoir pu prendre
        // le pilote -- le systeme de fichiers travaillait sur la cle. Attendre
        // alors vingt millisecondes de plus, c'est laisser filer la trace
        // exactement au moment ou elle devient interessante : les trois
        // archives physiques s'arretent toutes a l'instant ou le navigateur
        // demarre et se met a ecrire.
        if crate::kernel::blackbox::poll() {
            crate::kernel::task::sleep_ticks(1);
        } else {
            crate::kernel::task::sleep_ticks(20);
        }
    }
}

/// Lance le fil de l'enregistreur de vol.
pub fn demarre_le_fil_blackbox() -> bool {
    if FIL_BLACKBOX_ACTIF.load(Ordering::Acquire) {
        return true;
    }
    // INTERACTIVE, ET NON NORMALE -- ET LE RAISONNEMENT D'AVANT ETAIT FAUX.
    //
    // Il disait : « l'enregistreur n'a aucune latence a defendre, il ecrit ce
    // qui s'est passe, il ne fait pas partie de ce qui se passe ». Vrai sur un
    // systeme au repos. Faux des que quelque chose arrive -- et c'est
    // precisement alors qu'on le lit.
    //
    // L'archive du 12 septembre 17:55 s'arrete a la seconde 29,028, au milieu
    // d'un tour de scrutation : deux enregistrements de vol ecrits, puis plus
    // rien, alors que le bureau tournait a soixante-deux trames par seconde et
    // que l'utilisateur s'en est servi deux minutes de plus. L'enregistreur
    // n'a pas echoue -- `bb_failures=0` jusqu'au dernier echantillon --, il
    // n'a plus ete elu. Le navigateur venait de creer vingt taches de priorite
    // Normale, et le releve de charge montre un equilibrage qui refuse
    // presque tout (`rej_bal` par dizaines de milliers).
    //
    // Il dort vingt millisecondes entre deux tours : le promouvoir ne coute
    // rien a personne, et lui rend la seule chose dont il a besoin -- etre
    // elu de temps en temps.
    if crate::kernel::task::spawn_noyau_priorite(
        fil_blackbox,
        "blackbox",
        crate::kernel::task::Priorite::Interactive,
    ) {
        FIL_BLACKBOX_ACTIF.store(true, Ordering::Release);
        crate::serial_println!(
            "BOUCHAUD_BLACKBOX_FIL_LANCE periode_ms=20 priorite=interactive"
        );
        return true;
    }
    crate::serial_println!(
        "BOUCHAUD_BLACKBOX_FIL_REFUSE raison=tache-non-creee \
consequence=enregistreur-muet"
    );
    false
}

static FIL_BLACKBOX_ACTIF: AtomicBool = AtomicBool::new(false);

/// Instant du dernier releve de cadence, et compte de scrutations a cet instant.
static CADENCE_NS: AtomicU64 = AtomicU64::new(0);
static CADENCE_POLLS: AtomicUsize = AtomicUsize::new(0);
/// Derniere cadence de scrutation observee, en scrutations par seconde.
static CADENCE_PAR_SECONDE: AtomicUsize = AtomicUsize::new(0);

/// Scrutations par seconde, MESUREES.
///
/// # Pourquoi ce chiffre est publie
///
/// C'est celui qui a nomme le defaut. Sur la machine de reference il valait
/// 3,19 -- le fil d'entree demandait une milliseconde entre deux tours et en
/// mettait 313, parce qu'une attente non aboutie coutait 330 ms. Rien a
/// l'ecran ne le disait ; il a fallu extraire l'enregistreur de vol et lire
/// `samples.log` pour le voir.
///
/// Un chiffre qu'on ne peut lire qu'en demontant la machine n'est pas un
/// diagnostic. Celui-ci sort maintenant dans le journal, a cote de l'etat des
/// peripheriques, et se lit sans rien demonter.
pub fn cadence_scrutation() -> usize {
    let maintenant = crate::kernel::timer::monotonic_ns();
    let precedent_ns = CADENCE_NS.load(Ordering::Relaxed);
    let ecoule = maintenant.saturating_sub(precedent_ns);
    if precedent_ns == 0 || ecoule >= 1_000_000_000 {
        let polls = HID_POLLS.load(Ordering::Relaxed);
        if precedent_ns != 0 && ecoule != 0 {
            let delta = polls.saturating_sub(CADENCE_POLLS.load(Ordering::Relaxed));
            let par_seconde = (delta as u64)
                .saturating_mul(1_000_000_000)
                .checked_div(ecoule)
                .unwrap_or(0);
            CADENCE_PAR_SECONDE.store(par_seconde as usize, Ordering::Relaxed);
        }
        CADENCE_NS.store(maintenant, Ordering::Relaxed);
        CADENCE_POLLS.store(polls, Ordering::Relaxed);
    }
    CADENCE_PAR_SECONDE.load(Ordering::Relaxed)
}

/// Points de terminaison ecartes par la quarantaine du repli EP0.
pub fn replis_en_quarantaine() -> usize {
    REPLIS_EN_QUARANTAINE.load(Ordering::Relaxed)
}

pub fn demarre_le_fil_hid() -> bool {
    if FIL_HID_ACTIF.load(Ordering::Acquire) {
        return true;
    }
    if crate::kernel::task::spawn_noyau_priorite(
        fil_hid,
        "usb-hid",
        crate::kernel::task::Priorite::Interactive,
    ) {
        FIL_HID_ACTIF.store(true, Ordering::Release);
        crate::serial_println!("BOUCHAUD_USB_HID_FIL_LANCE periode_ms=1 priorite=interactive");
        return true;
    }
    crate::serial_println!(
        "BOUCHAUD_USB_HID_FIL_REFUSE raison=tache-non-creee \
consequence=scrutation-reste-dans-la-boucle-de-trames"
    );
    false
}

pub fn poll() {
    let hid = hid_polling();
    if !hid && !surveille_branchements() {
        return;
    }
    HID_POLLS.fetch_add(1, Ordering::Relaxed);
    if RUNTIME_BUSY
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return;
    }

    // L'heure de la prochaine relecture des ports, decidee UNE fois pour tous
    // les controleurs : la calculer par controleur ferait scruter le second
    // deux fois plus souvent que le premier.
    let maintenant_ns = crate::kernel::timer::monotonic_ns();
    let echue = maintenant_ns.saturating_sub(
        DERNIERE_SCRUTATION_PORTS_NS.load(Ordering::Relaxed),
    ) >= PERIODE_SCRUTATION_PORTS_MS.saturating_mul(1_000_000);
    if echue {
        DERNIERE_SCRUTATION_PORTS_NS.store(maintenant_ns, Ordering::Relaxed);
    }

    unsafe {
        if let Some(runtime) = RUNTIME.as_mut() {
            let poll_no = HID_POLLS.load(Ordering::Relaxed);
            // Un branchement ou un debranchement a-t-il eu lieu pendant ce
            // tour ? Le recompte se fait UNE fois, apres la boucle, et non par
            // controleur : le total est un total.
            let mut changement_hid = false;
            for controller in runtime.controllers.iter_mut() {
                // LES RAPPORTS MIS DE COTE D'ABORD.
                //
                // Ils sont ARRIVES AVANT ceux qui sont encore dans l'anneau :
                // les traiter apres inverserait l'ordre des frappes, et une
                // touche relachee avant d'etre appuyee reste enfoncee.
                traite_differes(controller);
                for _ in 0..MAX_EVENTS_PER_POLL {
                    let Some(event) = next_event(controller) else { break };
                    match trb_type(event.control) {
                        EVT_TRANSFER => process_hid_event(controller, event),
                        EVT_PORT_STATUS_CHANGE => {
                            note_port_a_traiter(controller, port_de_l_evenement(event))
                        }
                        _ => {}
                    }
                }

                // LE BRANCHEMENT A CHAUD, APRES le drainage et jamais dedans :
                // enumerer emet des transferts de controle qui attendent leurs
                // propres evenements, ce qui reentrerait dans la boucle qu'on
                // vient de quitter.
                if echue {
                    releve_ports_changes(controller);
                }
                let mut traites = 0usize;
                while traites < BRANCHEMENTS_PAR_TOUR && controller.ports_a_traiter != 0 {
                    let port_index = controller.ports_a_traiter.trailing_zeros() as usize;
                    controller.ports_a_traiter &= !(1u32 << port_index);
                    traite_port_change(controller, port_index);
                    traites += 1;
                }
                if traites != 0 {
                    changement_hid = true;
                }
                // Some physical controllers are conservative about periodic
                // scheduling after Configure Endpoint. Re-kicking an endpoint
                // with an already pending TD is harmless and gives us a robust
                // polling-only path without MSI-X. Roughly every 128 polls.
                if poll_no & 0x7f == 0 {
                    for index in 0..controller.hid_count {
                        let ep = controller.hids[index];
                        if ep.active {
                            fence(Ordering::SeqCst);
                            ring_doorbell(controller, ep.slot_id, ep.dci);
                            HID_KICKS.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            }
            if changement_hid {
                recompte_les_hid(runtime);
                crate::serial_println!(
                    "BOUCHAUD_USB_HID_RECOMPTE claviers={} souris={}",
                    HID_KEYBOARDS.load(Ordering::Acquire),
                    HID_MICE.load(Ordering::Acquire),
                );
            }
        }
    }
    RUNTIME_BUSY.store(false, Ordering::Release);
}

/// Nom lisible d'une vitesse xHCI.
fn nom_vitesse(vitesse: u8) -> &'static str {
    match vitesse {
        concentrateur::VITESSE_PLEINE => "full",
        concentrateur::VITESSE_BASSE => "low",
        concentrateur::VITESSE_HAUTE => "high",
        concentrateur::VITESSE_SUPER => "super",
        concentrateur::VITESSE_SUPER_PLUS => "super+",
        _ => "?",
    }
}

/// L'arbre USB, tel qu'il a ete enumere.
///
/// # Pourquoi cette commande existe
///
/// Sur une machine sans console serie, « le clavier ne marche pas » n'a
/// aucune information attachee. Voir l'arbre repond a la seule question qui
/// compte en premier : le peripherique a-t-il ete VU ? Un clavier absent de
/// cette liste est un probleme d'enumeration ; un clavier present mais muet
/// est un probleme de transport. Ce sont deux enquetes differentes, et sans
/// cette liste on ne sait pas laquelle mener.
///
/// Si l'arbre n'a jamais ete parcouru -- le cas hors du bureau de reference --
/// la commande le parcourt maintenant.
pub fn lsusb(attente_ms: u64) {
    if !ACTIVE.load(Ordering::Acquire) && CONTROLLERS.load(Ordering::Acquire) == 0 {
        let _ = bring_up();
    }
    // ATTENDRE QU'UN PERIPHERIQUE ARRIVE.
    //
    // Le branchement a chaud n'est decouvert que par une scrutation, et la
    // scrutation vient de la boucle du bureau. Depuis un interpreteur de
    // commandes, personne ne scrute -- `lsusb` afficherait l'etat du
    // demarrage, pas l'etat present.
    //
    // C'est aussi la reponse a « je viens de brancher le clavier, est-ce qu'il
    // est vu ? » : la commande rend la main des qu'il l'est, et au plus tard a
    // l'echeance.
    if attente_ms != 0 {
        let debut = crate::kernel::timer::monotonic_ns();
        let limite = debut.saturating_add(attente_ms.saturating_mul(1_000_000));
        let depart = hid_keyboards() + hid_mice();

        // ON ATTEND QUE CA SE STABILISE, PAS QUE LE PREMIER ARRIVE.
        //
        // Cette boucle sortait des qu'UN peripherique de plus etait compte. Or
        // brancher un clavier et une souris est un geste unique du point de vue
        // de l'utilisateur, et leurs enumerations ne se terminent pas au meme
        // instant : rendre la main au premier montre un arbre a moitie
        // decouvert, et fait conclure « la souris n'est pas vue » alors qu'elle
        // arrivait.
        //
        // Le defaut etait MASQUE par un autre : tant que les compteurs HID
        // n'etaient pas mis a jour sur branchement a chaud, la condition de
        // sortie ne devenait jamais vraie et la boucle tournait l'echeance
        // entiere -- ce qui laissait, par accident, le temps a tout d'arriver.
        // Reparer les compteurs a donc revele ce second defaut, ce qui est
        // exactement ce que doit faire une correction.
        //
        // La reponse est un temps de CALME : une fois qu'au moins un
        // peripherique est arrive, on continue de scruter tant que le total
        // bouge, et on rend la main quand il a cesse de bouger. L'echeance
        // globale reste le plafond.
        const CALME_MS: u64 = 400;
        let mut dernier_total = depart;
        let mut stable_depuis: Option<u64> = None;

        loop {
            // Chaque tour force la relecture des ports : sans cela on
            // attendrait la periode de scrutation, qui n'avance que si
            // quelqu'un d'autre appelle `poll()`.
            DERNIERE_SCRUTATION_PORTS_NS.store(0, Ordering::Relaxed);
            poll();
            let maintenant = crate::kernel::timer::monotonic_ns();
            let total = hid_keyboards() + hid_mice();
            if total != dernier_total {
                dernier_total = total;
                stable_depuis = None;
            }
            if total > depart {
                match stable_depuis {
                    None => stable_depuis = Some(maintenant),
                    Some(depuis)
                        if maintenant.saturating_sub(depuis)
                            >= CALME_MS.saturating_mul(1_000_000) =>
                    {
                        break;
                    }
                    Some(_) => {}
                }
            }
            if maintenant >= limite {
                break;
            }
            wait_ms(10);
        }
    } else {
        // Meme sans attente, un tour de scrutation : afficher l'etat du
        // demarrage alors qu'on peut afficher l'etat present serait un
        // mensonge poli.
        DERNIERE_SCRUTATION_PORTS_NS.store(0, Ordering::Relaxed);
        poll();
    }
    if RUNTIME_BUSY.swap(true, Ordering::Acquire) {
        crate::println!("lsusb: enumeration en cours, reessayer");
        return;
    }
    let mut total = 0usize;
    unsafe {
        if let Some(runtime) = RUNTIME.as_ref() {
            for (numero, controller) in runtime.controllers.iter().enumerate() {
                crate::println!(
                    "controleur {} : {} port(s) racine, {} peripherique(s)",
                    numero, controller.max_ports, controller.compte_noeuds,
                );
                for noeud in controller.arbre[..controller.compte_noeuds].iter().flatten() {
                    total += 1;
                    // L'indentation DIT la profondeur : un clavier decale de
                    // deux crans est derriere un concentrateur, et c'est
                    // l'information qu'on cherche quand il ne repond pas.
                    let mut marge = 0;
                    while marge < noeud.profondeur {
                        crate::print!("  ");
                        marge += 1;
                    }
                    let genre = match noeud.genre_hid {
                        1 => "clavier",
                        2 => "souris",
                        _ if noeud.classe == CLASSE_CONCENTRATEUR => "concentrateur",
                        _ => "-",
                    };
                    crate::println!(
                        "  port {} route {:#07x} slot {} {:04x}:{:04x} {} classe {:#04x} {}",
                        noeud.port_racine, noeud.route, noeud.slot,
                        noeud.vendeur, noeud.produit, nom_vitesse(noeud.vitesse),
                        noeud.classe, genre,
                    );
                }
            }
        }
    }
    // Le support de masse fait partie de l'arbre : « la cle est-elle vue ? »
    // et « la cle est-elle LISIBLE ? » sont deux questions differentes, et la
    // seconde ne se lit nulle part ailleurs. Sur une machine sans console
    // serie -- le cas du Reference Device -- la reponse doit etre a l'ECRAN.
    let (trouves, prets, transferts, octets, stalls, _rec, _rep, echecs, _des, _occ) =
        stockage_stats();
    if trouves != 0 {
        let blocs = USB_BLOCS.load(Ordering::Relaxed);
        let taille = USB_TAILLE_BLOC.load(Ordering::Relaxed);
        crate::println!(
            "stockage USB : {} trouve(s), {} pret(s), volume {} = {} Mio ({} blocs de {} o)",
            trouves,
            prets,
            VOLUME_USB.indice(),
            blocs.saturating_mul(taille as u64) / (1024 * 1024),
            blocs,
            taille,
        );
        crate::println!(
            "               {} transfert(s) bulk, {} octet(s), {} stall(s), {} echec(s)",
            transferts, octets, stalls, echecs,
        );
    }
    log_stockage();
    RUNTIME_BUSY.store(false, Ordering::Release);
    if total == 0 {
        crate::println!("lsusb: aucun peripherique USB enumere");
    }
    crate::println!(
        "lsusb: {} peripherique(s), {} concentrateur(s) dont {} non traverse(s), {} derriere un concentrateur",
        total,
        CONCENTRATEURS.load(Ordering::Acquire),
        CONCENTRATEURS_ECHOUES.load(Ordering::Acquire),
        PERIPHERIQUES_DERRIERE.load(Ordering::Acquire),
    );
    crate::serial_println!(
        "BOUCHAUD_LSUSB peripheriques={} concentrateurs={} non_traverses={} derriere_concentrateur={} claviers={} souris={} branchements={} debranchements={} evenements_perdus={}",
        total,
        CONCENTRATEURS.load(Ordering::Acquire),
        CONCENTRATEURS_ECHOUES.load(Ordering::Acquire),
        PERIPHERIQUES_DERRIERE.load(Ordering::Acquire),
        HID_KEYBOARDS.load(Ordering::Acquire),
        HID_MICE.load(Ordering::Acquire),
        BRANCHEMENTS.load(Ordering::Acquire),
        DEBRANCHEMENTS.load(Ordering::Acquire),
        EVENEMENTS_PERDUS.load(Ordering::Acquire),
    );
}

pub fn bring_up() -> ActiveSummary {
    ACTIVE.store(false, Ordering::Release);
    CONNECTED.store(0, Ordering::Release);
    ENABLED.store(0, Ordering::Release);
    ADDRESS_OK.store(0, Ordering::Release);
    CONTROL_OK.store(0, Ordering::Release);
    USB_DEVICES.store(0, Ordering::Release);
    HID_KEYBOARDS.store(0, Ordering::Release);
    HID_MICE.store(0, Ordering::Release);
    HID_POLLS.store(0, Ordering::Release);
    HID_TRANSFER_EVENTS.store(0, Ordering::Release);
    HID_REPORTS.store(0, Ordering::Release);
    HID_KEYBOARD_REPORTS.store(0, Ordering::Release);
    HID_MOUSE_REPORTS.store(0, Ordering::Release);
    HID_TRANSFER_ERRORS.store(0, Ordering::Release);
    HID_REARMS.store(0, Ordering::Release);
    HID_KICKS.store(0, Ordering::Release);
    CONCENTRATEURS.store(0, Ordering::Release);
    CONCENTRATEURS_ECHOUES.store(0, Ordering::Release);
    PERIPHERIQUES_DERRIERE.store(0, Ordering::Release);
    EVENEMENTS_PERDUS.store(0, Ordering::Release);
    BRANCHEMENTS.store(0, Ordering::Release);
    DEBRANCHEMENTS.store(0, Ordering::Release);
    HID_CONTROL_POLLS.store(0, Ordering::Release);
    HID_CONTROL_REPORTS.store(0, Ordering::Release);
    HID_CONTROL_FAILS.store(0, Ordering::Release);
    CONTROLLERS.store(0, Ordering::Release);
    BLACKBOX_STORAGE_READY.store(false, Ordering::Release);
    BLACKBOX_WRITES.store(0, Ordering::Release);
    BLACKBOX_FAILURES.store(0, Ordering::Release);
    unsafe { RUNTIME = None };

    let mut devices = Vec::<PciDevice>::new();
    pci::parcours(&mut |dev| {
        if dev.class == 0x0c && dev.subclass == 0x03 && dev.prog_if == 0x30 {
            devices.push(*dev);
        }
        true
    });
    let seen = devices.len();
    if seen == 0 {
        return ActiveSummary {
            present: false,
            active: false,
            controllers_seen: 0,
            controllers_active: 0,
            bar0: 0,
            hci_version: 0,
            max_slots: 0,
            max_ports: 0,
            scratchpads: 0,
            connected_ports: 0,
            enabled_ports: 0,
            usb_devices: 0,
            hid_keyboards: 0,
            hid_mice: 0,
            error: Some("xhci-absent"),
        };
    }

    let mut runtime = Runtime { controllers: Vec::new() };
    let mut first_bar = 0u64;
    let mut first_version = 0u16;
    let mut first_slots = 0u8;
    let mut first_ports = 0u8;
    let mut scratchpads_total = 0usize;
    let mut connected_total = 0usize;
    let mut enabled_total = 0usize;
    let mut usb_total = 0usize;
    let mut keyboard_total = 0usize;
    let mut mouse_total = 0usize;

    for dev in devices {
        match init_controller(dev) {
            Ok((mut controller, scratchpads)) => {
                if first_bar == 0 {
                    first_bar = pci::bar_decode(&controller.dev, 0).adresse();
                    first_version = controller.hci_version;
                    first_slots = controller.max_slots;
                    first_ports = controller.max_ports;
                }
                scratchpads_total = scratchpads_total.saturating_add(scratchpads);
                let ports = controller.max_ports.min(MAX_PORTS_PER_CONTROLLER as u8) as usize;
                // Les ports racine d'abord, puis ce que la traversee des
                // concentrateurs a mis en file -- a plat, sans recursion.
                let mut file = FileChemins::neuve();
                for port in 0..ports {
                    enumerate_port(&mut controller, port, &mut file);
                }
                while let Some(chemin) = file.tire() {
                    enumerate_device(&mut controller, chemin, &mut file);
                }
                if file.debordements != 0 {
                    crate::serial_println!(
                        "BOUCHAUD_USB_ARBRE_TRONQUE en_attente_max={} ignores={} \
                         consequence=des-peripheriques-branches-derriere-un-\
                         concentrateur-ne-sont-pas-enumeres",
                        CHEMINS_EN_ATTENTE, file.debordements,
                    );
                }
                arm_all_hids(&mut controller);
                connected_total = connected_total.saturating_add(controller.connected_ports);
                enabled_total = enabled_total.saturating_add(controller.enabled_ports);
                usb_total = usb_total.saturating_add(controller.usb_devices);
                for endpoint in controller.hids[..controller.hid_count].iter() {
                    match endpoint.kind {
                        1 => keyboard_total += 1,
                        2 => mouse_total += 1,
                        _ => {}
                    }
                }
                runtime.controllers.push(controller);
            }
            Err(error) => {
                crate::serial_println!(
                    "BOUCHAUD_XHCI_CONTROLLER_FAIL bdf={:02x}:{:02x}.{} error={}",
                    dev.bus,
                    dev.slot,
                    dev.func,
                    error
                );
            }
        }
    }

    let active_controllers = runtime.controllers.len();
    let active = active_controllers != 0;
    if active {
        unsafe { RUNTIME = Some(runtime) };
    }
    ACTIVE.store(active, Ordering::Release);
    CONNECTED.store(connected_total, Ordering::Release);
    ENABLED.store(enabled_total, Ordering::Release);
    USB_DEVICES.store(usb_total, Ordering::Release);
    HID_KEYBOARDS.store(keyboard_total, Ordering::Release);
    HID_MICE.store(mouse_total, Ordering::Release);
    CONTROLLERS.store(active_controllers, Ordering::Release);

    if connected_total != 0 {
        crate::serial_println!("BOUCHAUD_XHCI_ROOT_PORT_GREEN count={}", connected_total);
    }
    if usb_total != 0 {
        crate::serial_println!("BOUCHAUD_USB_ENUM_GREEN devices={}", usb_total);
    }
    if keyboard_total != 0 {
        crate::serial_println!("BOUCHAUD_HID_KEYBOARD_GREEN count={}", keyboard_total);
    }
    if mouse_total != 0 {
        crate::serial_println!("BOUCHAUD_HID_MOUSE_GREEN count={}", mouse_total);
    }
    crate::serial_println!(
        "BOUCHAUD_XHCI_V34_SUMMARY controllers={}/{} ports={} enabled={} addressed={} control_ok={} usb={} keyboards={} mice={} polling={} concentrateurs={} concentrateurs_non_traverses={} derriere_concentrateur={}",
        active_controllers,
        seen,
        connected_total,
        enabled_total,
        ADDRESS_OK.load(Ordering::Acquire),
        CONTROL_OK.load(Ordering::Acquire),
        usb_total,
        keyboard_total,
        mouse_total,
        (keyboard_total + mouse_total != 0) as u8,
        CONCENTRATEURS.load(Ordering::Acquire),
        CONCENTRATEURS_ECHOUES.load(Ordering::Acquire),
        PERIPHERIQUES_DERRIERE.load(Ordering::Acquire),
    );
    // Un clavier absent ALORS QU'un concentrateur n'a pas pu etre traverse a
    // une cause nommee. Le dire ici evite de chercher le defaut dans le code
    // du clavier, qui marche.
    if keyboard_total == 0 && CONCENTRATEURS_ECHOUES.load(Ordering::Acquire) != 0 {
        crate::serial_println!(
            "BOUCHAUD_HID_ABSENT_DERRIERE_CONCENTRATEUR remede=brancher-le-\
             clavier-sur-un-port-de-la-machine"
        );
    }

    ActiveSummary {
        present: true,
        active,
        controllers_seen: seen,
        controllers_active: active_controllers,
        bar0: first_bar,
        hci_version: first_version,
        max_slots: first_slots,
        max_ports: first_ports,
        scratchpads: scratchpads_total,
        connected_ports: connected_total,
        enabled_ports: enabled_total,
        usb_devices: usb_total,
        hid_keyboards: keyboard_total,
        hid_mice: mouse_total,
        error: if active { None } else { Some("all-xhci-init-failed") },
    }
}
