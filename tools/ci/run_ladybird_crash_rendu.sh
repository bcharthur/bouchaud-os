#!/usr/bin/env bash
# Un rendu plante, les autres vivent ; le navigateur le remplace
# (BOUCHAUD_CRASH_RENDU_V1, convergence : isolation des rendus et reprise).
#
#   tools/ci/run_ladybird_crash_rendu.sh BOOTIMAGE NATIVE_DIR [DUREE_S]
#
# L'onglet A (http://10.0.2.2:18082/crash-a.html) anime un canvas et rapporte
# chaque seconde ses trames et son plus long gel. Il ouvre un onglet sur le
# MEME site (crash-b0.html), qui navigue au niveau racine vers L'AUTRE site
# (10.0.2.100) : upstream change alors de WebContent ([LB] PROCESS_SWAP).
# (Un window.open direct vers l'autre site resterait dans le processus de A :
# l'onglet nait sur about:blank, et about:blank -> tout site ne change pas de
# processus.) B prend le titre
# BOUCHAUD_BANC_CRASH_RENDU ; UI/Bouchaud demande alors a SON WebContent une
# vraie faute (ecriture a une page non mappee, prepare-compositor-lien.py).
#
# Exige :
#   - B tourne dans un WebContent different de A ;
#   - le WebContent de B meurt d'une faute du noyau (PROCESS_FAULT, son pid) ;
#   - la reprise d'upstream lui donne un NOUVEAU WebContent ([LB:CRASH]
#     nouveau_pid != ancien), qui presente encore des trames pour B ;
#   - A continue : ses trames avancent apres le plantage, il finit
#     (HOST_CRASH_A_FIN), son WebContent n'a pas change ;
#   - le Compositor est le meme du debut a la fin, aucune assertion, aucune
#     panique, aucun COMPOSITOR_LINK_GIVE_UP.
# Mesure : plus long gel de A (gel_max_ms) avant et apres le plantage, delai
# entre la demande et la reprise.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_ladybird_crash_rendu.sh BOOTIMAGE NATIVE_DIR [DUREE_S]}
OUT=${2:?usage: run_ladybird_crash_rendu.sh BOOTIMAGE NATIVE_DIR [DUREE_S]}
DUREE=${3:-90}

SCENARIO=scenario-crash-rendu
IMAGE=ladybird-crash-rendu.img
LOG=serie-crash-rendu.log
rm -rf "$SCENARIO" "$IMAGE" "$LOG" "$LOG.propre" fixture-crash-rendu.log

python3 tools/health/browser_host_fixture.py > fixture-crash-rendu.log 2>&1 &
FIXTURE=$!
# Panique noyau : son contexte en DERNIER (tools/ci/extrait_panique.sh).
trap 'kill "$FIXTURE" 2>/dev/null || true; tools/ci/extrait_panique.sh "$LOG"' EXIT
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

URL="http://10.0.2.2:18082/crash-a.html?duree=$DUREE"
cat > "$SCENARIO/autorun" <<AUTORUN
echo "=== Bouchaud crash d'un rendu ==="
export BO_AUTOSTART_BROWSER=1
export BOUCHAUD_M9=1
export BOUCHAUD_BROWSER_HOST=1
export BOUCHAUD_M11=1
export BOUCHAUD_TIME_ZONE=Europe/Paris
export BOUCHAUD_ALLOW_POPUPS=1
export BOUCHAUD_LB_BANC_CRASH_RENDU=1
export BOUCHAUD_M9_URL='$URL'
desktop
AUTORUN
(cd tools/userland && IMAGE="$PWD/../../$IMAGE" ./mkdisk.sh "$PWD/../../$SCENARIO" >/dev/null)

: > "$LOG"
qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$IMAGE" \
  -m 8192 -smp 4 -cpu max -display none -no-reboot \
  -netdev "user,id=net0,guestfwd=tcp:10.0.2.100:18082-cmd:nc 127.0.0.1 18082" -device e1000,netdev=net0 \
  -audiodev none,id=muet -device AC97,audiodev=muet \
  -serial file:"$LOG" &
