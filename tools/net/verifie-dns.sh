#!/bin/bash
# Verifie la couche UDP du noyau avec une vraie resolution DNS, sous QEMU.
#
# ## Pourquoi ce test existe
#
# La navigation par nom se cassait dans une couche que rien n'eprouvait
# directement. Le defaut ne se voyait qu'au bout d'un build Ladybird complet —
# quatre-vingt-dix minutes — et sur un journal ou trois processus parlent en
# meme temps. Ce test le montre en une minute, sans Ladybird, sans la chaine
# croisee du userland : un ELF de treize kilooctets qui n'utilise que
# l'instruction `syscall`.
#
# ## Ce qu'il prouve
#
#   CAS1  un socket, une requete            -> une reponse
#   CAS2  deux sockets, deux requetes       -> deux reponses
#   CAS3  personne ne repond                -> l'attente est tenue, pas brulee
#
# CAS2 est celui qui comptait : un resolveur interroge A et AAAA en parallele,
# et la version precedente de `pump_udp` jetait le datagramme qui n'allait pas
# au socket qu'elle regardait. Le premier servi mangeait la reponse du second,
# les deux attendaient, et la page ne se chargeait jamais.
#
# ## Ce qu'il ne prouve pas
#
# Ni TLS, ni HTTP, ni Ladybird. Il s'arrete a la couche que le defaut touchait.
# Le trajet complet reste le travail des scenarios M9/M12.
#
#   tools/net/verifie-dns.sh
#
set -euo pipefail
cd "$(dirname "$0")/../.."
RACINE=$(pwd)
SORTIE=${SORTIE:-$RACINE/target/verifie-dns}

rouge() { printf '\033[31m%s\033[0m\n' "$*"; }
vert()  { printf '\033[32m%s\033[0m\n' "$*"; }
info()  { printf '\033[36m%s\033[0m\n' "$*"; }

for outil in clang qemu-system-x86_64 cargo; do
    command -v "$outil" >/dev/null 2>&1 || { rouge "$outil absent"; exit 1; }
done

mkdir -p "$SORTIE/scenario"

# Le noyau exige que le segment tombe dans le creneau utilisateur. `mcmodel=large`
# est indispensable : a cette adresse, les relocations 32 bits ne tiennent pas.
info "== sonde =="
clang -static -nostdlib -ffreestanding -fno-stack-protector -fno-pie \
      -mcmodel=large -O1 -Wl,-Ttext-segment=0x400000000000 \
      -o "$SORTIE/scenario/dns-probe" tools/userland/dns-probe.c
chmod 755 "$SORTIE/scenario/dns-probe"

printf 'uname\nifconfig\nexec /dns-probe\n' > "$SORTIE/scenario/autorun"

info "== disque =="
(cd tools/userland && IMAGE="$SORTIE/dns.img" ./mkdisk.sh "$SORTIE/scenario") >/dev/null

info "== noyau =="
command -v bootimage >/dev/null 2>&1 || cargo install bootimage --locked >/dev/null
cargo bootimage >/dev/null

info "== QEMU =="
JOURNAL="$SORTIE/serie.log"
# Le NAT de QEMU fournit un resolveur en 10.0.2.3 : c'est un vrai serveur DNS,
# interroge par un vrai datagramme. Rien n'est simule cote guest.
timeout 180 qemu-system-x86_64 \
    -drive format=raw,file=target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin \
    -drive format=raw,file="$SORTIE/dns.img" \
    -m 2048 -display none -no-reboot \
    -netdev user,id=net0 -device e1000,netdev=net0 \
    -serial stdio > "$JOURNAL" 2>&1 || true

sed -i 's/\x1b\[[0-9;]*m//g' "$JOURNAL"

echo
grep -a 'dns-probe' "$JOURNAL" | sed 's/^\[[^]]*\]\[[^]]*\] //' || true
echo

echecs=0
for marqueur in CAS1_OK CAS2_OK CAS3_ATTENTE_TENUE; do
    if grep -aq "$marqueur" "$JOURNAL"; then
        vert "  ok     $marqueur"
    else
        rouge "  ECHEC  $marqueur absent"
        echecs=$((echecs + 1))
    fi
done

# La preuve negative : le demultiplexage devait echouer avant, il ne doit plus.
if grep -aq 'CAS2_ECHEC' "$JOURNAL"; then
    rouge "  ECHEC  un datagramme est encore perdu entre deux sockets"
    echecs=$((echecs + 1))
fi

if grep -aqi 'panic' "$JOURNAL"; then
    rouge "  ECHEC  panique du noyau"
    echecs=$((echecs + 1))
fi

echo
if [ "$echecs" -ne 0 ]; then
    rouge "resolution DNS : $echecs verification(s) en echec"
    rouge "journal complet : $JOURNAL"
    exit 1
fi
vert "resolution DNS : trois cas au vert, datagrammes routes, attente non brulante"
