#!/usr/bin/env python3
"""Une ligne serie s'emet interruptions masquees, jeton tenu par son contexte.

BOUCHAUD_JETON_SERIE_CONTEXTE_V1 (remplace BOUCHAUD_JETON_SERIE_PROPRIETAIRE_V1)

Le jeton marque du seul numero de coeur laissait passer sans attendre tout
ecrivain du meme coeur : une tache preemptee en tenant le jeton laissait la
suivante meler sa ligne a la sienne, et une IRQ inserait son texte au milieu
de la ligne interrompue (mesure : `PROCESS_DEATH ... image=/bin[...]
[SMP-SNAPSHOT]...`). Sa priorite booleenne etait rendue par le premier de
deux ecrivains prioritaires pendant que le second attendait.

Le garde exige, dans `_print` :
  1. formatage AVANT les interruptions masquees ;
  2. interruptions masquees avant la prise du jeton, rendues (si elles etaient
     ouvertes) seulement APRES l'emission et la remise du jeton ;
  3. un jeton pris par CAS 0 -> cpu_index() + 1, rendu une fois, seulement
     s'il a ete pris ;
  4. la reentrance (meme coeur) comptee, jamais attendue ;
  5. l'attente bornee, et les shootdowns servis pendant l'attente ;
  6. plus de priorite booleenne ni de jeton tenu sur plusieurs lignes ;
  7. les compteurs publies (`[SONDE-IRQ-DUREE] ... serie_reentrees= serie_bornes=`).

BOUCHAUD_SERIE_ECRITURE_ATOMIQUE_V1 : ces regles portent sur `sous_jeton`,
le SEUL chemin qui prend le jeton, et l'exigent de ses deux appelants :
  8. `_print` formate hors du jeton, puis emet par `sous_jeton(|| sortie.vide())` ;
  9. `ecris_lignes` (sortie d'un `write(2)` userland) emet chaque ligne par
     `sous_jeton(|| ecris_octets(ligne))` ;
 10. `console_write` passe par `ecris_lignes` : plus aucun appel a
     `ecris_octets` hors du pilote ;
 11. un appelant interruptible qui perd la course masquee rouvre les
     interruptions et REFAIT LA QUEUE (`continue`) tant que l'echeance datee
     n'est pas passee ; il n'emet sans jeton qu'a cette echeance. Avant, il
     attendait masque 100 000 tours puis emettait sans jeton -- sous KVM,
     moins que le temps d'une ligne du gagnant : `pid=1\x1b6[ 90kmi[l...`
     (run 38482972584) ;
 12. le debordement du tampon de formatage (ligne > 2 Kio) se vide sous le
     jeton ;
 14. BOUCHAUD_SERIE_JAMAIS_SANS_JETON_V1 : hors panique, une ligne sans jeton
     (borne d'un contexte masque, renoncement, reentrance) n'est JAMAIS
     emise : `attente::repli(false)` rend `Differe`, la ligne est rangee
     entiere dans l'anneau de son coeur, et le detenteur suivant vide les
     anneaux AVANT sa propre ligne. Seul le gestionnaire de panique bascule
     (`passe_en_panique`) ; l'autorun vide les anneaux avant le bilan. Run
     38608429652 (KVM SMP4) : 3 lignes emises sans jeton par des contextes
     masques a leur borne courte, malgre la regle 13.
 13. BOUCHAUD_SERIE_ATTENTE_PROGRES_V1 : l'echeance d'un appelant
     interruptible est une seconde SANS PROGRES, renouvelee par
     `attente::a_l_echeance` tant que d'autres emettent (le compteur
     EMISSIONS_FAITES n'avance que sous le jeton), avec un plafond absolu
     FINI ; chaque emission sans jeton garde sa cause. Run 38603003639 (KVM
     SMP4) : une ligne emise sans jeton apres une seconde de file derriere
     des emetteurs qui progressaient, au milieu de celle d'un autre.

Fail-closed ; vingt et un tests negatifs.
"""
import re
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
UART = "src/drivers/serial/uart16550.rs"
SONDE = "src/kernel/process/thread/diagnostic_stall.rs"
CONSOLE = "src/compat/linux/file.rs"
ATTENTE = "src/drivers/serial/attente.rs"
PANIQUE = "src/kernel/debug/panic.rs"
AUTORUN = "src/kernel/autorun.rs"
FICHIERS = (UART, SONDE, CONSOLE, ATTENTE, PANIQUE, AUTORUN)


