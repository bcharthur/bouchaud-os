//! Pilote ATA en mode PIO : lecture de secteurs sur disque IDE.
//!
//! Choix delibere de la voie la plus simple. Un pilote virtio-blk serait plus
//! rapide, mais reclame une file de descripteurs, des tampons DMA et une
//! negociation de fonctionnalites — beaucoup de surface pour un besoin qui est,
//! aujourd'hui, « lire une archive au demarrage ». L'ATA en PIO tient en un
//! fichier, ne depend d'aucune allocation, et fonctionne sur toute machine PC
//! depuis trente ans.
//!
//! La lenteur habituelle du PIO (un mot de 16 bits par acces au port) est
//! evitee par `rep insw`, qui transfere un secteur entier en une instruction :
//! l'emulateur la traite comme un bloc, pas comme 256 acces separes.
//!
//! ## Disques vus
//!
//! Le controleur primaire (ports 0x1F0-0x1F7) porte deux disques : le maitre,
//! qui est l'image de boot, et l'esclave, ou l'on attend l'archive userland.
//! Un second `-drive` passe a QEMU se presente exactement la.

use crate::arch::x86_64::ports::{inb, inw, outb};
use core::sync::atomic::{AtomicU64, Ordering};

/// Taille d'un secteur ATA.
pub const SECTOR_SIZE: usize = 512;

// Registres du controleur primaire.
const DATA: u16 = 0x1F0;
const ERROR: u16 = 0x1F1;
const SECTOR_COUNT: u16 = 0x1F2;
const LBA_LOW: u16 = 0x1F3;
const LBA_MID: u16 = 0x1F4;
const LBA_HIGH: u16 = 0x1F5;
const DRIVE_HEAD: u16 = 0x1F6;
const STATUS: u16 = 0x1F7;
const COMMAND: u16 = 0x1F7;
/// Registre de controle : lu, il donne le statut sans acquitter l'interruption.
const ALT_STATUS: u16 = 0x3F6;

// Bits du registre de statut.
const ST_ERR: u8 = 0x01;
const ST_DRQ: u8 = 0x08;
const ST_DF: u8 = 0x20;
const ST_READY: u8 = 0x40;
const ST_BUSY: u8 = 0x80;

const CMD_READ_SECTORS: u8 = 0x20;
const CMD_WRITE_SECTORS: u8 = 0x30;
/// Vide le cache d'ecriture du disque : sans elle, une image peut ne pas
/// porter ce qu'on vient d'y ecrire quand la machine s'arrete.
const CMD_FLUSH_CACHE: u8 = 0xE7;
const CMD_IDENTIFY: u8 = 0xEC;

/// Disque du controleur primaire.
#[derive(Clone, Copy, PartialEq)]
pub enum Drive {
    /// Maitre : l'image de boot.
    Master,
    /// Esclave : le disque de donnees.
    Slave,
}

impl Drive {
    /// Bits a poser dans le registre drive/head (mode LBA).
    fn select_bits(self) -> u8 {
        match self {
            Drive::Master => 0xE0,
            Drive::Slave => 0xF0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Drive::Master => "hda (image de boot)",
            Drive::Slave => "hdb (donnees)",
        }
    }
}

/// Nombre de secteurs de chaque disque, 0 s'il est absent.
static mut SECTORS: [u64; 2] = [0, 0];
static mut PROBED: bool = false;
static CONTROLLER: crate::kernel::sync::SpinLock<()> = crate::kernel::sync::SpinLock::new(());
static CONTROLLER_ACQUIRES: AtomicU64 = AtomicU64::new(0);
static CONTROLLER_WAIT_NS: AtomicU64 = AtomicU64::new(0);
static CONTROLLER_MAX_WAIT_NS: AtomicU64 = AtomicU64::new(0);

fn lock_controller() -> crate::kernel::sync::SpinLockGuard<'static, ()> {
    let start = crate::kernel::timer::monotonic_ns();
    let guard = CONTROLLER.lock();
    let waited = crate::kernel::timer::monotonic_ns().saturating_sub(start);
    CONTROLLER_ACQUIRES.fetch_add(1, Ordering::Relaxed);
    CONTROLLER_WAIT_NS.fetch_add(waited, Ordering::Relaxed);
    CONTROLLER_MAX_WAIT_NS.fetch_max(waited, Ordering::Relaxed);
    guard
}

