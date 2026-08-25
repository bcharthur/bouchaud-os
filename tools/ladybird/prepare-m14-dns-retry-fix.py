#!/usr/bin/env python3
# Bouchaud OS / Ladybird M14 DNS retry fix.
#
# Executed against the disposable pinned Ladybird worktree after the existing
# M9/M13 preparation passes. It fixes one concrete LibDNS retry bug exposed by
# M13: when the 1 s retry timer fired, lookup() saw the in-flight cache entry
# and returned its existing Promise before reaching the retransmission branch.
# Result: one UDP query was sent, then the request could hang until the outer
# 300 s CI watchdog.
#
# M14 keeps normal cache joining for ordinary callers, but a repeating lookup
# is allowed to continue to the existing retransmission path. The same DNS ID
# is reused, times_repeated is incremented, the datagram is sent again and the
# timer is re-armed. Existing LibDNS timeout semantics remain unchanged.

from pathlib import Path
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: prepare-m14-dns-retry-fix.py <ladybird-worktree>")

root = Path(sys.argv[1])
resolver_h = root / "Libraries/LibDNS/Resolver.h"

if not resolver_h.is_file():
    raise SystemExit(f"M14 DNS retry fix: missing {resolver_h}")

data = resolver_h.read_text()


def replace_required(label: str, old: str, new: str) -> None:
    global data
    if new in data:
        return
    if old not in data:
        raise SystemExit(
            f"M14 DNS retry fix: upstream pattern changed ({label}) in {resolver_h}"
        )
    data = data.replace(old, new, 1)


# Keep the marker self-contained if M14 is ever run without M13.
if "#include <AK/Debug.h>" not in data:
    anchor = "#include <AK/CountingStream.h>\n"
    if anchor not in data:
        raise SystemExit("M14 DNS retry fix: AK include anchor changed upstream")
    data = data.replace(anchor, anchor + "#include <AK/Debug.h>\n", 1)
if "#include <cstdlib>" not in data:
    first_include = data.index("#include ")
    data = data[:first_include] + "#include <cstdlib>\n" + data[first_include:]


# The bug: the retry callback re-enters lookup() with repeating_lookup set.
# Because the LookupResult is still in cache and still has a pending Promise,
# this block returned that Promise immediately. The code below that reuses the
# DNS id and increments times_repeated was therefore unreachable for retries.
replace_required(
    "retry must not join pending cache promise",
    '''        Optional<u16> cached_result_id;
        if (already_in_cache) {''',
    '''        Optional<u16> cached_result_id;
        if (already_in_cache && !options.repeating_lookup) {''',
)


# Prove that a timer-driven lookup reaches the actual retransmission branch.
replace_required(
    "retry retransmission marker",
    '''        } else if (options.repeating_lookup) {
            query.header.id = options.repeating_lookup->id;
            options.repeating_lookup->times_repeated++;
        } else {''',
    '''        } else if (options.repeating_lookup) {
#if defined(BOUCHAUD_PORT)
            if (getenv("BOUCHAUD_M9") != nullptr)
                outln("[ladybird-bouchaud] M14_DNS_RETRY_TX id={} attempt={} name={}", options.repeating_lookup->id, options.repeating_lookup->times_repeated + 1, name);
#endif
            query.header.id = options.repeating_lookup->id;
            options.repeating_lookup->times_repeated++;
        } else {''',
)

resolver_h.write_text(data)
print("Bouchaud M14 DNS retry fix applied to", resolver_h)
