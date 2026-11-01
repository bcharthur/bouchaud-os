#!/usr/bin/env python3
"""C22 remote, fail-closed forensic snapshot of actual guest serial evidence.

Never overrides the benchmark verdict. Absence of a source is reported as missing
coverage, never interpreted as a zero or as an allocation-site attribution.
"""
import argparse
import csv
import json
import re
from collections import Counter, defaultdict
from pathlib import Path

CATEGORIES = {
    "phase": (r"HOST_MEMOIRE_(?:REPERE|ONGLET|FIN|ECHEC)", r"MEMOIRE_PROCESSUS"),
    "process": (r"\[PERF-PROC\]", r"PROCESS_(?:CREATE|EXIT|FAULT)", r"\[PERF-PROC"),
    "threads_scheduler": (r"\b(?:SCHED|SCHEDULER|IPI|WAKE|WAIT|TIMER|THREAD|READY|BLOCKED)\b", r"\[(?:P2|P4|P13)(?::|\])"),
    "ipc": (r"\bIPC\b", r"\b(?:CHANNEL|SOCKET|SENDMSG|RECVMSG|PIPE)\b"),
    "vma_physical": (r"\b(?:VMA|RSS|PRESENT|PAGE_FAULT|MAPPING|MUNMAP|MADVISE|MMAP)\b", r"\[(?:PERF-MEM|PERF-VMA|C22:VMA)"),
    "native_allocator": (r"\[LB:MI_(?:STATS|HEAP|PURGE)\]", r"\[LB:C22:V9\]\s+heap_stats_(?:begin|end)", r"\bmimalloc:\s*(?:heap|stats|purge|reserved|committed)\b"),
    "gc_dom": (r"\b(?:GC_RETENTION|GC:|LIBGC|Garbage collection|Live cells|Document|PAGE_DISCARD|PageClient|ROOT_PATH)\b",),
    "rendering": (r"\b(?:COMPOSITOR|CONTEXT_CREATE|CONTEXT_DESTROY|PAINT|SKIA|GPU|FRAMEBUFFER|BACKING_STORE|SURFACE)\b",),
    "network_worker": (r"\b(?:WEBWORKER|WEB_WORKER|RequestServer|ImageDecoder|PROCESS_SWAP|NETWORK|FETCH|DNS|HTTP)\b",),
}
PATTERNS = {name: [re.compile(s, re.I) for s in values] for name, values in CATEGORIES.items()}
KV = re.compile(r"([A-Za-z_][\w.-]*)=(-?(?:0x[0-9a-fA-F]+|\d+)|[^\s,;]+)")
MEM = re.compile(r"MEMOIRE_PROCESSUS\s+pid=(\d+)\s+image=(\S+)\s+m0_kio=(\d+)\s+m1_kio=(\d+)\s+m2_kio=(\d+)\s+g1_kio=(-?\d+)\s+g2_kio=(-?\d+)\s+verdict=(\S+)")
SPLIT = re.compile(r"(?:HOST_MEMOIRE_REPERE|HOST_MEMOIRE_ONGLET|\[LB\] PAGE_DISCARD)")
RSS_LIMIT_KIO = 1024

