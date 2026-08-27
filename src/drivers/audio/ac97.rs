//! Pilote audio AC'97 (Intel 82801AA, ce que QEMU emule sous `-device AC97`).
//!
//! Premiere sortie sonore de la machine. Le choix d'AC'97 plutot que d'Intel HDA
//! tient a la forme du materiel : deux espaces d'entrees/sorties, une liste de
//! descripteurs de tampons, et c'est tout. HDA demanderait les anneaux CORB et
//! RIRB, l'enumeration des verbes du codec et une machine a etats de flux — pour
//! le meme resultat sur une machine emulee.
//!
//! ## Comment le son sort
//!
//! Le controleur lit lui-meme la memoire : on lui remet une liste de 32
//! descripteurs, chacun designant un tampon de PCM et sa longueur, et il les
//! parcourt en boucle sans rien demander a personne. Le pilote ne fait que
//! remplir les tampons devant la tete de lecture et avancer l'index de dernier
//! tampon valide (`LVI`). Rien n'est jamais copie par le processeur pendant la
//! lecture.
//!
//! ```text
//!   userland --write--> tampon circulaire --DMA--> codec --> haut-parleur
//!                            ^                 |
//!                            +---- IRQ fin de tampon
//! ```
//!
//! ## Format
//!
//! AC'97 ne sait faire qu'une chose : 16 bits signes, deux voies, 48 kHz. Tout
//! le reste — 8 bits, mono, 44,1 kHz — est converti par le pilote au moment de
//! l'ecriture. C'est ce qui permet a `/dev/dsp` d'accepter les formats
//! habituels sans que l'appelant ait a savoir ce que la puce sait faire.

use core::ptr::write_volatile;

use crate::arch::x86_64::pci;
use crate::arch::x86_64::ports::{inb, inl, inw, outb, outl, outw};
use crate::kernel::{dmesg, memory};

// --- Registres du mixeur (BAR0, « NAM » : Native Audio Mixer) ----------------
const NAM_RESET: u16 = 0x00;
const NAM_MASTER_VOLUME: u16 = 0x02;
const NAM_PCM_VOLUME: u16 = 0x18;
const NAM_EXT_ID: u16 = 0x28;
const NAM_EXT_CTRL: u16 = 0x2A;
const NAM_PCM_FRONT_RATE: u16 = 0x2C;

// --- Registres du bus maitre (BAR1, « NABM ») -------------------------------
// Le canal de sortie PCM (« PCM out ») commence a l'offset 0x10.
const NABM_PO_BDBAR: u16 = 0x10; // adresse physique de la liste de descripteurs
const NABM_PO_CIV: u16 = 0x14; // index du descripteur courant
const NABM_PO_LVI: u16 = 0x15; // index du dernier descripteur valide
const NABM_PO_SR: u16 = 0x16; // etat
const NABM_PO_PICB: u16 = 0x18; // echantillons restants dans le tampon courant
const NABM_PO_CR: u16 = 0x1B; // controle
const NABM_GLOB_CNT: u16 = 0x2C;
const NABM_GLOB_STA: u16 = 0x30;

// Bits de `PO_CR`.
const CR_RUN: u8 = 0x01;
const CR_RESET: u8 = 0x02;

// Bits de `PO_SR`.
const SR_DCH: u16 = 0x01; // le moteur DMA s'est arrete

/// Nombre de descripteurs de la liste. Le materiel en impose 32.
const DESCRIPTEURS: usize = 32;

/// Echantillons (paires gauche/droite) par tampon.
///
/// 2048 paires a 48 kHz font ~42 ms : assez court pour que la latence reste
/// imperceptible, assez long pour qu'une interruption toutes les 42 ms ne pese
/// rien.
const ECHANTILLONS_PAR_TAMPON: usize = 2048;
const OCTETS_PAR_TAMPON: usize = ECHANTILLONS_PAR_TAMPON * 4; // 2 voies × 16 bits

