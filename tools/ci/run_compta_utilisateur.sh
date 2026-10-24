#!/usr/bin/env bash
# C6 : frontières utilisateur et stress I1/I2, sans injection C6a.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_compta_utilisateur.sh BOOTIMAGE [SMP]}
SMP=${2:-4}
OUT=${COMPTA_SORTIE:-"$PWD/target/compta-utilisateur-$(date -u +%Y%m%dT%H%M%S)-smp$SMP"}
[[ ! -e "$OUT" ]] || { echo "SORTIE_EXISTANTE $OUT" >&2; exit 2; }
mkdir -p "$OUT/scenario/bin"
musl-gcc -O2 -Wall -fno-stack-protector -fno-asynchronous-unwind-tables \
  -static-pie -pthread tools/userland/compta-stress-probe.c -o "$OUT/scenario/bin/compta-stress-probe"
printf '/bin/compta-stress-probe frontieres\n/bin/compta-stress-probe 8\necho COMPTA_UTILISATEUR_FIN\n' > "$OUT/scenario/autorun"
IMAGE="$OUT/scenario.img" tools/userland/mkdisk.sh "$OUT/scenario" >/dev/null
accel=(-cpu max)
if [[ ${BO_QEMU_KVM:-0} == 1 ]]; then
  [[ -r /dev/kvm && -w /dev/kvm ]] || exit 2
  accel=(-enable-kvm -cpu host,kvm-steal-time=on)
fi
: > "$OUT/serie.log"
qemu-system-x86_64 "${accel[@]}" -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$OUT/scenario.img" -m 2048 -smp "$SMP" \
  -display none -no-reboot -serial file:"$OUT/serie.log" &
pid=$!
trap 'kill "$pid" 2>/dev/null || true' EXIT
limite=$((SECONDS + 180))
while kill -0 "$pid" 2>/dev/null && ((SECONDS < limite)); do
  if grep -aqE 'KERNEL PANIC|panicked at|COMPTA_STRESS_(OK|ECHEC)' "$OUT/serie.log"; then break; fi
  sleep 1
done
kill -KILL "$pid" 2>/dev/null || true
wait "$pid" 2>/dev/null || true
trap - EXIT
python3 - "$OUT/serie.log" <<'PY'
import re, sys
s = re.sub(r'\x1b\[[0-9;]*m', '', open(sys.argv[1], errors='replace').read())
for line in s.splitlines():
    if re.search(r'COMPTA_(FRONTIERES|STRESS|DEPASSE|RECUL)|PROCESS_FAULT_DETAIL|panicked at', line): print(line)
ok = all(x in s for x in ('COMPTA_FRONTIERES_OK', 'COMPTA_STRESS_OK'))
ok = ok and not re.search(r'KERNEL PANIC|panicked at|COMPTA_\w*ECHEC|COMPTA_DEPASSE|COMPTA_RECUL', s)
print('COMPTA_UTILISATEUR_' + ('OK' if ok else 'ECHEC'))
sys.exit(0 if ok else 1)
PY
