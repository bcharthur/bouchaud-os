#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_ladybird_browser_host.sh BOOTIMAGE NATIVE_DIR}
OUT=${2:?usage: run_ladybird_browser_host.sh BOOTIMAGE NATIVE_DIR}

for f in BouchaudBrowserHost WebContent RequestServer ImageDecoder WebWorker Compositor WebDriver; do
  test -f "$OUT/$f"
  file "$OUT/$f"
  ! readelf -l "$OUT/$f" | grep -q INTERP
done

rm -rf scenario-browser-host ladybird-browser-host.img serie-browser-host.log fixture-browser-host.log
python3 tools/health/browser_host_fixture.py > fixture-browser-host.log 2>&1 &
FIXTURE=$!
trap 'kill "$FIXTURE" 2>/dev/null || true' EXIT
sleep 1
kill -0 "$FIXTURE"

SCENARIO=scenario-browser-host
mkdir -p "$SCENARIO/usr/libexec/ladybird" "$SCENARIO/usr/share/ladybird" "$SCENARIO/etc/ssl/certs"
for f in BouchaudBrowserHost WebContent RequestServer ImageDecoder WebWorker Compositor WebDriver; do
  cp "$OUT/$f" "$SCENARIO/usr/libexec/ladybird/$f"
  chmod 755 "$SCENARIO/usr/libexec/ladybird/$f"
done
cp "$OUT/BouchaudBrowserHost" "$SCENARIO/bo-navigateur"
chmod 755 "$SCENARIO/bo-navigateur"
if [ -d "$OUT/resources" ]; then
  cp -a "$OUT/resources/." "$SCENARIO/usr/share/ladybird/"
fi
cp /etc/ssl/certs/ca-certificates.crt "$SCENARIO/etc/ssl/certs/ca-certificates.crt"
cat > "$SCENARIO/autorun" <<'AUTORUN'
uname
df
ifconfig
echo "=== Bouchaud BrowserHost interactif ==="
export BO_AUTOSTART_BROWSER=1
export BOUCHAUD_M9=1
export BOUCHAUD_BROWSER_HOST=1
export BOUCHAUD_M11=1
export BOUCHAUD_TIME_ZONE=Europe/Paris
export BOUCHAUD_M9_URL='http://10.0.2.2:18082/browser-host.html'
desktop
AUTORUN
(cd tools/userland && IMAGE="$PWD/../../ladybird-browser-host.img" ./mkdisk.sh "$PWD/../../$SCENARIO")

LOG=serie-browser-host.log
: > "$LOG"
DEADLINE=$((SECONDS + 300))
qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file=ladybird-browser-host.img \
  -m 8192 -cpu max -display none -no-reboot \
  -netdev user,id=net0 -device e1000,netdev=net0 \
  -audiodev none,id=muet -device AC97,audiodev=muet \
  -serial file:"$LOG" &
PID=$!
while kill -0 "$PID" 2>/dev/null; do
  if grep -aFq "HOST_SMOKE_OK canvas=1 worker=1 image=1 frame=1" "$LOG" \
     && grep -aFq "BROWSER_HOST_M11_FRAME_PRESENTED" "$LOG" \
     && grep -aFq "M11_DOCUMENT_LOADED" "$LOG"; then
    break
  fi
  if (( SECONDS >= DEADLINE )); then break; fi
  sleep 0.5
done
kill -TERM "$PID" 2>/dev/null || true
sleep 1
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
cat "$LOG"

for marker in \
  '[ladybird-bouchaud] BROWSER_HOST_START' \
  '[ladybird-bouchaud] BROWSER_HOST_INITIALIZED' \
  '[ladybird-bouchaud] M11_GUI_HANDSHAKE_OK' \
  '[ladybird-bouchaud] M11_DOCUMENT_LOADED' \
  '[ladybird-bouchaud] BROWSER_HOST_M11_FRAME_PRESENTED' \
  'HOST_CANVAS OK' 'HOST_WORKER OK pong' 'HOST_IMAGE OK 1x1' 'HOST_IFRAME OK' \
  'HOST_SMOKE_OK canvas=1 worker=1 image=1 frame=1'; do
  grep -aF "$marker" "$LOG"
done
grep -F "BROWSER_HOST_FIXTURE_OK path=/browser-host.html" fixture-browser-host.log
grep -F "BROWSER_HOST_FIXTURE_IMAGE_OK path=/pixel.png" fixture-browser-host.log
grep -F "BROWSER_HOST_FIXTURE_FRAME_OK path=/frame.html" fixture-browser-host.log

for forbidden in 'VERIFICATION FAILED:' IMAGE_DECODER_ABSENT M11_GUI_STREAM_DESYNC 'instruction illegale dans le programme utilisateur'; do
  if grep -aFq "$forbidden" "$LOG"; then
    echo "diagnostic interdit detecte: $forbidden" >&2
    exit 1
  fi
done
echo LADYBIRD_BROWSER_HOST_OK
