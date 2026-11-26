#!/usr/bin/env bash
# AUDIT DES HORLOGES 1 / 10 / 60 s CONTRE LE TEMPS DE L'HOTE
# (BOUCHAUD_HORLOGE_AUDIT_V1).
#
#   tools/ci/run_horloge.sh BOOTIMAGE [PLAFOND_S]
#
# horloge-probe juge dans l'invite ce qu'il peut juger seul (sommeil jamais
# plus court, REALTIME qui suit MONOTONIC, MONOTONIC qui ne recule pas). Le
# juge exterieur mesure le temps de l'hote entre les marqueurs DEBUT et FIN
# a l'ecriture de leur fin de ligne dans l'UART de QEMU. Le lecteur du tube
# conserve aussi ses timestamps pour diagnostiquer les retards en aval.
# La vidange asynchrone peut retarder DEBUT et FIN differemment, surtout
# derriere le journal de demarrage. Ce banc dedie vide l'anneau et bascule
# la console en synchrone avant la sonde pour comparer les memes intervalles.
#
#   HORLOGE_AUDIT d= invite_ms= hote_ms= ecart_ms= ecart_pct= reel_decalage_ms=
#
# Tolerance : 1 s -> |ecart| <= 50 ms (latence du tube et de la console) ;
# 10 et 60 s -> |ecart| <= 1 %. reel_decalage_ms : CLOCK_REALTIME de
# l'invite moins l'heure de l'hote a l'arrivee de la ligne. Sans NTP, l'ancre
# est la seconde ENTIERE de la RTC CMOS : jusqu'a 1 s perdue a la lecture, et
# jusqu'a 1 s de plus parce que la RTC de QEMU compte ses secondes depuis son
# propre demarrage, pas sur les secondes de l'hote. Borne theorique < 2 s
# (-1,5 s observe sous TCG) ; tolere 2 100 ms (latence du tube comprise). La
# DERIVE, elle, est jugee par la duree.
# BO_QEMU_KVM=1 : meme banc sous KVM (-cpu host). Sans : TCG, -cpu max.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_horloge.sh BOOTIMAGE [PLAFOND_S]}
PLAFOND=${2:-60}

SCENARIO=scenario-horloge
IMAGE=horloge.img
LOG=serie-horloge.log
STAMPS=horloge-horodatage.tsv
TRACE=horloge-uart.log
rm -rf "$SCENARIO" "$IMAGE" "$LOG" "$STAMPS" "$TRACE"
OUT=out-horloge
rm -rf "tools/userland/$OUT"
(cd tools/userland && OUT=$OUT ./build.sh musl >/dev/null)
mkdir -p "$SCENARIO/bin"
cp "tools/userland/$OUT/horloge-probe" "$SCENARIO/bin/"
CONSOLE_SYNC=${BO_HORLOGE_CONSOLE_SYNC:-1}
{
  if [ "$CONSOLE_SYNC" = 1 ]; then echo "console-async off"; fi
  cat <<AUTORUN
/bin/horloge-probe $PLAFOND
echo HORLOGE_FIN_BANC
AUTORUN
} > "$SCENARIO/autorun"
(cd tools/userland && IMAGE="$PWD/../../$IMAGE" ./mkdisk.sh "$PWD/../../$SCENARIO" >/dev/null)

ACCEL="-cpu max"
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
fi
echo "HORLOGE_ACCEL ${ACCEL} plafond_s=${PLAFOND}"

# L'horodateur : lit la sortie serie de QEMU, ecrit le journal brut, et note
# l'instant d'arrivee (monotone et mural de l'hote) des lignes HORLOGE_.
cat > horodateur.py <<'PY'
import sys, time
journal, stamps = open(sys.argv[1], "wb"), open(sys.argv[2], "w")
for brut in sys.stdin.buffer:
    t_mono, t_mur = time.monotonic_ns(), time.time_ns()
    journal.write(brut)
    journal.flush()
    if b"HORLOGE_" in brut:
        texte = brut.decode("utf-8", "replace").rstrip("\r\n")
        stamps.write(f"{t_mono}\t{t_mur}\t{texte}\n")
        stamps.flush()
PY
# shellcheck disable=SC2086
qemu-system-x86_64 $ACCEL \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$IMAGE" \
  -m 2048 -smp 4 -display none -no-reboot \
  -serial stdio -msg timestamp=on -trace enable=serial_write \
  < /dev/null 2>"$TRACE" | python3 horodateur.py "$LOG" "$STAMPS" &
TUBE=$!
trap 'pkill -f "file=$IMAGE" 2>/dev/null || true' EXIT
LIMITE=$((SECONDS + PLAFOND * 2 + 240))
etat=echeance
while (( SECONDS < LIMITE )); do
  if grep -aq HORLOGE_FIN_BANC "$LOG" 2>/dev/null; then etat=fini; break; fi
  if grep -aqE 'KERNEL PANIC|panicked at' "$LOG" 2>/dev/null; then etat=panique; break; fi
  sleep 1
done
pkill -f "file=$IMAGE" 2>/dev/null || true
wait "$TUBE" 2>/dev/null || true
echo "HORLOGE_BANC $etat"

sed -E 's/\x1b\[[0-9;]*m//g; s/^\[[^]]*\]\[[^]]*\]\[FPS:[^]]*\] //' "$LOG" | grep -aE '^  .*(ok|ECHEC) \(|HORLOGE_(INVITE|DEBUT|FIN|CHEMIN)' || true

