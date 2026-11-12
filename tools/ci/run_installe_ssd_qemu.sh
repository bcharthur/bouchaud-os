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
installe() {  # reinstalle l'image (confirmation tapee) sur le SSD tel qu'il est
  echo "EFFACER BANC0001" | python3 "$I" installe --image "$OUT/disque-ab.img" --cible "$J/ssdq" --identite "$ID" --execute \
    >> "$OUT/installe-ssd.log"
}
zone() {  # premier_lba nb_secteurs de la zone persistante FINALE de STATE
  python3 - "$J/ssdq" <<'PY'
import sys
sys.path.insert(0, "tools/install")
import installe_ssd as ins
with open(sys.argv[1], "rb") as f:
    e, ent = ins.lit_gpt(f)
    st = [p for p in ins.partitions(e, ent) if p[1] == ins.TYPE_STATE][0]
print(st[3] + 1 - ins.SECTEURS_ZONE, ins.SECTEURS_ZONE)
PY
}
empreinte_zone() { read -r z n < <(zone); dd if="$J/ssdq" bs=512 skip="$z" count="$n" status=none | sha256sum | cut -c1-64; }
retourne() {  # LBA_RELATIF : inverse l octet 13 (generation, sous la somme de controle 0..44) d un superbloc
  read -r z n < <(zone)
  python3 -c 'import sys; p,o=sys.argv[1],int(sys.argv[2]); f=open(p,"r+b"); f.seek(o); b=f.read(1); f.seek(o); f.write(bytes([b[0]^0xFF]))' \
    "$J/ssdq" $(( (z + $1) * 512 + 13 ))
}
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
  until grep -aqE "BOUCHAUD_PERSIST_DEMARRAGES|BOUCHAUD_PERSIST_MONTAGE etat=echec" "$log"; do
    if (( SECONDS >= limite )) || ! kill -0 "$pid" 2>/dev/null; then
      echo "INSTALL_SSD_FAIL demarrage $1 : ni compteur ni echec de montage" >&2
      sed -E 's/\x1b\[[0-9;]*m//g' "$log" | grep -aE "PERSIST|AB_BOOT|PANIC|LOCKDEP" | tail -6 >&2
      return 1
    fi
    sleep 1
  done
  # Apres un montage en echec, laisser au bureau et a l'ecriture differee
  # (plafond 8 s) le temps de TENTER une synchronisation : elle doit etre refusee.
  if grep -aq "BOUCHAUD_PERSIST_MONTAGE etat=echec" "$log"; then sleep "${BO_SSD_ECHEC_S:-25}"; fi
  kill -KILL "$pid"; wait "$pid" 2>/dev/null || true; pid=""
}
# 1. Installation neuve, deux demarrages.
demarre 1
demarre 2
# 2. P1 revue #307 : REINSTALLATION sur ce SSD deja utilise. Rien de
#    l'installation precedente ne doit reapparaitre : compteur 1, volume neuf.
installe
grep -q "zone_persistante=initialisee" "$OUT/installe-ssd.log" || { echo "INSTALL_SSD_FAIL zone_non_initialisee" >&2; exit 1; }
demarre 3
demarre 4
# 3. P1 revue #307 : montage en ECHEC (les deux superblocs abimes, non nuls).
#    Aucun compteur, aucune synchronisation : la zone est INCHANGEE octet pour octet.
retourne 0; retourne 1
avant=$(empreinte_zone)
demarre 5
apres=$(empreinte_zone)
[ "$avant" = "$apres" ] || { echo "INSTALL_SSD_FAIL zone_modifiee_apres_montage_en_echec" >&2; exit 1; }
# 4. Reparation (les deux octets remis) : l'etat d'avant est intact, compteur 3.
retourne 0; retourne 1
demarre 6
python3 - "$OUT" <<'PY'
import re, sys
out = sys.argv[1]
propre = lambda p: re.sub(r"\x1b\[[0-9;]*m", "", open(p, errors="replace").read())
# (n attendu ou None, etat de montage attendu)
attendu = {1: (1, "neuf"), 2: (2, "restaure"), 3: (1, "neuf"), 4: (2, "restaure"),
           5: (None, "echec"), 6: (3, "restaure")}
e = []
for k, (n, etat) in attendu.items():
    t = propre(f"{out}/ssd-boot-{k}.log")
    m = re.findall(r"BOUCHAUD_PERSIST_DEMARRAGES n=(\d+) slot=(\w) sync=(-?\d+)", t)
    montage = re.findall(r"BOUCHAUD_PERSIST_MONTAGE etat=(\w+)", t)
    if montage != [etat]:
        e.append(f"demarrage {k} : montage {montage}, attendu {etat}")
    if n is None:
        if m:
            e.append(f"demarrage {k} : compteur ecrit apres un montage en echec {m}")
        if "superblocs_illisibles" not in t:
            e.append(f"demarrage {k} : raison de l'echec absente")
        if re.search(r"\[PERSIST\].*ecrit|BOUCHAUD_ECRITURE_DIFFEREE n=\d+ .*ok", t):
            e.append(f"demarrage {k} : une ecriture a eu lieu")
    elif [x[0] for x in m] != [str(n)] or int(m[0][2]) < 0:
        e.append(f"demarrage {k} : compteur {m}, attendu {n}")
    s = re.findall(r"BOUCHAUD_INSTALL_SYSTEME_MONTE premier=\d+ blocs=(\d+)", t)
    if not s or int(s[0]) * 512 < 1024 * 1024 * 1024:
        e.append(f"demarrage {k} : STATE non etendue ({s})")
    for x in ("*** KERNEL PANIC ***", "LOCKDEP"):
        if x in t:
            e.append(f"demarrage {k} : {x}")
for x in e:
    print("INSTALL_SSD_FAIL", x, file=sys.stderr)
if e:
    sys.exit(1)
print("INSTALL_SSD_QEMU_OK installe=oui relu=oui state_etendue=oui compteur=1,2 "
      "reinstallation=neuf(1,2) montage_echec=lecture_seule,zone_intacte reparation=3 PHYSICAL=NOT_RUN")
PY
