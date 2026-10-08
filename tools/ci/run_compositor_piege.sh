#!/usr/bin/env bash
# LE PIEGE DU COMPOSITOR, REJOUE (BOUCHAUD_COMPOSITOR_PIEGE_V1).
#
#   tools/ci/run_compositor_piege.sh BOOTIMAGE NATIVE_DIR [REPETITIONS]
#
# Run 37667817559 (e6799c24), banc « ordre worker », bras http : apres
# HOST_SMOKE_OK, le Compositor est mort sur `ud2` -- `ak_trap`, donc un
# `VERIFY` ou un `MUST()` d'AK qui echoue -- sans que la ligne
# `VERIFICATION FAILED` ne sorte. Une seule observation ne dit ni la
# frequence, ni le bras, ni le role du profil RIP.
#
# Chaque essai est un demarrage QEMU FROID complet du smoke BrowserHost, dans
# la configuration du banc d'origine (VM gardee vivante jusqu'au verdict A/B
# des workers). Pour chaque bras (http, blob), REPETITIONS essais, profil RIP
# alterne (essais impairs : actif ; pairs : coupe). Une ligne par essai :
#
#   PIEGE_ESSAI bras= essai= profil= smoke= faute_compositor= autres_fautes=
#
# puis, pour chaque faute du Compositor, sa symbolisation complete (RIP, pile
# publiee par le noyau, messages d'erreur et contexte -- symbolise_fautes.py).
#
# Diagnostic : rend 1 si le piege est reproduit (ou si un essai n'a pas pu
# conclure), 0 s'il ne l'est dans aucun essai. Ce n'est pas un gate de
# convergence ; c'est la mesure qui decide de la correction.
set -uo pipefail
cd "$(dirname "$0")/../.."

BOOT=${1:?usage: run_compositor_piege.sh BOOTIMAGE NATIVE_DIR [REPETITIONS]}
OUT=${2:?usage: run_compositor_piege.sh BOOTIMAGE NATIVE_DIR [REPETITIONS]}
N=${3:-4}

total=0
pieges=0
inconclus=0
resume=()
for essai in $(seq 1 "$N"); do
  for bras in http blob; do
    profil=on
    [ $((essai % 2)) -eq 0 ] && profil=off
    sortie="piege-$bras-$essai.sortie"
    BO_SMOKE_SUFFIXE="-piege-$bras-$essai" BO_SMOKE_ORDRE="$bras" BO_SMOKE_PROFIL_RIP="$profil" \
    BO_SMOKE_KEEP_GUEST_ALIVE=1 BO_SMOKE_ATTEND_AB=1 \
      tools/ci/run_ladybird_browser_host.sh "$BOOT" "$OUT" > "$sortie" 2>&1
    rc=$?
    journal="serie-browser-host-piege-$bras-$essai.log"
    total=$((total + 1))
    smoke=ECHEC
    grep -q "LADYBIRD_BROWSER_HOST_OK" "$sortie" && smoke=OK
    # Une faute du Compositor : la ligne symbolisee le nomme ; a defaut,
    # le PROCESS_FAULT d'un pid dont l'exec est le Compositor.
    faute=0
    grep -aq "FAUTE_SYMBOLE .*image=/usr/libexec/ladybird/Compositor" "$sortie" && faute=1
    autres=$(grep -ac "^FAUTE_SYMBOLE " "$sortie")
    autres=$((autres - faute))
    profil_vu=$(grep -aoE "PERF-RIP-RESUME\] t=[0-9]+ actif=[01]" "$journal" 2>/dev/null | tail -1 | grep -oE "actif=[01]")
    ligne="PIEGE_ESSAI bras=$bras essai=$essai profil=$profil ${profil_vu:-actif=?} smoke=$smoke rc=$rc faute_compositor=$faute autres_fautes=$autres"
    echo "$ligne"
    resume+=("$ligne")
    if [ "$faute" = 1 ]; then
      pieges=$((pieges + 1))
      # La section symbolisee du Compositor, bornee par awk (pas de `| head`
      # sous pipefail).
      awk '/^FAUTE_SYMBOLE .*Compositor/ { p = 1; n = 0 } /^FAUTE_SYMBOLE / && !/Compositor/ { p = 0 }
           p && n++ < 70 { print "    " $0 }' "$sortie"
    elif [ "$smoke" != OK ]; then
      inconclus=$((inconclus + 1))
      echo "    essai non conclusif (smoke=$smoke) ; fin de sortie :"
      tail -8 "$sortie" | sed 's/^/      /'
    fi
  done
done

echo
echo "== piege du Compositor : $pieges essai(s) sur $total =="
printf '%s\n' "${resume[@]}"
if [ "$pieges" -gt 0 ]; then
  echo "COMPOSITOR_PIEGE_REPRODUIT n=$pieges/$total inconclus=$inconclus"
  exit 1
fi
if [ "$inconclus" -gt 0 ]; then
  echo "COMPOSITOR_PIEGE_INCONCLUSIF inconclus=$inconclus/$total"
  exit 1
fi
echo "COMPOSITOR_PIEGE_ABSENT essais=$total"
