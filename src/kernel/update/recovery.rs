//! The minimal recovery environment (BOUCHAUD_RECOVERY_V1).
//!
//! Booted by the loader from the RECOVERY partition when:
//!   * both META copies are invalid (or a slot partition is missing);
//!   * the operator pressed R at the boot prompt;
//!   * the active slot failed to confirm health `echecs::SEUIL` boots in a row.
//!
//! It depends on nothing above the kernel: no compositor, no Ladybird, no
//! ring 3 at all. A text console on the framebuffer (mirrored to COM1) and
//! input from the keyboard OR the serial line. READ-ONLY by default: every
//! command that writes says so in its name and needs `--confirme`.
//!
//! Commands:
//!   aide                              this list
//!   etat                              handoff, A/B record, failure counter
//!   manifestes                        the image digest each slot is recorded with
//!   verifie A|B                       SHA-256 of the slot partition vs META
//!   boite-noire                       this boot's record + recent trace
//!   panique                           last panic (what is actually available)
//!   rollback --confirme               WRITE: active <- previous Good version
//!   remet-compteur --confirme         WRITE: failure counter <- 0
//!   redemarre | arrete
use alloc::string::String;

use bouchaud_update_core::{self as uc, Etat, Slot};

fn etat_txt(e: Etat) -> &'static str {
    match e { Etat::Vide => "vide", Etat::Candidat { .. } => "candidat", Etat::Bon => "bon", Etat::Mauvais => "mauvais" }
}

fn hex(o: &[u8]) -> String {
    let mut s = String::new();
    for b in o { s.push_str(&alloc::format!("{:02x}", b)); }
    s
}

fn etat() {
    match super::passage() {
        Some(p) => crate::println!("passage : raison={} slot_actif_vu={:?} boot_id={:#x} sequence={}",
                                   p.raison.texte(), p.slot, p.boot_id, p.sequence_meta),
        None => crate::println!("passage : absent"),
    }
    match super::meta_lecture() {
        Ok((e, compteur)) => {
            crate::println!("meta    : sequence={} actif={:?}", e.sequence, e.actif);
            for s in [Slot::A, Slot::B] {
                let i = e.slot(s);
                let essais = if let Etat::Candidat { essais } = i.etat { essais } else { 0 };
                crate::println!("slot {:?}  : {} essais={} image={}..", s, etat_txt(i.etat), essais, hex(&i.image[..8]));
            }
            crate::println!("echecs  : {} boot(s) actifs non confirmes (seuil {})", compteur, uc::echecs::SEUIL);
        }
        Err(r) => crate::println!("meta    : ILLISIBLE ({})", r),
    }
}

fn manifestes() {
    match super::meta_lecture() {
        Ok((e, _)) => for s in [Slot::A, Slot::B] {
            crate::println!("slot {:?} sha256={}", s, hex(&e.slot(s).image));
        },
        Err(r) => crate::println!("meta illisible ({}) : aucun manifeste enregistre", r),
    }
}

fn verifie(arg: Option<&str>) {
    let slot = match arg { Some("A") | Some("a") => Slot::A, Some("B") | Some("b") => Slot::B,
                           _ => { crate::println!("usage : verifie A|B"); return; } };
    match super::empreinte_slot(slot) {
        Ok((calcule, octets, attendu)) => {
            let ok = attendu == Some(calcule);
            crate::println!("slot {:?} : {} octets sha256={}", slot, octets, hex(&calcule));
            crate::println!("BOUCHAUD_RECOVERY_VERIFIE slot={:?} resultat={}", slot,
                            match attendu { None => "sans_reference", Some(_) if ok => "conforme", _ => "different" });
        }
        Err(r) => crate::println!("BOUCHAUD_RECOVERY_VERIFIE slot={:?} resultat=erreur raison={}", slot, r),
    }
}

