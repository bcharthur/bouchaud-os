#!/usr/bin/env python3
"""Capture les mesures invitées et les compteurs d'attente des vCPU de QEMU."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import socket
import subprocess
import tempfile
import time

TAILLES = {
    "BO_TRACE_REVEIL_TOTAL": 8, "BO_TRACE_REVEIL": 32768 * 88,
    "BO_TRACE_FILE_TOTAL": 8, "BO_TRACE_FILE": 32768 * 40,
    "BO_TRACE_CLOCK_BOOT_TSC": 8, "BO_TRACE_CLOCK_HZ": 8, "BO_TRACE_CLOCK_SYNC": 1,
    "BO_TRACE_HOTE_TOTAL": 8, "BO_TRACE_HOTE": 64 * 24,
}


def qmp(stream, command, args=None):
    msg = {"execute": command}
    if args is not None:
        msg["arguments"] = args
    stream.write(json.dumps(msg).encode() + b"\n")
    stream.flush()
    while line := stream.readline():
        reply = json.loads(line)
        if "error" in reply:
            raise RuntimeError(reply["error"])
        if "return" in reply:
            return reply["return"]
    raise RuntimeError("Connexion QMP interrompue")


def capture(vm, endpoint, output, symbols, avec_trace_hote):
    serial = output / "serial.log"
    with socket.socket(socket.AF_UNIX) as sock:
        sock.settimeout(10)
        deadline = time.monotonic() + 300
        while not endpoint.exists() and vm.poll() is None and time.monotonic() < deadline:
            time.sleep(.05)
        sock.connect(str(endpoint))
        with sock.makefile("rwb") as stream, (output / "hote-vcpu.jsonl").open("w") as host:
            stream.readline()
            qmp(stream, "qmp_capabilities")
            cpus = qmp(stream, "query-cpus-fast")
            host.write(json.dumps({"cpu_map": cpus}) + "\n")
            trace_hote = None
            if avec_trace_hote:
                from trace_hote_reveil import TraceHote
                trace_hote = TraceHote(vm, cpus, output / 'debugcon.bin', output)
            try:
                if trace_hote is not None:
                    qmp(stream, "cont")
                attend_export(vm, serial, host, cpus, deadline)
                qmp(stream, "stop")
            finally:
                if trace_hote is not None:
                    trace_hote.termine()
            for name, size in TAILLES.items():
                path = output / name
                response = qmp(stream, "human-monitor-command", {
                    "command-line": f'memsave {symbols[name]} {size} "{path}"',
                })
                if response or path.stat().st_size != size:
                    raise RuntimeError(f"Lecture RAM {name}: {response}")


def attend_export(vm, serial, host, cpus, deadline):
    while not serial.exists() and vm.poll() is None and time.monotonic() < deadline:
        time.sleep(.01)
    with serial.open("rb") as log:
        pending = b""
        while vm.poll() is None and time.monotonic() < deadline:
            debut = time.monotonic_ns()
            stats = {}
            for cpu in cpus:
                path = Path(f"/proc/{vm.pid}/task/{cpu['thread-id']}/schedstat")
                try:
                    stats[cpu["cpu-index"]] = list(map(int, path.read_text().split()))
                except FileNotFoundError:
                    continue
            host.write(json.dumps({"avant_ns": debut, "apres_ns": time.monotonic_ns(), "schedstat": stats}) + "\n")
            pending += log.read()
            if b"TRACE_EXPORT_FIN" in pending:
                return
            pending = pending[-64:]
            time.sleep(.02)
    raise RuntimeError("Export invité absent")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("elf", type=Path)
    parser.add_argument("bios", type=Path)
    parser.add_argument("scenario", type=Path)
    parser.add_argument("sortie", type=Path)
    parser.add_argument("--smp", type=int, choices=(4, 8), default=4)
    parser.add_argument("--hote-cpus", type=int, choices=(2, 4))
    parser.add_argument("--trace-hote", action="store_true")
    args = parser.parse_args()
    sortie = args.sortie.resolve()
    sortie.mkdir(parents=True, exist_ok=True)
    symbols = {}
    for line in subprocess.check_output(["nm", args.elf], text=True).splitlines():
        fields = line.split()
        if len(fields) == 3 and fields[-1] in TAILLES:
            symbols[fields[-1]] = "0x" + fields[0]
    if symbols.keys() != TAILLES.keys():
        raise ValueError("Le BIOS doit contenir les symboles de trace et d’horloge")

    with tempfile.TemporaryDirectory() as root, (sortie / "qemu.stderr").open("wb") as stderr:
        endpoint = Path(root) / "qmp"
        scenario = Path(root) / "scenario.img"
        shutil.copyfile(args.scenario, scenario)
        cmd = ["qemu-system-x86_64", "-drive", f"format=raw,file={args.bios}",
               "-m", "4096", "-smp", str(args.smp), "-cpu", "max", "-display", "none", "-no-reboot",
               "-serial", f"file:{sortie / 'serial.log'}", "-netdev", "user,id=net0",
               "-device", "e1000,netdev=net0", "-audiodev", "none,id=muet",
               "-device", "AC97,audiodev=muet", "-drive", f"format=raw,file={scenario}",
               "-qmp", f"unix:{endpoint},server=on,wait=off"]
        if args.trace_hote:
            cmd += ['-S', '-debugcon', f'file:{sortie / "debugcon.bin"}']
        if args.hote_cpus:
            cpus = sorted(os.sched_getaffinity(0))
            if len(cpus) < args.hote_cpus:
                raise ValueError(f'Affinité hôte insuffisante : {cpus}')
            # Même sélection sur tous les boots. Le reste des paramètres QEMU
            # est identique, seul le nombre de CPU Linux alloués change.
            cmd[:0] = ['taskset', '-c', ','.join(map(str, cpus[:args.hote_cpus]))]
        (sortie / 'qemu-argv.json').write_text(json.dumps(cmd) + '\n')
        with subprocess.Popen(cmd, stderr=stderr) as vm:
            try:
                capture(vm, endpoint, sortie, symbols, args.trace_hote)
            finally:
                if vm.poll() is None:
                    vm.terminate()
                    vm.wait(timeout=10)
    texte = (sortie / "serial.log").read_text(errors="replace")
    lignes = [ligne[ligne.index("ORDO_TRACE phase="):]
              for ligne in texte.splitlines() if "ORDO_TRACE phase=" in ligne]
    # Garder uniquement les tentatives dont l'export série est entier.
    valides = rassemble_exports(texte)
    (sortie / "echantillons.txt").write_text("\n".join(valides) + "\n")
    print(f"{len(valides)} échantillons récupérés dans {len(lignes)} lignes de trace")
    if not valides:
        raise ValueError("Aucune tentative intégralement exportée")


CHAMPS = ("echeance_us", "retard_us", "sommeil", "ecart", "non_apparies",
          "entree_noyau_us", "pret_ns", "externe_ns", "masque_ns", "inexplique_ns",
          "tache", "entree_ns")
PHASES = ("au repos", "sous charge (normal)", "sous charge (interactif)")
TRAME = re.compile(r"ORDO_TRACE phase=(.*?) i=(\d+)" +
                   "".join(fr" {champ}=-?\d+" for champ in CHAMPS))


def rassemble_exports(texte):
    # Les copies sont émises après les mesures et contiennent le même fichier.
    # N'utiliser que des trames complètes et refuser les copies contradictoires.
    copies = re.split(r"(?m)^.*?\+ echo TRACE_EXPORT_COPIE_[123]\s*$", texte)
    blocs = []
    for copie in copies:
        lignes = [s[s.index("ORDO_TRACE phase="):] for s in copie.splitlines()
                  if "ORDO_TRACE phase=" in s]
        if lignes:
            blocs.append(lignes)
    if not blocs:
        return []
    valides = []
    for debut in range(0, max(map(len, blocs)), 900):
        tentative = []
        for i in range(900):
            possibles = set()
            for bloc in blocs:
                if len(bloc) <= debut + i:
                    continue
                ligne = bloc[debut + i]
                trame = TRAME.fullmatch(ligne)
                if trame and (trame[1], int(trame[2])) == (PHASES[i // 300], i % 300):
                    possibles.add(ligne)
            if len(possibles) != 1:
                break
            tentative.append(possibles.pop())
        if len(tentative) == 900:
            valides.extend(tentative)
    return valides



if __name__ == "__main__":
    main()