pub fn contention_stats() -> (u64, u64, u64) {
    (
        CONTROLLER_ACQUIRES.load(Ordering::Relaxed),
        CONTROLLER_WAIT_NS.load(Ordering::Relaxed),
        CONTROLLER_MAX_WAIT_NS.load(Ordering::Relaxed),
    )
}

/// Attend que le controleur ne soit plus occupe. `false` en cas de blocage.
/// Attend la fin de l'occupation du controleur.
///
/// Le budget est exprime en TEMPS, pas en tours de boucle. Quatre millions de
/// lectures de port ne representent aucune duree connue : sous emulation, elles
/// peuvent s'ecouler en bien moins de temps qu'il n'en faut au controleur pour
/// terminer une commande, et l'attente rendait alors « occupe » un disque
/// parfaitement sain. C'est ce qu'a montre le run 32427434260 :
///
///     [kernel] ata: ecriture lba=2128427 occupee, statut=0xc0 erreur=0x00
///
/// 0xC0 vaut BSY|DRDY et le registre d'erreur est vide : aucune faute, juste
/// une commande pas encore finie. La norme ATA autorise des commandes longues ;
/// on borne donc genereusement, et en secondes.
///
/// Le compteur de tours reste comme filet : au tout debut du demarrage, la
/// sonde des disques tourne avant que le minuteur n'avance, et une borne
/// exprimee en ticks ne progresserait jamais.
fn wait_not_busy() -> bool {
    const TOURS_MAX: u64 = 400_000_000;
    let debut = crate::kernel::timer::ticks();
    let limite = 5 * crate::kernel::timer::TICKS_PER_SECOND;
    let mut tours = 0u64;
    loop {
        if unsafe { inb(STATUS) } & ST_BUSY == 0 {
            return true;
        }
        tours += 1;
        if tours > TOURS_MAX {
            return false;
        }
        if crate::kernel::timer::ticks().wrapping_sub(debut) > limite {
            return false;
        }
    }
}

/// Attend que des donnees soient disponibles.
/// Attend que des donnees soient disponibles.
///
/// Meme raison que [`wait_not_busy`] pour la borne en temps. Une erreur
/// signalee par le controleur, elle, rend la main tout de suite : c'est une
/// reponse, pas une attente.
fn wait_data_ready() -> bool {
    const TOURS_MAX: u64 = 400_000_000;
    let debut = crate::kernel::timer::ticks();
    let limite = 5 * crate::kernel::timer::TICKS_PER_SECOND;
    let mut tours = 0u64;
    loop {
        let status = unsafe { inb(STATUS) };
        if status & (ST_ERR | ST_DF) != 0 {
            return false;
        }
        if status & ST_BUSY == 0 && status & ST_DRQ != 0 {
            return true;
        }
        tours += 1;
        if tours > TOURS_MAX {
            return false;
        }
        if crate::kernel::timer::ticks().wrapping_sub(debut) > limite {
            return false;
        }
    }
}

/// Registre de controle (ecriture) : meme port que le statut alternatif.
const DEVICE_CONTROL: u16 = 0x3F6;
/// `nIEN` : coupe la ligne d'interruption du controleur.
const CTRL_NIEN: u8 = 0x02;

/// Selectionne un disque et laisse au controleur le temps de commuter.
fn select(drive: Drive) {
    wait_not_busy();
    unsafe {
        // Interruptions coupees. Ce pilote fonctionne par interrogation : une
        // IRQ14 servie par un vecteur non installe leve une faute de protection
        // generale, et la faute survenant dans un contexte que rien n'attend,
        // le CPU escalade en double faute. Autrement dit : sans ce bit, la
        // premiere commande ATA arrete le noyau.
        outb(DEVICE_CONTROL, CTRL_NIEN);
        outb(DRIVE_HEAD, drive.select_bits());
        // 400 ns d'attente reglementaires : quatre lectures du statut alternatif,
        // qui n'acquittent pas l'interruption.
        for _ in 0..4 {
            let _ = inb(ALT_STATUS);
        }
    }
}

