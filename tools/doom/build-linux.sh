#!/usr/bin/env bash
# DOOM pour Bouchaud OS, construit sous Linux (CI, WSL, poste Linux).
#
# Meme contrat que build-windows.ps1 : Doomgeneric epingle
# (dcb7a8dbc7a16ce3dda29382ac9aae9d77d21284), Freedoom 0.13.0 verifie par
# SHA-256, ELF x86-64 statique PIE sans PT_INTERP. Le compilateur est
# musl-gcc (paquet musl-tools) au lieu de Zig : la cible est la meme
# (x86_64-linux-musl).
#
#   tools/doom/build-linux.sh            -> target/doom-bouchaud/doom-bouchaud
#                                           tools/doom/wads/freedoom1.wad
set -euo pipefail
cd "$(dirname "$0")/../.."
DOOM=tools/doom
CIBLE=target/doom-bouchaud
mkdir -p "$CIBLE" "$DOOM/wads"
COMMIT=dcb7a8dbc7a16ce3dda29382ac9aae9d77d21284
if [ ! -f "$DOOM/upstream/doomgeneric/Makefile" ]; then
  git clone -q https://github.com/ozkl/doomgeneric.git "$DOOM/upstream"
fi
git -C "$DOOM/upstream" checkout -q --detach "$COMMIT"

WAD="$DOOM/wads/freedoom1.wad"
if [ ! -f "$WAD" ]; then
  ZIP="$CIBLE/freedoom.zip"
  [ -f "$ZIP" ] || curl -sSL -o "$ZIP" \
    https://github.com/freedoom/freedoom/releases/download/v0.13.0/freedoom-0.13.0.zip
  echo "3f9b264f3e3ce503b4fb7f6bdcb1f419d93c7b546f4df3e874dd878db9688f59  $ZIP" | sha256sum -c -
  python3 - "$ZIP" "$WAD" <<'PY'
import sys, zipfile
z = zipfile.ZipFile(sys.argv[1])
nom = next(n for n in z.namelist() if n.endswith('/freedoom1.wad'))
open(sys.argv[2], 'wb').write(z.read(nom))
PY
fi
[ "$(stat -c %s "$WAD")" -ge 10000000 ] || { echo "WAD tronque" >&2; exit 1; }

command -v musl-gcc >/dev/null || { echo "musl-gcc absent (apt-get install musl-tools)" >&2; exit 1; }
SRC="$DOOM/upstream/doomgeneric"
OBJS=$(sed -n 's/^SRC_DOOM\s*=\s*//p' "$SRC/Makefile" | tr ' ' '\n' | grep -v '^doomgeneric_xlib.o$' | grep '\.o$')
FICHIERS=()
for o in $OBJS; do FICHIERS+=("$SRC/${o%.o}.c"); done
[ "${#FICHIERS[@]}" -ge 75 ] || { echo "sources moteur insuffisantes" >&2; exit 1; }
musl-gcc -std=gnu11 -O2 -fPIE -static-pie -DNORMALUNIX -DLINUX -DSNDSERV -D_DEFAULT_SOURCE \
  -I"$SRC" -ffunction-sections -fdata-sections -Wl,--gc-sections -w \
  "${FICHIERS[@]}" "$DOOM/doomgeneric_bouchaud.c" -lm -o "$CIBLE/doom-bouchaud.new"
python3 - "$CIBLE/doom-bouchaud.new" <<'PY'
import struct, sys
d = open(sys.argv[1], 'rb').read()
assert d[:6] == b'\x7fELF\x02\x01', 'pas un ELF x86-64 LE'
typ, mach = struct.unpack_from('<HH', d, 16)
assert (typ, mach) == (3, 62), f'ELF incorrect type={typ} machine={mach}'
phoff, = struct.unpack_from('<Q', d, 32)
phentsize, phnum = struct.unpack_from('<HH', d, 54)
charges = 0
for i in range(phnum):
    k, = struct.unpack_from('<I', d, phoff + i * phentsize)
    assert k != 3, 'PT_INTERP present'
    charges += k == 1
assert charges, 'aucun PT_LOAD'
print('DOOM_ELF_OK static-pie')
PY
mv "$CIBLE/doom-bouchaud.new" "$CIBLE/doom-bouchaud"
sha256sum "$CIBLE/doom-bouchaud" "$WAD"
