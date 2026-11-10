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
  -monitor unix:"$OUT/moniteur.sock",server,nowait \
  -no-shutdown \
  -serial file:"$LOG" &
QEMU_PID=$!
CAPTURE="$OUT/ecran-composited.ppm"

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

  # BOUCHAUD_COMPOSITORD_PRESENTATION_V1 : l'ecran est capture par QEMU
  # des que le compositeur ring 3 a presente, avant l'extinction.
  if [ ! -s "$CAPTURE" ] && grep -aFq 'COMPOSITED_PRESENTATION_OK' "$LOG"; then
    python3 - "$OUT/moniteur.sock" "$CAPTURE" <<'PYMON' || true
import socket, sys, time
s = socket.socket(socket.AF_UNIX)
s.connect(sys.argv[1])
s.sendall(f"screendump {sys.argv[2]}\n".encode())
time.sleep(0.8)
s.close()
PYMON
  fi

  if grep -aFq '[NATIVE-IPC-RING3] OK' "$LOG" \
     && grep -aFq '[NATIVE-IPC] OK' "$LOG" \
     && grep -aFq 'COMPOSITED_SLICE_OK' "$LOG" \
     && grep -aFq 'COMPOSITED_PRESENTATION_' "$LOG" \
     && [ -s "$CAPTURE" ]; then
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

# The version the kernel DECLARES (types.rs), not a literal that goes stale at
# every minor bump: the probes must report exactly that.
ABI_TY=src/kernel/native/abi/types.rs
ABI_ATTENDU="$(sed -n 's/^pub const ABI_MAJOR: u16 = \([0-9]*\);/\1/p' "$ABI_TY").$(sed -n 's/^pub const ABI_MINOR: u16 = \([0-9]*\);/\1/p' "$ABI_TY")"
[[ "$ABI_ATTENDU" =~ ^[0-9]+\.[0-9]+$ ]] || { echo "NATIVE_IPC_FAIL version ABI illisible: $ABI_ATTENDU" >&2; exit 1; }
required=(
  'NATIVE_IPC_AUTORUN_BEGIN'
  "[NATIVE-IPC-RING3] ABI=$ABI_ATTENDU"
  '[NATIVE-IPC-RING3] CHANNEL_OK'
  '[NATIVE-IPC-RING3] HANDLE_TRANSFER_OK'
  '[NATIVE-IPC-RING3] EVENT_WAITSET_OK'
  '[NATIVE-IPC-RING3] RIGHTS_OK'
  '[NATIVE-IPC-RING3] SHM_OK'
  '[NATIVE-IPC-RING3] OK'
  "[NATIVE-IPC] ABI=$ABI_ATTENDU"
  '[NATIVE-IPC] OK'
  '[COMPOSITED] ok   surface accordee, region transferee attenuee'
  '[COMPOSITED] ok   les pixels ecrits par le client sont ceux que le compositeur compose'
  '[COMPOSITED] ok   un tampon est rendu au client apres la presentation'
  '[COMPOSITED] echecs=0'
  'COMPOSITED_SLICE_OK'
  # BOUCHAUD_COMPOSITORD_PRESENTATION_V1
  '[COMPOSITED] ok   presentation refusee sans capacite affichage'
  'BOUCHAUD_COMPOSITEUR_LANCE'
  '[COMPOSITED] ok   rectangle hors de la surface refuse'
  '[COMPOSITED] ok   destination hors ecran (debordement u32) refusee'
  '[COMPOSITED] ok   region sans droit de lecture refusee'
  '[COMPOSITED] ok   la capacite ne sert pas de region'
  '[COMPOSITED] ok   la capacite affichage ne se duplique pas'
  '[COMPOSITED] ok   mire posee sur ecran, relue au pixel par le noyau'
  'COMPOSITED_PRESENTATION_OK'
)
if grep -aFq 'COMPOSITED_PRESENTATION_FAIL' "$LOG"; then
  echo "presentation composited en echec" >&2
  grep -aF '[COMPOSITED]' "$LOG" >&2 || true
  exit 1
fi

for marker in "${required[@]}"; do
  if ! grep -aFq "$marker" "$LOG"; then
    echo "marqueur obligatoire absent: $marker" >&2
    exit 1
  fi
done

grep -aE 'NATIVE_IPC_AUTORUN|\[NATIVE-IPC|\[COMPOSITED\]|COMPOSITED_|BOUCHAUD_COMPOSITEUR|BOUCHAUD_DISPLAY_PRESENT' "$LOG" | tail -n 120
# La mire, decodee au pixel sur l'ecran que QEMU a capture.
python3 tools/ci/verifie_capture_composited.py "$CAPTURE" 100 100 320 200
echo "NATIVE_IPC_RUNTIME_OK"
