#!/usr/bin/env python3
# M9 deliberately runs WebContent without Ladybird's Browser/UI process.
# Keep the adaptations here narrow and M9-only:
#   1. do not pause a Document response for a cross-process transfer that cannot
#      happen in M9;
#   2. locally coordinate the Push/Replace history operation needed to commit a
#      cross-document navigation, using LibWeb's existing history-step jobs.
# Normal Ladybird builds and later milestones with a real Browser coordinator
# keep the upstream paths unchanged.

from pathlib import Path
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: prepare-m9-navigation.py <ladybird-worktree>")

root = Path(sys.argv[1])


def replace_once(path: Path, old: str, new: str, label: str):
    data = path.read_text()
    if new in data:
        return data
    if old not in data:
        raise SystemExit(f"M9 navigation pattern not found ({label}) in {path}")
    data = data.replace(old, new, 1)
    path.write_text(data)
    return data


# ---------------------------------------------------------------------------
# Fetch: a normal Ladybird Document response is paused so Browser can transfer
# it to another WebContent process (or turn it into a download). M9 explicitly
# decided the navigation stays Local and has no Browser peer, so body delivery
# must remain enabled.
# ---------------------------------------------------------------------------
fetching_cpp = root / "Libraries/LibWeb/Fetch/Fetching/Fetching.cpp"
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
data = replace_once(fetching_cpp, old, new, "local Document body delivery")
if "<cstdlib>" not in data:
    first_include = data.index("#include ")
    data = data[:first_include] + "#include <cstdlib>\n" + data[first_include:]
    fetching_cpp.write_text(data)


