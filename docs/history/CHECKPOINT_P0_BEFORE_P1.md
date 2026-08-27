# Bouchaud OS - validated P0 checkpoint before P1

Checkpoint branch: $branch

## Runtime-validated state

- P0 targeted scheduler IPI enabled.
- P0 BKL resume liveness v1.3 enabled.
- P0 scheduler idle/wake handshake v1.4 enabled.
- Ladybird reached and rendered https://example.com/.
- P1 kernel-concurrency source patch has **not** been applied yet.
- Corrected P1 PowerShell apply/verify scripts are included for the next session.

## Next session

`powershell
cargo check
.\APPLY-P1-KERNEL-CONCURRENCY-V1.ps1 -Preview
.\APPLY-P1-KERNEL-CONCURRENCY-V1.ps1
.\VERIFY-P1-KERNEL-CONCURRENCY-V1.ps1 -Build
.\run.ps1
`

## Local artifacts intentionally not stored in Git

The following may exist on the original workstation but are intentionally not
part of this Git checkpoint:

- 	arget/
- .idea/
- .bouchaud-history/
- ladybird-browser.img
- 
ative-browser-m9/
- scenario-m9/
- BOUCHAUD-SMP4-KNOWN-GOOD/

They are build/debug/runtime artifacts or historical recovery material. If the
second workstation needs a generated Ladybird image, rebuild it there from the
project tooling rather than committing the 1.4 GB image.