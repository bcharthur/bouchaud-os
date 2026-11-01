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
    "native_allocator": (r"\b(?:mimalloc|mi_heap|mi_collect|heap_stats|malloc|free|purge|commit|decommit)\b", r"\[LB:MI_(?:STATS|HEAP|PURGE)\]"),
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
    for line_no, raw in enumerate(serial.splitlines(), 1):
        tags = [name for name, regexes in PATTERNS.items() if any(p.search(raw) for p in regexes)]
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

def build(serial_path,verdict_path,output):
    serial=Path(serial_path).read_text(encoding="utf-8",errors="replace")
    verdict=Path(verdict_path).read_text(encoding="utf-8",errors="replace") if verdict_path and Path(verdict_path).exists() else ""
    records,memory,peaks,counts=examine(serial,verdict)
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
    "[LB:C22] heap=General live_bytes=6781408","[LB:GC] Live cells: 8195",
    "[C22:VMA] pid=20 start=0x404000000000 delta_present_kio=908",
    "[IPC] pid=20 send","[SCHED] pid=20 wake","[LB] CONTEXT_DESTROY ctx=3",
    "MEMOIRE_PROCESSUS pid=20 image=WebContent m0_kio=67180 m1_kio=71768 m2_kio=72812 g1_kio=4588 g2_kio=1044 verdict=croissance"])
    rows,mem,peaks,counts=examine(log)
    assert mem[0]["g2"]==1044 and len(rows)>=8
    assert any(p["mechanism"]=="contract_rss_overgrowth" for p in peaks)
    assert counts["ipc"]>=1 and counts["threads_scheduler"]>=1 and counts["gc_dom"]>=1
    assert counts["native_allocator"]>=1  # synthetic heap marker is explicit
    _,_,_,no_allocator=examine("[LB:C22] collecte partitions=3 temoin=0")
    assert no_allocator["native_allocator"]==0  # C22 collection is not allocator telemetry
    rows,mem,peaks,_=examine("not a memory verdict")
    assert not rows and not mem and not peaks
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
