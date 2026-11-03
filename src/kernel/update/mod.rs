//! kernel::update -- the MECHANISM behind the update capability
//! (BOUCHAUD_UPDATE_MECANISME_V1).
//!
//! Policy (which slot, how many attempts, when to roll back) lives in
//! `bouchaud-update-core`, shared with the loader. The kernel only:
//!
//!   * keeps the handoff the loader left (which slot it booted, from which
//!     metadata record, on which META partition);
//!   * finds THAT partition -- by its unique GPT GUID, on any registered
//!     volume -- never "a" Bouchaud layout on some other disk;
//!   * applies `passage::commit_sante` when an authorised caller (the system
//!     supervisor, through a capability) declares the boot healthy: write the
//!     older copy, FLUSH, read back, compare.
//!
//! Fail closed everywhere: no handoff, no partition, no real flush, a record
//! that changed since the loader, a read-back that differs -> nothing is
//! committed, the candidate stays a candidate, the loader rolls it back.

use alloc::vec;

use bouchaud_update_core::passage::{self, Passage, RefusCommit};
use bouchaud_update_core::{self as uc, disque, Enregistrement, Etat, Slot};

use crate::drivers::bloc::{self, Volume};
use crate::fs::gpt::{self, Guid, Support};
use crate::kernel::sync::SpinLock;

static PASSAGE: SpinLock<Option<Passage>> = SpinLock::new(None);
static COMMITTE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
static BRUT_INVALIDE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Called once by the boot normaliser with the 64 bytes of the handoff page.
pub fn note_passage_brut(brut: &[u8]) {
    match passage::decode(brut) {
        Some(p) => *PASSAGE.lock() = Some(p),
        None => BRUT_INVALIDE.store(true, core::sync::atomic::Ordering::Relaxed),
    }
}

pub fn passage() -> Option<Passage> { *PASSAGE.lock() }

const fn slot_texte(s: Slot) -> &'static str { match s { Slot::A => "A", Slot::B => "B" } }

fn etat_texte(e: Etat) -> &'static str {
    match e {
        Etat::Vide => "vide",
        Etat::Candidat { .. } => "candidat",
        Etat::Bon => "bon",
        Etat::Mauvais => "mauvais",
    }
}

/// Boot record for the blackbox (§28): one line, printed once at boot.
pub fn journalise_boot() {
    match passage() {
        Some(p) => crate::serial_println!(
            "BOUCHAUD_BOOT_RECORD boot_id={:#018x} system_slot={} boot_reason={} candidate_attempts_left={} meta_sequence={}",
            p.boot_id, slot_texte(p.slot), p.raison.texte(), p.essais_restants, p.sequence_meta
        ),
        None => crate::serial_println!(
            "BOUCHAUD_BOOT_RECORD boot_id=none system_slot=none boot_reason={}",
            if BRUT_INVALIDE.load(core::sync::atomic::Ordering::Relaxed) { "passage_invalide" } else { "sans_chargeur_ab" }
        ),
    }
}

// ---------------------------------------------------------------------------
// The META partition the loader used
// ---------------------------------------------------------------------------

/// A registered block volume seen as a GPT support (whole disk).
struct SupportVolume { volume: Volume, taille_bloc: usize, blocs: u64 }

impl Support for SupportVolume {
    fn taille_bloc(&self) -> usize { self.taille_bloc }
    fn blocs(&self) -> u64 { self.blocs }
    fn lit(&mut self, lba: u64, sortie: &mut [u8]) -> bool {
        let n = sortie.len() / self.taille_bloc;
        n != 0 && bloc::lit(self.volume, lba, n, sortie).blocs() == n
    }
    fn ecrit(&mut self, lba: u64, donnees: &[u8]) -> bool {
        let n = donnees.len() / self.taille_bloc;
        n != 0 && bloc::ecrit(self.volume, lba, n, donnees).blocs() == n
    }
    fn vidange(&mut self) -> bool { bloc::vidange(self.volume) }
}

struct Meta { support: SupportVolume, premier: u64 }

/// Volumes that can carry the boot disk: internal NVMe, USB mass storage,
/// the ATA boot disk. A window volume (DONNEES) never holds a GPT header.
const CANDIDATS: [Volume; 3] = [Volume(2), Volume(3), Volume::AMORCE];

fn guid_type(t: disque::TypeGuid) -> Guid { Guid::depuis(t.d1, t.d2, t.d3, t.d4) }

