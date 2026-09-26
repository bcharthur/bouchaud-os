#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_ladybird_browser_host.sh BOOTIMAGE NATIVE_DIR}
OUT=${2:?usage: run_ladybird_browser_host.sh BOOTIMAGE NATIVE_DIR}

# DEUX VARIABLES ADDITIVES, POUR POUVOIR APPELER CE BANC DEUX FOIS.
#
# BOUCHAUD_C57_LA_PREMIERE_POSITION_OU_L_ORIGINE
#
# L'experience d'ordre demande DEUX demarrages QEMU froids sur le MEME
# binaire. Sans suffixe, le second ecraserait les journaux du premier ; sans
# `BO_SMOKE_ORDRE`, les deux chargeraient la meme page.
#
# Les deux sont vides par defaut : le smoke de reference ne change ni de nom
# de fichier, ni d'URL, ni de comportement.
SUFFIXE=${BO_SMOKE_SUFFIXE:-}

for f in BouchaudBrowserHost WebContent RequestServer ImageDecoder WebWorker Compositor WebDriver; do
  test -f "$OUT/$f"
  file "$OUT/$f"
  ! readelf -l "$OUT/$f" | grep -q INTERP
done

rm -rf scenario-browser-host ladybird-browser-host${SUFFIXE}.img \
       serie-browser-host${SUFFIXE}.log fixture-browser-host${SUFFIXE}.log
python3 tools/health/browser_host_fixture.py > fixture-browser-host${SUFFIXE}.log 2>&1 &
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

# LA DUREE DE VIE DE LA VM, RENDUE A L'HOTE -- POUR LE DIAGNOSTIC SEULEMENT.
#
# BOUCHAUD_C63_LE_RUNNER_PEUT_POSSEDER_LA_DUREE_DE_VIE
#
# Le noyau cherche ce fichier avant d'eteindre sur fin d'autorun. Vide par
# defaut : le smoke de reference garde exactement son comportement.
if [ "${BO_SMOKE_KEEP_GUEST_ALIVE:-0}" = "1" ]; then
    : > "$SCENARIO/garde-vm-vivante"
    echo "BO_SMOKE_GARDE_VM active=1"
fi
# L'URL EST CONSTRUITE SUR L'HOTE, PAS DANS L'INVITE.
#
# BOUCHAUD_C58_UN_HEREDOC_PROTEGE_N_EXPANSE_RIEN
#
# La premiere version ecrivait, DANS le heredoc a delimiteur protege :
#
#     export BOUCHAUD_M9_URL='...html${BO_SMOKE_ORDRE:+?ordre=$BO_SMOKE_ORDRE}'
#
# Deux erreurs superposees. Le delimiteur `<<'AUTORUN'` interdit toute
# expansion : la chaine partait telle quelle dans l'invite. Et les quotes
# simples auraient de toute facon empeche l'expansion cote hote.
#
# Resultat au run 35900151523 : les DEUX bras ont charge la page sans
# parametre, donc tous les deux en `ordre=blob`, et l'analyseur n'a vu qu'un
# seul bras. Le banc ne testait pas ce qu'il croyait tester.
#
# L'URL est donc assemblee ici, ou les variables existent, puis injectee
# COMME VALEUR dans le script de l'invite.
URL_PAGE='http://10.0.2.2:18082/browser-host.html'
if [ -n "${BO_SMOKE_ORDRE:-}" ]; then
    URL_PAGE="${URL_PAGE}?ordre=${BO_SMOKE_ORDRE}"
fi
# Publiee : un banc doit pouvoir prouver la configuration qu'il pense tester.
echo "BO_SMOKE_URL ordre=${BO_SMOKE_ORDRE:-defaut} url=$URL_PAGE"

