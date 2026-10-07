#!/usr/bin/env bash
# Endurance : >= 10 minutes de navigation sans crash du Compositor
# (BOUCHAUD_ENDURANCE_V1, convergence P1/P6).
#
#   tools/ci/run_ladybird_endurance.sh BOOTIMAGE NATIVE_DIR [DUREE_S]
#
# La page /endurance.html de la fixture boucle DUREE_S secondes (600 par
# defaut) : cadres remplaces (contextes Compositor crees et detruits),
# workers crees puis termines, canvas anime, defilement, images rechargees,
# et tous les trois cycles un onglet qui passe par un relais du meme site sur
# L'AUTRE site (10.0.2.100) puis se ferme -- donc des processus WebContent
# crees ([LB] PROCESS_SWAP) et detruits, et leurs connexions au Compositor
# avec eux. C'est le cycle de vie qui menait au crash
# `ConnectionFromClient.cpp:68 VERIFICATION FAILED: connection`.
# (Jusqu'au run 37581515158, l'onglet s'ouvrait directement sur l'autre site
# et restait dans le WebContent de la page : 27 onglets, 2 WebContent.)
#
# Exige :
#   - HOST_ENDURANCE_FIN apres au moins 95 % de DUREE_S ;
#   - au moins DUREE_S/10 cycles, workers et cadres au rendez-vous ;
#   - au moins un onglet ouvert sur l'autre site (HOST_ENDURANCE_ENFANT) ;
#   - UN SEUL Compositor du debut a la fin (aucun redemarrage) ;
#   - aucune assertion, aucune panique, aucun COMPOSITOR_LINK_GIVE_UP,
#     aucune faute du Compositor.
# Mesure (sans seuil) : RSS de chaque service au premier et au dernier releve,
# evenements de cycle de vie des connexions Compositor.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_ladybird_endurance.sh BOOTIMAGE NATIVE_DIR [DUREE_S]}
OUT=${2:?usage: run_ladybird_endurance.sh BOOTIMAGE NATIVE_DIR [DUREE_S]}
DUREE=${3:-600}
SILENCE_MAX=${BO_ENDURANCE_SILENCE_S:-180}

SCENARIO=scenario-endurance
IMAGE=ladybird-endurance.img
LOG=serie-endurance.log
rm -rf "$SCENARIO" "$IMAGE" "$LOG" fixture-endurance.log

python3 tools/health/browser_host_fixture.py > fixture-endurance.log 2>&1 &
FIXTURE=$!
# Panique noyau : son contexte en DERNIER (tools/ci/extrait_panique.sh).
# Et si le banc echoue sans panique (KVM, run 37618172578 : 1206 s de QEMU,
# navigateur jamais lance), ce que faisaient les coeurs, symbolise contre le
# noyau qui a tourne (BOUCHAUD_SYMBOLISE_NOYAU_V1).
trap 'rc=$?; kill "$FIXTURE" 2>/dev/null || true; tools/ci/extrait_panique.sh "$LOG" "$OUT"; if [ "$rc" -ne 0 ]; then python3 tools/ci/symbolise_noyau.py "$LOG" "$(dirname "$BOOT")/bouchaud-os"; fi; exit "$rc"' EXIT
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

URL="http://10.0.2.2:18082/endurance.html?duree=$DUREE"
cat > "$SCENARIO/autorun" <<AUTORUN
echo "=== Bouchaud endurance ==="
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
# BOUCHAUD_ENDURANCE_KVM_V1 -- BO_QEMU_KVM=1 : meme banc sous KVM, pour
# separer ce que coute l'emulation TCG de ce que coute le logiciel (run
# 37585729384 : rAF ~1,3 Hz, WebContent a 37 % d'un coeur, Compositor 64 %).
# Diagnostic seulement : la porte de convergence reste sous TCG.
ACCEL="-cpu max"
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
fi
echo "ENDURANCE_ACCEL ${ACCEL}"
qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$IMAGE" \
  -m 8192 -smp 4 $ACCEL -display none -no-reboot \
  -netdev "user,id=net0,guestfwd=tcp:10.0.2.100:18082-cmd:nc 127.0.0.1 18082" -device e1000,netdev=net0 \
  -audiodev none,id=muet -device AC97,audiodev=muet \
  -serial file:"$LOG" &
