#!/usr/bin/env bash
# P10 -- COMPOSITEUR GPU DES FENETRES (BOUCHAUD_GPU_COMPOSITION_V2).
#
#   tools/ci/run_gpu_composition.sh BOOTIMAGE
#
# Trois demarrages QEMU avec `virtio-gpu-gl-pci` (VIRGL : virglrenderer rend
# sur le GL de l'hote ; sous Xvfb sans GPU, llvmpipe), et un sans VIRGL :
#
#   mire   BO_AUTOSTART_APP=/mire-gpu : chaque pixel de la surface encode sa
#          position. La capture de l'ECRAN DE L'HOTE (Xvfb) est decodee :
#          tout pixel de la fenetre doit etre le pixel exact de la surface
#          (carre mobile et curseur exceptes). Exige aussi l'autotest de
#          convention (BLIT GPU relu au pixel).
#   ab     DOOM, `gpu-composition on` puis `off`, meme peripherique : temps
#          processeur de dessin du bureau par trame, pixels clients recopies
#          par le processeur, cadence. CONVAINCANT : dessin /2 au moins,
#          < 1 % des pixels clients, cadence >= 90 %.
#   occlusion  DOOM sous la mire : les pixels caches ne sont plus recopies
#          par le processeur (< 10 % du bras processeur de l'A/B).
#   panne  (si GPU_COMPOSITION_BOOT_PANNE) une trame GPU refusee : repli
#          processeur, reprise du GPU, mire exacte apres la reprise.
#   repli  `virtio-gpu-pci` (sans VIRGL) : le compositeur reste eteint, DOOM
#          tourne en composition processeur, aucun echec.
#
# Prerequis : qemu-system-x86_64 avec virtio-gpu-gl-pci et SDL, Xvfb,
# ImageMagick (`import`), python3-pil, musl-gcc (DOOM et mire).
# BO_QEMU_KVM=1 : memes demarrages sous KVM.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_gpu_composition.sh BOOTIMAGE}
ACCEL="-cpu max"
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
fi
SORTIE=${GPU_COMPOSITION_SORTIE:-$PWD/gpu-composition}
DUREE_AB=${GPU_COMPOSITION_DUREE:-110}
rm -rf "$SORTIE"; mkdir -p "$SORTIE"
AIDE=$(qemu-system-x86_64 -device help 2>&1 || true)
grep -q virtio-gpu-gl-pci <<<"$AIDE" \
  || { echo "GPU_COMPOSITION_ECHEC qemu sans virtio-gpu-gl-pci" >&2; exit 1; }

[ -x target/doom-bouchaud/doom-bouchaud ] || tools/doom/build-linux.sh
musl-gcc -O2 -static-pie -fPIE -o "$SORTIE/mire-gpu" tools/userland/mire-gpu.c

ECRAN=60
demarre() {  # NOM DUREE DEVICE AUTORUN... ; capture a CAPTURE_A secondes si posee
  local nom=$1 duree=$2 dev=$3; shift 3
  local d="$SORTIE/$nom"; mkdir -p "$d/scen"
  cp target/doom-bouchaud/doom-bouchaud "$d/scen/doom"
  cp tools/doom/wads/freedoom1.wad "$d/scen/"
  cp "$SORTIE/mire-gpu" "$d/scen/mire-gpu"
  chmod +x "$d/scen/doom" "$d/scen/mire-gpu"
  : > "$d/scen/autorun"
  for l in "$@"; do echo "$l" >> "$d/scen/autorun"; done
  echo desktop >> "$d/scen/autorun"
  (cd tools/userland && IMAGE="$d/disque.img" ./mkdisk.sh "$d/scen" >/dev/null)
  cp "$BOOT" "$d/boot.bin"
  ECRAN=$((ECRAN + 1))
  Xvfb ":$ECRAN" -screen 0 1920x1200x24 -nolisten tcp >/dev/null 2>&1 &
  local xv=$!
  sleep 1
  DISPLAY=":$ECRAN" timeout "$duree" qemu-system-x86_64 $ACCEL \
    -drive format=raw,file="$d/boot.bin" -drive format=raw,file="$d/disque.img" \
    -m 2048 -smp 2 -serial file:"$d/serie.log" -vga none -device "$dev" \
    -display sdl,gl=on -no-reboot > "$d/qemu.log" 2>&1 &
  local q=$!
  if [ -n "${CAPTURE_A:-}" ]; then
    sleep "$CAPTURE_A"
    DISPLAY=":$ECRAN" import -window root "$d/capture.png" || true
  fi
  wait "$q" || true
  kill "$xv" 2>/dev/null || true
  echo "GPU_COMPOSITION_DEMARRAGE $nom fini ($dev)"
}

echec=0
CAPTURE_A=50 demarre mire 70 virtio-gpu-gl-pci 'export BO_AUTOSTART_APP=/mire-gpu'
python3 tools/ci/analyse_gpu_composition.py mire "$SORTIE/mire/serie.log" "$SORTIE/mire/capture.png" || echec=1

CAPTURE_A=60 demarre ab-gpu "$DUREE_AB" virtio-gpu-gl-pci 'gpu-composition on' 'export BO_AUTOSTART_DOOM=1'
demarre ab-cpu "$DUREE_AB" virtio-gpu-gl-pci 'gpu-composition off' 'export BO_AUTOSTART_DOOM=1'
python3 tools/ci/analyse_gpu_composition.py ab "$SORTIE/ab-gpu/serie.log" "$SORTIE/ab-cpu/serie.log" || echec=1

# Deux fenetres superposees : DOOM cache sous la mire (composee par le GPU).
demarre occlusion 80 virtio-gpu-gl-pci 'export BO_AUTOSTART_DOOM=1' 'export BO_AUTOSTART_APP=/mire-gpu'
python3 tools/ci/analyse_gpu_composition.py occlusion "$SORTIE/occlusion/serie.log" "$SORTIE/ab-cpu/serie.log" || echec=1

demarre repli 80 virtio-gpu-pci 'export BO_AUTOSTART_DOOM=1'
python3 tools/ci/analyse_gpu_composition.py repli "$SORTIE/repli/serie.log" || echec=1

# BOUCHAUD_GPU_COMPOSITION_REPRISE_V1 : image de banc (option
# `banc-gpu-composition-panne`) -- une trame GPU refusee a la 300e, file
# intacte : repli processeur, reprise du GPU 5 s plus tard, mire exacte apres.
if [ -n "${GPU_COMPOSITION_BOOT_PANNE:-}" ]; then
  BOOT_NORMAL=$BOOT
  BOOT=$GPU_COMPOSITION_BOOT_PANNE
  CAPTURE_A=50 demarre panne 70 virtio-gpu-gl-pci 'export BO_AUTOSTART_APP=/mire-gpu'
  BOOT=$BOOT_NORMAL
  python3 tools/ci/analyse_gpu_composition.py panne "$SORTIE/panne/serie.log" "$SORTIE/panne/capture.png" || echec=1
fi

if [ "$echec" = 0 ]; then
  echo "GPU_COMPOSITION_BANC_OK"
else
  echo "GPU_COMPOSITION_BANC_ECHEC"
fi
exit "$echec"
