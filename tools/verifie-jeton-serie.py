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
     jeton.

Fail-closed ; douze tests negatifs.
"""
import re
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
UART = "src/drivers/serial/uart16550.rs"
SONDE = "src/kernel/process/thread/diagnostic_stall.rs"
CONSOLE = "src/compat/linux/file.rs"
FICHIERS = (UART, SONDE, CONSOLE)


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


def position(texte: str, motif: str) -> int:
    i = texte.find(motif)
    return i if i >= 0 else 10**9


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
    if not fmt < position(impr, "sous_jeton(|| sortie.vide());") < 10**9:
        fautes.append(f"{UART} : `_print` ne formate pas hors du jeton puis n'emet pas par sous_jeton")
    if re.search(r"interrupts::|EMISSION|ecris_octets\(", impr) or impr.count("vide()") != 1:
        fautes.append(f"{UART} : `_print` emet ou masque hors de sous_jeton")
    lignes = corps(u, "ecris_lignes")
    if "sous_jeton(|| ecris_octets(ligne));" not in lignes or "longueur_ligne(" not in lignes:
        fautes.append(f"{UART} : `ecris_lignes` n'emet plus chaque ligne sous le jeton")
    console = corps(src[CONSOLE], "console_write")
    if "serial::ecris_lignes(data)" not in console or re.search(r"\becris_octets\(", console):
        fautes.append(f"{CONSOLE} : `console_write` contourne le jeton serie (ecris_octets direct)")
    cli = position(p, "interrupts::disable();")
    cas = position(p, "compare_exchange_weak(0, moi,")
    vide = position(p, "emet();")
    rend = position(p, "EMISSION.store(0, Ordering::Release);")
    # Le demasquage FINAL (apres l'emission) ; un seul autre est permis, celui
    # du retour dans la file, avant l'emission, suivi de `continue`.
    sti = p.rfind("interrupts::enable();")
    sti = sti if sti >= 0 else 10**9
    if p.count("interrupts::enable();") != 2 or not re.search(
            r"None if requeue => \{\s*relances \+= 1;\s*x86_64::instructions::interrupts::enable\(\);\s*continue;", p):
        fautes.append(f"{UART} : une course perdue ne rouvre plus et ne refait plus la queue (ou demasquage en trop)")
    if not re.search(r"let requeue = ouvertes\s*&& relances < 100_000\s*&& crate::kernel::timer::monotonic_ns\(\) < echeance;", p) \
            or "let borne = if requeue { COURSE_MASQUEE } else { 100_000 };" not in p \
            or "attend_jeton_libre(echeance);" not in p:
        fautes.append(f"{UART} : l'attente n'est plus une seule echeance datee (file + courses)")
    if not re.search(r"None => \{\s*EMISSIONS_A_LA_BORNE\.fetch_add\(1, Ordering::Relaxed\);\s*break false;", p):
        fautes.append(f"{UART} : une emission sans jeton n'est plus comptee")
    if "sous_jeton(|| self.vide());" not in corps(u, "write_str"):
        fautes.append(f"{UART} : le debordement du tampon de formatage se vide hors du jeton")
    if not cli < cas:
        fautes.append(f"{UART} : le jeton se prend interruptions ouvertes (detenteur preemptable)")
    if not (cas < vide < rend < sti < 10**9):
        fautes.append(f"{UART} : ordre masquage / jeton / emission / remise / demasquage rompu")
    if "if ouvertes {" not in p[rend:] or "let ouvertes = x86_64::instructions::interrupts::are_enabled();" not in p:
        fautes.append(f"{UART} : les interruptions sont rouvertes sans avoir ete ouvertes a l'entree")
    if "let moi = crate::arch::x86_64::smp::cpu_index() + 1;" not in p:
        fautes.append(f"{UART} : le detenteur n'est plus `cpu_index() + 1` (zero doit rester « libre »)")
    if p.count("EMISSION.store") != 1 or not re.search(r"if pris \{\s*EMISSION\.store\(0", p):
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
        (UART, "    let debut_emission = crate::kernel::timer::monotonic_ns();\n    emet();",
         "    if ouvertes {\n        x86_64::instructions::interrupts::enable();\n    }\n    let debut_emission = crate::kernel::timer::monotonic_ns();\n    emet();"),
        (UART, "            if tours > borne {", "            if false {"),
        (UART, "static EMISSIONS_REENTREES: AtomicU64", "static EMISSION_PRIORITAIRE: AtomicBool = AtomicBool::new(false);\nstatic EMISSIONS_REENTREES: AtomicU64"),
        (UART, "            if tours % 64 == 0 {\n                crate::arch::x86_64::smp::sert_shootdowns_en_attente();\n            }\n", ""),
        # la sortie userland retourne a des lots hors jeton
        (CONSOLE, "crate::drivers::serial::ecris_lignes(data);", "crate::drivers::serial::ecris_octets(data);"),
        (UART, "        sous_jeton(|| ecris_octets(ligne));", "        ecris_octets(ligne);"),
        # emission de _print hors du jeton
        (UART, "    sous_jeton(|| sortie.vide());", "    sortie.vide();\n    sous_jeton(|| ());"),
        # retour a « course perdue -> attente masquee jusqu'a la borne, puis sans jeton »
        (UART, "        let borne = if requeue { COURSE_MASQUEE } else { 100_000 };", "        let borne = 100_000;"),
        (UART, "            None if requeue => {\n                relances += 1;\n                x86_64::instructions::interrupts::enable();\n                continue;\n            }\n", ""),
        # debordement du tampon de formatage hors jeton
        (UART, "                sous_jeton(|| self.vide());", "                self.vide();"),
    ]
    for n, (f, a, b) in enumerate(negatifs, 1):
        if not mutation(f, a, b):
            print(f"jeton serie : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"JETON_SERIE_OK negatifs={len(negatifs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
