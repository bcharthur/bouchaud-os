#!/usr/bin/env bash
#
# BOUCHAUD_B3_UNE_REPARATION_INVOQUEE_N_EST_PAS_UNE_RECEPTION_RESTAUREE
#
# # La question
#
# Le releve physique Trigkey (`d131f2a`) dit :
#
#     rx_stall=39  repair_req=39  repair_exec=39  recoveries=39
#     recovery_failures=0  rx_ok_without_progress=54
#
# « Trente-neuf reprises, aucun echec. » Sauf que `recoveries` compte les
# EXECUTIONS de la reparation et `recovery_failures` ne monte que si la
# reprogrammation materielle de la puce echoue. Ces deux nombres sont
# compatibles avec une reception morte du debut a la fin.
#
# Ce banc exige donc que les DEUX issues soient atteignables et distinguees,
# sur une vraie panne materielle :
#
#     bras transitoire  anneau sature (RDT = RDH)   -> rearmement suffit
#                                                   -> result=effective
#     bras persistant   RCTL.EN coupe               -> rearmement impuissant
#                                                   -> result=ineffective
#
# La reparation appliquee est LA MEME dans les deux bras. Seule la panne
# change. Si les deux bras rendaient le meme verdict, le verdict ne mesurerait
# rien.
#
# Chaque bras prouve d'abord que sa panne a REELLEMENT ete injectee : la
# phase 1 genere du trafic entrant et exige que rien n'entre.
#
#     tools/ci/run_rx_recuperation.sh [image]
set -uo pipefail
cd "$(dirname "$0")/../.."

BOOT=${1:-target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin}
TRAVAIL=${BANC_TRAVAIL:-$(mktemp -d)}
mkdir -p "$TRAVAIL"
if [ -z "${BANC_TRAVAIL:-}" ]; then trap 'rm -rf "$TRAVAIL"' EXIT; fi

echo "=== image mesuree ==="
sha256sum "$BOOT"

joue() {
  local mode=$1 attendu=$2 dossier="$TRAVAIL/$1"
  mkdir -p "$dossier"
  printf 'netbanc-rx %s\npoweroff\n' "$mode" > "$dossier/autorun"
  python3 - "$dossier" <<'PY'
import sys, tarfile, pathlib
d = pathlib.Path(sys.argv[1])
image = d / "banc.img"
with tarfile.open(image, "w", format=tarfile.USTAR_FORMAT) as tar:
    info = tar.gettarinfo(str(d / "autorun"), arcname="./autorun")
    info.uid = info.gid = 0
    info.uname = info.gname = "root"
    info.mode = 0o644
    with (d / "autorun").open("rb") as src:
        tar.addfile(info, src)
with image.open("ab") as out:
    out.write(b"\0" * (4 * 1024 * 1024))
PY
  timeout -k 5 --signal=TERM 120 qemu-system-x86_64 \
    -drive format=raw,file="$BOOT" \
    -drive format=raw,file="$dossier/banc.img" \
    -m 2048 -display none -no-reboot \
    -netdev user,id=net0 -device e1000,netdev=net0 \
    -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
    -serial file:"$dossier/serie.log" >/dev/null 2>&1

  local serie
  serie=$(sed 's/\x1b\[[0-9;]*m//g' "$dossier/serie.log" 2>/dev/null || true)

  echo
  echo "########## bras $mode (attendu : $attendu) ##########"
  printf '%s\n' "$serie" | grep -aoE "RX_STALL_INJECTE[^\"]{0,140}|RX_RECOVERY_BEGIN[^\"]{0,260}|RX_RECOVERY_END[^\"]{0,300}|RX_RECOVERY_BANC[^\"]{0,80}" || true

  # DES CHAINES ICI, PAS DES TUBES, ET CE N'EST PAS UN DETAIL DE STYLE.
  #
  # `printf ... | grep -q` sous `set -o pipefail` rend 141 des que grep trouve
  # tot : il sort, `printf` prend un SIGPIPE, et le statut du TUBE devient
  # l'echec de printf. La premiere version de ce banc declarait ainsi
  # « preuve-d-injection-absente » sur une ligne qui etait bel et bien la, a la
  # ligne 93 d'un journal d'un megaoctet. Un faux rouge est aussi couteux
  # qu'un faux vert.
  contient() { grep -aqE "$1" <<<"$serie"; }

  # 1. LA PANNE A-T-ELLE ETE INJECTEE ? Un bras negatif qui passe parce que
  #    l'injection n'a pas eu lieu est un vert qui ne defend rien.
  if contient "RX_STALL_INJECTE verdict=NON_INJECTE"; then
    echo "RX_RECUPERATION bras=$mode verdict=ECHEC raison=injection-non-effective"
    return 1
  fi
  if ! contient "RX_STALL_INJECTE mode=$mode trames_pendant_injection=0"; then
    echo "RX_RECUPERATION bras=$mode verdict=ECHEC raison=preuve-d-injection-absente"
    return 1
  fi
  # 2. LE VERDICT EST-IL CELUI ATTENDU ?
  if ! contient "RX_RECOVERY_END .*result=$attendu"; then
    echo "RX_RECUPERATION bras=$mode verdict=ECHEC raison=verdict-inattendu"
    return 1
  fi
  # 3. ET L'AUTRE VERDICT NE DOIT PAS APPARAITRE DANS CE BRAS.
  #
  # Sans cela, un module qui emettrait les deux lignes a chaque reparation
  # passerait les deux bras sans rien distinguer.
  local interdit=effective
  [ "$attendu" = effective ] && interdit=ineffective
  if contient "RX_RECOVERY_END .*result=$interdit"; then
    echo "RX_RECUPERATION bras=$mode verdict=ECHEC raison=les-deux-verdicts-emis"
    return 1
  fi
  echo "RX_RECUPERATION bras=$mode verdict=OK result=$attendu"
  return 0
}

echecs=0
joue transitoire effective   || echecs=$((echecs + 1))
joue persistant  ineffective || echecs=$((echecs + 1))

echo
if [ "$echecs" -eq 0 ]; then
  echo "RX_RECUPERATION verdict=OK effective_et_ineffective_tous_deux_atteints"
  exit 0
fi
echo "RX_RECUPERATION verdict=ECHEC bras_en_echec=$echecs"
exit 1
