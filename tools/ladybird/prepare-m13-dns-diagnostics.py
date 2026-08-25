#!/usr/bin/env python3
# Bouchaud OS / Ladybird M13 DNS diagnostics.
#
# Executed against the disposable pinned Ladybird worktree after
# prepare-m9-diagnostics.py. It only instruments that temporary worktree.
#
# Goal: locate the exact boundary of the current DNS hang:
#   - resolver entry / socket creation
#   - DNS packet serialization
#   - write_until_depleted()
#   - retry timer
#   - LibCore notifier
#   - readiness / recv / DNS parsing
#   - lookup promise resolution

from pathlib import Path
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: prepare-m13-dns-diagnostics.py <ladybird-worktree>")

root = Path(sys.argv[1])
resolver_h = root / "Libraries/LibDNS/Resolver.h"

if not resolver_h.is_file():
    raise SystemExit(f"M13 DNS diagnostics: missing {resolver_h}")

data = resolver_h.read_text()


def replace_required(label: str, old: str, new: str) -> None:
    global data
    if new in data:
        return
    if old not in data:
        raise SystemExit(
            f"M13 DNS diagnostics: upstream pattern changed ({label}) in {resolver_h}"
        )
    data = data.replace(old, new, 1)


# Keep the instrumentation self-contained instead of relying on transitive
# includes. AK::Debug provides outln(), and cstdlib provides getenv().
if "#include <AK/Debug.h>" not in data:
    data = data.replace(
        "#include <AK/CountingStream.h>\n",
        "#include <AK/CountingStream.h>\n#include <AK/Debug.h>\n",
        1,
    )
if "#include <cstdlib>" not in data:
    first_include = data.index("#include ")
    data = data[:first_include] + "#include <cstdlib>\n" + data[first_include:]


# 1. Entering the configured asynchronous resolver path.
replace_required(
    "async lookup entry",
    '''        lookup_path = "async-query"sv;

        auto already_in_cache = false;''',
    '''        lookup_path = "async-query"sv;
#if defined(BOUCHAUD_PORT)
        if (getenv("BOUCHAUD_M9") != nullptr)
            outln("[ladybird-bouchaud] M13_DNS_LOOKUP_ASYNC name={} retry={}", name, options.repeating_lookup ? options.repeating_lookup->times_repeated : 0);
#endif

        auto already_in_cache = false;''',
)


# 2. Final resolver timeout. If this appears, LibCore timers are alive.
replace_required(
    "resolver final timeout",
    '''        if (options.repeating_lookup && options.repeating_lookup->times_repeated >= 5) {
            dbgln_if(DNS_DEBUG, "DNS: Repeating lookup for {} timed out", name);
            lookup_path = "repeat-timeout"sv;''',
    '''        if (options.repeating_lookup && options.repeating_lookup->times_repeated >= 5) {
            dbgln_if(DNS_DEBUG, "DNS: Repeating lookup for {} timed out", name);
#if defined(BOUCHAUD_PORT)
            if (getenv("BOUCHAUD_M9") != nullptr)
                outln("[ladybird-bouchaud] M13_DNS_TIMEOUT id={} name={} retries={}", options.repeating_lookup->id, name, options.repeating_lookup->times_repeated);
#endif
            lookup_path = "repeat-timeout"sv;''',
)


# 3. Retry timer callback.
replace_required(
    "retry timer callback",
    '''                  p->repeat_timer->on_timeout = [=, this] {
                      (void)lookup(name, class_, desired_types, { .validate_dnssec_locally = options.validate_dnssec_locally, .repeating_lookup = p });
                  };''',
    '''                  p->repeat_timer->on_timeout = [=, this] {
#if defined(BOUCHAUD_PORT)
                      if (getenv("BOUCHAUD_M9") != nullptr)
                          outln("[ladybird-bouchaud] M13_DNS_TIMER_FIRED id={} name={} retries={}", p->id, name, p->times_repeated);
#endif
                      (void)lookup(name, class_, desired_types, { .validate_dnssec_locally = options.validate_dnssec_locally, .repeating_lookup = p });
                  };''',
)


