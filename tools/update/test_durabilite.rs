//! Durability boundaries of an A/B update (BOUCHAUD_UPDATE_DURABILITE_V1).
//!
//!   rustc --edition 2021 --test tools/update/test_durabilite.rs -o /tmp/t && /tmp/t
//!
//! test_ab.rs models a write as a byte PREFIX reaching the medium. A real disk
//! does not work like that: writes land in a volatile controller cache, may be
//! reordered, reach the medium sector by sector, and only `flush` (or FUA)
//! makes them durable. This model:
//!
//!   * sectors of 512 bytes; a write goes to a VOLATILE cache;
//!   * `flush` makes every cached write durable;
//!   * a POWER LOSS keeps, for every sector, EITHER its durable content OR ANY
//!     of the cached writes to it (any subset, any order: reordering), and a
//!     metadata sector may additionally be TORN (prefix new / suffix old);
//!   * sector-level atomicity is NOT assumed for metadata (torn case), it IS
//!     assumed nowhere else: image sectors are only trusted after a hash check.
//!
//! The update procedure (what `updated` must do) and the loader's boot step are
//! replayed with a power loss injected at every durability boundary of the
//! mission (§8). After each loss the next boot must find a coherent, bootable
//! state: never RECOVERY_REQUIRED from a correct procedure, never a candidate
//! whose image is not fully durable, never a candidate booted more often than
//! its attempt budget.
//!
//! Two mutation tests prove the model can see the failures it guards against:
//! dropping the flush after the image, or booting before the consumed attempt
//! is flushed, MUST be caught.

#![allow(dead_code)]

#[path = "../../system/update-core/src/ab.rs"]
mod ab;

use ab::*;
use std::collections::BTreeMap;

const SECT: usize = 512;
type Secteur = [u8; SECT];

// Layout of the model disk (sectors): two metadata copies, then two slots.
const META: [usize; 2] = [0, 1];
const IMG_SECTEURS: usize = 4;
const fn base(slot: Slot) -> usize { match slot { Slot::A => 16, Slot::B => 32 } }

#[derive(Clone)]
struct Disque {
    durable: BTreeMap<usize, Secteur>,
    cache: Vec<(usize, Secteur)>,
}

impl Disque {
    fn neuf() -> Self { Disque { durable: BTreeMap::new(), cache: Vec::new() } }
    fn ecrit(&mut self, lba: usize, s: Secteur) { self.cache.push((lba, s)); }
    fn lit(&self, lba: usize) -> Secteur {
        self.cache.iter().rev().find(|(l, _)| *l == lba).map(|(_, s)| *s)
            .or_else(|| self.durable.get(&lba).copied()).unwrap_or([0; SECT])
    }
    fn flush(&mut self) {
        for (l, s) in self.cache.drain(..) { self.durable.insert(l, s); }
    }
    /// Every durable outcome of a power loss NOW.
    fn coupures(&self) -> Vec<Disque> {
        // Per sector: candidates = durable value + every cached version.
        let mut par_lba: BTreeMap<usize, Vec<Secteur>> = BTreeMap::new();
        for (l, s) in &self.cache {
            par_lba.entry(*l).or_insert_with(|| vec![self.durable.get(l).copied().unwrap_or([0; SECT])]).push(*s);
        }
        let mut issues = vec![Disque { durable: self.durable.clone(), cache: Vec::new() }];
        for (lba, versions) in par_lba {
            let mut suivants = Vec::new();
            for d in &issues {
                let ancien = versions[0];
                for v in &versions {
                    let mut e = d.clone();
                    e.durable.insert(lba, *v);
                    suivants.push(e);
                }
                // Torn metadata sector: new prefix over old suffix.
                if META.contains(&lba) {
                    for v in &versions[1..] {
                        for coupe in [8usize, 16, 40, 88, 95] {
                            let mut t = ancien;
                            t[..coupe].copy_from_slice(&v[..coupe]);
                            let mut e = d.clone();
                            e.durable.insert(lba, t);
                            suivants.push(e);
                        }
                    }
                }
            }
            issues = suivants;
        }
        issues
    }
}

