#!/usr/bin/env python3
"""Automate the pinned six-pass Windows WHPX campaign."""

import argparse
from contextlib import contextmanager
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import socket
import subprocess
import tarfile
from threading import Thread
import time
from urllib.request import urlopen

HEADS = {"A": "c856ece9aaced8167aa8c783276ca3a5da7222a4",
         "B": "218a556c733d412b30fc11c9ad0384e824edecf3"}
TOOLS = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("analysis", TOOLS / "analyse-whpx-browserhost.py")
ANALYSIS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ANALYSIS)
FIXTURE = "ladybird-whpx-p0.html"
QUERY = f"/{FIXTURE}?phases=1&auto-start=1&phase-markers=1"
BOOT = "target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin"
MIB = 1024 * 1024
PROFILE = "whpx,irqchip=off,smp=1,ram_mib=8192,surface=simple,audio=none,cpu"
PANIC = re.compile(r"KERNEL PANIC|panicked at|PANIC:|BOUCHAUD_PANIC|\[LOCKDEP\].*violations=[1-9]", re.I)


def digest(stream):
    sha = hashlib.sha256()
    for block in iter(lambda: stream.read(MIB), b""):
        sha.update(block)
    return sha.hexdigest()


def sha256(path):
    with path.open("rb") as source:
        return digest(source)


def save(path, value):
    path.write_text(json.dumps(value, indent=2, allow_nan=False) + "\n", encoding="utf-8")


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args], text=True).strip()


def disk_state(path):
    with path.open("rb") as disk:
        disk.seek(-128 * MIB, os.SEEK_END)
        for _ in range(128):
            if disk.read(MIB).strip(b"\0"):
                raise ValueError("Zone persistante initiale non vide")
    entries = {}
    with tarfile.open(path, "r:") as archive:
        for entry in archive:
            name = entry.name.removeprefix("./")
            if name in ("bo-navigateur", "usr/libexec/ladybird", "usr/share/ladybird"):
                continue
            if name.startswith(("usr/libexec/ladybird/", "usr/share/ladybird/")):
                continue
            if name in entries:
                raise ValueError(f"Entree disque dupliquee : {name}")
            content = digest(archive.extractfile(entry)) if entry.isfile() else entry.linkname
            entries[name] = (entry.type.hex(), entry.mode, entry.uid, entry.gid, content)
    return hashlib.sha256(json.dumps(entries, sort_keys=True).encode()).hexdigest()


@contextmanager
def serve(body, log):
    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            if self.path != QUERY:
                self.send_error(404)
                return
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, fmt, *args):
            with log.open("a", encoding="utf-8") as output:
                print(fmt % args, file=output)

    with ThreadingHTTPServer(("127.0.0.1", 0), Handler) as server:
        worker = Thread(target=server.serve_forever, daemon=True)
        worker.start()
        try:
            yield server.server_port
        finally:
            server.shutdown()
            worker.join()


def receive(wire):
    line = wire.readline()
    if not line:
        raise ValueError("Connexion QMP fermee")
    return json.loads(line)


def send(wire, name):
    wire.write(json.dumps({"execute": name}).encode() + b"\r\n")
    wire.flush()


def command(wire, name):
    send(wire, name)
    while True:
        message = receive(wire)
        if "event" in message:
            continue
        if "return" not in message:
            raise ValueError(f"QMP {name}: {message}")
        return message["return"]


def wait_for(process, seconds, probe):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        result = probe()
        if process.poll() is not None:
            raise ValueError(f"Lanceur arrete avant la fin, code {process.returncode}")
        if result is not None:
            return result
        time.sleep(0.2)
    raise ValueError("Delai depasse en attendant la VM ou le scenario")


def connect(port):
    try:
        return socket.create_connection(("127.0.0.1", port), timeout=1)
    except OSError:
        return None


def completed(logdir, old_logs):
    paths = set(logdir.glob("*-serie.log")) - old_logs
    if len(paths) > 1:
        raise ValueError("Plusieurs nouveaux journaux serie")
    if not paths:
        return None
    log = paths.pop()
    try:
        text = log.read_text(encoding="utf-8", errors="replace")
    except PermissionError:
        return None
    text = ANALYSIS.strip_ansi(text)
    if PANIC.search(text):
        raise ValueError("Panique ou violation LOCKDEP dans le journal")
    # Require a complete WebContent line, even while QEMU is still writing.
    lines = text.rsplit("\n", 1)[0]
    if any(match[3] == "CALLBACK" for match in ANALYSIS.JS.finditer(lines)):
        return log
    return None


