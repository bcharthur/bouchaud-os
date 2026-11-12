#!/usr/bin/env bash
# BOUCHAUD_INSTALL_SSD_V1 -- de l'image publiee a un SSD installe qui demarre et garde ses donnees.
#
#   tools/ci/run_installe_ssd_qemu.sh OUT_DIR
#
# OUT_DIR : sortie de tools/release/build_trigkey_ab_image.sh. Le « SSD » est
# un FICHIER de 2 Gio ; /sys et /proc sont factices (BO_INSTALL_RACINE_TEST) :
# AUCUN peripherique reel n'est ouvert, et l'installateur refuse /dev en mode
# test. Chaine exercee :
#   installe_ssd.py installe --execute (confirmation tapee) -> ecriture,
#   relecture, extension de STATE -> QEMU/OVMF demarre CE fichier deux fois,
#   coupe par SIGKILL : compteur de demarrages 1 puis 2, STATE montee a sa
#   taille etendue. PHYSICAL=NOT_RUN.
set -euo pipefail
cd "$(dirname "$0")/../.."
OUT=$(cd "${1:?usage: run_installe_ssd_qemu.sh OUT_DIR}" && pwd)
OVMF=${BO_OVMF_DIR:-/usr/share/OVMF}
J=$(mktemp -d)
pid=""
fin() { [ -n "$pid" ] && kill -KILL "$pid" 2>/dev/null; wait 2>/dev/null; rm -rf -- "$J"; }
trap fin EXIT
OCTETS=$((2048 * 1024 * 1024))
truncate -s "$OCTETS" "$J/ssdq"
mkdir -p "$J/racine/sys/block/ssdq/device" "$J/racine/proc"
echo "SSD Banc" > "$J/racine/sys/block/ssdq/device/model"
echo "BANC0001" > "$J/racine/sys/block/ssdq/device/serial"
echo $((OCTETS / 512)) > "$J/racine/sys/block/ssdq/size"
echo 0 > "$J/racine/sys/block/ssdq/removable"
: > "$J/racine/proc/mounts"
printf 'Filename Type Size Used Priority\n' > "$J/racine/proc/swaps"
export BO_INSTALL_RACINE_TEST="$J/racine"
ID="SSD Banc|BANC0001|$OCTETS"
I=tools/install/installe_ssd.py
# Temoin : sans confirmation, rien n'est ecrit.
if echo "non" | python3 "$I" installe --image "$OUT/disque-ab.img" --cible "$J/ssdq" --identite "$ID" --execute 2>/dev/null; then
  echo "INSTALL_SSD_FAIL confirmation_ignoree" >&2; exit 1
fi
[ "$(head -c 4096 "$J/ssdq" | tr -d '\0' | wc -c)" = 0 ] || { echo "INSTALL_SSD_FAIL refus_qui_ecrit" >&2; exit 1; }
echo "EFFACER BANC0001" | python3 "$I" installe --image "$OUT/disque-ab.img" --cible "$J/ssdq" --identite "$ID" --execute \
  | tee "$OUT/installe-ssd.log"
grep -q "^INSTALL_OK " "$OUT/installe-ssd.log" || { echo "INSTALL_SSD_FAIL installation" >&2; exit 1; }
cp "$OVMF/OVMF_VARS_4M.fd" "$J/vars.fd"
demarre() {
  local log="$OUT/ssd-boot-$1.log"
  : > "$log"
  qemu-system-x86_64 -machine q35 -m 2048 -smp 2 -cpu max \
    -drive if=pflash,format=raw,unit=0,readonly=on,file="$OVMF/OVMF_CODE_4M.fd" \
    -drive if=pflash,format=raw,unit=1,file="$J/vars.fd" \
    -drive if=none,id=disque,format=raw,file="$J/ssdq" \
    -device nvme,drive=disque,serial=BOUCHAUD_SSD_BANC \
    -display none -no-reboot -serial file:"$log" >/dev/null 2>&1 &
  pid=$!
  local limite=$((SECONDS + ${BO_STATE_AB_S:-300}))
  until grep -aq "BOUCHAUD_PERSIST_DEMARRAGES" "$log"; do
    if (( SECONDS >= limite )) || ! kill -0 "$pid" 2>/dev/null; then
      echo "INSTALL_SSD_FAIL demarrage $1 : pas de compteur" >&2
      sed -E 's/\x1b\[[0-9;]*m//g' "$log" | grep -aE "PERSIST|AB_BOOT|PANIC|LOCKDEP" | tail -6 >&2
      return 1
    fi
    sleep 1
  done
  kill -KILL "$pid"; wait "$pid" 2>/dev/null || true; pid=""
}
demarre 1
demarre 2
python3 - "$OUT" "$OCTETS" <<'PY'
import re, sys
out, octets = sys.argv[1], int(sys.argv[2])
propre = lambda p: re.sub(r"\x1b\[[0-9;]*m", "", open(p, errors="replace").read())
e = []
for k in (1, 2):
    t = propre(f"{out}/ssd-boot-{k}.log")
    m = re.findall(r"BOUCHAUD_PERSIST_DEMARRAGES n=(\d+) slot=(\w) sync=(-?\d+)", t)
    if m != [(str(k), "A", m[0][2])] if m else True:
        e.append(f"demarrage {k} : compteur {m}")
    elif int(m[0][2]) < 0:
        e.append(f"demarrage {k} : sync {m[0][2]}")
    s = re.findall(r"BOUCHAUD_INSTALL_SYSTEME_MONTE premier=\d+ blocs=(\d+)", t)
    # STATE etendue : bien plus que les 256 Mio de l'image (disque de 2 Gio).
    if not s or int(s[0]) * 512 < 1024 * 1024 * 1024:
        e.append(f"demarrage {k} : STATE non etendue ({s})")
    for x in ("*** KERNEL PANIC ***", "LOCKDEP"):
        if x in t:
            e.append(f"demarrage {k} : {x}")
for x in e:
    print("INSTALL_SSD_FAIL", x, file=sys.stderr)
if e:
    sys.exit(1)
print("INSTALL_SSD_QEMU_OK installe=oui relu=oui state_etendue=oui demarrages=2 compteur=1,2 PHYSICAL=NOT_RUN")
PY
