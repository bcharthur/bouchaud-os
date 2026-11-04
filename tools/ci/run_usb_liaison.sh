#!/usr/bin/env bash
#
# Le volume USB et LE support qui le porte, sous QEMU (BOUCHAUD_USB_LIAISON_SUPPORT_V1,
# BOUCHAUD_USB_REFUS_SCSI_V1).
#
#   tools/ci/run_usb_liaison.sh BOOTIMAGE [SORTIE] [SCENARIO...]
#
# Un demarrage par scenario, avec l'image PAR DEFAUT (aucune feature de banc) :
#
#   accepte            1 cle qui vide                           (test 1)
#   refuse-sonde       1 cle dont la sonde de montage echoue    (tests 2, 5)
#   deux-cles          cle 1 vide, la sonde de la cle 2 echoue  (test 3)
#   deux-cles-inverse  la sonde de la cle 1 echoue, cle 2 vide  (tests 3, 9)
#   refus-en-usage     la vidange passe au montage ; le support
#                      la REFUSE apres la premiere ecriture     (tests 4, 5)
#   rebranchement      la cle liee est retiree puis rebranchee  (tests 6, 7)
#
# Le diagnostic `usbvolume io` lit le dernier bloc du volume, le reecrit A
# L'IDENTIQUE, le relit, puis vide. Ses lignes `BOUCHAUD_USB_VOLUME_IO` et
# `BOUCHAUD_USB_VOLUME_ETAT` sont ce que les invariants lisent.
#
# # Deux sortes de refus, et pourquoi
#
# * REFUS SCSI REEL (refus-en-usage) : QEMU fait echouer la vidange du support
#   (`blkdebug`, `errno 5` sur `flush_to_disk`, `iotype = flush` : lectures et
#   ecritures ne sont pas touchees). Le peripherique rend CHECK CONDITION et
#   garde un sens en attente : c'est le chemin `REQUEST SENSE` sous le verrou.
#   QEMU ne transmet une vidange au support que si quelque chose a ete ecrit
#   depuis la precedente : une sonde de montage (aucune ecriture avant) ne peut
#   donc pas etre refusee ainsi.
# * SONDE REFUSEE (refuse-sonde, deux-cles*) : `usbfault vidange` arme un refus
#   ponctuel, consomme par la PROCHAINE vidange -- la sonde de la cle branchee
#   a chaud juste apres. Aucune commande ne part ; c'est l'attribution de la
#   capacite au bon support qui est mise a l'epreuve.
#
# Ce que ce banc N'ETABLIT PAS : rien sur une vraie cle. QEMU vide quand on le
# lui demande ; une cle reelle peut acquitter sans vider. PHYSICAL reste a faire.
set -uo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_usb_liaison.sh BOOTIMAGE [SORTIE] [SCENARIO...]}
SORTIE=${2:-target/usb-liaison}
if [ $# -ge 2 ]; then shift 2; else shift $#; fi
SCENARIOS=("$@")
[ ${#SCENARIOS[@]} -eq 0 ] && SCENARIOS=(accepte refuse-sonde deux-cles deux-cles-inverse refus-en-usage rebranchement)
mkdir -p "$SORTIE"; SORTIE=$(cd "$SORTIE" && pwd)

# Deux tailles differentes : la geometrie publiee dit QUELLE cle porte le volume.
MIO1=64; MIO2=32
BLOCS1=$(( MIO1 * 2048 ))

# Etat 1 au depart ; la premiere ecriture fait passer a l'etat 2, ou toute
# vidange echoue -- et elle seule.
printf '%s\n' '[set-state]' 'state = "1"' 'event = "write_aio"' 'new_state = "2"' '' \
  '[inject-error]' 'state = "2"' 'event = "flush_to_disk"' 'iotype = "flush"' 'errno = "5"' \
  > "$SORTIE/refuse-apres-ecriture.conf"

fabrique_autorun() { # $1 = dossier, puis lignes
  local dossier=$1; shift
  mkdir -p "$dossier"
  printf '%s\n' "journal off" "$@" "poweroff" > "$dossier/autorun"
  python3 - "$dossier/scenario.img" "$dossier/autorun" <<'PY'
import sys, tarfile
from pathlib import Path
image, autorun = Path(sys.argv[1]), Path(sys.argv[2])
with tarfile.open(image, "w", format=tarfile.USTAR_FORMAT) as tar:
    info = tar.gettarinfo(str(autorun), arcname="./autorun")
    info.uid = info.gid = 0; info.uname = info.gname = "root"; info.mode = 0o644
    with autorun.open("rb") as src:
        tar.addfile(info, src)
with image.open("ab") as out:
    out.write(b"\0" * (4 * 1024 * 1024))
PY
}

cle() { python3 tools/ci/fabrique-disque-gpt.py "$1" --mio "$2" >/dev/null || exit 1; }

# drive : $1 id, $2 image, $3 config blkdebug ou vide
drive() {
  if [ -n "${3:-}" ]; then echo "-drive if=none,id=$1,format=raw,file=blkdebug:$3:$2"
  else echo "-drive if=none,id=$1,format=raw,file=$2"; fi
}

propre() { sed -E 's/\x1b\[[0-9;]*m//g' "$1" | tr -d '\r'; }

lance() { # $1 = dossier, $2.. = options QEMU ; rend le pid
  local dossier=$1; shift
  : > "$dossier/serie.log"
  # shellcheck disable=SC2068
  qemu-system-x86_64 -drive format=raw,file="$BOOT" -drive format=raw,file="$dossier/scenario.img" \
    -m 2048 -smp 2 -display none -no-reboot -device qemu-xhci,id=xhci \
    -monitor "unix:$dossier/moniteur,server,nowait" $@ -serial file:"$dossier/serie.log" >/dev/null 2>&1 &
  echo $!
}

attend_fin() { # $1 = pid, $2 = budget s
  local fin=$((SECONDS + $2))
  while kill -0 "$1" 2>/dev/null && (( SECONDS < fin )); do sleep 1; done
  kill -9 "$1" 2>/dev/null; wait "$1" 2>/dev/null
}

attend_marque() { # $1 = log, $2 = motif, $3 = budget s
  local fin=$((SECONDS + $3))
  until [ "$(propre "$1" | grep -ac "$2")" -gt 0 ]; do
    (( SECONDS < fin )) || return 1
    sleep 0.5
  done
}

# `LIAISON_ETAPE_<k>` dans le journal declenche la k-ieme commande du moniteur.
etapes() { # $1 = dossier, $2.. = commandes moniteur
  local d=$1; shift
  local k=1
  for cmd in "$@"; do
    attend_marque "$d/serie.log" "LIAISON_ETAPE_$k" 200 || { echo "  etape $k jamais atteinte"; return 1; }
    python3 tools/ci/qemu_moniteur.py "$d/moniteur" "$cmd" >/dev/null
    k=$((k + 1))
  done
}

ECHECS=0
exige() { # $1 = log, $2 = motif ERE, $3 = libelle
  if [ "$(propre "$1" | grep -acE "$2")" -gt 0 ]; then printf '  ok      %s\n' "$3"
  else printf '  ECHEC   %s\n' "$3"; ECHECS=$((ECHECS + 1)); fi
}
interdit() {
  if [ "$(propre "$1" | grep -acE "$2")" -gt 0 ]; then printf '  ECHEC   %s\n' "$3"; ECHECS=$((ECHECS + 1))
  else printf '  ok      %s\n' "$3"; fi
}
# La n-ieme ligne d'un motif doit en contenir un second.
exige_n() { # $1 = log, $2 = motif de ligne, $3 = rang, $4 = motif ERE, $5 = libelle
  local ligne
  ligne=$(propre "$1" | grep -a "$2" | sed -n "${3}p" | sed 's/^.*\] //')  # rang : 1, 2... ou $ (la derniere)
  if grep -Eq "$4" <<<"$ligne"; then printf '  ok      %s\n' "$5"
  else printf '  ECHEC   %s  [%s]\n' "$5" "${ligne:-absent}"; ECHECS=$((ECHECS + 1)); fi
}
commun() {
  interdit "$1" 'KERNEL PANIC|panicked at|DOUBLE FAULT' 'aucune panique'
  interdit "$1" 'BOUCHAUD_USB_STOCKAGE_ECHEC' 'aucun echec de mise en service'
  exige "$1" 'LIAISON_FIN' 'scenario mene a son terme'
}

ENUM="lsusb --attends 6000"
COURT="lsusb --attends 3000"
ATTENTE="lsusb --attends 12000"

for sc in "${SCENARIOS[@]}"; do
  d="$SORTIE/$sc"; rm -rf "$d"; mkdir -p "$d"; L="$d/serie.log"
  echo "=== $sc ==="
  case "$sc" in
    accepte)
      cle "$d/cle1.img" $MIO1
      fabrique_autorun "$d" "$ENUM" "usbvolume io" "usbvolume" "echo LIAISON_FIN"
      pid=$(lance "$d" $(drive c1 "$d/cle1.img") -device usb-storage,bus=xhci.0,port=1,drive=c1)
      attend_fin "$pid" 240; commun "$L"
      exige "$L" 'BOUCHAUD_USB_STOCKAGE_VIDANGE numero=1 .*sonde=ok vidange_reelle=1' 'la sonde de montage passe'
      exige "$L" "BOUCHAUD_USB_STOCKAGE_VOLUME .*numero=1 vidange_reelle=1 blocs=$BLOCS1 " 'volume lie a la cle 1, vidange promise'
      exige "$L" 'BOUCHAUD_USB_VOLUME_IO .*lecture=ok ecriture=ok relecture=ok vidange=1' 'E/S et vidange'
      exige "$L" 'BOUCHAUD_USB_VOLUME_ETAT present=1 lie=1 vidange_reelle=1 orphelin=0 .*descripteur_vidange=1' 'descripteur : vidange reelle'
      interdit "$L" 'VIDANGE_REFUSEE' 'aucun refus'
      ;;
    refuse-sonde)
      cle "$d/cle1.img" $MIO1
      fabrique_autorun "$d" "$COURT" "usbfault vidange" "echo LIAISON_ETAPE_1" "$ATTENTE" \
        "usbvolume io" "usbvolume io" "usbvolume" "echo LIAISON_FIN"
      pid=$(lance "$d" $(drive c1 "$d/cle1.img"))
      etapes "$d" "device_add usb-storage,bus=xhci.0,port=1,drive=c1,id=u1"
      attend_fin "$pid" 120; commun "$L"
      exige "$L" 'BOUCHAUD_USB_STOCKAGE_VIDANGE numero=1 .*sonde=vidange-refus-injecte vidange_reelle=0' 'sonde de montage refusee'
      exige "$L" "BOUCHAUD_USB_STOCKAGE_VOLUME .*numero=1 vidange_reelle=0 blocs=$BLOCS1 " 'volume publie SANS promesse'
      exige_n "$L" 'BOUCHAUD_USB_VOLUME_IO' 1 'lecture=ok ecriture=ok relecture=ok vidange=0' 'E/S ordinaires, vidange refusee'
      exige_n "$L" 'BOUCHAUD_USB_VOLUME_IO' 2 'lecture=ok ecriture=ok relecture=ok vidange=0' 'toujours au second passage'
      exige "$L" 'BOUCHAUD_USB_VOLUME_ETAT present=1 lie=1 vidange_reelle=0 orphelin=0 .*descripteur_vidange=0' 'descripteur : aucune barriere'
      interdit "$L" 'VIDANGE_REFUSEE' 'aucune vidange emise sur un volume qui ne promet rien'
      interdit "$L" 'BOUCHAUD_USB_STOCKAGE_IO_ECHEC|BOT_REPRISE_ECHEC' 'aucun echec d E/S ni de reprise'
      ;;
    deux-cles)
      cle "$d/cle1.img" $MIO1; cle "$d/cle2.img" $MIO2
      fabrique_autorun "$d" "$ENUM" "usbfault vidange" "echo LIAISON_ETAPE_1" "$ATTENTE" \
        "usbvolume io" "usbvolume" "lsusb" "echo LIAISON_FIN"
      pid=$(lance "$d" $(drive c1 "$d/cle1.img") -device usb-storage,bus=xhci.0,port=1,drive=c1 $(drive c2 "$d/cle2.img"))
      etapes "$d" "device_add usb-storage,bus=xhci.0,port=2,drive=c2,id=u2"
      attend_fin "$pid" 120; commun "$L"
      exige "$L" 'BOUCHAUD_USB_STOCKAGE_VIDANGE numero=1 .*sonde=ok vidange_reelle=1' 'cle 1 : vidange prouvee'
      exige "$L" 'BOUCHAUD_USB_STOCKAGE_VIDANGE numero=2 .*sonde=vidange-refus-injecte vidange_reelle=0' 'cle 2 : sonde refusee'
      exige "$L" 'BOUCHAUD_USB_STOCKAGE_NON_PUBLIE numero=2 raison=volume-deja-lie lie=1' 'la cle 2 est ecartee, et le dit'
      interdit "$L" 'BOUCHAUD_USB_STOCKAGE_VOLUME .*numero=2' 'la cle 2 ne republie pas le volume'
      # Geometrie ET promesse de la cle 1, et les E/S y vont : la derniere LBA
      # de 64 Mio n'existe pas sur la cle de 32 Mio.
      exige "$L" "BOUCHAUD_USB_VOLUME_ETAT present=1 lie=1 vidange_reelle=1 orphelin=0 blocs=$BLOCS1 .*descripteur_vidange=1" \
        'la sonde de la cle 2 ne retire pas la promesse de la cle 1'
      exige "$L" "BOUCHAUD_USB_VOLUME_IO lba=$((BLOCS1 - 1)) lecture=ok ecriture=ok relecture=ok vidange=1" 'E/S et vidange sur la cle 1'
      ;;
    deux-cles-inverse)
      cle "$d/cle1.img" $MIO1; cle "$d/cle2.img" $MIO2
      fabrique_autorun "$d" "$COURT" "usbfault vidange" "echo LIAISON_ETAPE_1" "$ATTENTE" \
        "echo LIAISON_ETAPE_2" "$ATTENTE" "usbvolume io" "usbvolume" "lsusb" "echo LIAISON_FIN"
      pid=$(lance "$d" $(drive c1 "$d/cle1.img") $(drive c2 "$d/cle2.img"))
      etapes "$d" "device_add usb-storage,bus=xhci.0,port=1,drive=c1,id=u1" \
        "device_add usb-storage,bus=xhci.0,port=2,drive=c2,id=u2"
      attend_fin "$pid" 120; commun "$L"
      exige "$L" 'BOUCHAUD_USB_STOCKAGE_VIDANGE numero=1 .*sonde=vidange-refus-injecte vidange_reelle=0' 'cle 1 : sonde refusee'
      exige "$L" 'BOUCHAUD_USB_STOCKAGE_VIDANGE numero=2 .*sonde=ok vidange_reelle=1' 'cle 2 : vidange prouvee'
      exige "$L" 'BOUCHAUD_USB_STOCKAGE_NON_PUBLIE numero=2 raison=volume-deja-lie lie=1' 'la cle 2 est ecartee'
      exige "$L" "BOUCHAUD_USB_VOLUME_ETAT present=1 lie=1 vidange_reelle=0 orphelin=0 blocs=$BLOCS1 .*descripteur_vidange=0" \
        'la cle 2 n accorde AUCUNE promesse a la cle 1 (commit A/B refuse)'
      exige "$L" "BOUCHAUD_USB_VOLUME_IO lba=$((BLOCS1 - 1)) lecture=ok ecriture=ok relecture=ok vidange=0" 'E/S sur la cle 1, vidange refusee'
      ;;
    refus-en-usage)
      cle "$d/cle1.img" $MIO1
      fabrique_autorun "$d" "$ENUM" "usbvolume" "usbvolume io" "usbvolume io" "usbvolume" "echo LIAISON_FIN"
      pid=$(lance "$d" $(drive c1 "$d/cle1.img" "$SORTIE/refuse-apres-ecriture.conf") -device usb-storage,bus=xhci.0,port=1,drive=c1)
      attend_fin "$pid" 240; commun "$L"
      exige "$L" 'BOUCHAUD_USB_STOCKAGE_VIDANGE numero=1 .*sonde=ok vidange_reelle=1' 'la sonde de montage passe'
      exige_n "$L" 'BOUCHAUD_USB_VOLUME_ETAT' 1 'vidange_reelle=1 .*descripteur_vidange=1' 'promesse au depart'
      exige "$L" 'BOUCHAUD_USB_STOCKAGE_VIDANGE_REFUSEE n=1 numero=1 raison=bot-commande-echouee sens=[0-9]+/0x[0-9a-f]+/0x[0-9a-f]+\([a-z-]+\) vidange_reelle=0' \
        'refus SCSI reel : REQUEST SENSE sous le verrou, promesse retiree'
      exige_n "$L" 'BOUCHAUD_USB_VOLUME_IO' 1 'lecture=ok ecriture=ok relecture=ok vidange=0' 'la vidange refusee est rendue en echec'
      exige_n "$L" 'BOUCHAUD_USB_VOLUME_IO' 2 'lecture=ok ecriture=ok relecture=ok vidange=0' 'lectures et ecritures APRES le refus'
      exige_n "$L" 'BOUCHAUD_USB_VOLUME_ETAT' '$' 'lie=1 vidange_reelle=0 orphelin=0 .*descripteur_vidange=0' 'plus aucune promesse ensuite'
      interdit "$L" 'BOUCHAUD_USB_STOCKAGE_IO_ECHEC|BOT_REPRISE_ECHEC' 'aucun echec d E/S ni de reprise'
      ;;
    rebranchement)
      cle "$d/cle1.img" $MIO1
      fabrique_autorun "$d" "$ENUM" "usbvolume io" "echo LIAISON_ETAPE_1" "$ATTENTE" \
        "usbvolume io" "echo LIAISON_ETAPE_2" "lsusb --attends 15000" "usbvolume io" "usbvolume" "echo LIAISON_FIN"
      pid=$(lance "$d" $(drive c1 "$d/cle1.img") -device usb-storage,bus=xhci.0,port=1,drive=c1,id=u1)
      # QEMU detruit le support de `-drive` avec le peripherique : la cle
      # rebranchee est un drive neuf sur LA MEME image.
      etapes "$d" "device_del u1" "drive_add 0 if=none,id=c1b,format=raw,file=$d/cle1.img"
      python3 tools/ci/qemu_moniteur.py "$d/moniteur" "device_add usb-storage,bus=xhci.0,port=1,drive=c1b,id=u1b" >/dev/null
      attend_fin "$pid" 120; commun "$L"
      exige_n "$L" 'BOUCHAUD_USB_VOLUME_IO' 1 'lecture=ok ecriture=ok relecture=ok vidange=1' 'avant : E/S et vidange'
      exige "$L" 'BOUCHAUD_USB_STOCKAGE_RETIRE slot=[0-9]+ numero=1' 'debranchement vu'
      exige "$L" 'BOUCHAUD_USB_VOLUME_ORPHELIN numero=1 vidange_reelle=0' 'le volume devient orphelin'
      exige_n "$L" 'BOUCHAUD_USB_VOLUME_IO' 2 'lecture=absent ecriture=absent relecture=absent vidange=0' 'debranche : aucune E/S, aucune vidange'
      exige "$L" 'BOUCHAUD_USB_STOCKAGE_VIDANGE numero=2 .*sonde=ok' 'la cle rebranchee recoit un NOUVEAU numero'
      exige "$L" 'BOUCHAUD_USB_STOCKAGE_NON_PUBLIE numero=2 raison=volume-orphelin' 'et n est pas reliee'
      exige_n "$L" 'BOUCHAUD_USB_VOLUME_IO' 3 'lecture=absent ecriture=absent relecture=absent vidange=0' 'rebranchee : la capacite perimee n est pas reutilisee'
      exige_n "$L" 'BOUCHAUD_USB_VOLUME_ETAT' '$' 'present=0 lie=1 vidange_reelle=0 orphelin=1 blocs=0 .*descripteur_vidange=0' 'apres rebranchement : orphelin, absent, sans promesse'
      ;;
    *) echo "scenario inconnu : $sc" >&2; ECHECS=$((ECHECS + 1)) ;;
  esac
  propre "$L" | grep -aE 'BOUCHAUD_USB_(STOCKAGE_(VIDANGE|VOLUME|NON_PUBLIE|RETIRE|PRET)|VOLUME_|INJECTION)' | sed 's/^.*\] //' > "$d/releve.txt"
done

if (( ECHECS == 0 )); then echo "USB_LIAISON_OK scenarios=${#SCENARIOS[@]}"; exit 0; fi
echo "USB_LIAISON_ECHEC echecs=$ECHECS" >&2
exit 1
