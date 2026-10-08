#!/usr/bin/env bash
# INTEGRITE DU CHEMIN ATA DMA, ET SON CHEMIN D'ERREUR (BOUCHAUD_ATA_INTEGRITE_V1).
#
#   tools/ci/run_ata_integrite.sh BOOTIMAGE
#
# Deux demarrages sur la meme image de donnees, commande noyau `ata-integrite`
# (src/drivers/block/ata_integrite.rs) : huit plages de 1 a 1 024 secteurs,
# franchissant les frontieres de lot du pilote, ecrites par la couche bloc
# (DMA, repli PIO), videes, relues et comparees octet par octet, puis la zone
# rendue intacte.
#
#   1. nominal : aucune corruption, aucune ecriture refusee, la vidange est
#      une vraie barriere, lots DMA ecrits ;
#   2. refus injecte : QEMU blkdebug fait echouer UNE FOIS (EIO) l'ecriture qui
#      couvre le premier secteur de la plage de 256 secteurs. Le pilote doit le
#      VOIR (ATA_DMA echec), refaire ce lot en PIO, et rendre des donnees
#      exactes : aucune corruption silencieuse, aucune panique.
#
# BO_QEMU_KVM=1 : memes demarrages sous KVM. Le delai (commande qui ne finit
# jamais) n'est pas injectable par blkdebug : sa borne (5 s, puis repli) est
# lue dans le code, pas prouvee ici.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_ata_integrite.sh BOOTIMAGE}

SCENARIO=scenario-ata-integrite
IMAGE=ata-integrite.img
CONF=ata-integrite-blkdebug.conf
rm -rf "$SCENARIO" "$IMAGE" "$CONF" serie-ata-integrite-*.log
ACCEL="-cpu max"
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
fi
echo "ATA_INTEGRITE_ACCEL ${ACCEL}"

# Un demarrage : l'autorun lance la commande, puis rend la main.
demarre() {
  local nom=$1 drive=$2 mode=$3 log="serie-ata-integrite-$1.log"
  rm -rf "$SCENARIO" "$IMAGE"
  mkdir -p "$SCENARIO"
  printf 'ata-integrite %s\necho ATA_INTEGRITE_FIN\n' "$mode" > "$SCENARIO/autorun"
  (cd tools/userland && IMAGE="$PWD/../../$IMAGE" ./mkdisk.sh "$PWD/../../$SCENARIO" >/dev/null)
  # La zone persistante est ancree a la FIN du volume : 8 Mio de plus ouvrent
  # entre l'archive et elle l'espace libre ou la zone tampon se place.
  truncate -s +8M "$IMAGE"
  : > "$log"
  # shellcheck disable=SC2086
  qemu-system-x86_64 $ACCEL \
    -drive format=raw,file="$BOOT" \
    -drive "$drive" \
    -m 2048 -smp 4 -display none -no-reboot \
    -serial file:"$log" &
  local pid=$! limite=$((SECONDS + 300))
  while kill -0 "$pid" 2>/dev/null; do
    if (( SECONDS >= limite )); then echo "ECHEANCE atteinte ($nom)" >&2; break; fi
    grep -aqE 'ATA_INTEGRITE_FIN|KERNEL PANIC|panicked at' "$log" && { sleep 1; break; }
    sleep 1
  done
  kill -TERM "$pid" 2>/dev/null || true
  sleep 1
  kill -KILL "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  sed -E 's/\x1b\[[0-9;]*m//g; s/^\[[^]]*\]\[[^]]*\]\[FPS:[^]]*\] //' "$log" \
    | grep -aE '^ATA_INTEGRITE|ATA_DMA |ata: (lecture|ecriture)|KERNEL PANIC|panicked' | sed "s/^/  [$nom] /" || true
}

echo "== 1. nominal =="
demarre nominal "format=raw,file=$IMAGE" ""
L1=serie-ata-integrite-nominal.log
base=$(grep -aoE 'ATA_INTEGRITE base=[0-9]+' "$L1" | head -1 | cut -d= -f2 || true)

echo "== 2. une ecriture refusee (blkdebug, EIO une fois, secteur $(( ${base:-0} + 271 ))) =="
if [ -n "$base" ]; then
  cat > "$CONF" <<CONFIG
[inject-error]
event = "write_aio"
iotype = "write"
errno = "5"
sector = "$(( base + 271 ))"
once = "on"
CONFIG
  demarre refus "driver=raw,file.driver=blkdebug,file.config=$CONF,file.image.filename=$IMAGE" "--refus-attendu"
fi
L2=serie-ata-integrite-refus.log

echo "== verdict =="
echecs=()
exige() { local quoi=$1; shift; if "$@"; then echo "  ok      $quoi"; else echo "  ECHEC   $quoi"; echecs+=("$quoi"); fi; }
dans() { grep -aqE "$2" "$1"; }
exige "nominal : ATA_INTEGRITE_OK (0 corruption, 0 refus, vraie barriere, zone rendue)" dans "$L1" 'ATA_INTEGRITE_OK mode=nominal verifies=2048'
exige "nominal : des lots ecrits en DMA" dans "$L1" 'ATA_INTEGRITE base=.* lots_dma_ecrits=[1-9]'
exige "nominal : aucun echec DMA" bash -c "! grep -aq 'ATA_DMA echec' '$L1'"
exige "refus : la zone tampon a ete situee (base=${base:-?})" test -n "$base"
exige "refus : le pilote a VU l'erreur injectee (ATA_DMA echec)" dans "$L2" 'ATA_DMA echec raison=erreur'
exige "refus : refait en PIO, 0 corruption silencieuse, zone rendue" dans "$L2" 'ATA_INTEGRITE_OK mode=refus-attendu verifies=2048 replis_pio=[1-9]'
exige "aucune panique" bash -c "! grep -aqE 'KERNEL PANIC|panicked at' '$L1' '$L2' 2>/dev/null"
if [ ${#echecs[@]} -eq 0 ]; then
  echo "ATA_INTEGRITE_BANC_OK"
else
  echo "ATA_INTEGRITE_BANC_ECHEC n=${#echecs[@]}"
  exit 1
fi
