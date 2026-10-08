#!/usr/bin/env bash
# Endurance : >= 10 minutes de navigation sans crash du Compositor
# (BOUCHAUD_ENDURANCE_V1, convergence P1/P6).
#
#   tools/ci/run_ladybird_endurance.sh BOOTIMAGE NATIVE_DIR [DUREE_S]
#
# La page /endurance.html de la fixture boucle DUREE_S secondes (600 par
# defaut) : cadres remplaces (contextes Compositor crees et detruits),
# workers crees puis termines, canvas anime, defilement, images rechargees,
# et tous les trois cycles un onglet qui passe par un relais du meme site sur
# L'AUTRE site (10.0.2.100) puis se ferme -- donc des processus WebContent
# crees ([LB] PROCESS_SWAP) et detruits, et leurs connexions au Compositor
# avec eux. C'est le cycle de vie qui menait au crash
# `ConnectionFromClient.cpp:68 VERIFICATION FAILED: connection`.
# (Jusqu'au run 37581515158, l'onglet s'ouvrait directement sur l'autre site
# et restait dans le WebContent de la page : 27 onglets, 2 WebContent.)
#
# Exige :
#   - HOST_ENDURANCE_FIN apres au moins 95 % de DUREE_S ;
#   - au moins DUREE_S/10 cycles, workers et cadres au rendez-vous ;
#   - au moins un onglet ouvert sur l'autre site (HOST_ENDURANCE_ENFANT) ;
#   - UN SEUL Compositor du debut a la fin (aucun redemarrage) ;
#   - aucune assertion, aucune panique, aucun COMPOSITOR_LINK_GIVE_UP,
#     aucune faute du Compositor.
# Mesure (sans seuil) : RSS de chaque service au premier et au dernier releve,
# evenements de cycle de vie des connexions Compositor.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_ladybird_endurance.sh BOOTIMAGE NATIVE_DIR [DUREE_S]}
OUT=${2:?usage: run_ladybird_endurance.sh BOOTIMAGE NATIVE_DIR [DUREE_S]}
DUREE=${3:-600}
SILENCE_MAX=${BO_ENDURANCE_SILENCE_S:-180}

SCENARIO=scenario-endurance
IMAGE=ladybird-endurance.img
LOG=serie-endurance.log
rm -rf "$SCENARIO" "$IMAGE" "$LOG" fixture-endurance.log

python3 tools/health/browser_host_fixture.py > fixture-endurance.log 2>&1 &
FIXTURE=$!
# Panique noyau : son contexte en DERNIER (tools/ci/extrait_panique.sh).
# Et si le banc echoue sans panique (KVM, run 37618172578 : 1206 s de QEMU,
# navigateur jamais lance), ce que faisaient les coeurs, symbolise contre le
# noyau qui a tourne (BOUCHAUD_SYMBOLISE_NOYAU_V1).
trap 'rc=$?; kill "$FIXTURE" 2>/dev/null || true; tools/ci/extrait_panique.sh "$LOG" "$OUT"; if [ "$rc" -ne 0 ]; then python3 tools/ci/symbolise_noyau.py "$LOG" "$(dirname "$BOOT")/bouchaud-os"; fi; exit "$rc"' EXIT
sleep 1
kill -0 "$FIXTURE"

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

URL="http://10.0.2.2:18082/endurance.html?duree=$DUREE"
cat > "$SCENARIO/autorun" <<AUTORUN
echo "=== Bouchaud endurance ==="
export BO_AUTOSTART_BROWSER=1
export BOUCHAUD_M9=1
export BOUCHAUD_BROWSER_HOST=1
export BOUCHAUD_M11=1
export BOUCHAUD_TIME_ZONE=Europe/Paris
export BOUCHAUD_ALLOW_POPUPS=1
export BOUCHAUD_LB_LIFECYCLE_PROOF=1
export BOUCHAUD_M9_URL='$URL'
desktop
AUTORUN
# BOUCHAUD_PROFIL_RIP_V1 : BO_ENDURANCE_PROFIL_RIP=on|off fixe le profil RIP
# des l'autorun (A/B de son cout, tools/ci/run_ab_profil.sh). Absent : rien.
case "${BO_ENDURANCE_PROFIL_RIP:-}" in
  on|off) sed -i "1i profil-rip ${BO_ENDURANCE_PROFIL_RIP}" "$SCENARIO/autorun" ;;
  "") ;;
  *) echo "BO_ENDURANCE_PROFIL_RIP=${BO_ENDURANCE_PROFIL_RIP} : on ou off" >&2; exit 2 ;;
