#!/usr/bin/env bash
# Un fichier ordinaire se lit et s'ecrit EXACTEMENT (BOUCHAUD_LECTURE_EXACTE_V1,
# BOUCHAUD_COPIE_BORNEE_V1).
#
# Trouves en faisant tourner OpenJDK 21 sous Bouchaud :
#   - un `read()` de 140 Mio faisait paniquer le noyau (allocation sans borne) ;
#   - `read`/`pread` rendaient des lectures courtes au milieu du fichier, a
#     chaque frontiere d'etendue de 64 Kio du disque paresseux : la JVM lisait
#     ses classes tronquees (ClassFormatError).
#
# Le banc ne depend d'aucune JVM : un fichier pseudo-aleatoire de 150 Mio, une
# sonde ring 3 qui compare read/pread/lseek+read/write a la projection mmap.
#
#   tools/ci/run_lecture_fichiers.sh BOOTIMAGE [SORTIE]
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_lecture_fichiers.sh BOOTIMAGE [SORTIE]}
SORTIE=${2:-target/lecture-fichiers}
rm -rf "$SORTIE"; mkdir -p "$SORTIE/scen"
SORTIE=$(cd "$SORTIE" && pwd)

command -v gcc >/dev/null || { echo "LECTURE_FICHIERS_ECHEC gcc absent" >&2; exit 1; }
gcc -O2 -static-pie -fPIE -o "$SORTIE/scen/lecture-probe" tools/userland/lecture-fichiers-probe.c
# 150 Mio + 12345 octets : au-dela du plus grand bloc contigu du tas noyau
# (128 Mio) et pas un multiple de 64 Kio.
python3 - "$SORTIE/scen/gros.bin" <<'PY'
import random, sys
r = random.Random(20261118)
with open(sys.argv[1], "wb") as f:
    reste = 150 * 1024 * 1024 + 12345
    while reste:
        n = min(reste, 1 << 20)
        f.write(r.randbytes(n))
        reste -= n
PY
printf 'echo LECTURE_DEBUT\n/lecture-probe /gros.bin\necho LECTURE_BANC_FIN\n' > "$SORTIE/scen/autorun"
(cd tools/userland && IMAGE="$SORTIE/disque.img" ./mkdisk.sh "$SORTIE/scen" >/dev/null)

LOG="$SORTIE/serie.log"
: > "$LOG"
qemu-system-x86_64 -drive format=raw,file="$BOOT" -drive format=raw,file="$SORTIE/disque.img" \
  -m 4096 -smp 2 -display none -no-reboot -serial file:"$LOG" &
QEMU=$!
LIMITE=$((SECONDS + ${BO_LECTURE_S:-600}))
while kill -0 "$QEMU" 2>/dev/null; do
  grep -aq 'LECTURE_BANC_FIN\|KERNEL PANIC' "$LOG" && { sleep 1; break; }
  (( SECONDS >= LIMITE )) && break
  sleep 2
done
kill "$QEMU" 2>/dev/null || true
wait "$QEMU" 2>/dev/null || true

P="$SORTIE/serie.propre"
sed 's/\x1b\[[0-9;]*m//g' "$LOG" | tr -d '\r' > "$P"
grep -a 'LECTURE_\|KERNEL PANIC\|panicked' "$P" | sed 's/^\[[^]]*\]\[[^]]*\]\[[^]]*\] //' || true

echecs=()
grep -aq 'KERNEL PANIC' "$P" && echecs+=(panique_noyau)
grep -aqE 'LECTURE_GROS_READ .*egal_mmap=1' "$P" || echecs+=(gros_read)
grep -aqE 'LECTURE_GROS_WRITE .*egal=1' "$P" || echecs+=(gros_write)
grep -aqE 'LECTURE_FIN pread_faux=0 read_faux=0' "$P" || echecs+=(lectures_inexactes)
if [ ${#echecs[@]} -ne 0 ]; then
  echo "LECTURE_FICHIERS_ECHEC ${echecs[*]}"
  exit 1
fi
echo LECTURE_FICHIERS_OK
