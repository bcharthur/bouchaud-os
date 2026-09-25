#!/usr/bin/env bash
#
# BRDP / telemetry : preuve de bout en bout sur un vrai segment Ethernet TAP.
#
# Ce banc existe parce qu'un faux serveur Python ne prouve ni le tri Ethernet,
# ni l'ARP, ni l'ingress unique, ni smoltcp, ni l'absence de RST fratricide.
# Il construit une image LAB tokenisee, relie QEMU a un TAP 169.254/16 et fait
# parler le VRAI `bouchaud-lab.py` au VRAI serveur du noyau.
set -euo pipefail
cd "$(dirname "$0")/../.."

TOKEN="bouchaud-qemu-lab-test"
MAC_GUEST="52:54:00:12:34:56"
IP_HOTE="169.254.254.1/16"
CLIENT="tools/remote/bouchaud-lab.py"
LIVE="tools/remote/brdp_qemu_live.py"
PCAP_CHECK="tools/remote/brdp_pcap_check.py"

TRAVAIL=${BANC_TRAVAIL:-target/brdp-qemu-e2e}
mkdir -p "$TRAVAIL"
TRAVAIL=$(python3 -c 'import os,sys; print(os.path.abspath(sys.argv[1]))' "$TRAVAIL")
rm -rf "$TRAVAIL"/*

TAP="bolab$$"
# IFNAMSIZ inclut le NUL : quinze caracteres visibles au maximum.
TAP=${TAP:0:15}
SERIE="$TRAVAIL/serial.log"
PCAP="$TRAVAIL/brdp.pcap"
QEMU_LOG="$TRAVAIL/qemu.stderr.log"
QEMU_PID=""
TELE_PID=""
DISC_PID=""

if [ "$(id -u)" -eq 0 ]; then
    SUDO=()
else
    SUDO=(sudo)
fi

nettoie() {
    set +e
    if [ -n "$DISC_PID" ]; then kill "$DISC_PID" >/dev/null 2>&1 || true; fi
    if [ -n "$TELE_PID" ]; then kill "$TELE_PID" >/dev/null 2>&1 || true; fi
    if [ -n "$QEMU_PID" ]; then
        kill -TERM "$QEMU_PID" >/dev/null 2>&1 || true
        for _ in $(seq 1 20); do
            kill -0 "$QEMU_PID" >/dev/null 2>&1 || break
            sleep 0.1
        done
        kill -KILL "$QEMU_PID" >/dev/null 2>&1 || true
        wait "$QEMU_PID" >/dev/null 2>&1 || true
    fi
    "${SUDO[@]}" ip link del "$TAP" >/dev/null 2>&1 || true
}
trap nettoie EXIT INT TERM

for outil in python3 qemu-system-x86_64 ip strings; do
    command -v "$outil" >/dev/null 2>&1 || {
        echo "BOUCHAUD_BRDP_QEMU_E2E_ECHEC raison=outil-absent outil=$outil" >&2
        exit 1
    }
done
[ -c /dev/net/tun ] || {
    echo "BOUCHAUD_BRDP_QEMU_E2E_ECHEC raison=tun-absent" >&2
    exit 1
}

# ---------------------------------------------------------------------------
# 1. IMAGE LAB. L'artefact Integration ordinaire est volontairement sans jeton.
# ---------------------------------------------------------------------------
BOOT=${1:-}
if [ -z "$BOOT" ]; then
    echo "=== construction bootimage LAB ==="
    BOUCHAUD_DEBUG_TOKEN="$TOKEN" tools/ci/build_kernel.sh
    BOOT="target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin"
fi
[ -s "$BOOT" ] || {
    echo "BOUCHAUD_BRDP_QEMU_E2E_ECHEC raison=bootimage-absente" >&2
    exit 1
}

# Deux preuves independantes : le fil BRDP est present ET le jeton de BANC a
# bien influence CETTE image. `grep -q` lit un fichier, pas un tube : aucun
# faux negatif SIGPIPE sous pipefail.
if ! grep -aFq 'bouchaud-brdp' "$BOOT"; then
    echo "BOUCHAUD_BRDP_QEMU_E2E_ECHEC raison=image-sans-brdp" >&2
    exit 1
fi
if ! grep -aFq "$TOKEN" "$BOOT"; then
    echo "BOUCHAUD_BRDP_QEMU_E2E_ECHEC raison=image-non-tokenisee" >&2
    exit 1
fi
sha256sum "$BOOT" > "$TRAVAIL/bootimage.sha256"

# ---------------------------------------------------------------------------
# 2. VRAI SEGMENT L2, PAS SLIRP.
# ---------------------------------------------------------------------------
"${SUDO[@]}" ip tuntap add dev "$TAP" mode tap user "$(id -un)"
"${SUDO[@]}" ip addr add "$IP_HOTE" dev "$TAP"
"${SUDO[@]}" ip link set "$TAP" up
ip -details addr show dev "$TAP" > "$TRAVAIL/tap.txt"

# Le listener part AVANT le guest. Il est passif : aucun paquet n'est emis.
python3 "$CLIENT" telemetry --watch > "$TRAVAIL/telemetry-boot.log" 2>&1 &
TELE_PID=$!

# shellcheck disable=SC2046
qemu-system-x86_64 \
  $(. tools/ci/plateforme.sh; bouchaud_machine_args) \
  -m 2048 -smp 4 \
  -drive format=raw,file="$BOOT" \
  -display none -no-reboot \
  -netdev tap,id=lab0,ifname="$TAP",script=no,downscript=no \
  -device e1000,netdev=lab0,mac="$MAC_GUEST" \
  -object filter-dump,id=labcap,netdev=lab0,file="$PCAP" \
  -serial file:"$SERIE" \
  > /dev/null 2> "$QEMU_LOG" &
QEMU_PID=$!

echo "=== attente LAB_READY + premiere telemetrie ==="
PRET=0
for _ in $(seq 1 180); do
    if grep -aFq 'BOUCHAUD_LAB_READY' "$SERIE" 2>/dev/null && \
       grep -aEq '169\.254\.[0-9]+\.[0-9]+' "$TRAVAIL/telemetry-boot.log" 2>/dev/null; then
        PRET=1
        break
    fi
    kill -0 "$QEMU_PID" >/dev/null 2>&1 || break
    sleep 0.25
done
if [ "$PRET" -ne 1 ]; then
    echo "BOUCHAUD_BRDP_QEMU_E2E_ECHEC raison=lab-non-pret" >&2
    cat "$SERIE" 2>/dev/null || true
    exit 1
fi
if ! grep -aEq 'BOUCHAUD_LAB_READY .*brdp=armed .*telemetry=running' "$SERIE"; then
    echo "BOUCHAUD_BRDP_QEMU_E2E_ECHEC raison=services-lab-incomplets" >&2
    exit 1
fi

HOST=$(python3 - "$TRAVAIL/telemetry-boot.log" <<'PY'
import re,sys
s=open(sys.argv[1],encoding='utf-8',errors='replace').read()
m=re.search(r'\b(169\.254\.(?:[0-9]{1,3})\.(?:[0-9]{1,3}))\b',s)
if not m: raise SystemExit(1)
print(m.group(1))
PY
) || {
    echo "BOUCHAUD_BRDP_QEMU_E2E_ECHEC raison=ip-telemetrie-introuvable" >&2
    exit 1
}
kill "$TELE_PID" >/dev/null 2>&1 || true
wait "$TELE_PID" >/dev/null 2>&1 || true
TELE_PID=""
printf '%s\n' "$HOST" > "$TRAVAIL/guest-ip.txt"

echo "guest decouvert par telemetrie: $HOST"
ip route get "$HOST" > "$TRAVAIL/route.txt"
if ! grep -Fq "dev $TAP" "$TRAVAIL/route.txt"; then
    echo "BOUCHAUD_BRDP_QEMU_E2E_ECHEC raison=route-hors-tap" >&2
    cat "$TRAVAIL/route.txt" >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# 3. PREMIER BRDP REEL. Les retries couvrent seulement la montee initiale ; le
#    banc live qui suit exige ensuite douze poignees rapides consecutives.
# ---------------------------------------------------------------------------
export BOUCHAUD_DEBUG_TOKEN="$TOKEN"
BRDP_PRET=0
for tentative in $(seq 1 30); do
    if python3 "$CLIENT" --json status --host "$HOST" --timeout 1.5 \
        > "$TRAVAIL/status-initial.json" 2> "$TRAVAIL/status-initial.err"; then
        BRDP_PRET=1
        break
    fi
    sleep 0.2
done
if [ "$BRDP_PRET" -ne 1 ]; then
    echo "BOUCHAUD_BRDP_QEMU_E2E_ECHEC raison=brdp-injoignable" >&2
    cat "$TRAVAIL/status-initial.err" >&2 || true
    exit 1
fi

# ---------------------------------------------------------------------------
# 4. DISCOVER, REELLEMENT PASSIF. On le lance avant une commande qui cree des
#    evenements LAB ; sa seule source de connaissance doit rester UDP 2223.
# ---------------------------------------------------------------------------
python3 "$CLIENT" --json discover --timeout 3 > "$TRAVAIL/discover.json" 2> "$TRAVAIL/discover.err" &
DISC_PID=$!
sleep 0.25
python3 "$CLIENT" --json status --host "$HOST" --timeout 1.5 > "$TRAVAIL/status-trigger.json"
wait "$DISC_PID"
DISC_PID=""
HOST_DISC=$(python3 - "$TRAVAIL/discover.json" "$HOST" <<'PY'
import json,sys
j=json.load(open(sys.argv[1],encoding='utf-8'))
attendu=sys.argv[2]
h=[x.get('host') for x in j.get('hotes',[])]
if attendu not in h: raise SystemExit(1)
print(attendu)
PY
) || {
    echo "BOUCHAUD_BRDP_QEMU_E2E_ECHEC raison=discover-n-a-pas-vu-le-guest" >&2
    exit 1
}

# ---------------------------------------------------------------------------
# 5. COMMANDES DU VRAI CLIENT.
# ---------------------------------------------------------------------------
mkdir -p "$TRAVAIL/commandes"
commande() {
    local nom=$1; shift
    echo "  BRDP $nom"
    python3 "$CLIENT" --json "$nom" "$@" --host "$HOST" --timeout 2.0 \
        > "$TRAVAIL/commandes/$nom.json"
}
commande status
commande audit
commande audit-run
commande audit-last
commande net
commande rtl8168
commande rtl8168-ring
commande rtl8168-desc 0
mv "$TRAVAIL/commandes/rtl8168-desc.json" "$TRAVAIL/commandes/rtl8168-desc-0.json"
commande rtl8168-desc 63
mv "$TRAVAIL/commandes/rtl8168-desc.json" "$TRAVAIL/commandes/rtl8168-desc-63.json"
commande dhcp
commande blackbox
commande services
commande processes
commande memory
echo "=== events tail 5 ==="
python3 "$CLIENT" --json events --tail 5 --host "$HOST" --timeout 2.0 \
    > "$TRAVAIL/commandes/events-tail-5.jsonl"
EVENTS=$(python3 - "$TRAVAIL/commandes/events-tail-5.jsonl" <<'PY'
import json,sys
n=0
for l in open(sys.argv[1],encoding='utf-8'):
    l=l.strip()
    if not l: continue
    json.loads(l); n+=1
print(n)
PY
)
if [ "$EVENTS" -lt 1 ] || [ "$EVENTS" -gt 5 ]; then
    echo "BOUCHAUD_BRDP_QEMU_E2E_ECHEC raison=events-tail-compte compte=$EVENTS" >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# 6. TELEMETRIE PROVOQUEE PAR UNE ACTION REELLE.
# ---------------------------------------------------------------------------
echo "=== telemetrie provoquee ==="
python3 "$CLIENT" telemetry --timeout 3 > "$TRAVAIL/telemetry-action.log" 2>&1 &
TELE_PID=$!
sleep 0.25
python3 "$CLIENT" --json audit-run --host "$HOST" --timeout 2.0 \
    > "$TRAVAIL/audit-run-trigger.json"
wait "$TELE_PID"
TELE_PID=""
if ! grep -Fq "$HOST" "$TRAVAIL/telemetry-action.log"; then
    echo "BOUCHAUD_BRDP_QEMU_E2E_ECHEC raison=telemetrie-apres-action-absente" >&2
    cat "$TRAVAIL/telemetry-action.log" >&2 || true
    exit 1
fi

# ---------------------------------------------------------------------------
# 7. CAS TCP DIFFICILES CONTRE LE VRAI SERVEUR + INVARIANT DE VIVACITE RX.
# ---------------------------------------------------------------------------
echo "=== cas TCP difficiles BRDP ==="
if ! python3 "$LIVE" --host "$HOST" --timeout 2.0 --rounds 12 \
    --max-handshake-ms 1500 \
    --out "$TRAVAIL/live.json" > "$TRAVAIL/live.log" 2>&1; then
    cat "$TRAVAIL/live.log" >&2 || true
    echo "BOUCHAUD_BRDP_QEMU_E2E_ECHEC raison=brdp-live" >&2
    exit 1
fi
cat "$TRAVAIL/live.log"

# ---------------------------------------------------------------------------
# 8. DUMP REEL DU CLIENT. Une commande en erreur reste un fichier d'erreur ;
#    `dump` porte deja ce contrat et ses tests hote.
# ---------------------------------------------------------------------------
mkdir -p "$TRAVAIL/dumps"
python3 "$CLIENT" dump --host "$HOST" --timeout 2.0 --events 100 --out "$TRAVAIL/dumps" \
    > "$TRAVAIL/dump.log"
DUMP_DIR=$(python3 - "$TRAVAIL/dumps" <<'PY'
from pathlib import Path
import sys
p=Path(sys.argv[1])
d=sorted((x for x in p.iterdir() if x.is_dir()), key=lambda x:x.stat().st_mtime)
if not d: raise SystemExit(1)
print(d[-1])
PY
) || {
    echo "BOUCHAUD_BRDP_QEMU_E2E_ECHEC raison=dump-absent" >&2
    exit 1
}
for f in metadata.json summary.json audit.json audit-last.json net.json rtl8168.json \
         rtl8168-ring.json rtl8168-desc-0.json rtl8168-desc-63.json dhcp.json \
         blackbox.json services.json processes.json memory.json events.jsonl; do
    [ -s "$DUMP_DIR/$f" ] || {
        echo "BOUCHAUD_BRDP_QEMU_E2E_ECHEC raison=dump-fichier-absent fichier=$f" >&2
        exit 1
    }
done

# ---------------------------------------------------------------------------
# 9. ARRET QEMU POUR VIDER LE PCAP, PUIS PREUVE ANTI-RST / INDEPENDANCE DHCP.
# ---------------------------------------------------------------------------
kill -TERM "$QEMU_PID" >/dev/null 2>&1 || true
for _ in $(seq 1 50); do
    kill -0 "$QEMU_PID" >/dev/null 2>&1 || break
    sleep 0.1
done
kill -KILL "$QEMU_PID" >/dev/null 2>&1 || true
wait "$QEMU_PID" >/dev/null 2>&1 || true
QEMU_PID=""

python3 "$PCAP_CHECK" "$PCAP" --guest-ip "$HOST" --guest-mac "$MAC_GUEST" \
    --max-synack-ms 1500 --out "$TRAVAIL/pcap.json" > "$TRAVAIL/pcap.log"

# Le secret ne doit etre dans AUCUNE preuve publiee. L'image, qui le contient
# par conception, n'est pas dans TRAVAIL.
if grep -R -aFq "$TOKEN" "$TRAVAIL"; then
    echo "BOUCHAUD_BRDP_QEMU_E2E_ECHEC raison=jeton-dans-artifact" >&2
    exit 1
fi

cat > "$TRAVAIL/resultat.txt" <<EOF
BOUCHAUD_BRDP_QEMU_E2E_OK
host=$HOST
tap=$TAP
host_ip=$IP_HOTE
commands=15
events_tail=$EVENTS
EOF
cat "$TRAVAIL/resultat.txt"

echo "BOUCHAUD_BRDP_QEMU_E2E_OK"
