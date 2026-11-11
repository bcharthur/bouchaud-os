//! P13_PASSATION_SOUS_IF_COUPE_V1 (#264) : une passation n'est publiee
//! qu'interruptions coupees, de `prepare_switch_handoff` jusqu'a sa
//! completion. Garde structurel avec temoins negatifs ; la preuve en noyau
//! reel reste le soak SMP4 et la qualification P13 sous KVM.
use std::fs;

fn lire(p: &str) -> String {
    fs::read_to_string(p).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn corps<'a>(s: &'a str, signature: &str) -> &'a str {
    let a = s.find(signature).unwrap_or_else(|| panic!("{signature} introuvable"));
    let o = a + s[a..].find('{').unwrap();
    let mut p = 0i32;
    for (i, c) in s[o..].char_indices() {
        p += (c == '{') as i32 - (c == '}') as i32;
        if p == 0 {
            return &s[a..o + i + 1];
        }
    }
    panic!("corps non ferme")
}

fn regle(ordo: &str, courant: &str) -> Result<(), String> {
    let sw = corps(ordo, "fn switch_to(from: usize, to: usize)");
    let pos = |t: &str, m: &str| t.find(m).ok_or_else(|| format!("absent : {m}"));
    let coupe = pos(sw, "interrupts::disable();")?;
    let prep = pos(sw, "prepare_switch_handoff(")?;
    let bascule = pos(sw, "switch_context(")?;
    let fin = pos(sw, "complete_switch_handoff();")?;
    let rend = pos(sw, "interrupts::enable();")?;
    let avant = pos(sw, "let if_avant = interrupts::are_enabled();")?;
    if !(avant < coupe && coupe < prep && prep < bascule && bascule < fin && fin < rend) {
        return Err("switch_to : IRQ non coupees de la publication a la completion".into());
    }
    if sw.contains("cpu::interrupts_enabled()") {
        return Err("switch_to : IF relu apres la coupure (attribution faussee)".into());
    }
    for f in ["fn switch_to_kernel(", "fn reprend_continuation("] {
        let c = corps(ordo, f);
        let d = pos(c, "interrupts::disable();")?;
        let a = pos(c, "abandonne_la_tache_courante(")?;
        if d > a {
            return Err(format!("{f} : abandon avant la coupure des IRQ"));
        }
    }
    let pr = corps(courant, "fn prepare_switch_handoff(");
    let garde = pos(pr, "!interrupts::are_enabled()")?;
    let cas = pos(pr, "SWITCH_PENDING[cpu].compare_exchange(")?;
    if garde > cas {
        return Err("prepare_switch_handoff : publication avant la verification IF".into());
    }
    Ok(())
}

#[test]
fn passation_publiee_et_terminee_interruptions_coupees() {
    let ordo = lire("src/kernel/process/thread/ordonnancement.rs");
    let courant = lire("src/kernel/process/thread/courant.rs");
    regle(&ordo, &courant).unwrap();
}

#[test]
fn temoins_negatifs() {
    let ordo = lire("src/kernel/process/thread/ordonnancement.rs");
    let courant = lire("src/kernel/process/thread/courant.rs");
    let sw = corps(&ordo, "fn switch_to(from: usize, to: usize)").to_string();
    // 1. la coupure retiree (l'etat d'avant #264)
    let m1 = ordo.replacen(&sw, &sw.replacen("    interrupts::disable();\n", "", 1), 1);
    assert!(regle(&m1, &courant).is_err(), "coupure retiree non detectee");
    // 2. IF rendu avant la completion
    let sw2 = sw.replacen("    interrupts::disable();\n", "    interrupts::disable();\n    interrupts::enable();\n", 1);
    let m2 = ordo.replacen(&sw, &sw2, 1);
    assert!(regle(&m2, &courant).is_err(), "IF rendu trop tot non detecte");
    // 3. garde de prepare_switch_handoff retiree
    let m3 = courant.replacen("!interrupts::are_enabled()", "true", 1);
    assert!(regle(&ordo, &m3).is_err(), "garde IF retiree non detectee");
    // 4. sortie definitive sans coupure
    let m4 = ordo.replacen("    // P13_PASSATION_SOUS_IF_COUPE_V1 : idem `switch_to_kernel`.\n    interrupts::disable();\n", "", 1);
    assert!(regle(&m4, &courant).is_err(), "reprend_continuation sans coupure non detecte");
}