esac
echo "ENDURANCE_PROFIL_RIP valeur=${BO_ENDURANCE_PROFIL_RIP:-defaut}"
(cd tools/userland && IMAGE="$PWD/../../$IMAGE" ./mkdisk.sh "$PWD/../../$SCENARIO" >/dev/null)

: > "$LOG"
# BOUCHAUD_ENDURANCE_KVM_V1 -- BO_QEMU_KVM=1 : meme banc sous KVM, pour
# separer ce que coute l'emulation TCG de ce que coute le logiciel (run
# 37585729384 : rAF ~1,3 Hz, WebContent a 37 % d'un coeur, Compositor 64 %).
# Diagnostic seulement : la porte de convergence reste sous TCG.
ACCEL="-cpu max"
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
fi
echo "ENDURANCE_ACCEL ${ACCEL}"
qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$IMAGE" \
  -m 8192 -smp 4 $ACCEL -display none -no-reboot \
  -netdev "user,id=net0,guestfwd=tcp:10.0.2.100:18082-cmd:nc 127.0.0.1 18082" -device e1000,netdev=net0 \
  -audiodev none,id=muet -device AC97,audiodev=muet \
  -serial file:"$LOG" &
PID=$!

DEBUT=$SECONDS
PLAFOND=$((DUREE + 900))
taille_vue=0
derniere_avancee=$SECONDS
cycles_vus=0
verdict=inconnu
while kill -0 "$PID" 2>/dev/null; do
  if grep -aq 'HOST_ENDURANCE_FIN' "$LOG"; then verdict=fini; break; fi
  if grep -aq 'KERNEL PANIC' "$LOG"; then verdict=panique; break; fi
  n=$(sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -ac 'js log) "HOST_ENDURANCE cycle=' || true)
  if [ "$n" != "$cycles_vus" ]; then
    cycles_vus=$n
    if (( n % 12 == 0 )); then printf '  T+%-5ss %s cycle(s)\n' "$((SECONDS - DEBUT))" "$n"; fi
  fi
  taille=$(wc -c < "$LOG")
  if [ "$taille" -ne "$taille_vue" ]; then taille_vue=$taille; derniere_avancee=$SECONDS; fi
  if (( SECONDS - derniere_avancee > SILENCE_MAX )); then verdict=muet; break; fi
  if (( SECONDS - DEBUT > PLAFOND )); then verdict=plafond; break; fi
  sleep 5
done
[ "$verdict" = inconnu ] && verdict=vm_morte
sleep 3
kill -TERM "$PID" 2>/dev/null || true
sleep 1
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true

sed -E 's/\x1b\[[0-9;]*m//g; s/^\[[^]]*\]\[[^]]*\]\[FPS:[^]]*\] //' "$LOG" | tr -d '\r' > "$LOG.propre"
P="$LOG.propre"
echo "ENDURANCE_VERDICT_BOUCLE $verdict duree_reelle_s=$((SECONDS - DEBUT))"
grep -aoE 'HOST_ENDURANCE(_FIN)? cycle[s]?=.*' "$P" | awk 'NR % 20 == 1' | head -10 || true
grep -aoE 'HOST_ENDURANCE_FIN .*' "$P" | head -1 || true

