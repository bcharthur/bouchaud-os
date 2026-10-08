//! `ata-integrite` : ecrire, vider, relire et comparer, sur le disque de
//! donnees, par la couche bloc -- donc par le chemin DMA du pilote ATA et son
//! repli PIO.
//!
//! BOUCHAUD_ATA_INTEGRITE_V1
//!
//! Les sondes existantes exercent le disque A TRAVERS le systeme de fichiers
//! (disque-probe, wal-probe, ata-concurrence-probe, persistance sur deux
//! demarrages) : elles disent qu'un fichier revient, pas qu'un SECTEUR revient
//! tel qu'il a ete ecrit, ni ce qui arrive quand le disque refuse une
//! ecriture. Ce test :
//!
//!   1. choisit une zone tampon de 2 048 secteurs juste AVANT la zone
//!      persistante, et refuse si elle chevauche l'archive (fichiers relus a
//!      la demande) ;
//!   2. en sauve le contenu ;
//!   3. y ecrit huit plages de tailles choisies pour franchir les frontieres
//!      du pilote (1, 7, 8, 255, 256, 257, 1 024, 240 secteurs : un lot DMA
//!      fait au plus 256 secteurs), d'un motif pseudo-aleatoire reproductible
//!      par secteur ;
//!   4. vide le cache (la barriere de la couche bloc) ;
//!   5. relit chaque plage DECLAREE ecrite et la compare octet par octet --
//!      une plage declaree ecrite qui revient fausse est une corruption
//!      SILENCIEUSE, la seule faute impardonnable ; une plage refusee doit
//!      l'avoir ete bruyamment ;
//!   6. relit la zone entiere d'un seul appel (lots enchaines) ;
//!   7. repose le contenu d'origine, vide, et le verifie.
//!
//! `ata-integrite --refus-attendu` : la meme chose quand le banc injecte des
//! erreurs d'ecriture (QEMU blkdebug) -- le verdict exige alors qu'aucune
//! corruption ne soit silencieuse, et qu'au moins un refus ait ete VU (repli
//! PIO compte, ou plage declaree en echec).

use alloc::vec;

use crate::drivers::bloc::{self, Achevement, Volume};

const ZONE: u64 = 2048;
const MARGE: u64 = 64;
const PLAGES: [(u64, usize); 8] = [
    (0, 1), (1, 7), (8, 8), (16, 255), (271, 256), (527, 257), (784, 1024), (1808, 240),
];
const OCTETS: usize = 512;

/// Le motif d'un secteur : xorshift64 graine par (graine, lba).
fn motif(graine: u64, lba: u64, secteur: &mut [u8]) {
    let mut x = graine ^ lba.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xB0C0_DA7A;
    if x == 0 {
        x = 1;
    }
    for mot in secteur.chunks_exact_mut(8) {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        mot.copy_from_slice(&x.to_le_bytes());
    }
}

