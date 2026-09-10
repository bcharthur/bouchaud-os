#!/usr/bin/env bash
#
# Le transport USB Bulk-Only, EXERCE de bout en bout.
#
# # Ce qu'aucune campagne ne faisait
#
# `xhci_active.rs` porte environ mille lignes de transport Bulk et de commandes
# SCSI : CBW, CSW, signature, etiquette, residu, READ CAPACITY, READ(10),
# reinitialisation Bulk-Only, Reset Endpoint, Set TR Dequeue. Aucune campagne
# ne branchait de peripherique de stockage. Ces mille lignes n'etaient donc
# executees NULLE PART -- ni ici, ni sur le materiel, ou seul le cas de
# l'enregistreur de vol les touche.
#
# Une preuve de decodage hote ne vaut pas pour ce chemin : le decodage d'un CSW
# se verifie a la table, mais un CSW qui n'arrive jamais parce que le point de
# terminaison n'a pas ete configure ne se voit qu'a l'execution.
#
# # Ce que ce scenario etablit
#
#   1. le controleur est trouve, demarre, le port remis a zero ;
#   2. le peripherique est adresse et enumere ;
#   3. l'arbitrage de l'enregistreur de vol s'execute : il LIT la table de
#      partitions par-dessus le transport Bulk, n'y trouve pas la sienne, et
#      CEDE le peripherique au stockage general ;
#   4. les deux points de terminaison Bulk sont trouves et configures, avec
#      leurs DCI ;
#   5. READ CAPACITY rend la geometrie REELLE du support -- comparee ici a la
#      taille du fichier, pas a une constante du noyau ;
#   6. le volume est publie sous la couche bloc.
#
# # Ce que ce scenario N'ETABLIT PAS
#
# Rien sur le materiel. QEMU emule un peripherique de stockage USB conforme ;
# une vraie cle repond plus lentement, cale, se deconnecte, renvoie des paquets
# courts et des STALL. La colonne TRIGKEY reste vide apres cette campagne.
set -uo pipefail
cd "$(dirname "$0")/../.."

BOOT=${1:?usage: run_usb_stockage.sh BOOTIMAGE [LOG]}
LOG=${2:-target/usb-stockage-ci/usb-stockage.log}
RACINE=$(dirname "$LOG")
CLE="$RACINE/cle-usb.img"
SCENARIO="$RACINE/scenario.img"

# La geometrie attendue est DEDUITE de l'image, pas ecrite en dur : une
# constante recopiee des deux cotes passerait meme si READ CAPACITY rendait
# n'importe quoi.
MIO=64
BLOCS=$(( MIO * 1024 * 1024 / 512 ))

mkdir -p "$RACINE"
rm -f "$LOG" "$CLE" "$SCENARIO"

python3 tools/ci/fabrique-disque-gpt.py "$CLE" --mio "$MIO" || exit 1

python3 - "$SCENARIO" <<'PY'
import sys, tarfile
from pathlib import Path
image = Path(sys.argv[1])
autorun = image.parent / "autorun"
autorun.write_text(
    "journal off\n"
    "echo USB_STOCKAGE_BEGIN\n"
    # `--attends` laisse le temps a l'enumeration ET a la sequence Bulk :
    # READ CAPACITY passe par une commande complete aller-retour.
    "lsusb --attends 8000\n"
    "df\n"
    "echo USB_STOCKAGE_FIN\n"
    "poweroff\n",
    encoding="ascii",
)
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

echo "=== QEMU xHCI + usb-storage ==="
timeout 240 qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$SCENARIO" \
  -m 2048 -smp 4 -display none -no-reboot \
  -device qemu-xhci,id=xhci \
  -drive if=none,id=cleusb,format=raw,file="$CLE" \
  -device usb-storage,bus=xhci.0,port=1,drive=cleusb \
  -serial file:"$LOG"

if [ ! -s "$LOG" ]; then
    echo "aucune sortie serie : le noyau n'a rien emis" >&2
    exit 1
fi
echo "journal serie : $(wc -c < "$LOG") octets"

if grep -aiEq '\*\*\* KERNEL PANIC \*\*\*|DOUBLE FAULT|panicked at' "$LOG"; then
    echo "panic/faute pendant le transport Bulk" >&2
    grep -aiE 'PANIC|DOUBLE FAULT|panicked at' "$LOG" | head -5 >&2
    exit 1
fi

echo "--- releve stockage USB ---"
sed -e 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -aE 'USB_STOCKAGE|BLACKBOX_USB|USB_BULK' || true
echo "--- fin du releve ---"

problemes=0
exige() {
    if grep -aEq "$1" "$LOG"; then
        printf 'ok      %s\n' "$2"
    else
        printf 'ABSENT  %s\n' "$2"
        problemes=$((problemes + 1))
    fi
}
interdit() {
    if grep -aEq "$1" "$LOG"; then
        printf 'PRESENT %s\n' "$2"
        problemes=$((problemes + 1))
    fi
}

exige 'BOUCHAUD_XHCI_CONTROLLER_ACTIVE_OK' 'controleur xHCI demarre'
exige 'BOUCHAUD_USB_ADDRESS_OK' 'peripherique adresse'
exige 'BOUCHAUD_USB_ENUM_OK' 'peripherique enumere'
# L'arbitrage de l'enregistreur de vol : il lit la table par-dessus le Bulk et
# cede quand la partition n'est pas la sienne. Sans cette ligne, on ne saurait
# pas si le stockage general a ete atteint par arbitrage ou par accident.
exige 'BOUCHAUD_BLACKBOX_USB_SKIP' 'enregistreur de vol : cede le support'
exige 'BOUCHAUD_USB_STOCKAGE_TROUVE .*in=0x[0-9a-f]+/dci[0-9]+ out=0x[0-9a-f]+/dci[0-9]+' \
      'points de terminaison Bulk IN et OUT trouves, avec leurs DCI'
# La GEOMETRIE, comparee a l'image reelle. C'est ce qui distingue un
# READ CAPACITY execute d'un READ CAPACITY suppose.
exige "BOUCHAUD_USB_STOCKAGE_PRET .*blocs=$BLOCS taille_bloc=512" \
      "READ CAPACITY rend la geometrie reelle ($BLOCS blocs de 512)"
exige "BOUCHAUD_USB_STOCKAGE_VOLUME .*blocs=$BLOCS taille_bloc=512 mio=$MIO" \
      'volume publie sous la couche bloc'

interdit 'BOUCHAUD_USB_STOCKAGE_ECHEC' 'echec de mise en service du stockage'
interdit 'BOUCHAUD_USB_BULK_ECHEC' 'echec de transfert Bulk'
interdit 'BOUCHAUD_USB_STOCKAGE_IO_ECHEC' 'echec d entree-sortie du stockage'
interdit 'BOUCHAUD_USB_STOCKAGE_REINIT_ECHEC' 'echec de reinitialisation Bulk-Only'

if [ "$problemes" -ne 0 ]; then
    echo "USB_STOCKAGE_ECHEC problemes=$problemes" >&2
    exit 1
fi
echo "USB_STOCKAGE_OK"
