#!/usr/bin/env bash
set -euo pipefail

# Migration guard: canonical geometry/visibility may only be mutated by the
# policy implementation or the two explicitly-audited runtime adapters.
violations="$({ rg -n '\b(w|top|window)\.(x|y|w|h|min|placement|restore_rect)\s*(=|\+=|-=)' \
    src/gui -g '*.rs' || true; } | rg -v \
    'src/gui/(windowing/(manager|state)\.rs|window_manager\.rs|window\.rs):' || true)"
if [[ -n "$violations" ]]; then
    printf '%s\n' 'Window state mutation outside an audited adapter:' "$violations" >&2
    exit 1
fi
printf '%s\n' 'window state mutation guard: ok'
