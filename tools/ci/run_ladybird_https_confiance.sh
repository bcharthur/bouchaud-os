#!/usr/bin/env bash
# BOUCHAUD_CA_COMPLET_V2 -- la confiance HTTPS de Ladybird (RequestServer,
# curl/OpenSSL) sous Bouchaud OS, contre des serveurs TLS 1.3 de l'hote.
#
#   tools/ci/run_ladybird_https_confiance.sh BOOTIMAGE NATIVE_DIR
#
# Le bundle de l'invite est le magasin Mozilla de l'hote PLUS une CA de banc
# nee et morte ici. Trois sites, servis par tools/ci/serveur_ota_banc.py :
#   ok           certificat signe par la CA de banc, nom couvert -> page chargee
#   ca_inconnue  meme nom, CA absente du bundle       -> RS_ECHEC code=60, AUCUNE requete servie
#   nom          CA de banc, nom NON couvert          -> RS_ECHEC code=60, AUCUNE requete servie
# (dans les deux refus, Ladybird affiche sa page d'erreur : `document_charge`
# n'est donc pas un critere ; le serveur, si.)
# Aucun relachement : ni -k, ni VERIFYPEER=0 ; la verification refuse seule.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_ladybird_https_confiance.sh BOOTIMAGE NATIVE_DIR}
OUT=${2:?usage: run_ladybird_https_confiance.sh BOOTIMAGE NATIVE_DIR}
DUREE=${BO_HTTPS_DUREE_S:-60}
SCENARIO=scenario-https
IMAGE=ladybird-https.img
LOG=serie-https.log
T=$(mktemp -d)
PIDS=()
trap 'for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null || true; done; rm -rf "$T"; tools/ci/extrait_panique.sh "$LOG" "$OUT"' EXIT
rm -rf "$SCENARIO" "$IMAGE" "$LOG"

ca() { openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 2 -subj "/CN=$2" \
  -keyout "$T/$1.key" -out "$T/$1.pem" -addext "basicConstraints=critical,CA:TRUE" \
  -addext "keyUsage=critical,keyCertSign,cRLSign" 2>/dev/null; }
feuille() { openssl req -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -subj "/CN=$3" \
  -keyout "$T/$1.key" -out "$T/$1.csr" 2>/dev/null
  printf 'subjectAltName=%s\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth\n' "$4" > "$T/$1.ext"
  openssl x509 -req -in "$T/$1.csr" -CA "$T/$2.pem" -CAkey "$T/$2.key" -CAcreateserial -days 2 \
    -extfile "$T/$1.ext" -out "$T/$1.pem" 2>/dev/null; }
ca banc "Bouchaud banc HTTPS CA"; ca inconnue "CA inconnue"
feuille ok banc 10.0.2.2 "IP:10.0.2.2"
feuille inconnu inconnue 10.0.2.2 "IP:10.0.2.2"
feuille nom banc autre.test "DNS:autre.test"
mkdir -p "$T/www"
printf '<!doctype html><html><head><title>BOUCHAUD_HTTPS_OK</title></head><body style="background:#2a6"><h1>BOUCHAUD_HTTPS_OK</h1><p>chaine de confiance verifiee</p></body></html>\n' > "$T/www/ok.html"
port() { python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])'; }
serveur() {
  python3 tools/ci/serveur_ota_banc.py "$1" "$T/www" "$T/$2.pem" "$T/$2.key" 2> "$T/$2.log" & PIDS+=($!)
  for _ in $(seq 1 50); do grep -q PRET "$T/$2.log" 2>/dev/null && return 0; sleep 0.1; done
  echo "HTTPS_CONFIANCE_ECHEC serveur_$2_non_pret" >&2; exit 1
}
P_OK=$(port); serveur "$P_OK" ok
P_IN=$(port); serveur "$P_IN" inconnu
P_NOM=$(port); serveur "$P_NOM" nom
SITES="ok=https://10.0.2.2:$P_OK/ok.html ca_inconnue=https://10.0.2.2:$P_IN/ok.html nom=https://10.0.2.2:$P_NOM/ok.html"

mkdir -p "$SCENARIO/usr/libexec/ladybird" "$SCENARIO/usr/share/ladybird" "$SCENARIO/etc/ssl/certs"
for f in BouchaudBrowserHost WebContent RequestServer ImageDecoder WebWorker Compositor WebDriver; do
  cp "$OUT/$f" "$SCENARIO/usr/libexec/ladybird/$f"; chmod 755 "$SCENARIO/usr/libexec/ladybird/$f"
