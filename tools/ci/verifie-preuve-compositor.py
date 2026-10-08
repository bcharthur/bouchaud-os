#!/usr/bin/env python3
"""Regressions comportementales de la preuve cumulative (HOST)."""
from preuve_compositor import verify


def sample(seq, created=1, removed=0, live=1, pid=18):
    return f"[LB:COMPOSITOR_STATE] seq={seq} created={created} removed={removed} live={live} pid={pid} END\n"


start = "HOST_ENDURANCE cycle=1 t_s=1\n"
end = "HOST_ENDURANCE_FIN cycles=58\n"
good = sample(1) + start + "PERF_EX\x1b[broken\n" + sample(2) + end + sample(3)
assert verify(good)[0]
for bad in (
    "", start + end, good.replace(sample(1), ""), good.replace(sample(3), ""),
    good.replace(sample(3), sample(3, 2, 1)),  # remplacement avec PID recycle
    good.replace(sample(3), sample(3, 2, 1, pid=32)),
    good.replace(sample(3), sample(3, 1, 1, 0, 0)),
    good.replace(sample(3), sample(1)),  # autre bras
    good.replace(sample(2), sample(2, 0, 0, 0, 0)),
    good.replace(" END", " EN"),
):
    assert not verify(bad)[0], bad
print("COMPOSITOR_PREUVE_TEST_OK negatifs=10")
