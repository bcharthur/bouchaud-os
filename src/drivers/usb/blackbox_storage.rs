// BOUCHAUD_TRIGKEY_BLACKBOX_V2
//
// Extension interne de `xhci_active.rs`. Ce fichier est `include!` dans le
// meme module afin de reutiliser rings, contextes et wait_event xHCI prives.

// Le decodage du protocole est DECLARE PAR `xhci_active.rs`, qui inclut ce
// fichier : le declarer une seconde fois ici compilerait `stockage/decodage.rs`
// deux fois dans le meme module, avec deux jeux de constantes qui pourraient
// diverger sans que rien ne le dise. L'alias garde les appels inchanges.
use stockage as stockage_blackbox;

const BLACKBOX_PARTITION_NAME: &[u8] = b"BOUCHAUD-BLACKBOX";
const BLACKBOX_RECORD_BYTES: usize = 4096;
const BLACKBOX_HEADER_BYTES: usize = 64;
const BLACKBOX_PAYLOAD_MAX: usize = BLACKBOX_RECORD_BYTES - BLACKBOX_HEADER_BYTES;
const BLACKBOX_RECORD_MAGIC: &[u8; 8] = b"BOUBBX01";
const BLACKBOX_RECORD_VERSION: u16 = 1;
// BOUCHAUD_VIDAGE_PAR_LOTS_V1
//
// # Deux zones, et pourquoi elles ne peuvent plus partager l'offset zero
//
// Le tampon DMA servait a la fois d'enveloppe -- CBW puis CSW -- et de zone
// de donnees, toutes deux a l'offset zero. Cela marchait tant qu'on posait un
// enregistrement a la fois : l'enveloppe part, PUIS les donnees sont copiees.
//
// Un lot se construit autrement. Les enregistrements sont formates DIRECTEMENT
// dans la zone DMA -- c'est ce qui evite une copie par enregistrement sur les
// huit mebioctets d'un vidage final --, et ils y sont deja quand le CBW doit
// partir. Les deux zones sont donc separees, et c'est cette separation qui
// rend le lot possible.
const BLACKBOX_ENVELOPPE_OCTETS: usize = 4096;
/// Zone de donnees : seize enregistrements de quatre kibioctets.
const BLACKBOX_DONNEES_MAX: usize = 64 * 1024;
const BLACKBOX_DMA_BYTES: usize = BLACKBOX_ENVELOPPE_OCTETS + BLACKBOX_DONNEES_MAX;
/// Enregistrements par lot : ce que la zone de donnees peut tenir.
const BLACKBOX_RECORDS_PAR_LOT: usize = BLACKBOX_DONNEES_MAX / BLACKBOX_RECORD_BYTES;
// BOUCHAUD_LOT_QUI_RECULE_V1
//
// # Ce que la session physique a montre
//
// Cinquante commandes d'ecriture sur environ deux cent quatre-vingts ont ete
// refusees par la clef -- un STALL sur dix-sept pour cent des lots de
// soixante-quatre kibioctets. Le transport n'etait pas casse : chaque
// deblocage reussissait. C'est la TAILLE du transfert que la clef refusait.
//
// Soixante-quatre kibioctets, c'est exactement la longueur maximale d'un TRB
// Normal. Toutes les clefs ne la servent pas, et celle-ci ne la sert pas
// toujours.
//
// # Pourquoi reculer plutot que choisir une taille prudente
//
// Une constante plus basse ferait payer a TOUTES les machines le defaut
// d'une seule, et ne dirait rien : on ne saurait jamais si la grande taille
// passait. Le lot commence donc au maximum, RECULE de moitie a chaque refus,
// et remonte apres une serie de succes. Le compteur de reculs dit ce que la
// clef a accepte, ce qu'aucune constante ne dirait.
const BLACKBOX_LOT_MINIMUM: usize = 1;
/// Succes consecutifs avant de retenter une taille double.
///
/// Seize, et non soixante-quatre : un vidage final compte environ trois cents
/// lots. A soixante-quatre, une clef qui refuse une fois au debut resterait a
/// la taille reduite pour tout le vidage -- on paierait le refus bien plus
/// cher que ce qu'il a coute.
const BLACKBOX_SUCCES_AVANT_REMONTEE: u32 = 16;

/// Taille de lot courante, jamais plus que ce que l'appelant demande.
fn lot_courant(demande: usize) -> usize {
    demande
        .min(BLACKBOX_LOT.load(Ordering::Acquire))
        .max(BLACKBOX_LOT_MINIMUM)
}

/// La clef a refuse : le lot recule de moitie.
fn lot_recule() {
    let courant = BLACKBOX_LOT.load(Ordering::Acquire);
    let reduit = (courant / 2).max(BLACKBOX_LOT_MINIMUM);
    if reduit != courant {
        BLACKBOX_LOT.store(reduit, Ordering::Release);
        BLACKBOX_RECULS.fetch_add(1, Ordering::Relaxed);
        crate::serial_println!(
            "BOUCHAUD_BLACKBOX_LOT_RECULE de={} a={} stalls={}",
            courant, reduit, BLACKBOX_STALLS.load(Ordering::Relaxed),
        );
    }
    BLACKBOX_SUCCES.store(0, Ordering::Relaxed);
}

/// La clef a accepte : au bout d'une serie, on retente plus grand.
fn lot_avance() {
    let courant = BLACKBOX_LOT.load(Ordering::Acquire);
    if courant >= BLACKBOX_RECORDS_PAR_LOT {
        return;
    }
    if BLACKBOX_SUCCES.fetch_add(1, Ordering::AcqRel) + 1 < BLACKBOX_SUCCES_AVANT_REMONTEE {
        return;
    }
    BLACKBOX_SUCCES.store(0, Ordering::Relaxed);
    let double = (courant * 2).min(BLACKBOX_RECORDS_PAR_LOT);
    BLACKBOX_LOT.store(double, Ordering::Release);
    crate::serial_println!("BOUCHAUD_BLACKBOX_LOT_REMONTE de={} a={}", courant, double);
}

static BLACKBOX_SUCCES: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);

/// Points arretes, reculs de lot, et taille de lot courante.
pub fn blackbox_lot_stats() -> (u64, u64, usize) {
    (
        BLACKBOX_STALLS.load(Ordering::Relaxed),
        BLACKBOX_RECULS.load(Ordering::Relaxed),
        BLACKBOX_LOT.load(Ordering::Acquire),
    )
}

/// Attente maximale du verrou du pilote pour un lot de vidage.
///
/// Deux cents millisecondes, comme le systeme de fichiers : le vidage n'a lieu
/// qu'a l'extinction, ou son budget total est de cinq secondes, et il n'y a
/// personne dont la latence souffrirait de cette attente.
const ATTENTE_VIDAGE_NS: u64 = 200_000_000;
const BLACKBOX_SCAN_GPT_ENTRIES: usize = 16;
const BLACKBOX_SYNC_EVERY: u32 = 64;
const BLACKBOX_KIND_FATAL: u16 = 9;

