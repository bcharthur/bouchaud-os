#!/usr/bin/env python3
"""Forensic summary for real C22 CI logs. Does not alter contractual verdict."""
import argparse
import csv
import json
import re
from pathlib import Path

MEM = re.compile(r'MEMOIRE_PROCESSUS\s+pid=(\d+)\s+image=(\S+)\s+m0_kio=(\d+)\s+m1_kio=(\d+)\s+m2_kio=(\d+)\s+g1_kio=(\-?\d+)\s+g2_kio=(\-?\d+)\s+verdict=(\S+)')
PROC = re.compile(r'\[PERF-PROC\].*')
HEAP = re.compile(r'\[LB:C22:HEAP\].*')
GC = re.compile(r'\[LB:C22:GC_.*')
VMA = re.compile(r'\[C22:(?:ATLAS-VMA|VMA).*')
MARK = re.compile(r'HOST_MEMOIRE_(?:REPERE|ONGLET).*')
KEY = re.compile(r'([a-zA-Z_][a-zA-Z0-9_]*)=([^\s]+)')

def parse(path):
    lines=Path(path).read_text(errors="replace").splitlines()
    rows=[]
    for number,line in enumerate(lines,1):
        kind=None
        for name,pattern in (("process",PROC),("native_heap",HEAP),("gc",GC),("vma",VMA),("phase",MARK)):
            if pattern.search(line):
                kind=name;break
        if kind is None and ("[LB:C22:V9]" in line or "reserved" in line.lower() and "mimalloc" in line.lower()):
            kind="mimalloc"
        if kind is None and any(s in line.lower() for s in ("document_destroy","after_discard","ipc","scheduler","madvise","munmap")):
            kind="lifecycle_or_system"
        if kind:
            m=dict(KEY.findall(line))
            rows.append(dict(line=number,category=kind,pid=m.get("pid",""),image=m.get("image",m.get("role","")),
                            address=m.get("start",m.get("heap","")),rss_kio=m.get("rss_kio",""),
                            live_bytes=m.get("live_bytes",""),present_kio=m.get("present_kio",""),raw=line[:900]))
    return lines,rows

def main():
    p=argparse.ArgumentParser()
    p.add_argument("serial")
    p.add_argument("--verdict",default="")
    p.add_argument("--out",required=True)
    a=p.parse_args()
    out=Path(a.out);out.mkdir(parents=True,exist_ok=True)
    lines,rows=parse(a.serial)
    with (out/"chronologie.csv").open("w",newline="") as f:
        w=csv.DictWriter(f,fieldnames=["line","category","pid","image","address","rss_kio","live_bytes","present_kio","raw"])
        w.writeheader();w.writerows(rows)
    v=Path(a.verdict).read_text(errors="replace") if a.verdict and Path(a.verdict).exists() else ""
    memory=[]
    for line in v.splitlines():
        if m:=MEM.search(line):
            memory.append(dict(pid=int(m[1]),image=m[2],m0_kio=int(m[3]),m1_kio=int(m[4]),m2_kio=int(m[5]),g1_kio=int(m[6]),g2_kio=int(m[7]),verdict=m[8]))
    present=sorted(set(r["category"] for r in rows))
    coverage={k:k in present for k in ("process","native_heap","gc","vma","phase","mimalloc","lifecycle_or_system")}
    result={"measurements":memory,"coverage":coverage,"source_lines":len(lines),"events":len(rows),
            "provenance":"serial line only; no shared monotonic clock or native malloc callstacks",
            "limitations":["No individual malloc/free caller or stack captured","Missing domain does not prove no events",
            "GC, allocator live bytes and RSS are not additive","A green single run is not C22 qualification"]}
    (out/"rapport.json").write_text(json.dumps(result,indent=2),encoding="utf-8")
    with (out/"rapport.md").open("w") as f:
        f.write("# C22 V9 - forensic diagnostic\n\n")
        f.write("All measurements retain serial line numbers. This analysis does not weaken the C22 verdict.\n\n")
        for m in memory:
            f.write(f"- PID {m['pid']} {m['image']}: M1={m['m1_kio']} KiB M2={m['m2_kio']} KiB delta={m['g2_kio']} KiB verdict={m['verdict']}\n")
        f.write("\n## Coverage\n\n")
        for k,ok in coverage.items():f.write(f"- {k}: {'observed' if ok else 'NOT OBSERVED'}\n")
        f.write("\n## Important limits\n\n")
        for item in result["limitations"]:f.write(f"- {item}\n")
    print("C22_V9_FORENSIC_OK events=%d mem_procs=%d domains=%s"%(len(rows),len(memory),",".join(present)))
    if not memory:
        print("C22_V9_INCOMPLETE: no contractual per-process verdict found (analysis only)")
    return 0

if __name__=="__main__":raise SystemExit(main())
