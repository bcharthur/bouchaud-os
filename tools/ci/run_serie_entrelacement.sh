#!/usr/bin/env bash
# LIGNES ENTIERES SUR LA CONSOLE SERIE (BOUCHAUD_SERIE_ECRITURE_ATOMIQUE_V1).
#
#   tools/ci/run_serie_entrelacement.sh BOOTIMAGE [ECRIVAINS LIGNES PROCESSUS CHARGE PAS_MS]
#
# serie-entrelacement-probe : des ecrivains emettent des lignes longues et
# verifiables pendant qu'un brasseur fait naitre et mourir des processus (le
# noyau ecrit alors depuis d'autres coeurs). Toutes les lignes doivent sortir
# intactes, et le bilan serie de fin d'autorun doit dire 0 emission a la borne
# (une ligne emise sans le jeton) et 0 reentrance.
#
# Run 38482972584 (KVM) : `pid=1\x1b6[ 90kmi[l...`, une ligne du navigateur
# melangee octet contre octet au prefixe d'une ligne du noyau.
#
# BO_SMP : coeurs (4 par defaut). BO_QEMU_KVM=1 : sous KVM. BO_QEMU_EXTRA :
# options QEMU de diagnostic (ex. `-d cpu_reset -D reset.log`).
#
# BOUCHAUD_SERIE_JAMAIS_SANS_JETON_V1 -- BANC_SUSPENSION_MS=N : un fil noyau
# prend le jeton 1,5 s apres le depart et le garde N ms interruptions masquees,
# au milieu d'une ligne (un processeur virtuel retire par l'hote). Exige : sa
# ligne entiere, aucune ligne sans jeton, des lignes differees puis videes.
# Au-dela de ce que les anneaux tiennent (BANC_PERDUES_ADMISES=1), une ligne
# peut manquer -- comptee perdue par le noyau --, jamais etre coupee.
#
# BOUCHAUD_CONSOLE_VERROU_BORNE_V1 -- BANC_VERROU_SUSPENDU_MS=N : un fil noyau
# prend le VERROU DE L'ANNEAU de la console asynchrone 1,5 s apres le depart
# et le garde N ms interruptions masquees. Exige : la suspension a ete subie
# (des ecrivains ont attendu en cedant, interruptions ouvertes, ou renonce a la
# borne), aucune attente INTERRUPTIONS MASQUEES au-dela de 50 ms, aucune ligne
# perdue, toutes entieres, la console toujours asynchrone.
#
# BOUCHAUD_CONSOLE_FIL_SECOURS_V1 -- BANC_VERROU_CIBLE=fil : la suspension
# frappe le coeur dans la file duquel le fil `console` attend (le cas du run
# 39667691144, KVM SMP4 : fil endormi 3 s sur un « sommeil » d'une ms, repli
# a 2 s). Exige en plus : le fil a ete reloge (`secours>=1`). BANC_SECOURS=off
# coupe le relogement (bras temoin : le repli DOIT revenir).
# BANC_FIL_GELE_MS=N : la vraie panne -- le fil cesse de progresser N ms
# (0,5 s apres le depart). Exige l'INVERSE : repli de securite, explique
# (fil en phase 5), dans la borne (2 s de silence, 0,5 s de confirmation),
# et toutes les lignes entieres par le chemin synchrone. A lancer avec des
# producteurs LENTS (PAS_MS) et lignes courtes : un anneau plein est vide par
# ses producteurs eux-memes (`attend_place`), la console progresse alors sans
# le fil -- ce n'est pas une panne, et le chien de garde a raison de ne pas
# replier (banc sature, fil gele 6 s : aucun repli, 600/600).
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_serie_entrelacement.sh BOOTIMAGE [ECRIVAINS LIGNES PROCESSUS CHARGE PAS_MS]}
ECRIVAINS=${2:-3}
LIGNES=${3:-200}
PROCESSUS=${4:-120}
CHARGE=${5:-160}
PAS_MS=${6:-0}
SMP=${BO_SMP:-4}
ACCEL="-cpu max"
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
fi

