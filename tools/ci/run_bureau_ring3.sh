#!/usr/bin/env bash
#
# BOUCHAUD_BUREAU_RING3_2A -- la premiere vraie fenetre composee en ring 3.
#
#   tools/ci/run_bureau_ring3.sh BOOTIMAGE [SORTIE]
#
# Bras B (ring 3) : l'image configure le bureau ring 3 (`/bureau-ring3`) ; le
# noyau lance `mire-gpu` -- un client BOGU ORDINAIRE, inchange, dans son propre
# processus -- puis `/bin/composited bureau`, autre processus ring 3, avec la
# capacite d'affichage et la capacite `Fenetre` sur ce client. Exige :
#   - deux processus ring 3 distincts (pid du client != pid du compositeur) ;
#   - le bureau noyau REFUSE (`desktop` dans l'autorun) : un seul compositeur ;
#   - les refus de securite du pont (bornes, debordements, droits, adresses) ;
#   - une capture QEMU decodee AU PIXEL : chaque pixel de l'ecran est soit le
#     pixel exact de la mire, soit le carre mobile, soit le cadre, soit le fond ;
#   - le client tue par une VRAIE faute de page (`BO_MIRE_FAUTE_APRES`) : le
#     compositeur survit, la fenetre disparait (capture : tout l'ecran = fond),
#     aucune panique ;
#   - les mesures du pont (copie et presentation par trame, cadence).
# Bras A (reference) : le MEME client sous le bureau noyau actuel, meme QEMU,
# memes durees ; ses releves `[GPU-COMPOSITION]` donnent la cadence et le cout
# par trame du chemin existant. Le bras A n'a pas de seuil ici : c'est la
# reference mesuree, publiee a cote du bras B (BUREAU_RING3_AB).
#
# BO_QEMU_KVM=1 : sous KVM. Code 0 : OK ; 1 : ECHEC.
set -euo pipefail
cd "$(dirname "$0")/../.."

BOOT=${1:?usage: run_bureau_ring3.sh BOOTIMAGE [SORTIE]}
SORTIE=${2:-target/bureau-ring3}
ACCEL="-cpu max"
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
fi
FAUTE_APRES=${BO_BUREAU_FAUTE_APRES:-600}
rm -rf "$SORTIE"; mkdir -p "$SORTIE/bin"
SORTIE=$(cd "$SORTIE" && pwd)
# Les sockets unix sont limites a 108 octets : le moniteur vit dans un
# repertoire court, jamais sous $SORTIE.
SOCKETS=$(mktemp -d "${TMPDIR:-/tmp}/bo-bureau.XXXXXX")

echo "=== construction : composited (freestanding), mire-gpu, attends ==="
(cd userland/services/composited && OUT="$SORTIE/bin" bash ./build.sh >/dev/null)
musl-gcc -O2 -static-pie -fPIE -o "$SORTIE/bin/mire-gpu" tools/userland/mire-gpu.c
musl-gcc -O2 -static-pie -fPIE -o "$SORTIE/bin/attends" tools/userland/attends.c

fatal_re='KERNEL PANIC|panicked at|DOUBLE FAULT|TRIPLE FAULT|SpinLock recursive acquisition'
propre() { sed -E 's/\x1b\[[0-9;]*m//g' "$1" | tr -d '\r'; }

capture() {
  python3 - "$1" "$2" <<'PY' || true
import socket, sys, time
s = socket.socket(socket.AF_UNIX)
s.settimeout(5)
s.connect(sys.argv[1])
s.sendall(f"screendump {sys.argv[2]}\n".encode())
time.sleep(1.0)
s.close()
PY
}

# demarre NOM : lance QEMU sur $SORTIE/NOM/disque.img, en arriere-plan.
demarre() {
  local d="$SORTIE/$1"
  (cd tools/userland && IMAGE="$d/disque.img" ./mkdisk.sh "$d/scen" >/dev/null)
  : > "$d/serie.log"
  MONITEUR="$SOCKETS/$1.sock"
  rm -f "$MONITEUR"
  # shellcheck disable=SC2086
  qemu-system-x86_64 $ACCEL -drive format=raw,file="$BOOT" -drive format=raw,file="$d/disque.img" \
    -m 2048 -smp 4 -display none -no-reboot \
    -monitor "unix:$MONITEUR,server,nowait" -serial file:"$d/serie.log" &
  QEMU_PID=$!
}
arrete() { kill -TERM "$QEMU_PID" 2>/dev/null || true; sleep 1; kill -KILL "$QEMU_PID" 2>/dev/null || true; wait "$QEMU_PID" 2>/dev/null || true; }
trap 'arrete; rm -rf "$SOCKETS"' EXIT

