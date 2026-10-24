#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_contexte_execution.sh BOOTIMAGE}
OUT=target/contexte-execution
mkdir -p "$OUT/files"
printf 'contexte-banc\nsmpstat\necho CONTEXTE_EXECUTION_FIN\n' > "$OUT/files/autorun"
IMAGE="$PWD/$OUT/scenario.img" bash tools/userland/mkdisk.sh "$PWD/$OUT/files" >/dev/null
for smp in 1 4; do
    log="$OUT/smp$smp.log"
    set +e
    timeout 90 qemu-system-x86_64 -cpu max -m 2048 -smp "$smp" \
        -drive "format=raw,file=$BOOT" -drive "format=raw,file=$OUT/scenario.img" \
        -display none -no-reboot -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
        -serial "file:$log"
    rc=$?
    set -e
    test "$rc" -eq 33
    grep -aq 'CONTEXTE_BSP_OK sommeil=1 waitqueue=1 retour_sans_tache=1' "$log"
    grep -aq 'CONTEXTE_LIFECYCLE_OK service_hors_session_preserve=1' "$log"
    grep -aq 'CONTEXTE_IRQ_OK profondeur=0 entrees=' "$log"
    grep -aq '=== AUTORUN FIN === statut=0' "$log"
    ! grep -aqE 'KERNEL PANIC|panicked at|irq_depth underflow' "$log"
    echo "CONTEXTE_EXECUTION_OK smp=$smp"
done