def examine(serial, verdict=""):
    records, memory, peaks = [], [], []
    counts = Counter()
    inside_native_stats = False
    for line_no, raw in enumerate(serial.splitlines(), 1):
        if "[LB:C22:V9] heap_stats_begin" in raw:
            inside_native_stats = True
        tags = [name for name, regexes in PATTERNS.items() if any(p.search(raw) for p in regexes)]
        if inside_native_stats and "native_allocator" not in tags:
            tags.append("native_allocator")
        if "[LB:C22:V9] heap_stats_end" in raw:
            inside_native_stats = False
        if not tags:
            continue
        kv = dict(KV.findall(raw))
        pid = kv.get("pid", "")
        role = kv.get("image", kv.get("role", kv.get("service", "")))
        # Preserve the complete original line so every claim has traceable evidence.
        row = dict(line=line_no, categories=";".join(tags), pid=pid, role=role,
                   address=kv.get("start", kv.get("addr", kv.get("heap", ""))),
                   event_id=kv.get("onglet", kv.get("page", "")),
                   raw=raw.strip()[:4096])
        records.append(row)
        counts.update(tags)
        m = MEM.search(raw)
        if m:
            memory.append(dict(pid=int(m[1]), role=m[2], m0=int(m[3]), m1=int(m[4]),
                               m2=int(m[5]), g1=int(m[6]), g2=int(m[7]), verdict=m[8], line=line_no))
    for line_no,raw in enumerate(verdict.splitlines(),1):
        m=MEM.search(raw)
        if m:
            entry=dict(pid=int(m[1]), role=m[2], m0=int(m[3]), m1=int(m[4]),
                       m2=int(m[5]),g1=int(m[6]),g2=int(m[7]),verdict=m[8],line="verdict:"+str(line_no))
            if not any(x["pid"]==entry["pid"] and x["m2"]==entry["m2"] for x in memory):
                memory.append(entry)
    # Candidate peaks are indexed by serial position, not an invented wallclock.
    for record in records:
        raw=record["raw"]
        if (m:=re.search(r"delta_present_kio=(-?\d+)",raw)) and abs(int(m[1])) >= 128:
            peaks.append(dict(line=record["line"],mechanism="vma_resident_delta",
                              magnitude_kio=int(m[1]),pid=record["pid"],evidence=raw))
        if (m:=re.search(r"(?:general_live_bytes|live_bytes)=([0-9]+)",raw)):
            if int(m[1]) > 6000000:
                peaks.append(dict(line=record["line"],mechanism="native_live_high",
                                  magnitude_bytes=int(m[1]),pid=record["pid"],evidence=raw))
    for entry in memory:
        if entry["g2"] > RSS_LIMIT_KIO:
            peaks.append(dict(line=entry["line"],mechanism="contract_rss_overgrowth",
                              magnitude_kio=entry["g2"],pid=entry["pid"],evidence=entry))
    return records,memory,peaks,counts

