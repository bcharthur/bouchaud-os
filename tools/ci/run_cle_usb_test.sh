#!/usr/bin/env bash
#
# BOUCHAUD_CLE_USB_TEST_V1 -- une image d'essai sur cle USB n'ecrit JAMAIS sur
# les disques internes, prouve sous QEMU, DE L'EXTERIEUR de la machine.
#
#   run_cle_usb_test.sh BOOT_CLE BOOT_TEMOIN GUID [SORTIE]
#
# BOOT_CLE    : `cle-usb-test,banc-ecriture-interne`, construite avec
#               BOUCHAUD_MEDIA_AUTORISE=GUID ;
# BOOT_TEMOIN : `banc-ecriture-interne` SEULE -- la meme sonde, sans la regle.
# BO_QEMU_KVM=1 : sous KVM (sinon TCG). Le niveau de preuve est QEMU, JAMAIS
# PHYSICAL : aucune ligne de ce banc ne dit rien du Trigkey.
#
# # Les disques
#
#   IDE maitre  : l'image de demarrage ;
#   IDE esclave : le scenario (autorun) -- disque de donnees ATA, INTERNE ;
#   NVMe        : un disque GPT portant une partition Bouchaud, INTERNE ;
#   USB         : une cle GPT dont le GUID de disque est GUID (autorisee), ou
#                 un autre (etrangere), ou deux cles portant GUID (ambigu).
#
# # Ce que le scenario fait sur la machine
#
# Tout ce qui ecrit dans le systeme : `disktest --ecriture`, `ata-integrite`,
# `persist-test --pose`, `installer --go --ecrase`, `usbvolume --io`, puis la
# sonde, qui EMET de vraies commandes (ecriture, vidange, TRIM, effacement,
# Write Zeroes, Format NVM, cache d'ecriture, FORMAT UNIT, UNMAP, WRITE SAME,
# ATA PASS-THROUGH) vers chaque support.
#
# # Trois juges, dont deux hors de la machine
#
#   1. l'empreinte SHA-256 de chaque image disque, avant et apres ;
#   2. la TRACE de QEMU : les commandes que les peripheriques emules ont
#      REELLEMENT recues (`ide_bus_exec_cmd`, `pci_nvme_io_cmd`,
#      `pci_nvme_admin_cmd`, `scsi_req_parsed`) -- une commande refusee par la
#      regle n'y apparait pas ;
#   3. le journal du noyau (refus, identite du support, bilan).
# Le TEMOIN doit faire changer les empreintes et faire apparaitre ces
# commandes dans la trace : sans lui, un banc aveugle serait vert.
set -uo pipefail
cd "$(dirname "$0")/../.."

BOOT_CLE=${1:?usage: run_cle_usb_test.sh BOOT_CLE BOOT_TEMOIN GUID [SORTIE]}
BOOT_TEMOIN=${2:?BOOT_TEMOIN}
GUID=${3:?GUID}
SORTIE=${4:-target/cle-usb-test-ci}
ACCEL=tcg
[ "${BO_QEMU_KVM:-0}" = 1 ] && ACCEL=kvm
SORTIE="$SORTIE/$ACCEL"
rm -rf "$SORTIE"
mkdir -p "$SORTIE"

problemes=0
plainte() { printf 'ECHEC   %s\n' "$1"; problemes=$((problemes + 1)); }
ok() { printf 'ok      %s\n' "$1"; }
exige() { # FICHIER MOTIF LIBELLE
    if grep -aEq -- "$2" "$1"; then ok "$3"; else plainte "$3"; fi
}
refuse() { # FICHIER MOTIF LIBELLE : le motif ne doit PAS apparaitre
    local premier
    premier=$(awk -v m="$2" '$0 ~ m { print; exit }' "$1")
    if [ -n "$premier" ]; then plainte "$3 ($premier)"; else ok "$3"; fi
}
compte() { grep -aEc -- "$2" "$1" 2>/dev/null || true; }

empreinte() { sha256sum "$1" | cut -d' ' -f1; }

