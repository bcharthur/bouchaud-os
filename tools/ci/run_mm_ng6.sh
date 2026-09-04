#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_mm_ng6.sh BOOTIMAGE}

rm -rf scenario mm-ng6.img mm-ng6.log mmstress mmstress.sha256 mmstress.readelf.txt
CC=musl-gcc tools/userland/build-mmstress.sh
file mmstress | grep -E 'static-pie linked|statically linked'
readelf -h -l mmstress > mmstress.readelf.txt
if grep -q 'Requesting program interpreter' mmstress.readelf.txt; then
  echo 'mmstress must use the static Bouchaud-compatible Linux ABI' >&2
  exit 1
fi
truncate -s 5M mmstress
sha256sum mmstress > mmstress.sha256

mkdir -p scenario/bin
cp mmstress scenario/bin/mmstress
# `smpstat` encadre la charge. C'est ce qui rend les budgets d'EXECUTION
# verifiables : sans lui, le releve periodique ne sort jamais dans cette trace,
# les treize grandeurs ressortent « absent du journal », et
# `check_budgets.py --journal` passe au vert sans avoir rien mesure.
#
# Deux appels, pas un : le premier donne la ligne de base du boot, le second
# l'etat apres la charge. Les budgets se jugent sur le pire des deux, donc sur
# ce que le stress a reellement produit.
cat > scenario/autorun <<'AUTORUN'
smpstat
echo MMSTRESS_BASIC_BEGIN && /bin/mmstress 4 512 4 && echo MMSTRESS_BASIC_OK
echo MMSTRESS_UNRELATED_BEGIN && /bin/mmstress unrelated && echo MMSTRESS_UNRELATED_OK
echo MMSTRESS_ABA_BEGIN && /bin/mmstress aba && echo MMSTRESS_ABA_OK
echo MMSTRESS_CHURN_BEGIN && /bin/mmstress churn && echo MMSTRESS_CHURN_OK_GUEST
smpstat
echo MMSTRESS_NG6_ALL_DONE
AUTORUN
(cd tools/userland && IMAGE="$PWD/../../mm-ng6.img" ./mkdisk.sh "$PWD/../../scenario")

set +e
timeout 300 qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file=mm-ng6.img \
  -m 2048 -smp 4 -accel tcg -cpu max -display none -no-reboot \
  -netdev user,id=net0 -device e1000,netdev=net0 \
  -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
  -serial file:mm-ng6.log
code=$?
set -e
cat mm-ng6.log || true
[ "$code" -ne 124 ] || { echo 'QEMU MM stress timed out' >&2; exit 1; }
[ "$code" -eq 33 ] || { echo "unexpected QEMU guest exit code: $code (expected 33)" >&2; exit 1; }

for marker in \
  MMSTRESS_BASIC_BEGIN MMSTRESS_BASIC_OK \
  MMSTRESS_UNRELATED_BEGIN MMSTRESS_UNRELATED_OK \
  MMSTRESS_ABA_BEGIN MMSTRESS_ABA_OK \
  MMSTRESS_CHURN_BEGIN MMSTRESS_CHURN_OK_GUEST \
  MMSTRESS_NG6_ALL_DONE; do
  grep -aF "$marker" mm-ng6.log
 done
grep -aF '=== AUTORUN FIN === statut=0' mm-ng6.log

# Le releve doit avoir atteint le port serie, sinon les budgets d'execution qui
# suivent n'auraient rien a lire. On le verifie ICI, ou l'echec nomme sa cause,
# plutot que de laisser `check_budgets.py` annoncer treize absences.
for tag in '[BKL-DOMAINES]' '[SCHED-NG-LAT]' '[SCHED-NG-CENTILES]' '[SCHED-NG-FILE]'; do
  grep -aqF "$tag" mm-ng6.log || {
    echo "releve periodique absent de la trace : $tag -- smpstat n'a rien emis" >&2
    exit 1
  }
done
if grep -aqE '\*\*\* KERNEL PANIC \*\*\*|DOUBLE FAULT|FAULT_FATAL|faute de page utilisateur|assertion failed|panicked at' mm-ng6.log; then
  echo 'fatal kernel or user fault in MM stress log' >&2
  exit 1
fi
echo MM_NG6_OK
