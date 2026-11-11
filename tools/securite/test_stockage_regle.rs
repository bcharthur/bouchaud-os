//! BOUCHAUD_CLE_USB_TEST_V1 : la regle des commandes de stockage d'une image
//! d'essai sur cle USB (`src/drivers/api/stockage_regle.rs`, incluse telle
//! quelle).
//!
//! Ce qui est cherche ici, ce sont les REFUS : une ecriture, une vidange, un
//! formatage, un effacement, un TRIM, une commande inconnue ne doivent
//! jamais partir vers un disque interne -- ni vers une cle USB dont l'identite
//! n'est pas etablie au-dela du doute.
//!
//! Lance par `tools/ci/run_host_tests.sh`.

#![allow(dead_code)]

#[path = "../../src/drivers/api/stockage_regle.rs"]
mod regle;

use regle::*;

const CLE: bool = true;

fn v(support: Support, identite: Identite, classe: Classe) -> Verdict {
    verdict(CLE, support, identite, false, classe)
}

// --- Les classes -------------------------------------------------------------

#[test]
fn ata_lectures_permises_ecritures_et_le_reste_non() {
    for c in [0xEC, 0xA1, 0x20, 0x24, 0xC8, 0x25, 0x40, 0xE5] {
        assert_eq!(classe_ata(c), Classe::SansEffet, "{c:#x}");
    }
    for c in [0x30, 0x34, 0xCA, 0x35, 0xC5, 0x39, 0x3D, 0xE7, 0xEA] {
        assert_eq!(classe_ata(c), Classe::Ecriture, "{c:#x}");
    }
    // TRIM, SECURITY ERASE PREPARE/UNIT, DOWNLOAD MICROCODE, SET FEATURES,
    // SET MAX, SANITIZE, et ce que personne n'a liste.
    for c in [0x06, 0xF3, 0xF4, 0x92, 0x93, 0xEF, 0xF9, 0xB4, 0x00, 0xFF, 0x91] {
        assert_eq!(classe_ata(c), Classe::Interdite, "{c:#x}");
    }
}

#[test]
fn toute_commande_ata_inconnue_est_interdite() {
    let connues = [
        0xEC, 0xA1, 0x20, 0x21, 0x24, 0xC8, 0xC9, 0x25, 0xC4, 0x29, 0x40, 0x41, 0x42, 0xE5,
        0x30, 0x31, 0x34, 0xCA, 0xCB, 0x35, 0xC5, 0x39, 0x3D, 0xE7, 0xEA,
    ];
    for c in 0u8..=255 {
        if !connues.contains(&c) {
            assert_eq!(classe_ata(c), Classe::Interdite, "{c:#x}");
        }
    }
}

fn sqe(opcode: u8, cdw10: u32) -> [u32; 16] {
    let mut s = [0u32; 16];
    s[0] = opcode as u32 | (0x1234 << 16);
    s[10] = cdw10;
    s
}

#[test]
fn nvme_admin_seules_les_files_et_l_identification() {
    for op in [0x00, 0x01, 0x02, 0x04, 0x05, 0x06, 0x08, 0x0A, 0x0C] {
        assert_eq!(classe_nvme(true, &sqe(op, 0)), Classe::SansEffet, "{op:#x}");
    }
    // Set Features : le nombre de files seulement, et jamais « sauve ».
    assert_eq!(classe_nvme(true, &sqe(0x09, 0x07)), Classe::SansEffet);
    assert_eq!(classe_nvme(true, &sqe(0x09, 0x07 | 1 << 31)), Classe::Interdite);
    assert_eq!(classe_nvme(true, &sqe(0x09, 0x06)), Classe::Interdite, "cache d'ecriture volatil");
    assert_eq!(classe_nvme(true, &sqe(0x09, 0x02)), Classe::Interdite, "gestion d'energie");
    // Namespace Management/Attachment, Firmware Commit/Download, Format NVM,
    // Security Send, Sanitize, Device Self-test, propres au fabricant.
    for op in [0x0D, 0x15, 0x10, 0x11, 0x80, 0x81, 0x84, 0x14, 0xC0, 0xFF] {
        assert_eq!(classe_nvme(true, &sqe(op, 0)), Classe::Interdite, "{op:#x}");
    }
}

#[test]
fn nvme_entree_sortie_lecture_seule_permise() {
    assert_eq!(classe_nvme(false, &sqe(0x02, 0)), Classe::SansEffet);
    assert_eq!(classe_nvme(false, &sqe(0x00, 0)), Classe::Ecriture, "flush");
    assert_eq!(classe_nvme(false, &sqe(0x01, 0)), Classe::Ecriture, "write");
    // Write Uncorrectable, Compare, Write Zeroes, DSM, Reservation, Copy.
    for op in [0x04, 0x05, 0x08, 0x09, 0x0D, 0x0E, 0x11, 0x15, 0x19, 0x7D, 0x80] {
        assert_eq!(classe_nvme(false, &sqe(op, 0)), Classe::Interdite, "{op:#x}");
    }
}

