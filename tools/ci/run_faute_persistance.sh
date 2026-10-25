#!/usr/bin/env bash
# C5 : 48 morts multi-fils, puis persistance concurrente et instantane dmesg.
# Tous les demarrages comptent, y compris les rouges. Aucun fichier reutilise.
# Usage : bash tools/ci/run_faute_persistance.sh BOOTIMAGE [REPETITIONS=6]
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?bootimage requise}
REPETITIONS=${2:-6}
SMP=${BO_SMP:-4}
[[ $REPETITIONS =~ ^[1-9][0-9]*$ ]] || exit 2
OUT=${FAUTE_PERSIST_SORTIE:-"$PWD/target/faute-persistance-$(date -u +%Y%m%dT%H%M%S)-$$"}
[[ ! -e "$OUT" ]] || { echo "SORTIE_EXISTANTE $OUT" >&2; exit 2; }
mkdir -p "$OUT/scenario/bin"
for probe in faute-multifil-probe persist-course-probe; do
  musl-gcc -O2 -Wall -fno-stack-protector -fno-asynchronous-unwind-tables \
    -idirafter /usr/include -idirafter /usr/include/x86_64-linux-gnu \
    -static-pie -pthread "tools/userland/$probe.c" -o "$OUT/scenario/bin/$probe"
done
cat > "$OUT/scenario/autorun" <<'AUTORUN'
/bin/faute-multifil-probe 6
/bin/persist-course-probe
echo DMESG_INSTANTANE_DEBUT
dmesg
echo DMESG_INSTANTANE_FIN
echo FAUTE_PERSISTANCE_FIN
AUTORUN
accel=(-cpu max)
nom=tcg
if [[ ${BO_QEMU_KVM:-0} == 1 ]]; then
  [[ -r /dev/kvm && -w /dev/kvm ]] || { echo KVM_INACCESSIBLE >&2; exit 2; }
  accel=(-enable-kvm -cpu host,kvm-steal-time=on)
  nom=kvm
fi
{ git rev-parse HEAD; sha256sum "$BOOT" "$OUT/scenario/bin/"*; qemu-system-x86_64 --version; } > "$OUT/identite.txt"
echecs=0
for ((tour=1; tour<=REPETITIONS; tour++)); do
  dossier="$OUT/tour-$tour"
  mkdir "$dossier"
  IMAGE="$dossier/scenario.img" bash tools/userland/mkdisk.sh "$OUT/scenario" >/dev/null
  log="$dossier/serie.log"
  : > "$log"
  qemu-system-x86_64 "${accel[@]}" -smp "$SMP" -m 2048 -display none -no-reboot \
    -drive "format=raw,file=$BOOT" -drive "format=raw,file=$dossier/scenario.img" -serial "file:$log" &
  pid=$!
  trap 'kill "$pid" 2>/dev/null || true' EXIT
  limite=$((SECONDS + 360))
  while kill -0 "$pid" 2>/dev/null && ((SECONDS < limite)); do
    if grep -aqE 'KERNEL PANIC|panicked at' "$log"; then break; fi
    if sed -E 's/\x1b\[[0-9;]*m//g' "$log" | grep -a 'FAUTE_PERSISTANCE_FIN' | grep -av '+ echo' >/dev/null; then break; fi
    sleep 1
  done
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  trap - EXIT
  if python3 - "$log" > "$dossier/verdict.txt" <<'PY'
import re, sys
s = re.sub(r'\x1b\[[0-9;]*m', '', open(sys.argv[1], errors='replace').read())
s = '\n'.join(line for line in s.splitlines() if '+ echo ' not in line)
required = ('FAUTE_MULTIFIL_OK essais=48', 'PERSIST_COURSE_OK', 'DMESG_INSTANTANE_DEBUT', 'DMESG_INSTANTANE_FIN', 'FAUTE_PERSISTANCE_FIN')
missing = [m for m in required if m not in s]
fatal = bool(re.search(r'KERNEL PANIC|panicked at|FAUTE_MULTIFIL_ECHEC|PERSIST_COURSE_ECHEC|COMPTA_DEPASSE', s))
after = s.split('FAUTE_MULTIFIL_OK essais=48', 1)[-1]
unexpected = bool(re.search(r'\[kernel\] PROCESS_FAULT pid=', after))
view = after.split('DMESG_INSTANTANE_DEBUT', 1)[-1].split('DMESG_INSTANTANE_FIN', 1)[0]
indices = [int(i) for i in re.findall(r'\[(\d{2})\] ', view)]
ok = not missing and not fatal and not unexpected and indices == list(range(64))
print(f'manque={missing} fatal={int(fatal)} faute_inattendue={int(unexpected)} instantane={len(indices)}/64')
print('FAUTE_PERSISTANCE_OK' if ok else 'FAUTE_PERSISTANCE_ECHEC')
sys.exit(0 if ok else 1)
PY
  then echo "FAUTE_PERSISTANCE_TOUR_OK tour=$tour accel=$nom smp=$SMP";
  else echecs=$((echecs + 1)); echo "FAUTE_PERSISTANCE_TOUR_ECHEC tour=$tour accel=$nom smp=$SMP"; fi
  cat "$dossier/verdict.txt"
done
echo "FAUTE_PERSISTANCE_BILAN repetitions=$REPETITIONS echecs=$echecs accel=$nom smp=$SMP sortie=$OUT"
test "$echecs" -eq 0
