#!/usr/bin/env bash
# PRETRIGKEY_READY: QEMU/OVMF check of the exact A/B image, NEVER a disk flash.
# Usage: bash tools/release/smoke_trigkey_ab_qemu.sh OUT_DIR
set -euo pipefail
cd "$(dirname "$0")/../.."
OUT=${1:?usage: smoke_trigkey_ab_qemu.sh OUT_DIR}
OUT=$(cd "$OUT" && pwd)
[ -f "$OUT/disque-ab.img" ] && [ -f "$OUT/manifest-ab.json" ] || {
  echo "TRIGKEY_QEMU_REFUSE image_ou_manifeste_absent" >&2
  exit 2
}
[ -f /usr/share/OVMF/OVMF_CODE_4M.fd ] && [ -f /usr/share/OVMF/OVMF_VARS_4M.fd ] || {
  echo "TRIGKEY_QEMU_REFUSE ovmf_absent" >&2
  exit 2
}
(cd "$OUT" && sha256sum -c disque-ab.img.sha256)
cp /usr/share/OVMF/OVMF_VARS_4M.fd "$OUT/vars-qemu.fd"
LOG="$OUT/boot-qemu.log"
: > "$LOG"
qemu-system-x86_64 -machine q35 -m 2048 -smp 2 -cpu max \
  -drive if=pflash,format=raw,unit=0,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd \
  -drive if=pflash,format=raw,unit=1,file="$OUT/vars-qemu.fd" \
  -drive if=none,id=disque,format=raw,file="$OUT/disque-ab.img" \
  -device nvme,drive=disque,serial=BOUCHAUD_AB_TEST \
  -display none -no-reboot -serial file:"$LOG" >/dev/null 2>&1 &
pid=$!
cleanup() {
  kill -TERM "$pid" 2>/dev/null || true
  sleep 0.2
  kill -KILL "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
}
trap cleanup EXIT
end=$((SECONDS + ${BO_TRIGKEY_QEMU_S:-240}))
while (( SECONDS < end )); do
  if python3 tools/release/verify_trigkey_ab_boot.py "$OUT" >/dev/null 2>&1; then
    # Give a later kernel fault a chance to invalidate the proof.
    sleep 5
    python3 tools/release/verify_trigkey_ab_boot.py "$OUT"
    echo "TRIGKEY_QEMU_BOOT_OK PHYSICAL=NOT_RUN"
    exit 0
  fi
  if ! kill -0 "$pid" 2>/dev/null; then
    echo "TRIGKEY_QEMU_FAIL machine_arretee_avant_sante" >&2
    python3 tools/release/verify_trigkey_ab_boot.py "$OUT" >&2 || true
    exit 1
  fi
  sleep 1
done
echo "TRIGKEY_QEMU_FAIL timeout_sans_sante" >&2
python3 tools/release/verify_trigkey_ab_boot.py "$OUT" >&2 || true
exit 1