fn fnv(acc: u64, donnees: &[u8]) -> u64 {
    let mut h = acc;
    for &o in donnees {
        h ^= o as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn fait(a: Achevement, n: usize) -> bool {
    matches!(a, Achevement::Fait(k) if k == n)
}

pub fn commande(argc: usize, argv: &[&str; 12]) {
    let refus_attendu = argc >= 2 && argv[1] == "--refus-attendu";
    let d = bloc::descripteur(Volume::DONNEES);
    let Some(zone_persist) = crate::fs::persistance::premier_secteur_de_la_zone() else {
        crate::serial_println!("ATA_INTEGRITE verdict=refuse raison=pas-de-zone-persistante blocs={}", d.blocs);
        return;
    };
    let fin_archive = crate::fs::tar::fin_archive_secteur();
    let Some(base) = zone_persist.checked_sub(ZONE + MARGE) else {
        crate::serial_println!("ATA_INTEGRITE verdict=refuse raison=volume-trop-petit");
        return;
    };
    if base < fin_archive.saturating_add(MARGE) {
        crate::serial_println!(
            "ATA_INTEGRITE verdict=refuse raison=chevauche-archive base={} fin_archive={}",
            base, fin_archive
        );
        return;
    }

    let (lus_dma_avant, replis_avant, _) = crate::drivers::ata::dma_stats();
    let ecrits_dma_avant = crate::drivers::ata::dma_lots_ecrits();
    let debut_ns = crate::kernel::timer::monotonic_ns();

    // 2. Sauvegarde.
    let mut origine = vec![0u8; ZONE as usize * OCTETS];
    if !fait(bloc::lit(Volume::DONNEES, base, ZONE as usize, &mut origine), ZONE as usize) {
        crate::serial_println!("ATA_INTEGRITE verdict=echec phase=sauvegarde base={}", base);
        return;
    }

    // 3. Ecriture des plages.
    let graine = crate::kernel::timer::monotonic_ns() | 1;
    let mut ecrite = [false; PLAGES.len()];
    let mut ecritures_ko = 0u32;
    for (i, &(decalage, n)) in PLAGES.iter().enumerate() {
        let mut donnees = vec![0u8; n * OCTETS];
        for s in 0..n {
            motif(graine, base + decalage + s as u64, &mut donnees[s * OCTETS..(s + 1) * OCTETS]);
        }
        ecrite[i] = fait(bloc::ecrit(Volume::DONNEES, base + decalage, n, &donnees), n);
        if !ecrite[i] {
            ecritures_ko += 1;
            crate::serial_println!("ATA_INTEGRITE plage={} lba={} secteurs={} ecriture=refusee", i, base + decalage, n);
        }
    }

    // 4. La barriere.
    let vidange = bloc::vidange(Volume::DONNEES);

    // 5. Relecture plage par plage.
    let mut corrompus = 0u64;
    let mut verifies = 0u64;
    let mut somme = 0xcbf2_9ce4_8422_2325u64;
    let mut attendu = [0u8; OCTETS];
    for (i, &(decalage, n)) in PLAGES.iter().enumerate() {
        if !ecrite[i] {
            continue;
        }
        let mut relu = vec![0u8; n * OCTETS];
        if !fait(bloc::lit(Volume::DONNEES, base + decalage, n, &mut relu), n) {
            crate::serial_println!("ATA_INTEGRITE plage={} relecture=refusee", i);
            corrompus += n as u64;
            continue;
        }
        for s in 0..n {
            motif(graine, base + decalage + s as u64, &mut attendu);
            let secteur = &relu[s * OCTETS..(s + 1) * OCTETS];
            somme = fnv(somme, secteur);
            verifies += 1;
            if secteur != attendu {
                corrompus += 1;
                if corrompus <= 4 {
                    crate::serial_println!(
                        "ATA_INTEGRITE CORRUPTION plage={} lba={} premier_octet_lu={:#04x} attendu={:#04x}",
                        i, base + decalage + s as u64, secteur[0], attendu[0]
                    );
                }
            }
        }
    }

    // 6. Relecture d'un seul appel, lots enchaines.
    let mut zone = vec![0u8; ZONE as usize * OCTETS];
    let mut croises_ko = 0u64;
    if fait(bloc::lit(Volume::DONNEES, base, ZONE as usize, &mut zone), ZONE as usize) {
        for (i, &(decalage, n)) in PLAGES.iter().enumerate() {
            if !ecrite[i] {
                continue;
            }
            for s in 0..n {
                let lba = base + decalage + s as u64;
                motif(graine, lba, &mut attendu);
                let at = (decalage as usize + s) * OCTETS;
                if zone[at..at + OCTETS] != attendu {
                    croises_ko += 1;
                }
            }
        }
    } else {
        croises_ko = u64::MAX;
    }

    // 7. Restauration.
    let restaure_ecrit = fait(bloc::ecrit(Volume::DONNEES, base, ZONE as usize, &origine), ZONE as usize);
    let restaure_vide = bloc::vidange(Volume::DONNEES);
    let mut relu = vec![0u8; ZONE as usize * OCTETS];
    let restaure = restaure_ecrit
        && fait(bloc::lit(Volume::DONNEES, base, ZONE as usize, &mut relu), ZONE as usize)
        && relu == origine;

    let (lus_dma, replis, dma_pret) = crate::drivers::ata::dma_stats();
    let ecrits_dma = crate::drivers::ata::dma_lots_ecrits();
    let replis_delta = replis.saturating_sub(replis_avant);
    let duree_ms = (crate::kernel::timer::monotonic_ns() - debut_ns) / 1_000_000;
    crate::serial_println!(
        "ATA_INTEGRITE base={} secteurs={} verifies={} corrompus={} relecture_croisee_ko={} ecritures_refusees={} vidange={} restaure={} restaure_vidange={} lots_dma_ecrits={} lots_dma_lus={} replis_pio={} dma_pret={} somme={:#018x} duree_ms={}",
        base, ZONE, verifies, corrompus,
        if croises_ko == u64::MAX { -1 } else { croises_ko as i64 },
        ecritures_ko, vidange as u8, restaure as u8, restaure_vide as u8,
        ecrits_dma.saturating_sub(ecrits_dma_avant), lus_dma.saturating_sub(lus_dma_avant),
        replis_delta, dma_pret as u8, somme, duree_ms,
    );

    // Verdict. Jamais de corruption silencieuse ; zone rendue intacte.
    let sain = corrompus == 0 && croises_ko == 0 && restaure;
    let ok = if refus_attendu {
        // Un refus doit avoir ete VU : repli PIO, ou plage declaree en echec.
        sain && (replis_delta > 0 || ecritures_ko > 0)
    } else {
        sain && ecritures_ko == 0 && vidange && verifies == ZONE
    };
    if ok {
        crate::serial_println!(
            "ATA_INTEGRITE_OK mode={} verifies={} replis_pio={} ecritures_refusees={}",
            if refus_attendu { "refus-attendu" } else { "nominal" }, verifies, replis_delta, ecritures_ko
        );
    } else {
        crate::serial_println!(
            "ATA_INTEGRITE_ECHEC mode={} corrompus={} croises_ko={} restaure={} vidange={} ecritures_refusees={} replis_pio={}",
            if refus_attendu { "refus-attendu" } else { "nominal" },
            corrompus, croises_ko as i64, restaure as u8, vidange as u8, ecritures_ko, replis_delta
        );
    }
}