/// Selectionne un disque ET les quatre bits hauts du LBA, avec l'attente.
///
/// `read_batch` et `write_batch` ecrivaient DRIVE_HEAD directement, sans les
/// 400 ns que `select` respecte. Or apres une ecriture dans ce registre, le
/// contenu du registre de statut n'a pas de sens tant que le delai n'est pas
/// ecoule : on peut y lire le statut du disque PRECEDEMMENT selectionne.
///
/// Les deux disques sont sur le meme canal et le noyau alterne entre eux --
/// l'image de demarrage sur hda, l'archive et la zone persistante sur hdb. Un
/// bit ERR laisse par une operation sur l'autre disque fait alors echouer
/// `wait_data_ready`, et l'operation rend un compte court. C'est le mode de
/// defaillance observe au run 32426569316 : une ecriture d'un seul secteur, au
/// beau milieu d'une zone parfaitement valide, qui rend zero.
fn select_lba(drive: Drive, lba: u64) {
    // On n'ecrit JAMAIS dans DRIVE_HEAD pendant que le controleur est occupe :
    // le registre n'est pas garanti pris en compte, et la commande suivante
    // partirait vers le mauvais disque. `read_batch` ne guette pas la fin de
    // BSY apres son dernier secteur, donc il faut l'attendre ici.
    wait_not_busy();
    unsafe {
        outb(DEVICE_CONTROL, CTRL_NIEN);
        outb(DRIVE_HEAD, drive.select_bits() | (((lba >> 24) & 0x0F) as u8));
        for _ in 0..4 {
            let _ = inb(ALT_STATUS);
        }
    }
}

/// Etat du controleur au moment d'un echec, pour le journal.
///
/// Un pilote qui echoue en silence oblige a deviner. Les deux registres disent
/// lequel des bits a arrete l'operation.
fn etat_controleur() -> (u8, u8) {
    unsafe { (inb(STATUS), inb(ERROR)) }
}

/// Interroge un disque par IDENTIFY et renvoie sa taille en secteurs.
fn identify(drive: Drive) -> u64 {
    select(drive);
    unsafe {
        outb(SECTOR_COUNT, 0);
        outb(LBA_LOW, 0);
        outb(LBA_MID, 0);
        outb(LBA_HIGH, 0);
        outb(COMMAND, CMD_IDENTIFY);

        // Statut nul : aucun disque sur ce port.
        if inb(STATUS) == 0 {
            return 0;
        }
        if !wait_not_busy() {
            return 0;
        }
        // Un peripherique ATAPI repond a IDENTIFY en posant sa signature dans
        // les registres LBA : ce n'est pas un disque, on l'ignore.
        if inb(LBA_MID) != 0 || inb(LBA_HIGH) != 0 {
            return 0;
        }
        if !wait_data_ready() {
            return 0;
        }

        let mut identity = [0u16; 256];
        for word in identity.iter_mut() {
            *word = inw(DATA);
        }
        // Mots 60-61 : nombre de secteurs adressables en LBA28.
        let lba28 = (identity[60] as u64) | ((identity[61] as u64) << 16);
        // Mots 100-103 : LBA48, plus fiable au-dela de 128 Gio.
        let lba48 = (identity[100] as u64)
            | ((identity[101] as u64) << 16)
            | ((identity[102] as u64) << 32)
            | ((identity[103] as u64) << 48);
        if lba48 > 0 { lba48 } else { lba28 }
    }
}

/// Detecte les disques presents. Idempotent.
pub fn probe() {
    let _controller = lock_controller();
    if unsafe { PROBED } {
        return;
    }
    unsafe {
        SECTORS[0] = identify(Drive::Master);
        SECTORS[1] = identify(Drive::Slave);
        PROBED = true;
    }
    drop(_controller);
    let (master, slave) = capacities();
    crate::kernel::dmesg::log_fmt(format_args!(
        "ata: hda {} secteurs ({} Mio), hdb {} secteurs ({} Mio)",
        master,
        master * SECTOR_SIZE as u64 / (1024 * 1024),
        slave,
        slave * SECTOR_SIZE as u64 / (1024 * 1024)
    ));
}