rc_hote=0
python3 - "$STAMPS" "$PLAFOND" "$TRACE" <<'PY' || rc_hote=$?
import re, sys

def marqueur(texte):
    texte = re.sub(r"\x1b\[[0-9;]*m", "", texte)
    return re.search(r"HORLOGE_(?:DEBUT|FIN) d=\d+.*", texte)

lectures = []
with open(sys.argv[1], encoding="utf-8") as stamps:
    for ligne in stamps:
        t_mono, t_mur, texte = ligne.rstrip("\n").split("\t", 2)
        if m := marqueur(texte):
            lectures.append((m[0], int(t_mono), int(t_mur)))

uart = []
tampon = bytearray()
evenement = re.compile(r"^\d+@(\d+)\.(\d{6}):serial_write write addr 0x00 val (0x[0-9a-f]+)")
with open(sys.argv[3], encoding="utf-8", errors="replace") as trace:
    for ligne in trace:
        if not (m := evenement.match(ligne)):
            continue
        octet = int(m[3], 16)
        if octet != 10:
            tampon.append(octet)
            continue
        if repere := marqueur(tampon.decode("utf-8", "replace").rstrip("\r")):
            t_uart = int(m[1]) * 10**9 + int(m[2]) * 1000
            uart.append((repere[0], t_uart))
        tampon.clear()

if not lectures or [m for m, _, _ in lectures] != [m for m, _ in uart]:
    sys.exit("HORLOGE_ECHEC marqueurs UART manquants ou differents de la serie")
# QEMU trace avec gettimeofday. Une variation mesurable entre les deux
# horloges de l'hote invalide la campagne, y compris si son verdict serait vert.
ecarts_hote = [mur - mono for _, mono, mur in lectures]
if max(ecarts_hote) - min(ecarts_hote) > 1_000_000:
    sys.exit("HORLOGE_ECHEC ecart entre horloges monotone et murale de l'hote instable")

debut, echecs, vus, mesurees = {}, 0, 0, set()
for (texte, t_uart), (_, t_mono, t_mur) in zip(uart, lectures):
    retard_lecteur = (t_mur - t_uart) / 1e6
    repere = " ".join(texte.split(" ", 2)[:2])
    print(f"HORLOGE_TRANSPORT {repere} "
          f"uart_ns={t_uart} lecteur_mono_ns={t_mono} retard_lecteur_ms={retard_lecteur:.3f}")
    if m := re.search(r"HORLOGE_DEBUT d=(\d+)", texte):
        debut[int(m[1])] = t_uart
    elif m := re.search(r"HORLOGE_FIN d=(\d+) mono_us=(-?\d+) reel_us=(-?\d+) excedent_us=(-?\d+) reel_ms=(\d+)", texte):
        d = int(m[1])
        if d not in debut:
            continue
        vus += 1
        mesurees.add(d)
        hote_ms = (t_uart - debut[d]) / 1e6
        invite_ms = int(m[2]) / 1e3
        ecart = invite_ms - hote_ms
        pct = 100.0 * ecart / hote_ms if hote_ms else 0.0
        decalage = int(m[5]) - t_uart // 1_000_000
        ok_duree = hote_ms > 0 and (abs(ecart) <= 50 if d < 10 else abs(pct) <= 1.0)
        ok_reel = abs(decalage) <= 2100
        echecs += (not ok_duree) + (not ok_reel)
        print(f"HORLOGE_AUDIT d={d} invite_ms={invite_ms:.1f} hote_ms={hote_ms:.1f} ecart_ms={ecart:+.1f} "
              f"ecart_pct={pct:+.2f} reel_decalage_ms={decalage:+d} "
              f"{'ok' if ok_duree and ok_reel else 'ECHEC'}")
# Chaque duree attendue doit avoir sa mesure : une ligne perdue n'est pas un
# succes par omission (os-primitives 37746924011 : d=1 absente, verdict OK).
attendues = [d for d in (1, 10, 60) if d <= int(sys.argv[2])]
manquantes = [d for d in attendues if d not in mesurees]
for d in manquantes:
    print(f"HORLOGE_AUDIT d={d} ECHEC mesure absente (ligne DEBUT ou FIN illisible)")
    echecs += 1
if vus == 0:
    print("HORLOGE_AUDIT aucune mesure")
    sys.exit(1)
sys.exit(1 if echecs else 0)
PY
rm -f horodateur.py
if [ "$etat" != fini ]; then echo "HORLOGE_ECHEC banc=$etat" >&2; exit 1; fi
# Le bilan de la bascule doit preceder la premiere mesure.
if [ "$CONSOLE_SYNC" = 1 ]; then
  awk '/HORLOGE_DEBUT d=1[[:space:]]/ { exit }
     /BOUCHAUD_CONSOLE_BILAN active=/ { synchrone = /active=0 repli_securite=0 / }
     END { exit !synchrone }' "$LOG" ||
    { echo "HORLOGE_ECHEC console synchrone non confirmee" >&2; exit 1; }
fi
grep -aq HORLOGE_INVITE_OK "$LOG" || { echo "HORLOGE_ECHEC verifications de l'invite" >&2; exit 1; }
[ "$rc_hote" -eq 0 ] || { echo "HORLOGE_ECHEC duree vue par l'invite contre l'hote" >&2; exit 1; }
echo HORLOGE_AUDIT_OK
