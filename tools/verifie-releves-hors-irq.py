#!/usr/bin/env python3
"""Le hard IRQ capture ; une tache formate et imprime.

BOUCHAUD_RELEVES_HORS_IRQ_V1

Le releve d'ordonnancement s'imprimait depuis l'IRQ du minuteur du coeur
zero, interruptions masquees : une vingtaine de lignes, 118-127 ms au pire
apres BOUCHAUD_JETON_SERIE_PROPRIETAIRE_V1, et le texte insere au milieu de
la ligne qu'il interrompait. La veille d'attente vive imprimait de meme
jusqu'a trente-deux lignes depuis les tics.

Le garde exige :
  1. `stall_probe_from_timer` (IRQ) ne forme qu'UNE ligne, la ligne de
     souffrance, sous la condition `DIAG_SOUFFRANCE_NS` -- et n'appelle aucun
     des releves (`signale_etat_ordonnancement`, `resume_ordonnancement`,
     `signale_taches_orphelines`) ;
  2. `veille_attentes_vives` et `sonde_gel_tic` (IRQ) n'impriment rien ;
  3. `sert_diagnostic` porte les releves, `fil_diagnostic` l'appelle, et
     `diag-noyau` est lance au demarrage (`main.rs`).

Fail-closed ; cinq tests negatifs.
"""
import re
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
SONDE = "src/kernel/process/thread/diagnostic_stall.rs"
VEILLE = "src/kernel/process/thread/veille_attente.rs"
GEL = "src/kernel/process/thread/sonde_gel.rs"
MAIN = "src/main.rs"
FICHIERS = (SONDE, VEILLE, GEL, MAIN)
IMPRESSION = re.compile(r"\b(serial_println!|serial_print!|println!|print!|dmesg::log)")


def sans_commentaires(t: str) -> str:
    return "\n".join(l.split("//", 1)[0] for l in t.splitlines())


def corps(texte: str, nom: str) -> str:
    m = re.search(r"\bfn\s+" + re.escape(nom) + r"\s*\(", texte)
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


def verifie(racine: Path) -> list[str]:
    try:
        src = {f: sans_commentaires((racine / f).read_text(encoding="utf-8")) for f in FICHIERS}
    except OSError as e:
        return [f"lecture impossible : {e}"]
    fautes = []
    irq = corps(src[SONDE], "stall_probe_from_timer")
    if not irq:
        fautes.append(f"{SONDE} : stall_probe_from_timer introuvable")
    impressions = IMPRESSION.findall(irq)
    if len(impressions) != 1 or '"[DIAG-EN-SOUFFRANCE]' not in irq:
        fautes.append(f"{SONDE} : l'IRQ du minuteur forme {len(impressions)} ligne(s) ; seule la ligne de souffrance est permise")
    else:
        avant = irq[:irq.index('"[DIAG-EN-SOUFFRANCE]')]
        garde = avant[avant.rfind("if precedente"):]
        if "depuis) >= DIAG_SOUFFRANCE_NS" not in garde or garde.count(">= DIAG_SOUFFRANCE_NS") != 2:
            fautes.append(f"{SONDE} : la ligne de souffrance n'est plus bornee par DIAG_SOUFFRANCE_NS")
    for releve in ("signale_etat_ordonnancement(", "resume_ordonnancement(", "signale_taches_orphelines("):
        if releve in irq:
            fautes.append(f"{SONDE} : {releve.rstrip('(')} appele depuis l'IRQ du minuteur")
        if releve not in corps(src[SONDE], "sert_diagnostic"):
            fautes.append(f"{SONDE} : sert_diagnostic ne porte plus {releve.rstrip('(')}")
    if "sert_diagnostic()" not in corps(src[SONDE], "fil_diagnostic"):
        fautes.append(f"{SONDE} : fil_diagnostic ne sert plus les demandes")
    if '"diag-noyau"' not in corps(src[SONDE], "demarre_fil_diagnostic"):
        fautes.append(f"{SONDE} : diag-noyau n'est plus lance")
    if "kernel::task::demarre_fil_diagnostic();" not in src[MAIN]:
        fautes.append(f"{MAIN} : diag-noyau n'est pas lance au demarrage")
    for f, nom in ((VEILLE, "veille_attentes_vives"), (GEL, "sonde_gel_tic")):
        c = corps(src[f], nom)
        if not c:
            fautes.append(f"{f} : {nom} introuvable")
        elif IMPRESSION.search(c):
            fautes.append(f"{f} : {nom} imprime depuis l'interruption")
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
        print("releves hors IRQ : regle violee")
        print("\n".join("  " + f for f in fautes))
        return 1
    negatifs = [
        (SONDE, "    signale_taches_orphelines();\n    if complet {", "    signale_taches_orphelines();\n    if complet {"),
        (SONDE, "    // Capture : vingt-quatre lectures atomiques, rien d'autre.\n",
         "    // Capture : vingt-quatre lectures atomiques, rien d'autre.\n    signale_etat_ordonnancement();\n"),
        (SONDE, "        && debut_ns.saturating_sub(depuis) >= DIAG_SOUFFRANCE_NS\n", "\n"),
        (VEILLE, "        VEILLE_RAPPORT_PRET[n].store(1, Ordering::Release);\n",
         "        VEILLE_RAPPORT_PRET[n].store(1, Ordering::Release);\n        crate::serial_println!(\"x\");\n"),
        (MAIN, "    kernel::task::demarre_fil_diagnostic();\n", ""),
        (GEL, "    GEL_VUS.fetch_add(1, Ordering::Relaxed);\n",
         "    GEL_VUS.fetch_add(1, Ordering::Relaxed);\n    crate::kernel::dmesg::log(\"gel\");\n"),
    ]
    # le premier negatif est un temoin de mutation nulle : il ne doit PAS rougir
    temoin, negatifs = negatifs[0], negatifs[1:]
    if mutation(*temoin):
        print("releves hors IRQ : le garde rougit sur une mutation nulle")
        return 1
    for n, (f, a, b) in enumerate(negatifs, 1):
        if not mutation(f, a, b):
            print(f"releves hors IRQ : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"RELEVES_HORS_IRQ_OK negatifs={len(negatifs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
