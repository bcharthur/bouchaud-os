#!/usr/bin/env python3
"""Le banc BrowserHost ne quitte sa boucle que sur un verdict TERMINAL.

BOUCHAUD_SMOKE_TERMINAL_V1

# Le defaut

Ladybird #382, job « ordre worker », bras blob : la boucle sortait sur
`HOST_WORKER_AB_COMPLETE`, que la page ecrit AVANT ses verdicts FUNCTIONAL,
GLOBAL, COLD_START_PERF et SMOKE. Le banc tuait la VM avant que ces lignes
n'atteignent la console, puis les declarait « jamais atteintes » et ecrivait
« QEMU s'est arrete de lui-meme » -- `ab_complete` n'avait pas de branche.

# La regle

  1. tools/ci/smoke_terminal.py --test passe (ancienne regle prise en defaut,
     nouvelle sans perte, cas rouges toujours rouges) ;
  2. la page ecrit `HOST_WORKER_AB_VERDICT_COMPLETE` APRES `HOST_SMOKE_` ;
  3. le banc delegue sa sortie a smoke_terminal.py et ne sort plus sur
     `HOST_WORKER_AB_COMPLETE` ;
  4. le banc d'ordre exige le terminal de chaque bras.

Fail-closed ; trois tests negatifs.
"""
import subprocess
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
PAGE = "tools/health/browser_host_fixture.py"
BANC = "tools/ci/run_ladybird_browser_host.sh"
ORDRE = "tools/ci/run_worker_ordre.sh"
FICHIERS = (PAGE, BANC, ORDRE)


def verifie(racine: Path) -> list[str]:
    try:
        textes = {f: (racine / f).read_text(encoding="utf-8") for f in FICHIERS}
    except OSError as e:
        return [f"lecture impossible : {e}"]
    fautes = []
    page = textes[PAGE]
    smoke = page.find("console.log(`HOST_SMOKE_")
    terminal = page.find("console.log(`HOST_WORKER_AB_VERDICT_COMPLETE")
    if smoke < 0:
        fautes.append(f"{PAGE} : ligne HOST_SMOKE_ introuvable")
    elif terminal < 0:
        fautes.append(f"{PAGE} : HOST_WORKER_AB_VERDICT_COMPLETE n'est plus emis")
    elif terminal < smoke:
        fautes.append(f"{PAGE} : le terminal est emis AVANT HOST_SMOKE_")
    banc = "\n".join(l.split("#", 1)[0] for l in textes[BANC].splitlines())
    if "smoke_terminal.py" not in banc:
        fautes.append(f"{BANC} : la sortie de boucle ne passe plus par smoke_terminal.py")
    if '"HOST_WORKER_AB_COMPLETE"' in banc:
        fautes.append(f"{BANC} : HOST_WORKER_AB_COMPLETE redevient une condition de sortie")
    if "HOST_WORKER_AB_VERDICT_COMPLETE ordre=$ordre" not in textes[ORDRE]:
        fautes.append(f"{ORDRE} : le terminal de chaque bras n'est plus exige")
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
    test = subprocess.run([sys.executable, str(RACINE / "tools/ci/smoke_terminal.py"), "--test"],
                          capture_output=True, text=True)
    if test.returncode != 0 or "SMOKE_TERMINAL_OK" not in test.stdout:
        print("smoke terminal : le banc de protocole echoue")
        print(test.stdout + test.stderr)
        return 1
    fautes = verifie(RACINE)
    if fautes:
        print("smoke terminal : regle violee")
        print("\n".join("  " + f for f in fautes))
        return 1
    negatifs = [
        (PAGE, "  console.log(`HOST_WORKER_AB_VERDICT_COMPLETE", "  console.log(`HOST_WORKER_AB_VERDICT_FINI"),
        (BANC, "  sortie=$(python3 tools/ci/smoke_terminal.py",
         "  grep -aFq \"HOST_WORKER_AB_COMPLETE\" \"$LOG\" && break\n  sortie=$(python3 tools/ci/smoke_terminal.py"),
        (ORDRE, "HOST_WORKER_AB_VERDICT_COMPLETE ordre=$ordre", "HOST_WORKER_AB_COMPLETE ordre=$ordre"),
    ]
    for n, (f, a, b) in enumerate(negatifs, 1):
        if not mutation(f, a, b):
            print(f"smoke terminal : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"SMOKE_TERMINAL_GARDE_OK negatifs={len(negatifs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