#[test]
fn nvme_une_ecriture_n_est_jamais_lue_comme_une_commande_admin_permise() {
    // La file d'entree-sortie prise pour la file admin ferait de 0x01
    // (Write) un `Create I/O SQ`. Le pilote deduit `admin` de l'identifiant
    // de la file ; ici on fixe ce que chaque lecture donnerait.
    assert_eq!(classe_nvme(true, &sqe(0x01, 0)), Classe::SansEffet);
    assert_eq!(classe_nvme(false, &sqe(0x01, 0)), Classe::Ecriture);
    assert!(!v(Support::Nvme, Identite::Autorisee, classe_nvme(false, &sqe(0x01, 0))).permis());
}

#[test]
fn scsi_lectures_permises_ecritures_et_le_reste_non() {
    for cdb in [&[0x00u8][..], &[0x03], &[0x12], &[0x1A], &[0x5A], &[0x23], &[0x25], &[0x28], &[0xA8], &[0x88], &[0x9E, 0x10]] {
        assert_eq!(classe_scsi(cdb), Classe::SansEffet, "{cdb:02x?}");
    }
    for op in [0x2A, 0xAA, 0x8A, 0x35, 0x91] {
        assert_eq!(classe_scsi(&[op]), Classe::Ecriture, "{op:#x}");
    }
    // FORMAT UNIT, UNMAP, WRITE SAME (10/16), WRITE AND VERIFY, WRITE BUFFER,
    // MODE SELECT, START STOP UNIT, ATA PASS-THROUGH (12/16), SECURITY
    // PROTOCOL OUT, SANITIZE, PREVENT ALLOW MEDIUM REMOVAL.
    for op in [0x04, 0x42, 0x41, 0x93, 0x2E, 0xAE, 0x8E, 0x3B, 0x15, 0x55, 0x1B, 0xA1, 0x85, 0xB5, 0x48, 0x1E] {
        assert_eq!(classe_scsi(&[op, 0, 0, 0, 0, 0]), Classe::Interdite, "{op:#x}");
    }
    // SERVICE ACTION IN autre que READ CAPACITY (16), CDB vide.
    assert_eq!(classe_scsi(&[0x9E, 0x11]), Classe::Interdite);
    assert_eq!(classe_scsi(&[0x9E]), Classe::Interdite);
    assert_eq!(classe_scsi(&[]), Classe::Interdite);
}

// --- Le verdict ------------------------------------------------------------

#[test]
fn image_standard_tout_est_permis() {
    for support in [Support::Ata, Support::Nvme, Support::Usb] {
        for classe in [Classe::SansEffet, Classe::Ecriture, Classe::Interdite] {
            assert!(verdict(false, support, Identite::Inconnue, true, classe).permis());
        }
    }
}

#[test]
fn disques_internes_jamais_ecrits_quelle_que_soit_l_identite() {
    for support in [Support::Ata, Support::Nvme] {
        for identite in [Identite::Inconnue, Identite::Etrangere, Identite::Autorisee] {
            assert_eq!(v(support, identite, Classe::Ecriture), Verdict::Refuse(Raison::DisqueInterne));
            assert_eq!(v(support, identite, Classe::Interdite), Verdict::Refuse(Raison::DisqueInterne));
            assert!(v(support, identite, Classe::SansEffet).permis(), "les lectures restent permises");
        }
    }
}

#[test]
fn usb_ecriture_seulement_sur_le_support_autorise_et_non_ambigu() {
    assert!(v(Support::Usb, Identite::Autorisee, Classe::Ecriture).permis());
    assert_eq!(v(Support::Usb, Identite::Etrangere, Classe::Ecriture), Verdict::Refuse(Raison::SupportNonAutorise));
    assert_eq!(v(Support::Usb, Identite::Inconnue, Classe::Ecriture), Verdict::Refuse(Raison::IdentiteIncertaine));
    assert_eq!(
        verdict(CLE, Support::Usb, Identite::Autorisee, true, Classe::Ecriture),
        Verdict::Refuse(Raison::IdentiteAmbigue)
    );
}

#[test]
fn usb_commandes_interdites_meme_sur_le_support_autorise() {
    for identite in [Identite::Inconnue, Identite::Etrangere, Identite::Autorisee] {
        assert_eq!(v(Support::Usb, identite, Classe::Interdite), Verdict::Refuse(Raison::CommandeInterdite));
        assert_eq!(v(Support::Usb, identite, classe_scsi(&[0x85; 16])), Verdict::Refuse(Raison::CommandeInterdite), "ATA PASS-THROUGH");
    }
}