static BLACKBOX_STORAGE_READY: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);
static BLACKBOX_WRITES: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
static BLACKBOX_FAILURES: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
static BLACKBOX_CONSECUTIVE_FAILURES: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
static BLACKBOX_LAST_ERROR: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
static BLACKBOX_BUSY_SKIPS: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
static BLACKBOX_LAST_OK_NS: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
/// Points arretes par la clef en phase de donnees, et debloques sans reprise.
///
/// Ce compteur separe ce que la reinitialisation de classe confondait : une
/// clef qui refuse un transfert et attend qu'on lise son statut n'est PAS un
/// transport casse.
static BLACKBOX_STALLS: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
/// Enregistrements par lot, reduits quand la clef refuse. Voir `lot_courant`.
static BLACKBOX_LOT: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(BLACKBOX_RECORDS_PAR_LOT);
/// Reculs de la taille de lot.
static BLACKBOX_RECULS: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

#[derive(Clone, Copy, Default)]
struct BlackboxStorageDescriptor {
    interface: u8,
    bulk_in: u8,
    bulk_out: u8,
    mps_in: u16,
    mps_out: u16,
    burst_in: u8,
    burst_out: u8,
}

struct BlackboxStorage {
    slot_id: u8,
    interface: u8,
    dci_in: u8,
    dci_out: u8,
    ring_in: ProducerRing,
    ring_out: ProducerRing,
    buffer_phys: u64,
    buffer_virt: usize,
    buffer_len: usize,
    tag: u32,
    block_size: u32,
    blocks: u64,
    partition_first: u64,
    partition_blocks: u64,
    since_sync: u32,
    disabled: bool,
}

fn parse_blackbox_storage_descriptor(bytes: &[u8]) -> Option<BlackboxStorageDescriptor> {
    if bytes.len() < 9 || bytes[1] != 2 {
        return None;
    }
    let mut out = BlackboxStorageDescriptor::default();
    let mut storage_interface = false;
    let mut last_endpoint = 0u8;
    let mut offset = 0usize;

    while offset + 2 <= bytes.len() {
        let len = bytes[offset] as usize;
        let ty = bytes[offset + 1];
        if len < 2 || offset + len > bytes.len() {
            break;
        }
        match ty {
            4 if len >= 9 => {
                let alternate = bytes[offset + 3];
                let class = bytes[offset + 5];
                let subclass = bytes[offset + 6];
                let protocol = bytes[offset + 7];
                storage_interface =
                    alternate == 0 && class == 0x08 && subclass == 0x06 && protocol == 0x50;
                last_endpoint = 0;
                if storage_interface {
                    out.interface = bytes[offset + 2];
                }
            }
            5 if len >= 7 && storage_interface => {
                let address = bytes[offset + 2];
                let attributes = bytes[offset + 3] & 0x03;
                if attributes == 2 {
                    let mps = u16::from_le_bytes([bytes[offset + 4], bytes[offset + 5]]) & 0x07ff;
                    if address & 0x80 != 0 {
                        out.bulk_in = address;
                        out.mps_in = mps;
                        last_endpoint = 2;
                    } else {
                        out.bulk_out = address;
                        out.mps_out = mps;
                        last_endpoint = 1;
                    }
                }
            }
            0x30 if len >= 6 && storage_interface => {
                let burst = bytes[offset + 2];
                if last_endpoint == 1 {
                    out.burst_out = burst;
                } else if last_endpoint == 2 {
                    out.burst_in = burst;
                }
            }
            _ => {}
        }
        offset += len;
    }

    if out.bulk_in != 0 && out.bulk_out != 0 && out.mps_in != 0 && out.mps_out != 0 {
        Some(out)
    } else {
        None
    }
}

fn fill_blackbox_bulk_context(
    ctx: usize,
    ep_type: u8,
    max_packet: u16,
    max_burst: u8,
    ring: &ProducerRing,
) {
    fill_endpoint_context(ctx, ep_type, max_packet, 0, ring);
    unsafe {
        let dw1 = ctx_r32(ctx, 1);
        ctx_w32(ctx, 1, (dw1 & !(0xff << 8)) | ((max_burst as u32) << 8));
    }
}

fn free_blackbox_storage(storage: &BlackboxStorage) {
    memory::free_dma(storage.ring_in.phys, RING_BYTES);
    memory::free_dma(storage.ring_out.phys, RING_BYTES);
    memory::free_dma(storage.buffer_phys, storage.buffer_len);
}

fn blackbox_bulk_transfer(
    controller: &mut Controller,
    storage: &mut BlackboxStorage,
    input: bool,
    offset: usize,
    len: usize,
) -> Result<usize, &'static str> {
    if len == 0 {
        return Ok(0);
    }
    if offset.saturating_add(len) > storage.buffer_len {
        return Err("blackbox-dma-buffer-too-small");
    }
    let (dci, ring) = if input {
        (storage.dci_in, &mut storage.ring_in)
    } else {
        (storage.dci_out, &mut storage.ring_out)
    };

    ring_push(
        ring,
        Trb {
            parameter: storage.buffer_phys + offset as u64,
            status: len as u32,
            control: (TRB_NORMAL << 10) | TRB_IOC | if input { TRB_ISP } else { 0 },
        },
    );
    ring_doorbell(controller, storage.slot_id, dci);

    // LE BUDGET DU RUNTIME, PAS CELUI DE L'ENUMERATION.
    //
    // Ce transfert tient le verrou du pilote. Lui laisser une demi-seconde,
    // c'est faire sauter au clavier cinq cents tours de scrutation pour une
    // cle qui ne repond pas. Voir `budget_bot`.
    let event = wait_event_budget(
        controller,
        EVT_TRANSFER,
        Some(storage.slot_id),
        Some(dci),
        budget_bot(),
    )
    .ok_or("blackbox-bulk-timeout")?;
    let cc = completion_code(event.status);
    if cc == CC_STALL {
        return Err("blackbox-bulk-stall");
    }
    if cc != CC_SUCCESS && cc != CC_SHORT_PACKET {
        return Err("blackbox-bulk-status");
    }
    let residual = (event.status & 0x00ff_ffff) as usize;
    Ok(len.saturating_sub(residual.min(len)))
}

