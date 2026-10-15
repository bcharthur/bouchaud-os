#!/usr/bin/env bash
# P10 -- SCANOUT VIRTIO-GPU 2D (BOUCHAUD_VIRTIO_GPU_2D_V1).
#
#   tools/ci/run_virtio_gpu_scanout.sh BOOTIMAGE
#
# Quatre demarrages QEMU, et des PIXELS -- captures par le moniteur QEMU
# (`screendump`), donc ce que le peripherique presente vraiment :
#
#   mire      -vga std + virtio-gpu : l'autotest 2D du noyau (creation,
#             memoire, scanout, transfert, flush, 7 refus avec leur code)
#             laisse une mire 64x64 a l'ecran virtio. Elle est comparee au
#             pixel pres. BGA reste l'affichage du bureau (politique).
#   virtio    -vga none + virtio-gpu, autorun `desktop` : le bureau presente
#             par TRANSFER_TO_HOST_2D + RESOURCE_FLUSH. Capture de l'ecran
#             virtio.
#   bga       -vga std seul, meme scenario : la reference.
#             virtio et bga doivent montrer le meme bureau (>= 90 % de pixels
#             identiques ; l'horloge et le curseur bougent).
#   aucun     -vga none, sans virtio : le bureau compose sans ecran, une ligne
#             le dit, aucune panique.
#
# Ce n'est PAS une acceleration : aucun rendu n'est fait par le GPU, aucune
# 3D, aucun VIRGL. virtio-gpu (QEMU) n'est pas le GPU de la Trigkey.
# BO_QEMU_KVM=1 : memes demarrages sous KVM.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_virtio_gpu_scanout.sh BOOTIMAGE}
ACCEL="-cpu max"
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
fi
SORTIE=${SCANOUT_SORTIE:-$PWD/scanout-virtio}
rm -rf "$SORTIE"
mkdir -p "$SORTIE/scenario"
: > "$SORTIE/atteints"
# BOUCHAUD_VIRTIO_GPU_TAMPONS_V1 (P10+.2b) : SCANOUT_TAMPON=direct fait
# composer le bureau directement dans la memoire du scanout (sans effet sous
# BGA) ; les memes pixels sont exiges, et zero octet copie.
TAMPON=${SCANOUT_TAMPON:-copie}
printf 'gpu-tampon %s\ndesktop\n' "$TAMPON" > "$SORTIE/scenario/autorun"
(cd tools/userland && IMAGE="$SORTIE/bureau.img" ./mkdisk.sh "$SORTIE/scenario" >/dev/null)
echo "SCANOUT_ACCEL ${ACCEL}"

# Le chemin d'une socket unix est borne a 108 octets : les sockets du moniteur
# vont dans un repertoire court, pas a cote des journaux.
SOCKETS=$(mktemp -d /tmp/bo-scanout.XXXXXX)
trap 'rm -rf "$SOCKETS"' EXIT

propre() { sed -E 's/\x1b\[[0-9;]*m//g; s/^\[[^]]*\]\[[^]]*\]\[FPS:[^]]*\] //' "$1"; }

capture() {
  local mon=$1 fichier=$2 tete=$3
  if [ "$tete" = - ]; then
    python3 tools/ci/qemu_moniteur.py "$mon" "screendump $fichier" || true
  else
    python3 tools/ci/qemu_moniteur.py "$mon" "screendump $fichier $tete" || true
  fi
  # `screendump` rend la main avant la fin de l'ecriture : attendre que la
  # taille cesse de changer.
  local t0=-1 t1=0
  for _ in $(seq 1 10); do
    t1=$(wc -c < "$fichier" 2>/dev/null || echo 0)
    [ "$t1" -gt 0 ] && [ "$t1" = "$t0" ] && break
    t0=$t1; sleep 1
  done
}

# demarre NOM ATTENTE_REGEX DELAI_APRES CAPTURE_DEVICE ARGS_QEMU...
#   SECONDE=N   : seconde capture N secondes apres la premiere (capture-NOM-2)
#   FINAL=REGEX : apres les captures, attendre encore cette ligne (<= 90 s)
demarre() {
  local nom=$1 attente=$2 apres=$3 tete=$4; shift 4
  local log="$SORTIE/serie-$nom.log" mon="$SOCKETS/$nom.sock"
  : > "$log"
  rm -f "$mon"
  # shellcheck disable=SC2086
  qemu-system-x86_64 $ACCEL -drive format=raw,file="$BOOT" -m 2048 -smp 2 \
    -display none -no-reboot "$@" \
    -monitor "unix:$mon,server,nowait" -serial file:"$log" &
  local pid=$! limite=$((SECONDS + 240)) vu=0
  while kill -0 "$pid" 2>/dev/null && (( SECONDS < limite )); do
    if grep -aqE 'KERNEL PANIC|panicked at' "$log"; then break; fi
    if grep -aqE "$attente" "$log"; then vu=1; break; fi
    sleep 1
  done
  if [ "$vu" = 1 ]; then
    # Le demarrage a atteint son point de capture : sans cette trace, un
    # QEMU qui ne demarre pas laisserait des journaux vides -- et les
    # verifications « rien de mauvais n'apparait » passeraient a vide.
    echo "$nom" >> "$SORTIE/atteints"
    sleep "$apres"
    capture "$mon" "$SORTIE/capture-$nom.ppm" "$tete"
    if [ -n "${SECONDE:-}" ]; then
      sleep "$SECONDE"
      capture "$mon" "$SORTIE/capture-$nom-2.ppm" "$tete"
    fi
    if [ -n "${FINAL:-}" ]; then
      local fin=$((SECONDS + 90))
      while kill -0 "$pid" 2>/dev/null && (( SECONDS < fin )); do
        grep -aqE 'KERNEL PANIC|panicked at' "$log" && break
        grep -aqE "$FINAL" "$log" && break
        sleep 1
      done
    fi
  fi
  kill -KILL "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  propre "$log" | grep -aE 'VIRTIO_GPU|\[gfx\]|KERNEL PANIC|panicked' | sed "s/^/  [$nom] /" | head -16 || true
}

