#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."

BOOT=${1:?usage: run_security_runtime.sh BOOTIMAGE}
OUT=target/security-runtime-ci
LOG="$OUT/security-ring3.log"
IMAGE="$OUT/security-probe.img"
PROBE="$OUT/security-ring3-probe"

rm -rf "$OUT"
mkdir -p "$OUT"

(
  cd tools/security
  OUT="$PWD/../../$OUT" bash ./build-security-ring3-probe.sh
)

python3 tools/security/make-security-probe-image.py \
  --probe "$PROBE" \
  --image "$IMAGE"

: > "$LOG"
qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$IMAGE" \
  -m 4096 -smp 4 -cpu max \
  -display none -no-reboot \
  -netdev user,id=net0 -device e1000,netdev=net0 \
  -audiodev none,id=muet -device AC97,audiodev=muet \
  -serial file:"$LOG" &
PID=$!

cleanup() {
  kill -TERM "$PID" 2>/dev/null || true
  sleep 0.3
  kill -KILL "$PID" 2>/dev/null || true
  wait "$PID" 2>/dev/null || true
}
trap cleanup EXIT

deadline=$((SECONDS + 120))
while :; do
  if grep -aEq '\*\*\* KERNEL PANIC \*\*\*|DOUBLE FAULT|TRIPLE FAULT|SpinLock recursive acquisition|BKL(-FR)?.*VIOLATION' "$LOG"; then
    tail -n 200 "$LOG" >&2
    exit 1
  fi
  if grep -aFq '[SECURITY-RING3] OK' "$LOG"; then
    break
  fi
  if ! kill -0 "$PID" 2>/dev/null; then
    sleep 0.15
    grep -aFq '[SECURITY-RING3] OK' "$LOG" && break
    tail -n 200 "$LOG" >&2
    exit 1
  fi
  if (( SECONDS >= deadline )); then
    tail -n 200 "$LOG" >&2
    exit 1
  fi
  sleep 0.2
done

cleanup
trap - EXIT

python3 tools/ci/reliability/logscan.py "$LOG"

for marker in \
  '[SECURITY-RING3] WX_MMAP_DENIED' \
  '[SECURITY-RING3] WX_MPROTECT_DENIED' \
  '[SECURITY-RING3] SETUID_DROP_OK' \
  '[SECURITY-RING3] PRIV_ESC_DENIED' \
  '[SECURITY-RING3] DEVICE_DENIED' \
  '[SECURITY-RING3] PATH_CANONICAL_OK' \
  '[SECURITY-RING3] DIRFD_CANONICAL_OK' \
  '[SECURITY-RING3] DIRFD_MUTATION_OK' \
  '[SECURITY-RING3] STICKY_TMP_OK' \
  '[SECURITY-RING3] MMAP_DAC_OK' \
  '[SECURITY-RING3] RAW_SOCKET_DENIED' \
  '[SECURITY-RING3] SIGNAL_DENIED' \
  '[SECURITY-RING3] THREAD_SIGNAL_DENIED' \
  '[SECURITY-RING3] NNP_OK' \
  '[SECURITY-RING3] NATIVE_SHM_LIMIT_OK' \
  '[SECURITY-RING3] JIT_DENIED' \
  '[SECURITY-RING3] OK' \
  '[SECURITY-DENY]'
do
  grep -aFq "$marker" "$LOG" || {
    echo "marqueur security absent: $marker" >&2
    exit 1
  }
done

grep -aE '\[SECURITY-RING3\]|\[SECURITY-DENY\]' "$LOG" | tail -n 120
echo SECURITY_RUNTIME_OK