# 4. Exact write boundary.
replace_required(
    "DNS wire write and timer",
    '''        auto write_result = m_socket.with_write_locked([&](auto& socket) {
            return (*socket)->write_until_depleted(query_bytes.bytes());
        });
        if (write_result.is_error()) {
            promise->reject(write_result.release_error());
            return promise;
        }

        pending_lookup->repeat_timer->start();

        return promise;''',
    '''#if defined(BOUCHAUD_PORT)
        if (getenv("BOUCHAUD_M9") != nullptr)
            outln("[ladybird-bouchaud] M13_DNS_WRITE_BEGIN id={} name={} bytes={} mode={}", query.header.id, name, query_bytes.size(), m_mode == ConnectionMode::UDP ? "udp" : "tcp");
#endif
        auto write_result = m_socket.with_write_locked([&](auto& socket) {
            return (*socket)->write_until_depleted(query_bytes.bytes());
        });
        if (write_result.is_error()) {
#if defined(BOUCHAUD_PORT)
            if (getenv("BOUCHAUD_M9") != nullptr)
                outln("[ladybird-bouchaud] M13_DNS_WRITE_ERROR id={} name={} error={}", query.header.id, name, write_result.error());
#endif
            promise->reject(write_result.release_error());
            return promise;
        }
#if defined(BOUCHAUD_PORT)
        if (getenv("BOUCHAUD_M9") != nullptr)
            outln("[ladybird-bouchaud] M13_DNS_WRITE_OK id={} name={} bytes={}", query.header.id, name, query_bytes.size());
#endif

        pending_lookup->repeat_timer->start();
#if defined(BOUCHAUD_PORT)
        if (getenv("BOUCHAUD_M9") != nullptr)
            outln("[ladybird-bouchaud] M13_DNS_TIMER_ARMED id={} name={} interval_ms=1000", query.header.id, name);
#endif

        return promise;''',
)


# 5. Socket construction.
replace_required(
    "resolver socket creation",
    '''        if (attempt_restart && !result && !m_attempting_restart) {
            TemporaryChange change(m_attempting_restart, true);
            auto create_result = m_create_socket();
            if (create_result.is_error()) {
                dbgln_if(DNS_DEBUG, "DNS: Failed to create socket: {}", create_result.error());
                return false;
            }

            auto [socket, mode] = MUST(move(create_result));
            set_socket(move(socket), mode);
            result = true;
        }''',
    '''        if (attempt_restart && !result && !m_attempting_restart) {
            TemporaryChange change(m_attempting_restart, true);
#if defined(BOUCHAUD_PORT)
            if (getenv("BOUCHAUD_M9") != nullptr)
                outln("[ladybird-bouchaud] M13_DNS_SOCKET_BEGIN");
#endif
            auto create_result = m_create_socket();
            if (create_result.is_error()) {
#if defined(BOUCHAUD_PORT)
                if (getenv("BOUCHAUD_M9") != nullptr)
                    outln("[ladybird-bouchaud] M13_DNS_SOCKET_ERROR error={}", create_result.error());
#endif
                dbgln_if(DNS_DEBUG, "DNS: Failed to create socket: {}", create_result.error());
                return false;
            }

            auto [socket, mode] = MUST(move(create_result));
#if defined(BOUCHAUD_PORT)
            if (getenv("BOUCHAUD_M9") != nullptr)
                outln("[ladybird-bouchaud] M13_DNS_SOCKET_CREATED mode={}", mode == ConnectionMode::UDP ? "udp" : "tcp");
#endif
            set_socket(move(socket), mode);
#if defined(BOUCHAUD_PORT)
            if (getenv("BOUCHAUD_M9") != nullptr)
                outln("[ladybird-bouchaud] M13_DNS_SOCKET_OK mode={}", mode == ConnectionMode::UDP ? "udp" : "tcp");
#endif
            result = true;
        }''',
)


