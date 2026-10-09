#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
checks = {
    ROOT / "tools/ladybird/prepare-echange-processus.py": [
        "BOUCHAUD_P13_LIFECYCLE_CLOSURE_V4",
        "bouchaud_destroy_top_level_traversable_after_document_destruction",
        "m_html_parser_end_state->cancel()",
        "PAGE_DISCARD_DOCUMENT_DESTROY_BEGIN",
        "PAGE_DISCARD_DOCUMENT_DESTROY_END",
        "PAGE_DISCARD_END",
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
print("P13_LIFECYCLE_STATIC_OK V4 context_before_unload=1 callback_rooted=1 gc_hashmap_alias_fixed=1")