def resident_vma_delta(serial):
    """Compare actual guest PID20 RSS/VMA snapshots nearest to M1 and M2.

    A VMA size is virtual capacity, NOT resident bytes. The guest does not
    currently export per-VMA resident page counts or allocation call stacks.
    """
    ansi = re.compile(r"\x1b\[[0-9;]*m")
    mark = re.compile(r"(\d+\.\d+)\s+WebContent\(20\):.*HOST_MEMOIRE_REPERE\s+m=([12])\b")
    rss_re = re.compile(
        r"\[C22:RSS\]\s+t=(\d+)\s+pid=20\s+rss_kio=(\d+)\s+"
        r"anon_kio=(\d+)\s+fichier_kio=(\d+)\s+partage_kio=(\d+).*?\bvma=(\d+)"
    )
    vma_re = re.compile(
        r"\[C22:VMA\]\s+t=(\d+)\s+pid=20\s+id=\d+\s+"
        r"start=(0x[0-9a-fA-F]+)\s+end=(0x[0-9a-fA-F]+)\s+"
        r"size_kio=(\d+)\s+type=(\w+)"
    )
    marks, rss, vmas, vma_expected, vma_complete = {}, {}, defaultdict(dict), {}, set()
    for raw in serial.splitlines():
        line = ansi.sub("", raw)
        if (m := mark.search(line)):
            marks.setdefault(int(m[2]), round(float(m[1]) * 1000))
        if (m := rss_re.search(line)):
            rss[int(m[1])] = dict(rss_kio=int(m[2]), anon_kio=int(m[3]),
                                  fichier_kio=int(m[4]), partage_kio=int(m[5]),
                                  vma_count=int(m[6]))
        if (m := re.search(r"\[C22:VMA-BEGIN\]\s+t=(\d+)\s+pid=20\s+total=(\d+)", line)):
            vma_expected[int(m[1])] = int(m[2])
        if (m := vma_re.search(line)):
            vmas[int(m[1])][m[2]] = dict(size_kio=int(m[4]), type=m[5])
        if (m := re.search(r"\[C22:VMA-END\]\s+t=(\d+)\s+pid=20", line)):
            vma_complete.add(int(m[1]))
    result = dict(phase_markers_ms=marks, rss_samples=[
        dict(t_ms=t, **values) for t, values in sorted(rss.items())])
    if not all(x in marks for x in (1, 2)) or not rss:
        result["comparison"] = "missing_M1_M2_or_RSS"
        return result
    # A post-marker RSS sample includes subsequent browser activity and must
    # never be attributed to the M1/M2 marker. The 5-second sampler can land
    # on either side; use the most recent snapshot at or before each marker.
    chosen = {phase: max((t for t in rss if t <= marks[phase]), default=None) for phase in (1, 2)}
    if any(chosen[p] is None or marks[p] - chosen[p] > 6000 for p in (1, 2)):
        result["comparison"] = "no_preceding_RSS_within_6s_of_phase"
        return result
    t1, t2 = chosen[1], chosen[2]
    result["m1"] = dict(t_ms=t1, **rss[t1])
    result["m2"] = dict(t_ms=t2, **rss[t2])
    result["delta_kio"] = {
        key: rss[t2][key] - rss[t1][key]
        for key in ("rss_kio", "anon_kio", "fichier_kio", "partage_kio")
    }
    # The kernel publishes VMA and RSS in the same sampling pass. Require
    # identical timestamps and a complete BEGIN/END snapshot: otherwise a
    # truncated serial capture can fabricate added or removed mappings.
    map_times = {p: chosen[p] for p in (1, 2) if chosen[p] in vmas}
    if len(map_times) != 2:
        result["vma_comparison"] = "no_VMA_snapshot_at_RSS_timestamp"
        return result
    if any(t not in vma_complete or len(vmas[t]) != vma_expected.get(t, -1) for t in map_times.values()):
        result["vma_comparison"] = "incomplete_VMA_snapshot"
        return result
    a, b = vmas[map_times[1]], vmas[map_times[2]]
    added = [dict(start=k, **v) for k, v in b.items() if k not in a]
    removed = [dict(start=k, **v) for k, v in a.items() if k not in b]
    result["vma_comparison"] = dict(m1_t_ms=map_times[1], m2_t_ms=map_times[2],
        added=added, removed=removed, added_virtual_kio=sum(x["size_kio"] for x in added),
        warning="Virtual VMA size is NOT physical RSS; existing mappings may gain resident pages.")
    return result