/// Une commande BOT sur le support de l'enregistreur.
///
/// `longueur` est la taille de la phase de donnees, DEJA en place dans la
/// zone de donnees du tampon DMA pour une ecriture, a y relire pour une
/// lecture. Le lot final ne passe donc pas par un tampon intermediaire.
fn blackbox_bot_en_place(
    controller: &mut Controller,
    storage: &mut BlackboxStorage,
    cdb: &[u8],
    longueur: usize,
    data_in: bool,
) -> Result<usize, &'static str> {
    use crate::drivers::reprise_bot::{Incident, Phase};

    if longueur > BLACKBOX_DONNEES_MAX {
        return Err("blackbox-bot-data-too-large");
    }
    // AUCUNE ENTREE-SORTIE AVANT QUE LA REPRISE N'AIT EU LIEU.
    //
    // C'est la regle qui rend les budgets courts surs : une commande qui part
    // sur un transport dont la precedente a expire prendrait l'achevement
    // tardif du TD abandonne pour le sien.
    if !TRANSPORT_BOT.autorise_es() {
        return Err("blackbox-bot-refuse");
    }

    storage.tag = storage.tag.wrapping_add(1).max(1);
    let tag = storage.tag;
    let mut cbw = [0u8; stockage_blackbox::CBW_OCTETS];
    if !stockage_blackbox::encode_cbw(&mut cbw, tag, longueur as u32, data_in, 0, cdb) {
        return Err("blackbox-cbw");
    }

    unsafe {
        copy_nonoverlapping(cbw.as_ptr(), storage.buffer_virt as *mut u8, cbw.len());
    }
    TRANSPORT_BOT.entre_en_phase(Phase::Commande);
    match blackbox_bulk_transfer(controller, storage, false, 0, cbw.len()) {
        Ok(n) if n == cbw.len() => {}
        Ok(_) => return Err(blackbox_incident(storage, Incident::StatutInvalide, "blackbox-cbw-short")),
        Err("blackbox-bulk-timeout") => {
            return Err(blackbox_incident(storage, Incident::Echeance, "blackbox-cbw-timeout"))
        }
        Err(e) => return Err(blackbox_incident(storage, Incident::PointArrete, e)),
    }

    let mut actual = 0usize;
    if longueur != 0 {
        TRANSPORT_BOT.entre_en_phase(Phase::Donnees);
        if injecte(INJECTE_ECHEANCE_DONNEES) {
            return Err(blackbox_incident(storage, Incident::Echeance, "blackbox-injection-donnees"));
        }
        match blackbox_bulk_transfer(
            controller,
            storage,
            data_in,
            BLACKBOX_ENVELOPPE_OCTETS,
            longueur,
        ) {
            Ok(n) => actual = n,
            Err("blackbox-bulk-timeout") => {
                return Err(blackbox_incident(storage, Incident::Echeance, "blackbox-data-timeout"))
            }
            // UN STALL EN PHASE DE DONNEES EST PREVU PAR LE PROTOCOLE.
            //
            // # Ce que le traiter comme une panne a coute
            //
            // La session physique du 17 septembre compte cinquante echecs
            // d'ecriture, tous `blackbox-bulk-stall`, et autant de
            // reinitialisations de classe complete -- alors que la clef ne
            // demandait qu'a etre debloquee. C'est ce cout-la qui a empeche le
            // vidage final d'aller au bout : `draine=0`, pas de marque de FIN,
            // vingt-huit enregistrements laisses en memoire.
            //
            // La specification Bulk-Only est explicite : le peripherique qui
            // refuse la phase de donnees ARRETE son point et attend qu'on
            // vienne lire son CSW. Debloquer le point puis lire le statut est
            // la reponse ; la reinitialisation de classe est ce qu'on garde
            // pour un desaccord de phase, qui est autre chose.
            //
            // Le chemin du volume traitait deja ce cas correctement. Seul
            // celui de l'enregistreur ne le faisait pas.
            Err("blackbox-bulk-stall") => {
                BLACKBOX_STALLS.fetch_add(1, Ordering::Relaxed);
                actual = 0;
                let slot = storage.slot_id as usize;
                let debloque = if slot < controller.devices.len() {
                    match controller.devices[slot].take() {
                        Some(mut device) => {
                            let ok =
                                blackbox_debloque_point(controller, &mut device, storage, data_in);
                            controller.devices[slot] = Some(device);
                            ok
                        }
                        None => false,
                    }
                } else {
                    false
                };
                if !debloque {
                    return Err(blackbox_incident(
                        storage,
                        Incident::PointArrete,
                        "blackbox-bulk-stall",
                    ));
                }
                // Le CSW reste a lire : c'est lui qui dit POURQUOI la clef a
                // refuse, et le laisser dans le tuyau desynchroniserait la
                // commande suivante.
            }
            Err(e) => return Err(blackbox_incident(storage, Incident::PointArrete, e)),
        }
    }

    unsafe {
        write_bytes(storage.buffer_virt as *mut u8, 0, stockage_blackbox::CSW_OCTETS);
    }
    TRANSPORT_BOT.entre_en_phase(Phase::Statut);
    if injecte(INJECTE_ECHEANCE_STATUT) {
        return Err(blackbox_incident(storage, Incident::Echeance, "blackbox-injection-statut"));
    }
    let got = match blackbox_bulk_transfer(
        controller,
        storage,
        true,
        0,
        stockage_blackbox::CSW_OCTETS,
    ) {
        Ok(n) => n,
        Err("blackbox-bulk-timeout") => {
            return Err(blackbox_incident(storage, Incident::Echeance, "blackbox-csw-timeout"))
        }
        Err(e) => return Err(blackbox_incident(storage, Incident::PointArrete, e)),
    };
    if got != stockage_blackbox::CSW_OCTETS {
        return Err(blackbox_incident(storage, Incident::StatutInvalide, "blackbox-csw-short"));
    }
    let csw_bytes = unsafe {
        core::slice::from_raw_parts(storage.buffer_virt as *const u8, stockage_blackbox::CSW_OCTETS)
    };
    let Some(csw) = stockage_blackbox::decode_csw(csw_bytes) else {
        return Err(blackbox_incident(storage, Incident::StatutInvalide, "blackbox-csw-invalid"));
    };
    if !stockage_blackbox::transfert_complet(&csw, tag) {
        if csw.statut == stockage_blackbox::StatutCsw::ErreurDePhase {
            return Err(blackbox_incident(storage, Incident::PhaseIncoherente, "blackbox-csw-phase"));
        }
        TRANSPORT_BOT.sort_de_phase();
        return Err("blackbox-csw-failure");
    }
    TRANSPORT_BOT.sort_de_phase();
    Ok(if longueur == 0 { 0 } else { actual })
}

/// Note un incident sur le transport et rend le motif, inchange.
///
/// Le passage en reprise a lieu ICI plutot que chez l'appelant pour qu'il n'y
/// ait aucun chemin d'erreur qui laisse le transport en `Pret`.
fn blackbox_incident(
    storage: &BlackboxStorage,
    quoi: crate::drivers::reprise_bot::Incident,
    motif: &'static str,
) -> &'static str {
    TRANSPORT_BOT.incident(
        quoi,
        storage.slot_id,
        storage.dci_out,
        crate::kernel::timer::monotonic_ns(),
    );
    motif
}

/// Compatibilite : une commande BOT dont les donnees vivent dans un tampon
/// de l'appelant. Utilisee par le bring-up, qui lit de petites structures.
fn blackbox_bot(
    controller: &mut Controller,
    storage: &mut BlackboxStorage,
    cdb: &[u8],
    data: &mut [u8],
    data_in: bool,
) -> Result<usize, &'static str> {
    if data.len() > BLACKBOX_DONNEES_MAX {
        return Err("blackbox-bot-data-too-large");
    }
    if !data_in && !data.is_empty() {
        unsafe {
            copy_nonoverlapping(
                data.as_ptr(),
                (storage.buffer_virt + BLACKBOX_ENVELOPPE_OCTETS) as *mut u8,
                data.len(),
            );
        }
    }
    let actual = blackbox_bot_en_place(controller, storage, cdb, data.len(), data_in)?;
    if data_in && actual != 0 {
        unsafe {
            copy_nonoverlapping(
                (storage.buffer_virt + BLACKBOX_ENVELOPPE_OCTETS) as *const u8,
                data.as_mut_ptr(),
                actual.min(data.len()),
            );
        }
    }
    Ok(actual)
}

