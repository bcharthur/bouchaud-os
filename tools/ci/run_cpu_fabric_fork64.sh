#!/usr/bin/env bash
# Mesure comparative du retour de fork() pour 64 Mio de pages privées résidentes.
# run_cpu_fabric_fork64.sh BOOT_A BOOT_B NOUVEAU_REPERTOIRE [SMP]
# BO_QEMU_KVM=1 impose KVM et vérifie l'accélérateur dans chaque journal.
set -euo pipefail
a=$(realpath -e "${1:?image A}")
b=$(realpath -e "${2:?image B}")
out=$(realpath -m "${3:?répertoire de sortie}")
smp=${4:-1}
case "$smp" in 1|2|4) ;; *) echo 'SMP doit valoir 1, 2 ou 4' >&2; exit 2 ;; esac
mkdir -- "$out"
cd "$(dirname "$0")/../.."

accel=(-accel tcg -cpu max)
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
    test -r /dev/kvm && test -w /dev/kvm || { echo 'KVM indisponible' >&2; exit 1; }
    accel=(-accel kvm -cpu host)
fi
mkdir -p "$out/scenario/bin"
gcc -O2 -static-pie tools/userland/cpu-fabric-fork64.c -o "$out/scenario/bin/fork64"
printf '/bin/fork64\n' > "$out/scenario/autorun"
(cd tools/userland && IMAGE="$out/sonde.img" ./mkdisk.sh "$out/scenario")
sha256sum "$a" "$b" "$out/scenario/bin/fork64" "$out/sonde.img" > "$out/sha256.txt"
qemu-system-x86_64 --version > "$out/qemu-version.txt"
printf 'smp=%s\nqemu=%s\n' "$smp" "${accel[*]}" > "$out/configuration.txt"

n=0
for arm in A B B A; do
    n=$((n+1))
    boot=$a
    if [ "$arm" = B ]; then boot=$b; fi
    prefix="$out/$n-$arm"
    code=0
    timeout 90 qemu-system-x86_64 "${accel[@]}" -m 1024 -smp "$smp" \
        -display none -no-reboot -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
        -serial "file:$prefix.log" -drive "file=$boot,format=raw,snapshot=on" \
        -drive "file=$out/sonde.img,format=raw,snapshot=on" >"$prefix.qemu.log" 2>&1 || code=$?
    printf '%s\n' "$code" >"$prefix.qemu.status"
    [ "$code" -ne 124 ]
    if grep -aqE '\*\*\* KERNEL PANIC \*\*\*|DOUBLE FAULT|panicked at' "$prefix.log"; then
        exit 1
    fi
    if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
        grep -aqE 'BOUCHAUD_CPU_ISA hyperviseur=KVMKVMKVM([[:space:]]|$)' "$prefix.log"
    fi
done
python3 - "$out" <<'PY'
import pathlib
import re
import statistics
import sys

root = pathlib.Path(sys.argv[1])
for index, arm in enumerate("ABBA", 1):
    with (root / f"{index}-{arm}.log").open(errors="replace") as stream:
        serial = stream.read()
    items = re.findall(r"FORK64_SAMPLE i=(\d+) ns=(\d+)", serial)
    if [int(i) for i, _ in items] != list(range(20)) or serial.count("FORK64_OK") != 1:
        sys.exit(f"Passage {index}-{arm} incomplet")
    samples = [int(ns) / 1e6 for _, ns in items]
    print(f"{index}-{arm} n={len(samples)} mediane_ms={statistics.median(samples):.3f} "
          f"min_ms={min(samples):.3f} max_ms={max(samples):.3f}")
PY
