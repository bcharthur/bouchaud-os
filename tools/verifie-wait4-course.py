#!/usr/bin/env python3
"""`wait4` publie son attente AVANT de relire les zombies.

BOUCHAUD_WAIT4_PUBLIE_PUIS_RELIT_V1

# Le defaut

`sys_wait4` cherchait les zombies, n'en trouvait pas, PUIS posait
`waiting_for_child` et `Blocked`. Un fils mort entre les deux trouvait le
drapeau a faux et ne reveillait personne : le parent dormait pour toujours
(MESURE_DEMARRAGE_A_FROID §15). Le gros verrou serialisait les deux chemins ;
depuis le lot B8 (cycle de vie hors verrou), seule cette discipline les
ordonne. `wait4-course-probe` a attrape le blocage sur l'image B11.

# La regle

Dans `sys_wait4`, apres `waiting_for_child.range(true)` :
  1. une barriere `fence(...SeqCst)` ;
  2. une RELECTURE des zombies (`zombie_children`) ;
  3. puis seulement `schedule()`.
Dans `notify_parent_of_exit_for`, le reveil passe par un CAS sur
`waiting_for_child` (barriere pleine) -- l'autre moitie du motif croise.

Fail-closed ; deux tests negatifs.
"""

import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
PROC = "src/compat/linux/proc.rs"
VIE = "src/kernel/process/thread/lifecycle.rs"


def corps(texte: str, entete: str) -> str | None:
    debut = texte.find(entete)
    if debut < 0:
        return None
    ouverture = texte.find("{", debut)
    profondeur = 0
    for i in range(ouverture, len(texte)):
        if texte[i] == "{":
            profondeur += 1
        elif texte[i] == "}":
            profondeur -= 1
            if profondeur == 0:
                return texte[ouverture:i + 1]
    return None


def sans_commentaires(texte: str) -> str:
    return "\n".join(l.split("//", 1)[0] for l in texte.splitlines())


def verifie(racine: Path) -> list[str]:
    try:
        proc = sans_commentaires((racine / PROC).read_text(encoding="utf-8"))
        vie = sans_commentaires((racine / VIE).read_text(encoding="utf-8"))
    except OSError as e:
        return [f"lecture impossible : {e}"]
    fautes = []
    wait4 = corps(proc, "pub fn sys_wait4(")
    if wait4 is None:
        return [f"{PROC} : sys_wait4 introuvable"]
    pose = wait4.find("waiting_for_child.range(true)")
    barriere = wait4.find("fence(core::sync::atomic::Ordering::SeqCst)", pose)
    relit = wait4.find("zombie_children(", barriere)
    dort = wait4.find("schedule()", relit)
    if pose < 0:
        fautes.append("sys_wait4 ne publie plus waiting_for_child")
    elif barriere < 0:
        fautes.append("sys_wait4 : pas de barriere SeqCst apres la publication de l'attente")
    elif relit < 0:
        fautes.append("sys_wait4 : les zombies ne sont plus relus apres la publication")
    elif dort < 0:
        fautes.append("sys_wait4 : schedule() n'est plus apres la relecture")
    reveil = corps(vie, "fn notify_parent_of_exit_for(")
    if reveil is None:
        fautes.append(f"{VIE} : notify_parent_of_exit_for introuvable")
    elif "waiting_for_child" not in reveil or ".compare_exchange(true, false)" not in reveil:
        fautes.append("notify_parent_of_exit_for ne reveille plus par CAS sur waiting_for_child")
    return fautes


def mutation(fichier: str, avant: str, apres: str) -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        copie = Path(tmp)
        for f in (PROC, VIE):
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
        print("wait4 : ordre publication/relecture viole")
        print("\n".join("  " + f for f in fautes))
        return 1
    negatifs = [
        (PROC, "        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);\n        let deja_la", "        let deja_la"),
        (PROC, "        let deja_la = task::zombie_children(parent_pid)", "        let deja_la = alloc::vec::Vec::<(u32, i32)>::new()"),
    ]
    for n, (f, a, b) in enumerate(negatifs, 1):
        if not mutation(f, a, b):
            print(f"wait4 : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"WAIT4_COURSE_OK negatifs={len(negatifs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