fn blackbox_read_blocks(
    controller: &mut Controller,
    storage: &mut BlackboxStorage,
    lba: u64,
    blocks: u16,
    out: &mut [u8],
) -> Result<(), &'static str> {
    if storage.block_size != 512 || lba > u32::MAX as u64 {
        return Err("blackbox-read-address");
    }
    let bytes = blocks as usize * storage.block_size as usize;
    if out.len() != bytes || bytes > BLACKBOX_DONNEES_MAX {
        return Err("blackbox-read-size");
    }
    let cdb = stockage_blackbox::cdb_transfert_10(false, lba as u32, blocks);
    let got = blackbox_bot(controller, storage, &cdb, out, true)?;
    if got != bytes {
        return Err("blackbox-read-short");
    }
    Ok(())
}

fn blackbox_write_blocks(
    controller: &mut Controller,
    storage: &mut BlackboxStorage,
    lba: u64,
    blocks: u16,
    data: &mut [u8],
) -> Result<(), &'static str> {
    if storage.block_size != 512 || lba > u32::MAX as u64 {
        return Err("blackbox-write-address");
    }
    let bytes = blocks as usize * storage.block_size as usize;
    if data.len() != bytes || bytes > BLACKBOX_DONNEES_MAX {
        return Err("blackbox-write-size");
    }
    let cdb = stockage_blackbox::cdb_transfert_10(true, lba as u32, blocks);
    let got = blackbox_bot(controller, storage, &cdb, data, false)?;
    if got != bytes {
        return Err("blackbox-write-short");
    }
    Ok(())
}

fn blackbox_sync_cache(controller: &mut Controller, storage: &mut BlackboxStorage) -> bool {
    let cdb = [0x35u8, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let mut empty = [0u8; 0];
    let ok = blackbox_bot(controller, storage, &cdb, &mut empty, false).is_ok();
    if ok { storage.since_sync = 0; }
    ok
}

fn le_u32(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
}

fn le_u64(data: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        data[off], data[off + 1], data[off + 2], data[off + 3],
        data[off + 4], data[off + 5], data[off + 6], data[off + 7],
    ])
}

fn blackbox_partition_name_matches(entry: &[u8]) -> bool {
    if entry.len() < 128 {
        return false;
    }
    for (i, expected) in BLACKBOX_PARTITION_NAME.iter().copied().enumerate() {
        let off = 56 + i * 2;
        if off + 1 >= entry.len() || entry[off] != expected || entry[off + 1] != 0 {
            return false;
        }
    }
    let end = 56 + BLACKBOX_PARTITION_NAME.len() * 2;
    end + 1 < entry.len() && entry[end] == 0 && entry[end + 1] == 0
}

fn blackbox_find_partition(
    controller: &mut Controller,
    storage: &mut BlackboxStorage,
) -> Result<bool, &'static str> {
    let mut sector = [0u8; 512];
    blackbox_read_blocks(controller, storage, 1, 1, &mut sector)?;
    if &sector[0..8] != b"EFI PART" {
        return Ok(false);
    }
    let entries_lba = le_u64(&sector, 72);
    let entries = le_u32(&sector, 80) as usize;
    let entry_size = le_u32(&sector, 84) as usize;
    if entry_size < 128 || entry_size > 512 || 512 % entry_size != 0 {
        return Ok(false);
    }
    let per_sector = 512 / entry_size;
    let to_scan = entries.min(BLACKBOX_SCAN_GPT_ENTRIES);
    let mut cached_sector = usize::MAX;

    for index in 0..to_scan {
        let sector_index = index / per_sector;
        if cached_sector != sector_index {
            blackbox_read_blocks(
                controller,
                storage,
                entries_lba.saturating_add(sector_index as u64),
                1,
                &mut sector,
            )?;
            cached_sector = sector_index;
        }
        let off = (index % per_sector) * entry_size;
        let entry = &sector[off..off + entry_size];
        if entry[..16].iter().all(|b| *b == 0) {
            continue;
        }
        if !blackbox_partition_name_matches(entry) {
            continue;
        }
        let first = le_u64(entry, 32);
        let last = le_u64(entry, 40);
        if first == 0 || last < first {
            return Ok(false);
        }
        let blocks = last - first + 1;
        let record_blocks = (BLACKBOX_RECORD_BYTES / 512) as u64;
        if blocks < record_blocks.saturating_mul(64) {
            return Ok(false);
        }
        storage.partition_first = first;
        storage.partition_blocks = blocks;
        return Ok(true);
    }
    Ok(false)
}