// --- Le GUID grave a la construction ----------------------------------------

#[test]
fn guid_autorise_exige_32_chiffres_hexadecimaux_non_nuls() {
    let g = guid_autorise(Some("00112233445566778899AaBbCcDdEeFf")).unwrap();
    assert_eq!(g, [0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
    for faux in [
        None,
        Some(""),
        Some("00112233445566778899aabbccddeef"),
        Some("00112233445566778899aabbccddeeff0"),
        Some("00112233-4455-6677-8899-aabbccddeeff"),
        Some("0011223344556677889Gaabbccddeeff"),
        Some("00000000000000000000000000000000"),
        Some(" 0112233445566778899aabbccddeeff"),
    ] {
        assert_eq!(guid_autorise(faux), None, "{faux:?}");
    }
}

// --- L'en-tete GPT -----------------------------------------------------------

const GUID: [u8; 16] = [0xA5, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 0x5A];

/// Un en-tete GPT primaire minimal, somme comprise.
fn entete(guid: [u8; 16], mon_lba: u64) -> Vec<u8> {
    let mut b = vec![0u8; 512];
    b[0..8].copy_from_slice(b"EFI PART");
    b[8..12].copy_from_slice(&0x0001_0000u32.to_le_bytes());
    b[12..16].copy_from_slice(&92u32.to_le_bytes());
    b[24..32].copy_from_slice(&mon_lba.to_le_bytes());
    b[56..72].copy_from_slice(&guid);
    let somme = crc32(&b[..92]);
    b[16..20].copy_from_slice(&somme.to_le_bytes());
    b
}

#[test]
fn crc32_valeur_de_reference() {
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
}

#[test]
fn identite_autorisee_si_et_seulement_si_le_guid_correspond() {
    let b = entete(GUID, 1);
    assert_eq!(guid_disque(&b), Some(GUID));
    assert_eq!(identite(Some(&b), Some(GUID)), Identite::Autorisee);
    let mut autre = GUID;
    autre[15] ^= 1;
    assert_eq!(identite(Some(&b), Some(autre)), Identite::Etrangere);
    assert_eq!(identite(Some(&b), None), Identite::Etrangere, "noyau sans GUID : rien n'est autorise");
    assert_eq!(identite(None, Some(GUID)), Identite::Inconnue, "illisible");
}

#[test]
fn en_tete_corrompu_ou_deguise_n_autorise_rien() {
    let bon = entete(GUID, 1);
    // Un octet du GUID change SANS recalculer la somme : quelqu'un a copie le
    // GUID ailleurs, ou le bloc est abime.
    let mut b = bon.clone();
    b[60] ^= 0xFF;
    assert_eq!(identite(Some(&b), Some(GUID)), Identite::Inconnue);
    // Signature fausse.
    let mut b = bon.clone();
    b[0] = b'X';
    assert_eq!(identite(Some(&b), Some(GUID)), Identite::Inconnue);
    // L'en-tete de SECOURS (MyLBA != 1) presente en LBA 1.
    assert_eq!(identite(Some(&entete(GUID, 999)), Some(GUID)), Identite::Inconnue);
    // Taille annoncee absurde.
    let mut b = bon.clone();
    b[12..16].copy_from_slice(&4096u32.to_le_bytes());
    assert_eq!(identite(Some(&b), Some(GUID)), Identite::Inconnue);
    let mut b = bon.clone();
    b[12..16].copy_from_slice(&91u32.to_le_bytes());
    assert_eq!(identite(Some(&b), Some(GUID)), Identite::Inconnue);
    // Bloc tronque.
    assert_eq!(identite(Some(&bon[..91]), Some(GUID)), Identite::Inconnue);
    assert_eq!(identite(Some(&[]), Some(GUID)), Identite::Inconnue);
}

#[test]
fn table_gpt_ecrite_par_un_outil_independant() {
    // `tools/ci/fabrique-disque-gpt.py` (lance par run_host_tests.sh) ne
    // partage pas une ligne avec le noyau. Son GUID de disque, CAFEBABE-DEAD-
    // BEEF-0102-030405060708, est ecrit en ordre MIXTE : c'est ce qu'il faut
    // retrouver octet pour octet.
    let Ok(chemin) = std::env::var("BO_DISQUE_GPT") else {
        eprintln!("BO_DISQUE_GPT absent : test de la table independante non execute");
        return;
    };
    let disque = std::fs::read(chemin).expect("image GPT");
    let attendu = [0xBE, 0xBA, 0xFE, 0xCA, 0xAD, 0xDE, 0xEF, 0xBE, 1, 2, 3, 4, 5, 6, 7, 8];
    assert_eq!(guid_disque(&disque[512..1024]), Some(attendu));
    assert_eq!(identite(Some(&disque[512..1024]), Some(attendu)), Identite::Autorisee);
}
