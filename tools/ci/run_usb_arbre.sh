#!/usr/bin/env bash
#
# Un clavier et une souris DERRIERE un concentrateur repondent-ils ?
#
# # Pourquoi cette campagne existe
#
# La traversee d'un concentrateur est de l'arithmetique de champs de bits :
# chaine de route, bit `Hub`, transactionneur. Une faute n'y produit AUCUN
# message -- le controleur adresse un peripherique qui n'est pas la, et le
# journal dit « adresse ok ». Les preuves hote verifient chaque champ
# separement ; elles ne peuvent pas verifier que l'ensemble, ecrit dans un
# vrai controleur, atteint un vrai clavier.
#
# QEMU le peut. La topologie ci-dessous branche un concentrateur sur le
# controleur xHCI, et un clavier plus une souris DERRIERE lui. Rien n'est
# branche en direct : si la traversee ne marche pas, il n'y a aucune entree,
# et la campagne est rouge.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_usb_arbre.sh BOOTIMAGE}
LOG=${2:-usb-arbre.log}

rm -rf usb-arbre-scenario usb-arbre.img "$LOG"
python3 - <<'PY'
from pathlib import Path
import tarfile

root = Path("usb-arbre-scenario")
root.mkdir(exist_ok=True)
autorun = root / "autorun"
autorun.write_text(
    "journal off\n"
    "echo USB_ARBRE_BEGIN\n"
    "lsusb\n"
    "echo USB_ARBRE_FIN\n"
    "poweroff\n",
    encoding="ascii",
)
image = Path("usb-arbre.img")
with tarfile.open(image, "w", format=tarfile.USTAR_FORMAT) as tar:
    info = tar.gettarinfo(str(autorun), arcname="./autorun")
    info.uid = info.gid = 0
    info.uname = info.gname = "root"
    info.mode = 0o644
    with autorun.open("rb") as src:
        tar.addfile(info, src)
with image.open("ab") as out:
    out.write(b"\0" * (4 * 1024 * 1024))
PY

# LA TOPOLOGIE, ET CE QU'ELLE PROUVE
#
# Le clavier et la souris sont branches sur les ports 1 et 2 DU
# CONCENTRATEUR, lui-meme sur le port 1 du controleur. Aucun peripherique
# n'est branche en direct : c'est ce qui rend le resultat concluant.
set +e
timeout 120 qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file=usb-arbre.img \
  -m 2048 -display none -no-reboot \
  -device qemu-xhci,id=xhci \
  -device usb-hub,bus=xhci.0,port=1,id=concentrateur \
  -device usb-kbd,bus=xhci.0,port=1.1 \
  -device usb-mouse,bus=xhci.0,port=1.2 \
  -serial file:"$LOG"
code=$?
set -e
cat "$LOG" || true
printf 'qemu exit=%s\n' "$code"

test -s "$LOG" || { echo "aucune sortie serie" >&2; exit 1; }
if grep -aiEq '\*\*\* KERNEL PANIC \*\*\*|DOUBLE FAULT|panicked at' "$LOG"; then
  echo "panic/fault pendant l'enumeration USB" >&2
  exit 1
fi

echecs=0
exige() {
  if grep -aEq "$1" "$LOG"; then
    printf 'ok      %s\n' "$2"
  else
    printf 'ECHEC   %s\n' "$2"
    echecs=$((echecs + 1))
  fi
}

exige 'BOUCHAUD_USB_CONCENTRATEUR ' \
  "le concentrateur est reconnu"
exige 'BOUCHAUD_USB_CONCENTRATEUR_TRAVERSE .*occupes=[1-9]' \
  "la traversee a trouve des ports occupes"
exige 'BOUCHAUD_USB_ADDRESS_OK .*profondeur=1' \
  "un peripherique a ete adresse DERRIERE le concentrateur"
exige 'BOUCHAUD_LSUSB .*non_traverses=0 ' \
  "aucun concentrateur laisse de cote"
exige 'BOUCHAUD_LSUSB .*derriere_concentrateur=[2-9]' \
  "les deux peripheriques sont derriere le concentrateur"
exige 'BOUCHAUD_LSUSB .*claviers=[1-9]' \
  "le clavier derriere le concentrateur repond"
exige 'BOUCHAUD_LSUSB .*souris=[1-9]' \
  "la souris derriere le concentrateur repond"

if [ "$echecs" -ne 0 ]; then
  echo "arbre USB : $echecs verification(s) en echec" >&2
  exit 1
fi
echo "USB_ARBRE_OK"
