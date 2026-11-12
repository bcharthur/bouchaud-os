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
OVMF=${BO_OVMF_DIR:-/usr/share/OVMF}
[ -f "$OVMF/OVMF_CODE_4M.fd" ] && [ -f "$OVMF/OVMF_VARS_4M.fd" ] || {
  echo "TRIGKEY_QEMU_REFUSE ovmf_absent" >&2
  exit 2
}
(cd "$OUT" && sha256sum -c --quiet disque-ab.img.sha256) || {
  echo "TRIGKEY_QEMU_REFUSE empreinte_initiale_differente" >&2
  exit 2
}
# BOUCHAUD_RELEASE_V1 (review #306): the image under test is the image that
# gets PUBLISHED. The firmware and the shim write to the disk they boot
# (BOUCHAUD-PREBOOT.TXT on the ESP, META), so QEMU boots a DISPOSABLE COPY,
# never the original -- and the original's digest is checked again after.
JETABLE=$(mktemp -d)
cp "$OUT/disque-ab.img" "$JETABLE/disque-essai.img"
cp "$OVMF/OVMF_VARS_4M.fd" "$JETABLE/vars-qemu.fd"
LOG="$OUT/boot-qemu.log"
: > "$LOG"
qemu-system-x86_64 -machine q35 -m 2048 -smp 2 -cpu max \
  -drive if=pflash,format=raw,unit=0,readonly=on,file="$OVMF/OVMF_CODE_4M.fd" \
  -drive if=pflash,format=raw,unit=1,file="$JETABLE/vars-qemu.fd" \
  -drive if=none,id=disque,format=raw,file="$JETABLE/disque-essai.img" \
  -device nvme,drive=disque,serial=BOUCHAUD_AB_TEST \
  -display none -no-reboot -serial file:"$LOG" >/dev/null 2>&1 &
pid=$!
cleanup() {
  kill -TERM "$pid" 2>/dev/null || true
  sleep 0.2
  kill -KILL "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  rm -rf -- "$JETABLE"
}
trap cleanup EXIT
originale_intacte() {
  (cd "$OUT" && sha256sum -c --quiet disque-ab.img.sha256) >/dev/null 2>&1 || {
    echo "TRIGKEY_QEMU_FAIL image_originale_modifiee (l'image publiable a change pendant l'essai)" >&2
    return 1
  }
}
end=$((SECONDS + ${BO_TRIGKEY_QEMU_S:-240}))
while (( SECONDS < end )); do
  if python3 tools/release/verify_trigkey_ab_boot.py "$OUT" >/dev/null 2>&1; then
    # Give a later kernel fault a chance to invalidate the proof.
    sleep "${BO_TRIGKEY_QEMU_GRACE_S:-5}"
    cleanup
    trap - EXIT
    originale_intacte || exit 1
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