PID=$!
DEBUT=$SECONDS
LIMITE=$((SECONDS + DUREE + 600))
verdict=vm_morte
while kill -0 "$PID" 2>/dev/null; do
  if grep -aq 'HOST_CRASH_A_FIN' "$LOG"; then verdict=fini; break; fi
  if grep -aq 'KERNEL PANIC' "$LOG"; then verdict=panique; break; fi
  if (( SECONDS >= LIMITE )); then verdict=echeance; break; fi
  sleep 3
done
sleep 3
kill -TERM "$PID" 2>/dev/null || true
sleep 1
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true

# Nettoye UNE fois (couleurs ANSI, prefixe d'horodatage) : sous `pipefail`,
# `sed | grep -q` echoue des que grep ferme le tube avant la fin.
sed -E 's/\x1b\[[0-9;]*m//g; s/^\[[^]]*\]\[[^]]*\]\[FPS:[^]]*\] //' "$LOG" | tr -d '\r' > "$LOG.propre"
P="$LOG.propre"
echo "CRASH_RENDU_VERDICT_BOUCLE $verdict duree_reelle_s=$((SECONDS - DEBUT))"

echo "== chronologie =="
grep -anE 'HOST_CRASH_A_OUVRE_B|HOST_CRASH_B0?|RENDERER_CRASH_(REQUEST|TEST)|PRESENT_APRES_REPRISE|\[LB\] PROCESS_(CREATE|EXIT|SWAP)|\[LB\] SIGCHLD_|PROCESS_(EXIT|DEATH) t=[0-9]+ pid=[0-9]+ ppid=[0-9]+ image=/usr/libexec/ladybird/WebContent|PROCESS_FAULT|\[LB:CRASH\]|WebContent process crashed|CONNECTION_(CREATE|REMOVE)|COMPOSITOR_LINK|HOST_CRASH_A_FIN|VERIFICATION FAILED|KERNEL PANIC' "$P" \
  | sed -E 's/^([0-9]+):.*(HOST_|\[LB|PROCESS_FAULT|WebContent process|VERIFICATION|KERNEL)/\1: \2/' | awk 'NR <= 40 { print "  " $0 }' || true