SCENARIO=scenario-serie-entrelacement
IMAGE=serie-entrelacement.img
LOG=serie-entrelacement.log
rm -rf "$SCENARIO" "$IMAGE" "$LOG" tools/userland/out-serie-entrelacement
(cd tools/userland && OUT=out-serie-entrelacement ./build.sh musl >/dev/null)
mkdir -p "$SCENARIO/bin"
cp tools/userland/out-serie-entrelacement/serie-entrelacement-probe "$SCENARIO/bin/"
SUSPENSION=${BANC_SUSPENSION_MS:-0}
VERROU_SUSPENDU=${BANC_VERROU_SUSPENDU_MS:-0}
VERROU_CIBLE=${BANC_VERROU_CIBLE:-}
SECOURS=${BANC_SECOURS:-on}
FIL_GELE=${BANC_FIL_GELE_MS:-0}
# P13_CONSOLE_DIFFEREE_CONTRE_PRESSION_V1 -- BANC_RAFALE_MASQUEE=N (avec
# BANC_VERROU_SUSPENDU_MS) : N lignes du noyau, interruptions masquees,
# emises PENDANT la suspension du verrou de l'anneau. Chacune doit sortir
# entiere, dans l'ordre, et aucune ligne differee ne doit etre perdue.
RAFALE=${BANC_RAFALE_MASQUEE:-0}
# BOUCHAUD_SERIE_LENTE_BANC_V1 -- BANC_SERIE_LENTE_NS=N : N ns de plus par
# octet sur COM1, le cout du port sous KVM reproduit sous TCG (flot sature).
SERIE_LENTE=${BANC_SERIE_LENTE_NS:-0}
# BANC_CONSOLE=off : bras temoin, console synchrone (le chemin d'avant).
MODE_CONSOLE=${BANC_CONSOLE:-on}
{
  if [ "$MODE_CONSOLE" = off ]; then echo "console-async off"; fi
  if [ "$SERIE_LENTE" -gt 0 ]; then echo "serie-lente $SERIE_LENTE"; fi
  if [ "$SUSPENSION" -gt 0 ]; then echo "serie-detenteur-suspendu 1500 $SUSPENSION"; fi
  if [ "$SECOURS" = off ]; then echo "console-async secours off"; fi
  if [ "$FIL_GELE" -gt 0 ]; then echo "console-async gele-fil 500 $FIL_GELE"; fi
  if [ "$VERROU_SUSPENDU" -gt 0 ]; then echo "serie-verrou-suspendu 1500 $VERROU_SUSPENDU $VERROU_CIBLE"; fi
  if [ "$RAFALE" -gt 0 ]; then echo "serie-rafale-masquee $RAFALE"; fi
  printf '/bin/serie-entrelacement-probe %s %s %s %s %s\n' \
    "$ECRIVAINS" "$LIGNES" "$PROCESSUS" "$CHARGE" "$PAS_MS"
  # Strict join: only emit final autorun snapshot after guest injection succeeds.
  if [ "$SUSPENSION" -gt 0 ]; then echo 'serie-banc-attend detenteur'; fi
  if [ "$VERROU_SUSPENDU" -gt 0 ]; then echo 'serie-banc-attend verrou'; fi
  if [ "$RAFALE" -gt 0 ]; then echo 'serie-banc-attend rafale'; fi
  if [ "$SUSPENSION" -gt 0 ] || [ "$VERROU_SUSPENDU" -gt 0 ]; then echo 'console-async etat'; fi
  echo 'echo SERIE_ENTRELACEMENT_AUTORUN_FIN'
} > "$SCENARIO/autorun"
(cd tools/userland && IMAGE="$PWD/../../$IMAGE" ./mkdisk.sh "$PWD/../../$SCENARIO" >/dev/null)

echo "SERIE_ENTRELACEMENT smp=${SMP} accel=${ACCEL} ecrivains=${ECRIVAINS} lignes=${LIGNES} processus=${PROCESSUS} charge=${CHARGE} pas_ms=${PAS_MS} suspension_ms=${SUSPENSION} verrou_suspendu_ms=${VERROU_SUSPENDU} serie_lente_ns=${SERIE_LENTE} console=${MODE_CONSOLE} verrou_cible=${VERROU_CIBLE:-aucune} secours=${SECOURS} fil_gele_ms=${FIL_GELE}"
: > "$LOG"
# shellcheck disable=SC2086
qemu-system-x86_64 $ACCEL \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$IMAGE" \
  -m 2048 -smp "$SMP" -display none -no-reboot ${BO_QEMU_EXTRA:-} \
  -serial file:"$LOG" &
