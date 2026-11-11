#!/usr/bin/env bash
#
# BOUCHAUD_BUREAU_RING3_2B -- DOOM et Ladybird, deux applications ordinaires,
# composees ensemble par `composited` ring 3.
#
#   tools/ci/run_bureau_ring3_applications.sh BOOTIMAGE NATIVE_DIR
#
# NATIVE_DIR : l'artefact Ladybird (BouchaudBrowserHost et ses services). DOOM
# est construit par tools/doom/build-linux.sh s'il manque. Le noyau lance, au
# demarrage, trois clients BOGU INCHANGES, chacun dans son processus :
#   /doom (640x400), /bo-navigateur (Ladybird, page vivante /bureau.html) et
#   /mire-gpu (faute de page volontaire : le crash d'une application),
# puis `/bin/composited bureau` avec une capacite `Fenetre` par client et le
# scenario `deplace-empile` (DOOM glisse puis monte au premier plan).
#
# Exige :
#   - quatre processus ring 3 distincts (trois applications, le compositeur) ;
#   - le bureau noyau refuse (un seul compositeur) ;
#   - DOOM ET Ladybird reellement composes (trames par fenetre), Ladybird peint
#     sa page (HOST_BUREAU_PAGE, HOST_BUREAU_RAF) ;
#   - le scenario joue, et la capture qui suit jugee au pixel pres pour la
#     pile, les cadres et la mire, au contenu pour DOOM et Ladybird ;
#   - la mire meurt d'une faute de page ; DOOM et Ladybird continuent d'etre
#     composes ; aucune autre faute, aucune panique ;
#   - les mesures du compositeur (copie, presentation, cadence par fenetre).
# Code 0 : OK ; 1 : ECHEC. BO_QEMU_KVM=1 : sous KVM.
set -euo pipefail
cd "$(dirname "$0")/../.."

BOOT=${1:?usage: run_bureau_ring3_applications.sh BOOTIMAGE NATIVE_DIR}
NATIF=${2:?usage: run_bureau_ring3_applications.sh BOOTIMAGE NATIVE_DIR}
SORTIE=${BUREAU_SORTIE:-target/bureau-ring3-applications}
ACCEL="-cpu max"
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
fi
rm -rf "$SORTIE"; mkdir -p "$SORTIE/bin" "$SORTIE/scen/bin" "$SORTIE/scen/usr/libexec/ladybird" \
  "$SORTIE/scen/usr/share/ladybird" "$SORTIE/scen/etc/ssl/certs"
SORTIE=$(cd "$SORTIE" && pwd)
SOCKETS=$(mktemp -d "${TMPDIR:-/tmp}/bo-bureau-app.XXXXXX")
MONITEUR="$SOCKETS/moniteur.sock"
LOG="$SORTIE/serie.log"

echo "=== construction : composited, mire-gpu, attends, DOOM ==="
(cd userland/services/composited && OUT="$SORTIE/bin" bash ./build.sh >/dev/null)
musl-gcc -O2 -static-pie -fPIE -o "$SORTIE/bin/mire-gpu" tools/userland/mire-gpu.c
musl-gcc -O2 -static-pie -fPIE -o "$SORTIE/bin/attends" tools/userland/attends.c
[ -x target/doom-bouchaud/doom-bouchaud ] || tools/doom/build-linux.sh >/dev/null

S="$SORTIE/scen"
cp "$SORTIE/bin/composited-slice" "$S/bin/composited"
cp "$SORTIE/bin/attends" "$S/bin/attends"
cp "$SORTIE/bin/mire-gpu" "$S/mire-gpu"
cp target/doom-bouchaud/doom-bouchaud "$S/doom"
cp tools/doom/wads/freedoom1.wad "$S/"
for f in BouchaudBrowserHost WebContent RequestServer ImageDecoder WebWorker Compositor; do
  cp "$NATIF/$f" "$S/usr/libexec/ladybird/$f"
done
cp "$NATIF/BouchaudBrowserHost" "$S/bo-navigateur"
[ -d "$NATIF/resources" ] && cp -a "$NATIF/resources/." "$S/usr/share/ladybird/"
cp /etc/ssl/certs/ca-certificates.crt "$S/etc/ssl/certs/ca-certificates.crt"
chmod 755 "$S/bin/"* "$S/mire-gpu" "$S/doom" "$S/bo-navigateur" "$S/usr/libexec/ladybird/"*
printf '%s\n' '@scenario deplace-empile' '/doom 640x400' \
  '/bo-navigateur 720x420 BOUCHAUD_M9=1 BOUCHAUD_BROWSER_HOST=1 BOUCHAUD_M11=1 BOUCHAUD_TIME_ZONE=Europe/Paris BOUCHAUD_M9_URL=http://10.0.2.2:18082/bureau.html' \
  "/mire-gpu BO_MIRE_FAUTE_APRES=${BO_BUREAU_FAUTE_APRES:-2400}" > "$S/bureau-ring3"
