#!/usr/bin/env bash
# L'horloge OSS de /dev/dsp avance au rythme du son joue (BOUCHAUD_OSS_HORLOGE_V1).
#
#   tools/ci/run_oss_horloge.sh BOOTIMAGE
#
# AC'97 emule, `-audiodev none` : QEMU consomme le PCM en temps reel, comme
# s'il etait joue. C'est la boucle meme de PlaybackStreamBouchaud (Ladybird),
# sans Ladybird : ce que `currentTime` d'une balise <audio> verra.
# QEMU AC'97 n'est PAS le HDA du Trigkey : ceci ne valide aucun materiel reel.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_oss_horloge.sh BOOTIMAGE}

SCENARIO=scenario-oss-horloge
IMAGE=oss-horloge.img
LOG=serie-oss-horloge.log
rm -rf "$SCENARIO" "$IMAGE" "$LOG"
OUT=out-oss-horloge
rm -rf "tools/userland/$OUT"
(cd tools/userland && OUT=$OUT ./build.sh musl >/dev/null)
mkdir -p "$SCENARIO/bin"
cp "tools/userland/$OUT/oss-horloge-probe" "$SCENARIO/bin/"
cat > "$SCENARIO/autorun" <<'AUTORUN'
/bin/oss-horloge-probe
echo OSS_HORLOGE_FIN
AUTORUN
(cd tools/userland && IMAGE="$PWD/../../$IMAGE" ./mkdisk.sh "$PWD/../../$SCENARIO")

: > "$LOG"
qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$IMAGE" \
  -m 2048 -smp 2 -display none -no-reboot \
  -audiodev none,id=muet -device AC97,audiodev=muet \
  -serial file:"$LOG" &
PID=$!
LIMITE=$((SECONDS + 300))
while kill -0 "$PID" 2>/dev/null; do
  if (( SECONDS >= LIMITE )); then echo "ECHEANCE atteinte" >&2; break; fi
  grep -aq OSS_HORLOGE_FIN "$LOG" 2>/dev/null && { sleep 1; break; }
  sleep 0.5
done
kill -TERM "$PID" 2>/dev/null || true
sleep 1
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true

sed -E 's/\x1b\[[0-9;]*m//g; s/^\[[^]]*\]\[[^]]*\]\[FPS:[^]]*\] //' "$LOG" \
  | grep -aE '^oss-horloge-probe|^  |OSS_HORLOGE|ac97' || true
if grep -aq 'panicked at' "$LOG"; then echo "panique noyau" >&2; exit 1; fi
grep -aq OSS_HORLOGE_OK "$LOG" || { echo "horloge OSS fausse" >&2; exit 1; }
echo OSS_HORLOGE_CHAINE_OK
