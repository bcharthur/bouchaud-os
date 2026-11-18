#!/usr/bin/env bash
# BAR PCI au-dela de la fenetre physique du chargeur (BOUCHAUD_MMIO_HORS_FENETRE_V1).
#
# ## L'incident
#
# main@4307cf6f sous WHPX (Ryzen 7 7800X3D, 18/11) : panique au demarrage,
# avant le bureau. Faute de page CR2=0x3a8000000000 dans `xhci_probe::probe`,
# au premier octet lu des registres du xHCI.
#
# L'hote expose 46 bits d'adresse physique : OVMF place sa fenetre PCI 64 bits
# a 56 Tio, et la BAR0 du xHCI a 0x380000000000. Les pilotes lisaient leurs
# registres a `phys_offset + BAR`, comme si la fenetre physique du chargeur
# couvrait tout -- elle ne couvre que la RAM et les 4 premiers Gio. Sous TCG
# (40 bits), la fenetre PCI tombe sous 1 Tio, que la carte memoire d'OVMF
# couvre : les BAR y etaient par chance, et aucun banc ne le voyait.
#
# ## Ce que le banc prouve
#
# L'image Stage 2 UEFI, sous OVMF, avec `-cpu max,phys-bits=46` : la condition
# exacte de WHPX, sous TCG. Il exige
#
#   - que la condition soit REPRODUITE : BAR0 du xHCI au-dela de 1 Tio
#     (sinon le banc ne prouve rien et le dit) ;
#   - clavier et souris USB, NVMe et e1000 operationnels ;
#   - la premiere trame du bureau ;
#   - aucune faute de page, aucune panique, aucun refus de mappage.
#
#   tools/ci/run_bar_haute.sh [SORTIE]
#   BO_BAR_HAUTE_ELF=noyau.elf tools/ci/run_bar_haute.sh   # noyau deja construit
set -euo pipefail
cd "$(dirname "$0")/../.."
SORTIE=${1:-target/bar-haute}
TOOLCHAIN=$(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)
FEATURES=uefi-boot,reference-bringup,reference-desktop
OVMF_CODE=${OVMF_CODE:-/usr/share/OVMF/OVMF_CODE_4M.fd}
OVMF_VARS=${OVMF_VARS:-/usr/share/OVMF/OVMF_VARS_4M.fd}
DELAI_S=${BO_BAR_HAUTE_S:-240}
rm -rf "$SORTIE"; mkdir -p "$SORTIE"
SORTIE=$(cd "$SORTIE" && pwd)

for f in "$OVMF_CODE" "$OVMF_VARS"; do
  [ -f "$f" ] || { echo "BAR_HAUTE_ECHEC ovmf_absent=$f" >&2; exit 1; }
done

NOYAU=${BO_BAR_HAUTE_ELF:-}
if [ -z "$NOYAU" ]; then
  cargo build --target targets/x86_64-bouchaud_os_uefi.json --no-default-features --features "$FEATURES" >/dev/null
  NOYAU=target/x86_64-bouchaud_os_uefi/debug/bouchaud-os
fi

TRAVAIL=$(mktemp -d)
trap 'rm -rf "$TRAVAIL"' EXIT
cp -r tools/reference/uefi-image-builder "$TRAVAIL/builder"
( cd "$TRAVAIL/builder" && cargo +"$TOOLCHAIN" build --release -q )
"$TRAVAIL/builder/target/release/bouchaud-reference-uefi-image-builder" "$NOYAU" "$SORTIE/stage2.img" >/dev/null

cp "$OVMF_VARS" "$SORTIE/vars.fd"
truncate -s 64M "$SORTIE/donnees.img"
truncate -s 256M "$SORTIE/nvme.img"
LOG="$SORTIE/serie.log"
: > "$LOG"

# Les peripheriques du lanceur Windows (run-reference-stage2-ladybird.ps1),
# plus un NVMe : son pilote lit lui aussi une BAR 64 bits.
qemu-system-x86_64 -machine pc -m 12288 -smp 1 \
  -drive if=pflash,format=raw,unit=0,readonly=on,file="$OVMF_CODE" \
  -drive if=pflash,format=raw,unit=1,file="$SORTIE/vars.fd" \
  -drive if=ide,index=0,media=disk,format=raw,file="$SORTIE/stage2.img",snapshot=on \
  -drive if=ide,index=1,media=disk,format=raw,file="$SORTIE/donnees.img",snapshot=on \
  -drive if=none,id=nv,format=raw,file="$SORTIE/nvme.img",snapshot=on -device nvme,drive=nv,serial=bo \
  -netdev user,id=net0 -device e1000,netdev=net0 -vga std \
  -device qemu-xhci,id=xhci -device usb-kbd,bus=xhci.0 -device usb-mouse,bus=xhci.0 \
  -serial file:"$LOG" -monitor none -display none -no-reboot -no-shutdown \
  -accel tcg -cpu max,phys-bits=46 &
QEMU=$!
LIMITE=$((SECONDS + DELAI_S))
while kill -0 "$QEMU" 2>/dev/null; do
  grep -aq 'bureau-premiere-trame\|KERNEL PANIC' "$LOG" && { sleep 3; break; }
  (( SECONDS >= LIMITE )) && break
  sleep 2
done
kill "$QEMU" 2>/dev/null || true
wait "$QEMU" 2>/dev/null || true

P="$SORTIE/serie.propre"
sed 's/\x1b\[[0-9;]*m//g' "$LOG" | tr -d '\r' > "$P"
grep -a 'phys_offset=\|XHCI_CONTROLLER_DISCOVERED\|PCI-NG\] nvme\|MMIO_REFUSE\|HID_KEYBOARD_GREEN\|HID_MOUSE_GREEN\|NVME_GREEN\|e1000: \|bureau-premiere-trame\|PAGE FAULT\|KERNEL PANIC\|cr2=' "$P" \
  | sed 's/^\[[^]]*\]\[[^]]*\]\[[^]]*\] //' | awk 'NR <= 20' || true

echecs=()
bar=$(grep -aoE 'XHCI_CONTROLLER_DISCOVERED .*bar0=0x[0-9a-f]+' "$P" | grep -oE 'bar0=0x[0-9a-f]+' | awk 'NR == 1' | cut -d= -f2 || true)
if [ -z "$bar" ]; then
  echecs+=("xhci_non_decouvert")
elif (( bar < 0x10000000000 )); then
  # La condition n'est pas reproduite : un vert ici ne prouverait rien.
  echecs+=("condition_non_reproduite_bar0=$bar")
fi
for m in HID_KEYBOARD_GREEN HID_MOUSE_GREEN NVME_GREEN 'e1000: initialise' bureau-premiere-trame; do
  grep -aqF "$m" "$P" || echecs+=("absent:${m// /_}")
done
for m in 'PAGE FAULT' 'KERNEL PANIC' MMIO_REFUSE; do
  if grep -aqF "$m" "$P"; then echecs+=("present:${m// /_}"); fi
done

if [ ${#echecs[@]} -ne 0 ]; then
  echo "BAR_HAUTE_ECHEC ${echecs[*]}"
  exit 1
fi
echo "BAR_HAUTE_OK bar0_xhci=$bar"
