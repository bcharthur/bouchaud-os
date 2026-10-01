#!/usr/bin/env python3
"""Le gros verrou noyau a ete supprime ; il ne revient pas.

BOUCHAUD_BKL_SUPPRIME_V1

# Ce qui a ete retire

`smp_lock` (src/kernel/sync/bkl.rs et ses fragments), sa comptabilite
(`bkl_compte`), le modele de discipline sous verrou, les portees de domaine qui
attribuaient ses prises (`sync::portee`, `Domaine`), les portees `desktop_bkl`
du bureau, et la table `SANS_BKL` de l'aiguilleur -- dont les audits vivent
desormais dans docs/AUDIT_VERROUILLAGE_SYSCALLS.md.

Le retrait s'est fait par lots mesures (c1-c7, B1-B11) : 57 321 acquisitions
par cycle d'endurance avant le lot B2, 0 apres B10. Chaque etat que le verrou
serialisait a recu son verrou propre ; le detail est dans
docs/MESURE_DEMARRAGE_A_FROID.md §18.9.

# La regle

  * aucun des fichiers ou dossiers retires n'existe ;
  * aucun code de src/ (commentaires exclus) ne nomme le module ni ses
    primitives : `smp_lock`, `KernelGuard`, `suspend_for_schedule`,
    `resume_after_schedule`, `profondeur_locale`, `held_by_current_cpu`,
    `desktop_bkl`, `Domaine::`, `sync::portee(`, `SANS_BKL`, `exige_bkl` ;
  * le marqueur BOUCHAUD_BKL_SUPPRIME_V1 reste dans src/kernel/sync/mod.rs,
    la ou un lecteur chercherait le verrou.

Un nouveau besoin de serialisation se resout par un verrou de sous-systeme ou
d'objet (SpinLock, SpinLockIrq, SleepMutex, RankedSpinLock), jamais par un
verrou global. Fail-closed ; quatre tests negatifs.
"""

import re
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent

RETIRES = (
    "src/kernel/sync/bkl.rs",
    "src/kernel/sync/bkl",
    "src/kernel/sync/bkl_compte.rs",
    "src/kernel/sync/domaine.rs",
    "src/kernel/sync/discipline.rs",
    "src/gui/desktop_bkl.rs",
    "src/gui/desktop_bkl",
    "src/compat/linux/bkl.rs",
)

INTERDITS = re.compile(
    r"\bsmp_lock\b|\bKernelGuard\b|\bsuspend_for_schedule\b|\bresume_after_schedule\b"
    r"|\bprofondeur_locale\b|\bheld_by_current_cpu\b|\bdesktop_bkl\b|\bDomaine::"
    r"|\bsync::portee\(|\bSANS_BKL\b|\bexige_bkl\b"
)

MARQUEUR = "BOUCHAUD_BKL_SUPPRIME_V1"


def code_seul(ligne: str) -> str:
    return ligne.split("//", 1)[0]


def verifie(racine: Path) -> list[str]:
    fautes = []
    for relatif in RETIRES:
        if (racine / relatif).exists():
            fautes.append(f"{relatif} existe de nouveau")
    src = racine / "src"
    if not src.is_dir():
        return fautes + ["src/ introuvable"]
    for chemin in sorted(src.rglob("*.rs")):
        relatif = chemin.relative_to(racine).as_posix()
        for numero, ligne in enumerate(
            chemin.read_text(encoding="utf-8", errors="replace").splitlines(), 1
        ):
            trouve = INTERDITS.search(code_seul(ligne))
            if trouve:
                fautes.append(f"{relatif}:{numero} : `{trouve.group(0)}` -- {ligne.strip()}")
    sync = racine / "src/kernel/sync/mod.rs"
    if not sync.exists() or MARQUEUR not in sync.read_text(encoding="utf-8"):
        fautes.append(f"src/kernel/sync/mod.rs : marqueur {MARQUEUR} absent")
    return fautes


def negatif(fichiers: dict[str, str]) -> bool:
    """Un arbre minimal sain, plus `fichiers` : la regle doit le refuser."""
    with tempfile.TemporaryDirectory() as tmp:
        racine = Path(tmp)
        sync = racine / "src/kernel/sync/mod.rs"
        sync.parent.mkdir(parents=True)
        sync.write_text(f"//! {MARQUEUR}\n", encoding="utf-8")
        if verifie(racine):
            return False  # l'arbre sain doit passer
        for relatif, contenu in fichiers.items():
            cible = racine / relatif
            cible.parent.mkdir(parents=True, exist_ok=True)
            cible.write_text(contenu, encoding="utf-8")
        return bool(verifie(racine))


def main() -> int:
    fautes = verifie(RACINE)
    if fautes:
        print("gros verrou : il revient")
        print("\n".join("  " + f for f in fautes[:40]))
        return 1
    negatifs = [
        {"src/kernel/sync/bkl.rs": "pub fn enter() {}\n"},
        {"src/a.rs": "fn f() { let _k = crate::kernel::smp_lock::enter(); }\n"},
        {"src/b.rs": "fn f() { let _d = crate::kernel::sync::portee(Domaine::Fd); }\n"},
        {"src/kernel/sync/mod.rs": "// marqueur retire\n"},
    ]
    for n, fichiers in enumerate(negatifs, 1):
        if not negatif(fichiers):
            print(f"gros verrou : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"BKL_SUPPRIME_OK negatifs={len(negatifs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