fn configure_blackbox_storage(
    controller: &mut Controller,
    device: &mut Device,
    descriptor: BlackboxStorageDescriptor,
) -> Result<bool, &'static str> {
    if controller.blackbox_storage.is_some() {
        return Ok(false);
    }

    let ep_out = descriptor.bulk_out & 0x0f;
    let ep_in = descriptor.bulk_in & 0x0f;
    if ep_out == 0 || ep_in == 0 {
        return Ok(false);
    }
    let dci_out = ep_out.saturating_mul(2);
    let dci_in = ep_in.saturating_mul(2).saturating_add(1);
    if dci_out >= 32 || dci_in >= 32 {
        return Ok(false);
    }

    let ring_out = alloc_producer_ring().ok_or("blackbox-ring-out")?;
    let ring_in = match alloc_producer_ring() {
        Some(r) => r,
        None => {
            memory::free_dma(ring_out.phys, RING_BYTES);
            return Err("blackbox-ring-in");
        }
    };
    let (buffer_phys, buffer_virt) = match alloc_zeroed(BLACKBOX_DMA_BYTES) {
        Some(v) => v,
        None => {
            memory::free_dma(ring_out.phys, RING_BYTES);
            memory::free_dma(ring_in.phys, RING_BYTES);
            return Err("blackbox-buffer");
        }
    };

    clear_input(device);
    let slot_out = context_ptr(device.out_ctx_virt, controller.context_size, 0);
    let slot_in = context_ptr(device.in_ctx_virt, controller.context_size, 1);
    unsafe {
        copy_nonoverlapping(slot_out as *const u8, slot_in as *mut u8, controller.context_size);
    }

    let out_ctx = context_ptr(device.in_ctx_virt, controller.context_size, dci_out as usize + 1);
    let in_ctx = context_ptr(device.in_ctx_virt, controller.context_size, dci_in as usize + 1);
    fill_blackbox_bulk_context(out_ctx, 2, descriptor.mps_out, descriptor.burst_out, &ring_out);
    fill_blackbox_bulk_context(in_ctx, 6, descriptor.mps_in, descriptor.burst_in, &ring_in);

    let add_flags = 1u32 | (1u32 << dci_out) | (1u32 << dci_in);
    let highest_dci = dci_out.max(dci_in);
    unsafe {
        write_volatile((device.in_ctx_virt + 4) as *mut u32, add_flags);
        let dw0 = ctx_r32(slot_in, 0);
        ctx_w32(slot_in, 0, (dw0 & !(0x1f << 27)) | ((highest_dci as u32) << 27));
    }

    if let Err(error) = command(controller, device.in_ctx_phys, CMD_CONFIGURE_ENDPOINT, device.slot_id) {
        memory::free_dma(ring_out.phys, RING_BYTES);
        memory::free_dma(ring_in.phys, RING_BYTES);
        memory::free_dma(buffer_phys, BLACKBOX_DMA_BYTES);
        return Err(error);
    }

    let mut storage = BlackboxStorage {
        slot_id: device.slot_id,
        interface: descriptor.interface,
        dci_in,
        dci_out,
        ring_in,
        ring_out,
        buffer_phys,
        buffer_virt,
        buffer_len: BLACKBOX_DMA_BYTES,
        tag: 0,
        block_size: 0,
        blocks: 0,
        partition_first: 0,
        partition_blocks: 0,
        since_sync: 0,
        disabled: false,
    };

    let mut empty = [0u8; 0];
    let tur = stockage_blackbox::cdb_test_unite_prete();
    let _ = blackbox_bot(controller, &mut storage, &tur, &mut empty, true);

    let mut inquiry = [0u8; 36];
    let inquiry_cdb = stockage_blackbox::cdb_interroge(inquiry.len() as u8);
    if blackbox_bot(controller, &mut storage, &inquiry_cdb, &mut inquiry, true).is_err() {
        free_blackbox_storage(&storage);
        return Ok(false);
    }
    let Some(info) = stockage_blackbox::decode_interrogation(&inquiry) else {
        free_blackbox_storage(&storage);
        return Ok(false);
    };
    if !stockage_blackbox::support_utilisable(&info) {
        free_blackbox_storage(&storage);
        return Ok(false);
    }

    let mut cap = [0u8; 8];
    let cap_cdb = stockage_blackbox::cdb_lit_capacite();
    if blackbox_bot(controller, &mut storage, &cap_cdb, &mut cap, true).is_err() {
        free_blackbox_storage(&storage);
        return Ok(false);
    }
    let Some(capacity) = stockage_blackbox::decode_capacite(&cap) else {
        free_blackbox_storage(&storage);
        return Ok(false);
    };
    storage.block_size = capacity.taille_bloc;
    storage.blocks = capacity.blocs();
    if storage.block_size != 512 {
        free_blackbox_storage(&storage);
        return Ok(false);
    }

    match blackbox_find_partition(controller, &mut storage) {
        Ok(true) => {
            crate::serial_println!(
                "BOUCHAUD_BLACKBOX_USB_READY slot={} if={} blocks={} partition_first={} partition_blocks={}",
                storage.slot_id,
                storage.interface,
                storage.blocks,
                storage.partition_first,
                storage.partition_blocks,
            );
            controller.blackbox_storage = Some(storage);
            // UN SUPPORT NEUF REPART D'UN TRANSPORT NEUF.
            //
            // Sans cela, un support retire puis rebranche resterait hors
            // service pour le reste de la session -- et un debranchement
            // accidentel couterait l'enregistreur jusqu'au redemarrage.
            TRANSPORT_BOT.remet_a_neuf();
            BLACKBOX_STORAGE_READY.store(true, Ordering::Release);
            Ok(true)
        }
        _ => {
            free_blackbox_storage(&storage);
            Ok(false)
        }
    }
}

fn retire_blackbox_storage(controller: &mut Controller, slot: u8) {
    let matches = controller.blackbox_storage.as_ref().map(|s| s.slot_id == slot).unwrap_or(false);
    if matches {
        if let Some(storage) = controller.blackbox_storage.take() {
            free_blackbox_storage(&storage);
        }
        BLACKBOX_STORAGE_READY.store(false, Ordering::Release);
    }
}

/// Le code qui voyage dans l'archive pour chaque motif d'echec.
///
/// # Ce que l'absence d'un code a coute
///
/// L'ecran d'extinction de la session physique affichait `err=0xff` : le
/// motif etait tombe dans le `_` du filtre. Les motifs introduits avec la
/// reprise BOT -- et le STALL, qui est le seul a s'etre produit -- n'avaient
/// pas ete ajoutes ici. Le chiffre le plus important de l'ecran ne designait
/// donc rien, et il a fallu retrouver la reponse dans le journal serie.
///
/// Un code par motif, et une garde qui verifie qu'il n'en manque aucun :
/// c'est la seule facon de ne pas refaire ce trou au prochain motif ajoute.
fn blackbox_error_code(error: &'static str) -> u64 {
    match error {
        // Transport : ce que le materiel a repondu.
        "blackbox-bulk-timeout" => 1,
        "blackbox-bulk-status" => 2,
        "blackbox-cbw-short" => 3,
        "blackbox-csw-short" => 4,
        "blackbox-csw-invalid" => 5,
        "blackbox-csw-failure" => 6,
        "blackbox-write-short" => 7,
        "blackbox-write-address" => 8,
        "blackbox-write-size" => 9,
        "blackbox-bot-data-too-large" => 10,
        "blackbox-cbw" => 11,
        "blackbox-disabled-or-payload" => 12,
        "blackbox-no-slots" => 13,
        // Ajoutes avec la reprise BOT. Leur absence ici est ce qui a rendu
        // `err=0xff` illisible sur l'ecran d'extinction physique.
        "blackbox-bulk-stall" => 14,
        "blackbox-bot-refuse" => 15,
        "blackbox-cbw-timeout" => 16,
        "blackbox-data-timeout" => 17,
        "blackbox-csw-timeout" => 18,
        "blackbox-csw-phase" => 19,
        "blackbox-dma-buffer-too-small" => 20,
        // Lecture : le meme transport, dans l'autre sens.
        "blackbox-read-address" => 21,
        "blackbox-read-short" => 22,
        "blackbox-read-size" => 23,
        // Configuration : le support n'a jamais pu etre arme.
        "blackbox-ring-in" => 24,
        "blackbox-ring-out" => 25,
        "blackbox-buffer" => 26,
        // Pannes artificielles du banc. Elles portent un code pour qu'une
        // archive de banc ne se lise pas comme une archive de panne reelle.
        "blackbox-injection-donnees" => 30,
        "blackbox-injection-statut" => 31,
        _ => 255,
    }
}


/// Ecrit un lot d'enregistrements CONTIGUS, deja formates dans la zone de
/// donnees du tampon DMA.
///
/// Une commande BOT, c'est trois transferts et deux attentes, quelle que soit
/// la quantite de donnees. Poser seize enregistrements en une commande au lieu
/// de seize paie le protocole une fois au lieu de seize -- et c'est ce qui
/// fait tenir un vidage final de huit mebioctets dans le budget d'un ecran
/// d'extinction.
fn blackbox_write_lot(
    controller: &mut Controller,
    storage: &mut BlackboxStorage,
    lba: u64,
    records: usize,
) -> Result<(), &'static str> {
    if records == 0 {
        return Ok(());
    }
    if storage.block_size != 512 || lba > u32::MAX as u64 {
        return Err("blackbox-write-address");
    }
    let blocks = records * (BLACKBOX_RECORD_BYTES / 512);
    let bytes = blocks * 512;
    if bytes > BLACKBOX_DONNEES_MAX || blocks > u16::MAX as usize {
        return Err("blackbox-write-size");
    }
    let cdb = stockage_blackbox::cdb_transfert_10(true, lba as u32, blocks as u16);
    let got = blackbox_bot_en_place(controller, storage, &cdb, bytes, false)?;
    if got != bytes {
        return Err("blackbox-write-short");
    }
    BLACKBOX_WRITES.fetch_add(records as u64, Ordering::Relaxed);
    storage.since_sync = storage.since_sync.saturating_add(records as u32);
    Ok(())
}

