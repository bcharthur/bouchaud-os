#!/usr/bin/env bash
set -euo pipefail
BOOT=${1:?usage: run_platform_boot.sh BOOT DISK LOG}
DISK=${2:?usage: run_platform_boot.sh BOOT DISK LOG}
LOG=${3:?usage: run_platform_boot.sh BOOT DISK LOG}
: > "$LOG"
DEADLINE=$((SECONDS + 900))
MAX_LOG=$((64 * 1024 * 1024))
MAX_ECHO=$((512 * 1024))
GRACE=240
VERDICT_VU=0

qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$DISK" \
  -m 12288 -smp 4 -cpu max -display none -no-reboot \
  -netdev user,id=net0 -device e1000,netdev=net0 \
  -audiodev none,id=muet -device AC97,audiodev=muet \
  -serial file:"$LOG" &
PID=$!
while kill -0 "$PID" 2>/dev/null; do
  if (( SECONDS >= DEADLINE )); then echo "ECHEANCE atteinte" >&2; break; fi
  if (( $(stat -c '%s' "$LOG") > MAX_LOG )); then echo "JOURNAL EMBALLE" >&2; break; fi
  if (( VERDICT_VU == 0 )) && grep -aFq "PLATFORM_FULL_OK" "$LOG"; then VERDICT_VU=$SECONDS; fi
  if (( VERDICT_VU > 0 )) && (( SECONDS - VERDICT_VU > GRACE )) && ! grep -aFq "BrowserHost termine" "$LOG"; then
    echo "verdict moteur rendu mais BrowserHost ne s'est pas arrete dans ${GRACE}s" >&2
    break
  fi
  sleep 0.5
done
kill -TERM "$PID" 2>/dev/null || true
sleep 1
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
echo "journal serie: $(stat -c '%s' "$LOG") octets"
tail -c "$MAX_ECHO" "$LOG"
