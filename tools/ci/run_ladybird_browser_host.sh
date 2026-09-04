#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_ladybird_browser_host.sh BOOTIMAGE NATIVE_DIR}
OUT=${2:?usage: run_ladybird_browser_host.sh BOOTIMAGE NATIVE_DIR}

for f in BouchaudBrowserHost WebContent RequestServer ImageDecoder WebWorker Compositor WebDriver; do
  test -f "$OUT/$f"
  file "$OUT/$f"
  ! readelf -l "$OUT/$f" | grep -q INTERP
done

rm -rf scenario-browser-host ladybird-browser-host.img serie-browser-host.log fixture-browser-host.log
python3 tools/health/browser_host_fixture.py > fixture-browser-host.log 2>&1 &
FIXTURE=$!
trap 'kill "$FIXTURE" 2>/dev/null || true' EXIT
sleep 1
kill -0 "$FIXTURE"

SCENARIO=scenario-browser-host
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
cat > "$SCENARIO/autorun" <<'AUTORUN'
uname
df
ifconfig
echo "=== Bouchaud BrowserHost interactif ==="
export BO_AUTOSTART_BROWSER=1
export BOUCHAUD_M9=1
export BOUCHAUD_BROWSER_HOST=1
export BOUCHAUD_M11=1
export BOUCHAUD_TIME_ZONE=Europe/Paris
export BOUCHAUD_M9_URL='http://10.0.2.2:18082/browser-host.html'
desktop
AUTORUN
(cd tools/userland && IMAGE="$PWD/../../ladybird-browser-host.img" ./mkdisk.sh "$PWD/../../$SCENARIO")

LOG=serie-browser-host.log
: > "$LOG"

# Les jalons du navigateur, dans l'ordre ou il les franchit.
#
# UNE seule liste. Elle servait deux fois -- trois markers cables dans la
# condition d'arret, dix dans la verification finale -- et les deux ne
# pouvaient pas etre lues ensemble. Elle sert maintenant aussi a DATER
# l'avancee, ce qui est le seul moyen de distinguer un invite lent d'un
# invite bloque.
JALONS=(
  '[ladybird-bouchaud] BROWSER_HOST_START'
  '[ladybird-bouchaud] BROWSER_HOST_INITIALIZED'
  '[ladybird-bouchaud] M11_GUI_HANDSHAKE_OK'
  '[ladybird-bouchaud] M11_DOCUMENT_LOADED'
  '[ladybird-bouchaud] BROWSER_HOST_M11_FRAME_PRESENTED'
  'HOST_CANVAS OK'
  'HOST_WORKER OK pong'
  'HOST_IMAGE OK 1x1'
  'HOST_IFRAME OK'
  'HOST_SMOKE_OK canvas=1 worker=1 image=1 frame=1'
)
# Ce qui suffit a conclure : le verdict de la page, plus les deux jalons cote
# Ladybird que le JavaScript n'implique pas.
#
# BOUCHAUD_SMOKE_VERDICT_RENDU_V1 : on attendait la ligne de SUCCES. Une page
# qui echoue ecrit `HOST_SMOKE_FAIL`, une reponse tout aussi definitive -- mais
# la boucle continuait de guetter une ligne qui ne viendrait jamais. Sur le run
# 33901806167 la page avait conclu a T+236 s ; le script a tourne jusqu'a
# 901 s, soit onze minutes a attendre une reponse deja donnee.
#
# La conclusion, c'est le PREFIXE. Ce que la page a repondu se lit ensuite dans
# le rapport, ou les jalons manquants deviennent alors de vrais echecs et non
# un manque de temps.
VERDICT='HOST_SMOKE_'
DOCUMENT='[ladybird-bouchaud] M11_DOCUMENT_LOADED'
TRAME='[ladybird-bouchaud] BROWSER_HOST_M11_FRAME_PRESENTED'

# BOUCHAUD_SMOKE_BUDGET_MESURE_V1
#
# Le plafond valait 300 s. Sur le run 33681042470, l'invite a mis 124,7 s
# pour atteindre HOST_CANVAS OK -- soit 42 % du budget consomme avant le
# premier des quatre sous-tests. Les trois suivants demandent, entre autres,
# de LANCER UN PROCESSUS de plus : WebWorker est un binaire Ladybird complet,
# et il est arrive a 7 fils et 48 Mio residents avant que le delai ne tombe.
# Le test ne mesurait donc pas si le navigateur fonctionne, mais s'il tient
# dans une enveloppe que personne n'avait rapportee a son cout reel.
#
# Le plafond est desormais large et ce n'est PAS lui qui decide : c'est le
# silence. Un invite qui progresse a le temps qu'il lui faut ; un invite
# bloque echoue vite, et le rapport dit ou. Le job dispose de 35 minutes et
# la construction du noyau en consomme deux.
PLAFOND=${BO_SMOKE_PLAFOND_S:-900}
SILENCE_MAX=${BO_SMOKE_SILENCE_S:-120}

# `-smp 4` : tous les autres lanceurs de CI en donnent quatre, et `run.ps1`
# aussi. Celui-ci etait le seul a n'en donner aucun, donc un. Le journal du
# run 33681042470 le montre sans ambiguite -- `load=[97]` a une seule case,
# `[SMP-PF] c1=0 c2=0 c3=0` -- : le navigateur saturait un coeur pendant que
# trois autres n'existaient pas. Un test qui fait tourner le produit dans une
# configuration que personne n'expedie ne mesure pas le produit.
qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file=ladybird-browser-host.img \
  -m 8192 -smp 4 -cpu max -display none -no-reboot \
  -netdev user,id=net0 -device e1000,netdev=net0 \
  -audiodev none,id=muet -device AC97,audiodev=muet \
  -serial file:"$LOG" &
