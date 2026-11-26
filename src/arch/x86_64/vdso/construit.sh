#!/usr/bin/env bash
# BOUCHAUD_VDSO_V1 -- reconstruit vdso.so a partir de vdso.c et vdso.lds.
# Le noyau embarque vdso.so (include_bytes!) : la construction du noyau ne
# depend pas d'un compilateur C. Apres toute modification des sources :
#   src/arch/x86_64/vdso/construit.sh
# puis committer vdso.so et vdso.so.sources (empreinte verifiee par
# tools/verifie-vdso.py).
set -euo pipefail
cd "$(dirname "$0")"
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
gcc -O2 -fPIC -fno-stack-protector -fno-builtin -ffreestanding -nostdlib \
    -fno-asynchronous-unwind-tables -fcf-protection=none -mno-red-zone \
    -c vdso.c -o "$T/vdso.o"
gcc -nostdlib -shared -fPIC -Wl,-T,vdso.lds -Wl,--hash-style=both -Wl,-Bsymbolic \
    -Wl,-soname=linux-vdso.so.1 -Wl,--no-undefined -Wl,-z,max-page-size=4096 \
    -Wl,--build-id=none -Wl,-z,noexecstack "$T/vdso.o" -o "$T/vdso.full.so"
objcopy --strip-debug --strip-unneeded "$T/vdso.full.so" vdso.so
sha256sum vdso.c vdso.lds | awk '{print $1"  "$2}' > vdso.so.sources
ls -l vdso.so
