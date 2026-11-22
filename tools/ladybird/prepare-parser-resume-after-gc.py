#!/usr/bin/env python3
"""Defer parser wakeups requested by resource finalizers until after GC."""

from pathlib import Path
import sys


def replace_once(path: Path, before: str, after: str) -> None:
    source = path.read_text(encoding="utf-8")
    if source.count(after) == 1:
        return
    if source.count(before) != 1:
        raise SystemExit(f"Unexpected Ladybird source in {path}")
    path.write_text(source.replace(before, after, 1), encoding="utf-8")


root = Path(sys.argv[1])
replace_once(
    root / "Libraries/LibGC/Heap.h",
    "    bool is_collecting_everything() const",
    "    bool is_collecting_garbage() const { return m_collecting_garbage; }\n"
    "    bool is_collecting_everything() const",
)
replace_once(
    root / "Libraries/LibWeb/DOM/Document.cpp",
    "void Document::schedule_html_parser_end_check()\n{\n",
    """void Document::schedule_html_parser_end_check()
{
    // Resource finalizers release their load-event delay during the mark
    // epilogue. Both parser checks can allocate GC cells when scheduled.
    if (auto& document_heap = heap(); document_heap.is_collecting_garbage()) {
        if (!document_heap.is_collecting_everything()) {
            document_heap.enqueue_post_gc_task([document = GC::Weak(*this)] {
                if (document)
                    document->schedule_html_parser_end_check();
            });
        }
        return;
    }

""",
)
