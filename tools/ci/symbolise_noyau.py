#!/usr/bin/env python3
"""Ce que faisaient les coeurs quand la machine a cesse d'avancer.

BOUCHAUD_SYMBOLISE_NOYAU_V1

    tools/ci/symbolise_noyau.py JOURNAL_SERIE ELF_NOYAU

Run 37617868893 (os-primitives sous KVM) : apres WAL_PROBE_OK la machine
tourne encore neuf minutes sans plus rien finir -- CPU0 dans `lseek` depuis
3747 ticks, toujours a la meme adresse noyau. Les releves [SMP-IPI]
(`cN=compte/age/rip/uN`) et [SCHED-FILE] (`rip_noyau=`) donnent ces adresses ;
seul le binaire EXACT qui a tourne sait les nommer, et il n'existe que sur le
runner (artefacts inaccessibles d'ici). Ce script tourne donc LA, a la fin du
banc, et imprime :
  - les dernieres lignes qui disent ou en etait le scenario ;
  - chaque adresse noyau des trois derniers releves, symbolisee (addr2line -i).
Ne juge rien, rend 0.
"""
import re
import shutil
import subprocess
import sys
from pathlib import Path

ANSI = re.compile(r"\x1b\[[0-9;]*m")
IPI = re.compile(r"\[SMP-IPI\].*")
COEUR = re.compile(r"c(\d+)=\d+/\d+ms/(0x[0-9a-f]+)/u\d")
FILE = re.compile(r"\[SCHED-FILE\] cpu=(\d+).*?rip_noyau=(0x[0-9a-f]+)")
SNAPSHOT = re.compile(r"\[SMP-SNAPSHOT\].*")
REPERES = re.compile(r"_OK\b|_FAIL|ECHEC|NNP_ABSENT|LECTEUR_ATTEND|PROCESS_FAULT|panicked|ata: |exec: |SESSION_|PRIMITIVES_FIN")


def main(argv):
    if len(argv) != 3:
        print(__doc__.strip().splitlines()[4], file=sys.stderr)
        return 0
    journal, elf = Path(argv[1]), Path(argv[2])
    if not journal.is_file():
        return 0
    lignes = [ANSI.sub("", l) for l in journal.read_text(errors="replace").splitlines()]
    print("\n== reperes du scenario (fin) ==")
    for l in [l for l in lignes if REPERES.search(l)][-25:]:
        print("  " + l[-220:])
    snaps = [l for l in lignes if SNAPSHOT.search(l)]
    if snaps:
        print("== dernier [SMP-SNAPSHOT] ==\n  " + SNAPSHOT.search(snaps[-1]).group(0)[:300])
    adresses = {}
    for l in [l for l in lignes if IPI.search(l)][-3:]:
        for cpu, rip in COEUR.findall(l):
            adresses.setdefault(rip, set()).add(f"c{cpu}")
    for l in [l for l in lignes if FILE.search(l)][-8:]:
        cpu, rip = FILE.search(l).groups()
        if rip != "0x0":
            adresses.setdefault(rip, set()).add(f"file{cpu}")
    outil = shutil.which("addr2line")
    print(f"== adresses noyau des derniers releves ({len(adresses)}) ==")
    for rip, ou in sorted(adresses.items()):
        if outil is None or not elf.is_file():
            print(f"  {rip} [{','.join(sorted(ou))}] -> non symbolisable (addr2line ou ELF absent)")
            continue
        try:
            sortie = subprocess.run([outil, "-f", "-C", "-i", "-e", str(elf), rip],
                                    capture_output=True, text=True, timeout=120).stdout.strip().splitlines()
        except (OSError, subprocess.TimeoutExpired) as erreur:
            print(f"  {rip} -> {erreur}")
            continue
        print(f"  {rip} [{','.join(sorted(ou))}]")
        for i in range(0, min(len(sortie), 12), 2):
            lieu = sortie[i + 1] if i + 1 < len(sortie) else "?"
            print(f"      {sortie[i]}  ({lieu.split('/src/')[-1]})")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
