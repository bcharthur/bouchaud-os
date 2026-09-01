#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_os_primitives.sh BOOTIMAGE}

rm -rf scenario-primitives primitives.img serie-primitives.log tools/userland/out-sondes
(cd tools/userland && OUT=out-sondes ./build.sh musl)
cd tools/userland/out-sondes
./verrous-probe
./exec-fd-probe
./wal-probe
./nom-long-probe
./sendfile-probe
cd ../../..

SCENARIO=scenario-primitives
mkdir -p "$SCENARIO/bin"
for f in verrous-probe exec-fd-probe wal-probe disque-probe nom-long-probe session-probe sendfile-probe; do
  cp "tools/userland/out-sondes/$f" "$SCENARIO/bin/"
done
python3 - <<'PY'
bloc = (b"B" * 63 + b"\n") * 16384
with open("scenario-primitives/bin/gros.bin", "wb") as f:
    for _ in range(96):
        f.write(bloc)
PY
cat > "$SCENARIO/autorun" <<'AUTORUN'
strace echecs
/bin/verrous-probe
/bin/exec-fd-probe
/bin/wal-probe
/bin/disque-probe /bin/gros.bin
/bin/nom-long-probe
/bin/sendfile-probe
/bin/session-probe 4
echo SESSION_INVITE_REVENUE
strace off
echo PRIMITIVES_FIN
AUTORUN
(cd tools/userland && IMAGE="$PWD/../../primitives.img" ./mkdisk.sh "$PWD/../../scenario-primitives")

LOG=serie-primitives.log
: > "$LOG"
qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file=primitives.img \
  -m 4096 -smp 4 -display none -no-reboot \
  -netdev user,id=net0 -device e1000,netdev=net0 \
  -serial file:"$LOG" &
PID=$!
DEADLINE=$((SECONDS + 600))
while kill -0 "$PID" 2>/dev/null; do
  if (( SECONDS >= DEADLINE )); then echo "ECHEANCE atteinte" >&2; break; fi
  sleep 0.5
done
kill -TERM "$PID" 2>/dev/null || true
sleep 1
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
tail -c 262144 "$LOG"

for marker in VERROUS_POSIX_OK EXEC_FD_OK WAL_PROBE_OK DISQUE_PROBE_OK NOM_LONG_OK SENDFILE_OK \
              'SESSION_PERE_SORT fils=4' SESSION_INVITE_REVENUE PRIMITIVES_FIN; do
  grep -aF "$marker" "$LOG"
done
if grep -aqE "ata: (lecture|ecriture) " "$LOG"; then
  echo "Le pilote ATA a signale au moins une commande en echec" >&2
  grep -aE "ata: (lecture|ecriture) " "$LOG" >&2
  exit 1
fi
echo OS_PRIMITIVES_OK
