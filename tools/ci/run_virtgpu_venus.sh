#!/usr/bin/env bash
# QEMU 10+ et GTK/GL avec un virglrenderer qui propose Venus.
set -euo pipefail
cd "$(dirname "$0")/../.."
if [[ $# != 2 || -z ${DISPLAY:-} ]]; then
    echo 'usage: DISPLAY=:N run_virtgpu_venus.sh BOOTIMAGE_BANC_VENUS NEW_OUTPUT_DIR' >&2
    exit 2
fi
boot=$(realpath "$1")
sortie=$(realpath -m "$2")
mkdir "$sortie"
mkdir "$sortie/scen"
musl-gcc -Wall -Wextra -O2 -static-pie tools/userland/virtgpu-venus-version.c -o "$sortie/scen/version"
if [[ ${BO_VENUS_DEMAPPE:-0} == 1 ]]; then
    printf '/version demappe\n' > "$sortie/scen/autorun"
else
    printf '/version\n' > "$sortie/scen/autorun"
fi
IMAGE="$sortie/guest.img" tools/userland/mkdisk.sh "$sortie/scen" >/dev/null
timeout --kill-after=5 45 qemu-system-x86_64 -accel tcg -cpu max -smp 2 -m 2048 -vga none \
    -device virtio-gpu-gl-pci,venus=on,blob=on,hostmem=256M -display gtk,gl=on -no-reboot \
    -drive "format=raw,file=$boot,snapshot=on" \
    -drive "format=raw,file=$sortie/guest.img,snapshot=on" \
    -serial "file:$sortie/serie.log" -d guest_errors -trace 'enable=virtio_gpu_cmd_*' >"$sortie/qemu.log" 2>&1 || {
    rc=$?
    if [[ $rc != 124 ]]; then
        cat "$sortie/qemu.log" >&2
        exit "$rc"
    fi
}
grep -aqF 'P10_VENUS_HOSTMEM_OK' "$sortie/serie.log"
grep -aqF 'P10_BLOB_INVITE_OK' "$sortie/serie.log"
grep -aqF 'VIRTGPU_VENUS_DRM_CAPS_OK' "$sortie/serie.log"
grep -aqF 'VIRTGPU_VENUS_DRM_CONTEXT_OK' "$sortie/serie.log"
grep -aoE 'VIRTGPU_VENUS_VERSION_RING3_OK appels=3 api=[0-9.]+' "$sortie/serie.log"
if [[ ${BO_VENUS_DEMAPPE:-0} == 1 ]]; then
    test "$(grep -acF 'VIRTGPU_VENUS_VKFILL_FENCE_OK octets=4096' "$sortie/serie.log")" -eq 2
    test "$(grep -acF 'VIRTGPU_VENUS_VKBUFFER_OK octets=4096' "$sortie/serie.log")" -eq 2
    test "$(grep -acF 'VIRTGPU_VENUS_APPAREILS compte=' "$sortie/serie.log")" -eq 2
    grep -aqF 'VIRTGPU_VENUS_DRM_INSTANCES_OK cycles=2' "$sortie/serie.log"
    grep -aqF 'VIRTGPU_VENUS_TAILLES_OK cycles=4' "$sortie/serie.log"
    grep -aqF 'VIRTGPU_VENUS_PROPRIETAIRE_OK' "$sortie/serie.log"
    grep -aqF 'VIRTGPU_VENUS_CTX_ORPHELIN_OK' "$sortie/serie.log"
    grep -aqF 'P10_VENUS_ORPHELIN_RECUPERE' "$sortie/serie.log"
    grep -aqF 'VIRTGPU_VENUS_DEMAPPE_OK' "$sortie/serie.log"
fi
if grep -aEq 'PANIC|panicked at|DOUBLE FAULT|TRIPLE FAULT' "$sortie/serie.log"; then
    exit 1
fi