cat > "$SCENARIO/autorun" <<AUTORUN
uname
df
ifconfig
echo "=== Bouchaud BrowserHost interactif ==="
export BO_AUTOSTART_BROWSER=1
export BOUCHAUD_M9=1
export BOUCHAUD_BROWSER_HOST=1
export BOUCHAUD_M11=1
export BOUCHAUD_TIME_ZONE=Europe/Paris
export BOUCHAUD_M9_URL='$URL_PAGE'
echo "AUTORUN_DESKTOP_ENTER"
desktop
echo "AUTORUN_DESKTOP_RETURN statut=\$?"
AUTORUN
(cd tools/userland && IMAGE="$PWD/../../ladybird-browser-host${SUFFIXE}.img" ./mkdisk.sh "$PWD/../../$SCENARIO")

LOG=serie-browser-host${SUFFIXE}.log
: > "$LOG"

# LE MONITEUR QEMU, POUR REGARDER L'ECRAN.
#
# BOUCHAUD_C29_SURFACE_PRESENTEE
#
# Le banc de page lit ses pixels dans un canvas : cela prouve que le
# decodeur rend les bons octets, pas qu'ils arrivent a l'ecran. Le defaut
# observe sur la machine physique est justement une image qui se decode et
# reste BLANCHE dans la frame presentee.
#
# Aucune interface de page ne permet de relire la surface composee. On
# regarde donc l'ecran de la machine, par `screendump`.
MONITEUR=$PWD/moniteur-browser-host${SUFFIXE}.sock
CAPTURE=$PWD/surface-browser-host${SUFFIXE}.ppm
rm -f "$MONITEUR" "$CAPTURE"

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
  'HOST_IMAGE OK 1x1'
  'HOST_IFRAME OK'
  # LES DEUX BANCS QUI VERIFIENT UN COMPORTEMENT, ET NON UNE CAPACITE.
  #
  # `HOST_IMAGE OK 1x1` ci-dessus prouve qu'un PNG d'un pixel se decode. Ce
  # n'est pas « les images fonctionnent » : le defaut observe sur la machine
  # physique est une image qui arrive en HTTP 200, que l'ImageDecoder traite,
  # et qui reste BLANCHE. Il faut relire les PIXELS pour le voir, sur les
  # quatre codecs, et c'est ce que `HOST_IMAGES_OK` affirme.
  #
  # `HOST_CANVAS OK` prouve qu'un peu de JS tourne. `HOST_JS_OK` exige que
  # dix-sept comportements soient reellement EXECUTES -- boucle d'evenements,
  # micro-taches, minuteries, fetch, DOM, cadres.
  'HOST_IMAGES_OK codecs=11/11 fond=1 echelle=1 reutilise=1'
  'HOST_JS_OK'
  # LES TROIS VERDICTS DE WORKER SONT SEPARES, ET LE GLOBAL EST UNE
  # CONJONCTION.
  #
  # `HOST_WORKER OK pong` etait satisfait par `http || blob`. La CI est donc
  # passee au vert avec le chargement de script par le RESEAU casse :
  #
  #     HOST_WORKER_BLOB OK pong
  #     HOST_WORKER_HTTP FAIL
  #     HOST_WORKER      OK pong      <- faux vert
  #
  # Un jalon qui peut etre vert alors qu'une moitie de la fonction ne marche
  # pas ne mesure pas cette fonction.
  #
  # BOUCHAUD_C30_CAPACITE_ET_PERFORMANCE : la CI exige la CAPACITE. La
  # performance a ses propres lignes (`_PERF`), lues par le rapport mais pas
  # par les jalons -- un budget de performance ne doit pas eteindre une
  # capacite qui marche, sans quoi « le worker est casse » et « le worker est
  # lent » deviennent le meme echec.
  'HOST_WORKER_HTTP_FUNCTIONAL OK'
  'HOST_WORKER_BLOB_FUNCTIONAL OK'
  'HOST_WORKER_FUNCTIONAL_GLOBAL OK pong'
  # LA MIRE N'EST PAS UN JALON SERIE, ET L'Y METTRE LA RENDAIT INATTEIGNABLE.
  #
  # `HOST_SURFACE_VERDICT` est ecrit par CE script, sur sa propre sortie, apres
  # le `screendump`. Les jalons, eux, sont cherches par `grep` dans
  # `serie-browser-host.log`. La ligne n'y a jamais figure et n'y figurera
  # jamais : le jalon comptait donc comme manquant a CHAQUE run -- un faux
  # rouge permanent, qui plus est sur le point que ce chantier devait fermer.
  #
  # Le verdict de surface est teste directement, plus bas, sur `MIRE_VERDICT`.
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
# Secondes d'attente d'une trame composee APRES l'insertion de la mire.
TRAME_ATTENTE_MAX=${BO_SMOKE_TRAME_S:-45}
# Le verdict de performance echoue-t-il le script ? Faux par defaut : il a
# son propre travail de CI, et une machine lente n'est pas une machine
# cassee. Dans ce travail-la il vaut 1, et il bloque.
PERF_BLOQUANT=${BO_SMOKE_PERF_BLOQUANT:-0}
SILENCE_MAX=${BO_SMOKE_SILENCE_S:-120}