def sans_commentaires(t: str) -> str:
    return "\n".join(l.split("//", 1)[0] for l in t.splitlines())


def corps(texte: str, nom: str) -> str:
    m = re.search(r"\bfn\s+" + re.escape(nom) + r"\s*(?:<[^>{]*>)?\s*\(", texte)
    if not m:
        return ""
    o = texte.find("{", m.end())
    p = 0
    for i in range(o, len(texte)):
        p += texte[i] == "{"
        p -= texte[i] == "}"
        if p == 0:
            return texte[o:i + 1]
    return ""


def position(texte: str, motif: str, rang: int = 1) -> int:
    i = -1
    for _ in range(rang):
        i = texte.find(motif, i + 1)
        if i < 0:
            return 10**9
    return i


def verifie(racine: Path) -> list[str]:
    try:
        src = {f: sans_commentaires((racine / f).read_text(encoding="utf-8")) for f in FICHIERS}
    except OSError as e:
        return [f"lecture impossible : {e}"]
    fautes = []
    u = src[UART]
    impr = corps(u, "_print")
    p = corps(u, "sous_jeton")
    if not p:
        return [f"{UART} : `fn sous_jeton` introuvable"]
    fmt = position(impr, "sortie.write_fmt(args)")
    if not fmt < position(impr, "sortie.vide();") < 10**9 \
            or "sous_jeton(&self.donnees[..self.len]);" not in corps(u, "vide") \
            or "ecris_octets(" in corps(u, "vide"):
        fautes.append(f"{UART} : `_print` ne formate pas hors du jeton puis n'emet pas par sous_jeton")
    if re.search(r"interrupts::|EMISSION|ecris_octets\(", impr) or impr.count("vide()") != 1:
        fautes.append(f"{UART} : `_print` emet ou masque hors de sous_jeton")
    lignes = corps(u, "ecris_lignes")
    if "sous_jeton(ligne);" not in lignes or "longueur_ligne(" not in lignes or "ecris_octets(" in lignes:
        fautes.append(f"{UART} : `ecris_lignes` n'emet plus chaque ligne sous le jeton")
    console = corps(src[CONSOLE], "console_write")
    if "serial::ecris_lignes(data)" not in console or re.search(r"\becris_octets\(", console):
        fautes.append(f"{CONSOLE} : `console_write` contourne le jeton serie (ecris_octets direct)")
    cli = position(p, "interrupts::disable();")
    cas = position(p, "compare_exchange_weak(0, moi,")
    vide = position(p, "ecris_octets(octets);")
    rend = position(p, "EMISSION.store(0, Ordering::Release);")
    # Le demasquage FINAL (apres l'emission) ; un seul autre est permis, celui
    # du retour dans la file, avant l'emission, suivi de `continue`.
    sti = p.rfind("interrupts::enable();")
    sti = sti if sti >= 0 else 10**9
    if p.count("interrupts::enable();") != 2 or not re.search(
            r"None if requeue => \{\s*relances \+= 1;\s*x86_64::instructions::interrupts::enable\(\);\s*continue;", p):
        fautes.append(f"{UART} : une course perdue ne rouvre plus et ne refait plus la queue (ou demasquage en trop)")
    if not re.search(r"let mut requeue = ouvertes && relances < 100_000 && maintenant < echeance;", p) \
            or "let borne = if requeue { COURSE_MASQUEE } else { 100_000 };" not in p \
            or "attend_jeton_libre(echeance);" not in p:
        fautes.append(f"{UART} : l'attente n'est plus une seule echeance datee (file + courses)")
    if not re.search(r"None => \{\s*let cause = match.{0,500}?cause\.fetch_add\(1, Ordering::Relaxed\);\s*break None;", p, re.S) \
            or not re.search(r"Repli::EmetSansJeton => \{\s*if !octets\.is_empty\(\) \{\s*EMISSIONS_A_LA_BORNE\.fetch_add\(1, Ordering::Relaxed\);", p):
        fautes.append(f"{UART} : un renoncement ou une emission sans jeton n'est plus compte (avec sa cause)")
    # 14. Jamais sans le jeton hors panique.
    if "Some(false) | None => match repli(EN_PANIQUE.load(Ordering::Acquire)) {" not in p \
            or "Repli::Differe => differe(octets)," not in p \
            or p.count("ecris_octets(octets);") != 2 \
            or position(p, "Repli::EmetSansJeton") > position(p, "ecris_octets(octets);", 2):
        fautes.append(f"{UART} : hors panique, une ligne sans jeton n'est plus differee (emise, ou reentrance emise)")
    if not re.search(r"pub fn repli\(panique: bool\) -> Repli \{\s*if panique \{\s*Repli::EmetSansJeton\s*\} else \{\s*Repli::Differe\s*\}", src[ATTENTE]):
        fautes.append(f"{ATTENTE} : `repli(false)` ne rend plus `Differe`")
    if not re.search(r"Some\(true\) => \{.{0,900}?vide_differees\(\);\s*ecris_octets\(octets\);", p, re.S):
        fautes.append(f"{UART} : le detenteur n'emet plus les lignes differees AVANT la sienne (ordre d'un ecrivain)")
    if u.count("EN_PANIQUE.store(") != 1 or "EN_PANIQUE.store(true, Ordering::Release);" not in corps(u, "passe_en_panique"):
        fautes.append(f"{UART} : la bascule en panique est ecrite hors de `passe_en_panique`")
    panic = corps(src[PANIQUE], "panic")
    if not (position(panic, "idt::prends_la_panique(cpu)") < position(panic, "crate::drivers::serial::passe_en_panique();")
            < position(panic, "serial_println!(") < 10**9):
        fautes.append(f"{PANIQUE} : la panique ne bascule plus le journal (disponibilite) avant d'ecrire")
    if not (position(src[AUTORUN], "crate::drivers::serial::vidange_differees();")
            < position(src[AUTORUN], '"BOUCHAUD_SERIE_BILAN') < 10**9) or "lignes_perdues={}" not in src[AUTORUN]:
        fautes.append(f"{AUTORUN} : le bilan serie ne vide plus les lignes differees avant, ou ne compte plus les perdues")
    if u.count("differe(octets)") != 1 or "fn differe(octets: &[u8])" not in u:
        fautes.append(f"{UART} : une ligne est differee hors de la course masquee de sous_jeton")
    # 13. Echeance sans progres, renouvelee seulement par a_l_echeance.
    if not re.search(r"match a_l_echeance\(maintenant, debut_attente, emissions_vues, EMISSIONS_FAITES\.load\(Ordering::Relaxed\)\) \{"
                     r"\s*Echeance::Renouvelee \{ echeance_ns, emissions_vues: vues \} => \{\s*echeance = echeance_ns;", p) \
            or len(re.findall(r"\n\s*echeance = ", p)) != 1:
        fautes.append(f"{UART} : l'echeance est renouvelee hors de attente::a_l_echeance (progres non verifie)")
    progres = u.count("EMISSIONS_FAITES.fetch_add(")
    if progres < 1 or len(re.findall(r"EMISSIONS_FAITES\.fetch_add\(1, Ordering::Relaxed\);\s*EMISSION\.store\(0, Ordering::Release\);", u)) != progres \
            or not re.search(r"Some\(true\) => \{.{0,900}?EMISSIONS_FAITES\.fetch_add\(1, Ordering::Relaxed\);\s*EMISSION\.store\(0", p, re.S):
        fautes.append(f"{UART} : le progres est compte hors du jeton (une emission sans jeton renouvellerait l'attente)")
    a = src[ATTENTE]
    m = re.search(r"pub const PLAFOND_ABSOLU_NS: u64 = ([0-9_]+);", a)
    if not m or int(m.group(1).replace("_", "")) > 60_000_000_000 \
            or "return Echeance::Renonce(Renonce::Plafond);" not in a \
            or "if emissions == emissions_vues {\n        return Echeance::Renonce(Renonce::SansProgres);" not in a:
        fautes.append(f"{ATTENTE} : plafond absolu absent ou infini, ou absence de progres non renoncee")
    if "self.vide();" not in corps(u, "write_str") or "ecris_octets(" in corps(u, "write_str"):
        fautes.append(f"{UART} : le debordement du tampon de formatage se vide hors du jeton")
    if not cli < cas:
        fautes.append(f"{UART} : le jeton se prend interruptions ouvertes (detenteur preemptable)")
    if not (cas < vide < rend < sti < 10**9):
        fautes.append(f"{UART} : ordre masquage / jeton / emission / remise / demasquage rompu")
    if "if ouvertes {" not in p[rend:] or "let ouvertes = x86_64::instructions::interrupts::are_enabled();" not in p:
        fautes.append(f"{UART} : les interruptions sont rouvertes sans avoir ete ouvertes a l'entree")
    if "let moi = crate::arch::x86_64::smp::cpu_index() + 1;" not in p:
        fautes.append(f"{UART} : le detenteur n'est plus `cpu_index() + 1` (zero doit rester « libre »)")
    if p.count("EMISSION.store") != 1 or not re.search(r"Some\(true\) => \{.{0,900}?EMISSIONS_FAITES\.fetch_add\(1, Ordering::Relaxed\);\s*EMISSION\.store\(0", p, re.S):
        fautes.append(f"{UART} : le jeton est rendu sans avoir ete pris, ou plusieurs fois")
    if not re.search(r"Err\(detenteur\) if detenteur == moi => \{\s*EMISSIONS_REENTREES\.fetch_add\(1, Ordering::Relaxed\);\s*break Some\(false\);", p):
        fautes.append(f"{UART} : la reentrance n'est plus comptee sans attendre")
    if not re.search(r"if tours > borne \{\s*break None;", p):
        fautes.append(f"{UART} : l'attente du jeton n'est plus bornee")
    if "sert_shootdowns_en_attente()" not in p[cas:vide]:
        fautes.append(f"{UART} : l'attente masquee ne sert plus les shootdowns TLB")
    for interdit in ("EMISSION_PRIORITAIRE", "fn tiens_emission", "JetonEmission"):
        if interdit in u:
            fautes.append(f"{UART} : {interdit} est revenu")
    if "serie_reentrees={}" not in src[SONDE] or "emissions_reentrees()" not in src[SONDE] \
            or "serie_bornes={}" not in src[SONDE]:
        fautes.append(f"{SONDE} : serie_reentrees / serie_bornes ne sont plus publies")
    return fautes


