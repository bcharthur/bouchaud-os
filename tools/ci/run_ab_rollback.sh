#!/usr/bin/env bash
# A/B rollback, end to end, under QEMU + OVMF (BOUCHAUD_LOADER_AB_V1).
#
#   tools/ci/run_ab_rollback.sh SCENARIO [OUT_DIR]
#
# SCENARIO:
#   panique    slot B = candidate kernel that panics on every boot
#   gel        slot B = candidate kernel that freezes on every boot
#   meta-0     copy 0 of the metadata corrupted -> copy 1 used
#   meta-1     copy 1 corrupted -> copy 0 used
#   meta-2     both copies corrupted -> RECOVERY_REQUIRED, nothing booted
#
# Each "boot" is a fresh QEMU process on the SAME disk image (state persists
# between boots exactly as it would on a disk). A candidate failure ends by
# KILLING QEMU (SIGKILL): a power cut, never a clean shutdown. The loader
# (UEFI preboot shim) prints `BOUCHAUD_AB_BOOT slot=X raison=...` before the
# handoff; the bench checks the slot sequence:
#   panique / gel : B B B A A   (3 attempts, then automatic rollback)
#   meta-0/meta-1 : B B B A     (the surviving copy carries the candidate)
#   meta-2        : RECOVERY_REQUIRED, no kernel ever starts
# Fail-closed: a missing marker, an unexpected slot, a good kernel that does
# not start, or a kernel started under RECOVERY_REQUIRED is red.
set -euo pipefail
cd "$(dirname "$0")/../.."
SCENARIO=${1:?usage: run_ab_rollback.sh panique|gel|meta-0|meta-1|meta-2 [OUT]}
OUT=${2:-target/ab-rollback-$SCENARIO}
TOOLCHAIN=$(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)
FEATURES=uefi-boot,reference-bringup,reference-desktop
BOOT_DELAI_S=${BO_AB_BOOT_S:-150}
rm -rf "$OUT"; mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)

case "$SCENARIO" in
  panique|gel) MAUVAIS=$SCENARIO; META_SPEC=b-candidat:3; ATTENDU="B B B A A" ;;
  meta-0) MAUVAIS=panique; META_SPEC=corrompu:0; ATTENDU="B B B A" ;;
  meta-1) MAUVAIS=panique; META_SPEC=corrompu:1; ATTENDU="B B B A" ;;
  meta-2) MAUVAIS=panique; META_SPEC=corrompu:2; ATTENDU="RECOVERY" ;;
  *) echo "scenario inconnu: $SCENARIO" >&2; exit 2 ;;
esac

# --- build: shim, kernels, payloads, metadata, disk ------------------------
cargo +"$TOOLCHAIN" build --release --manifest-path tools/reference/uefi-preboot-probe/Cargo.toml \
  --target x86_64-unknown-uefi
SHIM=tools/reference/uefi-preboot-probe/target/x86_64-unknown-uefi/release/bouchaud-uefi-preboot-probe.efi
for k in bon "$MAUVAIS"; do
  f=$FEATURES; [ "$k" != bon ] && f="$FEATURES,banc-ab-$k"
  cargo +"$TOOLCHAIN" build --target targets/x86_64-bouchaud_os_uefi.json --no-default-features --features "$f"
  cp target/x86_64-bouchaud_os_uefi/debug/bouchaud-os "$OUT/kernel-$k.elf"
done
TRAVAIL=$(mktemp -d); trap 'rm -rf "$TRAVAIL"' EXIT
cp -r tools/reference/uefi-image-builder "$TRAVAIL/builder"
( cd "$TRAVAIL/builder" && cargo +"$TOOLCHAIN" build --release -q )
BUILDER=$TRAVAIL/builder/target/release/bouchaud-reference-uefi-image-builder
"$BUILDER" --charge-installation "$OUT/kernel-bon.elf" "$OUT/slot-a"
"$BUILDER" --charge-installation "$OUT/kernel-$MAUVAIS.elf" "$OUT/slot-b"
rustc +"$TOOLCHAIN" --edition 2021 -O tools/update/meta.rs -o "$TRAVAIL/meta"
"$TRAVAIL/meta" "$META_SPEC" "$OUT/meta.img"
"$BUILDER" --disque-ab "$OUT/disque-ab.img" "$SHIM" "$OUT/meta.img" "$OUT/slot-a" "$OUT/slot-b"
cp /usr/share/OVMF/OVMF_VARS_4M.fd "$OUT/vars.fd"

# --- boot loop --------------------------------------------------------------
sequence=()
nb=$(wc -w <<< "$ATTENDU")
for i in $(seq 1 "$nb"); do
  log="$OUT/boot-$i.log"
  : > "$log"
  qemu-system-x86_64 -machine q35 -m 2048 -smp 2 -cpu max \
    -drive if=pflash,format=raw,unit=0,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd \
    -drive if=pflash,format=raw,unit=1,file="$OUT/vars.fd" \
    -drive format=raw,file="$OUT/disque-ab.img" \
    -display none -no-reboot -serial file:"$log" >/dev/null 2>&1 &
  pid=$!
  verdict=""
  fin=$((SECONDS + BOOT_DELAI_S))
  while (( SECONDS < fin )); do
    propre=$(sed 's/\x1b\[[0-9;]*m//g' "$log" | tr -d '\r')
    if grep -aq "BOUCHAUD_AB_RECOVERY_REQUIRED" <<< "$propre"; then
      # Give a kernel the chance to (wrongly) start, then judge.
      sleep 15
      propre=$(sed 's/\x1b\[[0-9;]*m//g' "$log" | tr -d '\r')
      if grep -aqE "BOUCHAUD_HWPROBE_BEGIN|BOUCHAUD_AB_CANDIDAT" <<< "$propre"; then verdict=RECOVERY_MAIS_NOYAU; else verdict=RECOVERY; fi
      break
    fi
    slot=$(grep -aoE "BOUCHAUD_AB_BOOT slot=[AB]" <<< "$propre" | tail -1 | sed 's/.*slot=//' || true)
    if [ "$slot" = B ] && grep -aqE "BOUCHAUD_AB_CANDIDAT_(PANIQUE|GEL)_VOLONTAIRE" <<< "$propre"; then
      verdict=B; break
    fi
    if [ "$slot" = A ] && grep -aq "BOUCHAUD_HWPROBE_BEGIN" <<< "$propre"; then verdict=A; break; fi
    if ! kill -0 "$pid" 2>/dev/null; then break; fi
    sleep 1
  done
  # POWER CUT: SIGKILL, no shutdown path.
  kill -KILL "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  [ -n "$verdict" ] || verdict="AUCUN(slot=${slot:-?})"
  raison=$(sed 's/\x1b\[[0-9;]*m//g' "$log" | tr -d '\r' | grep -aoE "BOUCHAUD_AB_(BOOT slot=[AB] raison=[a-z_]+|RECOVERY_REQUIRED raison=[a-z_]+)" | tail -1 || true)
  echo "AB_BOOT n=$i verdict=$verdict ${raison}"
  sequence+=("$verdict")
done

obtenu="${sequence[*]}"
echo "AB_SEQUENCE scenario=$SCENARIO attendu=\"$ATTENDU\" obtenu=\"$obtenu\""
if [ "$obtenu" != "$ATTENDU" ]; then
  echo "AB_ROLLBACK_FAIL scenario=$SCENARIO" >&2
  exit 1
fi
echo "AB_ROLLBACK_OK scenario=$SCENARIO"