# `-smp 4` : tous les autres lanceurs de CI en donnent quatre, et `run.ps1`
# aussi. Celui-ci etait le seul a n'en donner aucun, donc un. Le journal du
# run 33681042470 le montre sans ambiguite -- `load=[97]` a une seule case,
# `[SMP-PF] c1=0 c2=0 c3=0` -- : le navigateur saturait un coeur pendant que
# trois autres n'existaient pas. Un test qui fait tourner le produit dans une
# configuration que personne n'expedie ne mesure pas le produit.
qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file=ladybird-browser-host${SUFFIXE}.img \
  -m 8192 -smp 4 -cpu max -display none -no-reboot \
  -netdev user,id=net0 -device e1000,netdev=net0 \
  -audiodev none,id=muet -device AC97,audiodev=muet \
  -monitor "unix:$MONITEUR,server,nowait" \
  -serial file:"$LOG" &
PID=$!

# ============================================================================
# BOUCHAUD_C41_CAPTURE_VIVANTE -- la preuve se prend pendant que QEMU vit
# ============================================================================
#
# Le run #347 a pose sa mire vers 211 s et QEMU a vecu jusqu'a 342 s. Cent
# trente secondes pendant lesquelles la capture etait possible -- et le banc ne
# la tentait qu'APRES sa boucle, donc apres la mort de la machine. Le verdict
# rendu etait `moniteur_muet` : une panne du banc presentee comme une panne du
# produit.
#
# La decision « est-ce le moment ? » vit dans `surface_declencheur.py`, ou elle
# est mise en echec sur des journaux synthetiques -- notamment celui ou la mire
# est posee, deux trames suivent, puis la machine meurt.

maintenant_ms() { date +%s%3N; }

MIRE_VERDICT=en_attente
SEQ_INSERTION=""
SEQ_VOULUE=""
T_INSERTION_MS=0