: > "$S/garde-vm-vivante"
printf '%s\n' 'echo BUREAU_RING3_AUTORUN_BEGIN' 'desktop' '/bin/attends 900' > "$S/autorun"
(cd tools/userland && IMAGE="$SORTIE/disque.img" ./mkdisk.sh "$S" >/dev/null)

python3 tools/health/browser_host_fixture.py > "$SORTIE/fixture.log" 2>&1 &
FIXTURE=$!
QEMU_PID=""
arrete() {
  [ -n "$QEMU_PID" ] && { kill -TERM "$QEMU_PID" 2>/dev/null || true; sleep 1; kill -KILL "$QEMU_PID" 2>/dev/null || true; wait "$QEMU_PID" 2>/dev/null || true; }
  QEMU_PID=""
}
trap 'arrete; kill "$FIXTURE" 2>/dev/null || true; rm -rf "$SOCKETS"' EXIT
sleep 1

capture() {
  python3 - "$MONITEUR" "$1" <<'PY' || true
import socket, sys, time
s = socket.socket(socket.AF_UNIX)
s.settimeout(5)
s.connect(sys.argv[1])
s.sendall(f"screendump {sys.argv[2]}\n".encode())
time.sleep(1.0)
s.close()
PY
}

echo "=== DOOM + Ladybird + mire sous composited ==="
: > "$LOG"
# shellcheck disable=SC2086
qemu-system-x86_64 $ACCEL -drive format=raw,file="$BOOT" -drive format=raw,file="$SORTIE/disque.img" \
  -m 8192 -smp 4 -display none -no-reboot \
  -netdev "user,id=net0,guestfwd=tcp:10.0.2.100:18082-cmd:nc 127.0.0.1 18082" -device e1000,netdev=net0 \
  -audiodev none,id=muet -device AC97,audiodev=muet \
  -monitor "unix:$MONITEUR,server,nowait" -serial file:"$LOG" &
QEMU_PID=$!

fatal_re='KERNEL PANIC|panicked at|DOUBLE FAULT|TRIPLE FAULT|SpinLock recursive acquisition'
scenario=0; mort=0; apres_mort=""
LIMITE=$((SECONDS + ${BO_BUREAU_PLAFOND_S:-900}))
while kill -0 "$QEMU_PID" 2>/dev/null; do
  if grep -aqE "$fatal_re" "$LOG"; then break; fi
  if [ "$scenario" = 0 ] && grep -aq 'COMPOSITED_BUREAU_SCENARIO_FIN fenetre=1' "$LOG"; then
    sleep 2; capture "$SORTIE/scenario.ppm"; scenario=1
  fi
  if [ "$mort" = 0 ] && grep -aq 'COMPOSITED_BUREAU_CLIENT_MORT' "$LOG"; then
    mort=1; apres_mort=$((SECONDS + 20))
  fi
  # Vingt secondes apres la mort de la mire : DOOM et Ladybird ont-ils
  # continue d'etre composes ?
  if [ -n "$apres_mort" ] && (( SECONDS >= apres_mort )); then
    capture "$SORTIE/apres-mort.ppm"; break
  fi
  if (( SECONDS >= LIMITE )); then echo "ECHEANCE bureau ring 3 applications" >&2; break; fi
  sleep 1
done
arrete
P="$SORTIE/serie.propre"
sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | tr -d '\r' > "$P"
grep -aE 'BUREAU_RING3|COMPOSITED_BUREAU_(PRET|FENETRE|SCENARIO|CLIENT|TRAMES|REFUS)|HOST_BUREAU|PROCESS_(FAULT|DEATH) |BUREAU_NOYAU' "$P" \
  | cut -c1-200 | tail -n 60 || true

echecs=0
echec() { echo "  ECHEC   $1"; echecs=$((echecs + 1)); }
ok() { echo "  ok      $1"; }
pid_de() { grep -aoE "BOUCHAUD_BUREAU_RING3_CLIENT pid=[0-9]+ chemin=$1 " "$P" | grep -oE 'pid=[0-9]+' | sed -n '1s/pid=//p' || true; }

grep -aqE "$fatal_re" "$P" && echec "panique noyau" || ok "aucune panique noyau"
doom=$(pid_de /doom); nav=$(pid_de /bo-navigateur); mire=$(pid_de /mire-gpu)
compo=$(grep -aoE 'BOUCHAUD_COMPOSITEUR_LANCE pid=[0-9]+' "$P" | grep -oE '[0-9]+$' | sed -n 1p || true)
if [ -n "$doom" ] && [ -n "$nav" ] && [ -n "$mire" ] && [ -n "$compo" ] \
   && [ "$(printf '%s\n' "$doom" "$nav" "$mire" "$compo" | sort -u | wc -l)" = 4 ]; then
  ok "quatre processus ring 3 : DOOM $doom, Ladybird $nav, mire $mire, compositeur $compo"
