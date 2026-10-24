#!/usr/bin/env bash
# Diagnostic opt-in. Aucun resultat de ce boot ne qualifie P1-P5.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_reveil_cout.sh BOOT SMP OUT}
SMP=${2:?}
OUT=${3:?}
test ! -e "$OUT/serie.log" # Ne pas ecraser une preuve precedente.
mkdir -p "$OUT/files/bin"
OUT=$(realpath "$OUT")
(cd tools/userland && OUT="$OUT/sondes" ./build.sh musl >/dev/null)
cp "$OUT/sondes/ordonnanceur-probe" "$OUT/files/bin/"
{
  [ "${BO_QEMU_KVM:-0}" = 1 ] || echo 'reveil-precis periodique on'
  echo 'reveil-precis cout on'
  for mode in off on; do
    echo "echo COUT_PHASE bras=$mode"
    echo "reveil-precis $mode"
    echo '/bin/ordonnanceur-probe 8 1'
    echo 'reveil-precis'
    echo "echo COUT_PHASE_FIN bras=$mode"
  done
  echo 'reveil-precis cout off'
  echo 'echo REVEIL_COUT_FIN'
  echo 'poweroff'
} > "$OUT/files/autorun"
(cd tools/userland && IMAGE="$OUT/cout.img" ./mkdisk.sh "$OUT/files" >/dev/null)
accel=(-cpu max)
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  test -w /dev/kvm
  accel=(-enable-kvm -cpu host,kvm-steal-time=on)
fi
timeout 180 qemu-system-x86_64 "${accel[@]}" -m 4096 -smp "$SMP" \
  -drive "format=raw,file=$BOOT" -drive "format=raw,file=$OUT/cout.img" \
  -display none -no-reboot -serial "file:$OUT/serie.log" \
  -netdev user,id=net0 -device e1000,netdev=net0 \
  -audiodev none,id=muet -device AC97,audiodev=muet
python3 - "$OUT/serie.log" "$SMP" <<'PY'
from pathlib import Path
import re, sys
sys.path.insert(0, 'tools/ci')
from publie_p13_reveil import blocs
raw = re.sub(r'\x1b\[[0-9;]*m', '', Path(sys.argv[1]).read_text(errors='replace'))
assert not re.search(r'KERNEL PANIC|panicked at', raw), 'panique'
assert re.search(r'(?m)^.*(?<!echo )REVEIL_COUT_FIN\r?$', raw), 'diagnostic incomplet'
assert raw.count('REVEIL_COUT diagnostic=1 site=scan_du ') == 5, 'releves absents'
lines = ['DIAGNOSTIC_SEULEMENT ne_qualifie_pas_P13=1 smp=' + sys.argv[2]]
phase, mesures = None, {}
for line in raw.splitlines():
    if '+ echo ' in line:
        continue
    debut = re.search(r'COUT_PHASE bras=(on|off)', line)
    if debut:
        phase = debut[1]
        mesures[phase] = {}
    if 'COUT_PHASE_FIN' in line:
        phase = None
    cout = re.search(r'REVEIL_COUT diagnostic=1 site=(\w+) appels=(\d+) cycles=(\d+) maximum=(\d+) hz=(\d+)', line)
    if cout and phase:
        mesures[phase].setdefault(cout[1], []).append(tuple(map(int, cout.groups()[1:])))
    for key in ('COUT_PHASE', 'REVEIL_COUT diagnostic=', 'VERDICT_SONDE :'):
        if key in line:
            lines.append(line[line.index(key):]); break
for phase, sites in mesures.items():
    for site, releves in sites.items():
        assert len(releves) == 2, (phase, site, 'releves incomplets')
        a, b = releves
        appels, cycles = b[0] - a[0], b[1] - a[1]
        assert appels >= 0 and cycles >= 0 and b[3] > 0, 'compteurs incoherents'
        # Les sites comptes seulement n'ont aucune mesure de duree.
        moyenne = str(cycles * 1_000_000_000 // b[3] // appels) if appels and cycles else 'non-mesuree'
        lines.append(f'COUT_DELTA bras={phase} site={site} appels={appels} moyenne_inclusive_ns={moyenne}')
report = '\n'.join(lines)
for n, part in enumerate(blocs(report), 1):
    part = part.replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
    print(f'::notice title=P13 cout SMP{sys.argv[2]} bloc{n}::{part}')
print(report)
PY
