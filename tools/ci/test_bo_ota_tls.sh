#!/usr/bin/env bash
# BOUCHAUD_OTA_ANNEAU3_V1 -- le TLS STRICT de l'agent OTA de l'anneau 3, sur
# l'hote, contre de vrais serveurs TLS 1.3 (tools/ci/serveur_ota_banc.py).
#
# La pile est celle du noyau (system/tls-core inclut ses fichiers) ; ici elle
# tourne sur un socket Linux. Temoins :
#   + CA de banc, nom couvert          -> 200, octets et empreinte exacts ;
#   - CA inconnue (meme nom, meme IP)  -> refus=confiance, et le serveur n'a
#                                         recu AUCUNE requete ;
#   - nom non couvert par le certificat-> refus=confiance (nom=false) ;
#   - serveur absent                   -> refus=transport.
set -euo pipefail
RACINE=$(cd "$(dirname "$0")/../.." && pwd)
T=$(mktemp -d)
PIDS=()
trap 'for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null || true; done; rm -rf "$T"' EXIT
ca() {  # NOM SUJET
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 2 -subj "/CN=$2" \
    -keyout "$T/$1.key" -out "$T/$1.pem" -addext "basicConstraints=critical,CA:TRUE" \
    -addext "keyUsage=critical,keyCertSign,cRLSign" 2>/dev/null
}
feuille() {  # NOM CA SAN
  openssl req -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -subj "/CN=$3" \
    -keyout "$T/$1.key" -out "$T/$1.csr" 2>/dev/null
  printf 'subjectAltName=DNS:%s\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth\n' "$3" > "$T/$1.ext"
  openssl x509 -req -in "$T/$1.csr" -CA "$T/$2.pem" -CAkey "$T/$2.key" -CAcreateserial -days 2 \
    -extfile "$T/$1.ext" -out "$T/$1.pem" 2>/dev/null
}
ca banc "Bouchaud banc CA"; ca inconnue "CA inconnue"
feuille srv banc 127.0.0.1; feuille srv2 inconnue 127.0.0.1; feuille srv3 banc autre.test
openssl x509 -in "$T/banc.pem" -outform DER -out "$T/banc.der"
mkdir -p "$T/www/rel"; head -c 300000 /dev/urandom > "$T/www/rel/blob"
attendu=$(sha256sum "$T/www/rel/blob" | cut -d' ' -f1)
BOUCHAUD_CA_BANC_DEV="$T/banc.der" BO_OTA_TARGET_DIR="$T/cible" "$RACINE/tools/release/build_bo_ota.sh" "$T/bo-ota" --banc
port() { python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])'; }
serveur() {  # PORT CERT CLE JOURNAL
  python3 "$RACINE/tools/ci/serveur_ota_banc.py" "$1" "$T/www" "$2" "$3" 2> "$4" & PIDS+=($!)
  for _ in $(seq 1 50); do grep -q PRET "$4" 2>/dev/null && return 0; sleep 0.1; done
  echo "BO_OTA_TLS_ECHEC serveur_non_pret $4" >&2; exit 1
}
P1=$(port); serveur "$P1" "$T/srv.pem" "$T/srv.key" "$T/s1.log"
P2=$(port); serveur "$P2" "$T/srv2.pem" "$T/srv2.key" "$T/s2.log"
P3=$(port); serveur "$P3" "$T/srv3.pem" "$T/srv3.key" "$T/s3.log"
echec=0
verifie() {  # LIBELLE CODE_ATTENDU MOTIF URL
  local sortie rc=0
  sortie=$("$T/bo-ota" --sonde "$4") || rc=$?
  if [ "$rc" = "$2" ] && grep -qE "$3" <<< "$sortie"; then
    echo "BO_OTA_TLS_OK   $1 :: $sortie"
  else
    echo "BO_OTA_TLS_FAIL $1 rc=$rc attendu=$2 :: $sortie"; echec=1
  fi
}
verifie "CA de banc acceptee" 0 "statut=200 octets=300000 sha256=$attendu" "https://127.0.0.1:$P1/rel/blob"
verifie "CA inconnue refusee" 2 "refus=confiance TLS_REFUS confiance=false" "https://127.0.0.1:$P2/rel/blob"
verifie "nom non couvert refuse" 2 "refus=confiance TLS_REFUS .*nom=false" "https://127.0.0.1:$P3/rel/blob"
verifie "serveur absent" 3 "refus=transport" "https://127.0.0.1:1/rel/blob"
if grep -q '"GET ' "$T/s2.log" || grep -q '"GET ' "$T/s3.log"; then
  echo "BO_OTA_TLS_FAIL une requete a ete servie malgre le refus de confiance"; echec=1
else
  echo "BO_OTA_TLS_OK   aucune requete envoyee aux serveurs refuses"
fi
[ "$echec" = 0 ] && echo "BO_OTA_TLS_TOUT_OK" || { echo "BO_OTA_TLS_ECHEC"; exit 1; }
