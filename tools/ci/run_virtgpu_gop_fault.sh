#!/usr/bin/env bash
# Requiert une image UEFI Stage 2 banc-gpu-perte-scanout et un affichage X libre.
set -euo pipefail
cd "$(dirname "$0")/../.."
if [[ $# != 4 || -z ${DISPLAY:-} ]]; then
    echo 'usage: run_virtgpu_gop_fault.sh IMAGE OVMF_CODE OVMF_VARS NEW_OUTPUT_DIR' >&2
    exit 2
fi
boot=$(realpath "$1")
code=$(realpath "$2")
vars=$(realpath "$3")
sortie=$(realpath -m "$4")
mkdir "$sortie" "$sortie/scen"
musl-gcc -O2 -static-pie tools/userland/mire-gpu.c -o "$sortie/scen/mire-gpu"
IMAGE="$sortie/disque.img" tools/userland/mkdisk.sh "$sortie/scen" >/dev/null
# Le socket doit tenir dans sun_path même si la sortie est profonde.
qmp=$(mktemp -d)
pid=''
arrete() {
    if [[ -n $pid ]]; then
        kill "$pid" 2>/dev/null || true
        wait "$pid" 2>/dev/null || true
    fi
    rm -f "$qmp/mon"
    rmdir "$qmp"
}
trap arrete EXIT
timeout --kill-after=5 70 qemu-system-x86_64 -accel tcg -cpu max -smp 2 -m 2048 -vga none \
    -device VGA,id=gop -device virtio-gpu-gl-pci,id=gpu -display gtk,gl=on -no-reboot \
    -drive "if=pflash,unit=0,format=raw,readonly=on,file=$code" \
    -drive "if=pflash,unit=1,format=raw,snapshot=on,file=$vars" \
    -drive "if=ide,index=0,format=raw,snapshot=on,file=$boot" \
    -drive "if=ide,index=1,format=raw,snapshot=on,file=$sortie/disque.img" \
    -serial "file:$sortie/serie.log" -qmp "unix:$qmp/mon,server=on,wait=off" \
    >"$sortie/qemu.log" 2>&1 &
pid=$!
python3 - "$qmp/mon" "$sortie" <<'PY'
import json
import socket
import sys
import time
from pathlib import Path

socket_path, output_path = sys.argv[1:]
out = Path(output_path)
serial = out / "serie.log"
deadline = time.monotonic() + 55

def attend(marqueur):
    while time.monotonic() < deadline:
        if serial.exists() and marqueur in serial.read_bytes():
            return
        time.sleep(0.05)
    sys.exit(f"GOP_PERTE_ECHEC marqueur_absent={marqueur!r}")

attend(b"GPU_COUCHE")
attend(b"GPU_PERTE_SCANOUT_INJECTION")
attend(b"UEFI scanout=gop raison=perte-virtio")
attend(b"MIRE_GPU trames=600")
trace = serial.read_bytes()
perte = trace.index(b"GPU_PERTE_SCANOUT_INJECTION")
gop = trace.index(b"UEFI scanout=gop raison=perte-virtio")
trames = trace.index(b"MIRE_GPU trames=600")
if not trace.index(b"GPU_COUCHE") < perte < gop < trames:
    sys.exit("GOP_PERTE_ECHEC ordre chronologique")
if b"plan_2d=0" not in trace[perte:trace.index(b"\n", perte)]:
    sys.exit("GOP_PERTE_ECHEC plan 2D present")
with socket.socket(socket.AF_UNIX) as qmp:
    qmp.settimeout(5)
    qmp.connect(socket_path)
    with qmp.makefile("rwb") as stream:
        json.loads(stream.readline())

        def commande(nom, **arguments):
            stream.write(json.dumps({"execute": nom, "arguments": arguments}).encode() + b"\n")
            stream.flush()
            while True:
                message = json.loads(stream.readline())
                if "error" in message:
                    sys.exit(f"GOP_PERTE_ECHEC qmp={message['error']}")
                if "return" in message:
                    return

        commande("qmp_capabilities")
        commande("screendump", filename=str(out / "apres.ppm"), device="gop", head=0)
PY
python3 tools/ci/analyse_gpu_composition.py mire "$sortie/serie.log" "$sortie/apres.ppm"
if grep -aEq 'KERNEL PANIC|panicked at|DOUBLE FAULT|TRIPLE FAULT' "$sortie/serie.log"; then
    exit 1
fi
echo GOP_PERTE_SCANOUT_OK