done
cp "$OUT/BouchaudBrowserHost" "$SCENARIO/bo-navigateur"; chmod 755 "$SCENARIO/bo-navigateur"
[ -d "$OUT/resources" ] && cp -a "$OUT/resources/." "$SCENARIO/usr/share/ladybird/"
cat /etc/ssl/certs/ca-certificates.crt "$T/banc.pem" > "$SCENARIO/etc/ssl/certs/ca-certificates.crt"
{
  echo "export BOUCHAUD_BROWSER_HOST=1"
  echo "export BOUCHAUD_TIME_ZONE=Europe/Paris"
  echo "export BOUCHAUD_LB_BANC_DUREE_S=$DUREE"
  for s in $SITES; do
    echo "echo SITE_DEBUT nom=${s%%=*}"
    echo "export BOUCHAUD_M9_URL='${s#*=}'"
    echo "/bo-navigateur"
    echo "echo SITE_FIN nom=${s%%=*} statut=\$?"
  done
  echo "echo HTTPS_BANC_FIN"
} > "$SCENARIO/autorun"
(cd tools/userland && IMAGE="$PWD/../../$IMAGE" ./mkdisk.sh "$PWD/../../$SCENARIO" >/dev/null)

: > "$LOG"
qemu-system-x86_64 -drive format=raw,file="$BOOT" -drive format=raw,file="$IMAGE" \
  -m 8192 -smp 4 -cpu max -display none -no-reboot \
  -netdev user,id=net0 -device e1000,netdev=net0 -audiodev none,id=muet -device AC97,audiodev=muet \
  -serial file:"$LOG" &
PID=$!
LIMITE=$((SECONDS + 3 * DUREE + 600))
while kill -0 "$PID" 2>/dev/null; do
  if (( SECONDS >= LIMITE )); then echo "ECHEANCE atteinte" >&2; break; fi
  grep -aq 'HTTPS_BANC_FIN\|KERNEL PANIC' "$LOG" && { sleep 2; break; }
  sleep 2
done
kill -TERM "$PID" 2>/dev/null || true; sleep 1; kill -KILL "$PID" 2>/dev/null || true; wait "$PID" 2>/dev/null || true

sed -E 's/\x1b\[[0-9;]*m//g; s/^\[[^]]*\]\[[^]]*\]\[FPS:[^]]*\] //' "$LOG" | tr -d '\r' > "$LOG.propre"
P="$LOG.propre"
grep -aq 'KERNEL PANIC' "$P" && { echo "panique noyau" >&2; exit 1; }
passage() { awk -v n="$1" '$0 ~ "SITE_DEBUT nom="n"$" {on=1} on {print} $0 ~ "SITE_FIN nom="n" " {on=0}' "$P"; }
echecs=()
for s in $SITES; do
  nom=${s%%=*}; url=${s#*=}
  bloc=$(passage "$nom")
  charge=$(grep -acE '\[LB:NAV\] onglet=1 document_charge' <<< "$bloc" || true)
  refus=$(grep -aoE "RS_ECHEC url=$url code=[0-9]+[^\n]*" <<< "$bloc" | awk 'NR==1' || true)
  code=$(sed -nE 's/.*code=([0-9]+).*/\1/p' <<< "$refus")
  case "$nom" in ca_inconnue) srv=inconnu ;; *) srv=$nom ;; esac
  servi=$(grep -c '"GET ' "$T/$srv.log" || true)
  echo "HTTPS_SITE nom=$nom charge=$charge refus_code=${code:--} requetes_servies=$servi detail=${refus:-aucun}"
  if [ "$nom" = ok ]; then
    { [ "$charge" -ge 1 ] && [ "$servi" -ge 1 ] && [ -z "$code" ]; } || echecs+=("ok:non_charge")
  else
    # Ladybird charge alors sa PAGE D'ERREUR (document_charge vaut 1, run
    # 43432150153) : ce n'est pas le critere. La preuve est que curl a
    # refuse la chaine (code 60) et que le serveur n'a recu AUCUNE requete :
    # le contenu protege n'a jamais quitte le serveur.
    { [ "$code" = 60 ] && [ "$servi" = 0 ]; } || echecs+=("$nom:non_refuse")
  fi
done
if [ ${#echecs[@]} -eq 0 ]; then
  echo "LADYBIRD_HTTPS_CONFIANCE_OK chaine_valide=chargee ca_inconnue=refusee(60) nom_non_couvert=refuse(60) requetes_aux_refuses=0"
else
  echo "LADYBIRD_HTTPS_CONFIANCE_ECHEC ${echecs[*]}"; exit 1
fi
