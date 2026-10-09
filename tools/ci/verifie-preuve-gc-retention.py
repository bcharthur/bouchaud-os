#!/usr/bin/env python3
from __future__ import annotations
import importlib.util
from pathlib import Path
import sys

AN = Path(__file__).with_name("analyse_gc_retention.py")
spec = importlib.util.spec_from_file_location("analyse_gc_retention", AN)
mod = importlib.util.module_from_spec(spec)
assert spec and spec.loader
sys.modules[spec.name] = mod
spec.loader.exec_module(mod)


def state(detached: int, finalized: int = 0) -> str:
    return f"[LB:PAGE_STATE] pid=19 seq={detached+1} roots=1 created={detached+1} detached={detached} finalized={finalized} END\n"


def summary(detached: int, *, unknown: int = 0, retained: int | None = None, tweak: int = 0) -> str:
    pageclients = detached + 1
    active = 1
    if retained is None:
        retained = detached
    strong = pageclients - unknown
    layout = max(0, detached - 3) + tweak
    timer = min(3, detached)
    pagehost = 1
    # Keep subsets coherent for synthetic proof.
    if layout + timer + pagehost > strong:
        layout = max(0, strong - timer - pagehost)
    return (
        f"[LB:GC_SUMMARY] pid=19 detached={detached} active={active} pageclients={pageclients} "
        f"retained={retained} rooted={pageclients-unknown} strong={strong} conservative=0 "
        f"unknown={unknown} layout={layout} timer={timer} pagehost={pagehost} paths={pageclients+tweak} END\n"
    )


def positive() -> str:
    text = state(10) + state(20)
    for d in (10, 20):
        s = summary(d)
        text += s + s + s
        text += f"[LB:GC_PATH] pid=19 detached={d} target=0x1 root=Root_Node frame=-1 frame_label= depth=3 path=HTMLDocument>Page>PageClient END\n"
    return text

cases_ok = [positive(), "\x1b[31m" + positive() + "\x1b[0m"]
for i, text in enumerate(cases_ok, 1):
    code, out = mod.analyse(text)
    assert code == 0, (i, out)

bad = []
# une seule copie du resume au jalon 20
x = state(10) + state(20) + summary(10)*2 + summary(20)
bad.append(x)
# deux resumes divergents au meme jalon
x = state(10) + state(20) + summary(10)*2 + summary(20) + summary(20, tweak=1)
bad.append(x)
# racine inconnue
x = state(10) + state(20) + summary(10, unknown=1)*2 + summary(20)*2
bad.append(x)
# retained incoherent
x = state(10) + state(20) + summary(10, retained=9)*2 + summary(20)*2
bad.append(x)
# pas 20 detachements
x = state(10) + summary(10)*2
bad.append(x)

for i, text in enumerate(bad, 1):
    code, out = mod.analyse(text)
    assert code != 0, (i, out)

print(f"GC_RETENTION_ANALYSER_V3_TESTS_OK positifs={len(cases_ok)} negatifs={len(bad)}")