# BOUCHAUD_ENDURANCE_ONGLETS_CHRONO_V1 : la vie de chaque onglet « autre
# site » (ouverture au cycle, relais, changement de WebContent, enfant
# charge, fermeture et par qui), horodatee par la ligne serie. Run
# 37661162354 (KVM) : 1 changement sur 10 onglets, aucun enfant charge.
echo "== chronologie des onglets autre site =="
awk '{ gsub(/\x1b\[[0-9;]*m/, ""); gsub(/\r/, "") }
  match($0, /HOST_ENDURANCE cycle=[0-9]+ t_s=[0-9]+/) { split(substr($0, RSTART, RLENGTH), c, /[= ]/); if (c[3] % 3 != 0) next }
  match($0, /HOST_ENDURANCE cycle=[0-9]+ t_s=[0-9]+|HOST_ENDURANCE_(RELAIS|ENFANT|ONGLET) [^"]*|\[LB\] PROCESS_SWAP .*/) {
    h = ""; if (match($0, /^\[[0-9:]+\]/)) h = substr($0, 2, 8)
    if (match($0, /HOST_ENDURANCE cycle=[0-9]+ t_s=[0-9]+|HOST_ENDURANCE_(RELAIS|ENFANT|ONGLET) [^"]*|\[LB\] PROCESS_SWAP .*/) && imprimees++ < 80) print "  " h " " substr($0, RSTART, RLENGTH)
  }' "$LOG"

echo "== cycle de vie des connexions Compositor =="
for m in CONNECTION_CREATE CONNECTION_REMOVE PEER_CLOSE CONTEXT_CREATE CONTEXT_DESTROY LATE_MESSAGE COMPOSITOR_LINK_LOST COMPOSITOR_LINK_RECOVERED COMPOSITOR_LINK_GIVE_UP PROCESS_SWAP; do
  printf '  %-26s %s\n' "$m" "$(grep -ac "\[LB\] $m" "$P" || true)"
done
compositors=$(grep -aoE 'PERF_EXECVE .*image=/usr/libexec/ladybird/Compositor pid=[0-9]+' "$P" | grep -oE 'pid=[0-9]+' | sort -u | wc -l || true)
webcontents=$(grep -aoE 'PERF_EXECVE .*image=/usr/libexec/ladybird/WebContent pid=[0-9]+' "$P" | grep -oE 'pid=[0-9]+' | sort -u | wc -l || true)
echo "  processus Compositor lances : $compositors ; WebContent lances : $webcontents"
echo "== processus du navigateur (ProcessManager : PROCESS_CREATE / PROCESS_EXIT) =="
for t in WebContent WebWorker RequestServer ImageDecoder Compositor; do
  printf '  %-14s crees %-5s sortis %s\n' "$t" "$(grep -ac "\[LB\] PROCESS_CREATE type=$t " "$P" || true)" "$(grep -ac "\[LB\] PROCESS_EXIT type=$t " "$P" || true)"
done

echo "== RSS (kio) premier et dernier releve, par image =="
for img in BouchaudBrowserHost bo-navigateur WebContent RequestServer ImageDecoder Compositor WebWorker; do
  premier=$(grep -aoE "\[PERF-PROC\] t=[0-9]+ pid=[0-9]+ image=[^ ]*$img rss_kio=[0-9]+" "$P" | head -1 | grep -oE 'rss_kio=[0-9]+' || true)
  dernier=$(grep -aoE "\[PERF-PROC\] t=[0-9]+ pid=[0-9]+ image=[^ ]*$img rss_kio=[0-9]+" "$P" | tail -1 | grep -oE 'rss_kio=[0-9]+' || true)
  [ -n "$premier" ] && printf '  %-20s premier %-16s dernier %s\n' "$img" "$premier" "$dernier"
done

# BOUCHAUD_VERDICT_DIAGNOSTIC_V1 : un outil de mesure qui echoue (ligne
# serie entrelacee, ELF absent, adresse non symbolisable) ne fait pas echouer
# le banc -- run 37667817559 (KVM) : une trace Python de profil_rip.py avait
# arrete le banc AVANT son verdict, un navigateur fonctionnel est sorti rouge.
# Il est dit (`DIAGNOSTIC_ECHEC outil= rc=`) et compte dans un verdict
# DIAGNOSTIC separe, imprime a cote du verdict fonctionnel.
diagnostics_ko=()
diagnostic() {
  local outil=$1
  shift
  if "$@"; then
    return 0
  else
    local rc=$?
    echo "  DIAGNOSTIC_ECHEC outil=$outil rc=$rc"
    diagnostics_ko+=("$outil")
  fi
}

# BOUCHAUD_LB_MEM_V1 : ce que le Compositor tient, au premier releve, au pire
# et au dernier -- contextes vivants, surfaces de rendu, caches de Skia (bornes,
# limite publiee). Une croissance de RSS sans croissance de ces compteurs
# designe autre chose qu'eux.
echo "== memoire du Compositor ([LB:MEM]) =="
awk '{ gsub(/\x1b\[[0-9;]*m/, ""); gsub(/\r/, "") }
  match($0, /\[LB:MEM\] ev=[a-z_]+ ctx=[0-9]+ contexts_live=[0-9]+ backing_stores_live=[0-9]+ backing_store_octets=[0-9]+ skia_ressources_octets=[0-9]+ skia_ressources_limite=[0-9]+ skia_polices_octets=[0-9]+/) {
    n = split(substr($0, RSTART, RLENGTH), f, /[ =]/)
    ctx = f[7]; bs = f[9]; oct = f[11]; skia = f[13]; lim = f[15]; pol = f[17]
    if (vus++ == 0) { p_ctx = ctx; p_oct = oct; p_skia = skia }
    if (ctx > m_ctx) m_ctx = ctx; if (oct > m_oct) m_oct = oct; if (skia > m_skia) m_skia = skia
    d_ctx = ctx; d_oct = oct; d_skia = skia; d_pol = pol; d_lim = lim; d_bs = bs
  }
  END {
    if (vus == 0) { print "  LB_MEM absent (aucune ligne [LB:MEM])"; exit }
    printf "  LB_MEM releves=%d contexts premier=%d max=%d dernier=%d backing_store_kio premier=%d max=%d dernier=%d surfaces_dernier=%d skia_ressources_kio premier=%d max=%d dernier=%d limite_kio=%d skia_polices_kio=%d\n",
      vus, p_ctx, m_ctx, d_ctx, p_oct / 1024, m_oct / 1024, d_oct / 1024, d_bs, p_skia / 1024, m_skia / 1024, d_skia / 1024, d_lim / 1024, d_pol / 1024
  }' "$LOG"
echo "== tendance RSS par processus (pente de la seconde moitie de vie) =="
diagnostic tendance_rss python3 tools/ci/tendance_rss.py "$P" --min-releves 10 --prefixe "  "

# Compteurs noyau du dernier releve : lectures de comptabilite refaites
# (BOUCHAUD_COMPTA_SEQLOCK_V1) et recalculs de l'identite du coeur par CPUID
# (BOUCHAUD_GS_NOYAU_EN_IRQ_V1). Affichage seulement.
# BOUCHAUD_APPELS_PAR_PROCESSUS_V1 : un service au repos qui brule un coeur
# (ImageDecoder 20 %, RequestServer 21 %, run 37654172489) se voit a son
# rythme d'appels systeme et a ce qu'il appelle.
echo "== appels systeme par processus (premier -> dernier releve) =="
awk '{ gsub(/\x1b\[[0-9;]*m/, "") }
  match($0, /\[PERF-APPELS\] t=[0-9]+ pid=[0-9]+ image=[^ ]+ appels=[0-9]+ top=[^ \r]+/) {
    l = substr($0, RSTART, RLENGTH); n = split(l, f, " ")
    split(f[2], a, "="); t = a[2]; split(f[3], a, "="); pid = a[2]; split(f[4], a, "="); img = a[2]
    split(f[5], a, "="); app = a[2]; split(f[6], a, "="); top = a[2]
    if (!(pid in t0)) { t0[pid] = t; a0[pid] = app } ; t1[pid] = t; a1[pid] = app; im[pid] = img; tp[pid] = top
  }
  END { for (p in t0) { d = (t1[p] - t0[p]) / 1000; if (d >= 60) printf "  APPELS_TENDANCE pid=%s image=%s vie_s=%d appels_par_s=%d top=%s\n", p, im[p], d, (a1[p] - a0[p]) / d, tp[p] } }' "$LOG" | sort -t= -k6 -nr | head -12
# BOUCHAUD_PROFIL_RIP_V1 : ou les fils passent leur temps (RIP echantillonnes
# au quantum, symbolises contre les binaires du run). Affichage seulement.
# Pas de `| head` : sous `pipefail`, un producteur coupe par SIGPIPE ferait
# sortir le banc avant son verdict ; les bornes sont dans le script.
diagnostic profil_rip python3 tools/ci/profil_rip.py "$LOG" "$OUT" --noyau "$(dirname "$BOOT")/bouchaud-os"
# BOUCHAUD_DRAIN_MESURE_V1 : ce que la lecture forcee du corps HTTP
# (prepare-m9-source.py) a recupere que le notificateur n'avait pas livre.
echo "== contournement M9_BODY_DRAIN =="
awk '{ gsub(/\x1b\[[0-9;]*m/, "") } match($0, /M9_BODY_DRAIN_DONE total=[0-9]+ deja_livre=[0-9]+ recupere=[0-9]+/) { s = substr($0, RSTART, RLENGTH); sub(/.*recupere=/, "", s); n++; if (s + 0 > 0) { k++; o += s } } END { printf "  DRAIN_MESURE drains=%d recupere_non_nul=%d octets_recuperes=%d\n", n, k, o }' "$LOG"
echo "== compteurs noyau (dernier [PROC-STAT]) =="
awk 'match($0, /compta_relues=[0-9]+ replis_apic=[0-9]+( ticks_ms=[0-9]+ mono_ms=[0-9]+)?/) { v = substr($0, RSTART, RLENGTH) } END { print "  " (v != "" ? v : "absents") }' "$LOG"
# BOUCHAUD_TSC_SOURCE_V1 : l'horloge de l'invite. Sous KVM (run
# 37654172489), la page comptait t_s=316 pour 206 s de QEMU : toutes les
# durees du banc (cycles, cadres > 30 s) en dependent.
awk '{ gsub(/\x1b\[[0-9;]*m/, ""); gsub(/\r/, "") } match($0, /BOUCHAUD_TSC_(EARLY_CALIBRATION_OK|CONTROLE) .*/) { print "  " substr($0, RSTART, RLENGTH) }' "$LOG"

echo "== verdict =="
# BOUCHAUD_ENDURANCE_MODERNE_V1 : deux verdicts, jamais melanges.
#
#   STABILITY_GATE    -- le systeme tient : aucune panique, aucune assertion,
#                        aucune faute de processus, aucun echec du bac a sable,
#                        chaque cycle conclu, workers et onglets coherents.
#   PERFORMANCE_GATE  -- le debit : cycles par duree, aucun cadre au-dela de
#                        30 s ; p50/p95/p99 de la latence d'un cadre imprimes.
#
# Les budgets de performance ne changent PAS ici (un cycle par 10 s, aucun
# cadre > 30 s) : ils ne seront redefinis que sur une baseline mesuree. Le banc
# reste rouge si l'un des deux verdicts l'est ; ils sont simplement lisibles
# separement.
echecs=()
stabilite_ko=0
performance_ko=0
exige() { local quoi=$1; shift; if "$@"; then echo "  ok      $quoi"; else echo "  ECHEC   $quoi"; echecs+=("$quoi"); stabilite_ko=$((stabilite_ko + 1)); fi; }
exige_perf() { local quoi=$1; shift; if "$@"; then echo "  ok      $quoi"; else echo "  ECHEC   $quoi"; echecs+=("$quoi"); performance_ko=$((performance_ko + 1)); fi; }
fin=$(grep -aoE 'HOST_ENDURANCE_FIN cycles=[0-9]+ t_s=[0-9]+ cadres_ok=[0-9]+ cadres_echus=[0-9]+ workers_ok=[0-9]+ onglets=[0-9]+.*' "$P" | head -1 || true)
# `|| true` : sans ligne FIN (panique, VM morte), grep rend 1 et `set -e`
# tuait le banc AVANT son verdict (run 37584587000 : rien apres « verdict »).
val() { echo "$fin" | grep -oE "(^| )$1=-?[0-9]+" | head -1 | cut -d= -f2 || true; }
cycles=$(val cycles); t_s=$(val t_s); cadres=$(val cadres_ok); echus=$(val cadres_echus); workers=$(val workers_ok); onglets=$(val onglets)
conclus=$(val cycles_conclus)
echo " -- STABILITE --"
exige "la page a fini (HOST_ENDURANCE_FIN)" test -n "$fin"
exige "duree >= 95 % de ${DUREE} s (t_s=${t_s:-?})" test "${t_s:-0}" -ge $((DUREE * 95 / 100))
exige "chaque cycle conclu, charge ou echu (${conclus:-?}/${cycles:-0})" test "${conclus:-0}" -ge $(( ${cycles:-0} - 1 )) -a "${cycles:-0}" -ge 1
exige "workers au rendez-vous (${workers:-0}/${cycles:-0})" test "${workers:-0}" -ge $(( ${cycles:-0} - 1 ))
exige "onglets sur l'autre site ouverts (${onglets:-0})" test "${onglets:-0}" -ge 1
exige "onglet enfant charge sur 10.0.2.100" grep -aq 'HOST_ENDURANCE_ENFANT .*origine=http://10.0.2.100:18082' "$P"
swaps=$(grep -ac '\[LB\] PROCESS_SWAP onglet=[0-9]* raison=autre_site' "$P" || true)
echo "  marqueurs ponctuels de swap lisibles : $swaps (diagnostic)"
exige "chaque onglet enfant a change de WebContent (preuve cumulative exacte)" python3 tools/ci/preuve_swaps.py "$LOG" "${onglets:-0}" --endurance
exige "un seul Compositor du debut a la fin (compteurs runtime cumulatifs)" python3 tools/ci/preuve_compositor.py "$LOG"
exige "aucune assertion (VERIFICATION FAILED)" bash -c "! grep -aq 'VERIFICATION FAILED' '$P'"
# Un `MUST()` qui echoue ou un ASSERT : toujours suivis d'`ak_trap`, jamais benins.
exige "aucun MUST() ni ASSERT en echec (UNEXPECTED ERROR, ASSERTION FAILED)" bash -c "! grep -aqE 'UNEXPECTED ERROR|ASSERTION FAILED' '$P'"
exige "aucune panique noyau" bash -c "! grep -aq 'KERNEL PANIC' '$P'"
exige "la boucle a fini sur la page, pas sur ${verdict}" test "$verdict" = fini
exige "aucun abandon de lien Compositor" bash -c "! grep -aq 'COMPOSITOR_LINK_GIVE_UP' '$P'"
exige "aucune mort du Compositor" bash -c "! grep -aqE '(PROCESS_FAULT|PROCESS_EXIT|PROCESS_DEATH).*Compositor' '$P'"
# Ce banc n'injecte aucune panne : toute faute d'un processus est inattendue.
exige "aucune faute de processus ($(grep -ac 'PROCESS_FAULT pid=' "$P" || true))" bash -c "! grep -aq 'PROCESS_FAULT pid=' '$P'"
exige "bac a sable : aucun echec, aucun NNP_ABSENT" bash -c "! grep -aqE '\[LB:SANDBOX\] ECHEC|NNP_ABSENT' '$P'"
echo " -- PERFORMANCE (budgets inchanges ; p50/p95/p99 publies) --"
echo "  latence d'un cadre : p50=$(val lat_cadre_p50_ms) ms p95=$(val lat_cadre_p95_ms) ms p99=$(val lat_cadre_p99_ms) ms max=$(val lat_cadre_max_ms) ms moyenne=$(val lat_cadre_moy_ms) ms"
exige_perf "au moins $((DUREE / 10)) cycles (${cycles:-0})" test "${cycles:-0}" -ge $((DUREE / 10))
exige_perf "cadres charges (${cadres:-0}/${cycles:-0}, ${echus:-?} au-dela de 30 s)" test "${cadres:-0}" -ge $(( ${cycles:-0} - 2 ))
exige_perf "aucun cadre au-dela de 30 s (${echus:-?})" test "${echus:-1}" -eq 0
if [ "$stabilite_ko" -eq 0 ]; then echo "STABILITY_GATE ok"; else echo "STABILITY_GATE echec n=$stabilite_ko"; fi
if [ "$performance_ko" -eq 0 ]; then echo "PERFORMANCE_GATE ok"; else echo "PERFORMANCE_GATE echec n=$performance_ko"; fi

# Le verdict DIAGNOSTIC, a part : il ne change pas le verdict fonctionnel.
if [ ${#diagnostics_ko[@]} -eq 0 ]; then
  echo "VERDICT_DIAGNOSTIC ok"
else
  echo "VERDICT_DIAGNOSTIC echec outils=${diagnostics_ko[*]}"
fi
# BOUCHAUD_PORTE_PERF_SOUS_KVM_V1 : BO_ENDURANCE_PERF=diagnostic (job TCG)
# rend le verdict sur la SEULE stabilite ; la performance est imprimee, pas
# jugee, et le marqueur n'est PAS LADYBIRD_ENDURANCE_OK. Sous TCG, la cadence
# mesure l'emulation SSE du raster Skia (Compositor 71 % en mode utilisateur,
# endurance 37746917003) : six releves de 30 a 39 cycles pour 600 s depuis
# a2f4b333, jamais 60. La meme porte, au meme budget, juge KVM (49 a 50
# cycles pour 300 s, 0 cadre au-dela de 30 s). docs/ENDURANCE.md.
if [ "${BO_ENDURANCE_PERF:-bloquant}" = diagnostic ] && [ "$stabilite_ko" -eq 0 ] && [ "$performance_ko" -gt 0 ] \
  && [ "${#echecs[@]}" -eq "$performance_ko" ]; then
  echo "LADYBIRD_ENDURANCE_STABILITE_OK duree_s=${t_s} cycles=${cycles} performance=echec_mesuree"
  exit 0
fi
if [ ${#echecs[@]} -eq 0 ]; then
  echo "LADYBIRD_ENDURANCE_OK duree_s=${t_s} cycles=${cycles}"
else
  echo "LADYBIRD_ENDURANCE_ECHEC n=${#echecs[@]}"
  exit 1
fi