fn secteur_meta(e: &Enregistrement) -> Secteur {
    let mut s = [0u8; SECT];
    s[..OCTETS].copy_from_slice(&encode(e));
    s
}

fn lit_meta(d: &Disque) -> Option<Enregistrement> {
    let a = d.lit(META[0]);
    let b = d.lit(META[1]);
    selectionne(&a[..OCTETS], &b[..OCTETS])
}

fn ecrit_meta(d: &mut Disque, e: &Enregistrement) {
    let a = d.lit(META[0]);
    let b = d.lit(META[1]);
    let i = copie_a_ecrire(&a[..OCTETS], &b[..OCTETS]);
    d.ecrit(META[i], secteur_meta(e));
}

fn image(version: u8) -> Vec<Secteur> {
    (0..IMG_SECTEURS).map(|i| [version ^ (i as u8).wrapping_mul(37); SECT]).collect()
}

/// FNV-1a 64 over the slot's sectors, in the first 8 bytes of a 32-byte id.
fn empreinte_secteurs(secteurs: &[Secteur]) -> [u8; 32] {
    let mut h: u64 = 0xcbf29ce484222325;
    for s in secteurs { for &o in s.iter() { h ^= o as u64; h = h.wrapping_mul(0x100000001b3); } }
    let mut out = [0u8; 32];
    out[..8].copy_from_slice(&h.to_le_bytes());
    out
}

fn empreinte_slot(d: &Disque, slot: Slot) -> [u8; 32] {
    let v: Vec<Secteur> = (0..IMG_SECTEURS).map(|i| d.lit(base(slot) + i)).collect();
    empreinte_secteurs(&v)
}

/// Fresh install: slot A good with image v1, everything durable.
fn installe() -> Disque {
    let mut d = Disque::neuf();
    for (i, s) in image(1).iter().enumerate() { d.ecrit(base(Slot::A) + i, *s); }
    let e = Enregistrement::initial(Slot::A, empreinte_secteurs(&image(1)));
    d.ecrit(META[0], secteur_meta(&e));
    d.ecrit(META[1], secteur_meta(&e));
    d.flush();
    d
}

// ---- the procedures under test --------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Frontiere {
    AvantMetaVide, ApresMetaVide, PendantImage(usize), ApresImage, AvantFlushImage,
    ApresFlushImage, ApresVerification, PendantMetaCandidat, ApresMetaCandidat,
    ApresConsommationEssai, AvantCommitSante, PendantCommitSante, Fin,
}

struct Options { flush_apres_image: bool, flush_avant_boot: bool }
const CORRECT: Options = Options { flush_apres_image: true, flush_avant_boot: true };

/// What `updated` does to install version 2 into the inactive slot, then what
/// the loader and the health commit do on the next boot. `observe` is called
/// at each boundary with the disk as it is (cache included).
fn deroule(d: &mut Disque, opts: &Options, observe: &mut dyn FnMut(Frontiere, &Disque)) {
    let e0 = lit_meta(d).expect("etat initial");
    let en_cours = e0.actif;
    let cible = en_cours.autre();
    observe(Frontiere::AvantMetaVide, d);
    let e1 = e0.commence_installation(cible, en_cours).unwrap();
    ecrit_meta(d, &e1);
    d.flush();
    observe(Frontiere::ApresMetaVide, d);
    let img = image(2);
    for (i, s) in img.iter().enumerate() {
        d.ecrit(base(cible) + i, *s);
        observe(Frontiere::PendantImage(i), d);
    }
    observe(Frontiere::ApresImage, d);
    observe(Frontiere::AvantFlushImage, d);
    if opts.flush_apres_image { d.flush(); }
    observe(Frontiere::ApresFlushImage, d);
    let relu = empreinte_slot(d, cible);
    let annonce = empreinte_secteurs(&img);
    assert_eq!(relu, annonce, "relecture");
    observe(Frontiere::ApresVerification, d);
    let e2 = e1.marque_candidat(cible, annonce, relu, 3).unwrap();
    ecrit_meta(d, &e2);
    observe(Frontiere::PendantMetaCandidat, d);
    d.flush();
    observe(Frontiere::ApresMetaCandidat, d);
    // -- reboot: the loader
    let e3 = lit_meta(d).unwrap();
    let (slot, e4) = e3.choisit_boot();
    ecrit_meta(d, &e4);
    if opts.flush_avant_boot { d.flush(); }
    observe(Frontiere::ApresConsommationEssai, d);
    // -- the booted system reaches health
    observe(Frontiere::AvantCommitSante, d);
    let e5 = lit_meta(d).unwrap().sante_ok(slot);
    ecrit_meta(d, &e5);
    observe(Frontiere::PendantCommitSante, d);
    d.flush();
    observe(Frontiere::Fin, d);
}

