#!/usr/bin/env python3
"""Le jeton d'emission serie connait son coeur ; une IRQ n'attend pas son detenteur.

BOUCHAUD_JETON_SERIE_PROPRIETAIRE_V1

`_print` serialise chaque ligne par un jeton (CAS), avec une attente bornee a
100 000 tours. Une interruption qui coupe, SUR LE MEME COEUR, une tache
detentrice du jeton ne peut pas le voir se liberer avant de rendre la main :
elle attendait donc la borne, ligne apres ligne. Le releve d'ordonnancement de
l'IRQ du minuteur (dix-huit lignes, interruptions masquees) tenait ainsi le
coeur zero 463 a 566 ms (scheduler-ng-banc SMP1, QEMU). Et, disputant chaque
ligne par un CAS non equitable aux taches des autres coeurs, il allait a la
borne ligne apres ligne : 5 a 12,6 s a SMP4/SMP8.

Le garde exige :
  1. un jeton qui porte son detenteur (`cpu_index() + 1`, zero = libre) ;
  2. une branche `detenteur == moi` qui emet SANS attendre et SANS rendre un
     jeton qu'elle ne tient pas ;
  3. un ecrivain aux interruptions masquees prioritaire, priorite rendue ;
  4. la borne des attentes conservee ;
  5. le releve de l'IRQ du minuteur qui prend le jeton UNE fois pour toutes
     ses lignes (`tiens_emission`) ;
  6. le compteur publie (`[SONDE-IRQ-DUREE] ... serie_bornes=`).

Fail-closed ; six tests negatifs.
"""
import re
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
UART = "src/drivers/serial/uart16550.rs"
SONDE = "src/kernel/process/thread/diagnostic_stall.rs"
FICHIERS = (UART, SONDE)


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


def bloc(texte: str, ouverture: str) -> str:
    i = texte.find(ouverture)
    if i < 0:
        return ""
    o = texte.find("{", i)
    p = 0
    for j in range(o, len(texte)):
        p += texte[j] == "{"
        p -= texte[j] == "}"
        if p == 0:
            return texte[o:j + 1]
    return ""


def verifie(racine: Path) -> list[str]:
    try:
        src = {f: sans_commentaires((racine / f).read_text(encoding="utf-8")) for f in FICHIERS}
    except OSError as e:
        return [f"lecture impossible : {e}"]
    fautes = []
    u = src[UART]
    if "static EMISSION: AtomicUsize = AtomicUsize::new(0);" not in u:
        fautes.append(f"{UART} : le jeton d'emission ne porte plus son detenteur")
    a = corps(u, "acquiert_emission")
    if "let moi = crate::arch::x86_64::smp::cpu_index() + 1;" not in a:
        fautes.append(f"{UART} : le detenteur n'est plus `cpu_index() + 1` (zero doit rester « libre »)")
    if "compare_exchange_weak(0, moi," not in a:
        fautes.append(f"{UART} : le jeton ne se prend plus par CAS 0 -> moi")
    if not re.search(r"if detenteur == moi \{\s*break Acquisition::DejaAuCoeur;", a):
        fautes.append(f"{UART} : un ecrivain attend encore un jeton tenu par son propre coeur")
    if not re.search(r"if tours > 100_000 \{", a):
        fautes.append(f"{UART} : l'attente du jeton n'est plus bornee")
    if not re.search(r"let cede = !masquees && EMISSION_PRIORITAIRE\.load", a) \
            or not re.search(r"if masquees \{\s*EMISSION_PRIORITAIRE\.store\(true", a):
        fautes.append(f"{UART} : l'ecrivain aux interruptions masquees ne passe plus devant")
    if not re.search(r"\};\s*if masquees \{\s*EMISSION_PRIORITAIRE\.store\(false", a):
        fautes.append(f"{UART} : la priorite n'est pas rendue a la sortie de l'attente")
    p = corps(u, "_print")
    pris = bloc(p, "Acquisition::Pris =>")
    if "EMISSION.store(0, Ordering::Release);" not in pris:
        fautes.append(f"{UART} : _print ne rend plus le jeton qu'il a pris")
    if p.count("EMISSION.store") != 1:
        fautes.append(f"{UART} : _print rend un jeton qu'il ne tient pas")
    t = corps(u, "tiens_emission")
    if "matches!(acquiert_emission(), Acquisition::Pris)" not in t:
        fautes.append(f"{UART} : tiens_emission rendrait un jeton qu'elle n'a pas pris")
    s = src[SONDE]
    if "serie_bornes={}" not in s or "emissions_a_la_borne()" not in s:
        fautes.append(f"{SONDE} : serie_bornes n'est plus publie")
    sp = corps(s, "stall_probe_from_timer")
    j, d = sp.find("let _jeton = crate::drivers::serial::tiens_emission();"), sp.find("let _duree = DureeReleve")
    if j < 0 or d < 0 or j > d or sp.find("[SMP-SNAPSHOT]") < d:
        fautes.append(f"{SONDE} : le releve ne tient plus le jeton une fois pour toutes ses lignes")
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
        (UART, "        if detenteur == moi {\n            break Acquisition::DejaAuCoeur;\n        }\n", ""),
        (UART, "Acquisition::DejaAuCoeur => {\n            EMISSIONS_IMBRIQUEES.fetch_add(1, Ordering::Relaxed);\n            sortie.vide();",
         "Acquisition::DejaAuCoeur => {\n            EMISSIONS_IMBRIQUEES.fetch_add(1, Ordering::Relaxed);\n            sortie.vide();\n            EMISSION.store(0, Ordering::Release);"),
        (UART, "crate::arch::x86_64::smp::cpu_index() + 1;", "crate::arch::x86_64::smp::cpu_index();"),
        (UART, "        if tours > 100_000 {", "        if false {"),
        (UART, "let cede = !masquees && EMISSION_PRIORITAIRE.load(Ordering::Relaxed);", "let cede = false;"),
        (SONDE, "    let _jeton = crate::drivers::serial::tiens_emission();\n", ""),
    ]
    for n, (f, a, b) in enumerate(negatifs, 1):
        if not mutation(f, a, b):
            print(f"jeton serie : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"JETON_SERIE_OK negatifs={len(negatifs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