# ---------------------------------------------------------------------------
# Session history: upstream delegates every history operation to the Browser/UI
# canonical traversable. That is correct for normal Ladybird, but M9 has no UI
# process at all. After load_document() returns, a cross-document navigation
# therefore used to stop forever at page_did_request_history_operation().
#
# For M9 Push/Replace only, run the same WebContent-side jobs the UI coordinator
# would dispatch:
#   history_operation_started -> pre_steps/finalize entry -> changing job ->
#   continuation -> complete_ui_history_operation.
# This is not a synthetic commit marker and does not skip document activation;
# it exercises LibWeb's real apply-history-step continuation and only replaces
# the absent transport/coordinator for this milestone.
# ---------------------------------------------------------------------------
traversable_cpp = root / "Libraries/LibWeb/HTML/LocalTraversableNavigable.cpp"
data = traversable_cpp.read_text()
old = '''void LocalTraversableNavigable::request_history_operation(HistoryOperationParameters parameters, HistoryOperationState state)
{
    if (parameters.has<ReloadHistoryOperationParameters>()) {
        VERIFY(!state.on_apply_complete);
        state.on_apply_complete = GC::create_function(heap(), [this](HistoryStepResult result) {
            if (result == HistoryStepResult::Applied)
                return;

            if (auto current_entry = current_session_history_entry(); current_entry && current_entry->document_state()->reload_pending()) {
                current_entry->document_state()->set_reload_pending(false);
                page().client().page_did_set_session_history_entry_document_state_reload_pending(
                    id(), current_entry->navigation_api_key(), false);
            }
        });
    }

    auto initiation_id = m_next_history_initiation_id++;
    m_history_operation_states.set(initiation_id, move(state));
    page().client().page_did_request_history_operation(initiation_id, move(parameters));
}'''
new = '''void LocalTraversableNavigable::request_history_operation(HistoryOperationParameters parameters, HistoryOperationState state)
{
    if (parameters.has<ReloadHistoryOperationParameters>()) {
        VERIFY(!state.on_apply_complete);
        state.on_apply_complete = GC::create_function(heap(), [this](HistoryStepResult result) {
            if (result == HistoryStepResult::Applied)
                return;

            if (auto current_entry = current_session_history_entry(); current_entry && current_entry->document_state()->reload_pending()) {
                current_entry->document_state()->set_reload_pending(false);
                page().client().page_did_set_session_history_entry_document_state_reload_pending(
                    id(), current_entry->navigation_api_key(), false);
            }
        });
    }

    auto initiation_id = m_next_history_initiation_id++;
    m_history_operation_states.set(initiation_id, move(state));

#if defined(BOUCHAUD_PORT)
    if (getenv("BOUCHAUD_M9") != nullptr
        && (parameters.has<PushHistoryOperationParameters>() || parameters.has<ReplaceHistoryOperationParameters>())) {
        auto run_local_m9_history_operation = [this, initiation_id](CrossProcessId navigable_id, UserNavigationInvolvement user_involvement, Bindings::NavigationType navigation_type) {
            // The UI operation id only keys WebContent's per-operation state. M9
            // has one local coordinator, so the already-unique initiation id is
            // a suitable operation id as well.
            auto operation_id = initiation_id;
            outln("[ladybird-bouchaud] M9_HISTORY_LOCAL_BEGIN op={} nav={}", operation_id, navigable_id);

            auto ready = GC::create_function(heap(), [this, operation_id, initiation_id, navigable_id, user_involvement, navigation_type](bool proceed, Optional<i32> step_override, HistoryStepResult abandon_result) {
                if (!proceed || !step_override.has_value()) {
                    outln("[ladybird-bouchaud] M9_HISTORY_LOCAL_ABORT op={} proceed={}", operation_id, proceed);
                    complete_ui_history_operation(operation_id, abandon_result, {}, initiation_id);
                    return;
                }

                auto target_step = *step_override;
                auto navigable = local_navigable_with_id(navigable_id);
                if (!navigable || navigable->has_been_destroyed()) {
                    complete_ui_history_operation(operation_id, HistoryStepResult::NoMatchingEntry, {}, initiation_id);
                    return;
                }

                auto target_entry = navigable->get_the_target_history_entry_if_present(target_step);
                if (!target_entry) {
                    complete_ui_history_operation(operation_id, HistoryStepResult::NoMatchingEntry, {}, initiation_id);
                    return;
                }

                auto target_descriptor = create_session_history_entry_descriptor(*target_entry);
                run_ui_changing_navigable_history_job(
                    operation_id,
                    navigable_id,
                    target_step,
                    move(target_descriptor),
                    user_involvement,
                    navigation_type,
                    false,
                    initiation_id,
                    GC::create_function(heap(), [this, operation_id, initiation_id, navigable_id, target_step](ChangingNavigableHistoryStepJobDisposition disposition) {
                        if (disposition == ChangingNavigableHistoryStepJobDisposition::Stale) {
                            complete_ui_history_operation(operation_id, HistoryStepResult::Applied, {}, initiation_id);
                            return;
                        }
                        if (disposition != ChangingNavigableHistoryStepJobDisposition::Ready) {
                            complete_ui_history_operation(operation_id, HistoryStepResult::NoMatchingEntry, {}, initiation_id);
                            return;
                        }

                        auto navigable = local_navigable_with_id(navigable_id);
                        if (!navigable || navigable->has_been_destroyed()) {
                            complete_ui_history_operation(operation_id, HistoryStepResult::NoMatchingEntry, {}, initiation_id);
                            return;
                        }

                        auto history_object_length_and_index = get_the_history_object_length_and_index(target_step);
                        auto entries = get_session_history_entries_for_the_navigation_api(*navigable, target_step);
                        Vector<SessionHistoryEntryDescriptor> entry_descriptors;
                        entry_descriptors.ensure_capacity(entries.size());
                        for (auto const& entry : entries)
                            entry_descriptors.unchecked_append(create_session_history_entry_descriptor(*entry));

                        apply_ui_changing_navigable_continuation(
                            operation_id,
                            navigable_id,
                            history_object_length_and_index,
                            move(entry_descriptors),
                            GC::create_function(heap(), [this, operation_id, initiation_id, target_step] {
                                outln("[ladybird-bouchaud] M9_HISTORY_LOCAL_COMMIT op={} step={}", operation_id, target_step);
                                complete_ui_history_operation(operation_id, HistoryStepResult::Applied, target_step, initiation_id);
                            }));
                    }));
            });

            handle_ui_history_operation_started(operation_id, initiation_id, ready);
        };

        if (parameters.has<PushHistoryOperationParameters>()) {
            auto const& push = parameters.get<PushHistoryOperationParameters>();
            run_local_m9_history_operation(push.navigable_id, push.user_involvement, Bindings::NavigationType::Push);
        } else {
            auto const& replace = parameters.get<ReplaceHistoryOperationParameters>();
            run_local_m9_history_operation(replace.navigable_id, replace.user_involvement, Bindings::NavigationType::Replace);
        }
        return;
    }
#endif

    page().client().page_did_request_history_operation(initiation_id, move(parameters));
}'''
if new not in data:
    if old not in data:
        raise SystemExit(f"M9 navigation pattern not found (local history coordinator) in {traversable_cpp}")
    data = data.replace(old, new, 1)

if "<cstdlib>" not in data:
    first_include = data.index("#include ")
    data = data[:first_include] + "#include <cstdlib>\n" + data[first_include:]

traversable_cpp.write_text(data)

print("Bouchaud M9 local navigation/history adaptations applied to", root)