echecs=0
echec() { echo "  ECHEC   $1"; echecs=$((echecs + 1)); }
ok() { echo "  ok      $1"; }

# --------------------------------------------------------------------------
# Bras B : composited ring 3.
# --------------------------------------------------------------------------
B="$SORTIE/ring3"; mkdir -p "$B/scen/bin"
cp "$SORTIE/bin/composited-slice" "$B/scen/bin/composited"
cp "$SORTIE/bin/attends" "$B/scen/bin/attends"
cp "$SORTIE/bin/mire-gpu" "$B/scen/mire-gpu"
chmod +x "$B/scen/bin/"* "$B/scen/mire-gpu"
echo "/mire-gpu BO_MIRE_FAUTE_APRES=$FAUTE_APRES" > "$B/scen/bureau-ring3"
: > "$B/scen/garde-vm-vivante"
printf '%s\n' 'echo BUREAU_RING3_AUTORUN_BEGIN' 'desktop' 'echo BUREAU_RING3_APRES_DESKTOP' \
  '/bin/attends 300' 'echo BUREAU_RING3_AUTORUN_END' > "$B/scen/autorun"
echo "=== bras B : bureau ring 3 (client en faute apres $FAUTE_APRES trames) ==="
demarre ring3
vivant=0; LIMITE=$((SECONDS + 240))
while kill -0 "$QEMU_PID" 2>/dev/null; do
  if grep -aqE "$fatal_re" "$B/serie.log"; then break; fi
  if [ "$vivant" = 0 ] && grep -aq 'COMPOSITED_BUREAU_MESURE trames=240' "$B/serie.log"; then
    capture "$MONITEUR" "$B/vivant.ppm"; vivant=1
  fi
  if grep -aqE 'COMPOSITED_BUREAU_(OK|FAIL)' "$B/serie.log"; then
    sleep 2; capture "$MONITEUR" "$B/mort.ppm"; break
  fi
  if (( SECONDS >= LIMITE )); then echo "ECHEANCE bras B" >&2; break; fi
  sleep 1
done
arrete
P="$B/serie.propre"; propre "$B/serie.log" > "$P"
grep -aE 'BUREAU_RING3|COMPOSITED|MIRE_GPU|PROCESS_(FAULT|EXIT|DEATH).*mire|BUREAU_NOYAU' "$P" \
  | grep -v 'BOUCHAUD_DISPLAY_PRESENT n=' | cut -c1-220 | tail -n 80 || true

grep -aqE "$fatal_re" "$P" && echec "panique noyau" || ok "aucune panique noyau"
pid_client=$(grep -aoE 'BOUCHAUD_BUREAU_RING3_CLIENT pid=[0-9]+' "$P" | grep -oE '[0-9]+$' | sed -n 1p || true)
pid_compo=$(grep -aoE 'BOUCHAUD_COMPOSITEUR_LANCE pid=[0-9]+' "$P" | grep -oE '[0-9]+$' | sed -n 1p || true)
if [ -n "$pid_client" ] && [ -n "$pid_compo" ] && [ "$pid_client" != "$pid_compo" ]; then
  ok "deux processus ring 3 : client pid=$pid_client, compositeur pid=$pid_compo"
else
  echec "processus distincts (client=${pid_client:-?} compositeur=${pid_compo:-?})"
fi
grep -aq 'BOUCHAUD_BUREAU_NOYAU_REFUSE raison=ecran-au-compositeur' "$P" \
  && ok "bureau noyau refuse : un seul compositeur" || echec "bureau noyau non refuse"
grep -aq 'COMPOSITED_BUREAU_REFUS_OK' "$P" && ok "refus de securite du pont (bornes, droits, adresses)" \
  || echec "refus de securite du pont"
grep -aqE 'PROCESS_FAULT pid='"${pid_client:-x}"' ' "$P" && ok "le client est mort d'une faute de page" \
  || echec "faute du client absente"
grep -aq "COMPOSITED_BUREAU_CLIENT_MORT pid=${pid_client:-x}" "$P" && ok "le compositeur a vu la mort du client" \
  || echec "mort du client non vue"
grep -aq 'COMPOSITED_BUREAU_OK' "$P" && ok "le compositeur a survecu (COMPOSITED_BUREAU_OK)" \
  || echec "COMPOSITED_BUREAU_OK absent"