def build(serial_path,verdict_path,output):
    serial=Path(serial_path).read_text(encoding="utf-8",errors="replace")
    verdict=Path(verdict_path).read_text(encoding="utf-8",errors="replace") if verdict_path and Path(verdict_path).exists() else ""
    records,memory,peaks,counts=examine(serial,verdict)
    resident=resident_vma_delta(serial)
    output=Path(output)
    output.mkdir(parents=True,exist_ok=True)
    with (output/"events.csv").open("w",newline="",encoding="utf-8") as f:
        w=csv.DictWriter(f,fieldnames=["line","categories","pid","role","address","event_id","raw"])
        w.writeheader();w.writerows(records)
    with (output/"processes.csv").open("w",newline="",encoding="utf-8") as f:
        w=csv.DictWriter(f,fieldnames=["pid","role","m0","m1","m2","g1","g2","verdict","line"])
        w.writeheader();w.writerows(memory)
    # Correlation windows preserve original order, do not fabricate shared timestamps.
    windows=[]
    for peak in peaks[:100]:
        n=peak["line"] if isinstance(peak["line"],int) else None
        nearby=[r for r in records if n is not None and abs(r["line"]-n)<=180]
        windows.append(dict(peak=peak,nearby=nearby[-100:]))
    report={
      "source":str(serial_path),"verdict_source":str(verdict_path or ""),
      "rss_limit_kio":RSS_LIMIT_KIO,
      "processes":memory,"peaks":peaks,"windows":windows,
      "resident_vma_delta":resident,
      "coverage":{category:dict(observed=counts[category]>0,events=counts[category])
                  for category in CATEGORIES},
      "inference_limitations":[
        "Serial line ordering is not a shared monotonic clock across processes.",
        "Thread ids, task owners and native caller stacks cannot be inferred unless present in source evidence.",
        "A VMA increase is not by itself proof of a leaking allocation.",
        "GC cell counts, native live bytes and RSS are not additive.",
        "This analyzer does not change the contractual C22 verdict, threshold, or historical witness.",
      ]
    }
    (output/"forensic.json").write_text(json.dumps(report,indent=2,ensure_ascii=False),encoding="utf-8")
    md=["# C22 A-Z: real guest evidence","",
      f"Contractual RSS threshold: {RSS_LIMIT_KIO} KiB (unchanged).","",
      "## Processus / RSS"]
    for m in memory:
        md.append(f"- PID {m['pid']} {m['role']}: M0={m['m0']} M1={m['m1']} M2={m['m2']} KiB; g2={m['g2']} KiB; verdict={m['verdict']} (evidence {m['line']})")
    md.extend(["","## PID20 anonymous RSS and VMA change M1 -> M2"])
    if "delta_kio" in resident:
        md.append("RSS delta (KiB): " + ", ".join(
            f"{k}={v}" for k,v in resident["delta_kio"].items()))
        vma = resident.get("vma_comparison", {})
        if isinstance(vma, dict):
            md.append(f"New VMAs: {len(vma['added'])}, removed: {len(vma['removed'])}, "
                      f"new virtual KiB: {vma['added_virtual_kio']} (NOT resident KiB)")
            for item in vma["added"][:20]:
                md.append(f"- added virtual mapping {item['start']} size={item['size_kio']} KiB type={item['type']}")
        else:
            md.append(f"VMA evidence incomplete: {vma}")
    else:
        md.append(f"RSS comparison incomplete: {resident.get('comparison', 'unknown')}")
    md.extend(["","## System coverage, actually observed"])
    for category, amount in counts.items():
        md.append(f"- {category}: {amount} serial records")
    for category in CATEGORIES:
        if not counts[category]:md.append(f"- {category}: **NOT OBSERVED**, cannot draw conclusions")
    md.extend(["","## Peak correlation",""])
    for peak in peaks[:30]:
        md.append(f"- {peak['mechanism']}: {peak.get('magnitude_kio',peak.get('magnitude_bytes'))} at evidence line {peak['line']}, PID {peak['pid']}")
    md.extend(["","## Attribution gaps","",
      "This reports available kernel, scheduler, IPC, GC, allocator, process and VMA signals. "
      "It does NOT claim to trace every allocation/free, page fault or context switch; an uninstrumented event is not a zero event.",
      "Native callstacks require allocation-site sampling inside the guest; address-to-symbol mapping requires exact matching debug symbols.",
      "A red contractual verdict remains RED."])
    (output/"forensic.md").write_text("\n".join(md)+"\n",encoding="utf-8")
    print(f"C22_FORENSIC_AZ_OK events={len(records)} processes={len(memory)} peaks={len(peaks)} output={output}")
    for k in CATEGORIES:
        if not counts[k]:print(f"C22_FORENSIC_GAP domain={k}")
    return 0

