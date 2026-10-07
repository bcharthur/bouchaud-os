#!/usr/bin/env bash
# Isolation des CADRES : un iframe d'un autre site tourne dans un autre
# WebContent (BOUCHAUD_OOPIF_V1, convergence P7).
#
#   tools/ci/run_ladybird_oopif.sh BOOTIMAGE NATIVE_DIR
#
# UI/Bouchaud transmet BOUCHAUD_SITE_ISOLATION a l'option upstream
# --site-isolation (main.cpp ; `top-level` par defaut). Une session invitee,
# deux passages sans fenetre de la meme page (oopif-a.html sur 10.0.2.2,
# un cadre de 10.0.2.100) :
#   iframe     EXIGE : le cadre dans un AUTRE WebContent que le parent,
#              postMessage dans les deux sens (3 messages, 3 echos), le
#              document du cadre inaccessible au parent, des trames presentees,
#              aucune assertion ni panique ;
#   top-level  TEMOIN mesure (mode par defaut) : le cadre partage le
#              WebContent du parent ; imprime, n'echoue pas.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_ladybird_oopif.sh BOOTIMAGE NATIVE_DIR}
OUT=${2:?usage: run_ladybird_oopif.sh BOOTIMAGE NATIVE_DIR}
DUREE=${BO_OOPIF_DUREE_S:-45}

SCENARIO=scenario-oopif
IMAGE=ladybird-oopif.img
LOG=serie-oopif.log
rm -rf "$SCENARIO" "$IMAGE" "$LOG" "$LOG.propre" fixture-oopif.log

python3 tools/health/browser_host_fixture.py > fixture-oopif.log 2>&1 &
FIXTURE=$!
trap 'kill "$FIXTURE" 2>/dev/null || true' EXIT
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

# Sans bureau : le navigateur tourne sans fenetre, le Compositor peint quand
# meme, et BOUCHAUD_LB_BANC_DUREE_S le fait quitter. L'autorun n'est pas un
# shell complet : pas de commentaire dans le texte qui suit.
{
  echo "export BOUCHAUD_BROWSER_HOST=1"
  echo "export BOUCHAUD_TIME_ZONE=Europe/Paris"
  echo "export BOUCHAUD_LB_BANC_DUREE_S=$DUREE"
  echo "export BOUCHAUD_M9_URL='http://10.0.2.2:18082/oopif-a.html'"
  for mode in iframe top-level; do
    echo "echo OOPIF_DEBUT mode=$mode"
    echo "export BOUCHAUD_SITE_ISOLATION=$mode"
    echo "/bo-navigateur"
    echo "echo OOPIF_FIN mode=$mode statut=\$?"
  done
  echo "echo OOPIF_BANC_FIN"
} > "$SCENARIO/autorun"
(cd tools/userland && IMAGE="$PWD/../../$IMAGE" ./mkdisk.sh "$PWD/../../$SCENARIO" >/dev/null)

: > "$LOG"
qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$IMAGE" \
  -m 8192 -smp 4 -cpu max -display none -no-reboot \
  -netdev "user,id=net0,guestfwd=tcp:10.0.2.100:18082-cmd:nc 127.0.0.1 18082" -device e1000,netdev=net0 \
  -audiodev none,id=muet -device AC97,audiodev=muet \
  -serial file:"$LOG" &
PID=$!
LIMITE=$((SECONDS + 2 * DUREE + 600))
while kill -0 "$PID" 2>/dev/null; do
  if (( SECONDS >= LIMITE )); then echo "ECHEANCE atteinte" >&2; break; fi
  grep -aq 'OOPIF_BANC_FIN\|KERNEL PANIC' "$LOG" && { sleep 2; break; }
  sleep 2
done
kill -TERM "$PID" 2>/dev/null || true
sleep 1
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true

sed -E 's/\x1b\[[0-9;]*m//g; s/^\[[^]]*\]\[[^]]*\]\[FPS:[^]]*\] //' "$LOG" | tr -d '\r' > "$LOG.propre"
P="$LOG.propre"

