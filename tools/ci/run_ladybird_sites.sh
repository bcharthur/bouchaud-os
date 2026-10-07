#!/usr/bin/env bash
# Sites reels en HTTPS : document recu, PAGE PEINTE, pixels non uniformes
# (BOUCHAUD_SITES_REELS_V1).
#
#   tools/ci/run_ladybird_sites.sh BOOTIMAGE NATIVE_DIR
#
# Une session invitee, un navigateur par site (sans fenetre : le Compositor
# peint quand meme, et la sonde de pixels lit la trame qu'il presente).
# Chaque passage dure BO_SITES_DUREE_S secondes (90 par defaut), puis le
# navigateur rejoue sa derniere sonde (`[LB] PRESENT_DERNIER`) et quitte.
#
# Distingue enfin, au lieu de deduire « page blanche » du DOM :
#   - document jamais charge      (pas de `[LB:NAV] ... document_charge`)
#   - charge mais rien peint       (aucune trame `[LB] PRESENT`)
#   - peint mais uniforme          (variance ~0, une ou deux couleurs)
#   - page reelle                  (variance et couleurs)
#
# Exige Example Domain. Wikipedia est MESURE (site externe lourd, sous TCG) :
# son resultat est imprime et ne fait pas echouer le banc.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_ladybird_sites.sh BOOTIMAGE NATIVE_DIR}
OUT=${2:?usage: run_ladybird_sites.sh BOOTIMAGE NATIVE_DIR}
DUREE=${BO_SITES_DUREE_S:-90}
SITES="example=https://example.com/ wikipedia=https://en.wikipedia.org/wiki/Main_Page"

SCENARIO=scenario-sites
IMAGE=ladybird-sites.img
LOG=serie-sites.log
rm -rf "$SCENARIO" "$IMAGE" "$LOG"
# Panique noyau : son contexte en DERNIER (tools/ci/extrait_panique.sh).
trap 'tools/ci/extrait_panique.sh "$LOG"' EXIT
mkdir -p "$SCENARIO/usr/libexec/ladybird" "$SCENARIO/usr/share/ladybird" "$SCENARIO/etc/ssl/certs"
for f in BouchaudBrowserHost WebContent RequestServer ImageDecoder WebWorker Compositor WebDriver; do
  cp "$OUT/$f" "$SCENARIO/usr/libexec/ladybird/$f"
  chmod 755 "$SCENARIO/usr/libexec/ladybird/$f"
done
cp "$OUT/BouchaudBrowserHost" "$SCENARIO/bo-navigateur"
chmod 755 "$SCENARIO/bo-navigateur"
if [ -d "$OUT/resources" ]; then
  cp -a "$OUT/resources/." "$SCENARIO/usr/share/ladybird/"
fi
cp /etc/ssl/certs/ca-certificates.crt "$SCENARIO/etc/ssl/certs/ca-certificates.crt"

