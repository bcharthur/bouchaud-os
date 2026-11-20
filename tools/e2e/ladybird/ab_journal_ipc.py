#!/usr/bin/env python3
"""A/B Ladybird CPU sur disque froid. D'abord profil-rip, puis noyaux IPC."""
import argparse
import json
import math
from pathlib import Path
import re
import subprocess
import sys
import tarfile

from e2e import sha256
from prepare_disque import ajoute, ajoute_zone_persistante

ICI = Path(__file__).resolve().parent
SCENARIOS = "E2E-01,E2E-02,E2E-11"
BUILD = re.compile(r"BOUCHAUD_BUILD commit=([a-f0-9]{8,40})\b")
FRAME = re.compile(
    r"\[LB:FRAME\] onglet=(\d+) seq=(\d+) t=(\d+)[^\r\n]*?"
    r"copie_px=(\d+) complet=([01]) present_us=(\d+)"
)
PRESSION = re.compile(
    r"BOUCHAUD_CONSOLE_CONTRE_PRESSION attentes=(\d+) attente_pire_us=(\d+) attente_moyenne_us=(\d+)"
)


def profil(lignes, mode):
    if lignes.count("desktop") != 1:
        raise ValueError("autorun doit contenir exactement une commande desktop")
    lignes = [l for l in lignes if not re.match(r"^\s*profil-rip\s", l)]
    lignes.insert(lignes.index("desktop"), f"profil-rip {mode}")
    return lignes


def prepare_image(original, destination, mode):
    with tarfile.open(original) as source, tarfile.open(destination, "w", format=tarfile.USTAR_FORMAT) as target:
        autorun = 0
        for member in source:
            if member.isfile() and member.name in ("./autorun", "autorun"):
                texte = source.extractfile(member).read().decode()
                ajoute(target, member.name, "\n".join(profil(texte.split("\n"), mode)).encode(), member.mode)
                autorun += 1
            else:
                target.addfile(member, source.extractfile(member) if member.isfile() else None)
    if autorun != 1:
        raise ValueError("disque sans autorun unique")
    ajoute_zone_persistante(destination)


def quantiles(valeurs):
    valeurs = sorted(valeurs)
    return {"n": len(valeurs), **{
        f"p{q}": valeurs[math.ceil(len(valeurs) * q / 100) - 1] if valeurs else None
        for q in (50, 95, 99)
    }}


def analyse(journal):
    frames = [tuple(map(int, f)) for f in FRAME.findall(journal)]
    intervalles = [
        b[2] - a[2] for a, b in zip(frames, frames[1:])
        if a[0] == b[0] and b[1] == a[1] + 1 and b[2] >= a[2]
    ]
    pression = PRESSION.findall(journal)
    return dict(
        lignes=len(journal.splitlines()),
        presentation_us=quantiles(f[5] for f in frames),
        intervalle_invite_ms=quantiles(intervalles),
        completes=sum(f[4] for f in frames), copies_pixels=sum(f[3] for f in frames),
        contre_pression=dict(zip(("attentes", "max_us", "moy_us"), map(int, pression[-1]))) if pression else None,
    )


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--boot", nargs=2, required=True, metavar=("SHA", "IMAGE"))
    p.add_argument("--ipc", nargs=2, metavar=("SHA", "IMAGE"))
    p.add_argument("--disque", type=Path, required=True)
    p.add_argument("--sortie", type=Path, required=True)
    p.add_argument("--qemu", default="qemu-system-x86_64")
    p.add_argument("--accel", choices=("whpx", "tcg"), default="whpx")
    p.add_argument("--smp", type=int, default=4)
    p.add_argument("--memoire", type=int, default=8192)
    p.add_argument("--paires", type=int, default=3)
    o = p.parse_args()
    if not o.disque.is_file() or o.sortie.exists() or o.paires < 3:
        p.error("disque initial requis, sortie neuve et au moins trois paires")
    images = {}
    for entry in filter(None, (o.boot, o.ipc)):
        sha, nom = entry
        chemin = Path(nom).resolve()
        if not re.fullmatch(r"[0-9a-f]{40}", sha) or not chemin.is_file() or sha in images:
            p.error("SHA Git complet et bootimage distincte par noyau")
        images[sha] = (chemin, sha256(chemin))
    o.sortie.mkdir(parents=True)
    disques = {mode: o.sortie.resolve() / f"profil-{mode}.img" for mode in ("on", "off")}
    initial = sha256(o.disque)
    for mode, chemin in disques.items():
        prepare_image(o.disque, chemin, mode)
    empreintes = {mode: sha256(chemin) for mode, chemin in disques.items()}
    essais = []
    campagnes = {"journal": ((o.boot[0], "on"), (o.boot[0], "off"))}
    if o.ipc:
        campagnes["ipc"] = ((o.boot[0], "off"), (o.ipc[0], "off"))
    for campagne, bras in campagnes.items():
        for tour in range(o.paires):
            for sha, mode in bras if tour % 2 == 0 else reversed(bras):
                chemin, boot_sha256 = images[sha]
                disque = disques[mode]
                if sha256(chemin) != boot_sha256 or sha256(disque) != empreintes[mode]:
                    raise SystemExit("Entrées du banc modifiées")
                dossier = o.sortie / f"{campagne}-{tour + 1:02d}-{sha[:8]}-{mode}"
                commande = [
                    sys.executable, str(ICI / "e2e.py"), "--boot", str(chemin),
                    "--disque", str(disque), "--sortie", str(dossier), "--accel", o.accel,
                    "--whpx-irqchip", "off", "--qemu", o.qemu, "--smp", str(o.smp),
                    "--memoire", str(o.memoire), "--scenarios", SCENARIOS, "--exige", SCENARIOS,
                ]
                code = subprocess.run(commande, check=False).returncode
                log = dossier / "serial.log"
                texte = log.read_text(errors="replace") if log.is_file() else ""
                commits = BUILD.findall(texte)
                perf = dossier / "performance.json"
                verdict = bool(commits) and all(sha.startswith(c) for c in commits)
                verdict &= sha256(disque) == empreintes[mode] and sha256(chemin) == boot_sha256
                mesures = analyse(texte)
                essais.append(dict(campagne=campagne, paire=tour + 1, sha=sha, boot_sha256=boot_sha256,
                                   profil_rip=mode, code=code, provenance=verdict, mesures=mesures,
                                   performance=json.loads(perf.read_text(encoding="utf-8")) if perf.is_file() else None))
                rapport = dict(disque_source_sha256=initial, disques=empreintes, scenarios=SCENARIOS,
                               scenarios_sha256=sha256(ICI / "scenarios.json"), essais=essais)
                (o.sortie / "campagne.json").write_text(json.dumps(rapport, indent=2, ensure_ascii=False) + "\n",
                                                      encoding="utf-8")
    return int(sha256(o.disque) != initial or any(
        r["code"] or not r["provenance"] or not r["mesures"]["presentation_us"]["n"] for r in essais
    ))


if __name__ == "__main__":
    sys.exit(main())
