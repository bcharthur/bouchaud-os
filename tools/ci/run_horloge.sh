#!/usr/bin/env bash
# AUDIT DES HORLOGES 1 / 10 / 60 s CONTRE LE TEMPS DE L'HOTE
# (BOUCHAUD_HORLOGE_AUDIT_V1).
#
#   tools/ci/run_horloge.sh BOOTIMAGE [PLAFOND_S]
#
# horloge-probe juge dans l'invite ce qu'il peut juger seul (sommeil jamais
# plus court, REALTIME qui suit MONOTONIC, MONOTONIC qui ne recule pas). Le
# juge exterieur est ici : la sortie serie passe par un tube et chaque ligne
# est horodatee A SON ARRIVEE par l'horloge monotone de l'hote. Pour chaque
# duree d, la duree vue par l'invite (mono_us) contre celle vue par l'hote
# entre HORLOGE_DEBUT et HORLOGE_FIN :
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
rm -rf "$SCENARIO" "$IMAGE" "$LOG" "$STAMPS"
OUT=out-horloge
rm -rf "tools/userland/$OUT"
(cd tools/userland && OUT=$OUT ./build.sh musl >/dev/null)
mkdir -p "$SCENARIO/bin"
cp "tools/userland/$OUT/horloge-probe" "$SCENARIO/bin/"
cat > "$SCENARIO/autorun" <<AUTORUN
/bin/horloge-probe $PLAFOND
echo HORLOGE_FIN_BANC
AUTORUN
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
  -serial stdio < /dev/null 2>/dev/null | python3 horodateur.py "$LOG" "$STAMPS" &
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

sed -E 's/\x1b\[[0-9;]*m//g; s/^\[[^]]*\]\[[^]]*\]\[FPS:[^]]*\] //' "$LOG" | grep -aE '^  .*(ok|ECHEC) \(|HORLOGE_(INVITE|DEBUT|FIN)' || true

rc_hote=0
python3 - "$STAMPS" "$PLAFOND" <<'PY' || rc_hote=$?
import re, sys
debut, echecs, vus, mesurees = {}, 0, 0, set()
for ligne in open(sys.argv[1], encoding="utf-8"):
    t_mono, t_mur, texte = ligne.rstrip("\n").split("\t", 2)
    texte = re.sub(r"\x1b\[[0-9;]*m", "", texte)
    if m := re.search(r"HORLOGE_DEBUT d=(\d+)", texte):
        debut[int(m[1])] = int(t_mono)
    elif m := re.search(r"HORLOGE_FIN d=(\d+) mono_us=(-?\d+) reel_us=(-?\d+) excedent_us=(-?\d+) reel_ms=(\d+)", texte):
        d = int(m[1])
        if d not in debut:
            continue
        vus += 1
        mesurees.add(d)
        hote_ms = (int(t_mono) - debut[d]) / 1e6
        invite_ms = int(m[2]) / 1e3
        ecart = invite_ms - hote_ms
        pct = 100.0 * ecart / hote_ms if hote_ms else 0.0
        decalage = int(m[5]) - int(t_mur) // 1_000_000
        ok_duree = abs(ecart) <= 50 if d < 10 else abs(pct) <= 1.0
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
grep -aq HORLOGE_INVITE_OK "$LOG" || { echo "HORLOGE_ECHEC verifications de l'invite" >&2; exit 1; }
[ "$rc_hote" -eq 0 ] || { echo "HORLOGE_ECHEC duree vue par l'invite contre l'hote" >&2; exit 1; }
echo HORLOGE_AUDIT_OK