PID=$!
LIMITE=$((SECONDS + 600))
while kill -0 "$PID" 2>/dev/null; do
  if (( SECONDS >= LIMITE )); then echo "ECHEANCE atteinte" >&2; break; fi
  grep -aqE 'KERNEL PANIC|panicked at' "$LOG" && { sleep 2; break; }
  grep -aq 'SERIE_BANC_ATTEND_ECHEC' "$LOG" && { echo 'INJECTION_CONSOLE_ECHEC' >&2; sleep 2; break; }
  # Le bilan de FIN d'autorun : `console-async off` en imprime un aussi, au
  # depart (bras temoin) -- s'arreter dessus coupait la sonde a 26 s.
  if awk '/SERIE_ENTRELACEMENT_AUTORUN_FIN/ { fin = 1 } fin && /BOUCHAUD_CONSOLE_BILAN/ { vu = 1 } END { exit !vu }' "$LOG"; then
    if [ "$SUSPENSION" -gt 0 ] && ! grep -aq 'SERIE_BANC_ATTEND_OK type=detenteur' "$LOG"; then sleep 1; continue; fi
    if [ "$VERROU_SUSPENDU" -gt 0 ] && ! grep -aq 'SERIE_BANC_ATTEND_OK type=verrou' "$LOG"; then sleep 1; continue; fi
    if [ "$RAFALE" -gt 0 ] && ! grep -aqE 'SERIE_BANC_ATTEND_(OK|ECHEC) type=rafale' "$LOG"; then sleep 1; continue; fi
    sleep 2
    break
  fi
  sleep 1
done
kill -TERM "$PID" 2>/dev/null || true
sleep 1
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true

echecs=0
exige() { local quoi=$1; shift; if "$@"; then echo "  ok      $quoi"; else echo "  ECHEC   $quoi"; echecs=$((echecs + 1)); fi; }
exige "aucune panique noyau" bash -c "! grep -aqE 'KERNEL PANIC|panicked at' '$LOG'"
exige "la sonde a fini, tous ses ecrivains a zero" grep -aq "SERIE_ENTRELACEMENT_FIN ecrivains=$ECRIVAINS lignes=$LIGNES processus=$PROCESSUS ecrivains_en_echec=0" "$LOG"
exige "le noyau a ecrit pendant ce temps (fautes voulues)" grep -aq 'PROCESS_FAULT' "$LOG"
ADMISES=""
[ "${BANC_PERDUES_ADMISES:-0}" = 1 ] && ADMISES=--perdues-admises
exige "toutes les lignes intactes" python3 tools/ci/analyse_serie_entrelacement.py "$LOG" "$ECRIVAINS" "$LIGNES" "$CHARGE" $ADMISES
exige "aucune ligne emise sans le jeton ni en reentrance" grep -aqE 'BOUCHAUD_SERIE_BILAN emissions_a_la_borne=0 emissions_reentrees=0' "$LOG"
# Le bilan lui-meme, pour le journal du job.
sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -a 'BOUCHAUD_SERIE_BILAN' | tail -1 | sed 's/^.*BOUCHAUD_SERIE_BILAN/  bilan :/'
if [ "${BANC_PERDUES_ADMISES:-0}" = 1 ]; then
  exige "chaque ligne differee est videe ou comptee perdue" bash -c "sed -E 's/\x1b\[[0-9;]*m//g' '$LOG' | grep -aqE 'lignes_differees=([0-9]+) lignes_perdues=[0-9]+ lignes_vidangees=\1'"
else
  exige "aucune ligne differee perdue, toutes videes avant le bilan" bash -c "sed -E 's/\x1b\[[0-9;]*m//g' '$LOG' | grep -aqE 'lignes_differees=([0-9]+) lignes_perdues=0 lignes_vidangees=\1'"
