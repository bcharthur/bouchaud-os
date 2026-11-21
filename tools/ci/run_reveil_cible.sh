#!/usr/bin/env bash
# Un message IPC ne reveille QUE son destinataire (BOUCHAUD_REVEIL_CIBLE_V1).
#
# Avant : chaque ecriture dans un tube, un canal de socketpair ou un eventfd
# reveillait TOUTES les attentes poll/select/epoll du systeme. La sonde
# `reveil-cible-probe` endort 12 processus en poll(-1) sur leurs propres
# canaux, puis fait 20 000 allers-retours IPC entre deux autres. Les
# inactifs ne doivent rien consommer : chacun moins de 50 ms de CPU.
#
#   tools/ci/run_reveil_cible.sh BOOTIMAGE [SORTIE]
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_reveil_cible.sh BOOTIMAGE [SORTIE]}
SORTIE=${2:-target/reveil-cible}
SMP=${BO_REVEIL_CIBLE_SMP:-2}
SEUIL_US=${BO_REVEIL_CIBLE_SEUIL_US:-50000}
rm -rf "$SORTIE"; mkdir -p "$SORTIE/scen"
SORTIE=$(cd "$SORTIE" && pwd)

command -v gcc >/dev/null || { echo "REVEIL_CIBLE_ECHEC gcc absent" >&2; exit 1; }
gcc -O2 -static-pie -fPIE -o "$SORTIE/scen/reveil-cible-probe" tools/userland/reveil-cible-probe.c
printf 'echo REVEIL_DEBUT\n/reveil-cible-probe\necho REVEIL_BANC_FIN\n' > "$SORTIE/scen/autorun"
(cd tools/userland && IMAGE="$SORTIE/disque.img" ./mkdisk.sh "$SORTIE/scen" >/dev/null)

LOG="$SORTIE/serie.log"
: > "$LOG"
qemu-system-x86_64 -drive format=raw,file="$BOOT" -drive format=raw,file="$SORTIE/disque.img" \
  -m 2048 -smp "$SMP" -display none -no-reboot -serial file:"$LOG" &
QEMU=$!
LIMITE=$((SECONDS + ${BO_REVEIL_CIBLE_S:-600}))
while kill -0 "$QEMU" 2>/dev/null; do
  grep -aq 'REVEIL_BANC_FIN\|KERNEL PANIC' "$LOG" && { sleep 1; break; }
  (( SECONDS >= LIMITE )) && break
  sleep 2
done
kill "$QEMU" 2>/dev/null || true
wait "$QEMU" 2>/dev/null || true

P="$SORTIE/serie.propre"
sed 's/\x1b\[[0-9;]*m//g' "$LOG" | tr -d '\r' > "$P"
grep -a 'REVEIL_CIBLE\|\[DISPO\]\|KERNEL PANIC' "$P" | sed 's/^\[[^]]*\]\[[^]]*\]\[[^]]*\] //' | tail -4 || true

echecs=()
grep -aq 'KERNEL PANIC' "$P" && echecs+=(panique_noyau)
ligne=$(grep -ao 'REVEIL_CIBLE inactifs=.*' "$P" | awk 'NR == 1' || true)
if [ -z "$ligne" ]; then
  echecs+=(sonde_sans_resultat)
else
  pire=$(echo "$ligne" | grep -oE 'cpu_max_inactif_us=[0-9]+' | cut -d= -f2)
  (( pire < SEUIL_US )) || echecs+=("inactif_reveille:${pire}us>=${SEUIL_US}us")
fi
if [ ${#echecs[@]} -ne 0 ]; then
  echo "REVEIL_CIBLE_ECHEC ${echecs[*]}"
  exit 1
fi
echo "REVEIL_CIBLE_OK $ligne"
