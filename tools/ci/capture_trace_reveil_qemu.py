#!/usr/bin/env python3
"""Capture les mesures invitées et les compteurs d'attente des vCPU de QEMU."""
import argparse
import json
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


def capture(vm, endpoint, output, symbols):
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
            with serial.open("rb") as log:
                pending = b""
                while vm.poll() is None and time.monotonic() < deadline:
                    debut = time.monotonic_ns()
                    stats = {}
                    for cpu in cpus:
                        path = Path(f"/proc/{vm.pid}/task/{cpu['thread-id']}/schedstat")
                        if path.exists():
                            try:
                                stats[cpu["cpu-index"]] = list(map(int, path.read_text().split()))
                            except FileNotFoundError:
                                continue
                    host.write(json.dumps({"avant_ns": debut, "apres_ns": time.monotonic_ns(), "schedstat": stats}) + "\n")
                    pending += log.read()
                    if b"TRACE_EXPORT_FIN" in pending:
                        break
                    pending = pending[-64:]
                    time.sleep(.02)
                else:
                    raise RuntimeError("Export invité absent")
            qmp(stream, "stop")
            for name, size in TAILLES.items():
                path = output / name
                response = qmp(stream, "human-monitor-command", {
                    "command-line": f'memsave {symbols[name]} {size} "{path}"',
                })
                if response or path.stat().st_size != size:
                    raise RuntimeError(f"Lecture RAM {name}: {response}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("elf", type=Path)
    parser.add_argument("bios", type=Path)
    parser.add_argument("scenario", type=Path)
    parser.add_argument("sortie", type=Path)
    args = parser.parse_args()
    sortie = args.sortie.resolve()
    sortie.mkdir(parents=True, exist_ok=True)
    symbols = {}
    for line in subprocess.check_output(["nm", args.elf], text=True).splitlines():
        fields = line.split()
        if len(fields) == 3 and fields[-1] in TAILLES:
            symbols[fields[-1]] = "0x" + fields[0]
    if symbols.keys() != TAILLES.keys():
        raise ValueError("Le BIOS doit contenir les quatre symboles de trace")

    with tempfile.TemporaryDirectory() as root, (sortie / "qemu.stderr").open("wb") as stderr:
        endpoint = Path(root) / "qmp"
        scenario = Path(root) / "scenario.img"
        shutil.copyfile(args.scenario, scenario)
        cmd = ["qemu-system-x86_64", "-drive", f"format=raw,snapshot=on,file={args.bios}",
               "-m", "4096", "-smp", "4", "-cpu", "max", "-display", "none", "-no-reboot",
               "-serial", f"file:{sortie / 'serial.log'}", "-netdev", "user,id=net0",
               "-device", "e1000,netdev=net0", "-audiodev", "none,id=muet",
               "-device", "AC97,audiodev=muet", "-drive", f"format=raw,file={scenario}",
               "-qmp", f"unix:{endpoint},server=on,wait=off"]
        with subprocess.Popen(cmd, stderr=stderr) as vm:
            try:
                capture(vm, endpoint, sortie, symbols)
            finally:
                if vm.poll() is None:
                    vm.terminate()
                    vm.wait(timeout=10)
    texte = (sortie / "serial.log").read_text(errors="replace")
    lignes = [ligne[ligne.index("ORDO_TRACE phase="):]
              for ligne in texte.splitlines() if "ORDO_TRACE phase=" in ligne]
    # Garder uniquement les tentatives dont l'export série est entier.
    valides = []
    for debut in range(0, len(lignes), 900):
        tentative = lignes[debut:debut + 900]
        if len(tentative) == 900 and all(re.search(r" i=(\d+) .* tache=-?\d+ entree_ns=-?\d+$", ligne)
                                       and f" i={i % 300} " in ligne
                                       for i, ligne in enumerate(tentative)):
            valides.extend(tentative)
    (sortie / "echantillons.txt").write_text("\n".join(valides) + "\n")
    print(f"{len(valides)}/{len(lignes)} échantillons dans des tentatives exportées intégralement")
    if not valides:
        raise ValueError("Aucune tentative intégralement exportée")


if __name__ == "__main__":
    main()
