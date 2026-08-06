#!/bin/sh
# Fabrique l'image du disque userland de Bouchaud OS.
#
# Le noyau lit ce disque au demarrage et deplie son contenu dans le RAMFS. Cela
# remplace l'ancienne methode — inclure chaque binaire dans le noyau par
# `include_bytes!` puis tout recompiler — qui interdisait en pratique d'installer
# quoi que ce soit de volumineux : l'image de boot depasse deja 20 Mio.
#
#   ./mkdisk.sh              archive le contenu de out/
#   ./mkdisk.sh mon-dossier  archive un autre dossier
#
# Puis, au lancement de QEMU, attacher l'image comme SECOND disque :
#
#   qemu-system-x86_64 \
#       -drive format=raw,file=target/.../bootimage-bouchaud-os.bin \
#       -drive format=raw,file=tools/userland/userland.img \
#       -m 2048 -serial stdio
#
# L'ordre compte : le premier `-drive` devient hda (le disque de boot), le
# second hdb, la ou le noyau va chercher l'archive.

set -e
cd "$(dirname "$0")"

SOURCE=${1:-out}
IMAGE=${IMAGE:-userland.img}

if [ ! -d "$SOURCE" ]; then
    echo "mkdisk: '$SOURCE' n'existe pas — lancer d'abord ./build.sh" >&2
    exit 1
fi
if [ -z "$(ls -A "$SOURCE" 2>/dev/null)" ]; then
    echo "mkdisk: '$SOURCE' est vide" >&2
    exit 1
fi

# Format `ustar` explicite : c'est celui que sait lire `src/fs/tar.rs`. Le
# format GNU par defaut de certaines versions de tar ajoute des en-tetes
# etendues que l'analyseur ignorerait.
#
# Les chemins sont relatifs a $SOURCE, donc ce qui s'y trouve atterrit a la
# racine du RAMFS : out/hello devient /hello.
tar --format=ustar -C "$SOURCE" -cf "$IMAGE" .

# Le noyau lit par secteurs de 512 octets ; on complete pour que la derniere
# lecture ne deborde pas du fichier.
SIZE=$(wc -c < "$IMAGE")
PADDED=$(( (SIZE + 511) / 512 * 512 ))
if [ "$PADDED" -gt "$SIZE" ]; then
    dd if=/dev/zero bs=1 count=$((PADDED - SIZE)) >> "$IMAGE" 2>/dev/null
fi

# Zone persistante : 8 Mio a la FIN de l'image (voir `src/fs/persistance.rs`).
# L'archive se lit depuis le debut, la zone s'ecrit depuis la fin ; tant que
# l'image porte les deux, elles ne se rencontrent jamais. On complete donc
# l'image de la taille de la zone plus une marge, et le noyau y ecrit ce qui
# doit survivre a l'extinction.
#
# Les secteurs ajoutes sont nuls, donc sans magie : le premier demarrage trouve
# une zone vierge, ce qui est exactement ce qu'il faut.
ZONE_SECTEURS=16384
ZONE=$((ZONE_SECTEURS * 512))
dd if=/dev/zero bs=1M count=$((ZONE / 1024 / 1024)) >> "$IMAGE" 2>/dev/null
TOTAL=$(wc -c < "$IMAGE")

echo "image : $IMAGE ($((TOTAL / 1024)) Kio, $((TOTAL / 512)) secteurs)"
echo "  dont archive $((PADDED / 1024)) Kio et zone persistante $((ZONE / 1024)) Kio"
echo "contenu :"
tar -tf "$IMAGE" | sed 's|^\./|  /|' | grep -v '^  /$' | head -40
COUNT=$(tar -tf "$IMAGE" | wc -l)
[ "$COUNT" -gt 40 ] && echo "  ... et $((COUNT - 40)) autres entrees"
echo ""
echo "attacher comme SECOND disque au lancement de QEMU (voir l'entete de ce script)"
