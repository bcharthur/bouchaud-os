#!/usr/bin/env python3
import sys
from smp_log import summarize
if len(sys.argv)!=2: raise SystemExit("usage: analyze-smp-log.py run.log")
s=summarize(sys.argv[1])
print("=== SMP SUMMARY ===")
for label,key,unit in [("duration","duration","s"),("total CPU avg","total_cpu_avg","%"),("imbalance avg","imbalance_avg"," points"),("migrations/sec","mig_s",""),("ctx/sec","ctx_s",""),("steal attempts/sec","steal_try_s",""),("steal success/sec","steal_ok_s",""),("steal success","steal_success_pct","%"),("reject balance/sec","rej_bal_s",""),("reject affinity/sec","rej_aff_s",""),("BKL wait","bkl_wait_ms_s"," ms/s"),("BKL hold","bkl_hold_ms_s"," ms/s"),("page faults/sec","pf_s",""),("TLB shootdowns/sec","tlb_s","")]: print(f"{label:22}: {s[key]:.2f}{unit}")
print("cores avg             : ["+", ".join(f"{x:.1f}" for x in s["cores_avg"])+"]")
print("\n=== PROCESSES ===")
for name,d in sorted(s["processes"].items()):
    cpu=d["cpu"]; width=max((len(x) for x in d["maps"]),default=0); cmap=[sum(x[i] for x in d["maps"] if i<len(x))/max(1,sum(i<len(x) for x in d["maps"])) for i in range(width)]
    print(f"{name:24} cpu_avg={sum(cpu)/max(1,len(cpu)):.1f}% cpu_max={max(cpu,default=0):.1f}% parallelism_max={max(cpu,default=0)/100:.2f} cpu_map=[{','.join(f'{x:.1f}' for x in cmap)}] mig/s={d['mig']/max(s['duration'],1e-9):.2f} ctx/s={d['ctx']/max(s['duration'],1e-9):.2f} rss_max={d['rss']:.0f}")

if s["applications"]:
    print("\n=== APPLICATIONS ===")
    for name,d in sorted(s["applications"].items()):
        cpu=d["cpu"]
        print(f"{name:24} cpu_avg={sum(cpu)/max(1,len(cpu)):.1f}% cpu_max={max(cpu,default=0):.1f}% mig/s={d['mig']/max(s['duration'],1e-9):.2f} ctx/s={d['ctx']/max(s['duration'],1e-9):.2f} rss_max={d['rss']:.0f}")