PID=$!

declare -A VU=()
DEBUT=$SECONDS
taille_vue=0
derniere_avancee=$SECONDS
verdict=inconnu

echo "== avancee de l'invite (plafond ${PLAFOND}s, silence max ${SILENCE_MAX}s) =="
while kill -0 "$PID" 2>/dev/null; do
  for jalon in "${JALONS[@]}"; do
    if [ -z "${VU[$jalon]:-}" ] && grep -aFq "$jalon" "$LOG"; then
      VU[$jalon]=$((SECONDS - DEBUT))
      printf '  T+%-4ss %s\n' "${VU[$jalon]}" "$jalon"
    fi
  done

  if [ -n "${VU[$DOCUMENT]:-}" ] && [ -n "${VU[$TRAME]:-}" ] \
     && grep -aFq "$VERDICT" "$LOG"; then
    verdict=rendu
    break
  fi

  # Le noyau emet ses releves periodiques tant qu'il vit : un journal qui
  # cesse de grossir n'est pas un navigateur lent, c'est une machine morte.
  taille=$(wc -c < "$LOG")
  if [ "$taille" -ne "$taille_vue" ]; then
    taille_vue=$taille
    derniere_avancee=$SECONDS
  elif (( SECONDS - derniere_avancee >= SILENCE_MAX )); then
    verdict=muet
    break
  fi

  if (( SECONDS - DEBUT >= PLAFOND )); then
    verdict=plafond
    break
  fi
  sleep 2
done
ECOULE=$((SECONDS - DEBUT))
kill -TERM "$PID" 2>/dev/null || true
sleep 1
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
cat "$LOG"

# Relecture finale. La boucle s'arrete des que QEMU meurt, et les jalons
# ecrits juste avant sa mort n'ont alors jamais ete cherches. Sans cette
# passe, le rapport declarerait « JAMAIS ATTEINT » des lignes qui sont dans
# le journal -- il l'a fait, sur le banc d'essai de cette boucle.
for jalon in "${JALONS[@]}"; do
  if [ -z "${VU[$jalon]:-}" ] && grep -aFq "$jalon" "$LOG"; then
    VU[$jalon]=tardif
  fi
done

# La page ecrit ses echecs dans la meme forme que ses reussites : « X OK ... »
# devient « X FAIL <raison> ». Le rapport disait « JAMAIS ATTEINT » et taisait
# la raison, alors qu'elle etait dans le journal deux lignes plus haut -- c'est
# la difference entre « le worker n'a pas repondu » et « worker timeout ».
motif_echec() {
  case "$1" in
    HOST_SMOKE_OK*) printf 'HOST_SMOKE_FAIL' ;;
    *' OK '*|*' OK') printf '%s FAIL' "${1%% OK*}" ;;
    *) printf '' ;;
  esac
}

echo
echo "== jalons apres ${ECOULE}s (verdict: $verdict) =="
manquants=0
for jalon in "${JALONS[@]}"; do
  if [ "${VU[$jalon]:-}" = tardif ]; then
    printf '  atteint (dernier souffle) %s\n' "$jalon"
  elif [ -n "${VU[$jalon]:-}" ]; then
    printf '  atteint a T+%-4ss %s\n' "${VU[$jalon]}" "$jalon"
  else
    printf '  JAMAIS ATTEINT     %s\n' "$jalon"
    motif=$(motif_echec "$jalon")
    if [ -n "$motif" ]; then
      raison=$(grep -aF "$motif" "$LOG" | head -1 | tr -d '\r')
      [ -n "$raison" ] && printf '                     la page a dit : %s\n' "$raison"
    fi
    manquants=$((manquants + 1))
  fi
done

if [ "$manquants" -ne 0 ]; then
  case "$verdict" in
    muet)
      echo "l'invite a cesse d'ecrire sur la console serie pendant ${SILENCE_MAX}s :" >&2
      echo "la machine est bloquee, pas lente. Le dernier jalon atteint dit ou." >&2
      ;;
    plafond)
      echo "plafond de ${PLAFOND}s atteint alors que l'invite ecrivait encore :" >&2
      echo "le navigateur progressait trop lentement, il n'etait pas bloque." >&2
      ;;
    rendu)
      echo "la page a rendu son verdict en ${ECOULE}s : les jalons manquants" >&2
      echo "ci-dessus sont de vrais echecs, pas un manque de temps." >&2
      ;;
    *)
      echo "QEMU s'est arrete de lui-meme apres ${ECOULE}s." >&2
      ;;
  esac
  echo "$manquants jalon(s) manquant(s)." >&2
  exit 1
fi
grep -F "BROWSER_HOST_FIXTURE_OK path=/browser-host.html" fixture-browser-host.log
grep -F "BROWSER_HOST_FIXTURE_IMAGE_OK path=/pixel.png" fixture-browser-host.log
grep -F "BROWSER_HOST_FIXTURE_FRAME_OK path=/frame.html" fixture-browser-host.log

for forbidden in 'VERIFICATION FAILED:' IMAGE_DECODER_ABSENT M11_GUI_STREAM_DESYNC 'instruction illegale dans le programme utilisateur'; do
  if grep -aFq "$forbidden" "$LOG"; then
    echo "diagnostic interdit detecte: $forbidden" >&2
    exit 1
  fi
done
echo LADYBIRD_BROWSER_HOST_OK
