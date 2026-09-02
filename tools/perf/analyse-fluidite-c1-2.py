#!/usr/bin/env python3
"""Resume les preuves de fluidite C1.2 dans un journal serie Bouchaud."""

import argparse
import codecs
import re
from pathlib import Path


def values(pattern: str, text: str) -> list[int]:
    return [int(match) for match in re.findall(pattern, text)]


def read_journal(path: Path) -> str:
    """Lit aussi bien un journal UTF-8 qu'un Tee-Object Windows PowerShell.

    Windows PowerShell 5.1 ecrit couramment les fichiers de Tee-Object en
    UTF-16. Une lecture UTF-8 avec remplacement ne leve aucune erreur, mais
    intercale alors des NUL et rend toutes les balises runtime invisibles.
    """
    raw = path.read_bytes()
    if raw.startswith((codecs.BOM_UTF16_LE, codecs.BOM_UTF16_BE)):
        return raw.decode("utf-16")
    if raw.startswith(codecs.BOM_UTF8):
        return raw.decode("utf-8-sig", errors="replace")

    sample = raw[:8192]
    even_nuls = sample[0::2].count(0)
    odd_nuls = sample[1::2].count(0)
    pairs = max(1, len(sample) // 2)
    if odd_nuls > pairs // 3 and odd_nuls > even_nuls * 4:
        return raw.decode("utf-16-le", errors="replace")
    if even_nuls > pairs // 3 and even_nuls > odd_nuls * 4:
        return raw.decode("utf-16-be", errors="replace")
    return raw.decode("utf-8", errors="replace")


parser = argparse.ArgumentParser()
parser.add_argument("journal", type=Path)
parser.add_argument("--max-mm-spin-ms", type=int, default=250)
args = parser.parse_args()

text = read_journal(args.journal)
rss = re.findall(
    r"\[MM-RSS-O1\] snapshots=(\d+) pages_observed=(\d+) "
    r"processes=(\d+) live_tasks=(\d+)",
    text,
)
if not rss:
    raise SystemExit("ECHEC: aucune preuve [MM-RSS-O1] dans le journal")

mm_spins = values(
    r"\[SMP-SPIN\].*?depuis=(\d+)ms "
    r"site=src[\\/]kernel[\\/]process[\\/]thread[\\/]processus\.rs:24",
    text,
)
frame_gaps = values(r"frame_gap_max_ms=(\d+)", text)
fault_deltas = values(r"pf_delta=(\d+)", text)
cluster = re.findall(
    r"\[MM-CLUSTER\].*?mapped=(\d+).*?max_batch=(\d+).*?mm_locks=(\d+)",
    text,
)

last_snapshots, last_pages, last_processes, last_tasks = map(int, rss[-1])
print(
    "RSS O(1): "
    f"snapshots={last_snapshots} pages_observed={last_pages} "
    f"processes={last_processes} live_tasks={last_tasks}"
)
print(
    "Contention Mm: "
    f"episodes={len(mm_spins)} max_ms={max(mm_spins, default=0)} "
    f"budget_ms={args.max_mm_spin_ms}"
)
print(
    "Fluidite: "
    f"frame_gap_max_ms={max(frame_gaps, default=0)} "
    f"pf_delta_max={max(fault_deltas, default=0)}"
)
if cluster:
    mapped, batch, locks = map(int, cluster[-1])
    ratio = mapped / locks if locks else 0.0
    print(
        "Grappes fichier: "
        f"mapped={mapped} mm_locks={locks} pages_par_prise={ratio:.2f} max_batch={batch}"
    )

if not any(int(tasks) > int(processes) for _, _, processes, tasks in rss):
    raise SystemExit("ECHEC: le journal ne prouve aucun processus multithread")
if mm_spins and max(mm_spins) > args.max_mm_spin_ms:
    raise SystemExit(
        f"ECHEC: attente Mm {max(mm_spins)} ms > budget {args.max_mm_spin_ms} ms"
    )
print("C1.2 FLUIDITE: OK")
