#!/usr/bin/env bash
# Cycle de vie des workers au-dela de la batterie (BOUCHAUD_CYCLE_WORKER_V1,
# convergence P4).
#
#   tools/ci/run_ladybird_worker_cycle.sh BOOTIMAGE NATIVE_DIR
#
# Sans fenetre, une page (worker-cycle.html) :
#   1. lance w1, mesure son premier message (demarrage a froid d'un worker) ;
#   2. prend le titre BOUCHAUD_BANC_CRASH_WORKER : UI/Bouchaud envoie SIGSEGV
#      au dernier processus WebWorker du gestionnaire d'upstream ;
#   3. lance w2 (le navigateur doit avoir survecu), puis w3, et navigue
#      PENDANT que w3 vit ;
#   4. sur worker-cycle-2.html, lance w4 (worker APRES navigation), puis prend
#      le titre BOUCHAUD_BANC_QUITTE : le navigateur quitte AVEC w4 vivant.
#
# Exige : processus du worker reellement tue (noyau) et recolte par le
# navigateur, plus aucun message de w1 entre 6 et 10 s apres, w2/w3/w4
# demarres, au plus UN processus WebWorker vivant (w4) au moment de quitter,
# sortie du navigateur en code 0, et TOUS les processus Ladybird lances sont
# sortis (noyau : autant de PROCESS_EXIT que de PERF_EXECVE). Aucune
# assertion, aucune panique.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_ladybird_worker_cycle.sh BOOTIMAGE NATIVE_DIR}
OUT=${2:?usage: run_ladybird_worker_cycle.sh BOOTIMAGE NATIVE_DIR}

SCENARIO=scenario-worker-cycle
IMAGE=ladybird-worker-cycle.img
LOG=serie-worker-cycle.log
rm -rf "$SCENARIO" "$IMAGE" "$LOG" "$LOG.propre" fixture-worker-cycle.log

python3 tools/health/browser_host_fixture.py > fixture-worker-cycle.log 2>&1 &
FIXTURE=$!
# Panique noyau : son contexte en DERNIER (tools/ci/extrait_panique.sh).
trap 'kill "$FIXTURE" 2>/dev/null || true; tools/ci/extrait_panique.sh "$LOG" "$OUT"' EXIT
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

# BOUCHAUD_LB_BANC_DUREE_S n'est qu'un filet : la page fait quitter le
# navigateur bien avant ; s'il sert, les jalons manquants le diront.
cat > "$SCENARIO/autorun" <<AUTORUN
export BOUCHAUD_BROWSER_HOST=1
export BOUCHAUD_TIME_ZONE=Europe/Paris
export BOUCHAUD_LB_BANC_QUITTE=1
export BOUCHAUD_LB_BANC_CRASH_WORKER=1
export BOUCHAUD_LB_BANC_DUREE_S=400
export BOUCHAUD_M9_URL='http://10.0.2.2:18082/worker-cycle.html'
echo WCYCLE_DEBUT
/bo-navigateur
echo WCYCLE_SORTI statut=\$?
echo WCYCLE_BANC_FIN
AUTORUN
(cd tools/userland && IMAGE="$PWD/../../$IMAGE" ./mkdisk.sh "$PWD/../../$SCENARIO" >/dev/null)

: > "$LOG"
qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$IMAGE" \
  -m 8192 -smp 4 -cpu max -display none -no-reboot \
  -netdev user,id=net0 -device e1000,netdev=net0 \
  -audiodev none,id=muet -device AC97,audiodev=muet \
  -serial file:"$LOG" &
PID=$!
LIMITE=$((SECONDS + 1000))
while kill -0 "$PID" 2>/dev/null; do
  if (( SECONDS >= LIMITE )); then echo "ECHEANCE atteinte" >&2; break; fi
  grep -aq 'WCYCLE_BANC_FIN\|KERNEL PANIC' "$LOG" && { sleep 2; break; }
  sleep 2
done
kill -TERM "$PID" 2>/dev/null || true
sleep 1
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true

sed -E 's/\x1b\[[0-9;]*m//g; s/^\[[^]]*\]\[[^]]*\]\[FPS:[^]]*\] //' "$LOG" | tr -d '\r' > "$LOG.propre"
P="$LOG.propre"

echo "== chronologie =="
grep -anE 'HOST_WCYCLE2?|WORKER_CRASH_REQUEST|\[LB\] PROCESS_(CREATE|EXIT) type=WebWorker|PROCESS_(EXIT|DEATH|FAULT) .*WebWorker|BROWSER_HOST_EXIT|WCYCLE_(DEBUT|SORTI)|VERIFICATION FAILED|KERNEL PANIC' "$P" \
  | sed -E 's/^([0-9]+):.*(HOST_|\[LB|PROCESS_|BROWSER_HOST|WCYCLE|VERIFICATION|KERNEL)/\1: \2/' | awk 'NR <= 60 { print "  " substr($0, 1, 200) }' || true

