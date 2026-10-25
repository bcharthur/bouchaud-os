#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
mode="${1:-host}"
case "$mode" in host|build) ;; *) echo "usage: $0 [host|build]" >&2; exit 2;; esac
python3 -m unittest -v tools/ci/reliability/test_p13_irq_prefilter.py
python3 tools/verifie-preemption-noyau.py
bash tools/smp/test-irq-deadline-prefilter.sh
if [[ "$mode" == build ]]; then
    cargo build
    cargo build --target targets/x86_64-bouchaud_os_uefi.json \
        --no-default-features --features uefi-boot,reference-desktop
    cargo build --manifest-path tools/reference/uefi-preboot-probe/Cargo.toml \
        --target x86_64-unknown-uefi
    cargo bootimage
    BOOT='target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin'
    test -s "$BOOT"
    sha256sum "$BOOT"
fi
