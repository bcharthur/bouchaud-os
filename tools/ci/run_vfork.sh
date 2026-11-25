#!/usr/bin/env bash
# BOUCHAUD_VFORK_REEL_V1 -- vfork / posix_spawn sans recopie, dans le vrai
# noyau, avec la glibc (tools/userland/vfork-probe.c).
#
#   tools/ci/run_vfork.sh BOOTIMAGE [N] [SMP] [ETAPE]
#
# ETAPE=tid : l'epreuve CLONE_CHILD_SETTID/getppid seule (sans exiger
# PERF_VFORK) -- sert de temoin sur une image d'avant le correctif.
#
# Code 0 si VFORK_PROBE_OK, sans faute noyau, et si au moins un PERF_VFORK
# a ete publie (le chemin sans copie a reellement servi).
#
# Banc court (quelques minutes en TCG) : ce n'est pas une campagne.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_vfork.sh BOOTIMAGE [N] [SMP]}
N=${2:-20}
SMP=${3:-2}
ETAPE=${4:-}
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
gcc -O2 -static-pie -fPIE -pthread -Wl,-z,noexecstack tools/userland/vfork-probe.c -o "$T/vfork-probe"
mkdir -p "$T/scenario/bin"
cp "$T/vfork-probe" "$T/scenario/bin/vfork-probe"
cat > "$T/scenario/autorun" <<AUTORUN
echo VFORK_BEGIN ; /bin/vfork-probe $N $ETAPE ; echo VFORK_FIN
AUTORUN
(cd tools/userland && IMAGE="$T/probe.img" ./mkdisk.sh "$T/scenario")
set +e
timeout 900 qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$T/probe.img" \
  -m 2048 -smp "$SMP" -accel tcg -cpu max -display none -no-reboot \
  -netdev user,id=net0 -device e1000,netdev=net0 \
  -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
  -serial file:"$T/serie.log"
code=$?
set -e
sed 's/\x1b\[[0-9;]*[A-Za-z]//g' "$T/serie.log" > "$T/emis.log"
[ -n "${VFORK_JOURNAL:-}" ] && cp "$T/emis.log" "$VFORK_JOURNAL"
grep -aoE 'VFORK_PROBE[A-Z_]*( .*)?$' "$T/emis.log" || true
grep -aoE 'PERF_VFORK .*' "$T/emis.log" | tail -3 || true
[ "$code" -ne 124 ] || { echo 'QEMU : delai depasse' >&2; exit 1; }
if grep -aqE '\*\*\* KERNEL PANIC \*\*\*|DOUBLE FAULT|panicked at' "$T/serie.log"; then
  echo 'faute noyau pendant la sonde' >&2
  exit 1
fi
grep -aqF 'VFORK_PROBE_OK' "$T/emis.log" || exit 1
[ -n "$ETAPE" ] || grep -aqF 'PERF_VFORK' "$T/emis.log" || { echo 'aucun PERF_VFORK : le chemin vfork na pas servi' >&2; exit 1; }
