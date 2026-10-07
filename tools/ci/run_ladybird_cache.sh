#!/usr/bin/env bash
# Le cache HTTP disque et la base SQL survivent a un redemarrage COMPLET du
# navigateur (BOUCHAUD_CACHE_REDEMARRAGE_V1, convergence P2).
#
#   tools/ci/run_ladybird_cache.sh BOOTIMAGE NATIVE_DIR
#
# Une session invitee, deux navigateurs l'un apres l'autre :
#   passage 1 : /bo-navigateur charge la page de banc, qui lit trois
#               ressources, ecrit localStorage et recoit un cookie persistant,
#               puis prend le titre BOUCHAUD_BANC_QUITTE : le navigateur
#               quitte et ferme ses services (BROWSER_HOST_EXIT) ;
#   passage 2 : un NOUVEAU /bo-navigateur (nouveaux RequestServer, WebContent)
#               recharge la meme page.
# Le serveur hote (tools/ci/ladybird_cache_fixture.py) est le TEMOIN : ce qui
# vient du cache disque ne lui parvient pas.
#
# Exige :
#   - passage 1 : [LB] STORE de /cache/stable.bin (65536 octets) ;
#   - passage 2 : [LB] HIT de /cache/stable.bin, ET le serveur n'a recu
#     qu'UN SEUL GET de /cache/stable.bin sur les deux passages, ET le
#     contenu lu est identique (meme somme) ;
#   - /cache/inchange.txt : revalide -> 304 (`inm="inchange-v1"`) ;
#   - /cache/change.txt : revalide -> 200 v2, et la page lit v2 ;
#   - SQLite : localStorage relu (HOST_SQL passage=2 REOPEN_OK) et cookie
#     renvoye au serveur au passage 2 (cookie=present) ;
#   - deux RequestServer distincts (le redemarrage est reel) ;
#   - aucune panique, aucune assertion.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_ladybird_cache.sh BOOTIMAGE NATIVE_DIR}
OUT=${2:?usage: run_ladybird_cache.sh BOOTIMAGE NATIVE_DIR}
PORT=18084

SCENARIO=scenario-cache
IMAGE=ladybird-cache.img
LOG=serie-cache.log
TEMOIN=temoin-cache.log
rm -rf "$SCENARIO" "$IMAGE" "$LOG" "$TEMOIN"
: > "$TEMOIN"

python3 tools/ci/ladybird_cache_fixture.py "$PORT" "$TEMOIN" &
SERVEUR=$!
# Panique noyau : son contexte en DERNIER (tools/ci/extrait_panique.sh).
trap 'kill "$SERVEUR" 2>/dev/null || true; tools/ci/extrait_panique.sh "$LOG"' EXIT
sleep 1
kill -0 "$SERVEUR"

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

# Pas de bureau : le navigateur tourne sans fenetre (`[LB:UI] sans_fenetre=1`)
# -- le reseau, le cache et la base SQL n'en dependent pas -- et l'autorun
# peut le lancer deux fois de suite.
# Le navigateur lit son URL dans BOUCHAUD_M9_URL (ui-bouchaud/main.cpp), pas
# dans argv : le run 37522655079 a ouvert le defaut, https://example.com/.
cat > "$SCENARIO/autorun" <<AUTORUN
export BOUCHAUD_BROWSER_HOST=1
export BOUCHAUD_TIME_ZONE=Europe/Paris
export BOUCHAUD_LB_BANC_QUITTE=1
echo CACHE_PASSAGE_1_DEBUT
export BOUCHAUD_M9_URL='http://10.0.2.2:$PORT/cache-test.html?passage=1'
/bo-navigateur
echo CACHE_PASSAGE_1_SORTI statut=\$?
echo CACHE_PASSAGE_2_DEBUT
export BOUCHAUD_M9_URL='http://10.0.2.2:$PORT/cache-test.html?passage=2'
/bo-navigateur
echo CACHE_PASSAGE_2_SORTI statut=\$?
echo CACHE_BANC_FIN
AUTORUN
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
LIMITE=$((SECONDS + ${BO_CACHE_PLAFOND_S:-900}))
while kill -0 "$PID" 2>/dev/null; do
  if (( SECONDS >= LIMITE )); then echo "ECHEANCE atteinte" >&2; break; fi
  grep -aq 'CACHE_BANC_FIN\|KERNEL PANIC' "$LOG" && { sleep 2; break; }
  sleep 1
