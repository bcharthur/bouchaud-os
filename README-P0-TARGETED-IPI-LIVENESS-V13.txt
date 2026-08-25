BOUCHAUD OS - P0 #1 v1.3 - TARGETED IPI LIVENESS
===================================================

WHAT THE v1.2 RUN PROVED
------------------------
The IPI reduction worked:

- before browser launch, CPU1/CPU2/CPU3 counters stopped at roughly 1 each;
- the old +250 IPI/s/AP broadcast storm disappeared.

But Ladybird did not start.

The browser task was queued on CPU1:

  [SMP-TASK] ... rq=1 ... name=/bo-navigateur

Then CPU1 received one wake IPI and remained at scheduler site 52 forever:

  site=[...,52:..., ...]
  c1 rq=1 cur=0:0
  ctx_delta=0

Site 52 is immediately after wait_for_interrupt() and immediately before
resume_after_schedule(depth).

ROOT CAUSE
----------
resume_after_schedule() currently reuses the adaptive BKL wait helper. After a
short spin, that helper can execute HLT.

That was accidentally safe while every AP received a scheduler broadcast every
4 ms: the next IPI woke the CPU again.

v1.2 removed that periodic heartbeat. If CPU1 wakes to run a queued task while
another CPU briefly owns the BKL, CPU1 can HLT inside resume_after_schedule().
BKL release does not send a wakeup IPI, so the AP can sleep forever.

This exact invariant was already identified by the older SMP BKL Liveness V2
work: resume_after_schedule() must not HLT.

WHAT v1.3 DOES
--------------
Keep the targeted IPI optimization.

Change ONLY resume_after_schedule():

  adaptive spin + HLT
          ->
  active spin until the BKL becomes free

Normal enter() keeps the adaptive HLT policy. Therefore idle APs still sleep;
only an AP that has already been explicitly woken for useful scheduler work
stays active while reacquiring the BKL.

RUN
---

  .\APPLY-P0-TARGETED-IPI-LIVENESS-V13.ps1 -Preview
  .\APPLY-P0-TARGETED-IPI-LIVENESS-V13.ps1
  .\VERIFY-P0-TARGETED-IPI-LIVENESS-V13.ps1 -Build
  .\run.ps1

EXPECTED RUNTIME
----------------
1. Desktop idle:
   c1/c2/c3 IPI counts should stay almost flat.

2. Browser launch:
   /bo-navigateur may be queued on an AP, but it must leave rq quickly.
   ctx_delta must become non-zero.
   BROWSER_HOST_INITIALIZED must appear.
   WEBCONTENT_READY must appear.
   M11_READY must appear.
   M11_DOCUMENT_LOADED must eventually appear.

3. If Ladybird remains queued with:
     rq=1 cur=0:0 site=52
   v1.3 failed and should be rolled back.

ROLLBACK
--------
  .\ROLLBACK-P0-TARGETED-IPI-LIVENESS-V13.ps1