fx=$(grep -aoE '\[COMPOSITED\] fenetre_x=[0-9]+' "$P" | grep -oE '[0-9]+$' | sed -n 1p || true)
fy=$(grep -aoE '\[COMPOSITED\] fenetre_y=[0-9]+' "$P" | grep -oE '[0-9]+$' | sed -n 1p || true)
if [ -n "$fx" ] && [ -s "$B/vivant.ppm" ]; then
  python3 tools/ci/verifie_capture_bureau_ring3.py "$B/vivant.ppm" "$fx" "$fy" 640 400 vivant \
    && ok "capture du client vivant decodee au pixel" || echec "capture du client vivant"
else
  echec "capture du client vivant absente"
fi
if [ -n "$fx" ] && [ -s "$B/mort.ppm" ]; then
  python3 tools/ci/verifie_capture_bureau_ring3.py "$B/mort.ppm" "$fx" "$fy" 640 400 mort \
    && ok "capture apres la mort : fenetre effacee, bureau intact" || echec "capture apres la mort"
else
  echec "capture apres la mort absente"
fi
mesure=$(awk '/COMPOSITED_BUREAU_MESURE trames=240/ { n = 7 } n > 0 { print; n-- }' "$P" | tr '\n' ' ')
val() { echo "$mesure" | grep -oE "$1=[0-9]+" | sed -n '1s/.*=//p' || true; }
b_trames=240; b_duree=$(val duree_ms); b_lit=$(val lit_us_moy); b_pres=$(val presente_us_moy)
b_px=$(val pixels_copies)
if [ -n "$b_duree" ] && [ -n "$b_lit" ] && [ -n "$b_pres" ]; then
  ok "mesures du pont relevees"
else
  echec "mesures du pont absentes"
fi

# --------------------------------------------------------------------------
# Bras B0 (temoin, revue #295) : bureau ring 3 configure, compositeur
# INVALIDE. Le noyau doit lever la reservation de l'ecran, rendre le client
# deja lance (il meurt), et laisser le bureau noyau demarrer et composer.
# Sans la levee, `desktop` serait refuse pour toute la session.
# --------------------------------------------------------------------------
Z="$SORTIE/sans-compositeur"; mkdir -p "$Z/scen/bin"
printf 'pas un ELF\n' > "$Z/scen/bin/composited"
cp "$SORTIE/bin/mire-gpu" "$Z/scen/mire-gpu"
chmod +x "$Z/scen/bin/composited" "$Z/scen/mire-gpu"
echo "/mire-gpu" > "$Z/scen/bureau-ring3"
: > "$Z/scen/garde-vm-vivante"
printf '%s\n' 'echo BUREAU_RING3_TEMOIN_BEGIN' 'export BO_AUTOSTART_APP=/mire-gpu' 'desktop' > "$Z/scen/autorun"
echo "=== bras B0 : compositeur invalide -- la reservation de l'ecran doit etre levee ==="
demarre sans-compositeur
LIMITE=$((SECONDS + 150))
while kill -0 "$QEMU_PID" 2>/dev/null; do
  if grep -aqE "$fatal_re" "$Z/serie.log"; then break; fi
  if [ "$(propre "$Z/serie.log" | grep -acE '\[GPU-COMPOSITION\] .*trames_clients=[1-9]' || true)" -gt 0 ]; then break; fi
  if grep -aq 'BOUCHAUD_BUREAU_NOYAU_REFUSE' "$Z/serie.log"; then sleep 2; break; fi
  if (( SECONDS >= LIMITE )); then echo "ECHEANCE bras B0" >&2; break; fi
  sleep 1
done
arrete
propre "$Z/serie.log" > "$Z/serie.propre"
grep -aE 'BUREAU_RING3|BOUCHAUD_COMPOSITEUR|BUREAU_NOYAU|APP_LAUNCH' "$Z/serie.propre" | cut -c1-200 | tail -n 10 || true
grep -aqE "$fatal_re" "$Z/serie.propre" && echec "B0 : panique noyau" || ok "B0 : aucune panique noyau"
grep -aq 'BOUCHAUD_COMPOSITEUR_ECHEC .*reservation_ecran=levee' "$Z/serie.propre" \
  && ok "B0 : compositeur invalide, reservation de l'ecran levee" || echec "B0 : reservation de l'ecran non levee"
