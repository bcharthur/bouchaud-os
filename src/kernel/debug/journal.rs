//! Préfixe des lignes du journal série : heure, charge et FPS utiles.
//!
//!     [18:51:48][CPU=12% RAM=32/640MiB(5%) DISK=... GPU=...][FPS: 58] ...
//!
//! V16.2 construit tout le préfixe dans un tampon fixe sur pile puis l'envoie
//! au UART d'un bloc. Auparavant chaque couleur, séparateur et nombre réentrait
//! dans `serial_print!`, ce qui multipliait les attentes THRE sous QEMU/TCG.

use crate::arch::x86_64::rtc;
use core::fmt::{self, Write};
use crate::kernel::system_metrics::{Available, Sample};
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

static PRET: AtomicBool = AtomicBool::new(false);
static COULEURS: AtomicBool = AtomicBool::new(true);
const PREFIXE_MAX: usize = 256;
const METRIC_WORDS: usize = 24;
const METRICS_UNAVAILABLE: &str = "CPU=N/A RAM=N/A DISK=N/A GPU=N/A";
static METRIC_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static METRIC_LENGTH: AtomicUsize = AtomicUsize::new(0);
static METRIC_CACHE: [AtomicU64; METRIC_WORDS] =
    [const { AtomicU64::new(0) }; METRIC_WORDS];

pub fn demarre() {
    PRET.store(true, Ordering::Release);
    crate::serial_println!("[journal] [heure][CPU RAM DISK GPU][FPS:trames utiles]");
}

pub fn pose_couleurs(actif: bool) {
    COULEURS.store(actif, Ordering::Relaxed);
}

pub fn couleurs() -> bool {
    COULEURS.load(Ordering::Relaxed)
}

const RESET: &str = "\x1b[0m";
const GRIS: &str = "\x1b[90m";
/// Format once per sample, then publish a bounded atomic byte snapshot. All
/// accesses are ordered so a reader can reject a concurrent update without
/// waiting, even if the writer was interrupted by this very log message.
pub(crate) fn publish_metrics(sample: Sample) {
    let mut formatted = Prefixe::neuf();
    let rate_kib = |rate: Option<u64>| Available(rate.map(|bytes| bytes / 1024));
    let (disk, reads, writes) = if sample.nvme_present {
        ("NVMe", sample.nvme_read_bytes_per_sec, sample.nvme_write_bytes_per_sec)
    } else {
        (if sample.ata_present { "ATA" } else { "N/A" }, sample.ata_read_bytes_per_sec, None)
    };
    let ram_pct = sample.ram_used_pct();
    if write!(formatted, "CPU={}{} RAM=", Available(sample.cpu_busy_pct),
        if sample.cpu_busy_pct.is_some() { "%" } else { "" }).is_err() {
        return;
    }
    let ram = if ram_pct.is_some() {
        write!(formatted, "{}/{}MiB({}%)", sample.ram_used_bytes >> 20,
            sample.ram_total_bytes >> 20, Available(ram_pct))
    } else {
        formatted.write_str("N/A")
    };
    if ram.is_err() || write!(formatted,
        " DISK={disk}:r{}/w{}KiB/s,p{}/b{} GPU={}:f{},p{}us,uN/A",
        rate_kib(reads), rate_kib(writes),
        Available(sample.persist_available.then_some(sample.persist_commits)),
        Available(sample.persist_available.then_some(sample.persist_barriers)),
        sample.desktop_backend, sample.virgl_frames, Available(sample.virtio_present_us)
    ).is_err() || formatted.len > METRIC_WORDS * 8 {
        return;
    }
    let sequence = METRIC_SEQUENCE.load(Ordering::SeqCst);
    if sequence & 1 != 0 || METRIC_SEQUENCE.compare_exchange(
        sequence, sequence.wrapping_add(1), Ordering::SeqCst, Ordering::SeqCst,
    ).is_err() {
        return;
    }
    for (word, bytes) in METRIC_CACHE.iter().zip(formatted.donnees.chunks_exact(8))
        .take(formatted.len.div_ceil(8))
    {
        word.store(u64::from_ne_bytes(bytes.try_into().unwrap()), Ordering::SeqCst);
    }
    METRIC_LENGTH.store(formatted.len, Ordering::SeqCst);
    METRIC_SEQUENCE.store(sequence.wrapping_add(2), Ordering::SeqCst);
}

/// BOUCHAUD_PREFIXE_METRIQUES_DELTA_V1 -- LE BLOC N'EST REPETE QUE S'IL CHANGE.
///
/// Smoke Ladybird de `main@e18a954` : 3 875 lignes, 900 Kio de serie, dont
/// 431 Kio (48 %) de ce seul prefixe -- le MEME bloc de metriques recopie sur
/// chaque ligne entre deux echantillons. Chaque octet est une ecriture sur le
/// port COM1, donc une sortie de VM sous KVM/WHPX (909 000 au total).
///
/// Le bloc est desormais ecrit quand l'echantillon a change depuis la ligne
/// precedente, et remplace sinon par `=` : `[12:00:01][=][FPS: 21] ...`. Le
/// format garde ses deux groupes entre crochets avant `[FPS:` -- celui que
/// les analyseurs retirent (`^(\[[^]]*\]){2}\[FPS:[^]]*\] `). Chaque
/// echantillon publie apparait au moins une fois dans le journal.
static SEQUENCE_EMISE: AtomicU64 = AtomicU64::new(u64::MAX);

