#!/usr/bin/env bash
# EXTINCTION PENDANT QUE DES ORPHELINS TIENNENT LE DISQUE
# (BOUCHAUD_EXTINCTION_FIL_NOYAU_V1).
#
#   tools/ci/run_extinction_contention.sh BOOTIMAGE
#
# Un demarrage, une sonde : extinction-contention-probe laisse trois fils
# orphelins qui bouclent sur ecriture + fsync sous /persist, puis sort.
# L'autorun se termine et l'extinction tombe en pleine contention sur la
# persistance et le controleur ATA.
#
# Sans BOUCHAUD_EXTINCTION_FIL_NOYAU_V1, l'extinction attendait ces verrous
# dormants depuis le contexte d'amorcage (aucune tache courante) :
#
#     panicked at src/kernel/process/thread/blocage.rs:105:5:
#     task: parking demande hors de toute tache
#
# (sites reels, run 38475302676 ; 3/3 en local a SMP1 avant le correctif).
#
# BO_SMP : nombre de coeurs (1 par defaut : la contention y est certaine).
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_extinction_contention.sh BOOTIMAGE}
SMP=${BO_SMP:-1}

SCENARIO=scenario-extinction-contention
IMAGE=extinction-contention.img
LOG=serie-extinction-contention.log
rm -rf "$SCENARIO" "$IMAGE" "$LOG" tools/userland/out-extinction-contention
(cd tools/userland && OUT=out-extinction-contention ./build.sh musl >/dev/null)
mkdir -p "$SCENARIO/bin"
cp tools/userland/out-extinction-contention/extinction-contention-probe "$SCENARIO/bin/"
printf '/bin/extinction-contention-probe 3\necho EXTINCTION_CONTENTION_AUTORUN_FIN\n' > "$SCENARIO/autorun"
(cd tools/userland && IMAGE="$PWD/../../$IMAGE" ./mkdisk.sh "$PWD/../../$SCENARIO" >/dev/null)

echo "EXTINCTION_CONTENTION smp=${SMP}"
: > "$LOG"
qemu-system-x86_64 -cpu max \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$IMAGE" \
  -m 2048 -smp "$SMP" -display none -no-reboot \
  -serial file:"$LOG" &
PID=$!
LIMITE=$((SECONDS + 300))
echeance=0
while kill -0 "$PID" 2>/dev/null; do
  if (( SECONDS >= LIMITE )); then echeance=1; echo "ECHEANCE atteinte" >&2; break; fi
  grep -aqE 'KERNEL PANIC|panicked at' "$LOG" && { sleep 2; break; }
  sleep 1
done
# QEMU doit s'etre arrete DE LUI-MEME (extinction ACPI) : s'il vit encore,
# la machine n'a pas fini de s'eteindre.
eteinte_seule=1
if kill -0 "$PID" 2>/dev/null; then
  eteinte_seule=0
  kill -TERM "$PID" 2>/dev/null || true
  sleep 1
  kill -KILL "$PID" 2>/dev/null || true
fi
wait "$PID" 2>/dev/null || true

propre() { sed -E 's/\x1b\[[0-9;]*m//g; s/^\[[^]]*\]\[[^]]*\]\[FPS:[^]]*\] //' "$LOG"; }
propre | grep -aE 'EXTINCTION_CONTENTION|SYSTEM_EXIT|\[kernel\] extinction|persistance:|EXTINCTION_RAPPORT DERNIER|KERNEL PANIC|panicked at' | head -20 || true
propre | grep -aA3 'panicked at' | head -8 || true

echecs=()
exige() { local quoi=$1; shift; if "$@"; then echo "  ok      $quoi"; else echo "  ECHEC   $quoi"; echecs+=("$quoi"); fi; }
exige "aucune panique noyau" bash -c "! grep -aqE 'KERNEL PANIC|panicked at' '$LOG'"
exige "la contention est en place (3 orphelins)" grep -aq 'EXTINCTION_CONTENTION_PRETE fils=3' "$LOG"
exige "l'extinction est confiee a un fil noyau" grep -aq 'extinction: contexte sans tache, confiee a un fil noyau' "$LOG"
exige "la persistance est ecrite a l'extinction" grep -aqE 'persistance: [0-9]+ fichier\(s\) ecrit\(s\) a l.extinction' "$LOG"
exige "le rapport d'extinction est emis" grep -aq 'BOUCHAUD_EXTINCTION_RAPPORT DERNIER' "$LOG"
exige "QEMU s'eteint de lui-meme avant l'echeance" test "$eteinte_seule" = 1 -a "$echeance" = 0
if [ ${#echecs[@]} -eq 0 ]; then
  echo "EXTINCTION_CONTENTION_BANC_OK"
else
  echo "EXTINCTION_CONTENTION_BANC_ECHEC n=${#echecs[@]}"
  exit 1
fi