grep -aq 'BOUCHAUD_BUREAU_NOYAU_REFUSE' "$Z/serie.propre" \
  && echec "B0 : bureau noyau refuse malgre l'absence de compositeur" || ok "B0 : bureau noyau non refuse"
[ "$(grep -acE '\[GPU-COMPOSITION\] .*trames_clients=[1-9]' "$Z/serie.propre" || true)" -gt 0 ] \
  && ok "B0 : le bureau noyau a repris l'ecran et compose une application" || echec "B0 : bureau noyau absent"
z_client=$(grep -aoE 'BOUCHAUD_BUREAU_RING3_CLIENT pid=[0-9]+' "$Z/serie.propre" | grep -oE '[0-9]+$' | sed -n 1p || true)
grep -aqE "PROCESS_(EXIT|DEATH) t=[0-9]+ pid=${z_client:-x} " "$Z/serie.propre" \
  && ok "B0 : le client confie au compositeur absent est termine" || echec "B0 : client orphelin (pid ${z_client:-?})"
grep -aq 'BOUCHAUD_EXEC_AVORTE poignees_liberees' "$Z/serie.propre" \
  && ok "B0 : poignees du processus avorte rendues" || echec "B0 : poignees du processus avorte non rendues"

# --------------------------------------------------------------------------
# Bras A : le meme client sous le bureau noyau actuel (reference).
# --------------------------------------------------------------------------
A="$SORTIE/noyau"; mkdir -p "$A/scen"
cp "$SORTIE/bin/mire-gpu" "$A/scen/mire-gpu"; chmod +x "$A/scen/mire-gpu"
: > "$A/scen/garde-vm-vivante"
printf '%s\n' 'echo BUREAU_NOYAU_AB_BEGIN' 'export BO_AUTOSTART_APP=/mire-gpu' 'desktop' > "$A/scen/autorun"
echo "=== bras A : le meme client sous le bureau noyau ==="
demarre noyau
LIMITE=$((SECONDS + 150))
while kill -0 "$QEMU_PID" 2>/dev/null; do
  if grep -aqE "$fatal_re" "$A/serie.log"; then break; fi
  n=$(propre "$A/serie.log" | grep -ac '\[GPU-COMPOSITION\] .*trames_clients=[1-9]' || true)
  if (( n >= 3 )); then capture "$MONITEUR" "$A/ecran.ppm"; break; fi
  if (( SECONDS >= LIMITE )); then echo "ECHEANCE bras A" >&2; break; fi
  sleep 1
done
arrete
propre "$A/serie.log" > "$A/serie.propre"
grep -aqE "$fatal_re" "$A/serie.propre" && echec "bras A : panique noyau" || true
a_releve=$(grep -a '\[GPU-COMPOSITION\] .*trames_clients=[1-9]' "$A/serie.propre" | tail -n 1 || true)
aval() { echo "$a_releve" | grep -oE "(^| )$1=[0-9]+" | sed -n '1s/.*=//p' || true; }
a_periode=$(aval periode_ms); a_tc=$(aval trames_clients); a_dessin=$(aval us_dessin_cpu_par_trame)
a_pres=$(aval us_presentation_par_trame); a_cpu=$(aval cpu_pct)
[ -n "$a_releve" ] && ok "bras A : releve du bureau noyau" || echec "bras A : aucun releve [GPU-COMPOSITION]"

# --------------------------------------------------------------------------
# A/B : publies cote a cote, sans seuil invente.
# --------------------------------------------------------------------------
fps() { [ -n "$1" ] && [ -n "$2" ] && [ "$2" -gt 0 ] && echo $(( $1 * 1000 / $2 )) || echo "?"; }
echo "BUREAU_RING3_AB accel=$( [ "${BO_QEMU_KVM:-0}" = 1 ] && echo kvm || echo tcg ) smp=4" \
  "B_ring3[trames_par_s=$(fps "$b_trames" "${b_duree:-0}") copie_us_par_trame=${b_lit:-?} presentation_us_par_trame=${b_pres:-?} pixels_par_trame=$(( ${b_px:-0} / b_trames ))]" \
  "A_noyau[trames_clients_par_s=$(fps "${a_tc:-}" "${a_periode:-0}") dessin_us_par_trame=${a_dessin:-?} presentation_us_par_trame=${a_pres:-?} cpu_pct=${a_cpu:-?}]"

if (( echecs == 0 )); then
  echo "BUREAU_RING3_2A_OK client=$pid_client compositeur=$pid_compo"
  exit 0
fi
echo "BUREAU_RING3_2A_ECHEC n=$echecs"
exit 1
