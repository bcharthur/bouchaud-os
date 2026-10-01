#!/usr/bin/env python3
"""`sched_getaffinity` rend le vrai masque de la tache.

BOUCHAUD_AFFINITE_VISIBLE_V1

L'appel rendait `1` en dur. musl calcule `sysconf(_SC_NPROCESSORS_ONLN)` par
lui : tout programme musl -- les pools de fils du navigateur compris -- se
croyait seul sur un coeur (scheduler-ng-banc : `coeurs_sysconf=1` a
SMP1/2/4/8/16, alors que /sys en annoncait N).

Le garde exige :
  1. SCHED_GETAFFINITY passe par `task::masque_affinite_visible`, et ne rend
     plus de constante ;
  2. le masque visible est celui de la tache, borne aux coeurs en ligne ;
  3. un tampon trop petit est refuse (EINVAL), une ecriture fautive aussi
     (EFAULT) ;
  4. le banc publie ce que voit un fils (`coeurs_sysconf_fils`) : la racine
     d'un lancement synchrone est epinglee, ses fils non.

Fail-closed ; trois tests negatifs.
"""
import re
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
ABI = "src/compat/linux/mod.rs"
ORDO = "src/kernel/process/thread/ordonnancement.rs"
BANC = "tools/userland/scheduler-ng-banc.c"
FICHIERS = (ABI, ORDO, BANC)


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
    abi = src[ABI]
    i = abi.find("SCHED_GETAFFINITY => {")
    bras = abi[i:abi.find("SCHED_SETAFFINITY", i)] if i >= 0 else ""
    if "task::masque_affinite_visible(" not in bras or "1u64.to_le_bytes()" in bras:
        fautes.append(f"{ABI} : sched_getaffinity ne rend plus le masque de la tache")
    if "-errno::EINVAL" not in bras or "-errno::EFAULT" not in bras:
        fautes.append(f"{ABI} : sched_getaffinity n'a plus ses refus (tampon court, ecriture fautive)")
    m = corps(src[ORDO], "masque_affinite_visible")
    if ".affinity_mask" not in m or "masque & en_ligne" not in m:
        fautes.append(f"{ORDO} : le masque visible n'est plus celui de la tache borne aux coeurs en ligne")
    if "coeurs_sysconf_fils" not in src[BANC]:
        fautes.append(f"{BANC} : le banc ne publie plus ce que voit un fils")
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
        print("affinite visible : regle violee")
        print("\n".join("  " + f for f in fautes))
        return 1
    negatifs = [
        (ABI, "user_write(args[2], &masque.to_le_bytes())", "user_write(args[2], &1u64.to_le_bytes())"),
        (ORDO, "    let visible = masque & en_ligne;", "    let visible = masque;"),
        (ABI, "                return -errno::EINVAL;\n            }\n            let Some(masque)",
         "            }\n            let Some(masque)"),
    ]
    for n, (f, a, b) in enumerate(negatifs, 1):
        if not mutation(f, a, b):
            print(f"affinite visible : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"AFFINITE_VISIBLE_OK negatifs={len(negatifs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
