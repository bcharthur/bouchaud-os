#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."

printf '\n=== Bouchaud CI: bootimage ===\n'
rustup show active-toolchain || rustup toolchain install
rustc --version
cargo --version

if ! command -v cargo-bootimage >/dev/null 2>&1; then
  echo "cargo-bootimage absent: installation"
  cargo install bootimage --locked
fi

cargo bootimage
BOOT="target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin"
test -s "$BOOT"
sha256sum "$BOOT"
stat -c 'bootimage: %s octets' "$BOOT"
