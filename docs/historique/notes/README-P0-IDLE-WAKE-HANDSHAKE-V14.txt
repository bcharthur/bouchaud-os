BOUCHAUD OS - P0 #1 v1.4 - IDLE WAKE HANDSHAKE
====================================================

WHAT THE v1.3 RUN PROVED
------------------------
v1.3 fixed the previous "site=52 / resume_after_schedule() can HLT" bug,
but Ladybird still never executed.

The important sequence is now:

  /bo-navigateur -> rq=1 on CPU1
  CPU1 IPI count -> 1 to 2
  CPU1 current    -> 0:0
  CPU1 runqueue   -> stays 1
  ctx_delta       -> stays 0
  BKL owner       -> usually 0

So CPU1 did receive a targeted wakeup, but it returned to sleep without
consuming the ready task.

ROOT CAUSE
----------
The scheduler sleep protocol has a classic lost-wakeup window.

Current AP path:

  check runqueue under BKL
  release BKL
  ...
  IDLE=true
  STI;HLT

publish_ready():

  enqueue task under BKL
  if target is IDLE:
      send targeted IPI

There are two races:

A) A producer can enqueue after BKL release but before IDLE=true.
   It sees IDLE=false and sends no IPI. The AP then halts forever.

B) IDLE can become true while IF is still enabled, before STI;HLT.
   The targeted IPI can be handled immediately, then execution returns and
   HLT runs after the only wakeup has already been consumed.

The old 4 ms broadcast hid both races by supplying another interrupt soon
afterward.

THE FIX
-------
Use the canonical scheduler idle handshake:

  while BKL is still owned:
      CLI
      publish IDLE=true

  release BKL

  STI;HLT

  after wake:
      IDLE=false
      reacquire BKL

Because every normal Ready publication is serialized by the BKL, a producer
cannot publish work in the transition window. If it sends the targeted IPI
after BKL release, IF is still zero and the interrupt remains pending. x86's
STI interrupt shadow guarantees HLT executes before that pending interrupt is
delivered, so the CPU cannot lose the wakeup.

v1.4 converts five scheduler sleep sites:
- ABI direct wait
- schedule() blocked wait
- AP pre-task idle
- AP normal no-work idle
- BSP exit_current idle loop

It does NOT:
- reintroduce periodic broadcast IPIs;
- change runqueue selection;
- change Ladybird;
- change the v1.3 resume_after_schedule active-spin fix.

RUN
---
  .\APPLY-P0-IDLE-WAKE-HANDSHAKE-V14.ps1 -Preview
  .\APPLY-P0-IDLE-WAKE-HANDSHAKE-V14.ps1
  .\VERIFY-P0-IDLE-WAKE-HANDSHAKE-V14.ps1 -Build
  .\run.ps1

EXPECTED RUNTIME
----------------
Before Ladybird:
  c1/c2/c3 IPI counters should remain almost flat.

At browser launch:
  [SMP-TASK] ... /bo-navigateur rq=1
  targeted wake reaches that CPU
  rq must return to 0 quickly
  ctx_delta must become > 0
  browser cpu_pct must become non-zero

Then:
  BROWSER_HOST_INITIALIZED
  WEBCONTENT_READY
  M11_READY
  M11_GUI_HANDSHAKE_OK
  M11_DOCUMENT_LOADED

IMPORTANT NEXT WATCH
--------------------
There is a separate latent issue to watch after tasks finally run:
BKL enter() can still park with HLT after a short active spin. If the local
TSC-deadline timer is not actually delivering periodic wakeups under TCG,
a contended AP may later park waiting for a BKL release that does not send an
IPI. Do not change that yet: v1.4 isolates the scheduler idle race first.

ROLLBACK
--------
  .\ROLLBACK-P0-IDLE-WAKE-HANDSHAKE-V14.ps1