def mutation(fichier: str, a: str, b: str) -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        for f in FICHIERS:
            d = Path(tmp) / f
            d.parent.mkdir(parents=True, exist_ok=True)
            t = (RACINE / f).read_text(encoding="utf-8")
            if f == fichier:
                if a not in t:
                    return False
                t = t.replace(a, b, 1)
            d.write_text(t, encoding="utf-8")
        return bool(verifie(Path(tmp)))


def main() -> int:
    fautes = verifie(RACINE)
    if fautes:
        print("jeton serie : regle violee")
        print("\n".join("  " + f for f in fautes))
        return 1
    negatifs = [
        # jeton pris interruptions ouvertes
        (UART, "        x86_64::instructions::interrupts::disable();\n        // Relu", "        // Relu"),
        # retour a « meme coeur = passe sans attendre et emet »
        (UART, "                    EMISSIONS_REENTREES.fetch_add(1, Ordering::Relaxed);\n                    break Some(false);",
         "                    EMISSIONS_REENTREES.fetch_add(1, Ordering::Relaxed);\n                    EMISSION.store(0, Ordering::Release);\n                    break Some(false);"),
        # demasquage avant la remise du jeton
        (UART, "            let debut_emission = crate::kernel::timer::monotonic_ns();\n            vide_differees();",
         "            x86_64::instructions::interrupts::enable();\n            let debut_emission = crate::kernel::timer::monotonic_ns();\n            vide_differees();"),
        (UART, "            if tours > borne {", "            if false {"),
        (UART, "static EMISSIONS_REENTREES: AtomicU64", "static EMISSION_PRIORITAIRE: AtomicBool = AtomicBool::new(false);\nstatic EMISSIONS_REENTREES: AtomicU64"),
        (UART, "            if tours % 64 == 0 {\n                crate::arch::x86_64::smp::sert_shootdowns_en_attente();\n            }\n", ""),
        # la sortie userland retourne a des lots hors jeton
        (CONSOLE, "crate::drivers::serial::ecris_lignes(data);", "crate::drivers::serial::ecris_octets(data);"),
        (UART, "        sous_jeton(ligne);", "        ecris_octets(ligne);"),
        # emission de _print hors du jeton
        (UART, "        sous_jeton(&self.donnees[..self.len]);", "        ecris_octets(&self.donnees[..self.len]);"),
        # retour a « course perdue -> attente masquee jusqu'a la borne »
        (UART, "        let borne = if requeue { COURSE_MASQUEE } else { 100_000 };", "        let borne = 100_000;"),
        (UART, "            None if requeue => {\n                relances += 1;\n                x86_64::instructions::interrupts::enable();\n                continue;\n            }\n", ""),
        # debordement du tampon de formatage hors jeton
        (UART, "                self.vide();", "                ecris_octets(&self.donnees[..self.len]);\n                self.len = 0;"),
        # le progres compte aussi les renoncements (sans jeton)
        (UART, "                cause.fetch_add(1, Ordering::Relaxed);\n                break None;",
         "                cause.fetch_add(1, Ordering::Relaxed);\n                EMISSIONS_FAITES.fetch_add(1, Ordering::Relaxed);\n                break None;"),
        # l'echeance renouvelee sans verifier de progres (retour a « attendre indefiniment »)
        (UART, "            match a_l_echeance(maintenant, debut_attente, emissions_vues, EMISSIONS_FAITES.load(Ordering::Relaxed)) {",
         "            match a_l_echeance(maintenant, maintenant, emissions_vues, emissions_vues.wrapping_add(1)) {"),
        # plafond absolu infini
        (ATTENTE, "pub const PLAFOND_ABSOLU_NS: u64 = 10_000_000_000;", "pub const PLAFOND_ABSOLU_NS: u64 = 18_446_744_073_709_551_615;"),
        # 14 : la ligne differee est emise quand meme (retour a V1)
        (UART, "            Repli::Differe => differe(octets),", "            Repli::Differe => ecris_octets(octets),"),
        # 14 : le repli normal devient l'emission sans jeton
        (ATTENTE, "    } else {\n        Repli::Differe\n    }", "    } else {\n        Repli::EmetSansJeton\n    }"),
        # 14 : la reentrance emet au milieu de la ligne du detenteur
        (UART, "        Some(false) | None => match repli(", "        Some(false) => ecris_octets(octets),\n        None => match repli("),
        # 14 : les lignes differees apres la sienne (ordre d'un ecrivain casse)
        (UART, "            vide_differees();\n            ecris_octets(octets);\n            let duree",
         "            ecris_octets(octets);\n            vide_differees();\n            let duree"),
        # 14 : la panique ne bascule plus le journal
        (PANIQUE, "    crate::drivers::serial::passe_en_panique();\n", ""),
        # 14 : le bilan ne vide plus les lignes differees
        (AUTORUN, "    crate::drivers::serial::vidange_differees();\n", ""),
    ]
    for n, (f, a, b) in enumerate(negatifs, 1):
        if not mutation(f, a, b):
            print(f"jeton serie : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"JETON_SERIE_OK negatifs={len(negatifs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
