#!/usr/bin/env bash
# MEMOIRE APRES DES ONGLETS FERMES (BOUCHAUD_MEMOIRE_ONGLETS_V1, P2/P9).
#
#   tools/ci/run_ladybird_memoire.sh BOOTIMAGE NATIVE_DIR [N_PAR_PHASE]
#
# Le RSS du Compositor monte de 38 a 102 Mio en 10 min d'endurance (run
# 37667817559). Fuite, ou cache borne ? Ce banc tient tout le reste constant :
# deux phases IDENTIQUES de N onglets autre site (ouverture, relais, nouveau
# WebContent, fermeture), chacune suivie de 20 s de stabilisation, reperes M0,
# M1, M2. tools/ci/analyse_memoire.py rend le verdict par processus (une fuite
# croit autant en phase 2 qu'en phase 1, un cache borne non) et, pour le
# Compositor, les objets vivants ([LB:MEM]) aux trois reperes.
#
# Verdict : stabilite (aucune panique, assertion, faute) ET memoire bornee.
set -euo pipefail
cd "$(dirname "$0")/../.."

BOOT=${1:?usage: run_ladybird_memoire.sh BOOTIMAGE NATIVE_DIR [N_PAR_PHASE]}
OUT=${2:?usage: run_ladybird_memoire.sh BOOTIMAGE NATIVE_DIR [N_PAR_PHASE]}
N=${3:-10}

SCENARIO=scenario-memoire
IMAGE=ladybird-memoire.img
LOG=serie-memoire.log
rm -rf "$SCENARIO" "$IMAGE" "$LOG" fixture-memoire.log

python3 tools/health/browser_host_fixture.py > fixture-memoire.log 2>&1 &
FIXTURE=$!
trap 'rc=$?; kill "$FIXTURE" 2>/dev/null || true; tools/ci/extrait_panique.sh "$LOG" "$OUT"; exit "$rc"' EXIT
sleep 1
kill -0 "$FIXTURE"

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

URL="http://10.0.2.2:18082/memoire.html?n=$N"
cat > "$SCENARIO/autorun" <<AUTORUN
echo "=== Bouchaud memoire ==="
export BO_AUTOSTART_BROWSER=1
export BOUCHAUD_M9=1
export BOUCHAUD_BROWSER_HOST=1
export BOUCHAUD_M11=1
export BOUCHAUD_TIME_ZONE=Europe/Paris
export BOUCHAUD_ALLOW_POPUPS=1
export BOUCHAUD_M9_URL='$URL'
desktop
AUTORUN
(cd tools/userland && IMAGE="$PWD/../../$IMAGE" ./mkdisk.sh "$PWD/../../$SCENARIO" >/dev/null)

: > "$LOG"
ACCEL="-cpu max"
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
fi
echo "MEMOIRE_ACCEL ${ACCEL} n_par_phase=$N"
qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$IMAGE" \
  -m 8192 -smp 4 $ACCEL -display none -no-reboot \
  -netdev "user,id=net0,guestfwd=tcp:10.0.2.100:18082-cmd:nc 127.0.0.1 18082" -device e1000,netdev=net0 \
  -audiodev none,id=muet -device AC97,audiodev=muet \
  -serial file:"$LOG" &
PID=$!

# Plafond : 2 phases x N onglets x (garde 15 s + 1 s) + 2 x 20 s + demarrage,
# large. Le banc s'arrete des que la page a fini.
PLAFOND=$(( 2 * N * 16 + 40 + 600 ))
DEBUT=$SECONDS
verdict=inconnu
while kill -0 "$PID" 2>/dev/null; do
  if grep -aq 'HOST_MEMOIRE_FIN' "$LOG"; then verdict=fini; break; fi
  if grep -aq 'KERNEL PANIC' "$LOG"; then verdict=panique; break; fi
  if (( SECONDS - DEBUT > PLAFOND )); then verdict=plafond; break; fi
  sleep 5
done
[ "$verdict" = inconnu ] && verdict=vm_morte
sleep 6   # un dernier [PERF-PROC] apres le repere M2
kill -TERM "$PID" 2>/dev/null || true
sleep 1
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
echo "MEMOIRE_BOUCLE $verdict duree_reelle_s=$((SECONDS - DEBUT))"

P="$LOG.propre"
sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | tr -d '\r' > "$P"
grep -aoE 'HOST_MEMOIRE_(REPERE|ONGLET) [^"]*' "$P" | awk 'NR <= 60' || true
swaps=$(grep -ac '\[LB\] PROCESS_SWAP onglet=[0-9]* raison=autre_site' "$P" || true)
echo "  changements de WebContent : $swaps pour $((2 * N)) onglets"
# BOUCHAUD_ECHANGE_PROCESSUS_V1 : la page laissee dans l'ancien processus
# a chaque changement est fermee, pas seulement oubliee par l'UI.
fermees=$(grep -ac '\[LB\] PROCESS_SWAP_CLOSE_OLD_PAGE' "$P" || true)
echo "  anciennes pages fermees au changement de processus : $fermees"

echo "== verdict =="
echecs=()
exige() { local quoi=$1; shift; if "$@"; then echo "  ok      $quoi"; else echo "  ECHEC   $quoi"; echecs+=("$quoi"); fi; }
exige "la page a fini (HOST_MEMOIRE_FIN), boucle sur ${verdict}" test "$verdict" = fini
exige "les onglets ont change de WebContent ($swaps / $((2 * N)))" test "$swaps" -ge $(( 2 * N - 2 ))
exige "aucune panique noyau" bash -c "! grep -aq 'KERNEL PANIC' '$P'"
exige "aucune assertion, aucun MUST() ni ASSERT en echec" bash -c "! grep -aqE 'VERIFICATION FAILED|UNEXPECTED ERROR|ASSERTION FAILED' '$P'"
exige "aucune faute de processus" bash -c "! grep -aq 'PROCESS_FAULT pid=' '$P'"
if python3 tools/ci/analyse_memoire.py "$LOG"; then
  echo "  ok      memoire bornee apres deux phases identiques"
else
  echo "  ECHEC   memoire : croissance ou reperes absents (voir ci-dessus)"
  echecs+=("memoire")
fi
if [ ${#echecs[@]} -eq 0 ]; then
  echo "LADYBIRD_MEMOIRE_OK n_par_phase=$N"
else
  echo "LADYBIRD_MEMOIRE_ECHEC n=${#echecs[@]}"
  exit 1
fi
