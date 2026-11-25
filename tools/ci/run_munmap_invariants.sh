#!/usr/bin/env bash
# BOUCHAUD_DEMONTAGE_PAR_TABLES_V1 -- INVARIANTS du demontage (munmap / brk) :
# pages gardees intactes, pages demontees qui fautent, remappage et
# re-croissance du tas a zero, aucun cadre perdu (tools/userland/munmap-invariants.c).
#
#   tools/ci/run_munmap_invariants.sh BOOTIMAGE [CYCLES] [SMP]
#
# Deux passes : `controle` (chaque page demontee lue par un enfant, qui doit
# fauter) puis `sans_fork` (memes demontages, aucun enfant) : le compte de
# memoire libre de la seconde passe est celui du demontage seul. Troisieme
# passe, `adresse_neuve` (#377) : sans enfant, reservation a une adresse neuve
# a chaque cycle -- les tables videes doivent etre rendues par le demontage.
#
# Code 0 si la sonde a conclu (MUNMAP_INV_OK) sans faute noyau.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_munmap_invariants.sh BOOTIMAGE [CYCLES] [SMP]}
CYCLES=${2:-10}
SMP=${3:-1}
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
musl-gcc -O2 -static-pie -fPIE -Wl,-z,noexecstack tools/userland/munmap-invariants.c -o "$T/munmap-invariants"
if readelf -l "$T/munmap-invariants" | grep 'Requesting program interpreter' >/dev/null; then
  echo 'munmap-invariants doit etre statique' >&2
  exit 1
fi
mkdir -p "$T/scenario/bin"
cp "$T/munmap-invariants" "$T/scenario/bin/munmap-invariants"
cat > "$T/scenario/autorun" <<AUTORUN
echo MUNMAP_INV_BEGIN && /bin/munmap-invariants $CYCLES ; /bin/munmap-invariants $CYCLES sans_fork ; /bin/munmap-invariants $CYCLES adresse_neuve ; echo MUNMAP_INV_FIN
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
sed 's/\x1b\[[0-9;]*[A-Za-z]//g' "$T/serie.log" \
  | grep -avE '^[[:space:]]*(\[[0-9:]{8}\]\[[^]]*\]\[FPS:[^]]*\][[:space:]]*)?\+ ' > "$T/emis.log" || true
grep -aoE 'MUNMAP_INV[A-Z_]*( .*)?$' "$T/emis.log" || true
[ "$code" -ne 124 ] || { echo 'QEMU : delai depasse' >&2; exit 1; }
if grep -aqE '\*\*\* KERNEL PANIC \*\*\*|DOUBLE FAULT|panicked at' "$T/serie.log"; then
  echo 'faute noyau pendant la sonde' >&2
  exit 1
fi
# Les lectures VOULUES d'une page demontee tuent un enfant de la sonde
# (FAULT_FATAL app=/bin/munmap-invariants) ; toute autre faute fatale echoue.
if grep -aE 'FAULT_FATAL' "$T/serie.log" | grep -avF 'app=/bin/munmap-invariants' | grep -a . >/dev/null; then
  echo 'faute fatale hors des lectures voulues de la sonde' >&2
  exit 1
fi
echo "MUNMAP_INV_FAUTES_NOYAU $(grep -acF 'FAULT_FATAL' "$T/serie.log")"
[ "$(grep -acF 'MUNMAP_INV_OK' "$T/emis.log")" -eq 3 ]
