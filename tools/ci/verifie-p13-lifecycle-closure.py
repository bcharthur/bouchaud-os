#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
checks = {
    ROOT / "tools/ladybird/prepare-echange-processus.py": [
        "BOUCHAUD_P13_LIFECYCLE_CLOSURE_V6",
        "bouchaud_destroy_top_level_traversable_after_document_destruction",
        "m_html_parser_end_state->cancel()",
        "PAGE_DISCARD_DOCUMENT_DESTROY_BEGIN",
        "PAGE_DISCARD_DOCUMENT_DESTROY_END",
        "PAGE_DISCARD_END",
        "PAGE_DISCARD_CLIENT_RELEASED",
        "BOUCHAUD_P13_PAGECLIENT_DISCARD_V5",
    ],
    ROOT / "tools/ladybird/prepare-gc-retention-proof.py": [
        "BOUCHAUD_P13_GC_RETENTION_V3",
        "[LB:GC_SUMMARY]",
        "copy < 3",
    ],
    ROOT / "tools/ci/analyse_gc_retention.py": [
        "LB:GC_SUMMARY",
        "resumes_redondants=2plus",
    ],
    ROOT / "tools/ci/run_ladybird_memoire.sh": [
        'if [ "${BO_GC_RETENTION_PROOF:-0}" = 1 ]; then',
        "preuve GC retention non demandee sur ce run de mesure",
    ],
    ROOT / ".github/workflows/ladybird-native-browser.yml": [
        "python3 tools/ci/verifie-p13-lifecycle-closure.py",
    ],
}
for path, markers in checks.items():
    text = path.read_text(encoding="utf-8")
    for marker in markers:
        if marker not in text:
            raise SystemExit(f"P13_LIFECYCLE_STATIC_FAIL {path}: marqueur absent: {marker}")

prep = (ROOT / "tools/ladybird/prepare-echange-processus.py").read_text(encoding="utf-8")
if prep.count('traversable->destroy_top_level_traversable();') != 0:
    raise SystemExit("P13_LIFECYCLE_STATIC_FAIL ancien destroy direct encore present dans le preparateur")

# V4 : refuse une capture tardive du contexte et les references invalidees dans le BFS.
context_capture = '    auto browsing_context = traversable->active_browsing_context();'
unload = 'document->unload_a_document_and_its_descendants'
if prep.count(context_capture) != 1 or prep.index(context_capture) > prep.index(unload):
    raise SystemExit("P13_LIFECYCLE_STATIC_FAIL BrowsingContext non capture avant unload")
required = [
    'bouchaud_destroy_top_level_traversable_after_document_destruction(GC::Ptr<BrowsingContext> browsing_context)',
    '[traversable, browsing_context]',
    'PAGE_DISCARD_BROWSING_CONTEXT_REMOVED',
]
for marker in required:
    if marker not in prep:
        raise SystemExit(f"P13_LIFECYCLE_STATIC_FAIL argument/callback manquant: {marker}")
if '    auto browsing_context = active_browsing_context();' in prep:
    raise SystemExit("P13_LIFECYCLE_STATIC_FAIL lookup du BrowsingContext apres unload")
gc_prep = (ROOT / "tools/ladybird/prepare-gc-retention-proof.py").read_text(encoding="utf-8")
for marker in [
    'parent.ensure_capacity(nodes.size());',
    'root_label.ensure_capacity(nodes.size());',
    'root_frame.ensure_capacity(nodes.size());',
    'queue.ensure_capacity(nodes.size());',
    'String label_copy = *label;',
    'auto frame_copy = *frame;',
    'root_label.set(edge, label_copy);',
    'root_frame.set(edge, frame_copy);',
]:
    if marker not in gc_prep:
        raise SystemExit(f"P13_LIFECYCLE_STATIC_FAIL BFS alias/capacite: {marker}")
if 'root_label.set(edge, *label)' in gc_prep or 'root_frame.set(edge, *frame)' in gc_prep:
    raise SystemExit("P13_LIFECYCLE_STATIC_FAIL BFS garde des references invalidables")
# V5 : checks that the already-dead PageClient edge is removed after all close bookkeeping.
page_close = 'discarded_page.client().page_did_close_top_level_traversable();'
remove_navigables = 'remove_from_all_local_navigables();'
release_client = 'discarded_page.bouchaud_release_client_after_discard();'
if not (page_close in prep and remove_navigables in prep and release_client in prep):
    raise SystemExit("P13_LIFECYCLE_STATIC_FAIL V5 page discard hook incomplete")
if not (prep.index(page_close) < prep.index(remove_navigables) < prep.index(release_client)):
    raise SystemExit("P13_LIFECYCLE_STATIC_FAIL V5 closes PageClient before compositor teardown")
for marker in [
    'GC::Ptr<PageClient> m_client;',
    'VERIFY(m_client); return *m_client;',
    'void Page::bouchaud_release_client_after_discard()',
    'm_client = nullptr;',
]:
    if marker not in prep:
        raise SystemExit(f"P13_LIFECYCLE_STATIC_FAIL V5 detach invariant: {marker}")
if 'GC::Ref<PageClient> m_client;' not in prep:
    raise SystemExit("P13_LIFECYCLE_STATIC_FAIL V5 weak-edge replacement anchor absent")
print("P13_LIFECYCLE_STATIC_OK V6 context_before_unload=1 pageclient_edge_detached_after_teardown=1 gc_hashmap_alias_fixed=1")
# V6 : verify both ways the old document console retains its PageClient
# are severed when (and only when) Document::destroy runs.
for marker in [
    "BOUCHAUD_P13_CONSOLE_LIFECYCLE_V6",
    "void bouchaud_clear_client_if(ConsoleClient const* client)",
    "if (m_client.ptr() == client)",
    "console_object->console().bouchaud_clear_client_if(m_console_client.ptr());",
    "m_console_client = nullptr;",
    "#include <LibJS/Runtime/ConsoleObject.h>",
]:
    if marker not in prep:
        raise SystemExit(f"P13_LIFECYCLE_STATIC_FAIL V6 ancien client console non detache: {marker}")
# The preparation uses the upstream AD-HOC anchor directly following
# run_unloading_cleanup_steps() in pinned Document::destroy(). The build
# checks the exact upstream anchor and will fail closed if it changes.
destroy_anchor = '"    // AD-HOC: Destruction does not go through did_stop_being_active_document_in_navigable(),'
if prep.count(destroy_anchor) != 2:
    raise SystemExit("P13_LIFECYCLE_STATIC_FAIL V6 ancien et nouvel anchor Document::destroy non conserves")
print("P13_CONSOLE_LIFECYCLE_STATIC_OK after_unload_cleanup=1 console_client_cleared=1 existing_pageclient_teardown=1")
# V9: decommit is asynchronous and coalesced at a fixed backlog, not GC-forcing.
for token in [
    "BOUCHAUD_P13_DECOMMIT_BACKLOG_V9",
    "m_freshly_freed.size() >= 32",
    "m_freshly_freed.size() % 32 == 0",
    "if (kick_for_backlog)",
    "DecommitWorker::the().kick();",
]:
    if token not in prep:
        raise SystemExit(f"P13_DECOMMIT_STATIC_FAIL missing {token}")
if "collect_garbage(CollectionType::CollectEverything)" in prep:
    raise SystemExit("P13_DECOMMIT_STATIC_FAIL GC force interdit")
print("P13_DECOMMIT_STATIC_OK asynchronous=1 backlog_blocks=32 no_forced_gc=1")