demarre mire 'BOUCHAUD_VIRTIO_GPU_2D_AUTOTEST_OK|VIRTIO_GPU_2D_AUTOTEST_ECHEC|VIRTIO_GPU_ECHEC' 2 vgpu \
  -device virtio-gpu-pci,id=vgpu
SECONDE=3 FINAL='\[GUI-VIRTIO-GPU\]' demarre virtio '\[gui\] window manager demarre' 8 vgpu \
  -vga none -device virtio-gpu-pci,id=vgpu -drive format=raw,file="$SORTIE/bureau.img"
demarre bga '\[gui\] window manager demarre' 8 - \
  -drive format=raw,file="$SORTIE/bureau.img"
demarre aucun '\[gui\] window manager demarre' 3 - \
  -vga none -drive format=raw,file="$SORTIE/bureau.img"

echo "== verdict =="
echecs=0
exige() { local quoi=$1; shift; if "$@"; then echo "  ok      $quoi"; else echo "  ECHEC   $quoi"; echecs=$((echecs + 1)); fi; }
for nom in mire virtio bga aucun; do
  exige "demarrage $nom : point de capture atteint" grep -qx "$nom" "$SORTIE/atteints"
done
exige "autotest 2D : chemin positif complet" grep -aq 'VIRTIO_GPU_2D positif creation+attache+scanout+transfert+flush ok' "$SORTIE/serie-mire.log"
exige "autotest 2D : 7 refus, chacun avec son code" grep -aq 'VIRTIO_GPU_2D negatifs refuses_avec_le_bon_code=7' "$SORTIE/serie-mire.log"
exige "mire a l'ecran virtio, exacte au pixel" python3 tools/ci/analyse_scanout.py mire "$SORTIE/capture-mire.ppm"
exige "avec BGA present, le bureau reste sur BGA" bash -c "grep -aq '\[gfx\] BGA HD actif' '$SORTIE/serie-bga.log' && ! grep -aq 'BOUCHAUD_VIRTIO_GPU_SCANOUT_ACTIF' '$SORTIE/serie-mire.log' '$SORTIE/serie-bga.log'"
exige "sans BGA, le bureau prend le scanout virtio-gpu 2D" grep -aq 'BOUCHAUD_VIRTIO_GPU_SCANOUT_ACTIF 1280x720' "$SORTIE/serie-virtio.log"
exige "bureau virtio : capture 1280x720" python3 tools/ci/analyse_scanout.py taille "$SORTIE/capture-virtio.ppm" 1280 720
exige "bureau virtio = bureau BGA (pixels)" python3 tools/ci/analyse_scanout.py compare "$SORTIE/capture-virtio.ppm" "$SORTIE/capture-bga.ppm"
exige "bureau virtio vivant : l'horloge avance a l'ecran (presentations partielles)" python3 tools/ci/analyse_scanout.py vivant "$SORTIE/capture-virtio.ppm" "$SORTIE/capture-virtio-2.ppm"
MESURE=$(propre "$SORTIE/serie-virtio.log" | grep -a '\[GUI-VIRTIO-GPU\]' | tail -1 || true)
echo "  mesure  ${MESURE:-aucune ligne [GUI-VIRTIO-GPU]}"
exige "presentations virtio : aucune en echec, au moins une reussie" bash -c "echo '$MESURE' | grep -qE 'presents_ok=[1-9][0-9]* presents_echec=0 .* actif=1'"
# BOUCHAUD_VIRTIO_GPU_VERROU_DORMANT_V1 : le compositeur presente depuis sa
# tache, sans verrou tournant tenu -- il prend le verrou dormant (jamais de
# presentation sautee) et peut ceder pendant l'attente.
exige "presentations virtio : aucune sautee faute de verrou" bash -c "echo '$MESURE' | grep -qE ' sautes=0 cedes=[0-9]+ non_cedables=[0-9]+'"
exige "aucune attente non cedable DANS une tache (verrou tournant tenu par un appelant)" bash -c "! grep -aq 'VIRTIO_GPU_ATTENTE non_cedable_en_tache' '$SORTIE'/serie-*.log"
if [ "$TAMPON" = direct ]; then
  exige "composition directe : le bureau virtio compose dans la memoire du scanout, zero octet copie, aucun acces pendant un transfert" \
    bash -c "echo '$MESURE' | grep -qE ' tampon=direct octets_copies=0 octets_directs=[1-9][0-9]* acces_en_transfert=0'"
else
  exige "copie : le bureau virtio copie son tampon arriere (politique par defaut)" \
    bash -c "echo '$MESURE' | grep -qE ' tampon=copie octets_copies=[1-9][0-9]* octets_directs=0 acces_en_transfert=0'"
fi
exige "ni BGA ni virtio : present() inactif, dit" grep -aq 'present() inactif (scanout=aucun)' "$SORTIE/serie-aucun.log"
exige "aucune panique, aucun abandon du scanout" bash -c "! grep -aqE 'KERNEL PANIC|panicked at|VIRTIO_GPU_SCANOUT_ABANDON|VIRTIO_GPU_ECHEC' '$SORTIE'/serie-*.log"
if [ "$echecs" -eq 0 ]; then
  echo "VIRTIO_GPU_SCANOUT_BANC_OK"
else
  echo "VIRTIO_GPU_SCANOUT_BANC_ECHEC n=$echecs"
  exit 1
fi