# Prend la capture et rend un verdict DEFINITIF. N'est appelee qu'une fois.
capture_mire() {
    local seq_vue=$1
    local t_capture_ms
    t_capture_ms=$(maintenant_ms)
    local delta_ms=$((t_capture_ms - T_INSERTION_MS))

    if ! echo "screendump $CAPTURE" | socat - "unix-connect:$MONITEUR" >/dev/null 2>&1; then
        MIRE_VERDICT=moniteur_muet
        echo "HOST_SURFACE_CAPTURE t=$t_capture_ms seq=$seq_vue delta_ms=$delta_ms etat=moniteur_muet"
        return
    fi

    # `screendump` rend la main avant que le fichier soit entierement ecrit. On
    # attend que sa TAILLE CESSE DE BOUGER, ce qui est une observation ; un
    # sommeil fixe est un pari.
    local taille taille_precedente=-1
    for _ in $(seq 1 15); do
        taille=$(wc -c < "$CAPTURE" 2>/dev/null || echo 0)
        if [ "$taille" -gt 0 ] && [ "$taille" -eq "$taille_precedente" ]; then
            break
        fi
        taille_precedente=$taille
        sleep 1
    done

    if [ ! -s "$CAPTURE" ]; then
        MIRE_VERDICT=capture_vide
    elif python3 tools/ci/analyse-surface-mire.py "$CAPTURE"; then
        MIRE_VERDICT=ok
    else
        MIRE_VERDICT=absente
    fi
    echo "HOST_SURFACE_CAPTURE t=$t_capture_ms seq=$seq_vue delta_ms=$delta_ms etat=$MIRE_VERDICT"
}

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

  # LA CAPTURE, PENDANT QUE LA MACHINE VIT.
  #
  # Deux etats successifs, chacun franchi une seule fois : on fige la cible a
  # l'insertion, puis on capture des que la trame voulue est composee. La
  # verification `en_attente` garantit qu'une preuve acquise ne peut plus etre
  # effacee -- ni par la mort de QEMU, ni par un tour de boucle suivant.
  if [ "$MIRE_VERDICT" = "en_attente" ] && command -v socat >/dev/null 2>&1; then
    ETAT_MIRE=$(python3 tools/ci/surface_declencheur.py "$LOG" "${SEQ_INSERTION:--}" 2>/dev/null || true)
    if [ -n "$ETAT_MIRE" ]; then
      eval "$ETAT_MIRE"
      if [ "$inseree" = "1" ] && [ -z "$SEQ_INSERTION" ]; then
        SEQ_INSERTION=$seq_insertion
        SEQ_VOULUE=$seq_voulue
        T_INSERTION_MS=$(maintenant_ms)
        echo "HOST_SURFACE_INSERTION t=$T_INSERTION_MS seq=$SEQ_INSERTION voulue=$SEQ_VOULUE"
      fi
      if [ "$inseree" = "1" ] && [ "$pret" = "1" ]; then
        capture_mire "$seq_vue"
      fi
    fi
  fi

  # LE VERDICT TERMINAL DE L'EXPERIENCE D'ORDRE, QUAND ON L'ATTEND.
  #
  # BOUCHAUD_C62_ATTENDRE_UN_VERDICT_PAS_UNE_FIN_D_AUTORUN
  #
  # La page dit elle-meme quand ses quatre mesures sont faites. Attendre
  # `AUTORUN FIN` ne disait rien d'elle : au run 35907201865 l'autorun s'est
  # termine alors que le premier worker n'avait pas franchi son constructeur.
  if [ "${BO_SMOKE_ATTEND_AB:-0}" = "1" ]; then
    if grep -aFq "HOST_WORKER_AB_COMPLETE" "$LOG"; then
      verdict=ab_complete
      break
    fi
    if grep -aFq "HOST_WORKER_AB_FAIL" "$LOG"; then
      verdict=ab_echec
      break
    fi
  fi

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

# UNE PREUVE ACQUISE NE SE PERD PLUS.
#
# Tout ce qui precede s'est joue PENDANT que QEMU vivait. Ce bloc ne fait que
# NOMMER ce qui n'a pas pu l'etre : il ne retente rien contre une machine
# morte, et surtout il n'ecrase jamais un verdict deja rendu. L'ancienne
# version recalculait tout ici, apres la boucle -- c'est-a-dire toujours trop
# tard, et elle transformait une capture reussie en `moniteur_muet`.
if [ "$MIRE_VERDICT" = "en_attente" ]; then
    if ! command -v socat >/dev/null 2>&1; then
        MIRE_VERDICT=socat_absent
    elif [ -z "$SEQ_INSERTION" ]; then
        # La page n'a jamais dit avoir pose sa mire : capturer n'aurait rien
        # prouve, et accuser le compositeur d'un retard de la page serait faux.
        MIRE_VERDICT=non_posee
    else
        # La mire est posee mais aucune trame posterieure n'est venue : le
        # compositeur n'a rien recompose depuis. C'est un resultat, pas un
        # incident du banc -- et il se distingue maintenant des deux autres.
        MIRE_VERDICT=sans_trame_posterieure
    fi
fi
echo "HOST_SURFACE_TRAME insertion=${SEQ_INSERTION:--} voulue=${SEQ_VOULUE:--}"
echo "HOST_SURFACE_VERDICT $MIRE_VERDICT"

