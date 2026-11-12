#!/usr/bin/env bash
# BOUCHAUD_STATE_AB_V1 -- la partition STATE survit aux coupures et au changement de slot.
#
#   tools/ci/run_state_ab.sh OUT_DIR
#
# OUT_DIR : une sortie de tools/release/build_trigkey_ab_image.sh (disque A/B
# AVEC partition BOUCHAUD-STATE). L'image n'est JAMAIS modifiee : le banc
# travaille sur une copie. Aucun peripherique physique n'est touche.
#
# Trois demarrages du meme disque, chacun coupe par SIGKILL (une coupure, pas
# une extinction) une fois son compteur synchronise :
#   1. slot A : compteur 1 ;
#   2. slot A : compteur 2 ;
#   META reecrit (B actif, comme apres une mise a jour confirmee) ;
#   3. slot B : compteur 3 -- STATE n'appartient a aucun slot.
# Verdict : tools/ci/state_ab_verdict.py. PHYSICAL=NOT_RUN.
set -euo pipefail
cd "$(dirname "$0")/../.."
OUT=$(cd "${1:?usage: run_state_ab.sh OUT_DIR}" && pwd)
OVMF=${BO_OVMF_DIR:-/usr/share/OVMF}
[ -f "$OUT/disque-ab.img" ] && [ -f "$OUT/disposition.txt" ] || { echo "STATE_AB_REFUSE image_absente" >&2; exit 2; }
grep -q "nom=BOUCHAUD-STATE " "$OUT/disposition.txt" || { echo "STATE_AB_REFUSE partition_state_absente" >&2; exit 2; }
(cd "$OUT" && sha256sum -c --quiet disque-ab.img.sha256) || { echo "STATE_AB_REFUSE empreinte" >&2; exit 2; }
J=$(mktemp -d)
pid=""
fin() { [ -n "$pid" ] && kill -KILL "$pid" 2>/dev/null; wait 2>/dev/null; rm -rf -- "$J"; }
trap fin EXIT
cp "$OUT/disque-ab.img" "$J/essai.img"
cp "$OVMF/OVMF_VARS_4M.fd" "$J/vars.fd"

demarre() {  # N : demarre, attend le compteur, coupe
  local log="$OUT/state-boot-$1.log"
  : > "$log"
  qemu-system-x86_64 -machine q35 -m 2048 -smp 2 -cpu max \
    -drive if=pflash,format=raw,unit=0,readonly=on,file="$OVMF/OVMF_CODE_4M.fd" \
    -drive if=pflash,format=raw,unit=1,file="$J/vars.fd" \
    -drive if=none,id=disque,format=raw,file="$J/essai.img" \
    -device nvme,drive=disque,serial=BOUCHAUD_STATE_AB \
    -display none -no-reboot -serial file:"$log" >/dev/null 2>&1 &
  pid=$!
  local limite=$((SECONDS + ${BO_STATE_AB_S:-300}))
  until grep -aq "BOUCHAUD_PERSIST_DEMARRAGES" "$log"; do
    if (( SECONDS >= limite )) || ! kill -0 "$pid" 2>/dev/null; then
      echo "STATE_AB_FAIL demarrage $1 : pas de compteur" >&2
      sed -E 's/\x1b\[[0-9;]*m//g' "$log" | grep -aE "PERSIST|AB_BOOT|PANIC|FAULT" | tail -8 >&2
      return 1
    fi
    sleep 1
  done
  kill -KILL "$pid"; wait "$pid" 2>/dev/null || true; pid=""
  echo "STATE_AB_DEMARRAGE $1 $(grep -m1 -aoE 'BOUCHAUD_PERSIST_DEMARRAGES n=[0-9]+ slot=[AB-] sync=-?[0-9]+' "$log")"
}

demarre 1
demarre 2
# B actif et confirme, comme apres une mise a jour reussie : META seul change.
TC=$(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)
rustc +"$TC" --edition 2021 -O tools/update/meta.rs -o "$J/meta"
"$J/meta" b-actif "$J/meta-b.img"
lba=$(sed -n 's/.*nom=BOUCHAUD-META premier_lba=\([0-9]*\).*/\1/p' "$OUT/disposition.txt")
[ -n "$lba" ] || { echo "STATE_AB_FAIL lba_meta_introuvable" >&2; exit 1; }
dd if="$J/meta-b.img" of="$J/essai.img" bs=512 seek="$lba" conv=notrunc status=none
demarre 3
python3 tools/ci/state_ab_verdict.py "$OUT"/state-boot-{1,2,3}.log
(cd "$OUT" && sha256sum -c --quiet disque-ab.img.sha256) || { echo "STATE_AB_FAIL image_originale_modifiee" >&2; exit 1; }