/// The next boot after a power loss: what the loader would do.
enum Boot { Slot(Slot, Etat), RecoveryRequired }

fn prochain_boot(d: &Disque) -> Boot {
    match lit_meta(d) {
        None => Boot::RecoveryRequired,
        Some(e) => { let (s, _) = e.choisit_boot(); Boot::Slot(s, e.slot(s).etat) }
    }
}

fn verifie_coherent(d: &Disque, ou: Frontiere) -> Result<(), String> {
    match prochain_boot(d) {
        Boot::RecoveryRequired => Err(format!("{ou:?}: RECOVERY_REQUIRED apres coupure d'une procedure correcte")),
        Boot::Slot(s, Etat::Bon) => {
            let e = lit_meta(d).unwrap();
            if empreinte_slot(d, s) != e.slot(s).image {
                return Err(format!("{ou:?}: slot Bon {s:?} dont l'image ne correspond plus"));
            }
            Ok(())
        }
        Boot::Slot(s, Etat::Candidat { .. }) => {
            let e = lit_meta(d).unwrap();
            if empreinte_slot(d, s) != e.slot(s).image {
                return Err(format!("{ou:?}: candidat {s:?} boote avec une image non durable"));
            }
            Ok(())
        }
        Boot::Slot(s, autre) => Err(format!("{ou:?}: boot d'un slot {s:?} en etat {autre:?}")),
    }
}

struct Bilan { issues: usize, frontieres: Vec<Frontiere>, max_par_frontiere: usize }

fn explore(opts: &Options) -> Result<Bilan, String> {
    let mut d = installe();
    let mut bilan = Bilan { issues: 0, frontieres: Vec::new(), max_par_frontiere: 0 };
    let mut erreur: Option<String> = None;
    deroule(&mut d, opts, &mut |ou, disque| {
        if erreur.is_some() { return; }
        bilan.frontieres.push(ou);
        let coupures = disque.coupures();
        bilan.max_par_frontiere = bilan.max_par_frontiere.max(coupures.len());
        for apres in coupures {
            bilan.issues += 1;
            if let Err(e) = verifie_coherent(&apres, ou) { erreur = Some(e); return; }
        }
    });
    match erreur { Some(e) => Err(e), None => Ok(bilan) }
}