def selftest():
    log="\n".join(["HOST_MEMOIRE_REPERE phase=M1","[PERF-PROC] pid=20 image=WebContent rss_kio=70000",
    "[LB:MI_STATS] heap=General live_bytes=6781408","[LB:GC] Live cells: 8195",
    "[C22:VMA] pid=20 start=0x404000000000 delta_present_kio=908",
    "[IPC] pid=20 send","[SCHED] pid=20 wake","[LB] CONTEXT_DESTROY ctx=3",
    "MEMOIRE_PROCESSUS pid=20 image=WebContent m0_kio=67180 m1_kio=71768 m2_kio=72812 g1_kio=4588 g2_kio=1044 verdict=croissance"])
    rows,mem,peaks,counts=examine(log)
    assert mem[0]["g2"]==1044 and len(rows)>=8
    assert any(p["mechanism"]=="contract_rss_overgrowth" for p in peaks)
    assert counts["ipc"]>=1 and counts["threads_scheduler"]>=1 and counts["gc_dom"]>=1
    assert counts["native_allocator"]>=1  # synthetic heap marker is explicit
    _,_,_,native_block=examine("[LB:C22:V9] heap_stats_begin cycle=16\nreserved: 123\n[LB:C22:V9] heap_stats_end cycle=16")
    assert native_block["native_allocator"]==3
    _,_,_,no_allocator=examine("\n".join([
        "[LB:C22] collecte partitions=3 temoin=0",
        "[LB:NAV-TRACE] evt=commit pid=20",
        "BOUCHAUD_BUILD commit=12345",
        "[LB] PAGE_DISCARD pid=20",
    ]))
    assert no_allocator["native_allocator"]==0  # no false mimalloc coverage from generic 'commit'
    rows,mem,peaks,_=examine("not a memory verdict")
    assert not rows and not mem and not peaks
    sample="\n".join([
        "61.200 WebContent(20): (js log) HOST_MEMOIRE_REPERE m=1 n=10",
        "[C22:RSS] t=61000 pid=20 rss_kio=70000 anon_kio=10000 fichier_kio=59000 partage_kio=100 vma=1",
        "[C22:VMA-BEGIN] t=61000 pid=20 total=1",
        "[C22:VMA] t=61000 pid=20 id=1 start=0x1000 end=0x3000 size_kio=8 type=zero",
        "[C22:VMA-END] t=61000 pid=20",
        "95.200 WebContent(20): (js log) HOST_MEMOIRE_REPERE m=2 n=10",
        "[C22:RSS] t=95000 pid=20 rss_kio=71100 anon_kio=11100 fichier_kio=59000 partage_kio=100 vma=2",
        "[C22:VMA-BEGIN] t=95000 pid=20 total=2",
        "[C22:VMA] t=95000 pid=20 id=1 start=0x1000 end=0x3000 size_kio=8 type=zero",
        "[C22:VMA] t=95000 pid=20 id=2 start=0x4000 end=0x6000 size_kio=8 type=zero",
        "[C22:VMA-END] t=95000 pid=20",
        # Regression: nearest-sample would incorrectly choose these post-marker
        # samples and invent an RSS decrease plus a missing VMA.
        "[C22:RSS] t=65000 pid=20 rss_kio=71000 anon_kio=11000 fichier_kio=59000 partage_kio=100 vma=1",
        "[C22:RSS] t=100000 pid=20 rss_kio=69000 anon_kio=9000 fichier_kio=59000 partage_kio=100 vma=1",
    ])
    delta=resident_vma_delta(sample)
    assert delta["delta_kio"]["anon_kio"]==1100
    assert delta["m1"]["t_ms"]==61000 and delta["m2"]["t_ms"]==95000
    assert len(delta["vma_comparison"]["added"])==1
    print("C22_FORENSIC_AZ_SELFTEST_OK")
    return 0

def main():
    p=argparse.ArgumentParser()
    p.add_argument("serial",nargs="?")
    p.add_argument("--verdict")
    p.add_argument("--out")
    p.add_argument("--selftest",action="store_true")
    a=p.parse_args()
    if a.selftest:return selftest()
    if not a.serial or not a.out:p.error("serial and --out required")
    return build(a.serial,a.verdict,a.out)
if __name__=="__main__":raise SystemExit(main())