/// Tailles des deux disques, en secteurs.
pub fn capacities() -> (u64, u64) {
    probe();
    unsafe { (SECTORS[0], SECTORS[1]) }
}

/// Ce disque est-il present ?
pub fn present(drive: Drive) -> bool {
    let (master, slave) = capacities();
    match drive {
        Drive::Master => master > 0,
        Drive::Slave => slave > 0,
    }
}

/// Lit `count` secteurs a partir du secteur `lba` dans `out`.
///
/// `out` doit faire au moins `count * SECTOR_SIZE` octets. Renvoie le nombre de
/// secteurs effectivement lus.
///
/// Le mode LBA28 limite chaque commande a 256 secteurs (128 Kio) : les demandes
/// plus grandes sont decoupees.
pub fn read(drive: Drive, lba: u64, count: usize, out: &mut [u8]) -> usize {
    probe();
    if !present(drive) || count == 0 {
        return 0;
    }
    let _controller = lock_controller();
    let mut done = 0usize;
    while done < count {
        let batch = core::cmp::min(count - done, 256);
        let sector = lba + done as u64;
        if sector >= (1 << 28) {
            break; // au-dela de LBA28 : non gere
        }
        let offset = done * SECTOR_SIZE;
        if offset + batch * SECTOR_SIZE > out.len() {
            break;
        }
        if !read_batch(drive, sector, batch, &mut out[offset..]) {
            break;
        }
        done += batch;
    }
    done
}

/// Lit un lot d'au plus 256 secteurs (une seule commande ATA).
fn read_batch(drive: Drive, lba: u64, count: usize, out: &mut [u8]) -> bool {
    // Selection AVEC le delai reglementaire : sans lui, le statut lu ensuite
    // peut etre celui de l'autre disque du canal. Voir `select_lba`.
    select_lba(drive, lba);
    if !wait_not_busy() {
        let (statut, erreur) = etat_controleur();
        crate::kernel::dmesg::log_fmt(format_args!(
            "ata: lecture lba={} occupee, statut={:#04x} erreur={:#04x}",
            lba, statut, erreur
        ));
        return false;
    }
    unsafe {
        outb(ERROR, 0);
        // Un compte de 0 signifie 256 secteurs.
        outb(SECTOR_COUNT, if count == 256 { 0 } else { count as u8 });
        outb(LBA_LOW, (lba & 0xFF) as u8);
        outb(LBA_MID, ((lba >> 8) & 0xFF) as u8);
        outb(LBA_HIGH, ((lba >> 16) & 0xFF) as u8);
        outb(COMMAND, CMD_READ_SECTORS);
    }

    for index in 0..count {
        if !wait_data_ready() {
            let (statut, erreur) = etat_controleur();
            crate::kernel::dmesg::log_fmt(format_args!(
                "ata: lecture lba={} secteur {}/{} sans donnees, statut={:#04x} erreur={:#04x}",
                lba, index, count, statut, erreur
            ));
            return false;
        }
        let start = index * SECTOR_SIZE;
        unsafe {
            read_sector_into(&mut out[start..start + SECTOR_SIZE]);
        }
    }
    // Rendre le controleur au repos avant de partir : la commande suivante
    // vise souvent l'AUTRE disque du canal, et la trouverait sinon en cours.
    wait_not_busy();
    true
}

/// Transfere un secteur depuis le port de donnees.
///
/// `rep insw` fait les 256 mots en une instruction : c'est ce qui rend le PIO
/// utilisable pour lire des dizaines de mega-octets sous emulation, la ou 256
/// acces separes par secteur seraient interminables.
///
/// # Securite
/// `out` doit faire exactement [`SECTOR_SIZE`] octets.
unsafe fn read_sector_into(out: &mut [u8]) {
    debug_assert_eq!(out.len(), SECTOR_SIZE);
    core::arch::asm!(
        "rep insw",
        in("dx") DATA,
        inout("rdi") out.as_mut_ptr() => _,
        inout("rcx") SECTOR_SIZE / 2 => _,
        options(nostack)
    );
}

