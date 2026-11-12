#!/usr/bin/env bash
#
# BOUCHAUD_BUREAU_RING3_2A -- la premiere vraie fenetre composee en ring 3.
#
#   tools/ci/run_bureau_ring3.sh BOOTIMAGE [SORTIE]
#
# Bras B1 (ring 3, tranche 2A) : l'image configure le bureau ring 3 (`/bureau-ring3`) ; le
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
# Bras B2 (ring 3, tranche 2B) : DEUX applications, chacune son processus,
# composees par le meme `composited` : capture des deux fenetres empilees,
# puis scenario `deplace-empile` (la fenetre du fond glisse en 20 pas, puis
# monte au premier plan) et capture decodee au pixel contre le nouvel ordre ;
# l'une meurt (faute de page), l'autre continue d'etre composee ; puis
# l'autre, et l'ecran redevient le fond.
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
# prepare_ring3 NOM LIGNE... : image avec composited, mire-gpu et la
# configuration du bureau ring 3 (une ligne par client, `@scenario` permis).
prepare_ring3() {
  local d="$SORTIE/$1"; shift
  mkdir -p "$d/scen/bin"
  cp "$SORTIE/bin/composited-slice" "$d/scen/bin/composited"
  cp "$SORTIE/bin/attends" "$d/scen/bin/attends"
  cp "$SORTIE/bin/mire-gpu" "$d/scen/mire-gpu"
  chmod +x "$d/scen/bin/"* "$d/scen/mire-gpu"
  printf '%s\n' "$@" > "$d/scen/bureau-ring3"
  : > "$d/scen/garde-vm-vivante"
  printf '%s\n' 'echo BUREAU_RING3_AUTORUN_BEGIN' 'desktop' 'echo BUREAU_RING3_APRES_DESKTOP' \
    '/bin/attends 300' 'echo BUREAU_RING3_AUTORUN_END' > "$d/scen/autorun"
}

# juge_capture FICHIER JOURNAL MARQUEUR LIBELLE : la capture prise juste apres
# MARQUEUR, decodee au pixel contre l'etat des fenetres publie a cet instant.
juge_capture() {
  local fenetres
  if [ ! -s "$1" ]; then echec "$4 : capture absente"; return; fi
  # MARQUEUR vide : l'etat a la fin du journal.
  if [ -n "$3" ]; then
    fenetres=$(python3 tools/ci/fenetres_bureau_ring3.py "$2" --jusqua "$3")
  else
    fenetres=$(python3 tools/ci/fenetres_bureau_ring3.py "$2")
  fi
  # shellcheck disable=SC2086
  python3 tools/ci/verifie_capture_bureau_ring3.py "$1" $fenetres && ok "$4 ($fenetres)" || echec "$4 ($fenetres)"
}

