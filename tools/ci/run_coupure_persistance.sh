#!/usr/bin/env bash
# Power cuts during atomic user-file writes (BOUCHAUD_PERSIST_COUPURE_V1).
#
#   tools/ci/run_coupure_persistance.sh BOOTIMAGE [CYCLES=10]
#
# One userland disk, kept across every boot. Each cycle boots Bouchaud, lets
# tools/userland/persist-coupure-probe run its write -> fsync -> rename ->
# fsync(dir) loop, waits for a random number (1..6) of ACKNOWLEDGED versions
# plus a random 0..900 ms, then KILLS QEMU (SIGKILL: a power cut, never a
# shutdown -- no sync, no unmount). The next boot reads the file back.
#
# Invariants (fail closed, every cycle):
#   I1  the file is always coherent: one whole version, old or new, never a
#       mix, never torn (COUPURE_RELU coherent=1);
#   I2  never older than the last ACKNOWLEDGED version: an fsync that
#       returned is durable;
#   I3  never newer than last acknowledged + 1 (nothing invented).
#
# The ORACLE is the serial line: a version counts as acknowledged when its
# COUPURE_ACQUITTE line reached the port. That holds only with a SYNCHRONOUS
# console. With BOUCHAUD_CONSOLE_ASYNCHRONE_V1 a user `write` returns once the
# line is in the kernel ring, before the UART: two durable versions could
# still have their acknowledgements in the ring at the cut, and I3 would
# report an "invention" the disk never made (issue #288, run 42931411983).
# The scenario therefore runs `console-async off` first, and every cycle
# REQUIRES `BOUCHAUD_CONSOLE_BILAN active=0` before the read-back; without it
# the cycle fails (ASYNC) -- the invariants are unchanged, never relaxed.
#
# What this does NOT model: a disk that loses its own volatile write cache.
# QEMU writes the image with the host page cache, so a host-side kill keeps
# every write QEMU issued. That part is covered on the host by
# tools/update/test_durabilite.rs (reordering, torn sectors). This harness
# proves the guest side: the kernel's cache, fsync, rename and directory
# fsync under abrupt death.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_coupure_persistance.sh BOOTIMAGE [CYCLES]}
CYCLES=${2:-10}
[[ $CYCLES =~ ^[1-9][0-9]*$ ]] || exit 2
OUT=${COUPURE_SORTIE:-target/coupure-persistance}
SMP=${BO_SMP:-2}
rm -rf "$OUT"; mkdir -p "$OUT/scenario/bin"
musl-gcc -O2 -Wall -fno-stack-protector -fno-asynchronous-unwind-tables \
  -idirafter /usr/include -idirafter /usr/include/x86_64-linux-gnu \
  -static-pie tools/userland/persist-coupure-probe.c -o "$OUT/scenario/bin/persist-coupure-probe"
printf 'console-async off\n/bin/persist-coupure-probe\n' > "$OUT/scenario/autorun"
IMAGE="$PWD/$OUT/disque.img" bash tools/userland/mkdisk.sh "$PWD/$OUT/scenario" >/dev/null
accel=(-cpu max); nom=tcg
if [[ ${BO_QEMU_KVM:-0} == 1 ]]; then accel=(-enable-kvm -cpu host); nom=kvm; fi

acquitte=0          # last version acknowledged before the previous cut
incoherences=0; regressions=0; inventions=0; muets=0; asynchrones=0
for ((c = 1; c <= CYCLES; c++)); do
  log="$OUT/boot-$c.log"; : > "$log"
  qemu-system-x86_64 "${accel[@]}" -smp "$SMP" -m 2048 -display none -no-reboot \
    -drive "format=raw,file=$BOOT" -drive "format=raw,file=$OUT/disque.img" -serial "file:$log" >/dev/null 2>&1 &
  pid=$!
  propre() { sed -E 's/\x1b\[[0-9;]*m//g' "$log" | tr -d '\r' | grep -av '+ ' || true; }
  # Same, minus a last line the cut left unterminated.
  complet() { if [ -n "$(tail -c 1 "$log")" ]; then propre | sed '$d'; else propre; fi; }
  fin=$((SECONDS + 240)); relu=""
  while (( SECONDS < fin )) && kill -0 "$pid" 2>/dev/null; do
    relu=$(propre | grep -aoE 'COUPURE_RELU version=[0-9]+ coherent=[01] raison=[a-z_]+' | head -n 1 || true)
    [ -n "$relu" ] && break
    sleep 0.5
  done
  attendre=$(( (RANDOM % 6) + 1 ))
  while (( SECONDS < fin )) && kill -0 "$pid" 2>/dev/null; do
    n=$(propre | awk '/COUPURE_ACQUITTE/ { n++ } END { print n + 0 }')
    (( n >= attendre )) && break
    if propre | grep -aE 'COUPURE_ECHEC|KERNEL PANIC|panicked at' >/dev/null; then break; fi
    sleep 0.2
  done
  sleep "0.$(( RANDOM % 900 ))"
  kill -KILL "$pid" 2>/dev/null || true      # POWER CUT
  wait "$pid" 2>/dev/null || true

  if [ -z "$relu" ]; then muets=$((muets + 1)); echo "COUPURE_CYCLE c=$c verdict=MUET"; continue; fi
  v=$(sed -E 's/.*version=([0-9]+).*/\1/' <<< "$relu")
  coh=$(sed -E 's/.*coherent=([01]).*/\1/' <<< "$relu")
  verdict=ok
  # Oracle valid only with a synchronous console (#288): the switch must be
  # on record BEFORE the read-back line.
  # awk lit TOUT le journal (aucun lecteur tronquant sous pipefail).
  if ! propre | awk '/COUPURE_RELU/ { relu = 1 } !relu && /BOUCHAUD_CONSOLE_BILAN active=0/ { sync = 1 } END { exit !sync }'; then
    asynchrones=$((asynchrones + 1)); verdict=ASYNC
  fi
  if [ "$coh" != 1 ]; then incoherences=$((incoherences + 1)); verdict=INCOHERENT; fi
  if (( v < acquitte )); then regressions=$((regressions + 1)); verdict=REGRESSION; fi
  if (( v > acquitte + 1 )); then inventions=$((inventions + 1)); verdict=INVENTION; fi
  # Only COMPLETE lines count: the cut can land mid-print, and a truncated
  # "COUPURE_ACQUITTE version=4" (of 499) must not lower what is required.
  max=$(complet | awk -F'version=' '/COUPURE_ACQUITTE version=[0-9]+$/ { if ($2 + 0 > v) v = $2 + 0 } END { print v + 0 }')
  echec=$(propre | grep -aoE 'COUPURE_ECHEC[^\n]*|KERNEL PANIC' | head -n 1 || true)
  echo "COUPURE_CYCLE c=$c relu=$v coherent=$coh attendu_min=$acquitte acquittes_ce_boot=$((max > v ? max - v : 0)) verdict=$verdict ${echec:+echec=\"$echec\"}"
  [ -n "$echec" ] && { incoherences=$((incoherences + 1)); }
  (( max > v )) && acquitte=$max || acquitte=$v
done
echo "COUPURE_BILAN cycles=$CYCLES incoherences=$incoherences regressions=$regressions inventions=$inventions muets=$muets asynchrones=$asynchrones accel=$nom smp=$SMP"
if (( incoherences + regressions + inventions + muets + asynchrones == 0 )); then echo COUPURE_PERSISTANCE_OK; else echo COUPURE_PERSISTANCE_ECHEC; exit 1; fi