# Les numeros de ligne ordonnent les evenements.
ligne_de() { grep -anE "$1" "$P" | head -1 | cut -d: -f1 || true; }
demande=$(grep -aoE '\[LB\] RENDERER_CRASH_REQUEST onglet=[0-9]+ page=[0-9]+ webcontent_pid=[0-9]+' "$P" | head -1 || true)
onglet_b=$(echo "$demande" | grep -oE 'onglet=[0-9]+' | cut -d= -f2 || true)
pid_b=$(echo "$demande" | grep -oE 'webcontent_pid=[0-9]+' | cut -d= -f2 || true)
l_demande=$(ligne_de '\[LB\] RENDERER_CRASH_REQUEST')
reprise=$(grep -aoE "\[LB:CRASH\] onglet=${onglet_b:-x} webcontent=mort nouveau_pid=[0-9]+" "$P" | head -1 || true)
pid_b2=$(echo "$reprise" | grep -oE 'nouveau_pid=[0-9]+' | cut -d= -f2 || true)
l_reprise=$(ligne_de "\[LB:CRASH\] onglet=${onglet_b:-x} ")
faute=$(grep -aoE "PROCESS_FAULT pid=${pid_b:-x} reason=[^ ]+" "$P" | head -1 || true)
# Qui execute quoi : le prefixe `WebContent(PID): (js log)` des consoles.
pid_js() { grep -aoE "WebContent\(([0-9]+)\): \(js log\) \"$1" "$P" | ${2:-head} -1 | grep -oE '[0-9]+' | head -1 || true; }
pid_a=$(pid_js 'HOST_CRASH_A t_s=')
pid_a_fin=$(pid_js 'HOST_CRASH_A_FIN')
pid_b_js=$(pid_js 'HOST_CRASH_B origine=')
swap_b=$(grep -aoE "\[LB\] PROCESS_SWAP onglet=${onglet_b:-x} raison=autre_site ancien_pid=[0-9-]+ nouveau_pid=[0-9]+" "$P" | head -1 | sed 's/.*raison=autre_site //' || true)
swap_nouveau=$(echo "$swap_b" | grep -oE 'nouveau_pid=[0-9]+' | cut -d= -f2 || true)
pids_wc=$(grep -aoE 'PERF_EXECVE .*image=/usr/libexec/ladybird/WebContent pid=[0-9]+' "$P" | grep -oE 'pid=[0-9]+$' | cut -d= -f2 | awk '!vu[$0]++' | tr '\n' ' ' || true)
compositors=$(grep -aoE 'PERF_EXECVE .*image=/usr/libexec/ladybird/Compositor pid=[0-9]+' "$P" | grep -oE 'pid=[0-9]+$' | sort -u | wc -l || true)
# Trames de A : derniere valeur avant la demande, et a la fin.
raf_avant=$(awk -v l="${l_demande:-0}" 'NR < l && match($0, /HOST_CRASH_A t_s=[0-9]+ raf=[0-9]+/) { s = substr($0, RSTART, RLENGTH); sub(/.*raf=/, "", s); v = s } END { print v + 0 }' "$P")
ticks_a_apres=$(awk -v l="${l_demande:-999999999}" 'NR > l && /HOST_CRASH_A t_s=/ && /WebContent\(/ { n++ } END { print n + 0 }' "$P")
fin=$(grep -aoE 'HOST_CRASH_A_FIN t_s=[0-9]+ raf=[0-9]+ gel_max_ms=[0-9]+' "$P" | head -1 || true)
raf_fin=$(echo "$fin" | grep -oE 'raf=[0-9]+' | cut -d= -f2 || true)
gel_avant=$(awk -v l="${l_demande:-0}" 'NR < l && match($0, /HOST_CRASH_A t_s=[0-9]+ raf=[0-9]+ gel_max_ms=[0-9]+/) { s = substr($0, RSTART, RLENGTH); sub(/.*gel_max_ms=/, "", s); v = s } END { print v + 0 }' "$P")
gel_fin=$(echo "$fin" | grep -oE 'gel_max_ms=[0-9]+' | cut -d= -f2 || true)
apres_reprise=$(grep -aoE "\[LB\] PRESENT_APRES_REPRISE onglet=${onglet_b:-x} page=[0-9]+ webcontent_pid=[0-9]+ .*" "$P" | head -1 || true)
pid_trame_b=$(echo "$apres_reprise" | grep -oE 'webcontent_pid=[0-9]+' | cut -d= -f2 || true)
a_change=$(awk -v l="${l_demande:-0}" -v o="${onglet_b:-x}" 'NR > l && /\[LB:CRASH\] onglet=/ && !index($0, "onglet=" o " ") { n++ } END { print n + 0 }' "$P")
delai=""
if [ -n "$l_demande" ] && [ -n "$l_reprise" ]; then
  t1=$(grep -aoE '\[LB\] RENDERER_CRASH_REQUEST .* t_ms=[0-9]+' "$P" | head -1 | grep -oE 't_ms=[0-9]+$' | cut -d= -f2 || true)
  t2=$(grep -aoE "\[LB:CRASH\] onglet=${onglet_b} .* t_ms=[0-9]+" "$P" | head -1 | grep -oE 't_ms=[0-9]+$' | cut -d= -f2 || true)
  [ -n "$t1" ] && [ -n "$t2" ] && delai=$((t2 - t1))
fi
echo "CRASH_RENDU_MESURE onglet_b=${onglet_b:-?} pid_a=${pid_a:-?} pid_a_fin=${pid_a_fin:-?} pid_b=${pid_b:-?} pid_b_nouveau=${pid_b2:-?} webcontents=[${pids_wc}] delai_reprise_ms=${delai:-?} raf_a_avant=${raf_avant} raf_a_fin=${raf_fin:-?} gel_a_avant_ms=${gel_avant} gel_a_fin_ms=${gel_fin:-?} releves_a_apres=${ticks_a_apres} trame_b_apres_reprise=[${apres_reprise#*PRESENT_APRES_REPRISE }]"

