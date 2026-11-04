#!/usr/bin/env bash
#
# CAPTURE REELLE DU BUREAU (BOUCHAUD_ATELIER_CAPTURE_V1).
#
#   tools/ci/run_bureau_capture.sh [BOOTIMAGE] [SORTIE]
#
# Demarre le bureau dans QEMU (TCG), attend la premiere trame, laisse le
# bureau se poser, capture le tampon video par le moniteur (`screendump`) et
# le convertit en PNG. Aucune maquette : c'est le framebuffer de l'OS.
#
#   BUREAU_EXPORTS   lignes `export X=Y` ajoutees a l'autorun (ex. ouvrir
#                    une application au demarrage par le chemin de son icone)
#   BUREAU_ATTENTE_S secondes apres la premiere trame (defaut 8)
#   BUREAU_QEMU      options QEMU ajoutees (ex. `-nic none` : aucune carte
#                    reseau, pour capturer l'etat « Pas de carte »)
#   BUREAU_GESTES    scenario d'interaction REEL (souris, clavier, captures
#                    a la volee), joue apres l'attente et la capture
#                    `bureau.png` : voir tools/ci/qemu_gestes.py
set -uo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:-target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin}
SORTIE=${2:-$(mktemp -d)}
mkdir -p "$SORTIE"
ATTENTE=${BUREAU_ATTENTE_S:-8}
[ -s "$BOOT" ] || { echo "image absente : $BOOT (tools/ci/build_kernel.sh)" >&2; exit 1; }
SCENARIO="$SORTIE/scenario"; rm -rf "$SCENARIO"; mkdir -p "$SCENARIO"
{ printf '%s\n' "${BUREAU_EXPORTS:-}" | sed '/^$/d'; echo desktop; } > "$SCENARIO/autorun"
(cd tools/userland && IMAGE="$SORTIE/scenario.img" ./mkdisk.sh "$SCENARIO") >/dev/null 2>&1 \
    || { echo "mkdisk a echoue" >&2; exit 1; }
# Un socket Unix ne depasse pas 108 octets : un SORTIE profond le rendait
# invalide et QEMU refusait de demarrer. Chemin court, retire a la sortie.
MONITEUR=$(mktemp -u /tmp/bo-moniteur.XXXXXX); LOG="$SORTIE/serie.log"; : > "$LOG"
qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" -drive format=raw,file="$SORTIE/scenario.img" \
  -m 2048 -smp 2 -cpu max -display none -no-reboot \
  -monitor "unix:$MONITEUR,server,nowait" -serial file:"$LOG" ${BUREAU_QEMU:-} &
PID=$!
trap 'kill $PID 2>/dev/null; wait $PID 2>/dev/null; rm -f "$MONITEUR"' EXIT
pret=0
for _ in $(seq 1 120); do
  grep -aq "BOUCHAUD_BOOT_POINT bureau-premiere-trame" "$LOG" 2>/dev/null && { pret=1; break; }
  kill -0 "$PID" 2>/dev/null || { echo "QEMU arrete avant le bureau" >&2; tail -20 "$LOG" >&2; exit 1; }
  sleep 1
done
[ "$pret" = 1 ] || { echo "pas de premiere trame en 120 s" >&2; tail -20 "$LOG" >&2; exit 1; }
sleep "$ATTENTE"
python3 tools/ci/qemu_moniteur.py "$MONITEUR" "screendump $SORTIE/bureau.ppm" >/dev/null
sleep 2
[ -s "$SORTIE/bureau.ppm" ] || { echo "aucune capture" >&2; exit 1; }
python3 tools/ci/ppm-vers-png.py "$SORTIE/bureau.ppm" "$SORTIE/bureau.png" || exit 1
if [ -n "${BUREAU_GESTES:-}" ]; then
  # Une seule connexion au moniteur : les gestes s'enchainent assez vite pour
  # saisir une animation en cours (voir tools/ci/qemu_gestes.py).
  printf '%s\n' "$BUREAU_GESTES" | python3 tools/ci/qemu_gestes.py "$MONITEUR" "$SORTIE" || exit 1
  sleep 1
  for ppm in "$SORTIE"/*.ppm; do
    [ "$(basename "$ppm")" = bureau.ppm ] && continue
    python3 tools/ci/ppm-vers-png.py "$ppm" "${ppm%.ppm}.png" >/dev/null || exit 1
    echo "BUREAU_CAPTURE=${ppm%.ppm}.png"
  done
fi
if grep -aE "KERNEL PANIC|panicked at" "$LOG" >/dev/null; then echo "BUREAU_CAPTURE_PANIQUE" >&2; exit 1; fi
echo "BUREAU_CAPTURE=$SORTIE/bureau.png"