// BOUCHAUD_C72_SUPPORT_COUPE
//
// Le support disparait-il artificiellement ?
//
// Les trois injections BOT sont CONSOMMEES a la premiere occasion : elles
// fabriquent une panne transitoire, que le chemin d'extinction rattrape par
// ses reprises. Le banc du scenario G l'a montre -- sabotage arme, et
// pourtant marque FIN posee, archive COMPLETE.
//
// Or ce qu'on doit prouver n'est pas la reprise : c'est la survie du
// checkpoint quand le vidage final echoue VRAIMENT. Il faut donc une panne
// qui TIENNE, et la plus fidele au cas physique est la plus simple : la cle
// ne repond plus.
//
// Armee seulement par le harnais de banc, jamais par defaut.
static SUPPORT_COUPE: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// Coupe le support pour de bon. Sans retour : c'est une panne, pas un test
/// de reprise.
pub fn coupe_le_support() {
    SUPPORT_COUPE.store(true, Ordering::Release);
}

pub fn blackbox_storage_ready() -> bool {
    if SUPPORT_COUPE.load(Ordering::Acquire) {
        return false;
    }
    BLACKBOX_STORAGE_READY.load(Ordering::Acquire)
}

pub fn blackbox_storage_counters() -> (u64, u64) {
    (
        BLACKBOX_WRITES.load(Ordering::Relaxed),
        BLACKBOX_FAILURES.load(Ordering::Relaxed),
    )
}

pub fn blackbox_storage_extended_counters() -> (u64, u64, u64, u64, u64, u64) {
    (
        BLACKBOX_WRITES.load(Ordering::Relaxed),
        BLACKBOX_FAILURES.load(Ordering::Relaxed),
        BLACKBOX_CONSECUTIVE_FAILURES.load(Ordering::Relaxed),
        BLACKBOX_LAST_ERROR.load(Ordering::Relaxed),
        BLACKBOX_BUSY_SKIPS.load(Ordering::Relaxed),
        BLACKBOX_LAST_OK_NS.load(Ordering::Relaxed),
    )
}

/// Vide un lot d'enregistrements du tambour RAM vers le support.
///
/// `fournit` remplit une zone de quatre kibioctets avec l'image COMPLETE d'un
/// enregistrement -- en-tete et charge utile -- et rend son numero. Rendre
/// `None` termine le lot. La zone qu'elle recoit est DIRECTEMENT le tampon
/// DMA : un vidage final deplace jusqu'a huit mebioctets, et une copie
/// intermediaire se paierait a chaque enregistrement.
///
/// Rend le nombre d'enregistrements effectivement poses.
///
/// # Ce que cette fonction remplace
///
/// `blackbox_append_record`, qui ecrivait UN enregistrement par appel, verrou
/// pris et rendu a chaque fois, depuis le chemin chaud de l'enregistreur.
/// C'est ce chemin-la qui est supprime : le pilote n'est plus sollicite que
/// par le vidage, et le vidage n'a lieu qu'a l'extinction.
pub fn blackbox_vidange_lot(
    maximum: usize,
    mut fournit: impl FnMut(&mut [u8]) -> Option<u64>,
) -> usize {
    if !BLACKBOX_STORAGE_READY.load(Ordering::Acquire) {
        return 0;
    }
    let maximum = lot_courant(maximum.min(BLACKBOX_RECORDS_PAR_LOT));
    if maximum == 0 {
        return 0;
    }
    // LE VIDAGE ATTEND, PARCE QU'IL PEUT ATTENDRE -- ET QUE LUI SEUL LE PEUT.
    //
    // Une prise INSTANTANEE etait juste tant que le vidage avait lieu quatre
    // fois par seconde : renoncer coutait un quart de seconde. Depuis V3, il
    // n'a plus lieu qu'a l'extinction, et renoncer y coute TOUTE LA TRACE.
    //
    // Le banc l'a montre net : le fil de charge lisait encore le volume
    // pendant l'extinction, le verrou n'etait jamais libre a l'instant precis
    // ou le vidage passait, `sautes_occupe=21`, `ecritures=0` -- mille sept
    // cent vingt-cinq enregistrements parfaitement poses en RAM, et pas un
    // seul sur la cle.
    //
    // La reclamation d'equite part AVANT l'attente : c'est elle qui fait
    // ceder `avec_le_pilote_usb`, et sans elle une lecture soutenue garde le
    // verrou presque en continu.
    super::xhci_active::enregistreur_a_saute(crate::kernel::timer::monotonic_ns());
    let Some(_jeton) = attends_le_pilote(Proprietaire::VidageBlackbox, ATTENTE_VIDAGE_NS) else {
        BLACKBOX_BUSY_SKIPS.fetch_add(1, Ordering::Relaxed);
        return 0;
    };
    super::xhci_active::enregistreur_a_reussi();

    let mut poses = 0usize;
    unsafe {
        #[allow(static_mut_refs)]
        let Some(runtime) = RUNTIME.as_mut() else {
            return 0;
        };
        for controller in runtime.controllers.iter_mut() {
            let Some(mut storage) = controller.blackbox_storage.take() else {
                continue;
            };
            poses = blackbox_vidange_un_support(controller, &mut storage, maximum, &mut fournit);
            controller.blackbox_storage = Some(storage);
            // UN SEUL SUPPORT EST SOLLICITE PAR APPEL, MEME S'IL ECHOUE.
            //
            // Passer au controleur suivant apres un echec ferait redistribuer
            // le rappel : les enregistrements deja pris par le premier support
            // seraient consommes sans etre ecrits, et le curseur de vidage ne
            // pourrait plus dire lesquels manquent. Un support qui refuse
            // repasse au tour suivant, entier.
            break;
        }
    }
    poses
}

