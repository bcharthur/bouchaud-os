#!/usr/bin/env python3
# Focused M9 diagnostics layered after prepare-m9-source.py.
# This file only touches the disposable pinned Ladybird worktree.

from pathlib import Path
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: prepare-m9-diagnostics.py <ladybird-worktree>")

root = Path(sys.argv[1])


def replace_once(path: Path, old: str, new: str, label: str):
    data = path.read_text()
    if new in data:
        return
    if old not in data:
        raise SystemExit(f"M9 diagnostics pattern not found ({label}) in {path}")
    path.write_text(data.replace(old, new, 1))


# ---------------------------------------------------------------------------
# LibRequests: distinguish four different states which previous markers could
# not separate:
#   1. request_finished IPC arrived;
#   2. response pipe was activated;
#   3. bytes were actually read and delivered;
#   4. the user's completion callback was finally released.
# ---------------------------------------------------------------------------
request_cpp = root / "Libraries/LibRequests/Request.cpp"
data = request_cpp.read_text()

if "M9_BODY_FINISH_SIGNAL" not in data:
    old = '''    on_finish = [this](auto total_size, auto const& timing_info, auto network_error) {
        // If the request was stopped while this IPC was in-flight, just bail.
        if (!m_internal_stream_data)
            return;

        m_internal_stream_data->total_size = total_size;
        m_internal_stream_data->network_error = network_error;
        m_internal_stream_data->timing_info = timing_info;
        m_internal_stream_data->request_done = true;
        m_internal_stream_data->on_finish();
    };'''
    new = '''    on_finish = [this](auto total_size, auto const& timing_info, auto network_error) {
        // If the request was stopped while this IPC was in-flight, just bail.
        if (!m_internal_stream_data)
            return;

#if defined(BOUCHAUD_PORT)
        if (getenv("BOUCHAUD_M9") != nullptr)
            outln("[ladybird-bouchaud] M9_BODY_FINISH_SIGNAL total={} delivered={}", total_size, m_internal_stream_data->delivered_size);
#endif
        m_internal_stream_data->total_size = total_size;
        m_internal_stream_data->network_error = network_error;
        m_internal_stream_data->timing_info = timing_info;
        m_internal_stream_data->request_done = true;
        m_internal_stream_data->on_finish();
    };'''
    if old not in data:
        raise SystemExit("M9 diagnostics: Request on_finish wrapper changed upstream")
    data = data.replace(old, new, 1)

if "M9_BODY_FINISH_GATE" not in data:
    old = '''        auto has_received_all_reported_bytes = m_internal_stream_data->request_done && m_internal_stream_data->delivered_size >= m_internal_stream_data->total_size;
        if (!m_internal_stream_data->user_finish_called && (!m_internal_stream_data->read_stream || m_internal_stream_data->read_stream->is_eof() || has_received_all_reported_bytes)) {
            m_internal_stream_data->user_finish_called = true;
            user_on_finish(m_internal_stream_data->total_size, m_internal_stream_data->timing_info, m_internal_stream_data->network_error);
        }'''
    new = '''        auto has_received_all_reported_bytes = m_internal_stream_data->request_done && m_internal_stream_data->delivered_size >= m_internal_stream_data->total_size;
#if defined(BOUCHAUD_PORT)
        if (getenv("BOUCHAUD_M9") != nullptr)
            outln("[ladybird-bouchaud] M9_BODY_FINISH_GATE done={} delivered={} total={} eof={} called={}", m_internal_stream_data->request_done, m_internal_stream_data->delivered_size, m_internal_stream_data->total_size, m_internal_stream_data->read_stream ? m_internal_stream_data->read_stream->is_eof() : true, m_internal_stream_data->user_finish_called);
#endif
        if (!m_internal_stream_data->user_finish_called && (!m_internal_stream_data->read_stream || m_internal_stream_data->read_stream->is_eof() || has_received_all_reported_bytes)) {
            m_internal_stream_data->user_finish_called = true;
#if defined(BOUCHAUD_PORT)
            if (getenv("BOUCHAUD_M9") != nullptr)
                outln("[ladybird-bouchaud] M9_BODY_USER_FINISH delivered={} total={}", m_internal_stream_data->delivered_size, m_internal_stream_data->total_size);
#endif
            user_on_finish(m_internal_stream_data->total_size, m_internal_stream_data->timing_info, m_internal_stream_data->network_error);
        }'''
    if old not in data:
        raise SystemExit("M9 diagnostics: Request finish gate changed upstream")
    data = data.replace(old, new, 1)

if "M9_BODY_READ_ERROR" not in data:
    old = '''            auto result = m_internal_stream_data->read_stream->read_some({ buffer, bytes_to_read });
            if (result.is_error() && (!result.error().is_errno() || (result.error().is_errno() && result.error().code() != EINTR)))
                break;
            if (result.is_error())
                continue;

            auto read_bytes = result.release_value();
            if (read_bytes.is_empty())
                break;

            m_internal_stream_data->delivered_size += read_bytes.size();
            m_internal_stream_data->on_data_available(ResponseData::from_bytes(read_bytes));'''
    new = '''            auto result = m_internal_stream_data->read_stream->read_some({ buffer, bytes_to_read });
            if (result.is_error() && (!result.error().is_errno() || (result.error().is_errno() && result.error().code() != EINTR))) {
#if defined(BOUCHAUD_PORT)
                if (getenv("BOUCHAUD_M9") != nullptr)
                    outln("[ladybird-bouchaud] M9_BODY_READ_ERROR code={} delivered={}", result.error().code(), m_internal_stream_data->delivered_size);
#endif
                break;
            }
            if (result.is_error())
                continue;

            auto read_bytes = result.release_value();
            if (read_bytes.is_empty()) {
#if defined(BOUCHAUD_PORT)
                if (getenv("BOUCHAUD_M9") != nullptr)
                    outln("[ladybird-bouchaud] M9_BODY_READ_EMPTY delivered={}", m_internal_stream_data->delivered_size);
#endif
                break;
            }

            m_internal_stream_data->delivered_size += read_bytes.size();
#if defined(BOUCHAUD_PORT)
            if (getenv("BOUCHAUD_M9") != nullptr)
                outln("[ladybird-bouchaud] M9_BODY_READ chunk={} delivered={}", read_bytes.size(), m_internal_stream_data->delivered_size);
#endif
            m_internal_stream_data->on_data_available(ResponseData::from_bytes(read_bytes));'''
    if old not in data:
        raise SystemExit("M9 diagnostics: Request read loop changed upstream")
    data = data.replace(old, new, 1)

