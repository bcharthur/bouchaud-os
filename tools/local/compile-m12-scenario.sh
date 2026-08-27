#!/usr/bin/env bash
set -euo pipefail

ROOT="${1:-$(pwd)}"
ROOT="$(cd "$ROOT" && pwd)"
TARGET="$ROOT/target/x86_64-unknown-linux-gnu/release"
SOURCE="$ROOT/tests/ladybird-native-http/src/main.rs"
OUTPUT="$ROOT/target/test-ladybird-native-http"
RLIB="$TARGET/libbouchaud_userland.rlib"
KERNEL="$TARGET/bouchaud"
USERLAND="$TARGET/bouchaud_userland"

for required in "$SOURCE" "$RLIB" "$KERNEL" "$USERLAND"; do
    if [[ ! -e "$required" ]]; then
        echo "Missing required file: $required" >&2
        exit 2
    fi
done

export CARGO_BIN_EXE_bouchaud="$KERNEL"
export CARGO_BIN_EXE_bouchaud_userland="$USERLAND"

rustc --edition=2021 --crate-type=bin -C opt-level=2 \
  --extern "bouchaud_userland=$RLIB" \
  "$SOURCE" \
  -o "$OUTPUT"

echo "M12 scenario compiled: $OUTPUT"