{
  echo "export BOUCHAUD_BROWSER_HOST=1"
  echo "export BOUCHAUD_TIME_ZONE=Europe/Paris"
  echo "export BOUCHAUD_LB_BANC_DUREE_S=$DUREE"
  for s in $SITES; do
    nom=${s%%=*}; url=${s#*=}
    echo "echo SITE_DEBUT nom=$nom"
    echo "export BOUCHAUD_M9_URL='$url'"
    echo "/bo-navigateur"
    echo "echo SITE_FIN nom=$nom statut=\$?"
  done
  echo "echo SITES_BANC_FIN"
} > "$SCENARIO/autorun"
(cd tools/userland && IMAGE="$PWD/../../$IMAGE" ./mkdisk.sh "$PWD/../../$SCENARIO" >/dev/null)

: > "$LOG"
qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$IMAGE" \
  -m 8192 -smp 4 -cpu max -display none -no-reboot \
  -netdev user,id=net0 -device e1000,netdev=net0 \
  -audiodev none,id=muet -device AC97,audiodev=muet \
  -serial file:"$LOG" &
PID=$!
LIMITE=$((SECONDS + 2 * DUREE + 600))
while kill -0 "$PID" 2>/dev/null; do
  if (( SECONDS >= LIMITE )); then echo "ECHEANCE atteinte" >&2; break; fi
  grep -aq 'SITES_BANC_FIN\|KERNEL PANIC' "$LOG" && { sleep 2; break; }
  sleep 2
done
kill -TERM "$PID" 2>/dev/null || true
sleep 1
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true

sed -E 's/\x1b\[[0-9;]*m//g; s/^\[[^]]*\]\[[^]]*\]\[FPS:[^]]*\] //' "$LOG" | tr -d '\r' > "$LOG.propre"
P="$LOG.propre"
if grep -aq 'KERNEL PANIC' "$P"; then echo "panique noyau" >&2; exit 1; fi

# Le passage de chaque site : les lignes entre son SITE_DEBUT et son SITE_FIN.
passage() { awk -v n="$1" '$0 ~ "SITE_DEBUT nom="n"$" {on=1} on {print} $0 ~ "SITE_FIN nom="n" " {on=0}' "$P"; }
champ() { echo "$1" | grep -oE "$2=[0-9a-f]+" | head -1 | cut -d= -f2; }

echecs=()
for s in $SITES; do
  nom=${s%%=*}; url=${s#*=}
  bloc=$(passage "$nom")
  charge=$(echo "$bloc" | grep -acE '\[LB:NAV\] onglet=1 document_charge' || true)
  trames=$(echo "$bloc" | grep -ac '\[LB\] PRESENT onglet=' || true)
  dernier=$(echo "$bloc" | grep -aoE '\[LB\] PRESENT_DERNIER .*' | tail -1 || true)
  variance=$(champ "$dernier" variance); couleurs=$(champ "$dernier" couleurs)
  non_blanc=$(champ "$dernier" non_blanc_pct); taille=$(echo "$dernier" | grep -oE 'taille=[0-9]+x[0-9]+' | cut -d= -f2)
  tls=$(echo "$bloc" | grep -acE 'SSL|TLS|certificate' || true)
  # Deux axes independants : l'evenement `load` (document_charge) et ce que
  # la trame presentee montre. Le run 37532626400 a classe Wikipedia
  # « document_non_charge » alors qu'il avait peint 16 trames, 64 couleurs :
  # la page etait affichee, seul `load` manquait au bout de 90 s.
  if [ "$trames" -eq 0 ]; then peint=rien_peint
  elif [ "${variance:-0}" -lt 50 ] || [ "${couleurs:-0}" -lt 3 ]; then peint=peint_uniforme
  else peint=page_reelle; fi
  if [ "$charge" -ge 1 ]; then verdict=$peint
  elif [ "$peint" = rien_peint ]; then verdict=document_non_charge
  else verdict=${peint}_sans_load; fi
  miss=$(echo "$bloc" | grep -ac '\[LB\] MISS url=' || true)
  store=$(echo "$bloc" | grep -ac '\[LB\] STORE url=' || true)
  hit=$(echo "$bloc" | grep -ac '\[LB\] HIT url=' || true)
  echo "SITE nom=$nom url=$url verdict=$verdict load=$charge peint=$peint trames=$trames taille=${taille:-?} variance=${variance:-?} couleurs=${couleurs:-?} non_blanc_pct=${non_blanc:-?} cache_miss=$miss cache_store=$store cache_hit=$hit lignes_tls=$tls"
  # awk lit tout le flux : pas de tube ferme avant la fin (SIGPIPE).
  echo "$bloc" | grep -aoE '\[LB:NAV\].*|\[LB\] (MISS|STORE|HIT) url=https://[^ ]{0,80}|M9_RS_[A-Z_]*(FAIL|ERR)[^ ]*.*|Request finished with error.*' | awk 'NR <= 8 { print "    " $0 }' || true
  if [ "$nom" = example ] && [ "$verdict" != page_reelle ]; then echecs+=("example:$verdict"); fi
done

if [ ${#echecs[@]} -eq 0 ]; then
  echo "LADYBIRD_SITES_OK"
else
  echo "LADYBIRD_SITES_ECHEC ${echecs[*]}"
  exit 1
fi
