#!/bin/bash
# Un segment TCP de donnees qui ne part pas doit etre renvoye, sous QEMU.
#
# ## L'incident
#
# Consentement YouTube bloque quatre minutes (journal WHPX du 18/11) : trois
# POST partis sur une connexion HTTP/2 reutilisee ne sont jamais arrives chez
# Google. `TcpConn::send` -- le TCP des sockets d'anneau 3, donc de Ladybird --
# ignorait l'echec de `send_ip_immediat` quand l'entree ARP de la passerelle
# avait expire (TTL 60 s), avancait son numero de sequence, et ne retransmettait
# jamais. Le pair attendait l'octet manquant jusqu'a fermer la connexion.
#
# ## Ce que le test prouve
#
# La sonde `tcp-perte-probe` reproduit la cause REELLE, sans injection : une
# requete, 65 s de silence (l'ARP expire), puis une requete de trois segments.
# Un serveur sur l'hote verifie chaque octet et repond. Il faut :
#
#   TCP_PERTE_E1_OK   temoin, avant l'expiration
#   TCP_PERTE_E2_OK   la requete apres expiration repond en moins de 10 s
#   TCP_PERTE_E3_OK   le flux est reste en ordre derriere
#
# et AUCUN `CORROMPU` cote serveur : une retransmission qui dupliquerait des
# octets se verrait ici.
#
# ## Ce qu'il ne prouve pas
#
# Ni TLS, ni HTTP/2, ni YouTube : il s'arrete a la couche que le defaut touche.
#
#   tools/net/verifie-tcp-perte.sh
set -euo pipefail
cd "$(dirname "$0")/../.."
RACINE=$(pwd)
SORTIE=${SORTIE:-$RACINE/target/verifie-tcp-perte}
PORT=${BO_TCP_PERTE_PORT:-18457}

rouge() { printf '\033[31m%s\033[0m\n' "$*"; }
vert()  { printf '\033[32m%s\033[0m\n' "$*"; }
info()  { printf '\033[36m%s\033[0m\n' "$*"; }

for outil in clang qemu-system-x86_64 cargo python3; do
    command -v "$outil" >/dev/null 2>&1 || { rouge "$outil absent"; exit 1; }
done

rm -rf "$SORTIE"
mkdir -p "$SORTIE/scenario"

info "== sonde =="
clang -static -nostdlib -ffreestanding -fno-stack-protector -fno-pie \
      -mcmodel=large -O1 -Wl,-Ttext-segment=0x400000000000 \
      -DPORT_SERVEUR="$PORT" \
      -o "$SORTIE/scenario/tcp-perte-probe" tools/userland/tcp-perte-probe.c
chmod 755 "$SORTIE/scenario/tcp-perte-probe"
printf 'uname\n/tcp-perte-probe\n' > "$SORTIE/scenario/autorun"

info "== disque =="
(cd tools/userland && IMAGE="$SORTIE/tcp.img" ./mkdisk.sh "$SORTIE/scenario") >/dev/null

info "== noyau =="
command -v bootimage >/dev/null 2>&1 || cargo install bootimage --locked >/dev/null
cargo bootimage >/dev/null

info "== serveur hote 127.0.0.1:$PORT =="
SERVEUR_LOG="$SORTIE/serveur.log"
python3 -u - "$PORT" > "$SERVEUR_LOG" 2>&1 <<'PY' &
import socket, sys, time
port = int(sys.argv[1])
def motif(n, i):
    return ord('a') + ((i * 7 + n * 13) % 26)
srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(("127.0.0.1", port))
srv.listen(1)
print("ECOUTE", flush=True)
c, _ = srv.accept()
f = c.makefile("rb")
while True:
    entete = f.readline()
    if not entete:
        print("FERME", flush=True)
        break
    t0 = time.monotonic()
    nom, taille = entete.decode().split()
    n, taille = int(nom[3:]), int(taille)
    corps = f.read(taille)
    attendu = bytes(motif(n, i) for i in range(taille))
    if corps == attendu:
        print(f"RECU {nom} taille={taille}", flush=True)
        c.sendall(f"OK{n}\n".encode())
    else:
        print(f"CORROMPU {nom} taille={taille} recu={len(corps)}", flush=True)
        c.sendall(f"CORROMPU{n}\n".encode())
PY
SERVEUR=$!
trap 'kill $SERVEUR 2>/dev/null || true' EXIT
for _ in $(seq 50); do grep -q ECOUTE "$SERVEUR_LOG" 2>/dev/null && break; sleep 0.1; done

info "== QEMU (silence de 65 s inclus) =="
JOURNAL="$SORTIE/serie.log"
: > "$JOURNAL"
qemu-system-x86_64 \
    -drive format=raw,file=target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin \
    -drive format=raw,file="$SORTIE/tcp.img" \
    -m 2048 -display none -no-reboot \
    -netdev user,id=net0 -device e1000,netdev=net0 \
    -serial file:"$JOURNAL" &
QEMU=$!
LIMITE=$((SECONDS + ${BO_TCP_PERTE_ECHEANCE_S:-420}))
while kill -0 "$QEMU" 2>/dev/null; do
    grep -aq 'TCP_PERTE_FIN\|TCP_PERTE_[A-Z]*_ECHEC\|KERNEL PANIC' "$JOURNAL" && { sleep 2; break; }
    (( SECONDS >= LIMITE )) && { rouge "echeance du banc atteinte"; break; }
    sleep 2
done
# E2 en echec n'arrete pas la sonde : laisser E3 se conclure.
for _ in $(seq 15); do grep -aq 'TCP_PERTE_FIN' "$JOURNAL" && break; kill -0 "$QEMU" 2>/dev/null || break; sleep 2; done
kill "$QEMU" 2>/dev/null || true
wait "$QEMU" 2>/dev/null || true
sed -i 's/\x1b\[[0-9;]*m//g' "$JOURNAL"

echo
grep -a 'tcp-perte\|TCP_PERTE_\|NET-TCP\]' "$JOURNAL" | sed 's/^\[[^]]*\]\[[^]]*\]\[[^]]*\] //' | tail -12 || true
echo "-- serveur --"
cat "$SERVEUR_LOG"
echo

echecs=0
for marqueur in TCP_PERTE_E1_OK TCP_PERTE_E2_OK TCP_PERTE_E3_OK; do
    if grep -aq "$marqueur" "$JOURNAL"; then
        ligne=$(grep -ao "$marqueur.*" "$JOURNAL" | tr -d '\r' | awk 'NR == 1')
        vert "  ok     $ligne"
    else
        rouge "  ECHEC  $marqueur absent"
        echecs=$((echecs + 1))
    fi
done
if grep -q CORROMPU "$SERVEUR_LOG"; then
    rouge "  ECHEC  le serveur a recu un flux corrompu"
    echecs=$((echecs + 1))
fi
if grep -aqi 'KERNEL PANIC' "$JOURNAL"; then
    rouge "  ECHEC  panique du noyau"
    echecs=$((echecs + 1))
fi

echo
if [ "$echecs" -ne 0 ]; then
    rouge "tcp-perte : $echecs verification(s) en echec -- journal : $JOURNAL"
    exit 1
fi
vert "tcp-perte : segment non parti renvoye, flux intact"