fi
if [ "$SUSPENSION" -gt 0 ]; then
  exige "la barriere noyau a confirme le detenteur" grep -aq 'SERIE_BANC_ATTEND_OK type=detenteur' "$LOG"
  exige "le detenteur suspendu a tenu le jeton ${SUSPENSION} ms" grep -aq "SERIE_DETENTEUR_SUSPENDU_FAIT pris=1 duree_ms=${SUSPENSION}" "$LOG"
  # Run 39740388363 (63b519a8, TCG) : ce controle a rougi sans que le job
  # dise si le banc n'avait pas pris le jeton (pris=0) ou n'avait jamais
  # tourne. La ligne, telle quelle (ou son absence), pour le journal.
  echo "  banc : $(sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -a 'SERIE_DETENTEUR_SUSPENDU_FAIT\|SERIE_DETENTEUR_SUSPENDU debut' | sed 's/^.*SERIE_DETENTEUR/SERIE_DETENTEUR/' | tr '\n' ' ' | cut -c1-300 || true)"
  exige "sa ligne est sortie entiere" bash -c "sed -E 's/\x1b\[[0-9;]*m//g' '$LOG' | tr -d '\r' | grep -aqE '\] SERIE_DETENTEUR_SUSPENDU debut et fin, tenu ${SUSPENSION} ms\$'"
  # BOUCHAUD_CONSOLE_ASYNCHRONE_V1 : console asynchrone, les lignes attendent
  # dans l'anneau et le fil `console` bute sur le jeton (jeton_occupe) ;
  # synchrone, elles sont differees comme avant.
  exige "la suspension a retenu des lignes (differees, ou fil bloque par le jeton)" bash -c "sed -E 's/\x1b\[[0-9;]*m//g' '$LOG' | grep -aqE 'lignes_differees=[1-9]|BOUCHAUD_CONSOLE_BILAN active=1 .* jeton_occupe=[1-9]'"
fi
if [ "$VERROU_SUSPENDU" -gt 0 ]; then
  exige "la barriere noyau a confirme le verrou" grep -aq 'SERIE_BANC_ATTEND_OK type=verrou' "$LOG"
  exige "le fil a tenu le verrou de l'anneau ${VERROU_SUSPENDU} ms" grep -aq "CONSOLE_VERROU_SUSPENDU_FAIT pris=1 duree_ms=${VERROU_SUSPENDU}" "$LOG"
  echo "  banc : $(sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -a 'CONSOLE_VERROU_SUSPENDU_' | sed 's/^.*CONSOLE_VERROU/CONSOLE_VERROU/' | tr '\n' ' ' | cut -c1-300 || true)"
  exige "la suspension a ete subie (attentes cedees ou renoncements)" bash -c "sed -E 's/\x1b\[[0-9;]*m//g' '$LOG' | grep -aqE 'BOUCHAUD_CONSOLE_BILAN .* (verrou_abandons=[1-9][0-9]* |verrou_cedees=[1-9])'"
  exige "aucune attente du verrou interruptions masquees au-dela de 50 ms" bash -c "sed -E 's/\x1b\[[0-9;]*m//g' '$LOG' | grep -a 'BOUCHAUD_CONSOLE_BILAN' | tail -1 | tr -d '\\r' | grep -qE 'verrou_attente_pire_us=([0-9]{1,4}|[1-4][0-9]{4})( |\$)'"
fi
if [ "$RAFALE" -gt 0 ]; then
  exige "la rafale masquee a commence pendant la suspension" grep -aq 'SERIE_BANC_ATTEND_OK type=rafale' "$LOG"
  echo "  rafale : $(sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -a 'SERIE_RAFALE_MASQUEE_FAIT' | sed 's/^.*SERIE_RAFALE/SERIE_RAFALE/' | tr -d '\r' | sed -n 1p)"
  exige "les ${RAFALE} lignes de la rafale sont sorties entieres et dans l'ordre" bash -c "sed -E 's/\x1b\[[0-9;]*m//g' '$LOG' | tr -d '\r' | grep -aoE 'SERIE_RAFALE_MASQUEE [0-9]+/${RAFALE} #+\$' | awk -v n=${RAFALE} '{ split(\$2, a, \"/\"); if (length(\$3) != 80 || a[1] != k + 1) bad = 1; k = a[1] } END { exit !(k == n && !bad) }'"
fi
# Le bilan de fin, sans couleurs : les exigences ci-dessous le lisent.
BILAN=$(sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -a 'BOUCHAUD_CONSOLE_BILAN' | tail -1 | tr -d '\r' || true)
# `sed -n 1p` lit tout (pas de SIGPIPE sous pipefail, contrairement a `head`).
champ() { printf '%s\n' "$BILAN" | grep -oE "(^| )$1=[0-9]+" | sed -n '1s/.*=//p' || true; }
if [ "$VERROU_CIBLE" = fil ]; then
  exige "la suspension a frappe le coeur du fil console" grep -aqE "CONSOLE_VERROU_SUSPENDU_FAIT pris=1 duree_ms=${VERROU_SUSPENDU} coeur=" "$LOG"
  if [ "$SECOURS" = on ]; then
    exige "le fil pret sur le coeur suspendu a ete reloge (secours>=1)" test "$(champ secours)" -ge 1
  fi
