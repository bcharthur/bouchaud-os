#!/usr/bin/env bash
# Ladybird SUR Bouchaud contre Ladybird sous Linux : un corpus WPT (P11).
#
# BOUCHAUD_WPT_V1
#
#   tools/ci/run_ladybird_wpt.sh BOOTIMAGE NATIVE_DIR CORPUS_DIR
#
# CORPUS_DIR vient de `tools/ladybird/wpt/prepare-corpus.py` (tests WPT
# vendorises par Ladybird au commit epingle, avec le resultat attendu sous
# Linux headless). La page `wpt/runner.html` les joue un a un dans le
# navigateur de l'invite et imprime `HOST_WPT ...` par fichier puis
# `HOST_WPT_FIN ...`. Le banc echoue si le runner ne conclut pas ; les ECARTS
# sont une mesure, rapportee par `tools/ladybird/wpt/rapport.py`.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_ladybird_wpt.sh BOOTIMAGE NATIVE_DIR CORPUS_DIR}
OUT=${2:?usage: run_ladybird_wpt.sh BOOTIMAGE NATIVE_DIR CORPUS_DIR}
CORPUS=$(cd "${3:?usage: run_ladybird_wpt.sh BOOTIMAGE NATIVE_DIR CORPUS_DIR}" && pwd)
PLAFOND=${BO_WPT_PLAFOND_S:-2400}
SILENCE_MAX=${BO_WPT_SILENCE_S:-300}

SCENARIO=scenario-wpt
IMAGE=ladybird-wpt.img
LOG=serie-wpt.log
rm -rf "$SCENARIO" "$IMAGE" "$LOG" http-wpt.log

(cd "$CORPUS" && exec python3 -m http.server 18083 --bind 0.0.0.0) > http-wpt.log 2>&1 &
SERVEUR=$!
# Panique noyau : son contexte en DERNIER (tools/ci/extrait_panique.sh).
trap 'kill "$SERVEUR" 2>/dev/null || true; tools/ci/extrait_panique.sh "$LOG" "$OUT"' EXIT
sleep 1
kill -0 "$SERVEUR"

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

cat > "$SCENARIO/autorun" <<'AUTORUN'
echo "=== Bouchaud WPT ==="
export BO_AUTOSTART_BROWSER=1
export BOUCHAUD_M9=1
export BOUCHAUD_BROWSER_HOST=1
export BOUCHAUD_M11=1
export BOUCHAUD_TIME_ZONE=Europe/Paris
export BOUCHAUD_M9_URL='http://10.0.2.2:18083/wpt/runner.html'
desktop
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

DEBUT=$SECONDS
derniere_avancee=$SECONDS
taille_vue=0
fichiers_vus=0
verdict=inconnu
while kill -0 "$PID" 2>/dev/null; do
  if grep -aq 'HOST_WPT_FIN' "$LOG"; then verdict=fini; break; fi
  # Sans codes ANSI : le journal brut colore « (js log) », et ce compteur
  # n'avancait jamais.
  n=$(sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -ac 'js log) "HOST_WPT fichier=' || true)
  if [ "$n" != "$fichiers_vus" ]; then
    fichiers_vus=$n
    derniere_ligne_wpt=$SECONDS
    printf '  T+%-5ss %s fichier(s)\n' "$((SECONDS - DEBUT))" "$n"
  fi
  taille=$(wc -c < "$LOG")
  if [ "$taille" -ne "$taille_vue" ]; then
    taille_vue=$taille
    derniere_avancee=$SECONDS
  elif (( SECONDS - derniere_avancee >= SILENCE_MAX )); then
    verdict=muet
    break
  fi
  if (( SECONDS - DEBUT >= PLAFOND )); then verdict=plafond; break; fi
  # Le runner a sa propre echeance de 120 s par fichier : sans nouvelle ligne
  # HOST_WPT pendant 420 s, c'est la PAGE du runner qui est figee (run
  # 37526620689 : 38 min sans avancer, le journal grossissant des releves du
  # noyau, d'ou ni `muet` ni sortie avant le plafond).
  if (( SECONDS - ${derniere_ligne_wpt:-$DEBUT} >= 420 )); then verdict=runner_fige; break; fi
  sleep 5
done
sleep 2
kill -TERM "$PID" 2>/dev/null || true
sleep 1
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true

python3 tools/ladybird/wpt/rapport.py "$LOG" "$CORPUS/wpt/manifeste.json" > wpt-rapport.md || true
cat wpt-rapport.md
echo "WPT_VERDICT $verdict duree_s=$((SECONDS - DEBUT))"
if [ "$verdict" != "fini" ]; then
  echo "le runner WPT n'a pas conclu ($verdict)" >&2
  # Ce qui permet de dire POURQUOI : le dernier fichier demande au serveur,
  # puis la fin du journal du navigateur (journal serie en artefact seulement).
  echo "== requetes HTTP (fin) =="
  tail -15 http-wpt.log | sed 's/^/  /' || true
  echo "== navigateur : fautes, IPC, [LB], console (fin) =="
  sed -E 's/\x1b\[[0-9;]*m//g; s/^\[[^]]*\]\[[^]]*\]\[FPS:[^]]*\] //' "$LOG" | tr -d '\r' \
    | grep -aE 'js log|\[LB\]|\[LB:(NAV|TAB|FRAME|UI)\]|PROCESS_(FAULT|EXIT|DEATH)|faute de|IPC|Failed|VERIFICATION|PANIC|unavailable|SECURITY-DENY' \
    | grep -avE 'PRESENT onglet|MISS url|STORE url' | tail -60 | sed 's/^/  /' || true
  exit 1
fi
if grep -aq 'panicked at' "$LOG"; then
  echo "panique noyau pendant le WPT" >&2
  exit 1
fi
echo "LADYBIRD_WPT_OK"