PID=$!

DEBUT=$SECONDS
PLAFOND=$((DUREE + 900))
taille_vue=0
derniere_avancee=$SECONDS
cycles_vus=0
verdict=inconnu
while kill -0 "$PID" 2>/dev/null; do
  if grep -aq 'HOST_ENDURANCE_FIN' "$LOG"; then verdict=fini; break; fi
  if grep -aq 'KERNEL PANIC' "$LOG"; then verdict=panique; break; fi
  n=$(sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -ac 'js log) "HOST_ENDURANCE cycle=' || true)
  if [ "$n" != "$cycles_vus" ]; then
    cycles_vus=$n
    if (( n % 12 == 0 )); then printf '  T+%-5ss %s cycle(s)\n' "$((SECONDS - DEBUT))" "$n"; fi
  fi
  taille=$(wc -c < "$LOG")
  if [ "$taille" -ne "$taille_vue" ]; then taille_vue=$taille; derniere_avancee=$SECONDS; fi
  if (( SECONDS - derniere_avancee > SILENCE_MAX )); then verdict=muet; break; fi
  if (( SECONDS - DEBUT > PLAFOND )); then verdict=plafond; break; fi
  sleep 5
done
[ "$verdict" = inconnu ] && verdict=vm_morte
sleep 3
kill -TERM "$PID" 2>/dev/null || true
sleep 1
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true

sed -E 's/\x1b\[[0-9;]*m//g; s/^\[[^]]*\]\[[^]]*\]\[FPS:[^]]*\] //' "$LOG" | tr -d '\r' > "$LOG.propre"
P="$LOG.propre"
echo "ENDURANCE_VERDICT_BOUCLE $verdict duree_reelle_s=$((SECONDS - DEBUT))"
grep -aoE 'HOST_ENDURANCE(_FIN)? cycle[s]?=.*' "$P" | awk 'NR % 20 == 1' | head -10 || true
grep -aoE 'HOST_ENDURANCE_FIN .*' "$P" | head -1 || true

echo "== cycle de vie des connexions Compositor =="
for m in CONNECTION_CREATE CONNECTION_REMOVE PEER_CLOSE CONTEXT_CREATE CONTEXT_DESTROY LATE_MESSAGE COMPOSITOR_LINK_LOST COMPOSITOR_LINK_RECOVERED COMPOSITOR_LINK_GIVE_UP PROCESS_SWAP; do
  printf '  %-26s %s\n' "$m" "$(grep -ac "\[LB\] $m" "$P" || true)"
done
compositors=$(grep -aoE 'PERF_EXECVE .*image=/usr/libexec/ladybird/Compositor pid=[0-9]+' "$P" | grep -oE 'pid=[0-9]+' | sort -u | wc -l || true)
webcontents=$(grep -aoE 'PERF_EXECVE .*image=/usr/libexec/ladybird/WebContent pid=[0-9]+' "$P" | grep -oE 'pid=[0-9]+' | sort -u | wc -l || true)
echo "  processus Compositor lances : $compositors ; WebContent lances : $webcontents"
echo "== processus du navigateur (ProcessManager : PROCESS_CREATE / PROCESS_EXIT) =="
for t in WebContent WebWorker RequestServer ImageDecoder Compositor; do
  printf '  %-14s crees %-5s sortis %s\n' "$t" "$(grep -ac "\[LB\] PROCESS_CREATE type=$t " "$P" || true)" "$(grep -ac "\[LB\] PROCESS_EXIT type=$t " "$P" || true)"
done

echo "== RSS (kio) premier et dernier releve, par image =="
for img in BouchaudBrowserHost bo-navigateur WebContent RequestServer ImageDecoder Compositor WebWorker; do
  premier=$(grep -aoE "\[PERF-PROC\] t=[0-9]+ pid=[0-9]+ image=[^ ]*$img rss_kio=[0-9]+" "$P" | head -1 | grep -oE 'rss_kio=[0-9]+' || true)
  dernier=$(grep -aoE "\[PERF-PROC\] t=[0-9]+ pid=[0-9]+ image=[^ ]*$img rss_kio=[0-9]+" "$P" | tail -1 | grep -oE 'rss_kio=[0-9]+' || true)
  [ -n "$premier" ] && printf '  %-20s premier %-16s dernier %s\n' "$img" "$premier" "$dernier"
