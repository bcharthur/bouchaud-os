BOUCHAUD OS - NIGHT CHECKPOINT V2
================================

The first SAVE-NIGHT-CHECKPOINT run did several useful things successfully:

- validated the P0 v1.2/v1.3/v1.4 markers;
- validated that P1 source was not applied;
- ran cargo check successfully;
- switched to:
  checkpoint/p0-working-before-p1-20260825-235438
- staged the project state.

It then stopped because `ladybird-browser.img` is untracked. The v1 script
blindly ran:

  git restore --staged -- ladybird-browser.img

Git correctly reported that the path is not known to Git, and PowerShell
stopped before commit/push.

Nothing was lost. The branch is local and the staged state is still usable.

V2 FIX
------
The v2 script:

- reuses the already-created checkpoint branch;
- does NOT recreate it;
- enumerates the actual staged paths before unstaging local artifacts;
- never calls Git on an untracked generated path by name;
- commits the source/project state;
- excludes target/.idea/.bouchaud-history, the 1.4 GB browser image and local
  scenario/history directories;
- pushes the checkpoint branch;
- verifies the remote branch SHA equals the local commit SHA.

TONIGHT
-------
Extract this ZIP into the repository root and overwrite the files.

Then run exactly:

  .\SAVE-NIGHT-CHECKPOINT.ps1

You already had a successful cargo check immediately before the v1 failure, so
v2 does not repeat it by default.

If you explicitly want another build check:

  .\SAVE-NIGHT-CHECKPOINT.ps1 -Recheck

Do not shut down until the script prints:

  REMOTE CHECKPOINT VERIFIED - SAFE TO SHUT DOWN
