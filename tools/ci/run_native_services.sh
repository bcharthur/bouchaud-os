#!/usr/bin/env bash
# Native platform qualification: SDK programs, cycles hardening, supervised
# ring3 service (BOUCHAUD_NATIVE_SERVICES_V1).
#
#   tools/ci/run_native_services.sh BOOTIMAGE [SMP]
#
# Fail-closed: a kernel fatal, a missing marker, an INVARIANT_VIOLATION, a
# FAIL verdict, or QEMU stopping before SERVICED_QUALIFY_OK is red. The IPC
# round-trip and recovery figures are published, not thresholded: they are
# measurements under TCG/KVM, not product budgets yet.
set -euo pipefail
cd "$(dirname "$0")/../.."

BOOT=${1:?usage: run_native_services.sh BOOTIMAGE [SMP]}
SMP=${2:-4}
OUT=${OUT:-target/native-services-ci}
LOG="$OUT/native-services-smp$SMP.log"
SDK=userland/libs/bouchaud-sdk/build-program.sh

rm -rf "$OUT/bin"
mkdir -p "$OUT/bin"
python3 tools/ci/verifie_sdk_abi.py
python3 tools/verifie-contrat-abi-v1.py
"$SDK" "$OUT/bin/native-cycles-probe" userland/tests/native-cycles-probe.c
"$SDK" "$OUT/bin/native-process-probe" userland/tests/native-process-probe.c
"$SDK" "$OUT/bin/healthd" userland/services/healthd/healthd.c
"$SDK" "$OUT/bin/serviced" userland/services/serviced/serviced.c
"$SDK" "$OUT/bin/logd" userland/services/logd/logd.c

python3 tools/native/make_native_image.py --image "$OUT/native-services.img" \
  --bin native-cycles-probe="$OUT/bin/native-cycles-probe" \
  --bin native-process-probe="$OUT/bin/native-process-probe" \
  --bin healthd="$OUT/bin/healthd" \
  --bin serviced="$OUT/bin/serviced" \
  --bin logd="$OUT/bin/logd" \
  --autorun "echo NATIVE_SERVICES_BEGIN" \
  --autorun "/bin/native-cycles-probe" \
  --autorun "/bin/native-process-probe" \
  --autorun "/bin/serviced qualify" \
  --autorun "/bin/serviced logd-qualify" \
  --autorun "ipc-attribution" \
  --autorun "echo NATIVE_SERVICES_END"

ACCEL=()
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then ACCEL=(-enable-kvm -cpu host); else ACCEL=(-cpu max); fi

: > "$LOG"
qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$OUT/native-services.img" \
  -m 2048 -smp "$SMP" "${ACCEL[@]}" \
  -display none -no-reboot \
  -netdev user,id=net0 -device e1000,netdev=net0 \
  -serial file:"$LOG" &
QEMU_PID=$!
cleanup() {
  kill -TERM "$QEMU_PID" 2>/dev/null || true
  sleep 0.5
  kill -KILL "$QEMU_PID" 2>/dev/null || true
  wait "$QEMU_PID" 2>/dev/null || true
}
trap cleanup EXIT

fatal_re='\*\*\* KERNEL PANIC \*\*\*|DOUBLE FAULT|TRIPLE FAULT|SpinLock recursive acquisition|INVARIANT_VIOLATION|SERVICED_QUALIFY_FAIL|LOGD_QUALIFY_FAIL|\[NATIVE-CYCLES\] FAIL|\[NATIVE-PROCESS\] FAIL'
deadline=$((SECONDS + ${BO_SERVICES_DELAI_S:-240}))
while :; do
  if grep -aEq "$fatal_re" "$LOG"; then
    echo "NATIVE_SERVICES_FAIL raison=fatal_ou_verdict" >&2
    sed 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -aE "$fatal_re|\[SERVICED\]|NATIVE-CYCLES" | tail -60 >&2
    exit 1
  fi
  grep -aFq 'NATIVE_SERVICES_END' "$LOG" && break
  if ! kill -0 "$QEMU_PID" 2>/dev/null; then
    sleep 0.2
    grep -aFq 'NATIVE_SERVICES_END' "$LOG" && break
    echo "NATIVE_SERVICES_FAIL raison=qemu_arrete_avant_fin" >&2
    sed 's/\x1b\[[0-9;]*m//g' "$LOG" | tail -60 >&2
    exit 1
  fi
  if (( SECONDS >= deadline )); then
    echo "NATIVE_SERVICES_FAIL raison=echeance" >&2
    sed 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -a "SERVICED\|NATIVE" | tail -60 >&2
    exit 1
  fi
  sleep 0.3