# attend_et_capture NOM "MARQUEUR=FICHIER"... FIN : capture a chaque marqueur,
# s'arrete apres FIN (ou a l'echeance).
attend_et_capture() {
  local d="$SORTIE/$1" fin=$2; shift 2
  local -A fait=()
  local LIMITE=$((SECONDS + 300)) paire m f
  while kill -0 "$QEMU_PID" 2>/dev/null; do
    if grep -aqE "$fatal_re" "$d/serie.log"; then break; fi
    for paire in "$@"; do
      m=${paire%=*}; f=${paire##*=}
      if [ -z "${fait[$f]:-}" ] && grep -aqF "$m" "$d/serie.log"; then
        sleep 1; capture "$MONITEUR" "$d/$f"; fait[$f]=1
      fi
    done
    if grep -aqE "$fin" "$d/serie.log"; then sleep 2; [ -z "${fait[fin.ppm]:-}" ] && capture "$MONITEUR" "$d/fin.ppm"; break; fi
    if (( SECONDS >= LIMITE )); then echo "ECHEANCE $1" >&2; break; fi
    sleep 0.5
  done
  arrete
  propre "$d/serie.log" > "$d/serie.propre"
  grep -aE 'BUREAU_RING3|COMPOSITED_BUREAU|MIRE_GPU|PROCESS_(FAULT|EXIT|DEATH).*mire|BUREAU_NOYAU' "$d/serie.propre" \
    | grep -v 'BOUCHAUD_DISPLAY_PRESENT n=' | cut -c1-200 | tail -n 40 || true
}

pids_clients() { grep -aoE 'BOUCHAUD_BUREAU_RING3_CLIENT pid=[0-9]+' "$1" | grep -oE '[0-9]+$' || true; }

# B1 -- une application (tranche 2A).
echo "=== bras B1 : une application ring 3 (faute apres $FAUTE_APRES trames) ==="
prepare_ring3 ring3 "/mire-gpu BO_MIRE_FAUTE_APRES=$FAUTE_APRES"
demarre ring3
attend_et_capture ring3 'COMPOSITED_BUREAU_(OK|FAIL)' 'COMPOSITED_BUREAU_MESURE trames=240=vivant.ppm'
B="$SORTIE/ring3"; P="$B/serie.propre"
grep -aqE "$fatal_re" "$P" && echec "panique noyau" || ok "aucune panique noyau"
pid_client=$(pids_clients "$P" | sed -n 1p)
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
juge_capture "$B/vivant.ppm" "$P" 'COMPOSITED_BUREAU_MESURE trames=240' "capture du client vivant decodee au pixel"
juge_capture "$B/fin.ppm" "$P" 'COMPOSITED_BUREAU_OK' "capture apres la mort : fenetre effacee, bureau intact"
mesure=$(awk '/COMPOSITED_BUREAU_MESURE trames=240/ { n = 9 } n > 0 { print; n-- }' "$P" | tr '\n' ' ')
val() { echo "$mesure" | grep -oE "$1=[0-9]+" | sed -n '1s/.*=//p' || true; }
b_trames=240; b_duree=$(val duree_ms); b_lit=$(val lit_us_moy); b_pres=$(val presente_us_moy)
b_px=$(val pixels_copies)
if [ -n "$b_duree" ] && [ -n "$b_lit" ] && [ -n "$b_pres" ]; then
  ok "mesures du pont relevees"
else
  echec "mesures du pont absentes"
fi

# B2 -- deux applications, deplacement, empilement, morts successives (2B).
echo "=== bras B2 : deux applications ring 3, deplacement, empilement, une meurt puis l'autre ==="
prepare_ring3 deux '@scenario deplace-empile' \
  "/mire-gpu BO_MIRE_FAUTE_APRES=$((FAUTE_APRES * 2))" "/mire-gpu BO_MIRE_FAUTE_APRES=$((FAUTE_APRES * 3))"
demarre deux
attend_et_capture deux 'COMPOSITED_BUREAU_(OK|FAIL)' \
  'COMPOSITED_BUREAU_FENETRE indice=1=deux-avant.ppm' \
  'COMPOSITED_BUREAU_SCENARIO_PILE_PUBLIEE=deux-apres.ppm'
D="$SORTIE/deux"; Q="$D/serie.propre"
grep -aqE "$fatal_re" "$Q" && echec "B2 : panique noyau" || ok "B2 : aucune panique noyau"
mapfile -t pids < <(pids_clients "$Q")
compo2=$(grep -aoE 'BOUCHAUD_COMPOSITEUR_LANCE pid=[0-9]+' "$Q" | grep -oE '[0-9]+$' | sed -n 1p || true)
if [ "${#pids[@]}" = 2 ] && [ "${pids[0]}" != "${pids[1]}" ] && [ -n "$compo2" ] \
   && [ "$compo2" != "${pids[0]}" ] && [ "$compo2" != "${pids[1]}" ]; then
  ok "B2 : trois processus ring 3 (applications ${pids[0]}, ${pids[1]} ; compositeur $compo2)"
else
  echec "B2 : processus distincts (${pids[*]:-?} / ${compo2:-?})"
fi
grep -aq 'COMPOSITED_BUREAU_SCENARIO_FIN fenetre=1' "$Q" && ok "B2 : deplacement puis passage au premier plan joues" \
  || echec "B2 : scenario non joue"
juge_capture "$D/deux-avant.ppm" "$Q" 'COMPOSITED_BUREAU_FENETRE indice=1' "B2 : deux fenetres empilees, decodees au pixel"
juge_capture "$D/deux-apres.ppm" "$Q" 'COMPOSITED_BUREAU_SCENARIO_PILE_PUBLIEE' "B2 : fenetre deplacee et remontee, decodee au pixel"
premier_mort=$(grep -aoE 'COMPOSITED_BUREAU_CLIENT_MORT pid=[0-9]+' "$Q" | sed -n 1p || true)
if [ -n "$premier_mort" ]; then
  # Le survivant continue d'etre compose apres la mort de l'autre.
  apres=$(awk -v m="$premier_mort" 'index($0, m) { vu = 1 } vu && /COMPOSITED_BUREAU_MESURE trames=/ { n++ } END { print n + 0 }' "$Q")
  [ "$apres" -ge 1 ] && ok "B2 : le survivant est encore compose apres la mort de l'autre ($apres releves)" \
    || echec "B2 : rien n'est compose apres la premiere mort"
else
  echec "B2 : aucune mort de client"
fi
[ "$(grep -ac 'COMPOSITED_BUREAU_CLIENT_MORT pid=' "$Q")" = 2 ] && ok "B2 : les deux morts vues" || echec "B2 : morts non vues"
grep -aq 'COMPOSITED_BUREAU_OK' "$Q" && ok "B2 : le compositeur a survecu" || echec "B2 : COMPOSITED_BUREAU_OK absent"
juge_capture "$D/fin.ppm" "$Q" 'COMPOSITED_BUREAU_OK' "B2 : bureau vide apres les deux morts"

# --------------------------------------------------------------------------
# Bras B3 (tranche 2B) : cycle de vie complet des fenetres -- deplacement,
# empilement, redimensionnement (Configure, borne par l'allocation),
# minimisation, restauration, fermeture polie (CloseRequest), relance par la
# capacite d'affichage (configuration de l'image seulement). Chaque etape est
# capturee et jugee au pixel contre l'etat publie par le compositeur.
# --------------------------------------------------------------------------
echo "=== bras B3 : cycle de vie complet de deux fenetres ==="
prepare_ring3 complet '@scenario complet' "/mire-gpu" "/mire-gpu"
demarre complet
attend_et_capture complet 'COMPOSITED_BUREAU_SCENARIO_(COMPLET|ECHEC)' \
  'COMPOSITED_BUREAU_MINIMISE fenetre=0=c-minimise.ppm' \
  'COMPOSITED_BUREAU_RESTAURE fenetre=0=c-restaure.ppm'
C="$SORTIE/complet"; K="$C/serie.propre"
grep -aqE "$fatal_re" "$K" && echec "B3 : panique noyau" || ok "B3 : aucune panique noyau"
for etape in 'redimensionnement 480x300 accepte' 'zone vide refusee' 'fermeture demandee' \
             'application relancee' 'relance hors configuration refusee'; do
  grep -aq "\[COMPOSITED\] ok   bureau : $etape" "$K" && ok "B3 : $etape" || echec "B3 : $etape"
done
grep -aq 'COMPOSITED_BUREAU_SCENARIO_COMPLET' "$K" && ok "B3 : scenario complet sans echec" || echec "B3 : scenario incomplet"
grep -aqE 'PROCESS_FAULT' "$K" && echec "B3 : faute d'un processus" || ok "B3 : aucune faute (fermeture polie)"
[ "$(grep -ac 'BOUCHAUD_BUREAU_RING3_RELANCE rang=1 n=1' "$K")" = 1 ] && ok "B3 : une relance, comptee par le noyau" \
  || echec "B3 : relance non comptee"
# La derniere capture apres la relance : deux fenetres de nouveau.
juge_capture "$C/fin.ppm" "$K" 'COMPOSITED_BUREAU_SCENARIO_COMPLET' "B3 : apres la relance, deux fenetres au pixel"
juge_capture "$C/c-minimise.ppm" "$K" 'COMPOSITED_BUREAU_MINIMISE fenetre=0' "B3 : fenetre minimisee absente, l'autre redimensionnee 480x300 au pixel"
juge_capture "$C/c-restaure.ppm" "$K" 'COMPOSITED_BUREAU_RESTAURE fenetre=0' "B3 : fenetre restauree au pixel"

# --------------------------------------------------------------------------
# Bras B4 (BOUCHAUD_BUREAU_RING3_MORT_ISOLEE_V1) : la mort d'UN client ne tue
# pas les autres. Le banc applications l'a montre : la mire qui faute
# emportait Ladybird, parce que la sortie de tout client arretait l'arbre du
# navigateur enregistre. Ici le « navigateur » est une copie de la mire
# installee sous /bo-navigateur (le noyau l'enregistre comme racine du
# navigateur exactement comme Ladybird) : il doit survivre a la mort de
# l'autre client, et rester compose. Sans le correctif : tue_par_gestionnaire.
# --------------------------------------------------------------------------
echo "=== bras B4 : la mort d'un client n'emporte pas le navigateur ==="
prepare_ring3 isolee "/mire-gpu BO_MIRE_FAUTE_APRES=$FAUTE_APRES" "/bo-navigateur"
cp "$SORTIE/bin/mire-gpu" "$SORTIE/isolee/scen/bo-navigateur"; chmod +x "$SORTIE/isolee/scen/bo-navigateur"
demarre isolee
I="$SORTIE/isolee"
LIMITE=$((SECONDS + 300)); mort_vue=0
while kill -0 "$QEMU_PID" 2>/dev/null; do
  if grep -aqE "$fatal_re" "$I/serie.log"; then break; fi
  if [ "$mort_vue" = 0 ] && grep -aq 'COMPOSITED_BUREAU_CLIENT_MORT' "$I/serie.log"; then mort_vue=$SECONDS; fi
  # Quinze secondes apres la mort : le survivant doit encore etre la.
  if [ "$mort_vue" != 0 ] && (( SECONDS >= mort_vue + 15 )); then capture "$MONITEUR" "$I/fin.ppm"; sleep 1; break; fi
  if (( SECONDS >= LIMITE )); then echo "ECHEANCE isolee" >&2; break; fi
  sleep 0.5
done
arrete
propre "$I/serie.log" > "$I/serie.propre"
J="$I/serie.propre"
grep -aqE "$fatal_re" "$J" && echec "B4 : panique noyau" || ok "B4 : aucune panique noyau"
nav=$(grep -aoE 'BOUCHAUD_BUREAU_RING3_CLIENT pid=[0-9]+ chemin=/bo-navigateur' "$J" | grep -oE 'pid=[0-9]+' | grep -oE '[0-9]+' | sed -n 1p || true)
mire=$(grep -aoE 'BOUCHAUD_BUREAU_RING3_CLIENT pid=[0-9]+ chemin=/mire-gpu' "$J" | grep -oE 'pid=[0-9]+' | grep -oE '[0-9]+' | sed -n 1p || true)
grep -aq "COMPOSITED_BUREAU_CLIENT_MORT pid=${mire:-x}" "$J" && ok "B4 : la mire est morte et vue morte" || echec "B4 : mort de la mire non vue"
if grep -aqE "PROCESS_DEATH .*pid=${nav:-x} .*cause=tue_par_gestionnaire" "$J"; then
  echec "B4 : le navigateur ${nav:-?} a ete TUE par la mort de la mire"
else
  ok "B4 : le navigateur ${nav:-?} n'est pas tue par la mort de la mire"
fi
grep -aq "COMPOSITED_BUREAU_CLIENT_MORT pid=${nav:-x}" "$J" && echec "B4 : le navigateur est mort" || ok "B4 : le navigateur vit encore"
apres=$(awk -v m="COMPOSITED_BUREAU_CLIENT_MORT pid=${mire:-x}" -v n="pid=${nav:-x} " \
  'index($0, m) { vu = 1 } vu && /COMPOSITED_BUREAU_TRAMES/ && index($0, n) && /vivante=1/ { k++ } END { print k + 0 }' "$J")
[ "$apres" -ge 1 ] && ok "B4 : le navigateur est encore compose apres la mort ($apres releves)" \
  || echec "B4 : le navigateur n'est plus compose apres la mort de la mire"
juge_capture "$I/fin.ppm" "$J" '' "B4 : capture apres la mort, le navigateur seul au pixel"

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
  echo "BUREAU_RING3_OK client=$pid_client compositeur=$pid_compo"
  exit 0
fi
echo "BUREAU_RING3_ECHEC n=$echecs"
exit 1