fn boite_noire() {
    super::journalise_boot();
    let t = crate::drivers::serial::trace_snapshot();
    let debut = t.len().saturating_sub(2048);
    crate::println!("--- trace de ce boot ({} octets, 2048 derniers) ---", t.len());
    crate::println!("{}", String::from_utf8_lossy(&t[debut..]));
    crate::println!("--- fin ---");
}

fn execute(ligne: &str) -> bool {
    let mut mots = ligne.split_whitespace();
    let Some(cmd) = mots.next() else { return true };
    let arg = mots.next();
    let confirme = ligne.split_whitespace().any(|m| m == "--confirme");
    crate::serial_println!("BOUCHAUD_RECOVERY_CMD {}", cmd);
    match cmd {
        "aide" | "help" => crate::println!(
            "etat | manifestes | verifie A|B | boite-noire | panique | rollback --confirme | remet-compteur --confirme | redemarre | arrete"),
        "etat" => etat(),
        "manifestes" => manifestes(),
        "verifie" => verifie(arg),
        "boite-noire" => boite_noire(),
        "panique" => crate::println!(
            "derniere panique : non disponible ici -- la boite noire est ecrite sur la cle USB (tools/reference/extract-blackbox.py) ; ce boot : voir boite-noire"),
        "rollback" if !confirme => crate::println!("rollback ECRIT le bloc d'amorcage : actif <- version precedente Bonne. Retaper avec --confirme."),
        "rollback" => match super::rollback_manuel() {
            Ok(s) => crate::println!("BOUCHAUD_RECOVERY_ROLLBACK_OK actif={:?} (effet au prochain demarrage)", s),
            Err(r) => crate::println!("BOUCHAUD_RECOVERY_ROLLBACK_REFUS raison={}", r),
        },
        "remet-compteur" if !confirme => crate::println!("remet-compteur ECRIT le compteur d'echecs a 0. Retaper avec --confirme."),
        "remet-compteur" => match super::remet_compteur() {
            Ok(()) => crate::println!("BOUCHAUD_RECOVERY_COMPTEUR_OK"),
            Err(r) => crate::println!("BOUCHAUD_RECOVERY_COMPTEUR_REFUS raison={}", r),
        },
        "redemarre" | "reboot" => { crate::println!("redemarrage"); crate::kernel::power::reboot() }
        "arrete" | "poweroff" => crate::kernel::power::shutdown_avec_raison(crate::kernel::power::EXIT_OK, "recovery"),
        _ => crate::println!("commande inconnue : {} (aide)", cmd),
    }
    true
}

/// Never returns. Stage 2 calls it instead of starting serviced and the
/// desktop when the handoff says RECOVERY.
pub fn run() -> ! {
    crate::drivers::vga::set_serial_mirror(true);
    let raison = super::passage().map(|p| p.raison.texte()).unwrap_or("sans_passage");
    crate::println!("");
    crate::println!("=== Bouchaud OS -- RECOVERY ({}) ===", raison);
    crate::println!("lecture seule par defaut ; les ecritures exigent --confirme. 'aide' pour la liste.");
    crate::serial_println!("BOUCHAUD_RECOVERY_PRET raison={}", raison);
    etat();
    let mut ligne = String::new();
    crate::print!("recovery> ");
    loop {
        let mut octet = crate::drivers::serial::lit_octet();
        if octet.is_none() {
            octet = match crate::drivers::keyboard::try_key() {
                Some(crate::drivers::keyboard::Key::Char(c)) => Some(c),
                Some(crate::drivers::keyboard::Key::Enter) => Some(b'\n'),
                Some(crate::drivers::keyboard::Key::Backspace) => Some(8),
                _ => None,
            };
        }
        match octet {
            Some(b'\r') | Some(b'\n') => {
                crate::println!("");
                let l = core::mem::take(&mut ligne);
                execute(l.trim());
                crate::print!("recovery> ");
            }
            Some(8) | Some(127) => { ligne.pop(); }
            Some(c) if (0x20..0x7f).contains(&c) && ligne.len() < 120 => { ligne.push(c as char); crate::print!("{}", c as char); }
            _ => core::hint::spin_loop(),
        }
    }
}