/// Frequence native de la puce.
pub const FREQUENCE_NATIVE: u32 = 48000;

static mut NAM: u16 = 0;
static mut NABM: u16 = 0;
static mut PRET: bool = false;

/// Liste de descripteurs : 32 × (adresse 32 bits, longueur et drapeaux 32 bits).
static mut BDL_PHYS: u64 = 0;
static mut BDL_VIRT: *mut u32 = core::ptr::null_mut();
/// Les 32 tampons de PCM, contigus.
static mut TAMPONS_VIRT: *mut u8 = core::ptr::null_mut();

/// Prochain descripteur que le pilote remplira.
static mut ECRITURE: usize = 0;
/// Nombre de tampons remplis et pas encore joues.
static mut EN_VOL: usize = 0;
/// Le moteur DMA tourne-t-il ?
static mut EN_LECTURE: bool = false;

/// Format demande par le programme, converti a l'ecriture.
static mut FREQUENCE: u32 = 48000;
static mut VOIES: u8 = 2;
/// 16 = PCM 16 bits signe, 8 = PCM 8 bits non signe.
static mut BITS: u8 = 16;

/// Echantillons joues depuis le demarrage : c'est l'horloge de reference pour
/// synchroniser l'image sur le son.
static mut ECHANTILLONS_JOUES: u64 = 0;

// --- Acces au mixeur ---------------------------------------------------------

unsafe fn mixeur_ecrit(offset: u16, valeur: u16) {
    outw(NAM + offset, valeur);
}

unsafe fn mixeur_lit(offset: u16) -> u16 {
    inw(NAM + offset)
}

// --- Initialisation ----------------------------------------------------------

/// Le peripherique audio a-t-il ete initialise ?
pub fn pret() -> bool {
    unsafe { PRET }
}

/// Cherche et initialise la carte. Rend `false` s'il n'y en a pas.
///
/// Appele a la demande et non au boot : une machine sans carte son doit demarrer
/// exactement comme avant.
pub fn init() -> bool {
    unsafe {
        if PRET {
            return true;
        }
        let peripherique = match pci::find_audio() {
            Some(d) => d,
            None => {
                dmesg::log("ac97: aucun peripherique audio PCI");
                return false;
            }
        };
        pci::enable_bus_master(&peripherique);

        // Les deux BAR sont des espaces d'entrees/sorties : le bit 0 les marque
        // comme tels, l'adresse est dans les bits superieurs.
        NAM = (pci::bar(&peripherique, 0) & 0xFFFC) as u16;
        NABM = (pci::bar(&peripherique, 1) & 0xFFFC) as u16;
        if NAM == 0 || NABM == 0 {
            dmesg::log("ac97: BAR d'entrees/sorties absents");
            return false;
        }

        // Reveil du controleur, puis reset du codec.
        outl(NABM + NABM_GLOB_CNT, 0x00000002);
        mixeur_ecrit(NAM_RESET, 0);
        // Le codec met quelques microsecondes a repondre.
        for _ in 0..1000 {
            if mixeur_lit(NAM_RESET) != 0xFFFF {
                break;
            }
            core::hint::spin_loop();
        }

        // Volume au maximum : l'attenuation se fait plus haut, dans le
        // programme, ou elle sait ce qu'elle attenue.
        mixeur_ecrit(NAM_MASTER_VOLUME, 0x0000);
        mixeur_ecrit(NAM_PCM_VOLUME, 0x0000);

        // Frequence variable, si le codec la propose. Sinon tout sera reechantillonne
        // vers 48 kHz par le pilote.
        let extensions = mixeur_lit(NAM_EXT_ID);
        if extensions & 0x0001 != 0 {
            mixeur_ecrit(NAM_EXT_CTRL, mixeur_lit(NAM_EXT_CTRL) | 0x0001);
            mixeur_ecrit(NAM_PCM_FRONT_RATE, FREQUENCE_NATIVE as u16);
        }

        // Anneau DMA : la liste de descripteurs et les tampons.
        let (bdl_phys, bdl_virt) = match memory::alloc_dma(DESCRIPTEURS * 8) {
            Some(v) => v,
            None => {
                dmesg::log("ac97: allocation DMA de la liste impossible");
                return false;
            }
        };
        let (tampons_phys, tampons_virt) =
            match memory::alloc_dma(DESCRIPTEURS * OCTETS_PAR_TAMPON) {
                Some(v) => v,
                None => {
                    dmesg::log("ac97: allocation DMA des tampons impossible");
                    return false;
                }
            };

        BDL_PHYS = bdl_phys;
        BDL_VIRT = bdl_virt as *mut u32;
        TAMPONS_VIRT = tampons_virt;

        // Chaque descripteur pointe son tampon. La longueur est **en
        // echantillons de 16 bits**, pas en octets : une erreur ici fait jouer
        // le double ou la moitie du tampon, et s'entend tout de suite.
        for index in 0..DESCRIPTEURS {
            let adresse = tampons_phys + (index * OCTETS_PAR_TAMPON) as u64;
            write_volatile(BDL_VIRT.add(index * 2), adresse as u32);
            write_volatile(BDL_VIRT.add(index * 2 + 1), OCTETS_PAR_TAMPON as u32 / 2);
        }

        // Reset du canal de sortie, puis on lui donne sa liste.
        outb(NABM + NABM_PO_CR, CR_RESET);
        for _ in 0..10000 {
            if inb(NABM + NABM_PO_CR) & CR_RESET == 0 {
                break;
            }
            core::hint::spin_loop();
        }
        outl(NABM + NABM_PO_BDBAR, BDL_PHYS as u32);
        outb(NABM + NABM_PO_LVI, 0);

        ECRITURE = 0;
        EN_VOL = 0;
        EN_LECTURE = false;
        ECHANTILLONS_JOUES = 0;
        PRET = true;

        crate::serial_println!(
            "[kernel] ac97: {:04x}:{:04x}, mixeur {:#06x}, bus maitre {:#06x}",
            peripherique.vendor, peripherique.device, NAM, NABM
        );
        dmesg::log("ac97: sortie audio prete (48 kHz, 16 bits, stereo)");
        true
    }
}

