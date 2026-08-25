#!/usr/bin/env python3
import sys
from smp_log import summarize
if len(sys.argv)!=2: raise SystemExit("usage: analyze-smp-log.py run.log")
s=summarize(sys.argv[1])
print("=== SMP SUMMARY ===")
for label,key,unit in [("duration","duration","s"),("total CPU avg","total_cpu_avg","%"),("imbalance avg","imbalance_avg"," points"),("runqueue avg","rq_avg",""),("runqueue max","rq_max",""),("migrations/sec","mig_s",""),("ctx/sec","ctx_s",""),("IRQ preemptions/sec","irq_preempt_s",""),("deferred preempt/sec","deferred_preempt_s",""),("steal attempts/sec","steal_try_s",""),("steal success/sec","steal_ok_s",""),("steal success","steal_success_pct","%"),("reject balance/sec","rej_bal_s",""),("reject affinity/sec","rej_aff_s",""),("BKL wait","bkl_wait_ms_s"," ms/s"),("BKL hold","bkl_hold_ms_s"," ms/s"),("BKL acquisitions/sec","bkl_acq_s",""),("framebuffer presents","fb_fps","/s"),("framebuffer copies","fb_mib_s"," MiB/s"),("page faults/sec","pf_s",""),("TLB shootdowns/sec","tlb_s","")]: print(f"{label:22}: {s[key]:.2f}{unit}")
maximum = "unavailable" if s["bkl_max_hold_ms"] is None else f'{s["bkl_max_hold_ms"]:.2f} ms'
print(f"{'BKL maximum hold':22}: {maximum}")
print(f"{'BKL maximum site':22}: {s['bkl_max_hold_site'] or 'unavailable'}")
print("\n=== BACKING CACHE RATES ===")
for key,value in s["backing_rates"].items(): print(f"{key:28}: {'unavailable' if value is None else f'{value:.2f}'}")
print("cores avg             : ["+", ".join(f"{x:.1f}" for x in s["cores_avg"])+"]")
print("\n=== MM NG6 LIFETIME ===")
for key,value in s["mm_lifetime"].items():
    print(f"{key:28}: {'unavailable' if value is None else f'{value:.0f}'}")
print("\n=== PROCESSES ===")
for name,d in sorted(s["processes"].items()):
    cpu=d["cpu"]; width=max((len(x) for x in d["maps"]),default=0); cmap=[sum(x[i] for x in d["maps"] if i<len(x))/max(1,sum(i<len(x) for x in d["maps"])) for i in range(width)]
    print(f"{name:24} cpu_avg={sum(cpu)/max(1,len(cpu)):.1f}% cpu_max={max(cpu,default=0):.1f}% parallelism_max={max(cpu,default=0)/100:.2f} cpu_map=[{','.join(f'{x:.1f}' for x in cmap)}] mig/s={d['mig']/max(s['duration'],1e-9):.2f} ctx/s={d['ctx']/max(s['duration'],1e-9):.2f} rss_max={d['rss']:.0f}")

if s["applications"]:
    print("\n=== APPLICATIONS ===")
    for name,d in sorted(s["applications"].items()):
        cpu=d["cpu"]
        print(f"{name:24} cpu_avg={sum(cpu)/max(1,len(cpu)):.1f}% cpu_max={max(cpu,default=0):.1f}% mig/s={d['mig']/max(s['duration'],1e-9):.2f} ctx/s={d['ctx']/max(s['duration'],1e-9):.2f} rss_max={d['rss']:.0f}")