# 6. LibCore read notifier.
replace_required(
    "socket notifier",
    '''            (*s)->on_ready_to_read = [this] {
                process_incoming_messages();
            };
            (*s)->set_notifications_enabled(true);''',
    '''            (*s)->on_ready_to_read = [this] {
#if defined(BOUCHAUD_PORT)
                if (getenv("BOUCHAUD_M9") != nullptr)
                    outln("[ladybird-bouchaud] M13_DNS_NOTIFIER_FIRED");
#endif
                process_incoming_messages();
            };
            (*s)->set_notifications_enabled(true);
#if defined(BOUCHAUD_PORT)
            if (getenv("BOUCHAUD_M9") != nullptr)
                outln("[ladybird-bouchaud] M13_DNS_NOTIFIER_ARMED");
#endif''',
)


# 7. Readiness result inside the notifier.
replace_required(
    "incoming readiness loop",
    '''    void process_incoming_messages()
    {
        while (true) {
            if (auto result = m_socket.with_read_locked([](auto& socket) {
                    return (*socket)->can_read_without_blocking();
                });
                result.is_error() || !result.value())
                break;
            auto message_or_err = parse_one_message();''',
    '''    void process_incoming_messages()
    {
#if defined(BOUCHAUD_PORT)
        if (getenv("BOUCHAUD_M9") != nullptr)
            outln("[ladybird-bouchaud] M13_DNS_PROCESS_INCOMING");
#endif
        while (true) {
            auto ready_result = m_socket.with_read_locked([](auto& socket) {
                return (*socket)->can_read_without_blocking();
            });
            if (ready_result.is_error()) {
#if defined(BOUCHAUD_PORT)
                if (getenv("BOUCHAUD_M9") != nullptr)
                    outln("[ladybird-bouchaud] M13_DNS_READY_ERROR error={}", ready_result.error());
#endif
                break;
            }
#if defined(BOUCHAUD_PORT)
            if (getenv("BOUCHAUD_M9") != nullptr)
                outln("[ladybird-bouchaud] M13_DNS_READY value={}", ready_result.value());
#endif
            if (!ready_result.value())
                break;

            auto message_or_err = parse_one_message();''',
)


# 8. DNS parse result and lookup-id matching.
replace_required(
    "parsed DNS message",
    '''            auto message = message_or_err.release_value();
            auto result = m_pending_lookups.with_write_locked([&](auto& lookups) -> ErrorOr<void> {
                auto* lookup = lookups->find(message.header.id);
                if (!lookup)
                    return Error::from_string_literal("No pending lookup found for this message");''',
    '''            auto message = message_or_err.release_value();
#if defined(BOUCHAUD_PORT)
            if (getenv("BOUCHAUD_M9") != nullptr)
                outln("[ladybird-bouchaud] M13_DNS_MESSAGE id={} answers={}", message.header.id, message.answers.size());
#endif
            auto result = m_pending_lookups.with_write_locked([&](auto& lookups) -> ErrorOr<void> {
                auto* lookup = lookups->find(message.header.id);
                if (!lookup) {
#if defined(BOUCHAUD_PORT)
                    if (getenv("BOUCHAUD_M9") != nullptr)
                        outln("[ladybird-bouchaud] M13_DNS_UNMATCHED id={}", message.header.id);
#endif
                    return Error::from_string_literal("No pending lookup found for this message");
                }''',
)


# 9. Promise resolution.
replace_required(
    "DNS promise resolution",
    '''                result->finished_request();
                lookup->promise->resolve(*result);
                lookups->remove(message.header.id);''',
    '''                result->finished_request();
#if defined(BOUCHAUD_PORT)
                if (getenv("BOUCHAUD_M9") != nullptr)
                    outln("[ladybird-bouchaud] M13_DNS_PROMISE_RESOLVE id={} addresses={}", message.header.id, result->cached_addresses().size());
#endif
                lookup->promise->resolve(*result);
                lookups->remove(message.header.id);''',
)


resolver_h.write_text(data)
print("Bouchaud M13 LibDNS diagnostics applied to", resolver_h)