#[test]
fn coupure_a_chaque_frontiere_de_durabilite() {
    let b = explore(&CORRECT).unwrap_or_else(|e| panic!("{e}"));
    // Every boundary of the mission was actually reached, and the model did
    // explore non-trivial losses (a non-empty volatile cache somewhere).
    let attendues = [
        Frontiere::AvantMetaVide, Frontiere::ApresMetaVide, Frontiere::PendantImage(0),
        Frontiere::PendantImage(IMG_SECTEURS - 1), Frontiere::ApresImage, Frontiere::AvantFlushImage,
        Frontiere::ApresFlushImage, Frontiere::ApresVerification, Frontiere::PendantMetaCandidat,
        Frontiere::ApresMetaCandidat, Frontiere::ApresConsommationEssai, Frontiere::AvantCommitSante,
        Frontiere::PendantCommitSante, Frontiere::Fin,
    ];
    for f in attendues { assert!(b.frontieres.contains(&f), "frontiere non atteinte : {f:?}"); }
    assert!(b.max_par_frontiere > 1, "aucune coupure non triviale exploree");
    println!("UPDATE_DURABILITE frontieres={} issues={} max_par_frontiere={}",
             b.frontieres.len(), b.issues, b.max_par_frontiere);
}

#[test]
fn mutation_sans_flush_apres_image_est_detectee() {
    // Without the flush, the candidate metadata can become durable while some
    // image sectors are still only in the volatile cache.
    let r = explore(&Options { flush_apres_image: false, flush_avant_boot: true });
    let e = r.err().expect("le modele n'a pas vu un candidat sans image durable");
    assert!(e.contains("image non durable"), "{e}");
}

/// Boot loop with a candidate that freezes before health on every boot; each
/// freeze is a power cycle that loses everything not flushed.
fn boucle_de_gel(opts: &Options, cycles: usize) -> (usize, Slot) {
    let mut d = installe();
    // Install candidate B correctly first.
    let e0 = lit_meta(&d).unwrap();
    let e1 = e0.commence_installation(Slot::B, Slot::A).unwrap();
    ecrit_meta(&mut d, &e1); d.flush();
    for (i, s) in image(2).iter().enumerate() { d.ecrit(base(Slot::B) + i, *s); }
    d.flush();
    let h = empreinte_slot(&d, Slot::B);
    ecrit_meta(&mut d, &e1.marque_candidat(Slot::B, h, h, 3).unwrap()); d.flush();
    let mut boots_b = 0;
    let mut dernier = Slot::A;
    for _ in 0..cycles {
        let e = lit_meta(&d).expect("metadata");
        let (s, apres) = e.choisit_boot();
        ecrit_meta(&mut d, &apres);
        if opts.flush_avant_boot { d.flush(); }
        dernier = s;
        if s == Slot::B {
            boots_b += 1;
            // Freeze: power cycle, unflushed writes lost.
            d.cache.clear();
        } else {
            d.flush();
        }
    }
    (boots_b, dernier)
}

#[test]
fn candidat_qui_gele_est_borne_puis_rollback() {
    let (boots, dernier) = boucle_de_gel(&CORRECT, 20);
    assert_eq!(boots, 3, "le candidat doit etre boote exactement 3 fois");
    assert_eq!(dernier, Slot::A, "rollback vers A");
}

#[test]
fn mutation_boot_avant_flush_de_l_essai_est_detectee() {
    // The loader boots BEFORE the consumed attempt is durable: a freezing
    // candidate is booted forever. This is the failure §9 forbids.
    let (boots, dernier) = boucle_de_gel(&Options { flush_apres_image: true, flush_avant_boot: false }, 20);
    assert_eq!(boots, 20, "sans flush avant boot, le candidat est boote a chaque cycle");
    assert_eq!(dernier, Slot::B);
}

#[test]
fn deux_copies_invalides_exigent_le_recovery() {
    let mut d = installe();
    d.ecrit(META[0], [0xA5; SECT]);
    d.ecrit(META[1], [0x5A; SECT]);
    d.flush();
    assert!(matches!(prochain_boot(&d), Boot::RecoveryRequired), "fail closed : jamais deviner un slot");
}

#[test]
fn une_copie_corrompue_l_autre_est_utilisee() {
    for corrompue in 0..2 {
        let mut d = installe();
        let e = lit_meta(&d).unwrap();
        d.ecrit(META[corrompue], [0xFF; SECT]);
        d.flush();
        assert_eq!(lit_meta(&d), Some(e), "copie {corrompue} corrompue");
    }
}