fn trouve_meta(p: &Passage) -> Result<Meta, &'static str> {
    if p.meta_unique == [0u8; 16] { return Err("meta_sans_identite"); }
    let type_meta = guid_type(disque::TYPE_META);
    for volume in CANDIDATS {
        if !bloc::present(volume) { continue; }
        let d = bloc::descripteur(volume);
        if d.blocs == 0 || d.taille_bloc < uc::OCTETS { continue; }
        let mut support = SupportVolume { volume, taille_bloc: d.taille_bloc, blocs: d.blocs };
        let Ok((parts, _degradee)) = gpt::lit_table(&mut support) else { continue };
        if let Some(m) = parts.iter().find(|x| x.type_guid == type_meta && x.unique.octets() == p.meta_unique) {
            if m.blocs() < 2 { return Err("meta_trop_petite"); }
            if !d.vidange_reelle { return Err("volume_sans_vidange_reelle"); }
            return Ok(Meta { support, premier: m.premier });
        }
    }
    Err("meta_introuvable")
}

fn lit_copies(m: &mut Meta) -> Option<(vec::Vec<u8>, vec::Vec<u8>)> {
    let t = m.support.taille_bloc;
    let mut c0 = vec![0u8; t];
    let mut c1 = vec![0u8; t];
    if !m.support.lit(m.premier, &mut c0) || !m.support.lit(m.premier + 1, &mut c1) { return None; }
    Some((c0, c1))
}

fn lit(m: &mut Meta) -> Result<Enregistrement, &'static str> {
    let (c0, c1) = lit_copies(m).ok_or("meta_illisible")?;
    uc::selectionne(&c0[..uc::OCTETS], &c1[..uc::OCTETS]).ok_or("deux_copies_invalides")
}

/// Older copy <- e, FLUSH, read back. Ok only if the re-read copy decodes to e.
fn persiste(m: &mut Meta, e: &Enregistrement) -> Result<(), &'static str> {
    let (c0, c1) = lit_copies(m).ok_or("meta_illisible")?;
    let i = uc::copie_a_ecrire(&c0[..uc::OCTETS], &c1[..uc::OCTETS]);
    let mut bloc = if i == 0 { c0 } else { c1 };
    bloc[..uc::OCTETS].copy_from_slice(&uc::encode(e));
    if !m.support.ecrit(m.premier + i as u64, &bloc) { return Err("ecriture"); }
    if !m.support.vidange() { return Err("vidange"); }
    let (r0, r1) = lit_copies(m).ok_or("relecture")?;
    let relu = if i == 0 { r0 } else { r1 };
    if uc::decode(&relu[..uc::OCTETS]).as_ref() != Some(e) { return Err("relecture_differente"); }
    Ok(())
}

/// What the update capability's READ_STATE returns.
#[derive(Clone, Copy)]
pub struct EtatBoot {
    pub passage: Passage,
    pub enregistrement: Enregistrement,
}

pub fn etat() -> Result<EtatBoot, &'static str> {
    let p = passage().ok_or("sans_passage")?;
    let mut m = trouve_meta(&p)?;
    Ok(EtatBoot { passage: p, enregistrement: lit(&mut m)? })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Commit {
    /// The booted candidate is now the active, Good slot (durable).
    Committe,
    /// Already the good active slot: nothing written.
    DejaSain,
}

/// Declares the running boot healthy. See `passage::commit_sante`.
pub fn commit_sante() -> Result<Commit, &'static str> {
    let p = passage().ok_or("sans_passage")?;
    if COMMITTE.load(core::sync::atomic::Ordering::Acquire) { return Ok(Commit::DejaSain); }
    let mut m = trouve_meta(&p)?;
    let e = lit(&mut m)?;
    let r = match passage::commit_sante(&p, &e) {
        Ok(None) => Ok(Commit::DejaSain),
        Ok(Some(n)) => persiste(&mut m, &n).map(|_| {
            COMMITTE.store(true, core::sync::atomic::Ordering::Release);
            Commit::Committe
        }),
        Err(RefusCommit::MetaChangee) => Err("meta_changee_depuis_le_boot"),
        Err(RefusCommit::SlotNiCandidatNiActif) => Err("slot_ni_candidat_ni_actif"),
    };
    match r {
        Ok(c) => crate::serial_println!(
            "BOUCHAUD_UPDATE_SANTE_COMMIT resultat={} slot={} etat_avant={}",
            if c == Commit::Committe { "committe" } else { "deja_sain" },
            slot_texte(p.slot), etat_texte(e.slot(p.slot).etat)
        ),
        Err(raison) => crate::serial_println!("BOUCHAUD_UPDATE_SANTE_REFUS raison={}", raison),
    }
    r
}

