#!/usr/bin/env bash
# Politique de crash des services du navigateur (BOUCHAUD_CRASH_SERVICES_V1).
#
#   tools/ci/run_ladybird_crash_services.sh BOOTIMAGE NATIVE_DIR
#
# Politique attendue, service par service :
#   WebWorker     -- mort isolee            (run_ladybird_worker_cycle.sh)
#   WebContent    -- onglet repris          (run_ladybird_crash_rendu.sh)
#   ImageDecoder  -- le navigateur vit ; un nouveau decodeur, les images
#                    se decodent a nouveau ;
#   RequestServer -- reprise controlee : un nouveau RequestServer, chaque
#                    WebContent y est rebranche, le fetch remarche ;
#   Compositor    -- aucune corruption noyau ; upstream relance et rebranche
#                    (3 relances au plus, puis arret franc du navigateur --
#                    pas de boucle) ; il doit PRESENTER a nouveau.
#
# Une page (crash-services.html) verifie image + fetch, prend le titre
# BOUCHAUD_BANC_CRASH_SERVICE=<service> -- UI/Bouchaud envoie SIGSEGV au
# processus de ce type connu du gestionnaire d'upstream -- puis attend la
# reprise. Les trois crashs se suivent dans le MEME demarrage.
#
# Exige pour chacun : kill=ok, le noyau l'a vu mourir, le navigateur l'a
# recolte, UN remplacant (autre pid, une seule creation de plus : pas de
# boucle de relance), image et fetch reussis apres ; pour le Compositor, des
# trames presentees apres son remplacant. Puis : le navigateur quitte en code
# 0, aucune assertion, aucune panique, aucune autre faute de processus.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_ladybird_crash_services.sh BOOTIMAGE NATIVE_DIR}
OUT=${2:?usage: run_ladybird_crash_services.sh BOOTIMAGE NATIVE_DIR}
SERVICES=(ImageDecoder RequestServer Compositor)

SCENARIO=scenario-crash-services
IMAGE=ladybird-crash-services.img
LOG=serie-crash-services.log
rm -rf "$SCENARIO" "$IMAGE" "$LOG" "$LOG.propre" fixture-crash-services.log

python3 tools/health/browser_host_fixture.py > fixture-crash-services.log 2>&1 &
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
export BOUCHAUD_LB_BANC_CRASH_SERVICE=1
export BOUCHAUD_LB_BANC_DUREE_S=600
export BOUCHAUD_M9_URL='http://10.0.2.2:18082/crash-services.html'
echo CSERV_DEBUT
/bo-navigateur
echo CSERV_SORTI statut=\$?
echo CSERV_BANC_FIN
AUTORUN
(cd tools/userland && IMAGE="$PWD/../../$IMAGE" ./mkdisk.sh "$PWD/../../$SCENARIO" >/dev/null)

: > "$LOG"
ACCEL="-cpu max"
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
fi
echo "CSERV_ACCEL ${ACCEL}"
# shellcheck disable=SC2086
qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$IMAGE" \
  -m 8192 -smp 4 $ACCEL -display none -no-reboot \
  -netdev user,id=net0 -device e1000,netdev=net0 \
  -audiodev none,id=muet -device AC97,audiodev=muet \
  -serial file:"$LOG" &
PID=$!
LIMITE=$((SECONDS + 1200))
while kill -0 "$PID" 2>/dev/null; do
  if (( SECONDS >= LIMITE )); then echo "ECHEANCE atteinte" >&2; break; fi
  grep -aq 'CSERV_BANC_FIN\|KERNEL PANIC' "$LOG" && { sleep 2; break; }
  sleep 2
done
kill -TERM "$PID" 2>/dev/null || true
sleep 1
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true

sed -E 's/\x1b\[[0-9;]*m//g; s/^\[[^]]*\]\[[^]]*\]\[FPS:[^]]*\] //' "$LOG" | tr -d '\r' > "$LOG.propre"
P="$LOG.propre"

echo "== chronologie =="
grep -anE 'HOST_CRASH_SERVICE|SERVICE_(CRASH_REQUEST|RESTART|BILAN)|\[LB\] PROCESS_(CREATE|EXIT) type=(ImageDecoder|RequestServer|Compositor)|Compositor process died|PROCESS_(EXIT|DEATH|FAULT) .*ladybird/(ImageDecoder|RequestServer|Compositor)|BROWSER_HOST_EXIT|CSERV_(DEBUT|SORTI)|VERIFICATION FAILED|UNEXPECTED ERROR|KERNEL PANIC' "$P" \
  | awk 'NR <= 120 { print "  " substr($0, 1, 220) }' || true

champ() { echo "$1" | grep -oE "$2=[-0-9a-z]+" | head -1 | cut -d= -f2 || true; }