request_cpp.write_text(data)


# ---------------------------------------------------------------------------
# PageClient: M9 deliberately has no Browser process. These three messages are
# DevTools/network-observability notifications only; ResourceLoader calls them
# before forwarding headers/completion into Fetch. Do not send them into the
# bootstrap socket in M9. This mirrors the existing M9 cookie/HSTS/local
# navigation policy and leaves every normal Ladybird build untouched.
# ---------------------------------------------------------------------------
page_cpp = root / "Services/WebContent/PageClient.cpp"
data = page_cpp.read_text()

patches = [
    (
        '''void PageClient::page_did_start_network_request(u64 request_id, URL::URL const& url, ByteString const& method, Vector<HTTP::Header> const& request_headers, ReadonlyBytes request_body, Optional<String> initiator_type, String const& referrer_policy, bool is_navigation_request, Web::Fetch::Infrastructure::Request::Priority priority)
{
    client().async_did_start_network_request(m_id, request_id, url, method, request_headers, request_body, move(initiator_type), referrer_policy, is_navigation_request, priority);
}''',
        '''void PageClient::page_did_start_network_request(u64 request_id, URL::URL const& url, ByteString const& method, Vector<HTTP::Header> const& request_headers, ReadonlyBytes request_body, Optional<String> initiator_type, String const& referrer_policy, bool is_navigation_request, Web::Fetch::Infrastructure::Request::Priority priority)
{
#if defined(BOUCHAUD_PORT)
    if (bouchaud_m9_enabled()) {
        outln("[ladybird-bouchaud] M9_BROWSER_NET_START_SKIPPED id={} url={}", request_id, url);
        return;
    }
#endif
    client().async_did_start_network_request(m_id, request_id, url, method, request_headers, request_body, move(initiator_type), referrer_policy, is_navigation_request, priority);
}''',
        "PageClient network start",
    ),
    (
        '''void PageClient::page_did_receive_network_response_headers(u64 request_id, u32 status_code, Optional<String> reason_phrase, Vector<HTTP::Header> const& response_headers, Requests::CameFromCache came_from_cache)
{
    client().async_did_receive_network_response_headers(m_id, request_id, status_code, move(reason_phrase), response_headers, came_from_cache);
}''',
        '''void PageClient::page_did_receive_network_response_headers(u64 request_id, u32 status_code, Optional<String> reason_phrase, Vector<HTTP::Header> const& response_headers, Requests::CameFromCache came_from_cache)
{
#if defined(BOUCHAUD_PORT)
    if (bouchaud_m9_enabled()) {
        outln("[ladybird-bouchaud] M9_BROWSER_NET_HEADERS_SKIPPED id={} status={}", request_id, status_code);
        return;
    }
#endif
    client().async_did_receive_network_response_headers(m_id, request_id, status_code, move(reason_phrase), response_headers, came_from_cache);
}''',
        "PageClient network headers",
    ),
    (
        '''void PageClient::page_did_finish_network_request(u64 request_id, u64 body_size, Requests::RequestTimingInfo const& timing_info, Optional<Requests::NetworkError> const& network_error)
{
    client().async_did_finish_network_request(m_id, request_id, body_size, timing_info, network_error);
}''',
        '''void PageClient::page_did_finish_network_request(u64 request_id, u64 body_size, Requests::RequestTimingInfo const& timing_info, Optional<Requests::NetworkError> const& network_error)
{
#if defined(BOUCHAUD_PORT)
    if (bouchaud_m9_enabled()) {
        outln("[ladybird-bouchaud] M9_BROWSER_NET_FINISH_SKIPPED id={} size={} error={}", request_id, body_size, network_error.has_value());
        return;
    }
#endif
    client().async_did_finish_network_request(m_id, request_id, body_size, timing_info, network_error);
}''',
        "PageClient network finish",
    ),
]

for old, new, label in patches:
    if new in data:
        continue
    if old not in data:
        raise SystemExit(f"M9 diagnostics pattern not found ({label}) in {page_cpp}")
    data = data.replace(old, new, 1)

page_cpp.write_text(data)

# The next adaptation is deliberately kept in its own small source-patch file:
# it changes one upstream Fetch behaviour (Document-body pausing), while this
# file remains diagnostics/observability. browser-upstream.sh already invokes
# this script, so chain the local-navigation patch here without another build
# entry point.
navigation_script = Path(__file__).with_name("prepare-m9-navigation.py")
exec(compile(navigation_script.read_text(), str(navigation_script), "exec"))

print("Bouchaud M9 body/Fetch diagnostics applied to", root)