// ---------------------------------------------------------------------------
// The system supervisor: the ONLY holder of the update capability
// ---------------------------------------------------------------------------

/// Where the system supervisor lives (ramdisk of the booted slot).
pub const SUPERVISEUR: &str = "/bin/serviced";

/// Rights the kernel grants the supervisor. It may delegate (attenuated) to
/// `updated`; it never receives UPDATE_INSTALL itself, so a supervisor that
/// is compromised cannot write a slot -- only confirm or not confirm health.
pub const DROITS_SUPERVISEUR: crate::kernel::native::abi::types::Rights =
    crate::kernel::native::abi::types::Rights(
        crate::kernel::native::abi::types::Rights::INSPECT.0
            | crate::kernel::native::abi::types::Rights::UPDATE_HEALTH.0
            | crate::kernel::native::abi::types::Rights::DUP.0
            | crate::kernel::native::abi::types::Rights::TRANSFER.0,
    );

/// Starts `/bin/serviced system` with two bootstrap handles: the health
/// capability and the install capability. No syscall creates an Update
/// object; this is its only origin.
///
/// No supervisor in the slot -> nobody can declare health -> a candidate is
/// never committed and the loader rolls it back. That is the intended,
/// fail-closed outcome, and it is printed.
pub fn lance_superviseur() -> Option<u32> {
    use alloc::string::{String, ToString};
    if crate::fs::ramfs::fs().resolve(SUPERVISEUR, 0).is_none() {
        crate::serial_println!("BOUCHAUD_SUPERVISEUR_ABSENT chemin={} sante=jamais_declaree", SUPERVISEUR);
        return None;
    }
    let argv = [SUPERVISEUR.to_string(), "system".to_string()];
    let env = [String::from("PATH=/bin:/usr/bin")];
    // Two capabilities: [0] health (kept by serviced), [1] install (passed
    // on to updated, never used by serviced itself).
    let mut prepare = |process: &crate::kernel::task::Process| -> alloc::vec::Vec<String> {
        use crate::kernel::native::{handle, object::Object};
        let sante = handle::install_for(process.pid, Object::Update, DROITS_SUPERVISEUR);
        let install = handle::install_for(process.pid, Object::Update, DROITS_INSTALLATION);
        match (sante, install) {
            (Ok(a), Ok(b)) => alloc::vec![alloc::format!("BO_BOOTSTRAP={},{}", a.raw(), b.raw())],
            _ => alloc::vec![],
        }
    };
    match crate::kernel::exec::lance_detache(SUPERVISEUR, &argv, &env, 0, &mut prepare) {
        Ok(pid) => {
            crate::serial_println!("BOUCHAUD_SUPERVISEUR_LANCE pid={} capacite=update droits={:#x}", pid, DROITS_SUPERVISEUR.0);
            Some(pid)
        }
        Err(e) => {
            crate::serial_println!("BOUCHAUD_SUPERVISEUR_ECHEC raison={}", e);
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Installation into the INACTIVE slot (right UPDATE_INSTALL, held by updated)
// ---------------------------------------------------------------------------
//
// Three steps, and nothing else reaches the disk:
//   begin(target)      policy: commence_installation(target, running) -- the
//                      running slot and the active slot are refused (I5, I6);
//                      the slot becomes Empty on disk BEFORE any byte of it is
//                      overwritten, so a cut mid-write never leaves a slot the
//                      loader would boot;
//   write(off, data)   block-aligned, bounded to THAT slot's partition: no
//                      offset reaches META, the other slot or user data;
//   verify(len)        real flush once, then read the slot back ONE CHUNK per
//                      call into a running SHA-256 (a long readback in one
//                      syscall starved the caller's heartbeats: QEMU bench);
//   finish(sha, len)   all `len` bytes read back, SHA-256 must equal `sha`,
//                      then marque_candidat.
//
// The session belongs to the process that called begin. Any other process
// -- including a restarted updated -- must begin again; a write from a dead
// owner's late syscall can no longer land in a new session (QEMU bench: a
// killed updated's in-flight finish completed inside its successor's
// session).

/// Why an install step is refused (mapped to ABI errors by the dispatcher).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefusInstall {
    /// Policy or capability state: wrong slot, no begin, out of the slot.
    Refuse(&'static str),
    /// Malformed request (alignment, size).
    Argument(&'static str),
    /// The device failed (write, flush, read back).
    Io(&'static str),
}

#[derive(Clone, Copy)]
struct Installation {
    proprietaire: u32,
    /// Bytes already read back into VERIFICATION (0: not started).
    verifie: u64,
    cible: Slot,
    volume: Volume,
    taille_bloc: usize,
    premier: u64,
    octets: u64,
}

static INSTALLATION: SpinLock<Option<Installation>> = SpinLock::new(None);
static VERIFICATION: SpinLock<Option<crate::net::security::tls::sha256::Sha256>> = SpinLock::new(None);

fn appelant() -> u32 { crate::kernel::task::current_process().pid }

/// The session, if the caller owns it.
fn session() -> Result<Installation, RefusInstall> {
    let inst = (*INSTALLATION.lock()).ok_or(RefusInstall::Refuse("sans_begin"))?;
    if inst.proprietaire != appelant() { return Err(RefusInstall::Refuse("session_d_un_autre_processus")); }
    Ok(inst)
}

fn note_refus(etape: &str, r: RefusInstall) -> RefusInstall {
    let raison = match r { RefusInstall::Refuse(x) | RefusInstall::Argument(x) | RefusInstall::Io(x) => x };
    crate::serial_println!("BOUCHAUD_UPDATE_INSTALL_REFUS etape={} raison={}", etape, raison);
    r
}

pub fn installation_commence(cible: Slot) -> Result<(), RefusInstall> {
    let r = (|| {
        let p = passage().ok_or(RefusInstall::Refuse("sans_passage"))?;
        let mut m = trouve_meta(&p).map_err(RefusInstall::Refuse)?;
        let e = lit(&mut m).map_err(RefusInstall::Io)?;
        // The policy, not this module, says which targets are acceptable.
        let n = e.commence_installation(cible, p.slot).map_err(|refus| RefusInstall::Refuse(match refus {
            uc::Refus::CibleEnCoursDExecution => "cible_en_cours_d_execution",
            uc::Refus::CibleActive => "cible_active",
            uc::Refus::CandidatEnCours => "candidat_en_cours",
            _ => "etat_inattendu",
        }))?;
        let type_cible = guid_type(disque::type_du_slot(cible));
        let (parts, _) = gpt::lit_table(&mut m.support).map_err(|_| RefusInstall::Io("gpt"))?;
        let part = parts.iter().find(|x| x.type_guid == type_cible).ok_or(RefusInstall::Refuse("slot_introuvable"))?;
        let inst = Installation {
            proprietaire: appelant(), verifie: 0,
            cible, volume: m.support.volume, taille_bloc: m.support.taille_bloc,
            premier: part.premier, octets: part.blocs() * m.support.taille_bloc as u64,
        };
        persiste(&mut m, &n).map_err(RefusInstall::Io)?;
        *VERIFICATION.lock() = None;
        *INSTALLATION.lock() = Some(inst);
        crate::serial_println!("BOUCHAUD_UPDATE_INSTALL_BEGIN cible={} octets={}", slot_texte(cible), inst.octets);
        Ok(())
    })();
    r.map_err(|e| note_refus("begin", e))
}

pub fn installation_ecrit(decalage: u64, donnees: &[u8]) -> Result<(), RefusInstall> {
    let r = (|| {
        let inst = session()?;
        let t = inst.taille_bloc as u64;
        if donnees.is_empty() || decalage % t != 0 || donnees.len() as u64 % t != 0 {
            return Err(RefusInstall::Argument("non_aligne"));
        }
        let fin = decalage.checked_add(donnees.len() as u64).ok_or(RefusInstall::Refuse("hors_slot"))?;
        if fin > inst.octets { return Err(RefusInstall::Refuse("hors_slot")); }
        let n = donnees.len() / inst.taille_bloc;
        // Any write invalidates a readback in progress.
        if let Some(i) = INSTALLATION.lock().as_mut() { i.verifie = 0; }
        *VERIFICATION.lock() = None;
        if bloc::ecrit(inst.volume, inst.premier + decalage / t, n, donnees).blocs() != n {
            return Err(RefusInstall::Io("ecriture"));
        }
        Ok(())
    })();
    r.map_err(|e| note_refus("write", e))
}

/// Reads back the next chunk (at most 1 MiB) of the first `longueur` bytes
/// of the target slot into the running SHA-256. Returns the bytes still to
/// read back (0: done). The first call flushes the device.
pub fn installation_verifie(longueur: u64) -> Result<u64, RefusInstall> {
    let r = (|| {
        let inst = session()?;
        if longueur == 0 || longueur > inst.octets { return Err(RefusInstall::Argument("longueur")); }
        if inst.verifie == 0 {
            if !bloc::vidange(inst.volume) { return Err(RefusInstall::Io("vidange")); }
            *VERIFICATION.lock() = Some(crate::net::security::tls::sha256::Sha256::new());
        }
        if inst.verifie >= longueur { return Ok(0); }
        let t = inst.taille_bloc as u64;
        let reste = longueur - inst.verifie;
        let blocs = ((reste + t - 1) / t).min(1024 * 1024 / t) as usize;
        let mut tampon = vec![0u8; blocs * inst.taille_bloc];
        if bloc::lit(inst.volume, inst.premier + inst.verifie / t, blocs, &mut tampon).blocs() != blocs {
            return Err(RefusInstall::Io("relecture"));
        }
        let utile = (tampon.len() as u64).min(reste) as usize;
        {
            let mut v = VERIFICATION.lock();
            let h = v.as_mut().ok_or(RefusInstall::Refuse("verification_interrompue"))?;
            h.update(&tampon[..utile]);
        }
        let mut g = INSTALLATION.lock();
        let i = g.as_mut().filter(|i| i.proprietaire == inst.proprietaire)
            .ok_or(RefusInstall::Refuse("session_perdue"))?;
        i.verifie += utile as u64;
        Ok(longueur - i.verifie)
    })();
    r.map_err(|e| note_refus("verify", e))
}

pub fn installation_termine(empreinte: [u8; 32], longueur: u64, essais: u8) -> Result<(), RefusInstall> {
    let r = (|| {
        let inst = session()?;
        // Only what the DISK holds counts: every byte must have been read back.
        if longueur == 0 || inst.verifie != longueur { return Err(RefusInstall::Refuse("relecture_incomplete")); }
        // A refused finish (wrong digest) leaves the readback intact.
        let relue = VERIFICATION.lock().as_ref().ok_or(RefusInstall::Refuse("relecture_incomplete"))?.clone().finalize();
        let p = passage().ok_or(RefusInstall::Refuse("sans_passage"))?;
        let mut m = trouve_meta(&p).map_err(RefusInstall::Refuse)?;
        let e = lit(&mut m).map_err(RefusInstall::Io)?;
        let n = e.marque_candidat(inst.cible, empreinte, relue, essais).map_err(|refus| RefusInstall::Refuse(match refus {
            uc::Refus::ImageNonVerifiee => "empreinte_differente",
            uc::Refus::EssaisNuls => "essais_nuls",
            uc::Refus::CibleActive => "cible_active",
            _ => "etat_inattendu",
        }))?;
        persiste(&mut m, &n).map_err(RefusInstall::Io)?;
        *INSTALLATION.lock() = None;
        *VERIFICATION.lock() = None;
        crate::serial_println!(
            "BOUCHAUD_UPDATE_INSTALL_CANDIDAT cible={} octets={} essais={} empreinte={:02x}{:02x}{:02x}{:02x}",
            slot_texte(inst.cible), longueur, essais, relue[0], relue[1], relue[2], relue[3]
        );
        Ok(())
    })();
    r.map_err(|e| note_refus("finish", e))
}

/// Rights the kernel grants for `updated` (handed to serviced, which only
/// passes it on): INSPECT | UPDATE_INSTALL | TRANSFER. No DUP, no
/// UPDATE_HEALTH.
pub const DROITS_INSTALLATION: crate::kernel::native::abi::types::Rights =
    crate::kernel::native::abi::types::Rights(
        crate::kernel::native::abi::types::Rights::INSPECT.0
            | crate::kernel::native::abi::types::Rights::UPDATE_INSTALL.0
            | crate::kernel::native::abi::types::Rights::TRANSFER.0,
    );
