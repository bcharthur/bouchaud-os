#!/usr/bin/env python3
import re
from collections import defaultdict
from pathlib import Path

KV = re.compile(r"([A-Za-z_]+)=((?:\[[^]]*\])|(?:[^ ]+))")

def fields(line):
    return dict(KV.findall(line))

def vec(value):
    if not value or value == "[]": return []
    return [float(x) for x in value.strip("[]").split(",") if x]

def number(value, default=0.0):
    try: return float(value)
    except (TypeError, ValueError): return default

def _decode_log(path):
    raw = Path(path).read_bytes()
    if raw.startswith((b"\xff\xfe", b"\xfe\xff")):
        return raw.decode("utf-16")
    if raw.startswith(b"\xef\xbb\xbf"):
        return raw.decode("utf-8-sig")
    # Windows PowerShell 5 peut produire UTF-16LE sans BOM après certaines
    # concaténations. Un NUL sur les octets impairs est un signal fiable pour
    # nos lignes ASCII de télémétrie.
    probe = raw[:256]
    if probe and len(probe) >= 4 and probe[1::2].count(0) * 4 >= len(probe):
        return raw.decode("utf-16-le")
    return raw.decode("utf-8", errors="replace")

def parse(path):
    smp, proc, perf = [], [], []
    text = _decode_log(path)
    for line_number, line in enumerate(text.splitlines(), 1):
        if "[SMP-SAMPLE]" in line:
            sample = fields(line)
            missing = {"v", "t_ns", "window_ns", "load"} - sample.keys()
            if missing:
                raise ValueError(f"SMP-SAMPLE ligne {line_number} invalide: champs manquants {sorted(missing)}")
            smp.append(sample)
        elif "[PROC-SAMPLE]" in line: proc.append(fields(line))
        elif "PERF_" in line: perf.append((line.strip(), fields(line)))
    return smp, proc, perf

def summarize(path):
    smp, proc, perf = parse(path)
    duration = sum(number(s.get("window_ns")) for s in smp) / 1e9
    loads = [vec(s.get("load")) for s in smp]
    width = max((len(x) for x in loads), default=0)
    core_avg = [sum(x[i] for x in loads if i < len(x)) / max(1, sum(i < len(x) for x in loads)) for i in range(width)]
    total_avg = sum(sum(x) / max(1, len(x)) for x in loads) / max(1, len(loads))
    imbalance = sum((max(x)-min(x)) if x else 0 for x in loads) / max(1, len(loads))
    sums = defaultdict(float)
    for s in smp:
        sums["ctx"] += number(s.get("ctx_delta")); sums["mig"] += number(s.get("mig_delta"))
        sums["steal_ok"] += sum(vec(s.get("steal_ok_delta"))); sums["steal_try"] += sum(vec(s.get("steal_try_delta")))
        sums["rej_bal"] += sum(vec(s.get("steal_rej_bal_delta"))); sums["rej_aff"] += sum(vec(s.get("steal_rej_aff_delta")))
        sums["bkl_wait"] += number(s.get("bkl_wait_delta_ns")); sums["bkl_hold"] += number(s.get("bkl_hold_delta_ns"))
        sums["pf"] += sum(vec(s.get("pf_delta"))); sums["tlb"] += number(s.get("tlb_delta"))
    rate = lambda key: sums[key] / duration if duration else 0.0
    processes = defaultdict(lambda: {"cpu":[], "maps":[], "ctx":0.0, "mig":0.0, "rss":0.0})
    for p in proc:
        d=processes[p.get("name", p.get("pid","?"))]; d["cpu"].append(number(p.get("cpu_pct"))); d["maps"].append(vec(p.get("cpu_map")))
        d["ctx"] += number(p.get("ctx_delta")); d["mig"] += number(p.get("mig_delta")); d["rss"] = max(d["rss"], number(p.get("rss")))
    click_to_paint = None
    for line, f in perf:
        if "PERF_FIRST_PAINT" in line and number(f.get("since_click_ms")) > 0: click_to_paint = number(f["since_click_ms"])
    return {"duration":duration,"total_cpu_avg":total_avg,"cores_avg":core_avg,"imbalance_avg":imbalance,
      "ctx_s":rate("ctx"),"mig_s":rate("mig"),"steal_try_s":rate("steal_try"),"steal_ok_s":rate("steal_ok"),
      "steal_success_pct":100*sums["steal_ok"]/sums["steal_try"] if sums["steal_try"] else 0,
      "rej_bal_s":rate("rej_bal"),"rej_aff_s":rate("rej_aff"),"bkl_wait_ms_s":rate("bkl_wait")/1e6,
      "bkl_hold_ms_s":rate("bkl_hold")/1e6,"pf_s":rate("pf"),"tlb_s":rate("tlb"),"processes":processes,
      "click_first_paint_ms":click_to_paint}
