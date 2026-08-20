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

# Plafond de lecture de l'archive, en miroir de `MAX_ARCHIVE_DISK_SIZE` dans
# `src/fs/tar.rs` : les deux valeurs doivent bouger ensemble. Le noyau n'indexe
# QUE les premiers octets du disque jusqu'a ce plafond ; une archive plus grande
# verrait ses dernieres entrees disparaitre sans que rien ne manque a l'appel
# cote outil. On refuse donc de fabriquer une telle image, avec les tailles
# exactes, plutot que de laisser la troncature se decouvrir dans QEMU vingt
# minutes plus tard.
ARCHIVE_MAX=$((2048 * 1024 * 1024))
if [ "$PADDED" -gt "$ARCHIVE_MAX" ]; then
    echo "mkdisk: archive de $PADDED octets ($((PADDED / 1024 / 1024)) Mio) au-dela du plafond noyau de $ARCHIVE_MAX octets ($((ARCHIVE_MAX / 1024 / 1024)) Mio)" >&2
    echo "mkdisk: src/fs/tar.rs MAX_ARCHIVE_DISK_SIZE borne la lecture de hdb ; reduire la charge utile ou relever les deux ensemble" >&2
    echo "mkdisk: entrees les plus lourdes de '$SOURCE' :" >&2
    find "$SOURCE" -type f -printf '%s\t%p\n' 2>/dev/null | sort -rn | head -10 >&2 || true
    rm -f "$IMAGE"
    exit 1
fi

# Zone persistante : 128 Mio a la FIN de l'image (voir `src/fs/persistance.rs`,
# `SECTEURS_ZONE`). L'archive se lit depuis le debut, la zone s'ecrit depuis la
# fin ; tant que l'image porte les deux, elles ne se rencontrent jamais.
#
# `persistance::debut()` exige en plus que le disque soit STRICTEMENT plus grand
# que la zone augmentee de son en-tete et de sa table (`SECTEUR_CONTENU`), sans
# quoi il declare « disque trop petit, zone absente ». Une archive minuscule --
# un scenario de test qui ne porte qu'un `autorun` -- produisait donc une image
# sans persistance du tout, et le test de survie au redemarrage n'avait alors
# aucune zone ou ecrire. On complete la region d'archive jusqu'a ce plancher
# avant d'ajouter la zone : les octets ajoutes sont nuls, donc invisibles pour
# l'analyseur TAR qui s'arrete a son premier bloc nul.
ZONE_SECTEURS=262144
ZONE=$((ZONE_SECTEURS * 512))
SECTEUR_CONTENU=1025
PLANCHER_ARCHIVE=$(((SECTEUR_CONTENU + 1) * 512))
if [ "$PADDED" -lt "$PLANCHER_ARCHIVE" ]; then
    dd if=/dev/zero bs=1 count=$((PLANCHER_ARCHIVE - PADDED)) >> "$IMAGE" 2>/dev/null
    PADDED=$PLANCHER_ARCHIVE
fi

dd if=/dev/zero bs=1M count=$((ZONE / 1024 / 1024)) >> "$IMAGE" 2>/dev/null
TOTAL=$(wc -c < "$IMAGE")

echo "image : $IMAGE ($((TOTAL / 1024)) Kio, $((TOTAL / 512)) secteurs)"
echo "  dont archive $((PADDED / 1024)) Kio et zone persistante $((ZONE / 1024)) Kio"
echo "  plafond archive noyau : $((ARCHIVE_MAX / 1024)) Kio (src/fs/tar.rs)"
echo "contenu :"
tar -tf "$IMAGE" | sed 's|^\./|  /|' | grep -v '^  /$' | head -40
COUNT=$(tar -tf "$IMAGE" | wc -l)
[ "$COUNT" -gt 40 ] && echo "  ... et $((COUNT - 40)) autres entrees"
echo ""
echo "attacher comme SECOND disque au lancement de QEMU (voir l'entete de ce script)"