/// Ecrit `count` secteurs a partir du secteur `lba`. Rend le nombre ecrit.
///
/// Symetrique de [`read`], memes limites : LBA28, et 256 secteurs par commande.
/// Le cache du disque est vide a la fin — une ecriture qui reste dans le cache
/// du controleur n'est pas une ecriture persistante, et c'est precisement ce
/// qu'on cherche ici.
pub fn write(drive: Drive, lba: u64, count: usize, data: &[u8]) -> usize {
    probe();
    if !present(drive) || count == 0 {
        return 0;
    }
    let _controller = lock_controller();
    let mut done = 0usize;
    while done < count {
        let batch = core::cmp::min(count - done, 256);
        let sector = lba + done as u64;
        if sector >= (1 << 28) {
            break;
        }
        let offset = done * SECTOR_SIZE;
        if offset + batch * SECTOR_SIZE > data.len() {
            break;
        }
        if !write_batch(drive, sector, batch, &data[offset..]) {
            break;
        }
        done += batch;
    }
    if done > 0 {
        flush(drive);
    }
    done
}

/// Ecrit un lot d'au plus 256 secteurs (une seule commande ATA).
fn write_batch(drive: Drive, lba: u64, count: usize, data: &[u8]) -> bool {
    select_lba(drive, lba);
    if !wait_not_busy() {
        let (statut, erreur) = etat_controleur();
        crate::kernel::dmesg::log_fmt(format_args!(
            "ata: ecriture lba={} occupee, statut={:#04x} erreur={:#04x}",
            lba, statut, erreur
        ));
        return false;
    }
    unsafe {
        outb(ERROR, 0);
        outb(SECTOR_COUNT, if count == 256 { 0 } else { count as u8 });
        outb(LBA_LOW, (lba & 0xFF) as u8);
        outb(LBA_MID, ((lba >> 8) & 0xFF) as u8);
        outb(LBA_HIGH, ((lba >> 16) & 0xFF) as u8);
        outb(COMMAND, CMD_WRITE_SECTORS);
    }

    for index in 0..count {
        if !wait_data_ready() {
            let (statut, erreur) = etat_controleur();
            crate::kernel::dmesg::log_fmt(format_args!(
                "ata: ecriture lba={} secteur {}/{} refusee, statut={:#04x} erreur={:#04x}",
                lba, index, count, statut, erreur
            ));
            return false;
        }
        let start = index * SECTOR_SIZE;
        unsafe {
            write_sector_from(&data[start..start + SECTOR_SIZE]);
        }
    }
    if !wait_not_busy() {
        let (statut, erreur) = etat_controleur();
        crate::kernel::dmesg::log_fmt(format_args!(
            "ata: ecriture lba={} inachevee, statut={:#04x} erreur={:#04x}",
            lba, statut, erreur
        ));
        return false;
    }
    true
}

/// Demande au disque de vider son cache d'ecriture.
fn flush(drive: Drive) {
    select(drive);
    if !wait_not_busy() {
        return;
    }
    unsafe {
        outb(COMMAND, CMD_FLUSH_CACHE);
    }
    wait_not_busy();
}

/// Transfere un secteur vers le port de donnees.
///
/// `rep outsw` pour la meme raison que `rep insw` a la lecture : un acces par
/// mot rendrait l'ecriture de quelques mega-octets interminable sous emulation.
///
/// # Securite
/// `data` doit faire exactement [`SECTOR_SIZE`] octets.
unsafe fn write_sector_from(data: &[u8]) {
    debug_assert_eq!(data.len(), SECTOR_SIZE);
    core::arch::asm!(
        "rep outsw",
        in("dx") DATA,
        inout("rsi") data.as_ptr() => _,
        inout("rcx") SECTOR_SIZE / 2 => _,
        options(nostack)
    );
}
