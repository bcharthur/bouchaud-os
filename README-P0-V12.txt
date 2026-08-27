BOUCHAUD OS - P0 #1 v1.2
============================

WHY v1.1 FAILED
---------------
The project was not the problem.

The v1.1 repair script itself was saved as UTF-8 without BOM and contained
non-ASCII text. Windows PowerShell 5.1 can parse such a script as the legacy
ANSI code page. The script was therefore corrupted while PowerShell was parsing
it, before a single command could execute.

That is why the parser showed text such as:

  nA...uds

inside REPAIR-P0-SCHED-IPI-TARGETED.ps1 itself.

v1.2 FIX
--------
This package is intentionally ASCII-only and is also written with a UTF-8 BOM.

FIX-P0-TARGETED-IPI-V12.ps1:
1. scans P0 backups;
2. identifies a known-good backup using the original raw UTF-8 byte sequence;
3. restores thread.rs and idt.rs byte-for-byte;
4. verifies their SHA-256 against the backup;
5. decodes source explicitly as strict UTF-8;
6. reapplies the targeted IPI patch;
7. verifies the original UTF-8 bytes survived;
8. runs git diff --check;
9. runs cargo check.

RUN
---

  .\FIX-P0-TARGETED-IPI-V12.ps1
  .\VERIFY-P0-TARGETED-IPI-V12.ps1 -Build
  .\run.ps1

Do not use the old v1 or v1.1 APPLY/REPAIR scripts again.

RUNTIME TEST
------------
Leave the desktop idle for roughly 10 seconds before opening Ladybird.

Expected result:
- CPU1/CPU2/CPU3 SMP-IPI counters should no longer rise mechanically by about
  250 interrupts per second while those CPUs are idle.
- Ladybird must still start and load https://example.com/.