// --- Ecriture ----------------------------------------------------------------

/// Regle le format attendu par le programme. Rend le format reellement retenu.
pub fn configure(frequence: u32, voies: u8, bits: u8) -> (u32, u8, u8) {
    unsafe {
        FREQUENCE = frequence.clamp(4000, 96000);
        VOIES = if voies >= 2 { 2 } else { 1 };
        BITS = if bits >= 16 { 16 } else { 8 };
        (FREQUENCE, VOIES, BITS)
    }
}

/// Format courant `(frequence, voies, bits)`.
pub fn format() -> (u32, u8, u8) {
    unsafe { (FREQUENCE, VOIES, BITS) }
}

/// Nombre de tampons libres : c'est la place disponible pour ecrire.
pub fn libres() -> usize {
    unsafe {
        recolte();
        DESCRIPTEURS.saturating_sub(EN_VOL).saturating_sub(1)
    }
}

/// Octets que le programme peut ecrire sans bloquer, dans **son** format.
pub fn place_disponible() -> usize {
    unsafe {
        let par_tampon = ECHANTILLONS_PAR_TAMPON * VOIES as usize * (BITS as usize / 8)
            * FREQUENCE as usize
            / FREQUENCE_NATIVE as usize;
        libres() * par_tampon.max(1)
    }
}