# Le scenario : une archive tar portant `autorun`, suivie de 4 Mio de zeros.
fabrique_scenario() {
    python3 - "$1" <<'PY'
import sys, tarfile
from pathlib import Path
image = Path(sys.argv[1])
autorun = image.parent / "autorun"
autorun.write_text("""journal off
echo CLE_USB_TEST_BEGIN
lsusb --attends 8000
disques-internes
nvmetest
disktest --ecriture
ata-integrite
persist-test --pose
installer --go --ecrase
usbvolume --io
sonde-ecriture-interne
disques-internes
echo CLE_USB_TEST_FIN
poweroff
""", encoding="ascii")
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
}

# Les evenements de trace QEMU : les commandes recues par les peripheriques.
EVENEMENTS="$SORTIE/evenements-trace"
printf '%s\n' ide_bus_exec_cmd pci_nvme_io_cmd pci_nvme_admin_cmd scsi_req_parsed > "$EVENEMENTS"

# lance NOM BOOT CLES... : un demarrage complet, disques neufs.
lance() {
    local nom=$1 boot=$2; shift 2
    local d="$SORTIE/$nom"
    mkdir -p "$d"
    cp "$boot" "$d/boot.img"
    fabrique_scenario "$d/scenario.img"
    python3 tools/ci/fabrique-disque-gpt.py "$d/nvme.img" --mio 64 >/dev/null || return 1
    local usb=() i=0
    for guid in "$@"; do
        i=$((i + 1))
        python3 tools/ci/fabrique-disque-gpt.py "$d/cle$i.img" --mio 32 --nom CLE >/dev/null || return 1
        python3 tools/release/pose_guid_disque_gpt.py "$d/cle$i.img" "$guid" >/dev/null || return 1
        # La cle 1 sur le premier controleur xHCI, les suivantes sur un SECOND
        # (revue #299 : un slot_id est local a son controleur ; deux cles sur
        # deux controleurs recoivent le meme emplacement).
        local bus=xhci.0 port=1
        [ "$i" -ge 2 ] && { bus=xhci2.0; port=$((i - 1)); }
        usb+=(-drive "if=none,id=cle$i,format=raw,file=$d/cle$i.img"
              -device "usb-storage,bus=$bus,port=$port,drive=cle$i,removable=on")
    done
    for f in boot scenario nvme $(seq -f 'cle%g' 1 $i); do
        empreinte "$d/$f.img" > "$d/$f.avant"
    done
    local accel=(-accel tcg)
    [ "$ACCEL" = kvm ] && accel=(-accel kvm -cpu host)
    timeout "${BO_CLE_TIMEOUT_S:-420}" qemu-system-x86_64 "${accel[@]}" \
        -drive format=raw,file="$d/boot.img" \
        -drive format=raw,file="$d/scenario.img" \
        -drive if=none,id=nvm,format=raw,file="$d/nvme.img" -device nvme,serial=cle-usb-test,drive=nvm \
        -device qemu-xhci,id=xhci -device qemu-xhci,id=xhci2 "${usb[@]}" \
        -m 2048 -smp 2 -display none -no-reboot \
        -trace events="$EVENEMENTS" -D "$d/qemu-trace.log" \
        -serial file:"$d/serie.log"
    echo "$?" > "$d/qemu.rc"
    sed -e 's/\x1b\[[0-9;]*m//g' "$d/serie.log" | tr -d '\r' > "$d/propre.log"
    for f in boot scenario nvme $(seq -f 'cle%g' 1 $i); do
        empreinte "$d/$f.img" > "$d/$f.apres"
    done
}

change() { ! cmp -s "$1.avant" "$1.apres"; }

# Les commandes recues, en clair : "ide 0x30", "nvme-io 0x1", "nvme-admin 0x80",
# "scsi 0x2a". Les formats de trace de QEMU 8.x :
#   ide_bus_exec_cmd bus 0x... state 0x... cmd 0x30
#   pci_nvme_io_cmd cid 1 nsid 0x1 sqid 1 opc 0x1 opname 'NVME_NVM_CMD_WRITE'
#   pci_nvme_admin_cmd cid 1 sqid 0 opc 0x6 opname '...'
#   scsi_req_parsed target 0 lun 0 tag 1 command 42 dir 2 length 512
recues() {
    python3 - "$1" <<'PY'
import re, sys
for ligne in open(sys.argv[1], errors="replace"):
    m = re.search(r"ide_bus_exec_cmd .*cmd 0x([0-9a-f]+)", ligne)
    if m: print(f"ide 0x{int(m.group(1),16):02x}"); continue
    m = re.search(r"pci_nvme_io_cmd .*opc 0x([0-9a-f]+)", ligne)
    if m: print(f"nvme-io 0x{int(m.group(1),16):02x}"); continue
    m = re.search(r"pci_nvme_admin_cmd .*opc 0x([0-9a-f]+)", ligne)
    if m: print(f"nvme-admin 0x{int(m.group(1),16):02x}"); continue
    m = re.search(r"scsi_req_parsed .*command (\d+)", ligne)
    if m: print(f"scsi 0x{int(m.group(1)):02x}")
PY
}