done
cleanup
trap - EXIT

required=(
  '[NATIVE-CYCLES] OK'
  '[NATIVE-PROCESS] OK'
  '[SERVICED] ok   no zombie left (wait4(-1) = ECHILD)'
  '[SERVICED] ok   health reaches Running'
  '[SERVICED] ok   echo was not started before health was Running'
  '[SERVICED] ok   ipc rtt measured (200 round trips)'
  '[SERVICED] ok   crashed service detected and restarting'
  '[SERVICED] ok   crash exit code is a fault, not a clean exit'
  '[SERVICED] ok   crashed service back to Running'
  '[SERVICED] ok   no handle leak across a crash/restart'
  '[SERVICED] ok   hung service killed and restarting'
  '[SERVICED] ok   crash budget spent -> Failed (no restart storm)'
  '[SERVICED] ok   every handle released at shutdown'
  'SERVICED_QUALIFY_OK'
  '[SERVICED] ok   producer never blocked: every send accepted or refused'
  '[SERVICED] ok   backpressure observed (intake queue is bounded)'
  '[SERVICED] ok   no silent loss: logd received exactly what was accepted'
  '[SERVICED] ok   malformed lines are counted, not stored'
  '[SERVICED] ok   ring bounded and overwriting: overwritten = received - 256'
  'LOGD_QUALIFY_OK'
)
for marker in "${required[@]}"; do
  if ! grep -aFq "$marker" "$LOG"; then
    echo "NATIVE_SERVICES_FAIL marqueur_absent=$marker" >&2
    exit 1
  fi
done
# Every death the supervisor saw must have had its channel already closed.
# Counted with awk over the whole file: no early-exiting reader under
# pipefail (grep -q would SIGPIPE the producer and turn a violation green).
morts=$(sed 's/\x1b\[[0-9;]*m//g' "$LOG" | awk '/\[SERVICED\] svc=.* death / { n++ } END { print n + 0 }')
violations=$(sed 's/\x1b\[[0-9;]*m//g' "$LOG" | awk '/\[SERVICED\] svc=.* death / && !/peer_closed=1/ { n++ } END { print n + 0 }')
if [ "$morts" -lt 5 ] || [ "$violations" -ne 0 ]; then
  echo "NATIVE_SERVICES_FAIL raison=mort_sans_peer_closed morts=$morts violations=$violations" >&2
  exit 1
fi
sed 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -aoE 'SERVICED_(IPC_RTT_NS|IPC_ALLER_NS|IPC_RETOUR_NS|RECOVERY_MS|HANDLES|RESTARTS).*|LOGD_QUALIFY_(COUNTS|HANDLES).*|svc=[a-z]+ start_latency ms=[0-9]+' | sed 's/^/  /'
# BOUCHAUD_IPC_ATTRIBUTION_V1 : where each native wakeup spent its time
# (signal -> run queue -> election -> back in the syscall). Published, not
# thresholded; the attribution must exist before any scheduler change.
attribution=$(sed 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -aoE 'NATIVE_IPC_ATTRIBUTION(_COMPTES)? .*' || true)
if ! grep -q 'etape=total' <<< "$attribution"; then
  echo "NATIVE_SERVICES_FAIL raison=attribution_ipc_absente" >&2
  exit 1
fi
sed 's/^/  /' <<< "$attribution"
echo "NATIVE_SERVICES_OK smp=$SMP accel=$([ "${BO_QEMU_KVM:-0}" = 1 ] && echo kvm || echo tcg)"