# POURQUOI LA SESSION S'EST-ELLE ARRETEE ?
#
# BOUCHAUD_C40_AUCUN_ARRET_SILENCIEUX
#
# Le run #347 s'est arrete vers 342 s, en pleine matrice de workers, et le
# journal ne le disait pas. Tous les chemins d'arret volontaire -- fin
# d'autorun, menu Quitter, commande shell, banc d'entree/sortie -- ainsi que
# le gestionnaire de panique emettent desormais la meme ligne.
#
# Cette lecture a lieu AVANT que le banc ne tue QEMU : apres, on ne saurait
# plus distinguer « la session s'est arretee toute seule » de « c'est nous qui
# l'avons tuee », ce qui est justement la question.
if kill -0 "$PID" 2>/dev/null; then
    echo "BOUCHAUD_SESSION_FIN etat=vivante raison=tuee_par_le_banc"
else
    # Meme piege que plus bas, et il a survecu a sa propre correction : en
    # expression reguliere, `\r` dans une classe vaut « ni backslash ni r ».
    # La ligne etait coupee au premier `r`, donc juste apres le nom du
    # marqueur, et le run 35907201865 n'a affiche que :
    #
    #     BOUCHAUD_SESSION_FIN etat=arretee BOUCHAUD_SYSTEM_EXIT
    #
    # La raison -- `autorun_termine` -- etait dans le journal et invisible ici.
    FIN=$(grep -ao 'BOUCHAUD_SYSTEM_EXIT .*' "$LOG" | tr -d '\r' | tail -1 || true)
    if [ -n "$FIN" ]; then
        echo "BOUCHAUD_SESSION_FIN etat=arretee $FIN"
    else
        # Un arret sans marqueur n'est PAS une information manquante : c'est un
        # chemin d'arret qu'on ne connait pas. Triple faute, reinitialisation,
        # QEMU tue de l'exterieur, coupure d'alimentation emulee.
        echo "BOUCHAUD_SESSION_FIN etat=arretee raison=SANS_MARQUEUR"
        echo "  QEMU est mort sans qu'aucun chemin connu ne l'annonce." >&2
        echo "  Dernieres lignes du journal serie :" >&2
        tail -c 2000 "$LOG" | sed -E 's/\x1b\[[0-9;]*m//g' | tail -12 | sed 's/^/    /' >&2
    fi
fi

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
      # `|| true` sur les DEUX : le script tourne sous `set -e` avec
      # `pipefail`. `grep` rend 1 quand il ne trouve rien, et un
      # `[ ... ] && printf` rend 1 quand le test est faux. L'un ou l'autre
      # tuait le rapport en plein milieu -- il s'est arrete sur
      # « JAMAIS ATTEINT HOST_IFRAME OK », emportant la ligne de verdict et
      # le compte des jalons manquants, c'est-a-dire ce qui explique
      # l'echec. Un rapport de diagnostic ne doit jamais pouvoir se taire
      # parce qu'il n'a rien trouve a dire sur une ligne.
      raison=$(grep -aF "$motif" "$LOG" | head -1 | sed 's/\r//g; s/\x1b\[[0-9;]*m//g') || true
      if [ -n "$raison" ]; then
        printf '                     la page a dit : %s\n' "$raison"
      fi
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
      # LA SESSION S'EST-ELLE ARRETEE, ET POURQUOI ?
      #
      # Le run 35830736815 s'est eteint a T+342 s en plein milieu de la
      # matrice de workers, sans un mot. Le bureau dit desormais lequel de ses
      # deux chemins de sortie a ete pris ; le rapport le repete ici, ou on
      # le lit.
      raison_bureau=$(grep -aF "BOUCHAUD_BUREAU_FIN" "$LOG" | head -1 | sed 's/\r//g; s/\x1b\[[0-9;]*m//g') || true
      if [ -n "$raison_bureau" ]; then
        echo "  le bureau a rendu la main : $raison_bureau" >&2
      else
        echo "  aucune ligne BOUCHAUD_BUREAU_FIN : la session n'est pas sortie par le bureau" >&2
      fi
      ;;
  esac
  echo "$manquants jalon(s) manquant(s)." >&2
  echo "LADYBIRD_FUNCTIONAL_SMOKE fail raison=jalons manquants=$manquants"
  exit 1
