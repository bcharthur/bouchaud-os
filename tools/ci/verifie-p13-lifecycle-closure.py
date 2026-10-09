#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
checks = {
    ROOT / "tools/ladybird/prepare-echange-processus.py": [
        "BOUCHAUD_P13_LIFECYCLE_CLOSURE_V3",
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

print("P13_LIFECYCLE_STATIC_OK order=document_destroy_before_pagehost_remove parser_end_cancel=1 gc_summary_redundant=1")
