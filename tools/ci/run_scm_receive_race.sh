#!/usr/bin/env bash
# A single in-flight SCM_RIGHTS message races a nonblocking receiver.
# This is a kernel ABI regression test, independent of Ladybird's rebuild.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=$(realpath "${1:?usage: run_scm_receive_race.sh BOOTIMAGE}")
LOG="$PWD/serie-scm-receive.log"
WORK=$(mktemp -d)
PID=""
cleanup() {
    if [ -n "$PID" ]; then
        kill "$PID" 2>/dev/null || true
        wait "$PID" 2>/dev/null || true
    fi
    rm -rf "$WORK"
}
trap cleanup EXIT
mkdir -p "$WORK/scenario/bin"
musl-gcc -std=c11 -O2 -Wall -Wextra -Werror -pthread -static-pie \
    tools/userland/scm-receive-race-probe.c -o "$WORK/scenario/bin/scm-receive-race-probe"
"$WORK/scenario/bin/scm-receive-race-probe"
printf '/bin/scm-receive-race-probe\necho SCM_RECEIVE_RUN_END\n' > "$WORK/scenario/autorun"
(cd tools/userland && IMAGE="$WORK/disk.img" ./mkdisk.sh "$WORK/scenario")
[ -w /dev/kvm ] || { echo 'KVM required for the concurrent receive proof' >&2; exit 1; }
: > "$LOG"
qemu-system-x86_64 -enable-kvm -cpu host -m 4096 -smp 4 \
    -drive format=raw,file="$BOOT" -drive format=raw,file="$WORK/disk.img" \
    -display none -no-reboot -netdev user,id=net0 -device e1000,netdev=net0 \
    -serial file:"$LOG" > "$WORK/qemu.log" 2>&1 &
PID=$!
for ((i=0; i<90; i++)); do
    if grep -aq 'SCM_RECEIVE_RUN_END' "$LOG" || ! kill -0 "$PID" 2>/dev/null; then break; fi
    sleep 1
done
kill "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
PID=""
cat "$WORK/qemu.log"
tail -c 15000 "$LOG"
python3 - "$LOG" <<'PY'
import re, sys
s = re.sub(r'\x1b\[[0-9;]*m', '', open(sys.argv[1], errors='replace').read())
assert 'SCM_RECEIVE_RUN_END' in s, 'probe did not finish'
assert re.search(r'SCM_RECEIVE_RACE_OK rounds=10000 expected=10000 END\r?$', s, re.M), 'missing complete success record'
assert 'SCM_RECEIVE_RACE_FAIL' not in s, 'receive atomicity violated'
assert 'KERNEL PANIC' not in s and 'PROCESS_FAULT' not in s, 'kernel or process fault'
print('SCM_RECEIVE_KVM_OK')
PY