def run_trial(root, trial, powershell, evidence, boot, run, invariants):
    logdir = root / "target/journaux-qemu"
    old_logs = set(logdir.glob("*-serie.log"))
    disk = root / "ladybird-browser.img"
    if disk.exists():
        raise ValueError(f"Disque deja present : {disk}")
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))
        port = reservation.getsockname()[1]
    args = [
        str(powershell), "-NoProfile", "-File", str(root / "run-whpx-ab.ps1"),
        "-BenchmarkBootImage", str(boot), "-BenchmarkBootSha256", evidence["boot_sha256"],
        "-BenchmarkMonitorPort", str(port), "-Accel", "whpx", "-WhpxIrqchip", "off",
        "-CpuCount", "1", "-RamMiB", "8192", "-Surface", "simple", "-Audio", "none",
        "-LadybirdRunId", str(run), "-Gate0Autostart", "-LadybirdUrl", evidence["url"],
    ]
    with (trial / "launcher.log").open("wb") as output:
        process = subprocess.Popen(args, cwd=root, stdin=subprocess.DEVNULL,
                                   stdout=output, stderr=subprocess.STDOUT)
        try:
            with wait_for(process, 1200, lambda: connect(port)) as connection:
                connection.settimeout(10)
                with connection.makefile("rwb") as wire:
                    if "QMP" not in receive(wire):
                        raise ValueError("Salutation QMP absente")
                    command(wire, "qmp_capabilities")
                    if command(wire, "query-name")["name"] != f"bouchaud-whpx-{port}":
                        raise ValueError("QMP ne correspond pas au lanceur")
                    if command(wire, "query-status")["status"] != "prelaunch":
                        raise ValueError("QEMU a demarre avant verification du disque")
                    state = disk_state(disk)
                    if state != invariants.setdefault("disk_state_sha256", state):
                        raise ValueError("Etat initial du disque different hors charge Ladybird")
                    browser = root / "native-browser-m9"
                    manifest = browser / "BOUCHAUD_ARTIFACT_MANIFEST.json"
                    shutil.copyfile(manifest, trial / manifest.name)
                    hashes = {key: sha256(browser / name) for key, name in (
                        ("manifest_sha256", manifest.name), ("browserhost_sha256", "BouchaudBrowserHost"),
                        ("webcontent_sha256", "WebContent"),
                    )}
                    if hashes != invariants.setdefault(root, hashes):
                        raise ValueError("Artefact different entre essais du meme bras")
                    evidence.update(hashes, disk_state_sha256=state, disk_sha256=sha256(disk))
                    command(wire, "cont")
                    log = wait_for(process, 180, lambda: completed(logdir, old_logs))
                    # QEMU can close the socket before replying to quit.
                    send(wire, "quit")
            if process.wait(timeout=30):
                raise ValueError(f"Sortie QEMU/lanceur non nulle : {process.returncode}")
            text = ANALYSIS.strip_ansi(log.read_text(encoding="utf-8", errors="replace"))
            prefix = evidence["boot_commit"][:12]
            if (PANIC.search(text) or not re.search(rf"BOUCHAUD_BUILD commit={prefix}\b", text)
                    or not re.search(r"BOUCHAUD_CPU_ISA hyperviseur=MicrosoftHv\b", text)):
                raise ValueError("Panique, noyau inattendu ou signature WHPX absente")
            result = ANALYSIS.analyse(log)
            view = result["viewport"]
            if view != invariants.setdefault("viewport", view):
                raise ValueError(f"Dimensions differentes entre sessions : {view}")
            if sha256(boot) != evidence["boot_sha256"] or sha256(disk) != evidence["disk_sha256"]:
                raise ValueError("Image source modifiee en mode snapshot")
            save(trial / "evidence.json", evidence)
        finally:
            if process.poll() is None:
                subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"],
                               stdout=output, stderr=output, check=False)
                process.wait(timeout=15)
            new_logs = set(logdir.glob("*-serie.log")) - old_logs
            for log in new_logs:
                shutil.copyfile(log, trial / ("serial.log" if len(new_logs) == 1 else log.name))
                qemu_log = log.with_name(log.name.replace("-serie.log", "-qemu.log"))
                if qemu_log.is_file():
                    shutil.copyfile(qemu_log, trial / qemu_log.name)
            if disk.exists():
                disk.unlink()