/// Reprend le transport du support de l'enregistreur.
///
/// Meme procedure que pour le volume : reinitialisation de classe, PUIS
/// deblocage des deux points -- l'inverse ne marche pas, car la
/// reinitialisation remet le peripherique en attente d'un CBW neuf et c'est
/// seulement apres qu'il accepte de voir ses points debloques.
///
/// Reposer le pointeur de file de chaque anneau est ce qui JETTE le TD qu'une
/// echeance avait laisse : sans cela, l'achevement tardif reviendrait et
/// serait pris pour la reponse de la commande suivante.
fn blackbox_reprend_le_transport(
    controller: &mut Controller,
    storage: &mut BlackboxStorage,
) -> bool {
    use crate::drivers::reprise_bot::Etat;
    if TRANSPORT_BOT.etat() == Etat::HorsService {
        return false;
    }
    if !TRANSPORT_BOT.commence_reprise() {
        return TRANSPORT_BOT.etat() == Etat::Pret;
    }
    let slot = storage.slot_id as usize;
    if slot >= controller.devices.len() {
        TRANSPORT_BOT.reprise_echouee();
        return false;
    }
    let Some(mut device) = controller.devices[slot].take() else {
        TRANSPORT_BOT.reprise_echouee();
        return false;
    };

    // 0x21 : hote vers peripherique, type classe, destinataire interface.
    // 0xFF : Bulk-Only Mass Storage Reset.
    let setup = setup_packet(0x21, 0xFF, 0, storage.interface as u16, 0);
    let mut ok = control_transfer(
        controller,
        &mut device,
        setup,
        0,
        false,
        BUDGET_ENUMERATION_NS,
    )
    .is_ok();
    ok &= blackbox_debloque_point(controller, &mut device, storage, true);
    ok &= blackbox_debloque_point(controller, &mut device, storage, false);
    controller.devices[slot] = Some(device);

    if ok {
        TRANSPORT_BOT.reprise_reussie();
        crate::serial_println!("BOUCHAUD_BLACKBOX_BOT_REPRISE_OK slot={}", storage.slot_id);
        return true;
    }
    let etat = TRANSPORT_BOT.reprise_echouee();
    crate::serial_println!(
        "BOUCHAUD_BLACKBOX_BOT_REPRISE_ECHEC slot={} etat={}",
        storage.slot_id, etat.nom(),
    );
    false
}

/// Remet un point bulk de l'enregistreur dans un etat coherent.
///
/// # La commande depend de l'ETAT, et c'est tout le correctif
///
/// `Reset Endpoint` ne s'applique qu'a un point ARRETE. Sur un point encore en
/// marche -- ce qu'est un point dont le transfert vient d'expirer, puisque le
/// controleur n'a rien rendu --, il repond « Context State Error » et la
/// reprise echoue. Le banc l'a montre net : trois tentatives, trois echecs,
/// transport hors service, et pas un seul enregistrement pose.
///
/// Lire l'etat AVANT de choisir la commande est la difference entre une
/// reprise qui repare et une reprise qui condamne.
fn blackbox_debloque_point(
    controller: &mut Controller,
    device: &mut Device,
    storage: &mut BlackboxStorage,
    entree: bool,
) -> bool {
    let dci = if entree { storage.dci_in } else { storage.dci_out };
    let slot = storage.slot_id;
    let etat = etat_du_point(controller, device, dci);

    match etat {
        EP_ETAT_HALTED => {
            let controle =
                (CMD_RESET_ENDPOINT << 10) | ((dci as u32) << 16) | ((slot as u32) << 24);
            if command_raw(controller, 0, controle).is_err() {
                return false;
            }
        }
        EP_ETAT_RUNNING => {
            // LE TD ABANDONNE SE VIDE ICI, ET NULLE PART AILLEURS.
            //
            // Sans cet arret, le peripherique peut achever plus tard le
            // transfert qu'on a abandonne, et son evenement serait pris pour
            // la reponse de la commande suivante -- qui rendrait alors le
            // contenu d'un autre secteur avec un statut valide.
            let controle =
                (CMD_STOP_ENDPOINT << 10) | ((dci as u32) << 16) | ((slot as u32) << 24);
            if command_raw(controller, 0, controle).is_err() {
                return false;
            }
        }
        // Desactive ou deja arrete : rien a defaire, le pointeur de file
        // suffit.
        _ => {}
    }

    // L'anneau repart de son debut. Les TRB deja consommes -- et celui qu'une
    // echeance a laisse -- portent l'ancien cycle et ne seront pas repris.
    let ring = if entree {
        &mut storage.ring_in
    } else {
        &mut storage.ring_out
    };
    ring.index = 0;
    ring.cycle = 1;
    unsafe { prepare_link(ring, 1) };
    let phys = ring.phys;
    let controle = (CMD_SET_TR_DEQUEUE << 10) | ((dci as u32) << 16) | ((slot as u32) << 24);
    if command_raw(controller, phys | 1, controle).is_err() {
        return false;
    }

    // CLEAR_FEATURE(ENDPOINT_HALT) n'a de sens que si le PERIPHERIQUE tenait
    // le point pour bloque. L'emettre sur un point qui ne l'etait pas est au
    // mieux inutile, au pire refuse -- et ce refus ferait echouer une reprise
    // qui avait deja abouti.
    if etat != EP_ETAT_HALTED {
        return true;
    }
    // Destinataire « point de terminaison », index = adresse du point. Le
    // numero se deduit du DCI : impair = entree, numero = `dci / 2`.
    let adresse = if entree { 0x80 | (dci >> 1) } else { dci >> 1 };
    let setup = setup_packet(0x02, 1, 0, adresse as u16, 0);
    control_transfer(controller, device, setup, 0, false, BUDGET_ENUMERATION_NS).is_ok()
}

fn blackbox_vidange_un_support(
    controller: &mut Controller,
    storage: &mut BlackboxStorage,
    maximum: usize,
    fournit: &mut impl FnMut(&mut [u8]) -> Option<u64>,
) -> usize {
    let record_blocks = (BLACKBOX_RECORD_BYTES / 512) as u64;
    let places = storage.partition_blocks / record_blocks;
    if places == 0 || storage.disabled {
        return 0;
    }
    // Le transport doit etre coherent AVANT le premier octet du lot : le
    // formatage des enregistrements ecrit dans le tampon DMA, et le faire
    // pour une commande qui sera refusee couterait seize copies pour rien.
    if !TRANSPORT_BOT.autorise_es() && !blackbox_reprend_le_transport(controller, storage) {
        return 0;
    }

    let base = storage.buffer_virt + BLACKBOX_ENVELOPPE_OCTETS;
    let mut lot = 0usize;
    let mut place_debut = 0u64;
    let mut place_suivante = 0u64;

    loop {
        if lot >= maximum {
            break;
        }
        let zone = unsafe {
            core::slice::from_raw_parts_mut(
                (base + lot * BLACKBOX_RECORD_BYTES) as *mut u8,
                BLACKBOX_RECORD_BYTES,
            )
        };
        let Some(seq) = fournit(zone) else { break };
        let place = seq % places;
        if lot == 0 {
            place_debut = place;
            place_suivante = place + 1;
            lot = 1;
            continue;
        }
        // LE SEUL DECOUPAGE IMPOSE : LE TOUR DE L'ANNEAU DU SUPPORT.
        //
        // Des numeros consecutifs donnent des emplacements consecutifs, sauf
        // quand `numero % places` repasse par zero. Ecrire le lot entier au
        // premier LBA ecraserait alors des enregistrements qui n'ont rien a
        // voir -- et la relecture y verrait des sommes de controle valides sur
        // des charges utiles depareillees, ce qui est pire qu'un trou.
        if place != place_suivante {
            let lba = storage
                .partition_first
                .saturating_add(place_debut.saturating_mul(record_blocks));
            if !blackbox_ecris_lot_avec_reprise(controller, storage, lba, lot) {
                return 0;
            }
            // L'enregistrement qu'on vient de recevoir ouvre le lot suivant.
            unsafe {
                core::ptr::copy(
                    (base + lot * BLACKBOX_RECORD_BYTES) as *const u8,
                    base as *mut u8,
                    BLACKBOX_RECORD_BYTES,
                );
            }
            let lba = storage
                .partition_first
                .saturating_add(place.saturating_mul(record_blocks));
            if !blackbox_ecris_lot_avec_reprise(controller, storage, lba, 1) {
                return lot;
            }
            return lot + 1;
        }
        place_suivante = place + 1;
        lot += 1;
    }

    if lot == 0 {
        return 0;
    }
    let lba = storage
        .partition_first
        .saturating_add(place_debut.saturating_mul(record_blocks));
    if !blackbox_ecris_lot_avec_reprise(controller, storage, lba, lot) {
        return 0;
    }
    if storage.since_sync >= BLACKBOX_SYNC_EVERY {
        blackbox_sync_cache(controller, storage);
    }
    lot
}

