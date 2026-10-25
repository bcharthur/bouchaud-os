#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
: "\${RUSTC:=rustc}"
BIN="$(mktemp "\${TMPDIR:-/tmp}/p13-irq-prefilter.XXXXXX")"
trap 'rm -f "$BIN"' EXIT
"$RUSTC" --edition=2021 --test -O tools/smp/test_irq_deadline_prefilter.rs -o "$BIN"
"$BIN"
bash tools/smp/test-echeances.sh
"$RUSTC" --edition=2021 --test -O tools/smp/test_reveil_precis.rs -o "$BIN"
"$BIN"
