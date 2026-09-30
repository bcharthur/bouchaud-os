#!/usr/bin/env python3
"""Les pilotes liberes du gros verrou ne gardent pas d'etat en `static mut`.

BOUCHAUD_AC97_VERROU_V1, BOUCHAUD_CONSOLE_VERROU_V1,
BOUCHAUD_GFX_DRAPEAUX_ATOMIQUES_V1

# Le defaut

`ioctl` et `write` sur /dev/dsp et la console prenaient le gros verrou : c'est
tout ce qui serialisait l'etat du pilote AC97 (treize `static mut`), la pile
de captures et l'ecrivain VGA (deux `static mut`, que les fils noyau
ecrivaient deja sans lui), et les drapeaux de mode du framebuffer lus par
`ioctl`. Le lot B9 libere `ioctl` et ces branches de `write`. Un `static mut`
qui reviendrait dans l'un de ces fichiers serait une course de donnees sans
que rien ne le signale.

# La regle

  * `ac97.rs` et `vga_text.rs` : aucun `static mut` ;
  * `ac97.rs` garde son etat sous `SleepMutex`, `vga_text.rs` sous
    `SpinLockIrq` avec une prise bornee (`try_lock`) ;
  * `bochs.rs` : `HD_ACTIVE`, `USERLAND_OWNS_DISPLAY` et `LFB_PHYS` sont
    atomiques ;
  * `file.rs` : aucune branche console ou audio ne prend le gros verrou.

Fail-closed : un fichier illisible est une faute. Quatre tests negatifs.
"""

import re
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
AC97 = "src/drivers/audio/ac97.rs"
VGA = "src/drivers/display/vga_text.rs"
BOCHS = "src/drivers/display/bochs.rs"
FICHIERS = (AC97, VGA, BOCHS, "src/compat/linux/file.rs")


def sans_commentaires(texte: str) -> str:
    return "\n".join(re.sub(r"//.*", "", ligne) for ligne in texte.splitlines())


def verifie(racine: Path) -> list[str]:
    fautes = []
    try:
        textes = {f: sans_commentaires((racine / f).read_text(encoding="utf-8")) for f in FICHIERS}
    except OSError as e:
        return [f"lecture impossible : {e}"]
    for f in (AC97, VGA):
        for m in re.finditer(r"\bstatic\s+mut\s+(\w+)", textes[f]):
            fautes.append(f"{f} : `static mut {m.group(1)}` (etat hors verrou)")
    if not re.search(r"static\s+ETAT\s*:\s*SleepMutex<", textes[AC97]):
        fautes.append(f"{AC97} : l'etat n'est plus sous `SleepMutex`")
    if not re.search(r"static\s+CONSOLE\s*:\s*SpinLockIrq<", textes[VGA]):
        fautes.append(f"{VGA} : la console n'est plus sous `SpinLockIrq`")
    if "CONSOLE.try_lock()" not in textes[VGA]:
        fautes.append(f"{VGA} : la prise de la console n'est plus bornee (try_lock)")
    for nom in ("HD_ACTIVE", "USERLAND_OWNS_DISPLAY", "LFB_PHYS"):
        if not re.search(rf"\bstatic\s+{nom}\s*:\s*Atomic", textes[BOCHS]):
            fautes.append(f"{BOCHS} : `{nom}` n'est plus atomique")
    fichier = textes["src/compat/linux/file.rs"]
    for branche in ("FdKind::Console =>", "FdKind::Audio =>"):
        occurrences = [m.start() for m in re.finditer(re.escape(branche), fichier)]
        if not occurrences:
            fautes.append(f"file.rs : branche `{branche}` introuvable")
        # Toutes les branches : read, write, ioctl, stat, poll...
        for debut in occurrences:
            fin = fichier.find("FdKind::", debut + len(branche))
            if "smp_lock::enter" in fichier[debut:fin]:
                fautes.append(f"file.rs : une branche `{branche}` reprend le gros verrou")
    return fautes


def mutation(fichier: str, avant: str, apres: str) -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        copie = Path(tmp)
        for f in FICHIERS:
            dest = copie / f
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_text((RACINE / f).read_text(encoding="utf-8"), encoding="utf-8")
        cible = copie / fichier
        texte = cible.read_text(encoding="utf-8")
        if avant not in texte:
            return False
        cible.write_text(texte.replace(avant, apres, 1), encoding="utf-8")
        return bool(verifie(copie))


def main() -> int:
    fautes = verifie(RACINE)
    if fautes:
        print("pilotes hors gros verrou : regle violee")
        print("\n".join("  " + f for f in fautes))
        return 1
    negatifs = [
        (AC97, "static ETAT: SleepMutex<Ac97>", "static mut EN_VOL: usize = 0;\nstatic ETAT: SleepMutex<Ac97>"),
        (VGA, "if let Some(mut console) = CONSOLE.try_lock()", "if let Some(mut console) = Some(CONSOLE.lock())"),
        (BOCHS, "static HD_ACTIVE: AtomicBool", "static mut HD_ACTIVE_BIS: bool = false;\nstatic HD_ACTIVE_X: AtomicBool"),
        ("src/compat/linux/file.rs", "            console_write(&data);", "            let _k = crate::kernel::smp_lock::enter();\n            console_write(&data);"),
    ]
    for n, (f, a, b) in enumerate(negatifs, 1):
        if not mutation(f, a, b):
            print(f"pilotes hors gros verrou : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"PILOTES_SANS_STATIC_MUT_OK negatifs={len(negatifs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
