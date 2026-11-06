#!/usr/bin/env bash
# Image physique Trigkey TRACABLE jusqu'au commit (BOUCHAUD_IMAGE_TRACABLE_V1).
#
#   tools/release/build_trigkey_image.sh OUT_DIR [RAMDISK_LADYBIRD]
#
# Produit dans OUT_DIR :
#   bouchaud-trigkey.img        image UEFI a flasher (mode DD)
#   bouchaud-trigkey.img.sha256
#   kernel.elf                  ELF noyau NON strip (symbolisation des pannes)
#   manifest.json               commit, arbre propre, SHA Ladybird epingle,
#                               toolchain, cible, fonctionnalites, empreintes
#
# Equivalent Linux/CI de tools/reference/IMAGE-TRIGKEY.ps1. Refuse un arbre
# sale sauf BO_IMAGE_ACCEPTE_SALE=1, et le manifeste le dit alors.
set -euo pipefail
cd "$(dirname "$0")/../.."
OUT=${1:?usage: build_trigkey_image.sh OUT_DIR [RAMDISK]}
RAMDISK=${2:-}
TOOLCHAIN=$(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)
FEATURES=uefi-boot,reference-bringup,reference-desktop
TARGET=targets/x86_64-bouchaud_os_uefi.json

COMMIT=$(git rev-parse HEAD)
# Inclure les fichiers non suivis non ignores : une source oubliee ne doit
# pas produire une image qualifiee sur un commit qui ne la contient pas.
SALE=$(git status --porcelain --untracked-files=normal | wc -l)
if [ "$SALE" != 0 ] && [ "${BO_IMAGE_ACCEPTE_SALE:-0}" != 1 ]; then
  echo "IMAGE_REFUSEE arbre_sale fichiers=$SALE (BO_IMAGE_ACCEPTE_SALE=1 pour forcer)" >&2
  exit 2
fi
LADYBIRD_SHA=$(awk '/^## Ladybird/{l=1} l && $1=="sha"{print $2; exit}' third_party/UPSTREAM.md)
[ -n "$LADYBIRD_SHA" ] || { echo "IMAGE_REFUSEE sha_ladybird_introuvable" >&2; exit 2; }

mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)
BOUCHAUD_BUILD_COMMIT=$(git rev-parse --short=12 HEAD) \
  cargo +"$TOOLCHAIN" build --target "$TARGET" --no-default-features --features "$FEATURES"
NOYAU=target/x86_64-bouchaud_os_uefi/debug/bouchaud-os
cp "$NOYAU" "$OUT/kernel.elf"

# Le builder a son propre Cargo : il se construit HORS de la config du depot
# (qui force la cible bare-metal).
TRAVAIL=$(mktemp -d)
trap 'rm -rf "$TRAVAIL"' EXIT
cp -r tools/reference/uefi-image-builder "$TRAVAIL/builder"
ARGS=("$OUT/kernel.elf" "$OUT/bouchaud-trigkey.img" 1920 1080)
[ -n "$RAMDISK" ] && ARGS+=("$(cd "$(dirname "$RAMDISK")" && pwd)/$(basename "$RAMDISK")")
( cd "$TRAVAIL/builder" && cargo +"$TOOLCHAIN" run --release -q -- "${ARGS[@]}" )

( cd "$OUT" && sha256sum bouchaud-trigkey.img > bouchaud-trigkey.img.sha256 )
python3 - "$OUT" "$COMMIT" "$SALE" "$LADYBIRD_SHA" "$TOOLCHAIN" "$FEATURES" "$TARGET" "$RAMDISK" <<'PY'
import hashlib, json, os, subprocess, sys, datetime
out, commit, sale, lb, tc, feats, target, ramdisk = sys.argv[1:9]
def sha(p):
    h = hashlib.sha256()
    with open(p, 'rb') as f:
        for b in iter(lambda: f.read(1 << 20), b''): h.update(b)
    return h.hexdigest()
def cmd(*a):
    try: return subprocess.run(a, capture_output=True, text=True).stdout.strip().splitlines()[0]
    except Exception: return 'UNKNOWN'
img = os.path.join(out, 'bouchaud-trigkey.img')
elf = os.path.join(out, 'kernel.elf')
m = {
    'schema': 'bouchaud-image-manifest/1',
    'bouchaud_commit': commit,
    'tree_clean': sale == '0',
    'ladybird_upstream_sha': lb,
    'ladybird_ramdisk': ({'path': os.path.basename(ramdisk), 'sha256': sha(ramdisk)} if ramdisk else None),
    'toolchain': {'rustc': cmd('rustc', '+' + tc, '-V'), 'cargo': cmd('cargo', '+' + tc, '-V'), 'channel': tc},
    'build': {'target': target, 'profile': 'dev', 'features': feats.split(','),
              'kernel_strip': False, 'min_resolution': '1920x1080'},
    'image': {'file': 'bouchaud-trigkey.img', 'bytes': os.path.getsize(img), 'sha256': sha(img)},
    'kernel_elf': {'file': 'kernel.elf', 'bytes': os.path.getsize(elf), 'sha256': sha(elf)},
    'built_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(timespec='seconds'),
    'proof_level': 'BUILD_ONLY',
    'note': 'Ce manifeste prouve la provenance, pas un boot. PHYSICAL exige la campagne docs/current/TRIGKEY_CAMPAIGN.md.',
}
with open(os.path.join(out, 'manifest.json'), 'w') as f:
    json.dump(m, f, indent=2); f.write('\n')
print('IMAGE_TRACABLE_OK commit=%s image_sha256=%s kernel_sha256=%s ladybird=%s' % (
    commit[:12], m['image']['sha256'][:16], m['kernel_elf']['sha256'][:16], lb[:12]))
PY