passage() { awk -v m="$1" '$0 ~ "OOPIF_DEBUT mode="m"$" {on=1} on {print} $0 ~ "OOPIF_FIN mode="m" " {on=0}' "$P"; }
pid_js() { echo "$1" | grep -aoE "WebContent\\(([0-9]+)\\): \\(js log\\) \"$2" | head -1 | grep -oE '[0-9]+' | head -1 || true; }

echecs=()
exige() { local quoi=$1; shift; if "$@"; then echo "  ok      $quoi"; else echo "  ECHEC   $quoi"; echecs+=("$quoi"); fi; }
for mode in iframe top-level; do
  bloc=$(passage "$mode")
  pa=$(pid_js "$bloc" 'HOST_OOPIF_A origine=')
  pe=$(pid_js "$bloc" 'HOST_OOPIF_ENFANT origine=')
  fin=$(echo "$bloc" | grep -aoE 'HOST_OOPIF_A_FIN recus=[0-9]+ echos=[0-9]+' | head -1 || true)
  recus=$(echo "$fin" | grep -oE 'recus=[0-9]+' | cut -d= -f2 || true)
  echos=$(echo "$fin" | grep -oE 'echos=[0-9]+' | cut -d= -f2 || true)
  acces=$(echo "$bloc" | grep -aoE 'acces_document_enfant=[A-Za-z]+' | head -1 | cut -d= -f2 || true)
  trames=$(echo "$bloc" | grep -ac '\[LB\] PRESENT onglet=' || true)
  dernier=$(echo "$bloc" | grep -aoE '\[LB\] PRESENT_DERNIER .*' | tail -1 || true)
  couleurs=$(echo "$dernier" | grep -oE 'couleurs=[0-9]+' | cut -d= -f2 || true)
  wc_pids=$(echo "$bloc" | grep -aoE '\[LB\] PROCESS_CREATE type=WebContent pid=[0-9]+' | grep -oE '[0-9]+$' | tr '\n' ' ' || true)
  assertions=$(echo "$bloc" | grep -acE 'VERIFICATION FAILED|KERNEL PANIC|COMPOSITOR_LINK_GIVE_UP' || true)
  echo "OOPIF mode=$mode pid_parent=${pa:-?} pid_cadre=${pe:-?} webcontents_crees=[${wc_pids}] recus=${recus:-?} echos=${echos:-?} acces_document=${acces:-?} trames=$trames couleurs=${couleurs:-?} assertions=$assertions"
  echo "$bloc" | grep -aE 'OOPIF_FIN|BROWSER_HOST_EXIT|VERIFICATION FAILED|PROCESS_FAULT|Unable to|ASSERTION' | awk 'NR <= 6 { print "    " $0 }' || true
  if [ "$mode" = iframe ]; then
    echo "== verdict (isolation des cadres) =="
    exige "le parent a tourne (pid ${pa:-?})" test -n "$pa"
    exige "le cadre de l'autre site a tourne (pid ${pe:-?})" test -n "$pe"
    exige "cadre et parent dans des WebContent differents" test -n "$pa" -a -n "$pe" -a "${pa:-x}" != "${pe:-x}"
    exige "postMessage cadre -> parent (3 attendus, ${recus:-0})" test "${recus:-0}" -ge 3
    exige "postMessage parent -> cadre -> parent (3 attendus, ${echos:-0})" test "${echos:-0}" -ge 3
    exige "le document du cadre reste inaccessible au parent (${acces:-?})" test "${acces:-}" = refuse
    exige "trames presentees ($trames)" test "$trames" -ge 1
    exige "aucune assertion, panique ni abandon de lien ($assertions)" test "$assertions" -eq 0
  fi
done

if [ ${#echecs[@]} -eq 0 ]; then
  echo "LADYBIRD_OOPIF_OK"
else
  echo "LADYBIRD_OOPIF_ECHEC n=${#echecs[@]}"
  exit 1
fi