echo "== verdict =="
echecs=()
exige() { local quoi=$1; shift; if "$@"; then echo "  ok      $quoi"; else echo "  ECHEC   $quoi"; echecs+=("$quoi"); fi; }
dans() { grep -aqE "$1" "$P"; }
exige "A a ouvert B" dans 'HOST_CRASH_A_OUVRE_B ok=true'
exige "B est parti du site de A" dans 'HOST_CRASH_B0 part_de=http://10\.0\.2\.2:18082'
exige "B tourne sur l'autre site" dans 'HOST_CRASH_B origine=http://10\.0\.2\.100:18082'
exige "demande de plantage de B (pid ${pid_b:-?})" test -n "$pid_b"
exige "B a change de WebContent en changeant de site (PROCESS_SWAP ${swap_b:-absent})" test -n "$swap_b" -a "${swap_nouveau:-x}" = "${pid_b:-y}"
exige "le pid demande est celui qui execute B (${pid_b_js:-?})" test -n "$pid_b_js" -a "${pid_b_js:-x}" = "${pid_b:-y}"
exige "B et A dans des WebContent differents (${pid_a:-?} / ${pid_b:-?})" test -n "$pid_a" -a "${pid_a:-x}" != "${pid_b:-x}"
exige "le WebContent de B a faute (noyau : ${faute:-rien})" test -n "$faute"
exige "B repris dans un NOUVEAU WebContent (${pid_b:-?} -> ${pid_b2:-?})" test -n "$pid_b2" -a "${pid_b2:-x}" != "${pid_b:-x}"
exige "le navigateur a recolte la mort de ${pid_b:-?} (PROCESS_EXIT)" dans "\\[LB\\] PROCESS_EXIT type=WebContent pid=${pid_b:-x} "
exige "le navigateur a enregistre ${pid_b2:-?} (PROCESS_CREATE)" dans "\\[LB\\] PROCESS_CREATE type=WebContent pid=${pid_b2:-x} "
exige "une trame de B presentee par son NOUVEAU WebContent (${pid_trame_b:-aucune})" test -n "$pid_trame_b" -a "${pid_trame_b:-x}" = "${pid_b2:-y}"
exige "A a fini (HOST_CRASH_A_FIN)" test -n "$fin"
# A est un onglet d'ARRIERE-PLAN des que B s'ouvre : sans rAF, a juste titre
# (run 37584587000 : raf=19 avant ET apres). Sa vie se lit a ses minuteries :
# une ligne HOST_CRASH_A par seconde.
exige "A a continue de tourner apres le plantage (${ticks_a_apres} releves)" test "$ticks_a_apres" -ge 10
exige "A garde son WebContent (${pid_a:-?} -> ${pid_a_fin:-?})" test -n "$pid_a_fin" -a "${pid_a_fin:-x}" = "${pid_a:-y}"
exige "aucun autre onglet n'a plante (${a_change})" test "$a_change" -eq 0
exige "un seul Compositor du debut a la fin ($compositors)" test "$compositors" -eq 1
exige "aucune assertion (VERIFICATION FAILED)" bash -c "! grep -aq 'VERIFICATION FAILED' '$P'"
exige "aucune panique noyau" bash -c "! grep -aq 'KERNEL PANIC' '$P'"
exige "aucun abandon de lien Compositor" bash -c "! grep -aq 'COMPOSITOR_LINK_GIVE_UP' '$P'"

if [ ${#echecs[@]} -eq 0 ]; then
  echo "LADYBIRD_CRASH_RENDU_OK pid_b=${pid_b} pid_b_nouveau=${pid_b2} delai_reprise_ms=${delai:-?}"
else
  echo "LADYBIRD_CRASH_RENDU_ECHEC n=${#echecs[@]}"
  exit 1
fi
