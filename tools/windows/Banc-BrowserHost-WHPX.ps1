#requires -Version 5.1
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$ReferenceCheckout,
    [Parameter(Mandatory)][string]$CandidateCheckout,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [ValidateSet('CommonKernel', 'FullStack')][string]$Mode = 'CommonKernel',
    [long]$ReferenceRunId = 45009297744,
    [long]$CandidateRunId = 45044774669
)
$ErrorActionPreference = 'Stop'
$argsBanc = @('--reference', $ReferenceCheckout, '--candidate', $CandidateCheckout,
    '--output', $OutputDirectory, '--mode', $Mode,
    '--reference-run', $ReferenceRunId, '--candidate-run', $CandidateRunId)
& python (Join-Path $PSScriptRoot 'banc-whpx-browserhost.py') @argsBanc
if ($LASTEXITCODE -ne 0) { throw "Campagne WHPX interrompue (code $LASTEXITCODE). Voir $OutputDirectory." }