fi
# BOUCHAUD_CONSOLE_ASYNCHRONE_V1 : console asynchrone active jusqu'au bout, sans
# repli de securite (fil muet) ni reentrance sur le verrou de l'anneau.
if [ "$FIL_GELE" -gt 0 ] && [ "$PAS_MS" -eq 0 ]; then
  # Fil gele sous un flot qui SATURE l'anneau : les producteurs vident
  # eux-memes, la console progresse -- pas une panne, aucun repli.
  exige "le fil a ete gele (gels=1)" test "$(champ gels)" -eq 1
  exige "fil gele, anneau vide par ses producteurs : aucun repli" bash -c "printf '%s\n' \"\$1\" | grep -qE 'BOUCHAUD_CONSOLE_BILAN active=1 repli_securite=0 .* reentrees=0 '" _ "$BILAN"
elif [ "$FIL_GELE" -gt 0 ]; then
  # La vraie panne : producteurs lents, personne d'autre que le fil ne vide.
  # Le chien de garde DOIT replier, dans sa borne, et dire pourquoi. Un banc
  # ou la console resterait asynchrone prouverait un chien de garde desarme.
  exige "le fil a ete gele (gels=1)" test "$(champ gels)" -eq 1
  exige "repli de securite sur le fil gele" bash -c "printf '%s\n' \"\$1\" | grep -qE 'BOUCHAUD_CONSOLE_BILAN active=0 repli_securite=1 .* reentrees=0 '" _ "$BILAN"
  exige "repli explique : fil en phase 5 (gele)" bash -c "sed -E 's/\x1b\[[0-9;]*m//g' '$LOG' | grep -aqE 'BOUCHAUD_CONSOLE_REPLI phases=[0-9]+ fil_phase=5 '"
  muet=$(champ repli_muet_ms); soupcon=$(champ repli_soupcon_ms)
  exige "silence borne : 2 000 <= repli_muet_ms (${muet:-?}) <= 3 000" bash -c "[ -n '$muet' ] && [ '$muet' -ge 2000 ] && [ '$muet' -le 3000 ]"
  exige "confirmation bornee : 500 <= repli_soupcon_ms (${soupcon:-?}) <= 1 000" bash -c "[ -n '$soupcon' ] && [ '$soupcon' -ge 500 ] && [ '$soupcon' -le 1000 ]"
elif [ "$SECOURS" = off ] && [ "$VERROU_CIBLE" = fil ]; then
  # Bras temoin : sans relogement, le fil reste dans la file du coeur
  # suspendu -- le defaut du run 39667691144 doit revenir.
  exige "sans secours, le repli revient (temoin du defaut)" bash -c "printf '%s\n' \"\$1\" | grep -qE 'BOUCHAUD_CONSOLE_BILAN active=0 repli_securite=1 '" _ "$BILAN"
elif [ "$MODE_CONSOLE" != off ]; then
  exige "console asynchrone active, sans repli ni reentrance" bash -c "sed -E 's/\x1b\[[0-9;]*m//g' '$LOG' | grep -aqE 'BOUCHAUD_CONSOLE_BILAN active=1 repli_securite=0 .* reentrees=0 '"
fi
sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -a 'BOUCHAUD_CONSOLE_BILAN' | tail -1 | sed 's/^.*BOUCHAUD_CONSOLE_BILAN/  console :/' 
# BOUCHAUD_CONSOLE_CONTRE_PRESSION_V1 : le cout de l'attente des producteurs
# (anneau plein) et les chemins qui finissent dans l'anneau d'un coeur.
sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -a 'BOUCHAUD_CONSOLE_CONTRE_PRESSION' | tail -1 | sed 's/^.*BOUCHAUD_CONSOLE_CONTRE_PRESSION/  contre-pression :/'
sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -a 'BOUCHAUD_VGA_TEXTE' | tail -1 | sed 's/^.*BOUCHAUD_VGA_TEXTE/  vga texte :/'
if [ "$echecs" -eq 0 ]; then
  echo "SERIE_ENTRELACEMENT_BANC_OK"
else
  echo "SERIE_ENTRELACEMENT_BANC_ECHEC n=$echecs"
  exit 1
fi
