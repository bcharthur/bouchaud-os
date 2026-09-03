#!/usr/bin/env bash
set -euo pipefail
# La plateforme de reference vit dans un seul fichier : voir tools/ci/plateforme.sh.
. "$(dirname "$0")/plateforme.sh"
BOOT=${1:?usage: run_qemu_smoke.sh BOOTIMAGE}
LOG=${2:-qemu-smoke.log}

echo "=== QEMU smoke: $BOOT ==="
rm -f "$LOG"
set +e
timeout 45 qemu-system-x86_64 \
  $(bouchaud_machine_args) \
  -drive format=raw,file="$BOOT" \
  -display none -serial file:"$LOG" -no-reboot
code=$?
set -e
cat "$LOG" || true

# Sans autorun la machine peut rester vivante: timeout est donc normal.
if [ "$code" -ne 0 ] && [ "$code" -ne 124 ]; then
  echo "QEMU a quitte avec un code inattendu: $code" >&2
  exit 1
fi

test -s "$LOG" || { echo "aucune sortie serie" >&2; exit 1; }
if grep -aiEq '\*\*\* KERNEL PANIC \*\*\*|DOUBLE FAULT|panicked at' "$LOG"; then
  echo "panic/fault detecte au boot" >&2
  exit 1
fi
echo "QEMU_SMOKE_OK"
