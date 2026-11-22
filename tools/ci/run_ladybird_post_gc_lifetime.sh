#!/usr/bin/env bash
# Execute against an isolated, writable checkout of the pinned Ladybird sources.
set -euo pipefail
if (( $# != 1 )); then
    echo "Usage: $0 LADYBIRD_TEST_CHECKOUT" >&2
    exit 2
fi
root=$(cd "$(dirname "$0")/../.." && pwd)
source=$(realpath -e "$1")
pin=cdfe5f858eb5fc64a8d9d3fcc247d71b03fbd1f6
if [[ $(git -C "$source" rev-parse HEAD) != "$pin" ]]; then
    echo "Expected pinned Ladybird revision $pin" >&2
    exit 2
fi
suite="$source/Tests/LibGC"
test_source="$suite/TestLadybirdPostGCLifetime.cpp"
if [[ -e "$test_source" ]] || ! git -C "$source" diff --quiet HEAD -- Tests/LibGC; then
    echo 'Use a clean, disposable Ladybird test checkout.' >&2
    exit 2
fi
backup=$(mktemp)
cp "$suite/CMakeLists.txt" "$backup"
cleanup() {
    cp "$backup" "$suite/CMakeLists.txt"
    rm -f -- "$backup" "$test_source"
}
trap cleanup EXIT
cp "$root/tools/ci/TestLadybirdPostGCLifetime.cpp" "$test_source"
printf '\nladybird_test(TestLadybirdPostGCLifetime.cpp LibGC LIBS LibGC)\n' >> "$suite/CMakeLists.txt"
cd "$source"
python3 Meta/ladybird.py build --preset Release TestLadybirdPostGCLifetime
ctest --preset Release --test-dir Build/release --output-on-failure -R '^TestLadybirdPostGCLifetime$' --no-tests=error