# Ce qui ne doit JAMAIS atteindre un disque interne dans l'image cle : toute
# ecriture/vidange/effacement ATA, toute ecriture/vidange/Write Zeroes/DSM
# NVMe, Format NVM, Sanitize, Set Features autre que le nombre de files (la
# trace ne donne pas l'attribut : on compte les Set Features, voir plus bas).
INTERDIT_INTERNE='^(ide 0x(30|31|34|ca|cb|35|c5|39|3d|e7|ea|06|f4|f3)|nvme-io 0x(00|01|04|08|09)|nvme-admin 0x(80|84|10|11|0d|15))$'
# Interdit vers TOUT stockage USB : FORMAT UNIT, UNMAP, WRITE SAME, ATA
# PASS-THROUGH.
INTERDIT_USB='^scsi 0x(04|42|41|93|85|a1)$'
# Ecritures USB : permises vers la seule cle autorisee.
ECRITURE_USB='^scsi 0x(2a|aa|8a|35|91)$'

commun() { # REPERTOIRE : ce que tout demarrage doit montrer
    local d=$1
    test -s "$d/propre.log" || { plainte "$d : aucune sortie serie"; return; }
    refuse "$d/propre.log" 'KERNEL PANIC|DOUBLE FAULT|panicked at' "$d : ni panique ni double faute"
    exige "$d/propre.log" 'CLE_USB_TEST_FIN' "$d : le scenario va jusqu'au bout"
    exige "$d/propre.log" 'BOUCHAUD_SONDE_ECRITURE_FIN' "$d : la sonde a tourne"
    test -s "$d/qemu-trace.log" || plainte "$d : trace QEMU vide (le juge exterieur n'a rien vu)"
    recues "$d/qemu-trace.log" > "$d/commandes-recues.txt"
    printf '        commandes recues par les peripheriques : %s\n' \
        "$(sort "$d/commandes-recues.txt" | uniq -c | awk '{printf "%s:%s ", $2" "$3, $1}')"
}

# --- 1. Image cle, cle AUTORISEE --------------------------------------------
echo "=== [$ACCEL] 1. image cle-usb-test, cle autorisee ==="
lance autorisee "$BOOT_CLE" "$GUID"
d="$SORTIE/autorisee"
commun "$d"
exige "$d/propre.log" 'BOUCHAUD_DISQUES_INTERNES lecture_seule=1 image=cle-usb-test media_autorise=guid-gpt' 'l image s annonce en lecture seule, GUID grave'
exige "$d/propre.log" 'BOUCHAUD_MEDIA_IDENTITE support=usb controleur=0x[0-9a-f]+ slot=[0-9]+ identite=autorisee ambigu=0' 'la cle est reconnue par son GUID GPT'
for f in boot scenario nvme; do
    if change "$d/$f"; then plainte "image $f MODIFIEE ($(cat "$d/$f.avant") -> $(cat "$d/$f.apres"))"; else ok "image $f intacte (sha256 $(cut -c1-16 "$d/$f.apres"))"; fi
done
if change "$d/cle1"; then ok 'la cle AUTORISEE a ete ecrite (le support de demarrage reste utilisable)'; else plainte 'la cle autorisee n a pas ete ecrite'; fi
if python3 -c "import sys; d=open(sys.argv[1],'rb').read(); sys.exit(0 if b'BOUCHAUD-SONDE-ECRITURE-INTERNE!' in d[-1024:] else 1)" "$d/cle1.img"; then
    ok 'la marque de la sonde est relue a la fin de la cle autorisee'
else
    plainte 'marque de la sonde absente de la cle autorisee'