ligne_de() { grep -anE "$1" "$P" | head -1 | cut -d: -f1 || true; }
champ() { echo "$1" | grep -oE "$2=[0-9a-z]+" | head -1 | cut -d= -f2 || true; }
w1=$(grep -aoE 'HOST_WCYCLE w1_vivant ms=[0-9]+' "$P" | head -1 || true)
demande=$(grep -aoE '\[LB\] WORKER_CRASH_REQUEST onglet=[0-9]+ pid=[0-9]+ workers_vivants=[0-9]+ kill=[a-z]+' "$P" | head -1 || true)
pid_w=$(champ "$demande" pid)
mort_noyau=$(grep -aoE "PROCESS_(EXIT|DEATH) t=[0-9]+ pid=${pid_w:-x} ppid=[0-9]+ image=/usr/libexec/ladybird/WebWorker code=[-0-9]+" "$P" | head -1 || true)
recolte=$(grep -aoE "\\[LB\\] PROCESS_EXIT type=WebWorker pid=${pid_w:-x} " "$P" | head -1 || true)
crash=$(grep -aoE 'HOST_WCYCLE crash recus_avant=[0-9]+ recus_6s=[0-9]+ recus_10s=[0-9]+ erreur=[a-z]+' "$P" | head -1 || true)
r6=$(champ "$crash" recus_6s); r10=$(champ "$crash" recus_10s)
w2=$(grep -aoE 'HOST_WCYCLE w2_apres_crash ok ms=[0-9]+' "$P" | head -1 || true)
w3=$(grep -aoE 'HOST_WCYCLE w3_vivant ms=[0-9]+' "$P" | head -1 || true)
w4=$(grep -aoE 'HOST_WCYCLE2 w4_apres_navigation ok ms=[0-9]+' "$P" | head -1 || true)
l_quitte=$(ligne_de 'HOST_WCYCLE2 quitte_avec_worker_vivant')
vivants=$(awk -v l="${l_quitte:-0}" 'NR < l && /\[LB\] PROCESS_CREATE type=WebWorker / { c++ } NR < l && /\[LB\] PROCESS_EXIT type=WebWorker / { e++ } END { print c - e }' "$P")
crees=$(grep -ac '\[LB\] PROCESS_CREATE type=WebWorker ' "$P" || true)
# Vue du NOYAU, independante de la recolte par le navigateur : combien de
# processus WebWorker sont REELLEMENT sortis avant que la page ne quitte.
noyau_sortis=$(awk -v l="${l_quitte:-0}" 'NR < l && /PROCESS_EXIT t=[0-9]+ pid=[0-9]+ ppid=[0-9]+ image=\/usr\/libexec\/ladybird\/WebWorker / { n++ } END { print n + 0 }' "$P")
# Noyau : chaque image Ladybird lancee doit etre sortie quand l'autorun reprend la main.
l_sorti=$(ligne_de 'WCYCLE_SORTI statut=')
lances=$(awk -v l="${l_sorti:-999999999}" 'NR < l && match($0, /PERF_EXECVE .*image=\/usr\/libexec\/ladybird\/[A-Za-z]+ pid=[0-9]+/) { s = substr($0, RSTART, RLENGTH); sub(/.*pid=/, "", s); vu[s] = 1 } END { for (k in vu) n++; print n + 0 }' "$P")
sortis=$(awk -v l="${l_sorti:-999999999}" 'NR < l && /PROCESS_EXIT t=[0-9]+ pid=[0-9]+ ppid=[0-9]+ image=\/usr\/libexec\/ladybird\// && match($0, / pid=[0-9]+/) { vu[substr($0, RSTART + 5, RLENGTH - 5)] = 1 } END { for (k in vu) n++; print n + 0 }' "$P")
statut=$(grep -aoE 'WCYCLE_SORTI statut=[0-9]+' "$P" | head -1 | cut -d= -f2 || true)
echo "WCYCLE_MESURE demarrage_w1_ms=$(champ "$w1" ms) pid_tue=${pid_w:-?} workers_vivants_au_crash=$(champ "$demande" workers_vivants) recus_6s=${r6:-?} recus_10s=${r10:-?} erreur_w1=$(champ "$crash" erreur) w2_ms=$(champ "$w2" ms) w4_ms=$(champ "$w4" ms) webworkers_crees=$crees vivants_a_la_sortie=$vivants webworkers_sortis_noyau_avant_sortie=$noyau_sortis ladybird_lances=$lances ladybird_sortis=$sortis statut=${statut:-?}"

echo "== verdict =="
echecs=()
exige() { local quoi=$1; shift; if "$@"; then echo "  ok      $quoi"; else echo "  ECHEC   $quoi"; echecs+=("$quoi"); fi; }
dans() { grep -aqE "$1" "$P"; }
exige "w1 a demarre" test -n "$w1"
exige "le processus du worker a ete vise (kill=ok, pid ${pid_w:-?})" test -n "$pid_w" -a "$(champ "$demande" kill)" = ok
exige "le noyau l'a vu mourir (${mort_noyau:-rien})" test -n "$mort_noyau"
exige "le navigateur l'a recolte ([LB] PROCESS_EXIT)" test -n "$recolte"
exige "w1 ne parle plus entre 6 et 10 s apres (${r6:-?} -> ${r10:-?})" test -n "$r6" -a "${r6:-x}" = "${r10:-y}"
exige "w2 demarre apres le crash (navigateur vivant)" test -n "$w2"
exige "w3 demarre, puis navigation pendant qu'il vit" dans 'HOST_WCYCLE navigation_avec_worker_vivant'
exige "w4 demarre apres la navigation" test -n "$w4"
exige "au plus un WebWorker vivant a la sortie de page (${vivants})" test "$vivants" -le 1
exige "le navigateur quitte avec un worker vivant, code 0" dans 'BROWSER_HOST_EXIT boucle_quittee code=0'
exige "l'autorun reprend la main (statut ${statut:-?})" test "${statut:-x}" = 0
exige "tous les processus Ladybird lances sont sortis (${sortis}/${lances})" test "$lances" -ge 1 -a "$sortis" -eq "$lances"
exige "aucune assertion ni panique" bash -c "! grep -aqE 'VERIFICATION FAILED|KERNEL PANIC' '$P'"

if [ ${#echecs[@]} -eq 0 ]; then
  echo "LADYBIRD_WORKER_CYCLE_OK demarrage_w1_ms=$(champ "$w1" ms)"
else
  echo "LADYBIRD_WORKER_CYCLE_ECHEC n=${#echecs[@]}"
  exit 1
fi