fn ajoute_metriques(out: &mut Prefixe) {
    let sequence = METRIC_SEQUENCE.load(Ordering::SeqCst);
    if sequence & 1 == 0 {
        // Avant le premier echantillon, l'etat « indisponible » a sa propre
        // marque (impaire, donc jamais une sequence publiee stable).
        let marque = if METRIC_LENGTH.load(Ordering::SeqCst) == 0 { u64::MAX - 2 } else { sequence };
        if SEQUENCE_EMISE.swap(marque, Ordering::Relaxed) == marque {
            let _ = out.write_str("=");
            return;
        }
    }
    if sequence & 1 == 0 {
        let len = METRIC_LENGTH.load(Ordering::SeqCst);
        if len != 0 {
            let start = out.len;
            for (i, word) in METRIC_CACHE.iter().take(len.div_ceil(8)).enumerate() {
                let offset = start + i * 8;
                out.donnees[offset..offset + 8].copy_from_slice(&word.load(Ordering::SeqCst).to_ne_bytes());
            }
            if METRIC_SEQUENCE.load(Ordering::SeqCst) == sequence {
                out.len += len;
                return;
            }
        }
    }
    let _ = out.write_str(METRICS_UNAVAILABLE);
}

// BOUCHAUD_V16_2_PREFIX_BUFFER
pub struct Prefixe {
    donnees: [u8; PREFIXE_MAX],
    len: usize,
}

impl Prefixe {
    const fn neuf() -> Self {
        Self { donnees: [0; PREFIXE_MAX], len: 0 }
    }

    pub fn octets(&self) -> &[u8] {
        &self.donnees[..self.len]
    }
}

impl fmt::Write for Prefixe {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let octets = s.as_bytes();
        if self.len + octets.len() > self.donnees.len() {
            return Err(fmt::Error);
        }
        self.donnees[self.len..self.len + octets.len()].copy_from_slice(octets);
        self.len += octets.len();
        Ok(())
    }
}

#[inline]
fn ajoute_couleur(out: &mut Prefixe, couleur: &str) {
    if couleurs() {
        let _ = out.write_str(couleur);
    }
}

fn ajoute_fps(out: &mut Prefixe, valeur: Option<u16>) {
    match valeur {
        Some(valeur) => {
            let _ = out.write_fmt(format_args!("{:3}", valeur.min(999)));
        }
        None => {
            let _ = out.write_str(" --");
        }
    }
}

/// BOUCHAUD_PREFIXE_SANS_CMOS_V1 : heure, minute, seconde locales du prefixe,
/// lues sur l'horloge murale ancree (aucun acces CMOS) ; la RTC tant que
/// l'ancre n'est pas posee.
fn heure_locale() -> rtc::DateTime {
    match crate::kernel::abi::realtime_ns_ancree() {
        Some(ns) => {
            let decalage = rtc::LOCAL_UTC_OFFSET_HOURS as i64 * 3600;
            let s = ((ns / 1_000_000_000) as i64 + decalage).rem_euclid(86_400) as u32;
            rtc::DateTime {
                second: (s % 60) as u8,
                minute: (s / 60 % 60) as u8,
                hour: (s / 3600) as u8,
                day: 0,
                month: 0,
                year: 0,
            }
        }
        None => rtc::now(),
    }
}

pub fn ecris_prefixe() {
    let out = prefixe();
    crate::drivers::serial::ecris_octets_sans_prefixe(out.octets());
}

/// BOUCHAUD_CONSOLE_ASYNCHRONE_V1 : le prefixe d'une ligne, construit sur pile.
/// La console asynchrone le pose A L'AJOUT de la ligne : l'heure est celle de
/// l'evenement, pas celle de son emission par le fil de vidange.
pub fn prefixe() -> Prefixe {
    let heure = heure_locale();
    let frame = if PRET.load(Ordering::Acquire) {
        Some(crate::gui::frame_clock::snapshot())
    } else {
        None
    };

    let mut out = Prefixe::neuf();

    ajoute_couleur(&mut out, GRIS);

    let _ = out.write_str("[");
    let _ = out.write_fmt(format_args!(
        "{:02}:{:02}:{:02}",
        heure.hour, heure.minute, heure.second
    ));
    let _ = out.write_str("][");

    ajoute_metriques(&mut out);
    let _ = out.write_str("][FPS:");
    ajoute_fps(&mut out, frame.filter(|s| s.active).map(|s| s.fps_arrondi()));
    let _ = out.write_str("] ");
    ajoute_couleur(&mut out, RESET);
    out
}