fi
refuse "$d/commandes-recues.txt" "$INTERDIT_INTERNE" 'AUCUNE commande d ecriture/effacement n a atteint un disque interne (trace QEMU)'
refuse "$d/commandes-recues.txt" "$INTERDIT_USB" 'aucune commande interdite n a atteint la cle (FORMAT/UNMAP/WRITE SAME/PASS-THROUGH)'
exige "$d/commandes-recues.txt" "$ECRITURE_USB" 'les ecritures vers la cle autorisee sont arrivees (trace QEMU)'
setf=$(compte "$d/commandes-recues.txt" '^nvme-admin 0x09$')
[ "$setf" -le 1 ] && ok "Set Features NVMe recus : $setf (le seul permis : nombre de files)" || plainte "Set Features NVMe recus : $setf"
for c in write-sectors flush-cache data-set-management security-erase-unit; do
    exige "$d/propre.log" "BOUCHAUD_SONDE_ECRITURE support=ata commande=$c resultat=refusee" "ata $c refusee"
done
for c in write flush write-zeroes dataset-management set-features-vwc format-nvm; do
    exige "$d/propre.log" "BOUCHAUD_SONDE_ECRITURE support=nvme commande=$c resultat=refusee" "nvme $c refusee"
done
exige "$d/propre.log" 'BOUCHAUD_SONDE_ECRITURE_POT support=nvme ecritures=300 refusees=300 lecture_apres=fait' \
    'NVMe : 300 ecritures refusees n epuisent pas les identifiants, une lecture part encore'
for c in write-10-volume write-10 synchronize-cache; do
    exige "$d/propre.log" "BOUCHAUD_SONDE_ECRITURE support=usb commande=$c resultat=partie" "usb $c PERMISE sur la cle autorisee"
done
for c in format-unit unmap ata-pass-through-16 write-same-16; do
    exige "$d/propre.log" "BOUCHAUD_SONDE_ECRITURE support=usb commande=$c resultat=refusee" "usb $c refusee meme sur la cle autorisee"
done
exige "$d/propre.log" 'BOUCHAUD_ECRITURE_INTERNE_REFUSEE support=nvme .*raison=disque-interne' 'refus NVMe journalise'
exige "$d/propre.log" 'BOUCHAUD_ECRITURE_INTERNE_REFUSEE support=ata .*raison=disque-interne' 'refus ATA journalise'
refuse "$d/propre.log" 'H10_DISKTEST verdict=ok phase=ecriture' 'disktest --ecriture n a rien ecrit'
refuse "$d/propre.log" 'BOUCHAUD_INSTALL.*(resultat|verdict)=(ok|reussi)' 'l installateur n a rien pose'

# --- 2. Image cle, cle ETRANGERE --------------------------------------------
echo
echo "=== [$ACCEL] 2. image cle-usb-test, cle etrangere ==="
AUTRE=$(python3 tools/release/pose_guid_disque_gpt.py --tire)
lance etrangere "$BOOT_CLE" "$AUTRE"
d="$SORTIE/etrangere"
commun "$d"
exige "$d/propre.log" 'BOUCHAUD_MEDIA_IDENTITE support=usb controleur=0x[0-9a-f]+ slot=[0-9]+ identite=etrangere' 'la cle etrangere est reconnue comme telle'
for f in boot scenario nvme cle1; do
    if change "$d/$f"; then plainte "image $f MODIFIEE"; else ok "image $f intacte"; fi
done
refuse "$d/commandes-recues.txt" "$INTERDIT_INTERNE" 'aucune ecriture vers un disque interne (trace QEMU)'
refuse "$d/commandes-recues.txt" "$ECRITURE_USB" 'aucune ecriture ni vidange vers la cle etrangere (trace QEMU)'
refuse "$d/commandes-recues.txt" "$INTERDIT_USB" 'aucune commande interdite vers la cle etrangere'
for c in write-10-volume write-10 synchronize-cache format-unit unmap ata-pass-through-16 write-same-16; do
    exige "$d/propre.log" "BOUCHAUD_SONDE_ECRITURE support=usb commande=$c resultat=refusee" "usb $c refusee"
done
exige "$d/propre.log" 'BOUCHAUD_ECRITURE_INTERNE_REFUSEE support=usb .*identite=etrangere raison=support-non-autorise' 'refus USB journalise'

