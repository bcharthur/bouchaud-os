#!/usr/bin/env python3
import sys
from smp_log import summarize
if len(sys.argv)!=3: raise SystemExit("usage: compare-smp-logs.py baseline.log candidate.log")
a,b=summarize(sys.argv[1]),summarize(sys.argv[2])
metrics=[("migrations/s","mig_s"),("context switches/s","ctx_s"),("IRQ preemptions/s","irq_preempt_s"),("deferred preemptions/s","deferred_preempt_s"),("runqueue average","rq_avg"),("runqueue maximum","rq_max"),("steal success %","steal_success_pct"),("imbalance","imbalance_avg"),("BKL wait ms/s","bkl_wait_ms_s"),("BKL hold ms/s","bkl_hold_ms_s"),("BKL acquisitions/s","bkl_acq_s"),("BKL maximum hold ms","bkl_max_hold_ms"),("framebuffer presents/s","fb_fps"),("framebuffer MiB/s","fb_mib_s"),("page faults/s","pf_s"),("TLB shootdowns/s","tlb_s"),("click->first paint ms","click_first_paint_ms")]
print(f"{'metric':28} {'baseline':>12} {'candidate':>12} {'delta':>12}")
print("-"*68)
for label,key in metrics:
    x,y=a.get(key),b.get(key)
    if x is None or y is None: print(f"{label:28} {'n/a':>12} {'n/a':>12} {'n/a':>12}")
    else: print(f"{label:28} {x:12.2f} {y:12.2f} {y-x:+12.2f}")
for name in sorted(set(a['processes'])|set(b['processes'])):
    for suffix,field in [("CPU avg","cpu"),("migrations/s","mig")]:
        def val(s):
            d=s['processes'].get(name); return 0 if not d else (sum(d[field])/max(1,len(d[field])) if field=='cpu' else d[field]/max(s['duration'],1e-9))
        x,y=val(a),val(b); print(f"{(name+' '+suffix)[:28]:28} {x:12.2f} {y:12.2f} {y-x:+12.2f}")
