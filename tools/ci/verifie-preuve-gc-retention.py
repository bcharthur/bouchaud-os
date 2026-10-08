#!/usr/bin/env python3
"""Tests positifs et negatifs du parseur de retention GC V2."""
import importlib.util
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("analyse_gc_retention", HERE / "analyse_gc_retention.py")
MOD = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = MOD
SPEC.loader.exec_module(MOD)


def state(pid: int, seq: int, roots: int, created: int, detached: int, finalized: int) -> str:
    return f"[LB:PAGE_STATE] pid={pid} seq={seq} roots={roots} created={created} detached={detached} finalized={finalized} END"


def summary(pid: int, detached: int, active: int, pageclients: int, retained: int, paths: int) -> str:
    return f"[LB:GC_RETENTION] pid={pid} detached={detached} active={active} pageclients={pageclients} retained={retained} paths={paths} END"


def path(pid: int, detached: int, n: int, root: str = "StackPointer", label: str = "WebContent::close_tab PageClient.cpp:1") -> str:
    return (
        f"[LB:GC_PATH] pid={pid} detached={detached} target={1000+n} root={root} "
        f"frame=2 frame_label={label} depth=3 path=Window>Page>PageClient END"
    )


def good(retained: bool = True) -> str:
    lines = [state(19, 1, 1, 1, 0, 0), state(19, 20, 1, 11, 10, 0)]
    if retained:
        lines.append(summary(19, 10, 1, 11, 10, 11))
        lines.extend(path(19, 10, i, "StackPointer" if i < 10 else "Root PageHost x:1") for i in range(11))
    else:
        lines.append(summary(19, 10, 1, 1, 0, 1))
        lines.append(path(19, 10, 0, "Root PageHost x:1", ""))
    lines.append(state(19, 40, 1, 21 if retained else 1, 20, 0 if retained else 20))
    if retained:
        lines.append(summary(19, 20, 1, 21, 20, 21))
        lines.extend(path(19, 20, i, "ConservativeVector" if i < 20 else "Root PageHost x:1") for i in range(21))
    else:
        lines.append(summary(19, 20, 1, 1, 0, 1))
        lines.append(path(19, 20, 0, "Root PageHost x:1", ""))
    return "\n".join(lines)


def expect(code: int, text: str, name: str) -> None:
    got, report = MOD.analyse(text)
    if got != code:
        raise SystemExit(f"{name}: code {got}, attendu {code}: {report}")


expect(0, good(True), "retention attribuee")
expect(0, good(False), "aucune retention")
expect(2, "\n".join(good(True).splitlines()[:20]), "preuve tronquee")
expect(2, state(19, 1, 1, 1, 0, 0), "sans jalons")
corrupt = good(True).replace("retained=20 paths=21", "retained=19 paths=21")
expect(2, corrupt, "compte incoherent")
legacy = good(True).replace(" frame_label=WebContent::close_tab PageClient.cpp:1", "")
expect(2, legacy, "ancienne preuve sans label")
print("GC_RETENTION_ANALYSER_V2_TESTS_OK positifs=2 negatifs=4")
