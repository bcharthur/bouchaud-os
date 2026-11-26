#!/usr/bin/env bash
# BOUCHAUD_COUT_APPELS_V1 / BOUCHAUD_VDSO_V1 -- cout des appels frequents vu
# de l'anneau 3, appels arrives au noyau, monotonie (tools/userland/appels-probe.c).
#
#   tools/ci/run_appels.sh BOOTIMAGE [SMP] [ITERATIONS]
#
# La sonde tourne deux fois : liee a la glibc (statique) et a musl (statique)
# -- les deux libc consultent le vDSO par AT_SYSINFO_EHDR.
# Code 0 si APPELS_PROBE_OK deux fois, sans faute noyau.
# BO_QEMU_KVM=1 exige KVM ; BO_APPELS_REQUIRE_VDSO=1 exige la projection.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_appels.sh BOOTIMAGE [SMP] [ITERATIONS]}
SMP=${2:-2}
N=${3:-200000}
T=$(mktemp -d)
trap '[ -n "${GARDE_JOURNAL:-}" ] && cp "$T/serie.log" "$GARDE_JOURNAL"; rm -rf "$T"' EXIT
gcc -O2 -static-pie -fPIE -pthread tools/userland/appels-probe.c -o "$T/appels-glibc"
musl-gcc -O2 -static-pie -fPIE tools/userland/appels-probe.c -o "$T/appels-musl" -lpthread
mkdir -p "$T/scenario/bin"
cp "$T/appels-glibc" "$T/appels-musl" "$T/scenario/bin/"
cp "$BOOT" "$T/boot.bin"
if [ "$SMP" -gt 1 ]; then
  gcc -O2 -static-pie tools/userland/cpu-fabric-horloge-coeurs.c -o "$T/scenario/bin/horloge-coeurs"
fi
cat > "$T/scenario/autorun" <<SCEN
echo APPELS_PROBE_BANC libc=glibc ; /bin/appels-glibc $N
echo APPELS_PROBE_BANC libc=musl ; /bin/appels-musl $N
SCEN
if [ "$SMP" -gt 1 ]; then
  printf '%s\n' '/bin/horloge-coeurs' >> "$T/scenario/autorun"
fi
printf '%s\n' 'echo APPELS_PROBE_FIN' >> "$T/scenario/autorun"
(cd tools/userland && IMAGE="$T/probe.img" ./mkdisk.sh "$T/scenario" >/dev/null)
accel=(-accel tcg -cpu max)
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  test -r /dev/kvm && test -w /dev/kvm || { echo 'KVM indisponible' >&2; exit 1; }
  accel=(-accel kvm -cpu host)
fi
if [ "${BO_APPELS_REPLI_GETCPU:-0}" = 1 ]; then
  accel[3]+=',-rdpid,-rdtscp'
fi
set +e
timeout 900 qemu-system-x86_64 \
  -drive format=raw,file="$T/boot.bin" \
  -drive format=raw,file="$T/probe.img" \
  -m 1024 -smp "$SMP" "${accel[@]}" -display none -no-reboot \
  -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
  -serial file:"$T/serie.log"
code=$?
set -e
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  grep -aqE 'BOUCHAUD_CPU_ISA hyperviseur=KVMKVMKVM([[:space:]]|$)' "$T/serie.log"
fi
sed 's/\x1b\[[0-9;]*[A-Za-z]//g' "$T/serie.log" > "$T/emis.log"
grep -aoE 'APPELS_PROBE_[A-Z_]+( .*)?$|PROCESS_FAULT .*' "$T/emis.log" | grep -av 'APPELS_PROBE_BANC libc=[a-z]* ;' || true
[ "$code" -ne 124 ] || { echo 'QEMU : delai depasse' >&2; exit 1; }
if grep -aqE '\*\*\* KERNEL PANIC \*\*\*|DOUBLE FAULT|panicked at' "$T/serie.log"; then
  echo 'faute noyau pendant la sonde' >&2
  exit 1
fi
if [ "${BO_APPELS_REQUIRE_VDSO:-0}" = 1 ]; then
  [ "$(grep -acE 'APPELS_PROBE_VDSO base=0x[1-9a-f][0-9a-f]*' "$T/emis.log")" -eq 2 ]
  [ "$(grep -acF 'APPELS_PROBE_ETAPE protection=ok' "$T/emis.log")" -eq 2 ]
fi
if grep -aq 'APPELS_PROBE_FAIL' "$T/emis.log"; then
  echo 'Une sonde appels a echoue' >&2
  exit 1
fi
[ "$(grep -acE '(^|[^A-Z_])APPELS_PROBE_OK' "$T/emis.log")" -eq 2 ]
if [ "$SMP" -gt 1 ]; then
  grep -aoE 'HORLOGE_COEURS distincts=[1-9][0-9]* reculs=0 .*' "$T/emis.log"
  [ "$(grep -acF HORLOGE_COEURS_OK "$T/emis.log")" -eq 1 ]
fi
if [ "${BO_APPELS_REPLI_GETCPU:-0}" = 1 ]; then
  grep -aqE 'BOUCHAUD_VDSO publication=.* rdtscp=0 getcpu=0 ' "$T/emis.log"
  [ "$(grep -acE "APPELS_PROBE_COUT appel=sched_getcpu ns=[0-9]+ noyau=$N n=$N" "$T/emis.log")" -eq 2 ]
fi