/// Ecrit du PCM dans le format configure. Rend le nombre d'octets consommes.
///
/// N'attend jamais : ce qui ne tient pas dans les tampons libres n'est pas
/// consomme, et l'appelant reessaie. C'est ce que fait `/dev/dsp` en mode non
/// bloquant, et cela evite qu'une ecriture tienne le noyau pendant 40 ms.
pub fn ecrit(donnees: &[u8]) -> usize {
    unsafe {
        if !PRET || donnees.is_empty() {
            return 0;
        }
        let octets_par_trame = VOIES as usize * (BITS as usize / 8);
        if octets_par_trame == 0 {
            return 0;
        }

        let mut consommes = 0usize;
        while consommes < donnees.len() {
            if libres() == 0 {
                break;
            }
            let restant = &donnees[consommes..];
            let pris = remplit_tampon(restant, octets_par_trame);
            if pris == 0 {
                break;
            }
            consommes += pris;
        }

        if consommes > 0 && !EN_LECTURE {
            demarre();
        }
        consommes
    }
}

/// Remplit un descripteur avec ce qu'on peut prendre de `source`.
///
/// C'est ici que se fait la conversion de format : la puce ne connait que 48 kHz
/// 16 bits stereo, et tout le reste est ramene a cela par duplication de voie
/// et repetition d'echantillon. Le reechantillonnage est le plus simple
/// possible — au plus proche voisin — ce qui suffit pour du 44,1 kHz vers
/// 48 kHz et evite d'embarquer un filtre dans le noyau.
unsafe fn remplit_tampon(source: &[u8], octets_par_trame: usize) -> usize {
    let index = ECRITURE % DESCRIPTEURS;
    let destination = TAMPONS_VIRT.add(index * OCTETS_PAR_TAMPON) as *mut i16;

    let trames_source_dispo = source.len() / octets_par_trame;
    if trames_source_dispo == 0 {
        return 0;
    }

    let mut ecrites = 0usize; // paires stereo ecrites dans le tampon
    let mut consommees = 0usize; // trames lues dans la source
    // Position fractionnaire dans la source, en 16.16.
    let pas = ((FREQUENCE as u64) << 16) / FREQUENCE_NATIVE as u64;
    let mut position: u64 = 0;

    while ecrites < ECHANTILLONS_PAR_TAMPON {
        let trame = (position >> 16) as usize;
        if trame >= trames_source_dispo {
            break;
        }
        let base = trame * octets_par_trame;
        let (gauche, droite) = lit_trame(source, base, octets_par_trame);
        write_volatile(destination.add(ecrites * 2), gauche);
        write_volatile(destination.add(ecrites * 2 + 1), droite);
        ecrites += 1;
        position += pas;
        consommees = ((position >> 16) as usize).min(trames_source_dispo);
    }

    if ecrites == 0 {
        return 0;
    }
    // Un tampon partiel est complete par du silence : le materiel joue toujours
    // le descripteur en entier, et laisser l'ancien contenu ferait entendre la
    // fin du son precedent.
    for reste in ecrites..ECHANTILLONS_PAR_TAMPON {
        write_volatile(destination.add(reste * 2), 0);
        write_volatile(destination.add(reste * 2 + 1), 0);
    }

    ECRITURE = ECRITURE.wrapping_add(1);
    EN_VOL += 1;
    // `LVI` designe le dernier descripteur que le materiel a le droit de jouer.
    outb(NABM + NABM_PO_LVI, (index % DESCRIPTEURS) as u8);
    consommees * octets_par_trame
}

/// Lit une trame de la source et la rend en deux voies 16 bits signees.
unsafe fn lit_trame(source: &[u8], base: usize, octets_par_trame: usize) -> (i16, i16) {
    if BITS == 8 {
        // PCM 8 bits non signe, centre sur 128.
        let g = ((source[base] as i16) - 128) << 8;
        let d = if VOIES == 2 && base + 1 < source.len() {
            ((source[base + 1] as i16) - 128) << 8
        } else {
            g
        };
        let _ = octets_par_trame;
        return (g, d);
    }
    let g = i16::from_le_bytes([source[base], source[base + 1]]);
    let d = if VOIES == 2 && base + 3 < source.len() {
        i16::from_le_bytes([source[base + 2], source[base + 3]])
    } else {
        g
    };
    (g, d)
}

