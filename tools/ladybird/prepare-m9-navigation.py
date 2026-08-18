#!/usr/bin/env python3
# Bouchaud M9 has no Browser process yet: WebContent is intentionally kept as
# the local navigation process. Upstream pauses Document response delivery so a
# response can be transferred to another process (or turned into a download).
# For the M9 local-only architecture there is nobody to perform that hand-off,
# so keeping the request paused can strand navigation after a complete HTTP
# response. Patch only the disposable pinned Ladybird worktree and only M9.

from pathlib import Path
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: prepare-m9-navigation.py <ladybird-worktree>")

root = Path(sys.argv[1])
fetching_cpp = root / "Libraries/LibWeb/Fetch/Fetching/Fetching.cpp"
data = fetching_cpp.read_text()

old = '''    auto keep_alive_for_transfer = request->destination() == Infrastructure::Request::Destination::Document
        ? Requests::RequestClient::KeepAliveForTransfer::Yes
        : Requests::RequestClient::KeepAliveForTransfer::No;
    auto network_request = ResourceLoader::the().load(load_request, on_headers_received, on_data_received, on_cached_body_available, on_complete, keep_alive_for_transfer);
    if (network_request && request->destination() == Infrastructure::Request::Destination::Document)
        network_request->set_body_delivery_paused(true);
    fetch_params.controller()->set_pending_request(network_request);
'''

new = '''    auto keep_alive_for_transfer = request->destination() == Infrastructure::Request::Destination::Document
        ? Requests::RequestClient::KeepAliveForTransfer::Yes
        : Requests::RequestClient::KeepAliveForTransfer::No;
    auto network_request = ResourceLoader::the().load(load_request, on_headers_received, on_data_received, on_cached_body_available, on_complete, keep_alive_for_transfer);
    if (network_request && request->destination() == Infrastructure::Request::Destination::Document) {
#if defined(BOUCHAUD_PORT)
        // Normal Ladybird pauses a navigation response here because the Browser
        // may transfer it to another WebContent process or turn it into a
        // download. M9 deliberately has no Browser process and
        // PageClient::decide_navigation_process() returns Local. There is no
        // transfer peer that could resume this body, so keep local delivery
        // enabled and let the normal Fetch/HTML navigation pipeline consume it.
        if (getenv("BOUCHAUD_M9") != nullptr) {
            outln("[ladybird-bouchaud] M9_DOCUMENT_BODY_LOCAL_UNPAUSED");
            network_request->set_body_delivery_paused(false);
        } else
#endif
        {
            network_request->set_body_delivery_paused(true);
        }
    }
    fetch_params.controller()->set_pending_request(network_request);
'''

if "M9_DOCUMENT_BODY_LOCAL_UNPAUSED" not in data:
    if old not in data:
        raise SystemExit("M9 navigation: document body pause block changed upstream")
    data = data.replace(old, new, 1)

# Trace the exact Fetch hand-off after ResourceLoader has completed. This is not
# a behavioural shortcut: it only makes the remaining boundary visible if the
# local-body fix is insufficient.
old_complete = '''    auto on_complete = GC::create_function(GC::Heap::the(), [&realm, pending_response, stream, fetched_data_receiver](bool success, Requests::RequestTimingInfo const&, Optional<StringView> error_message) {
        // FIXME: Implement on_complete timing info for unbuffered requests
        HTML::TemporaryExecutionContext execution_context { realm, HTML::TemporaryExecutionContext::CallbacksEnabled::Yes };

        if (success) {
            fetched_data_receiver->handle_network_data(realm, Requests::ResponseData::from_bytes({}), FetchedDataReceiver::NetworkState::Complete);
        } else {'''

new_complete = '''    auto on_complete = GC::create_function(GC::Heap::the(), [&realm, pending_response, stream, fetched_data_receiver](bool success, Requests::RequestTimingInfo const&, Optional<StringView> error_message) {
        // FIXME: Implement on_complete timing info for unbuffered requests
        HTML::TemporaryExecutionContext execution_context { realm, HTML::TemporaryExecutionContext::CallbacksEnabled::Yes };

#if defined(BOUCHAUD_PORT)
        if (getenv("BOUCHAUD_M9") != nullptr)
            outln("[ladybird-bouchaud] M9_FETCH_ON_COMPLETE success={}", success);
#endif
        if (success) {
            fetched_data_receiver->handle_network_data(realm, Requests::ResponseData::from_bytes({}), FetchedDataReceiver::NetworkState::Complete);
#if defined(BOUCHAUD_PORT)
            if (getenv("BOUCHAUD_M9") != nullptr)
                outln("[ladybird-bouchaud] M9_FETCH_COMPLETE_ENQUEUED");
#endif
        } else {'''

if "M9_FETCH_ON_COMPLETE" not in data:
    if old_complete not in data:
        raise SystemExit("M9 navigation: Fetch on_complete block changed upstream")
    data = data.replace(old_complete, new_complete, 1)

if "<cstdlib>" not in data:
    first_include = data.index("#include ")
    data = data[:first_include] + "#include <cstdlib>\n#include <AK/Format.h>\n" + data[first_include:]

fetching_cpp.write_text(data)
print("Bouchaud M9 local navigation response adaptation applied to", root)
