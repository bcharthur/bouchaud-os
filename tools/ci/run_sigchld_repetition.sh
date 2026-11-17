#!/usr/bin/env bash
# Repeat ONLY tools/userland/sigchld-multifil-probe in one VM
# (BOUCHAUD_SIGCHLD_REPETITION_V1).
#
#   tools/ci/run_sigchld_repetition.sh BOOTIMAGE [REPETITIONS=30] [SMP=4]
#
# Why: run 41648148378 (main@992e67e1, p13-primitives-noyau, TCG SMP4) died
# on SIGCHLD_MULTIFIL_ECHEC n=1 inside a 45-minute campaign. This bench
# isolates the probe: same binary, same build flags, same timings (no delay
# of the tested behaviour is changed), N runs in a row, and a per-case
# failure count. Fail-closed: any ECHEC, a missing verdict, or a kernel
# fatal is red.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_sigchld_repetition.sh BOOTIMAGE [REPETITIONS] [SMP]}
N=${2:-30}
SMP=${3:-4}
[[ $N =~ ^[1-9][0-9]*$ ]] || exit 2
OUT=${SIGCHLD_SORTIE:-target/sigchld-repetition}
rm -rf "$OUT"; mkdir -p "$OUT/scenario/bin"
UAPI=(-idirafter /usr/include -idirafter /usr/include/x86_64-linux-gnu)
musl-gcc -O2 -Wall -fno-stack-protector -fno-asynchronous-unwind-tables "${UAPI[@]}" \
  -static-pie -pthread tools/userland/sigchld-multifil-probe.c -o "$OUT/scenario/bin/sigchld-multifil-probe"
{
  # SIGCHLD_STRACE=1: same as run_os_primitives.sh, which starts its autorun
  # with `strace echecs` (every failing syscall logged): the red run
  # 41648148378 ran the probe under that extra kernel logging.
  [ "${SIGCHLD_STRACE:-0}" = 1 ] && echo "strace echecs"
  for ((i = 1; i <= N; i++)); do
    echo "echo SIGCHLD_REPETITION_DEBUT i=$i"
    echo "/bin/sigchld-multifil-probe"
  done
  echo "echo SIGCHLD_REPETITION_FIN"
} > "$OUT/scenario/autorun"
IMAGE="$PWD/$OUT/disque.img" bash tools/userland/mkdisk.sh "$PWD/$OUT/scenario" >/dev/null
ACCEL=(-cpu max); nom=tcg
if [[ ${BO_QEMU_KVM:-0} == 1 ]]; then ACCEL=(-enable-kvm -cpu host); nom=kvm; fi
LOG="$OUT/serie.log"; : > "$LOG"
qemu-system-x86_64 "${ACCEL[@]}" -drive format=raw,file="$BOOT" -drive format=raw,file="$OUT/disque.img" \
  -m 4096 -smp "$SMP" -display none -no-reboot -serial file:"$LOG" >/dev/null 2>&1 &
pid=$!
fin=$((SECONDS + 60 + N * 30))
while kill -0 "$pid" 2>/dev/null && (( SECONDS < fin )); do
  if sed 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -a 'SIGCHLD_REPETITION_FIN' | grep -av 'echo ' >/dev/null; then break; fi
  if grep -aqE 'KERNEL PANIC|panicked at|DOUBLE FAULT' "$LOG"; then break; fi
  sleep 1
done
kill -KILL "$pid" 2>/dev/null || true
wait "$pid" 2>/dev/null || true
python3 - "$LOG" "$N" "$nom" "$SMP" <<'PY'
import re, sys, collections
log, n, nom, smp = sys.argv[1], int(sys.argv[2]), sys.argv[3], sys.argv[4]
t = re.sub(r"\x1b\[[0-9;]*m", "", open(log, errors="replace").read())
t = "\n".join(l for l in t.splitlines() if "+ echo" not in l and "echo SIGCHLD_REPETITION" not in l)
cas = re.findall(r"SIGCHLD_CAS cas=(\w+) (.*?) (ok|ECHEC)\s*$", t, re.M)
# Serial lines can interleave with kernel lines: a case line may be unreadable.
# The per-run verdict (SIGCHLD_MULTIFIL_OK / _ECHEC) is what counts.
par = collections.Counter(c for c, _, v in cas)
ech = collections.Counter(c for c, _, v in cas if v == "ECHEC")
ok_runs = len(re.findall(r"^.*SIGCHLD_MULTIFIL_OK\s*$", t, re.M))
ko_runs = len(re.findall(r"SIGCHLD_MULTIFIL_ECHEC n=\d+", t))
fatal = bool(re.search(r"KERNEL PANIC|panicked at|DOUBLE FAULT", t))
for c in ["mono", "principal", "secondaire", "attente", "boucle", "abi_glibc", "exit_group_sommeil", "sortie_normale", "sommeil"]:
    print(f"SIGCHLD_REPETITION_CAS cas={c} executions={par[c]} echecs={ech[c]}")
for c, d, v in cas:
    if v == "ECHEC":
        print(f"  ECHEC cas={c} {d}")
for l in re.findall(r"SIGCHLD_ATTENTE .*", t)[:0]:
    print(l)
print(f"SIGCHLD_REPETITION_BILAN repetitions={n} ok={ok_runs} echec={ko_runs} fatal={int(fatal)} accel={nom} smp={smp}")
bon = ok_runs == n and ko_runs == 0 and not fatal
print("SIGCHLD_REPETITION_OK" if bon else "SIGCHLD_REPETITION_ECHEC")
sys.exit(0 if bon else 1)
PY