/// Demarre le moteur DMA.
///
/// Les interruptions du peripherique (`IOCE`, `LVBIE`) restent **coupees**, et
/// c'est deliberé : le pilote se tient a jour en lisant l'index courant du
/// materiel, qui est de toute facon la source de verite. Une IRQ n'apporterait
/// qu'un reveil plus tot — sans interet ici, puisque le seul moment ou l'etat
/// compte est celui ou un programme ecrit ou demande la position. Le pilote
/// disque suit le meme raisonnement, et cela evite d'avoir a router une ligne
/// PCI partagee vers le bon vecteur.
unsafe fn demarre() {
    outb(NABM + NABM_PO_CR, CR_RUN);
    EN_LECTURE = true;
}

/// Compte les tampons que le materiel a fini de jouer.
///
/// Appele avant chaque ecriture plutot que depuis l'interruption seule : le son
/// doit continuer meme si une IRQ est perdue, et l'index courant du materiel
/// est de toute facon la source de verite.
unsafe fn recolte() {
    if !PRET || !EN_LECTURE {
        return;
    }
    let courant = inb(NABM + NABM_PO_CIV) as usize % DESCRIPTEURS;
    let ecriture = ECRITURE % DESCRIPTEURS;
    // Distance entre la tete de lecture et la tete d'ecriture.
    let occupes = if ecriture >= courant {
        ecriture - courant
    } else {
        DESCRIPTEURS - courant + ecriture
    };
    if occupes < EN_VOL {
        ECHANTILLONS_JOUES += ((EN_VOL - occupes) * ECHANTILLONS_PAR_TAMPON) as u64;
        EN_VOL = occupes;
    }
    // Plus rien a jouer : on arrete le moteur pour ne pas boucler sur du vieux
    // contenu.
    if EN_VOL == 0 {
        let etat = inw(NABM + NABM_PO_SR);
        if etat & SR_DCH != 0 {
            outb(NABM + NABM_PO_CR, 0);
            EN_LECTURE = false;
        }
    }
}

/// Position de lecture, en echantillons joues depuis le demarrage.
///
/// C'est l'horloge sur laquelle l'image se cale : le son ne peut pas accelerer
/// ni ralentir sans qu'on l'entende, l'image si.
pub fn position_echantillons() -> u64 {
    unsafe {
        recolte();
        if !PRET {
            return 0;
        }
        let restants = inw(NABM + NABM_PO_PICB) as u64 / 2;
        ECHANTILLONS_JOUES
            .saturating_add((ECHANTILLONS_PAR_TAMPON as u64).saturating_sub(restants))
    }
}

/// Vide la file : tout ce qui n'est pas encore joue est abandonne.
pub fn arrete() {
    unsafe {
        if !PRET {
            return;
        }
        outb(NABM + NABM_PO_CR, 0);
        outb(NABM + NABM_PO_CR, CR_RESET);
        for _ in 0..10000 {
            if inb(NABM + NABM_PO_CR) & CR_RESET == 0 {
                break;
            }
            core::hint::spin_loop();
        }
        outl(NABM + NABM_PO_BDBAR, BDL_PHYS as u32);
        outb(NABM + NABM_PO_LVI, 0);
        ECRITURE = 0;
        EN_VOL = 0;
        EN_LECTURE = false;
    }
}

/// Etat lisible pour la commande `audio`.
pub fn resume() -> (bool, u32, u8, u8, usize, usize) {
    unsafe {
        (
            PRET,
            FREQUENCE,
            VOIES,
            BITS,
            EN_VOL,
            DESCRIPTEURS,
        )
    }
}

/// Numero d'IRQ du peripherique audio, s'il y en a un.
pub fn irq() -> Option<u8> {
    pci::find_audio().map(|d| pci::interrupt_line(&d))
}

/// Lecture directe du registre d'etat global, pour le diagnostic.
pub fn etat_global() -> u32 {
    unsafe {
        if !PRET {
            return 0;
        }
        inl(NABM + NABM_GLOB_STA)
    }
}
