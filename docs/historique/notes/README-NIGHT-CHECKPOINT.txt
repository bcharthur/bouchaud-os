BOUCHAUD OS - NIGHT CHECKPOINT - 2026-08-25
=============================================

CURRENT SITUATION
-----------------
The first P1 apply attempt failed while creating its backup directory.

Root cause:
PowerShell variable names are case-insensitive. The script used both
`$LinuxBkl` for the path and `$linuxBkl` for the Rust source contents.
Those names are identical to PowerShell, so the path variable was overwritten
with the contents of bkl.rs. New-Item then tried to interpret Rust source text
as a filesystem path.

Important: the failure happened in the backup loop BEFORE any source write.
The subsequent VERIFY failure ("thread.rs marker missing") confirms that P1 was
not partially applied.

WHAT THIS PACKAGE DOES
----------------------
1. Replaces the broken P1 scripts with a corrected v1.1.
2. Provides SAVE-NIGHT-CHECKPOINT.ps1.
3. The checkpoint script validates the known-good P0 markers, runs git
   diff --check and cargo check, creates a dedicated checkpoint branch,
   commits the real project state, excludes build/history/IDE artifacts and
   the 1.4 GB browser image, then pushes the branch to origin.
4. Tomorrow, the other PC can clone/fetch that branch and continue from exactly
   the saved P0 state.

TONIGHT - ONE COMMAND SEQUENCE
------------------------------
Extract this ZIP INTO the bouchaud-os repository root, replacing the four P1
files already there.

Then run:

  .\SAVE-NIGHT-CHECKPOINT.ps1

If cargo check is already known-good and you only need to save immediately:

  .\SAVE-NIGHT-CHECKPOINT.ps1 -SkipCargoCheck

Do NOT apply P1 tonight. Save the known-good P0 state first.

TOMORROW
--------
The checkpoint script prints the exact branch name.

On the other PC:

  git clone <your-origin-url>
  cd bouchaud-os
  git fetch origin
  git switch <checkpoint-branch>

Then:

  cargo check
  .\APPLY-P1-KERNEL-CONCURRENCY-V1.ps1 -Preview
  .\APPLY-P1-KERNEL-CONCURRENCY-V1.ps1
  .\VERIFY-P1-KERNEL-CONCURRENCY-V1.ps1 -Build
  .\run.ps1

WHY P1 IS NOT APPLIED BEFORE BED
--------------------------------
The P0 scheduler state is the last runtime-validated milestone. P1 changes BKL,
WaitQueue, poll/ppoll and migration behavior. It should be applied on top of a
remote checkpoint and then tested with logs, rather than left as an unverified
local state on a PC you will not use tomorrow.
