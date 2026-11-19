#!/usr/bin/env bash
# Requires a VIRGL-enabled BIOS bootimage, QEMU SDL/GL and a free X display.
set -euo pipefail
cd "$(dirname "$0")/../.."
if [[ $# != 2 || -z $(printenv DISPLAY) ]]; then
    echo 'usage: DISPLAY=:N run_virtgpu_ring3.sh BOOTIMAGE NEW_OUTPUT_DIR' >&2
    exit 2
fi
boot=$(realpath "$1")
sortie=$(realpath -m "$2")
mkdir "$sortie"
pid=""
arrete() {
    if [[ -n $pid ]]; then
        kill "$pid" 2>/dev/null || true
        for _ in 1 2 3 4 5; do
            kill -0 "$pid" 2>/dev/null || break
            sleep 0.2
        done
        if kill -0 "$pid" 2>/dev/null; then
            kill -KILL "$pid" 2>/dev/null || true
        fi
        wait "$pid" 2>/dev/null || true
        pid=""
    fi
}
trap arrete EXIT
mkdir "$sortie/virgl" "$sortie/2d"
musl-gcc -Wall -Wextra -O2 -static-pie tools/userland/virtgpu-caps-probe.c -o "$sortie/virgl/caps"
musl-gcc -Wall -Wextra -O2 -static-pie tools/userland/mire-gpu.c -o "$sortie/virgl/mire-gpu"
musl-gcc -Wall -Wextra -O2 -static-pie tools/userland/virtgpu-textures.c -o "$sortie/virgl/textures"
cp "$sortie/virgl/caps" "$sortie/2d/caps"
printf '/caps\n/textures\nexport BO_AUTOSTART_APP=/mire-gpu\ndesktop\n' > "$sortie/virgl/autorun"
printf '/caps absent\n' > "$sortie/2d/autorun"

demarre() {
    local nom=$1 peripherique=$2 affichage=$3
    local repertoire="$sortie/$nom"
    IMAGE="$sortie/$nom.img" tools/userland/mkdisk.sh "$repertoire" >/dev/null
    qemu-system-x86_64 -accel tcg -cpu max -smp 2 -m 2048 -vga none \
        -device "$peripherique" -display "$affichage" -no-reboot \
        -drive "format=raw,file=$boot,snapshot=on" \
        -drive "format=raw,file=$sortie/$nom.img,snapshot=on" \
        -serial "file:$repertoire/serie.log" >"$repertoire/qemu.log" 2>&1 &
    pid=$!
}

attend() {
    local nom=$1 marqueur=$2 fin=$((SECONDS + 60))
    while (( SECONDS < fin )); do
        if grep -aqF "$marqueur" "$sortie/$nom/serie.log" 2>/dev/null; then return 0; fi
        kill -0 "$pid" 2>/dev/null || break
        sleep 0.5
    done
    echo "VIRTGPU_RING3_ECHEC $nom marqueur_absent=$marqueur" >&2
    cat "$sortie/$nom/qemu.log" >&2
    return 1
}

demarre virgl virtio-gpu-gl-pci sdl,gl=on
attend virgl 'VIRTGPU_BLIT_RING3_OK pixels=64'
attend virgl 'VIRTGPU_CAPS_OK'
attend virgl 'VIRTGPU_TEXTURES_RING3_OK'
attend virgl 'VIRTGPU_TEXTURES_VARIABLES_OK'
attend virgl 'GPU_COUCHE'
sleep 3
import -window root "$sortie/virgl/capture.png"
python3 tools/ci/analyse_gpu_composition.py mire "$sortie/virgl/serie.log" "$sortie/virgl/capture.png"
arrete

demarre 2d virtio-gpu-pci none
attend 2d 'BOUCHAUD_VIRTIO_GPU_2D_AUTOTEST_OK'
attend 2d 'VIRTGPU_CAPS_ABSENT_OK'
if grep -aEq 'KERNEL PANIC|panicked at|DOUBLE FAULT|TRIPLE FAULT' "$sortie"/{virgl,2d}/serie.log; then
    echo 'VIRTGPU_RING3_ECHEC faute noyau' >&2
    exit 1
fi
echo VIRTGPU_RING3_OK