fi
grep -F "BROWSER_HOST_FIXTURE_OK path=/browser-host.html" fixture-browser-host${SUFFIXE}.log
grep -F "BROWSER_HOST_FIXTURE_IMAGE_OK path=/pixel.png" fixture-browser-host${SUFFIXE}.log
grep -F "BROWSER_HOST_FIXTURE_FRAME_OK path=/frame.html" fixture-browser-host${SUFFIXE}.log

for forbidden in 'VERIFICATION FAILED:' IMAGE_DECODER_ABSENT M11_GUI_STREAM_DESYNC 'instruction illegale dans le programme utilisateur'; do
  if grep -aFq "$forbidden" "$LOG"; then
    echo "diagnostic interdit detecte: $forbidden" >&2
    echo "LADYBIRD_FUNCTIONAL_SMOKE fail raison=diagnostic_interdit"
    exit 1
  fi
done
# ====================================================================
# LE VERDICT DE SURFACE, TESTE POUR LUI-MEME
#
# Il etait dans `JALONS`, donc cherche dans le journal serie, ou il n'a
# jamais pu figurer : ce script l'ecrit sur SA sortie. Le jalon comptait
# manquant a chaque run. Il se teste ici, sur la variable qui le porte.
# ====================================================================
if [ "$MIRE_VERDICT" != ok ]; then
  echo "surface : la mire n'a pas ete retrouvee dans la frame presentee ($MIRE_VERDICT)" >&2
  case "$MIRE_VERDICT" in
    socat_absent)   echo "  socat manque : le moniteur QEMU est injoignable, la mesure n'a pas eu lieu" >&2 ;;
    non_posee)      echo "  la page n'a jamais dit avoir insere la mire" >&2 ;;
    sans_trame_posterieure)
                    echo "  aucune trame n'a ete composee apres l'insertion : rien a capturer" >&2 ;;
    moniteur_muet)  echo "  le moniteur n'a pas repondu a screendump" >&2 ;;
    capture_vide)   echo "  screendump a rendu un fichier vide" >&2 ;;
    absente)        echo "  la capture existe mais ne contient pas les pixels attendus" >&2 ;;
    en_attente)     echo "  BANC : le verdict n'a jamais ete rendu -- ni capture, ni raison." >&2
                    echo "         C'est un defaut du banc, pas du navigateur." >&2 ;;
    *)              echo "  verdict inconnu du banc : $MIRE_VERDICT" >&2 ;;
  esac
  echo "FONCTIONNEL" >&2
  echo "LADYBIRD_FUNCTIONAL_SMOKE fail raison=surface"
  exit 1
fi

echo "LADYBIRD_FUNCTIONAL_SMOKE ok"

# ====================================================================
# LA CHRONOLOGIE DU DEMARRAGE A FROID
#
# Les mesures existaient, eparpillees dans trois vocabulaires -- jalons de
# l'hote, `WORKER_ETAPE`, `PERF_FORK`/`PERF_EXECVE`/`[PERF-PROC]` du noyau.
# On pouvait lire chacune sans rien conclure : le seul chiffre qui interesse
# est une soustraction entre deux d'entre elles.
#
# Le rapport ne juge pas -- pas de budget, pas de verdict. Il rassemble.
# ====================================================================
echo
echo "== chronologie du demarrage =="
python3 tools/ci/analyse-demarrage.py "$LOG" || true