def campaign(args):
    if os.name != "nt":
        raise ValueError("Windows et WHPX requis")
    sources = {"A": args.reference.resolve(strict=True), "B": args.candidate.resolve(strict=True)}
    for arm, sha in HEADS.items():
        # Inspect the user's worktrees without changing them.
        if git(sources[arm], "rev-parse", "HEAD") != sha:
            raise ValueError(f"{sources[arm]} doit pointer sur {sha}")
    qemu = Path(os.environ["ProgramFiles"]) / "qemu/qemu-system-x86_64.exe"
    ps = Path(os.environ["SystemRoot"]) / "System32/WindowsPowerShell/v1.0/powershell.exe"
    for tool in ("git", "gh", "cargo", "taskkill", "tasklist"):
        if not shutil.which(tool):
            raise ValueError(f"Outil absent : {tool}")
    if not ps.is_file() or not qemu.is_file():
        raise ValueError("Windows PowerShell 5.1 ou QEMU introuvable")
    if "whpx" not in subprocess.check_output([str(qemu), "-accel", "help"], text=True).split():
        raise ValueError("WHPX absent de QEMU")
    running = subprocess.check_output(
        ["tasklist", "/FI", "IMAGENAME eq qemu-system-x86_64.exe", "/FO", "CSV", "/NH"],
        text=True, errors="replace",
    )
    if "qemu-system-x86_64.exe" in running.lower():
        raise ValueError("Fermer la VM QEMU deja active")
    output = args.output.resolve()
    output.mkdir(parents=True)
    fixture = (TOOLS.parent / "health" / FIXTURE).read_bytes()
    expected = subprocess.check_output(
        ["git", "-C", str(sources["B"]), "show", f"{HEADS['B']}:tools/health/{FIXTURE}"]
    )
    if fixture.replace(b"\r\n", b"\n") != expected:
        raise ValueError("Fixture differente de celle de #366")
    launcher = TOOLS.parents[1] / "run.ps1"
    common = dict(launcher_sha256=sha256(launcher), qemu_sha256=sha256(qemu),
                  fixture_sha256=hashlib.sha256(fixture).hexdigest(), profile=PROFILE, mode=args.mode)
    roots, boots, invariants = {}, {}, {}
    runs = {"A": args.reference_run, "B": args.candidate_run}
    kernels = {"A": "A", "B": "A" if args.mode == "CommonKernel" else "B"}
    try:
        for arm, sha in HEADS.items():
            root = output / f"src-{arm}"
            git(sources[arm], "worktree", "add", "--detach", str(root), sha)
            roots[arm] = root
            if arm in kernels.values():
                print(f"Construction du noyau {arm} {sha}", flush=True)
                with (output / f"build-{arm}.log").open("wb") as log:
                    subprocess.run(["cargo", "bootimage", "--locked"], cwd=root,
                                   stdout=log, stderr=subprocess.STDOUT, check=True, timeout=1800)
                boot = output / f"boot-{arm}.bin"
                shutil.copyfile(root / BOOT, boot)
                boots[arm] = boot, sha256(boot)
            shutil.copyfile(launcher, root / "run-whpx-ab.ps1")
        with serve(fixture, output / "http.log") as port:
            for index, arm in enumerate(ANALYSIS.ORDER, 1):
                print(f"Passage {index}/6 ({arm}, {args.mode})", flush=True)
                trial = output / f"{index:02}-{arm}"
                trial.mkdir()
                root = roots[arm]
                kernel = kernels[arm]
                boot, boot_hash = boots[kernel]
                if git(root, "rev-parse", "HEAD") != HEADS[arm] or git(root, "status", "--porcelain", "-uno"):
                    raise ValueError(f"Checkout de mesure inattendu ou modifie : {root}")
                if (sha256(boot) != boot_hash or sha256(qemu) != common["qemu_sha256"]
                        or sha256(root / "run-whpx-ab.ps1") != common["launcher_sha256"]):
                    raise ValueError("Bootimage, QEMU ou lanceur modifie")
                with urlopen(f"http://127.0.0.1:{port}{QUERY}", timeout=5) as response:
                    if response.status != 200 or response.read() != fixture:
                        raise ValueError("Fixture HTTP ne correspond pas aux octets attendus")
                evidence = dict(common, arm=arm, checkout=HEADS[arm], run_id=runs[arm],
                                boot_commit=HEADS[kernel], boot_sha256=boot_hash,
                                url=f"http://10.0.2.2:{port}{QUERY}")
                run_trial(root, trial, ps, evidence, boot,
                          0 if root in invariants else runs[arm], invariants)
        save(output / "rapport.json", ANALYSIS.campaign(output))
        print(f"Rapport : {output / 'rapport.json'}", flush=True)
    finally:
        for arm, root in roots.items():
            try:
                git(sources[arm], "worktree", "remove", "--force", str(root))
            except subprocess.CalledProcessError:
                print(f"Checkout temporaire conserve : {root}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for option in ("reference", "candidate", "output"):
        parser.add_argument(f"--{option}", type=Path, required=True)
    parser.add_argument("--mode", choices=("CommonKernel", "FullStack"), required=True)
    parser.add_argument("--reference-run", type=int, required=True)
    parser.add_argument("--candidate-run", type=int, required=True)
    try:
        campaign(parser.parse_args())
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as exc:
        parser.exit(1, f"Campagne WHPX non qualifiable : {exc}\n")
