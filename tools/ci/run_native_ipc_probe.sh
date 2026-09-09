#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."

BOOT=${1:?usage: run_native_ipc_probe.sh BOOTIMAGE}
OUT=${OUT:-target/native-ipc-runtime-ci}
LOG="$OUT/native-ipc-runtime.log"
IMAGE="$OUT/native-ipc-probe.img"
RING3="$OUT/native-ipc-ring3-probe"
LIBC="$OUT/native-ipc-probe"
COMPOSITED="$OUT/composited-slice"

rm -rf "$OUT"
mkdir -p "$OUT"

echo "=== build freestanding native ring3 probe ==="
(
  cd tools/userland
  OUT="$PWD/../../$OUT" bash ./build-native-ipc-ring3-probe.sh
)

echo "=== build compositeur ring3 (tranchant vertical) ==="
# Il etait COMPILE en CI Fast et jamais EXECUTE, alors qu'un commentaire
# affirmait le contraire. Un contrat qu'aucun processus n'exerce ne prouve rien
# de plus que sa syntaxe.
(
  cd userland/services/composited
  OUT="$PWD/../../../$OUT" bash ./build.sh
)

echo "=== build libc/header integration probe ==="
musl-gcc \
  -O2 -Wall -Wextra -fno-stack-protector \
  -static-pie \
  tools/userland/native-ipc-probe.c \
  -o "$LIBC"

file "$RING3" "$LIBC" "$COMPOSITED"

python3 tools/native/make_native_ipc_probe_image.py \
  --ring3-probe "$RING3" \
  --libc-probe "$LIBC" \
  --composited "$COMPOSITED" \
  --image "$IMAGE"

: > "$LOG"
qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$IMAGE" \
  -m 4096 \
  -smp 4 \
  -cpu max \
  -display none \
  -no-reboot \
  -netdev user,id=net0 \
  -device e1000,netdev=net0 \
  -audiodev none,id=muet \
  -device AC97,audiodev=muet \
  -serial file:"$LOG" &
QEMU_PID=$!

cleanup() {
  kill -TERM "$QEMU_PID" 2>/dev/null || true
  sleep 0.5
  kill -KILL "$QEMU_PID" 2>/dev/null || true
  wait "$QEMU_PID" 2>/dev/null || true
}
trap cleanup EXIT

fatal_re='\*\*\* KERNEL PANIC \*\*\*|DOUBLE FAULT|TRIPLE FAULT|SpinLock recursive acquisition|BKL(-FR)?.*VIOLATION'
deadline=$((SECONDS + 120))

while :; do
  if grep -aEq "$fatal_re" "$LOG"; then
    echo "fatal kernel detecte" >&2
    tail -n 200 "$LOG" >&2
    exit 1
  fi

  if grep -aFq 'COMPOSITED_SLICE_FAIL' "$LOG"; then
    echo "tranchant composited en echec" >&2
    grep -aF '[COMPOSITED]' "$LOG" >&2 || true
    exit 1
  fi

  if grep -aFq '[NATIVE-IPC-RING3] OK' "$LOG" \
     && grep -aFq '[NATIVE-IPC] OK' "$LOG" \
     && grep -aFq 'COMPOSITED_SLICE_OK' "$LOG"; then
    break
  fi

  # QEMU may exit immediately after AUTORUN requests a clean shutdown.
  # Re-read the completed log once after exit before declaring failure.
  if ! kill -0 "$QEMU_PID" 2>/dev/null; then
    sleep 0.15
    if grep -aEq "$fatal_re" "$LOG"; then
      echo "fatal kernel detecte" >&2
      tail -n 200 "$LOG" >&2
      exit 1
    fi
    if grep -aFq '[NATIVE-IPC-RING3] OK' "$LOG" \
       && grep -aFq '[NATIVE-IPC] OK' "$LOG"; then
      break
    fi
    echo "QEMU a quitte avant les deux marqueurs de succes" >&2
    tail -n 200 "$LOG" >&2
    exit 1
  fi

  if (( SECONDS >= deadline )); then
    echo "ECHEANCE: native IPC probe n'a pas termine" >&2
    tail -n 200 "$LOG" >&2
    exit 1
  fi

  sleep 0.2
done

cleanup
trap - EXIT

python3 tools/ci/reliability/logscan.py "$LOG"

required=(
  'NATIVE_IPC_AUTORUN_BEGIN'
  '[NATIVE-IPC-RING3] ABI=1.0'
  '[NATIVE-IPC-RING3] CHANNEL_OK'
  '[NATIVE-IPC-RING3] HANDLE_TRANSFER_OK'
  '[NATIVE-IPC-RING3] EVENT_WAITSET_OK'
  '[NATIVE-IPC-RING3] RIGHTS_OK'
  '[NATIVE-IPC-RING3] SHM_OK'
  '[NATIVE-IPC-RING3] OK'
  '[NATIVE-IPC] ABI=1.0'
  '[NATIVE-IPC] OK'
  '[COMPOSITED] ok   surface accordee, region transferee attenuee'
  '[COMPOSITED] ok   les pixels ecrits par le client sont ceux que le compositeur compose'
  '[COMPOSITED] ok   un tampon est rendu au client apres la presentation'
  '[COMPOSITED] echecs=0'
  'COMPOSITED_SLICE_OK'
)

for marker in "${required[@]}"; do
  if ! grep -aFq "$marker" "$LOG"; then
    echo "marqueur obligatoire absent: $marker" >&2
    exit 1
  fi
done

grep -aE 'NATIVE_IPC_AUTORUN|\[NATIVE-IPC|\[COMPOSITED\]|COMPOSITED_SLICE' "$LOG" | tail -n 120
echo "NATIVE_IPC_RUNTIME_OK"
