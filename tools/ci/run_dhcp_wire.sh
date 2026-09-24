#!/usr/bin/env bash
#
# BOUCHAUD_C76_L_OFFRE_EXISTE_T_ELLE_SUR_LE_FIL
#
# # La question, et pourquoi elle precede tout le reste
#
# La session physique Trigkey (d131f2a) montre neuf DISCOVER pour une seule
# OFFRE. Le banc QEMU montre un DISCOVER et AUCUNE offre, puis un repli sur
# la configuration SLIRP statique.
#
# Deux familles de defauts completement differentes peuvent produire ce meme
# silence, et rien dans les compteurs internes ne les distingue :
#
#   CAS A   le serveur repond, et notre pile perd l'OFFRE quelque part
#           entre le fil et le client DHCP
#
#   CAS B   le serveur ne repond pas -- parce que notre DISCOVER est
#           malforme, mal adresse, ou mal sommé
#
# `offer_seen=0` ne tranche pas : il dit seulement que le client n'a rien vu.
# La preuve est le paquet SUR LE FIL, et QEMU sait le capturer.
#
#     tools/ci/run_dhcp_wire.sh [image-d-amorcage]
set -uo pipefail
cd "$(dirname "$0")/../.."

BOOT=${1:-target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin}
TRAVAIL=${BANC_TRAVAIL:-$(mktemp -d)}
mkdir -p "$TRAVAIL"
if [ -z "${BANC_TRAVAIL:-}" ]; then trap 'rm -rf "$TRAVAIL"' EXIT; fi

# Un autorun minimal : on laisse le reseau demarrer, on releve son etat, et on
# s'arrete. Rien d'autre ne doit bouger pendant la capture.
# DEUX negociations, et c'est la comparaison qui porte l'information.
#
# La premiere a lieu au demarrage, dans `demarre_interne`. La seconde est
# lancee a la main, plus tard, par la commande `dhcp`. Si la premiere echoue
# et la seconde reussit, quelque chose change entre les deux -- et c'est CE
# quelque chose qu'il faut nommer.
cat > "$TRAVAIL/autorun" <<'AUTORUN'
netetat
dhcp
netetat
ifconfig
poweroff
AUTORUN

python3 - "$TRAVAIL" <<'PY'
import sys, tarfile, pathlib
travail = pathlib.Path(sys.argv[1])
image = travail / "net.img"
with tarfile.open(image, "w", format=tarfile.USTAR_FORMAT) as tar:
    info = tar.gettarinfo(str(travail / "autorun"), arcname="./autorun")
    info.uid = info.gid = 0
    info.uname = info.gname = "root"
    info.mode = 0o644
    with (travail / "autorun").open("rb") as src:
        tar.addfile(info, src)
with image.open("ab") as out:
    out.write(b"\0" * (4 * 1024 * 1024))
PY

echo "=== QEMU avec capture du netdev ==="
set +e
timeout 120 qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$TRAVAIL/net.img" \
  -m 2048 -display none -no-reboot \
  -netdev user,id=net0 -device e1000,netdev=net0 \
  -object filter-dump,id=cap0,netdev=net0,file="$TRAVAIL/fil.pcap" \
  -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
  -serial file:"$TRAVAIL/serie.log"
code=$?
set -e
printf 'qemu exit=%s\n' "$code"

echo "=== ce que le noyau a vu ==="
sed 's/\x1b\[[0-9;]*m//g' "$TRAVAIL/serie.log" \
  | grep -aoE "NET_BOOT [^\"]{0,130}|DHCP_RX_TRACE[^\"]{0,320}|DHCP_RX_ANNEAU[^\"]{0,160}|DHCP: [^\"]{0,70}" || true

echo "=== ce qui est passe SUR LE FIL ==="
python3 tools/ci/lis_dhcp_pcap.py "$TRAVAIL/fil.pcap"
