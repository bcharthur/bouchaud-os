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

# The response callback for a navigation using UseParallelQueue::Yes is not run
# inline. It is queued into HTML::ParallelQueue, which in Ladybird is a unique
# HTML task source serviced by the main HTML event loop. Trace that exact hand-
# off so a stalled M9 cannot be mistaken for a RequestServer/body problem.
local_navigable_cpp = root / "Libraries/LibWeb/HTML/LocalNavigable.cpp"
nav_data = local_navigable_cpp.read_text()
nav_old = '''        auto process_response = [state_holder](GC::Ref<Fetch::Infrastructure::Response> fetch_response) {
            // 1. Set response to fetchedResponse.
            state_holder->response = fetch_response;
            VERIFY(state_holder->continuation_steps);
            state_holder->continuation_steps->function()(NavigationParamsFetchStateHolder::ContinuationReason::GotResponse);
        };'''
nav_new = '''        auto process_response = [state_holder](GC::Ref<Fetch::Infrastructure::Response> fetch_response) {
#if defined(BOUCHAUD_PORT)
            if (getenv("BOUCHAUD_M9") != nullptr)
                outln("[ladybird-bouchaud] M9_NAV_PROCESS_RESPONSE_TASK_RUN");
#endif
            // 1. Set response to fetchedResponse.
            state_holder->response = fetch_response;
            VERIFY(state_holder->continuation_steps);
            state_holder->continuation_steps->function()(NavigationParamsFetchStateHolder::ContinuationReason::GotResponse);
        };'''
if "M9_NAV_PROCESS_RESPONSE_TASK_RUN" not in nav_data:
    if nav_old not in nav_data:
        raise SystemExit("M9 navigation: LocalNavigable process_response block changed upstream")
    nav_data = nav_data.replace(nav_old, nav_new, 1)
if "<cstdlib>" not in nav_data:
    first_include = nav_data.index("#include ")
    nav_data = nav_data[:first_include] + "#include <cstdlib>\n" + nav_data[first_include:]
local_navigable_cpp.write_text(nav_data)

# ParallelQueue::enqueue() feeds TaskQueue::add(), which arms a zero-delay
# single-shot Platform::Timer. Seeing ENQUEUE without TASK_RUN narrows the fault
# to task scheduling rather than networking.
task_cpp = root / "Libraries/LibWeb/HTML/EventLoop/Task.cpp"
task_data = task_cpp.read_text()
task_old = '''TaskID ParallelQueue::enqueue(GC::Ref<GC::Function<void()>> algorithm)
{
    auto& event_loop = HTML::main_thread_event_loop();
    auto task = HTML::Task::create(m_task_source.source, nullptr, algorithm);
    event_loop.task_queue().add(task);
    return task->id();
}'''
task_new = '''TaskID ParallelQueue::enqueue(GC::Ref<GC::Function<void()>> algorithm)
{
#if defined(BOUCHAUD_PORT)
    if (getenv("BOUCHAUD_M9") != nullptr)
        outln("[ladybird-bouchaud] M9_PARALLEL_TASK_ENQUEUE");
#endif
    auto& event_loop = HTML::main_thread_event_loop();
    auto task = HTML::Task::create(m_task_source.source, nullptr, algorithm);
    event_loop.task_queue().add(task);
    return task->id();
}'''
if "M9_PARALLEL_TASK_ENQUEUE" not in task_data:
    if task_old not in task_data:
        raise SystemExit("M9 navigation: ParallelQueue::enqueue block changed upstream")
    task_data = task_data.replace(task_old, task_new, 1)
if "<cstdlib>" not in task_data:
    first_include = task_data.index("#include ")
    task_data = task_data[:first_include] + "#include <cstdlib>\n#include <AK/Format.h>\n" + task_data[first_include:]
task_cpp.write_text(task_data)

# Finally trace the zero-delay timer that services the HTML task queue. The
# markers are M9-only and do not alter timer state or queue ordering.
event_loop_cpp = root / "Libraries/LibWeb/HTML/EventLoop/EventLoop.cpp"
event_data = event_loop_cpp.read_text()
event_old = '''void EventLoop::schedule()
{
    if (!m_system_event_loop_timer) {
        m_system_event_loop_timer = Platform::Timer::create_single_shot(GC::Heap::the(), 0, GC::create_function(GC::Heap::the(), [this] {
            process();
        }));
    }

    if (!m_system_event_loop_timer->is_active())
        m_system_event_loop_timer->restart();
}'''
event_new = '''void EventLoop::schedule()
{
    if (!m_system_event_loop_timer) {
        m_system_event_loop_timer = Platform::Timer::create_single_shot(GC::Heap::the(), 0, GC::create_function(GC::Heap::the(), [this] {
#if defined(BOUCHAUD_PORT)
            if (getenv("BOUCHAUD_M9") != nullptr)
                outln("[ladybird-bouchaud] M9_HTML_TIMER_FIRED");
#endif
            process();
        }));
    }

    if (!m_system_event_loop_timer->is_active()) {
#if defined(BOUCHAUD_PORT)
        if (getenv("BOUCHAUD_M9") != nullptr)
            outln("[ladybird-bouchaud] M9_HTML_TIMER_RESTART");
#endif
        m_system_event_loop_timer->restart();
    }
}'''
if "M9_HTML_TIMER_FIRED" not in event_data:
    if event_old not in event_data:
        raise SystemExit("M9 navigation: HTML EventLoop::schedule block changed upstream")
    event_data = event_data.replace(event_old, event_new, 1)
if "<cstdlib>" not in event_data:
    first_include = event_data.index("#include ")
    event_data = event_data[:first_include] + "#include <cstdlib>\n#include <AK/Format.h>\n" + event_data[first_include:]
event_loop_cpp.write_text(event_data)

print("Bouchaud M9 local navigation response adaptation applied to", root)