# ====================================================================
# LE VERDICT DE PERFORMANCE, INDEPENDANT DU FONCTIONNEL
#
# BOUCHAUD_C32_DEUX_VERDICTS
#
# Separer capacite et performance etait juste, mais incomplet : les lignes
# `_PERF` n'etaient lues par PERSONNE. Un verdict que rien ne consulte est
# decoratif, et une regression de performance repassait inapercue.
#
# Le resultat est publie a part et, dans le travail de CI qui pose
# `BO_SMOKE_PERF_BLOQUANT=1`, il bloque. Ailleurs il informe : une machine
# lente n'est pas une machine cassee, mais elle doit se voir.
# ====================================================================
# La logique vit dans `verifie_ladybird_perf.sh`, qui est le verdict autoritaire
# et porte son PROPRE statut de CI. Ici elle n'informe que le lecteur du banc :
# `BO_SMOKE_PERF_BLOQUANT=1` reste honore pour les appelants qui s'en servent,
# mais la CI ne depend plus de cette variable pour voir une regression.
if tools/ci/verifie_ladybird_perf.sh "$LOG"; then
  :
elif [ "$PERF_BLOQUANT" = 1 ]; then
  exit 1
else
  echo "  (verdict de performance non bloquant ici ; voir le travail de CI dedie)"
fi

# ====================================================================
# LES PREUVES, EN FIN DE SORTIE
#
# BOUCHAUD_C53_LISIBLE_DANS_LA_CI
#
# Un journal serie de cinq mille lignes est un artefact, pas un rapport. Les
# outils qui lisent la CI n'en voient que la fin -- et c'est precisement la que
# les lignes qui decident manquaient. Les republier ici coute trois `grep` et
# evite de telecharger l'artefact pour repondre a « la mire a-t-elle ete
# capturee » ou « pourquoi la session s'est-elle arretee ».
# ====================================================================
echo
echo "== preuves du demarrage a froid =="
# `[^\r]*` ETAIT UN PIEGE, ET IL A COUTE UN RUN.
#
# Dans une expression reguliere etendue, `\r` a l'interieur d'une classe ne
# designe PAS un retour chariot : il vaut « ni backslash ni r ». Le motif
# coupait donc chaque ligne au premier `r` rencontre, et le bloc de preuves du
# run 35888970521 a rendu :
#
#     BACKING_PROBE path=/bo-navigateu
#     FAULT_FILE_BREAKDOWN t=353624 pid=15 sou
#     FAULT_WAIT count=0 total_us=0 wo
#
# Les controles `source=` et `faults=` etaient impossibles sur ce journal. Le
# meme piege avait deja ete corrige dans le balayage de tranche ; il restait
# ici. `.*` suffit, et `tr -d` retire les retours chariot pour de vrai.
for motif in \
    'BACKING_PROBE .*' \
    'HOST_SURFACE_INSERTION .*' \
    'HOST_SURFACE_CAPTURE .*' \
    'BOUCHAUD_SESSION_FIN .*' \
    'BOUCHAUD_SYSTEM_EXIT .*' \
    'PERF_EXECVE_BKL .*' \
    'FAULT_FILE_BREAKDOWN .*' \
    'FAULT_FILE_SNAPSHOT .*' \
    'CPU_CUMUL .*' \
    'SYSCALL_TEMPS .*' \
    'CACHE_BALAYAGE .*' \
    'FAULT_REPRISE .*' \
    'BACKING_DISK_GLOBAL .*' \
    'BACKING_MEMORY_GLOBAL .*' \
    'CLEAN_PAGE_CACHE_GLOBAL .*' \
    'FAULT_WAIT .*'
do
    grep -aoE "$motif" "$LOG" 2>/dev/null | tr -d '\r' | awk '!vu[$0]++' | tail -10 \
        | sed 's/^/  /' || true
done
echo "  (une famille absente ci-dessus n'a pas ete emise par ce noyau)"

echo
echo "== les six services, cote a cote =="
python3 tools/ci/analyse-demarrage.py "$LOG" 2>/dev/null \
    | sed -n '/six services/,/^$/p' | head -12 || true

# BOUCHAUD_P13_VERDICT_CONVERGENCE
python3 tools/ci/ladybird_runtime_verdict.py "$LOG" \
    --surface "$MIRE_VERDICT" \
    --json-out "convergence-browser-host${SUFFIXE}.json"
echo LADYBIRD_BROWSER_HOST_OK