/// Ecrit un lot, et le rejoue UNE FOIS si une reprise a rendu le materiel
/// coherent.
///
/// # Pourquoi le rejeu appartient au vidage, et pas a l'appelant
///
/// Borner la latence d'un transfert, c'est abandonner plus tot -- donc plus
/// souvent. Si chaque abandon remontait en erreur, un lot rate coutait seize
/// enregistrements ET arretait le vidage : le banc a mesure mille cent
/// trente-six enregistrements poses sur mille quatre cent cinquante-cinq, sans
/// marque de fin, c'est-a-dire une archive indistinguable d'une coupure.
///
/// La reprise a jete le TD abandonne et remis le peripherique en attente d'un
/// CBW neuf. Rejouer est donc sur, et borne a une seule fois : un support qui
/// repond systematiquement de travers ne doit pas tenir le verrou sans fin.
fn blackbox_ecris_lot_avec_reprise(
    controller: &mut Controller,
    storage: &mut BlackboxStorage,
    lba: u64,
    records: usize,
) -> bool {
    // SEUL UN REFUS DE TAILLE FAIT RECULER LE LOT.
    //
    // # Le defaut que ceci corrige, attrape par le banc
    //
    // La premiere version reculait sur TOUT echec, echeance comprise. Or une
    // echeance ne dit rien de la taille : elle dit que le peripherique n'a pas
    // repondu a temps. Sous QEMU, ou seize processeurs virtuels en attente
    // active se font deordonnancer et produisent treize cents echeances, le
    // lot tombait a un enregistrement en quelques secondes et n'y remontait
    // jamais. Le vidage devenait seize fois plus lent et n'allait plus au
    // bout : vingt-huit secondes d'archive au lieu de cent vingt-neuf, pas de
    // marque de FIN, journal serie vide.
    //
    // Les deux machines le disent chacune a leur facon, et c'est ce qui rend
    // la distinction incontournable : la TRIGKEY a cinquante STALL et ZERO
    // echeance ; QEMU a treize cents echeances et ZERO STALL. Deux signaux
    // differents, deux reponses differentes.
    let avant = BLACKBOX_STALLS.load(Ordering::Relaxed);
    let resultat = blackbox_note(blackbox_write_lot(controller, storage, lba, records));
    let refus_de_taille = BLACKBOX_STALLS.load(Ordering::Relaxed) != avant;
    if resultat.is_ok() {
        // Un STALL suivi d'un deblocage rend une commande REUSSIE, mais il a
        // coute un aller-retour de controle. C'est le refus qu'il faut
        // compter, pas l'echec.
        if refus_de_taille {
            lot_recule();
        } else {
            lot_avance();
        }
        return true;
    }
    if refus_de_taille {
        lot_recule();
    }
    if !TRANSPORT_BOT.autorise_es() && !blackbox_reprend_le_transport(controller, storage) {
        return false;
    }
    blackbox_note(blackbox_write_lot(controller, storage, lba, records)).is_ok()
}

/// Compte l'issue d'une ecriture et la rend telle quelle.
fn blackbox_note(resultat: Result<(), &'static str>) -> Result<(), &'static str> {
    match resultat {
        Ok(()) => {
            BLACKBOX_CONSECUTIVE_FAILURES.store(0, Ordering::Relaxed);
            BLACKBOX_LAST_ERROR.store(0, Ordering::Relaxed);
            BLACKBOX_LAST_OK_NS.store(
                crate::kernel::timer::monotonic_ns(),
                Ordering::Relaxed,
            );
            Ok(())
        }
        Err(erreur) => {
            let total = BLACKBOX_FAILURES.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
            let consecutif = BLACKBOX_CONSECUTIVE_FAILURES
                .fetch_add(1, Ordering::Relaxed)
                .wrapping_add(1);
            let code = blackbox_error_code(erreur);
            BLACKBOX_LAST_ERROR.store(code, Ordering::Relaxed);
            // FAIL-OPEN : une erreur de transport ne coupe pas l'enregistreur.
            // V1 le coupait apres trois erreurs, ce qui rendait precisement la
            // panne invisible.
            if total <= 4 || total.is_power_of_two() {
                crate::serial_println!(
                    "BOUCHAUD_BLACKBOX_WRITE_FAIL total={} consecutive={} code={} error={}",
                    total, consecutif, code, erreur,
                );
            }
            Err(erreur)
        }
    }
}

/// Pourquoi une synchronisation n'a pas eu lieu.
///
/// Un `sync=0` sans cause est une panne muette : il peut vouloir dire « pas de
/// support », « pilote occupe » ou « la cle a refuse ». Ces trois-la ne se
/// soignent pas pareil, et le checkpoint tourne PENDANT la session -- ou le
/// pilote xHCI travaille -- alors que l'extinction, elle, a deja arrete les
/// services. Il fallait pouvoir les distinguer.
pub const SYNC_OK: u8 = 0;
pub const SYNC_SANS_SUPPORT: u8 = 1;
pub const SYNC_VERROU_REFUSE: u8 = 2;
pub const SYNC_PILOTE_KO: u8 = 3;

pub fn blackbox_force_sync() -> bool {
    blackbox_force_sync_detaille().0
}

/// Comme `blackbox_force_sync`, mais dit POURQUOI quand elle echoue.
pub fn blackbox_force_sync_detaille() -> (bool, u8) {
    if !BLACKBOX_STORAGE_READY.load(Ordering::Acquire) {
        return (false, SYNC_SANS_SUPPORT);
    }
    super::xhci_active::enregistreur_a_saute(crate::kernel::timer::monotonic_ns());
    let Some(_jeton) = attends_le_pilote(Proprietaire::VidageBlackbox, ATTENTE_VIDAGE_NS) else {
        return (false, SYNC_VERROU_REFUSE);
    };
    super::xhci_active::enregistreur_a_reussi();
    let mut synced = false;
    let mut failed = false;
    unsafe {
        if let Some(runtime) = RUNTIME.as_mut() {
            for controller in runtime.controllers.iter_mut() {
                let Some(mut storage) = controller.blackbox_storage.take() else { continue; };
                if blackbox_sync_cache(controller, &mut storage) { synced = true; } else { failed = true; }
                controller.blackbox_storage = Some(storage);
            }
        }
    }
    if synced && !failed {
        (true, SYNC_OK)
    } else {
        (false, SYNC_PILOTE_KO)
    }
}