echo "== verdict =="
echecs=()
exige() { local quoi=$1; shift; if "$@"; then echo "  ok      $quoi"; else echo "  ECHEC   $quoi"; echecs+=("$quoi"); fi; }
tues=()
for s in "${SERVICES[@]}"; do
  demande=$(grep -aoE "\\[LB\\] SERVICE_CRASH_REQUEST onglet=[0-9]+ service=$s pid=[0-9]+ kill=[a-z]+" "$P" | head -1 || true)
  pid=$(champ "$demande" pid)
  tues+=("${pid:-x}")
  mort=$(grep -aoE "PROCESS_(EXIT|DEATH) t=[0-9]+ pid=${pid:-x} ppid=[0-9]+ image=/usr/libexec/ladybird/$s code=[-0-9]+" "$P" | head -1 || true)
  recolte=$(grep -aoE "\\[LB\\] PROCESS_EXIT type=$s pid=${pid:-x} " "$P" | head -1 || true)
  relance=$(grep -aoE "\\[LB\\] SERVICE_RESTART service=$s ancien_pid=[0-9]+ nouveau_pid=[0-9a-z]+ delai_ms=[0-9]+" "$P" | head -1 || true)
  nouveau=$(champ "$relance" nouveau_pid)
  creations=$(grep -acE "\\[LB\\] PROCESS_CREATE type=$s " "$P" || true)
  apres=$(grep -aoE "HOST_CRASH_SERVICE apres service=$s image=[a-z]+ reseau=[a-z]+ reprise_ms=[-0-9]+ raf_2s=[0-9]+" "$P" | head -1 || true)
  bilan=$(grep -aoE "\\[LB\\] SERVICE_BILAN service=$s nouveau_pid=[0-9]+ trames_avant_remplacant=[0-9]+ trames_apres_remplacant=[0-9]+" "$P" | head -1 || true)
  echo "CSERV_MESURE service=$s pid_tue=${pid:-?} nouveau_pid=${nouveau:-?} delai_relance_ms=$(champ "$relance" delai_ms) creations=$creations reprise_page_ms=$(champ "$apres" reprise_ms) raf_2s_apres=$(champ "$apres" raf_2s) trames_apres_remplacant=$(champ "$bilan" trames_apres_remplacant)"
  exige "$s : vise (kill=ok, pid ${pid:-?})" test -n "$pid" -a "$(champ "$demande" kill)" = ok
  exige "$s : le noyau l'a vu mourir" test -n "$mort"
  exige "$s : le navigateur l'a recolte ([LB] PROCESS_EXIT)" test -n "$recolte"
  exige "$s : un remplacant, autre pid (${nouveau:-aucun})" bash -c "[[ '${nouveau:-x}' =~ ^[0-9]+$ ]] && [ '${nouveau:-x}' != '${pid:-x}' ]"
  if [ "$s" = Compositor ]; then
    # P13 : les lignes [LB] PROCESS_CREATE peuvent etre perdues sur la
    # console serie pendant la relance. Utiliser les execve du NOYAU,
    # identifies par PID, sans tolerer une 3e relance silencieuse.
    execve_observes=$(grep -aoE "PERF_EXECVE t=[0-9]+ image=/usr/libexec/ladybird/Compositor pid=[0-9]+" "$P" | sed -E 's/^.* pid=([0-9]+)$/\1/' | sort -nu || true)
    execve_attendus=$(printf '%s\n' "$pid" "$nouveau" | grep -E '^[0-9]+$' | sort -nu || true)
    execve_n=$(printf '%s\n' "$execve_observes" | grep -cE '^[0-9]+$' || true)
    exige "Compositor : uniquement deux execve noyau, pid initial et remplacant" \
      test "$execve_observes" = "$execve_attendus" -a "$execve_n" -eq 2
  else
    exige "$s : une seule relance, pas de boucle ($creations creations)" test "$creations" -eq 2
  fi
  exige "$s : image et fetch reussis apres la reprise" test "$(champ "$apres" image)" = true -a "$(champ "$apres" reseau)" = true
  if [ "$s" = Compositor ]; then
    exige "Compositor : des trames presentees apres son remplacant ($(champ "$bilan" trames_apres_remplacant))" test "$(champ "$bilan" trames_apres_remplacant)" -gt 0 2>/dev/null
  fi
done
# Seules les fautes VOULUES : les pids tues.
autres=$(grep -aoE 'PROCESS_FAULT pid=[0-9]+' "$P" | cut -d= -f2 | sort -u | grep -vxF -f <(printf '%s\n' "${tues[@]}") || true)
exige "aucune autre faute de processus (${autres:-aucune})" test -z "$autres"
exige "la page a fini les trois" grep -aq 'HOST_CRASH_SERVICE fin' "$P"
exige "le navigateur quitte, code 0" grep -aq 'BROWSER_HOST_EXIT boucle_quittee code=0' "$P"
statut=$(grep -aoE 'CSERV_SORTI statut=[0-9]+' "$P" | head -1 | cut -d= -f2 || true)
exige "l'autorun reprend la main (statut ${statut:-?})" test "${statut:-x}" = 0
exige "aucune assertion, aucun MUST() en echec, aucune panique" bash -c "! grep -aqE 'VERIFICATION FAILED|UNEXPECTED ERROR|ASSERTION FAILED|KERNEL PANIC' '$P'"

if [ ${#echecs[@]} -eq 0 ]; then
  echo "LADYBIRD_CRASH_SERVICES_OK services=${SERVICES[*]}"
else
  echo "LADYBIRD_CRASH_SERVICES_ECHEC n=${#echecs[@]}"
  exit 1
fi