# --- 3. Deux cles portant le GUID : AMBIGU ----------------------------------
echo
echo "=== [$ACCEL] 3. image cle-usb-test, deux cles portant le meme GUID, sur DEUX controleurs xHCI ==="
lance ambigu "$BOOT_CLE" "$GUID" "$GUID"
d="$SORTIE/ambigu"
commun "$d"
exige "$d/propre.log" 'BOUCHAUD_MEDIA_IDENTITE support=usb .*ambigu=1' 'l ambiguite est detectee'
# Les deux cles ont-elles vraiment ete vues sur deux controleurs ? Sinon le
# scenario ne prouve pas ce qu'il dit.
nctrl=$(grep -aoE 'BOUCHAUD_MEDIA_IDENTITE support=usb controleur=0x[0-9a-f]+' "$d/propre.log" | sort -u | wc -l)
[ "$nctrl" -ge 2 ] && ok "les cles autorisees sont vues sur $nctrl controleurs distincts" || plainte "une seule identite de controleur vue ($nctrl)"
for f in boot scenario nvme cle1 cle2; do
    if change "$d/$f"; then plainte "image $f MODIFIEE"; else ok "image $f intacte"; fi
done
# LIMITE CONNUE, mesuree ici : l'enumeration est sequentielle. La PREMIERE
# cle, seule autorisee a cet instant, recoit la vidange de la sonde de montage
# (SYNCHRONIZE CACHE, sans donnees : aucune ecriture n'a precede, le support
# n'est pas modifie). L'ambiguite se verrouille a l'arrivee de la seconde ;
# des lors, plus rien. Aucune ECRITURE de donnees, sur aucune des deux.
refuse "$d/commandes-recues.txt" '^scsi 0x(2a|aa|8a)$' 'aucune ecriture de donnees USB quand deux cles se disent autorisees (trace QEMU)'
sync=$(compte "$d/commandes-recues.txt" '^scsi 0x(35|91)$')
[ "$sync" -le 1 ] && ok "vidanges USB recues : $sync (au plus celle du montage de la premiere cle, avant la seconde)" || plainte "vidanges USB recues : $sync"
for c in write-10-volume write-10 synchronize-cache; do
    exige "$d/propre.log" "BOUCHAUD_SONDE_ECRITURE support=usb commande=$c resultat=refusee" "usb $c refusee (identite ambigue)"
done
exige "$d/propre.log" 'BOUCHAUD_ECRITURE_INTERNE_REFUSEE support=usb .*raison=identite-ambigue' 'refus pour ambiguite journalise'
refuse "$d/commandes-recues.txt" "$INTERDIT_INTERNE" 'aucune ecriture vers un disque interne (trace QEMU)'

# --- 4. TEMOIN : la meme sonde, SANS la regle -------------------------------
echo
echo "=== [$ACCEL] 4. TEMOIN banc-ecriture-interne sans cle-usb-test ==="
lance temoin "$BOOT_TEMOIN" "$GUID"
d="$SORTIE/temoin"
commun "$d"
exige "$d/propre.log" 'BOUCHAUD_DISQUES_INTERNES lecture_seule=0 image=standard' 'le temoin s annonce standard'
refuse "$d/propre.log" 'BOUCHAUD_ECRITURE_INTERNE_REFUSEE' 'le temoin ne refuse rien'
for f in scenario nvme cle1; do
    if change "$d/$f"; then ok "temoin : image $f MODIFIEE (le juge des empreintes voit une ecriture)"; else plainte "temoin : image $f intacte -- le banc est aveugle"; fi
done
for motif in '^ide 0x(30|ca)$' '^ide 0xe7$' '^nvme-io 0x01$' '^nvme-io 0x00$' '^nvme-io 0x08$' '^nvme-io 0x09$' '^nvme-admin 0x80$' '^scsi 0x2a$' '^scsi 0x04$'; do
    exige "$d/commandes-recues.txt" "$motif" "temoin : la trace QEMU voit ${motif//[\^\$()]/}"
done
exige "$d/propre.log" 'BOUCHAUD_SONDE_ECRITURE_FIN lecture_seule=0 parties=[0-9]+ refusees=0' 'temoin : rien de refuse'

echo
if [ "$problemes" -ne 0 ]; then
    echo "CLE_USB_TEST_ECHEC accel=$ACCEL problemes=$problemes niveau=QEMU-$ACCEL"
    exit 1
fi
echo "CLE_USB_TEST_OK accel=$ACCEL niveau=QEMU-$ACCEL physique=non-qualifie"
