#!/bin/bash
# Build the tiny launcher used by the QEMU job.
set -euo pipefail
cd "$(dirname "$0")/../.."
OUT=third_party/native-browser-bouchaud
mkdir -p "$OUT"
CC=${CC:-cc}
$CC -O2 -static-pie -fPIE tools/ladybird/webcontent-bootstrap.c -o "$OUT/webcontent-bootstrap"
file "$OUT/webcontent-bootstrap"