done
kill -TERM "$PID" 2>/dev/null || true
sleep 1
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true

# Nettoye UNE fois : sous `pipefail`, `sed | grep -q` echoue des que grep
# ferme le tube avant la fin (SIGPIPE), meme quand la ligne est trouvee.
sed -E 's/\x1b\[[0-9;]*m//g; s/^\[[^]]*\]\[[^]]*\]\[FPS:[^]]*\] //' "$LOG" | tr -d '\r' > "$LOG.propre"
propre() { cat "$LOG.propre"; }
echo "== invite =="
propre | grep -aE 'CACHE_PASSAGE|CACHE_BANC_FIN|HOST_CACHE|HOST_SQL|\[LB\] (MISS|STORE|HIT|REVALIDATE|REVALIDATED|INVALIDATE|BROWSER_QUIT)|BROWSER_HOST_(EXIT|ARRET)|PERF_EXECVE .*RequestServer|KERNEL PANIC|VERIFICATION FAILED' \
  | sed -E 's/^.*(\[LB\]|HOST_|CACHE_|BROWSER_HOST|PERF_EXECVE|KERNEL|VERIFICATION)/\1/' | head -80 || true
echo "== temoin (serveur hote) =="
cat "$TEMOIN"

echecs=()
exige() { # exige <description> <commande...>
  local quoi=$1; shift
  if "$@"; then echo "  ok      $quoi"; else echo "  ECHEC   $quoi"; echecs+=("$quoi"); fi
}
dans_invite() { grep -aqE "$1" "$LOG.propre"; }
gets_stable=$(grep -c 'CACHE_FIXTURE GET /cache/stable.bin ' "$TEMOIN" || true)
somme1=$(propre | grep -aoE 'HOST_CACHE passage=1 res=stable.bin statut=200 taille=65536 somme=[0-9]+' | grep -oE '[0-9]+$' | head -1 || true)
somme2=$(propre | grep -aoE 'HOST_CACHE passage=2 res=stable.bin statut=200 taille=65536 somme=[0-9]+' | grep -oE '[0-9]+$' | head -1 || true)
rs=$(propre | grep -aoE 'PERF_EXECVE .*image=/usr/libexec/ladybird/RequestServer pid=[0-9]+' | grep -oE 'pid=[0-9]+$' | sort -u | wc -l || true)

echo "== verdict =="
exige "les deux navigateurs sont sortis" dans_invite 'CACHE_PASSAGE_2_SORTI'
exige "passage 1 : quitte proprement (code 0)" dans_invite 'BROWSER_HOST_EXIT boucle_quittee code=0'
exige "deux RequestServer distincts (rs=$rs)" test "$rs" -ge 2
exige "passage 1 : STORE stable.bin 65536 octets" dans_invite '\[LB\] STORE url=http://10\.0\.2\.2:'"$PORT"'/cache/stable\.bin bytes=65536'
exige "passage 2 : HIT stable.bin depuis le disque" dans_invite '\[LB\] HIT url=http://10\.0\.2\.2:'"$PORT"'/cache/stable\.bin bytes=65536'
exige "un seul GET de stable.bin sur deux passages (vu: $gets_stable)" test "$gets_stable" = 1
exige "meme contenu lu aux deux passages ($somme1 / $somme2)" test -n "$somme1" -a "$somme1" = "$somme2"
exige "inchange.txt revalide en 304" grep -q 'CACHE_FIXTURE GET /cache/inchange.txt inm="inchange-v1" .*statut=304' "$TEMOIN"
exige "change.txt revalide en 200 v2, lu en v2" dans_invite 'HOST_CACHE passage=2 res=change.txt statut=200 .*texte=change v2'
exige "SQLite : localStorage relu apres redemarrage" dans_invite 'HOST_SQL passage=2 REOPEN_OK'
exige "SQLite : cookie persistant renvoye au passage 2" grep -q 'CACHE_FIXTURE GET /cookie?pose=0 .*cookie=present' "$TEMOIN"
exige "aucune panique ni assertion" bash -c "! grep -aqE 'KERNEL PANIC|VERIFICATION FAILED' '$LOG'"

if [ ${#echecs[@]} -eq 0 ]; then
  echo "LADYBIRD_CACHE_REDEMARRAGE_OK"
else
  echo "LADYBIRD_CACHE_REDEMARRAGE_ECHEC n=${#echecs[@]}"
  exit 1
fi
