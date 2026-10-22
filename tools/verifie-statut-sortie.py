#!/usr/bin/env python3
"""`wait4` distingue une sortie de code 128..255 d'une mort par signal.

BOUCHAUD_STATUT_SORTIE_V1

# Le defaut

Une mort par signal etait notee comme le code `128 + signal`, et `wait4`
decodait tout code >= 128 comme une mort par signal. statut-sortie-probe,
TCG SMP4 : `exit(128)` rendu comme `exit(0)` -- un echec lu comme un
succes --, `exit(130)` comme SIGINT, `exit(200)` comme « signal 72 »,
`exit(255)` comme « signal 127 ».

# La regle

  1. `ProcessLifecycle.par_signal`, pose dans `sortie_du_fil` AVEC
     `exit_code`, sous le meme verrou ;
  2. toute mort par signal passe par `exit_group_par_signal` : plus aucun
     `exit_group(128 + ...)` ;
  3. `sys_wait4` encode par `statut_wait(code, par_signal)` -- jamais par un
     test `code >= 128` ;
  4. la sonde tourne dans os-primitives.

Fail-closed ; tests negatifs.
"""

import re
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
VIE = "src/kernel/process/thread/lifecycle.rs"
PROC = "src/compat/linux/proc.rs"
PRIMITIVES = "tools/ci/run_os_primitives.sh"
# BOUCHAUD_VERDICT_OS_PRIMITIVES_V1 : le marqueur est exige par le verdict a part.
VERDICT = "tools/ci/verdict_os_primitives.sh"
FICHIERS = (VERDICT, VIE, PROC, PRIMITIVES)


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
        t = {f: (racine / f).read_text(encoding="utf-8") for f in FICHIERS}
        sources = {p: sans_commentaires(p.read_text(encoding="utf-8"))
                   for p in (racine / "src").rglob("*.rs")}
    except OSError as e:
        return [f"lecture impossible : {e}"]
    vie, proc = sans_commentaires(t[VIE]), sans_commentaires(t[PROC])
    fautes = []

    fil = corps(vie, "fn sortie_du_fil(code: i32, par_signal: bool)")
    if fil is None:
        fautes.append("sortie_du_fil(code, par_signal) introuvable")
    else:
        code = fil.find("lifecycle.exit_code = code;")
        drapeau = fil.find("lifecycle.par_signal = par_signal;")
        if code < 0 or drapeau < 0 or not 0 < drapeau - code < 120:
            fautes.append("sortie_du_fil ne pose plus par_signal avec exit_code")
    signal = corps(vie, "pub fn exit_group_par_signal(signal: i32)")
    if signal is None or "sortie_du_groupe(128 + signal, true)" not in signal:
        fautes.append("exit_group_par_signal ne marque plus la mort par signal")

    for chemin, texte in sources.items():
        if re.search(r"exit_group\(\s*128\s*\+", texte):
            fautes.append(f"{chemin.relative_to(racine)} : exit_group(128 + ...) -- passer par exit_group_par_signal")

    wait4 = corps(proc, "pub fn sys_wait4(")
    statut = corps(proc, "pub fn statut_wait(code: i32, par_signal: bool)")
    if wait4 is None or "statut_wait(code, par_signal)" not in wait4:
        fautes.append("sys_wait4 n'encode plus par statut_wait(code, par_signal)")
    elif re.search(r"code\s*>=\s*128", wait4):
        fautes.append("sys_wait4 devine encore une mort par signal d'un code >= 128")
    if statut is None or "if par_signal" not in statut or "<< 8" not in statut:
        fautes.append("statut_wait ne distingue plus sortie et signal")

    if "/bin/statut-sortie-probe" not in t[PRIMITIVES] or "STATUT_SORTIE_OK" not in t[VERDICT]:
        fautes.append("os-primitives ne lance plus statut-sortie-probe")
    return fautes


def mutation(fichier: str, avant: str, apres: str) -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        copie = Path(tmp)
        for p in (RACINE / "src").rglob("*.rs"):
            dest = copie / p.relative_to(RACINE)
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_text(p.read_text(encoding="utf-8"), encoding="utf-8")
        dest = copie / PRIMITIVES
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text((RACINE / PRIMITIVES).read_text(encoding="utf-8"), encoding="utf-8")
        cible = copie / fichier
        texte = cible.read_text(encoding="utf-8")
        if avant not in texte:
            return False
        cible.write_text(texte.replace(avant, apres, 1), encoding="utf-8")
        return bool(verifie(copie))


def main() -> int:
    fautes = verifie(RACINE)
    if fautes:
        print("statut de sortie : regle violee")
        print("\n".join("  " + f for f in fautes))
        return 1
    negatifs = [
        (VIE, "        lifecycle.par_signal = par_signal;\n", ""),
        (VIE, "sortie_du_groupe(128 + signal, true)", "sortie_du_groupe(128 + signal, false)"),
        ("src/arch/x86_64/idt/exceptions.rs", "crate::kernel::task::exit_group_par_signal(signal as i32)",
         "crate::kernel::task::exit_group(128 + signal as i32)"),
        (PROC, "let status = statut_wait(code, par_signal);",
         "let status = if code >= 128 { (code as u32 - 128) & 0x7F } else { ((code as u32) & 0xFF) << 8 };"),
        (PRIMITIVES, "/bin/statut-sortie-probe", "/bin/true"),
    ]
    for n, (f, a, b) in enumerate(negatifs, 1):
        if not mutation(f, a, b):
            print(f"statut de sortie : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"STATUT_SORTIE_OK negatifs={len(negatifs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
