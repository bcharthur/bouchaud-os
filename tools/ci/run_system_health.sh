#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_system_health.sh BOOTIMAGE}

rm -rf health-scenario health.img health.log health-report.json fixture.log fixture.pid
python3 - <<'PY'
from pathlib import Path
import tarfile
root = Path("health-scenario")
root.mkdir(exist_ok=True)
(root / "autorun").write_bytes(Path("tools/health/autorun.bsh").read_bytes())
image = Path("health.img")
with tarfile.open(image, "w", format=tarfile.USTAR_FORMAT) as tar:
    info = tar.gettarinfo(str(root / "autorun"), arcname="./autorun")
    info.uid = info.gid = 0
    info.uname = info.gname = "root"
    info.mode = 0o644
    with (root / "autorun").open("rb") as src:
        tar.addfile(info, src)
with image.open("ab") as out:
    out.write(b"\0" * (8 * 1024 * 1024))
print(f"health.img: {image.stat().st_size} octets")
PY

python3 tools/health/fixture_server.py --port 18080 >fixture.log 2>&1 &
FIXTURE=$!
trap 'kill "$FIXTURE" 2>/dev/null || true' EXIT
sleep 1
curl --fail http://127.0.0.1:18080/health

set +e
timeout 180 qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file=health.img \
  -m 2048 -display none -no-reboot \
  -netdev user,id=net0 -device e1000,netdev=net0 \
  -audiodev none,id=snd0 -device AC97,audiodev=snd0 \
  -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
  -serial file:health.log
code=$?
set -e
cat health.log
printf 'qemu exit=%s\n' "$code"
test "$code" -eq 33
# BOUCHAUD_PIPEFAIL_SANS_SIGPIPE_V1 : `! grep ...` seul sur sa ligne ne
# declenche jamais `set -e` (bash l'exclut) : cette verification etait inerte.
# Les marqueurs exacts d'une panique, pas « panic » insensible a la casse, qui
# prendrait « aucune panique » pour une panique.
if grep -aqE 'KERNEL PANIC|panicked at' health.log; then
  echo "panique noyau dans health.log" >&2
  grep -aE 'KERNEL PANIC|panicked at' health.log >&2 || true
  exit 1
fi
grep -F "=== AUTORUN FIN === statut=0" health.log
grep -F "HEALTH_COMPLETE" health.log
python3 tools/health/verify_health.py --log health.log --report health-report.json

echo SYSTEM_HEALTH_OK