done

echo "== tendance RSS par processus (pente de la seconde moitie de vie) =="
python3 tools/ci/tendance_rss.py "$P" --min-releves 10 | sed 's/^/  /'

echo "== verdict =="
echecs=()
exige() { local quoi=$1; shift; if "$@"; then echo "  ok      $quoi"; else echo "  ECHEC   $quoi"; echecs+=("$quoi"); fi; }
fin=$(grep -aoE 'HOST_ENDURANCE_FIN cycles=[0-9]+ t_s=[0-9]+ cadres_ok=[0-9]+ cadres_echus=[0-9]+ workers_ok=[0-9]+ onglets=[0-9]+' "$P" | head -1 || true)
# `|| true` : sans ligne FIN (panique, VM morte), grep rend 1 et `set -e`
# tuait le banc AVANT son verdict (run 37584587000 : rien apres « verdict »).
val() { echo "$fin" | grep -oE "$1=[0-9]+" | cut -d= -f2 || true; }
cycles=$(val cycles); t_s=$(val t_s); cadres=$(val cadres_ok); echus=$(val cadres_echus); workers=$(val workers_ok); onglets=$(val onglets)
exige "la page a fini (HOST_ENDURANCE_FIN)" test -n "$fin"
exige "duree >= 95 % de ${DUREE} s (t_s=${t_s:-?})" test "${t_s:-0}" -ge $((DUREE * 95 / 100))
exige "au moins $((DUREE / 10)) cycles (${cycles:-0})" test "${cycles:-0}" -ge $((DUREE / 10))
exige "cadres charges (${cadres:-0}/${cycles:-0}, ${echus:-?} au-dela de 30 s)" test "${cadres:-0}" -ge $(( ${cycles:-0} - 2 ))
exige "aucun cadre au-dela de 30 s (${echus:-?})" test "${echus:-1}" -eq 0
exige "workers au rendez-vous (${workers:-0}/${cycles:-0})" test "${workers:-0}" -ge $(( ${cycles:-0} - 2 ))
exige "onglets sur l'autre site ouverts (${onglets:-0})" test "${onglets:-0}" -ge 1
exige "onglet enfant charge sur 10.0.2.100" grep -aq 'HOST_ENDURANCE_ENFANT .*origine=http://10.0.2.100:18082' "$P"
swaps=$(grep -ac '\[LB\] PROCESS_SWAP onglet=[0-9]* raison=autre_site' "$P" || true)
exige "chaque onglet enfant a change de WebContent (${swaps} swaps / ${onglets:-0} onglets)" test "$swaps" -ge $(( ${onglets:-0} - 2 )) -a "${onglets:-0}" -ge 1
exige "un seul Compositor du debut a la fin ($compositors)" test "$compositors" -eq 1
exige "aucune assertion (VERIFICATION FAILED)" bash -c "! grep -aq 'VERIFICATION FAILED' '$P'"
exige "aucune panique noyau" bash -c "! grep -aq 'KERNEL PANIC' '$P'"
exige "la boucle a fini sur la page, pas sur ${verdict}" test "$verdict" = fini
exige "aucun abandon de lien Compositor" bash -c "! grep -aq 'COMPOSITOR_LINK_GIVE_UP' '$P'"
exige "aucune mort du Compositor" bash -c "! grep -aqE '(PROCESS_FAULT|PROCESS_EXIT|PROCESS_DEATH).*Compositor' '$P'"

if [ ${#echecs[@]} -eq 0 ]; then
  echo "LADYBIRD_ENDURANCE_OK duree_s=${t_s} cycles=${cycles}"
else
  echo "LADYBIRD_ENDURANCE_ECHEC n=${#echecs[@]}"
  exit 1
fi
