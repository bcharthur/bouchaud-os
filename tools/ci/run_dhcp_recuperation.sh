#!/usr/bin/env bash
#
# BOUCHAUD_C77_LA_MACHINE_SE_RATTRAPE_T_ELLE_TOUTE_SEULE
#
# # Ce que ce banc demande, et pourquoi
#
# La mesure B2 a etabli qu'au demarrage la reception ne peut pas fonctionner
# avant environ une seconde. Sous QEMU c'est le `flush_queue_timer` du modele
# e1000, arme par chaque ecriture de RCTL : il fait refuser toute trame tant
# qu'il court, sans rien laisser paraitre dans les registres. La preuve est
# une prediction verifiee -- reecrire RCTL repousse la fenetre d'autant.
#
# Le DHCP de demarrage part une milliseconde apres l'init du pilote et
# abandonne 700 ms plus tard : il est ENTIEREMENT dans cette fenetre.
#
# Echouer la n'est pas une faute ; un premier instant peut echouer. La faute
# serait d'en faire un verdict definitif. D'ou la question :
#
#     SANS AUCUNE COMMANDE MANUELLE, la machine finit-elle par obtenir un
#     vrai bail ?
#
# Le banc n'envoie rien, ne tape rien, n'aide en rien. Il laisse tourner et
# regarde le fil et le journal.
#
#     tools/ci/run_dhcp_recuperation.sh [image] [secondes]
#
# `BANC_NETDEV` permet de sortir du sous-reseau 10.0.2.x par defaut. C'est le
# seul moyen de distinguer « le repli compile est juste » de « le repli
# compile a devine juste parce que QEMU utilise ses valeurs d'usine ».
set -uo pipefail
cd "$(dirname "$0")/../.."

BOOT=${1:-target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin}
FENETRE=${2:-25}
TRAVAIL=${BANC_TRAVAIL:-$(mktemp -d)}
mkdir -p "$TRAVAIL"
if [ -z "${BANC_TRAVAIL:-}" ]; then trap 'rm -rf "$TRAVAIL"' EXIT; fi

# L'AUTORUN TIENT LA MACHINE EVEILLEE, ET C'EST LE POINT DELICAT DU BANC.
#
# Deux versions precedentes rendaient un releve net et faux.
#
# La premiere posait `netetat` seul : le lanceur d'autorun ETEINT la machine
# des que le script se termine (`BOUCHAUD_EXTINCTION_RAPPORT` au journal), si
# bien que la fenetre d'observation n'existait pas. On mesurait le demarrage
# en croyant mesurer vingt-cinq secondes de vie.
#
# La seconde retirait le second disque pour eviter cette extinction : le
# demarrage cale alors a `prechauffage` et n'atteint jamais le veilleur. Le
# silence se lisait a nouveau comme une absence d'activite.
#
# `desktop` ne rend jamais la main : la machine vit, le veilleur de lien
# tourne, et c'est QEMU qu'on arrete au bout du compte. Surtout pas `dhcp` --
# ce serait aider la machine, et la question porte precisement sur ce qu'elle
# fait quand personne ne l'aide.
printf 'netetat\ndesktop\n' > "$TRAVAIL/autorun"

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

# L'EMPREINTE DE L'IMAGE, INSCRITE DANS LE RELEVE.
#
# Une image oubliee d'une mesure precedente rend un resultat parfaitement
# coherent et parfaitement faux. `cargo build` ne regenere pas le bootimage :
# la seule protection est de dire QUELLE image a tourne.
echo "=== image mesuree ==="
sha256sum "$BOOT"

# TERM, PAS KILL : avec `--signal=KILL`, QEMU meurt sans vider ses tampons.
echo "=== QEMU, $FENETRE s d'observation, aucune intervention ==="
timeout -k 5 --signal=TERM "$FENETRE" qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$TRAVAIL/net.img" \
  -m 2048 -display none -no-reboot \
  -netdev "${BANC_NETDEV:-user,id=net0}" -device e1000,netdev=net0 \
  -object filter-dump,id=cap0,netdev=net0,file="$TRAVAIL/fil.pcap" \
  -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
  -serial file:"$TRAVAIL/serie.log" >/dev/null 2>&1

SERIE=$(sed 's/\x1b\[[0-9;]*m//g' "$TRAVAIL/serie.log" 2>/dev/null || true)

# LA DUREE REELLEMENT OBSERVEE, PAS CELLE QU'ON A DEMANDEE.
#
# Les deux bancs faux ci-dessus se seraient vus tout de suite ici : une
# machine eteinte ou calee n'ecrit plus, et l'ecart des horodatages le dit.
echo "=== fenetre reellement vecue ==="
printf '%s\n' "$SERIE" | grep -aoE "\[[0-9:]{8}\]" | sed -n '1p;$p' | tr '\n' ' '
echo

echo "=== verdicts poses ==="
printf '%s\n' "$SERIE" | grep -aoE "net: eth0 [^\"]{0,90}|BOUCHAUD_NET_RECONFIGURE verdict=[a-z-]+" || true

echo "=== le fil ==="
python3 tools/ci/lis_dhcp_pcap.py "$TRAVAIL/fil.pcap" 2>/dev/null | tail -3 || true

# Le critere n'est pas « un bail a ete vu quelque part » mais « le verdict
# durable de la machine est devenu `pret` sans qu'on lui demande rien ».
if printf '%s\n' "$SERIE" | grep -aq "BOUCHAUD_NET_RECONFIGURE verdict=pret"; then
  echo "DHCP_RECUPERATION verdict=RATTRAPE"
  exit 0
fi
FINAL=$(printf '%s\n' "$SERIE" | grep -aoE "BOUCHAUD_NET_RECONFIGURE verdict=[a-z-]+" | tail -1)
echo "DHCP_RECUPERATION verdict=FIGE dernier=${FINAL:-aucun-changement-depuis-le-demarrage}"
exit 1
