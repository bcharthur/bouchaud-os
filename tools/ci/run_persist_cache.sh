#!/usr/bin/env bash
# Un cache qui deborde ne doit pas emporter la persistance : deux demarrages
# sur la MEME image (BOUCHAUD_PERSIST_CACHE_JETABLE_V1).
#
#   tools/ci/run_persist_cache.sh BOOTIMAGE
#
# Le premier demarrage depose temoins, reglages, un cache `CACHEDIR.TAG` de
# 2100 fichiers (plus que les 2048 entrees de la zone) et un petit cache, puis
# `fsync`. Le second relit : le non-jetable doit etre intact, le gros cache
# ecarte en entier, le petit conserve. Refabriquer l'image entre les deux
# effacerait ce qu'on verifie : la sonde reconnait seule son passage.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_persist_cache.sh BOOTIMAGE}

SCENARIO=scenario-persist-cache
IMAGE=persist-cache.img
rm -rf "$SCENARIO" "$IMAGE" serie-persist-cache-*.log
OUT=out-persist-cache
rm -rf "tools/userland/$OUT"
(cd tools/userland && OUT=$OUT ./build.sh musl >/dev/null)
mkdir -p "$SCENARIO/bin"
cp "tools/userland/$OUT/cache-persist-probe" "$SCENARIO/bin/"
cat > "$SCENARIO/autorun" <<'AUTORUN'
/bin/cache-persist-probe
echo PERSIST_CACHE_FIN
AUTORUN
(cd tools/userland && IMAGE="$PWD/../../$IMAGE" ./mkdisk.sh "$PWD/../../$SCENARIO")

demarre() { # demarre <journal>
  : > "$1"
  qemu-system-x86_64 \
    -drive format=raw,file="$BOOT" \
    -drive format=raw,file="$IMAGE" \
    -m 2048 -smp 2 -display none -no-reboot \
    -serial file:"$1" &
  local pid=$! limite=$((SECONDS + 300))
  while kill -0 "$pid" 2>/dev/null; do
    if (( SECONDS >= limite )); then echo "ECHEANCE atteinte ($1)" >&2; break; fi
    if grep -aq PERSIST_CACHE_FIN "$1" 2>/dev/null; then sleep 3; break; fi
    sleep 0.5
  done
  kill -TERM "$pid" 2>/dev/null || true
  sleep 1
  kill -KILL "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
}

# Le journal serie prefixe chaque ligne d'un horodatage colore.
lignes() { sed -E 's/\x1b\[[0-9;]*m//g; s/^\[[^]]*\]\[[^]]*\]\[FPS:[^]]*\] //' "$1" \
  | grep -aE '^cache-persist-probe|^  |CACHE_PERSIST|BOUCHAUD_PERSIST|persistance:' || true; }
echo "== demarrage 1 =="
demarre serie-persist-cache-1.log
lignes serie-persist-cache-1.log
echo "== demarrage 2 =="
demarre serie-persist-cache-2.log
lignes serie-persist-cache-2.log

grep -aq CACHE_PERSIST_PASSAGE1_OK serie-persist-cache-1.log || { echo "passage 1 en echec" >&2; exit 1; }
grep -aq 'BOUCHAUD_PERSIST_CACHE_ECARTE arbres=1 fichiers=2101' serie-persist-cache-1.log \
  || { echo "le noyau n'a pas annonce l'ecart du gros cache" >&2; exit 1; }
grep -aq CACHE_PERSIST_OK serie-persist-cache-2.log || { echo "passage 2 en echec" >&2; exit 1; }
if grep -aq 'panicked at' serie-persist-cache-1.log serie-persist-cache-2.log; then
  echo "panique noyau" >&2; exit 1
fi
echo PERSIST_CACHE_OK
