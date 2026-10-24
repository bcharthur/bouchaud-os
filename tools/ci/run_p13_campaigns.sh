#!/usr/bin/env bash
# Three independent baseline boots/campaigns, one immutable commit and image.
# Linux KVM prerequisites: rustup, musl-tools, build-essential, qemu-system-x86.
# BOUCHAUD_DEDICATED_HOST=YES bash tools/ci/run_p13_campaigns.sh FULL_SHA [NEW_OUT]
set -Eeuo pipefail
cd "$(dirname "$0")/../.."
SHA=${1:?usage: run_p13_campaigns.sh FULL_SHA [NEW_OUT]}
[[ "$SHA" =~ ^[0-9a-f]{40}$ ]] || { echo SHA_COMPLET_REQUIS >&2; exit 2; }
[[ $(git rev-parse HEAD) == "$SHA" ]] || { echo SHA_DIFFERENT >&2; exit 2; }
[[ -z $(git status --porcelain) ]] || { echo SOURCES_NON_PROPRES >&2; exit 2; }
OUT=${2:-"$PWD/target/p13-campaigns-${SHA:0:12}-$(date -u +%Y%m%dT%H%M%S)"}
OUT=$(realpath -m "$OUT")
[[ ! -e "$OUT" ]] || { echo "PREUVES_EXISTANTES $OUT" >&2; exit 2; }
mkdir -p "$OUT"
python3 tools/ci/p13_kvm_host_preflight.py --min-cpus 4 --output "$OUT/host.json"
for outil in rustup cargo musl-gcc; do
  command -v "$outil" >/dev/null || { echo "OUTIL_ABSENT $outil" >&2; exit 2; }
done
# A separate detached worktree prevents edits in the development checkout
# from changing a campaign. Keep it with the evidence, never reset sources.
git worktree add --detach "$OUT/source" "$SHA"
cd "$OUT/source"
CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4} bash tools/ci/build_kernel.sh > "$OUT/build.log" 2>&1
BOOT="$PWD/target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin"
sha256sum "$BOOT" > "$OUT/image.sha256"
result=0
for campagne in 1 2 3; do
  [[ $(git rev-parse HEAD) == "$SHA" && -z $(git status --porcelain) ]] || { echo CANDIDAT_MODIFIE >&2; exit 2; }
  sha256sum -c "$OUT/image.sha256" || exit 2
  status=0
  P13_QUALIFICATION_OUT="$OUT/campagne-$campagne" \
    bash tools/ci/run_p13_dedicated_kvm.sh "$BOOT" baseline > "$OUT/campagne-$campagne.log" 2>&1 || status=$?
  printf 'campagne=%s sha=%s exit=%s\n' "$campagne" "$SHA" "$status" | tee -a "$OUT/resultats.txt"
  if ((status != 0)); then result=1; fi
done
[[ $(git rev-parse HEAD) == "$SHA" && -z $(git status --porcelain) ]] || result=1
sha256sum -c "$OUT/image.sha256" || result=1
python3 - "$OUT" <<'PY' || result=1
from pathlib import Path
import sys
root = Path(sys.argv[1])
failed = False
expected = [f'attribution-smp{s}.strict.log' for s in (1, 2, 4)] + [f'ab-smp{s}.strict.log' for s in (1, 4)]
with (root / 'echecs.txt').open('w') as out:
    for campaign in range(1, 4):
        for name in expected:
            path = root / f'campagne-{campaign}' / name
            if not path.is_file() or not any(line.startswith('P13_STRICT_OK ') for line in path.read_text(errors='replace').splitlines()):
                failed = True
                out.write(f'PREUVE_STRICTE_ABSENTE {path.relative_to(root)}\n')
    for path in sorted(root.glob('campagne-*/*.strict.log')):
        text = path.read_text(errors='replace')
        if 'P13_STRICT_OK' not in text:
            out.write(str(path.relative_to(root)) + '\n' + text + '\n')
    for path in sorted(root.glob('campagne-*.log')):
        text = path.read_text(errors='replace')
        if 'verdict=QUALIFIED' not in text:
            failed = True
            out.write(str(path.relative_to(root)) + '\n' + text + '\n')
sys.exit(1 if failed else 0)
PY
printf 'P13_THREE_CAMPAIGNS sha=%s exit=%s evidence=%s\n' "$SHA" "$result" "$OUT"
exit "$result"