else
  echec "processus distincts (doom=${doom:-?} ladybird=${nav:-?} mire=${mire:-?} compositeur=${compo:-?})"
fi
grep -aq 'BOUCHAUD_BUREAU_NOYAU_REFUSE raison=ecran-au-compositeur' "$P" \
  && ok "bureau noyau refuse : un seul compositeur" || echec "bureau noyau non refuse"
grep -aq 'COMPOSITED_BUREAU_REFUS_OK' "$P" && ok "refus de securite du pont" || echec "refus de securite du pont"
grep -aq 'HOST_BUREAU_PAGE OK' "$P" && grep -aq 'HOST_BUREAU_RAF n=' "$P" \
  && ok "Ladybird charge et anime sa page" || echec "page Ladybird non chargee ou figee"

# Trames composees par fenetre (dernier releve avant la mort, et dernier tout court).
trames() {  # PID [MARQUEUR_AVANT]
  awk -v m="${2:-}" 'm != "" && index($0, m) { fini = 1 } !fini { print }' "$P" \
    | grep -aoE "COMPOSITED_BUREAU_TRAMES fenetre=[0-9]+ pid=$1 trames=[0-9]+" | sed -n '$s/.*trames=//p' || true
}
for paire in "DOOM:$doom" "Ladybird:$nav"; do
  nom=${paire%%:*}; pid=${paire#*:}
  n=$(trames "$pid"); n=${n:-0}
  (( n >= 100 )) && ok "$nom compose par composited ($n trames)" || echec "$nom : $n trame(s) composee(s)"
done
grep -aq 'COMPOSITED_BUREAU_SCENARIO_FIN fenetre=1' "$P" && ok "deplacement puis passage au premier plan joues" \
  || echec "scenario non joue"
juge() {  # FICHIER MARQUEUR LIBELLE
  local fenetres
  if [ ! -s "$1" ]; then echec "$3 : capture absente"; return; fi
  fenetres=$(python3 tools/ci/fenetres_bureau_ring3.py "$P" --jusqua "$2" --opaques "$doom,$nav")
  # shellcheck disable=SC2086
  python3 tools/ci/verifie_capture_bureau_ring3.py "$1" $fenetres && ok "$3 ($fenetres)" || echec "$3 ($fenetres)"
}
juge "$SORTIE/scenario.ppm" 'COMPOSITED_BUREAU_SCENARIO_FIN fenetre=1' "capture apres le scenario : pile, cadres, mire au pixel, DOOM et Ladybird peints"
grep -aqE "PROCESS_FAULT pid=${mire:-x} " "$P" && ok "la mire meurt d'une faute de page" || echec "faute de la mire absente"
grep -aq "COMPOSITED_BUREAU_CLIENT_MORT pid=${mire:-x}" "$P" && ok "le compositeur a vu sa mort" || echec "mort de la mire non vue"
autres=$(grep -aoE 'PROCESS_FAULT pid=[0-9]+ ' "$P" | grep -vE "pid=${mire:-x} " | sort -u | tr '\n' ' ' || true)
[ -z "$autres" ] && ok "aucune autre faute (DOOM, Ladybird, compositeur)" || echec "autres fautes : $autres"
for paire in "DOOM:$doom" "Ladybird:$nav"; do
  nom=${paire%%:*}; pid=${paire#*:}
  avant=$(trames "$pid" "COMPOSITED_BUREAU_CLIENT_MORT pid=${mire:-x}"); apres=$(trames "$pid")
  (( ${apres:-0} > ${avant:-0} )) && ok "$nom encore compose apres la mort de la mire (${avant:-0} -> ${apres:-0})" \
    || echec "$nom fige apres la mort de la mire (${avant:-0} -> ${apres:-0})"
done
juge "$SORTIE/apres-mort.ppm" "COMPOSITED_BUREAU_CLIENT_MORT pid=${mire:-x}" "capture apres la mort de la mire : DOOM et Ladybird intacts"

mesure=$(grep -aA8 'COMPOSITED_BUREAU_MESURE trames=' "$P" | tail -n 9 | tr '\n' ' ' || true)
val() { echo "$mesure" | grep -oE "$1=[0-9]+" | sed -n '1s/.*=//p' || true; }
echo "BUREAU_RING3_APPLICATIONS_MESURE accel=$( [ "${BO_QEMU_KVM:-0}" = 1 ] && echo kvm || echo tcg ) trames=$(val trames)" \
  "duree_ms=$(val duree_ms) copie_us_moy=$(val lit_us_moy) presentation_us_moy=$(val presente_us_moy)" \
  "recompositions=$(val recompositions) doom=$(trames "$doom") ladybird=$(trames "$nav")"
if (( echecs == 0 )); then
  echo "BUREAU_RING3_APPLICATIONS_OK doom=$doom ladybird=$nav compositeur=$compo"
  exit 0
fi
echo "BUREAU_RING3_APPLICATIONS_ECHEC n=$echecs"
exit 1
