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
./renommage-probe
./faute-noncanonique-probe
./sigchld-multifil-probe
./compta-stress-probe 5
./mtime-probe
cd ../../..

SCENARIO=scenario-primitives
mkdir -p "$SCENARIO/bin"
for f in verrous-probe exec-fd-probe wal-probe disque-probe nom-long-probe session-probe sendfile-probe renommage-probe faute-noncanonique-probe sigchld-multifil-probe compta-stress-probe mtime-probe; do
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
/bin/renommage-probe
/bin/faute-noncanonique-probe
/bin/sigchld-multifil-probe
/bin/compta-stress-probe 30
/bin/mtime-probe
/bin/session-probe 4
echo SESSION_INVITE_REVENUE
strace off
echo PRIMITIVES_FIN
AUTORUN
(cd tools/userland && IMAGE="$PWD/../../primitives.img" ./mkdisk.sh "$PWD/../../scenario-primitives")

LOG=serie-primitives.log
: > "$LOG"
# BOUCHAUD_SYMBOLISE_NOYAU_V1 : en cas d'echec, ce que faisaient les coeurs,
# symbolise contre le noyau EXACT qui a tourne (les artefacts ne sont pas
# toujours lisibles ; le journal du job, si). Imprime EN DERNIER : l'API ne
# rend que la fin d'un journal de job.
ELF="$(dirname "$BOOT")/bouchaud-os"
trap 'rc=$?; if [ "$rc" -ne 0 ]; then python3 tools/ci/symbolise_noyau.py "$LOG" "$ELF"; fi; exit "$rc"' EXIT
# BO_QEMU_KVM=1 : memes sondes sous KVM (-cpu host). La course que
# compta-stress-probe fabrique depend du rythme reel des vCPU : sous TCG les
# coeurs avancent par tranches emulees, sous KVM ils tournent en parallele
# et l'hote peut suspendre un vCPU au milieu d'une section (run 37589903681).
ACCEL=""
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
fi
echo "PRIMITIVES_ACCEL ${ACCEL:-tcg}"
# shellcheck disable=SC2086
qemu-system-x86_64 $ACCEL \
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

for marker in VERROUS_POSIX_OK EXEC_FD_OK WAL_PROBE_OK DISQUE_PROBE_OK NOM_LONG_OK SENDFILE_OK RENOMMAGE_OK FAUTE_NONCANONIQUE_OK SIGCHLD_MULTIFIL_OK COMPTA_STRESS_OK MTIME_STABLE_OK \
              'SESSION_PERE_SORT fils=4' SESSION_INVITE_REVENUE PRIMITIVES_FIN; do
  grep -aF "$marker" "$LOG"
done
# BOUCHAUD_COMPTA_STRESS_V1 : ce que la sequence a du refaire (preuve que la
# course a eu lieu), lu sur le dernier releve du noyau.
awk 'match($0, /compta_relues=[0-9]+/) { v = substr($0, RSTART, RLENGTH) } END { print (v != "" ? v : "compta_relues=absent") }' "$LOG"
if grep -aq 'KERNEL PANIC' "$LOG"; then echo "panique noyau" >&2; exit 1; fi
if grep -aqE "ata: (lecture|ecriture) " "$LOG"; then
  echo "Le pilote ATA a signale au moins une commande en echec" >&2
  grep -aE "ata: (lecture|ecriture) " "$LOG" >&2
  exit 1
fi
echo OS_PRIMITIVES_OK
