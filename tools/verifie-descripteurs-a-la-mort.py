#!/usr/bin/env python3
"""Les descripteurs d'un processus se ferment a sa mort.

BOUCHAUD_DESCRIPTEURS_A_LA_MORT_V1

scheduler-ng-banc (ligne de base, tous les SMP) : `eof-avant-recolte` 0/10 et
`racine-avant-descendants` 0/5. Un tube dont l'ecrivain etait mort ne rendait
jamais la fin de fichier -- ni avant ni apres `wait4` : la table de
descripteurs vivait avec le processus, que l'emplacement de la tache zombie
tenait jusqu'a son recyclage.

Le garde exige :
  1. `exit_current` ferme les descripteurs du dernier fil AVANT de prevenir
     le parent (comme `exit_files` precede `exit_notify` sous Linux) ;
  2. `tue_processus` aussi, avant `notify_parent_of_exit_for` ;
  3. la fermeture vide la table sous son verrou et ferme HORS du verrou
     (`prend_tout`, puis `drop`).

Fail-closed ; trois tests negatifs.
"""
import re
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
VIE = "src/kernel/process/thread/lifecycle.rs"
FD = "src/kernel/object/fd.rs"
FICHIERS = (VIE, FD)


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


def avant(t: str, a: str, b: str) -> bool:
    i, j = t.find(a), t.find(b)
    return 0 <= i < j


def verifie(racine: Path) -> list[str]:
    try:
        src = {f: sans_commentaires((racine / f).read_text(encoding="utf-8")) for f in FICHIERS}
    except OSError as e:
        return [f"lecture impossible : {e}"]
    fautes = []
    ec = corps(src[VIE], "exit_current")
    if not avant(ec, "ferme_descripteurs(&current().process);", "notify_parent_of_exit();"):
        fautes.append(f"{VIE} : exit_current previent le parent avant de fermer les descripteurs")
    tp = corps(src[VIE], "tue_processus")
    if not avant(tp, "ferme_descripteurs(&process);", "notify_parent_of_exit_for(process.parent);"):
        fautes.append(f"{VIE} : tue_processus ne ferme pas les descripteurs avant de prevenir")
    fd = corps(src[VIE], "ferme_descripteurs")
    if "process.files.lock().prend_tout();" not in fd or "drop(fermes);" not in fd:
        fautes.append(f"{VIE} : ferme_descripteurs ne ferme plus hors du verrou de la table")
    if "core::mem::take(&mut self.entries)" not in corps(src[FD], "prend_tout"):
        fautes.append(f"{FD} : prend_tout ne vide plus la table")
    return fautes


def mutation(fichier: str, a: str, b: str) -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        copie = Path(tmp)
        for f in FICHIERS:
            d = copie / f
            d.parent.mkdir(parents=True, exist_ok=True)
            d.write_text((RACINE / f).read_text(encoding="utf-8"), encoding="utf-8")
        c = copie / fichier
        t = c.read_text(encoding="utf-8")
        if a not in t:
            return False
        c.write_text(t.replace(a, b, 1), encoding="utf-8")
        return bool(verifie(copie))


def main() -> int:
    fautes = verifie(RACINE)
    if fautes:
        print("descripteurs a la mort : regle violee")
        print("\n".join("  " + f for f in fautes))
        return 1
    negatifs = [
        (VIE, "        ferme_descripteurs(&current().process);\n", ""),
        (VIE, "            ferme_descripteurs(&process);\n", ""),
        (VIE, "    let fermes = process.files.lock().prend_tout();\n    drop(fermes);",
         "    let mut t = process.files.lock();\n    let fermes = t.prend_tout();\n    drop(fermes);"),
    ]
    for n, (f, a, b) in enumerate(negatifs, 1):
        if not mutation(f, a, b):
            print(f"descripteurs a la mort : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"DESCRIPTEURS_A_LA_MORT_OK negatifs={len(negatifs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
