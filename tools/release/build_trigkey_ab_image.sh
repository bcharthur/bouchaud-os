#!/usr/bin/env bash
# BOUCHAUD_TRIGKEY_AB_IMAGE_V1 — image A/B de PRE-QUALIFICATION.
#
# Usage: bash tools/release/build_trigkey_ab_image.sh target/trigkey-ab
# N'écrit JAMAIS sur un disque/USB : produit uniquement des fichiers sous OUT.
# Système initial A=bon, B=bon (non candidat). Les quatre services natifs
# nécessaires au commit de santé sont embarqués dans les deux slots.
# Pas de clé de version de production : updated ne peut pas accepter d'update
# signée tant qu'une vraie politique de confiance n'a pas été configurée.
# PHYSICAL=NOT_RUN, même après compilation réussie.
set -euo pipefail
cd "$(dirname "$0")/../.."
OUT=${1:?usage: build_trigkey_ab_image.sh OUT_DIR}
[ -z "$(git status --porcelain --untracked-files=normal)" ] || {
  echo "TRIGKEY_AB_REFUSE arbre_sale : committer les changements d'abord" >&2; exit 2;
}
COMMIT=$(git rev-parse HEAD)
TOOLCHAIN=$(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)
[ -n "$TOOLCHAIN" ] || { echo "TRIGKEY_AB_REFUSE toolchain_absente" >&2; exit 2; }
mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)
[ ! -e "$OUT/disque-ab.img" ] || {
  echo "TRIGKEY_AB_REFUSE image_deja_presente (aucun ecrasement)" >&2; exit 2;
}
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# Les mêmes chemins que le banc A/B de production.
cargo +"$TOOLCHAIN" build --release \
  --manifest-path tools/reference/uefi-preboot-probe/Cargo.toml \
  --target x86_64-unknown-uefi
SHIM="$PWD/tools/reference/uefi-preboot-probe/target/x86_64-unknown-uefi/release/bouchaud-uefi-preboot-probe.efi"
BOUCHAUD_BUILD_COMMIT=$(git rev-parse --short=12 HEAD) \
  cargo +"$TOOLCHAIN" build --target targets/x86_64-bouchaud_os_uefi.json \
  --no-default-features --features uefi-boot,reference-bringup,reference-desktop
cp target/x86_64-bouchaud_os_uefi/debug/bouchaud-os "$OUT/kernel.elf"
strip --strip-debug -o "$OUT/kernel-slot.elf" "$OUT/kernel.elf"

cp -r tools/reference/uefi-image-builder "$WORK/builder"
(cd "$WORK/builder" && cargo +"$TOOLCHAIN" build --release -q)
BUILDER="$WORK/builder/target/release/bouchaud-reference-uefi-image-builder"
for slot in a b; do
  "$BUILDER" --charge-installation "$OUT/kernel-slot.elf" "$OUT/slot-$slot"
done
"$BUILDER" --charge-installation "$OUT/kernel-slot.elf" "$OUT/recovery"

SDK=userland/libs/bouchaud-sdk/build-program.sh
for name in serviced healthd updated logd; do
  "$SDK" "$WORK/$name" "userland/services/$name/$name.c"
done

# Le ramdisk utilise le même tar USTAR déterministe que run_ab_rollback.sh.
python3 - "$OUT" "$WORK" <<'PY'
import io
import sys
import tarfile
from pathlib import Path
out, work = map(Path, sys.argv[1:3])
for slot in ("a", "b"):
    with tarfile.open(out / f"slot-{slot}" / "ramdisk", "w", format=tarfile.USTAR_FORMAT) as archive:
        for name in ("serviced", "healthd", "updated", "logd"):
            data = (work / name).read_bytes()
            info = tarfile.TarInfo(f"bin/{name}")
            info.size = len(data)
            info.mtime = 0
            info.mode = 0o755
            archive.addfile(info, io.BytesIO(data))
PY

# Boot actif A connu bon, B sain mais non candidat ; pas de mise à jour fantôme.
rustc +"$TOOLCHAIN" --edition 2021 -O tools/update/meta.rs -o "$WORK/meta"
"$WORK/meta" a-bon "$OUT/meta.img"
"$BUILDER" --disque-ab "$OUT/disque-ab.img" "$SHIM" "$OUT/meta.img" \
  "$OUT/slot-a" "$OUT/slot-b" "$OUT/recovery" > "$OUT/disposition.txt"
# BOUCHAUD_RELEASE_V1 : l'image d'UN slot, telle que --disque-ab la pose --
# la charge utile d'une mise a jour (ce que `updated` ecrit dans le slot
# inactif et que le manifeste signe nomme).
"$BUILDER" --image-slot "$OUT/slot-a" "$OUT/system-slot.img"
python3 tools/release/verify_trigkey_ab_image.py "$OUT" "$COMMIT"
(cd "$OUT" && sha256sum disque-ab.img > disque-ab.img.sha256)
echo "TRIGKEY_AB_BUILD_ONLY_OK commit=${COMMIT:0:12} image=$OUT/disque-ab.img PHYSICAL=NOT_RUN"
